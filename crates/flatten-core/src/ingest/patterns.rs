// crates/flatten-core/src/ingest/patterns.rs
//
// Pattern matching, directory walking, and .gitignore import.
// See: Ingest Rules and Symlinks contracts in docs/design/2_INGEST.md.

use std::path::Path;

use ignore::gitignore::{Gitignore, GitignoreBuilder};

use super::error::{Error, Result};
use crate::trie;

// ---------------------------------------------------------------------------
// Pattern matcher
// ---------------------------------------------------------------------------

/// Build a `Gitignore` matcher from stored patterns.
///
/// Prepends `.git` unconditionally (matches both the `.git` directory in
/// normal repos and the `.git` gitdir file in worktrees/submodules).
/// Validates each pattern via `add_line`; returns `InvalidPattern` on
/// malformed globs.
pub(crate) fn build_matcher(root: &Path, patterns: &[String]) -> Result<Gitignore> {
    let mut builder = GitignoreBuilder::new(root);

    builder
        .add_line(None, ".git")
        .map_err(|e| Error::InvalidPattern {
            pattern: ".git".into(),
            source: e,
        })?;

    for p in patterns {
        builder
            .add_line(None, p)
            .map_err(|e| Error::InvalidPattern {
                pattern: p.clone(),
                source: e,
            })?;
    }

    builder.build().map_err(|e| Error::InvalidPattern {
        pattern: "<pattern set>".into(),
        source: e,
    })
}

// ---------------------------------------------------------------------------
// Shared walk iterator
// ---------------------------------------------------------------------------

/// Walk a directory under the given patterns, calling `f` for each included
/// file/symlink entry. Returns the total lossy count across ALL entries
/// (including directories, before the file-type filter).
///
/// This preserves `walk_and_hash`'s existing counting semantics where
/// directory names contribute to `lossy_count`. The closure receives
/// `(absolute_path, trie_path, metadata)` and decides what to do with
/// each matched entry (hash it, collect its path, etc).
///
/// Both `walk_and_hash` and `walk_paths_filtered` consume this. The shared
/// implementation guarantees identical filtering behavior.
fn for_each_included<F>(
    root: &Path,
    patterns: &[String],
    mut f: F,
) -> Result<usize>
where
    F: FnMut(&Path, String, std::fs::Metadata) -> Result<()>,
{
    if !root.is_dir() {
        return Err(Error::NonExistentPath {
            path: root.display().to_string(),
        });
    }

    let matcher = build_matcher(root, patterns)?;
    let mut lossy_count = 0usize;

    let walker = ignore::WalkBuilder::new(root)
        .standard_filters(false)
        .sort_by_file_name(|a, b| a.cmp(b))
        .filter_entry(move |entry| {
            if entry.depth() == 0 {
                return true; // never cut root
            }
            let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());
            !matcher.matched(entry.path(), is_dir).is_ignore()
        })
        .build();

    for entry in walker {
        let entry = entry.map_err(|e| Error::Io {
            context: "walk entry".into(),
            source: std::io::Error::other(e.to_string()),
        })?;

        if entry.depth() == 0 {
            continue;
        }

        let rel = entry.path().strip_prefix(root).map_err(|_| Error::Io {
            context: format!("strip prefix: {}", entry.path().display()),
            source: std::io::Error::other("path not under root"),
        })?;
        let (trie_path, was_lossy) = trie::path_from_os(rel)?;
        if was_lossy {
            lossy_count += 1;
        }

        let metadata = entry.path().symlink_metadata().map_err(|e| Error::Io {
            context: format!("stat: {}", entry.path().display()),
            source: e,
        })?;

        // Only files and symlinks go to the closure.
        // Directories, FIFOs, sockets, devices: silently skipped.
        // But lossy_count above counts all entries (including dirs).
        if metadata.is_file() || metadata.file_type().is_symlink() {
            f(entry.path(), trie_path, metadata)?;
        }
    }

    Ok(lossy_count)
}

// ---------------------------------------------------------------------------
// Walk and hash
// ---------------------------------------------------------------------------

/// Walk a directory with gitignore-style patterns and produce trie leaves.
///
/// Callers must pass a canonicalized root (the `GitignoreBuilder` and
/// `strip_prefix` must see the same path). `register_repo` canonicalizes;
/// direct callers should too.
///
/// Returns `(leaves, lossy_count)` where `lossy_count` is the number of
/// entries (including directories) where `path_from_os` returned
/// `was_lossy = true`.
pub fn walk_and_hash(
    root: &Path,
    patterns: &[String],
) -> Result<(Vec<(String, trie::LeafNode)>, usize)> {
    let mut leaves = Vec::new();

    let lossy_count = for_each_included(root, patterns, |abs_path, trie_path, metadata| {
        if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(abs_path).map_err(|e| Error::Io {
                context: format!("readlink: {}", abs_path.display()),
                source: e,
            })?;

            #[cfg(unix)]
            let target_bytes = {
                use std::os::unix::ffi::OsStrExt;
                target.as_os_str().as_bytes().to_vec()
            };
            #[cfg(not(unix))]
            let target_lossy = target.to_string_lossy();
            #[cfg(not(unix))]
            let target_bytes = target_lossy.as_bytes().to_vec();

            let content_hash = blake3::hash(&target_bytes);
            leaves.push((
                trie_path,
                trie::LeafNode {
                    content_hash: *content_hash.as_bytes(),
                    size: target_bytes.len() as u64,
                    mtime: metadata_mtime_nanos(&metadata),
                },
            ));
        } else if metadata.is_file() {
            // Stream through hasher; no whole-file read (avoids OOM on large files).
            let mut file = std::fs::File::open(abs_path).map_err(|e| Error::Io {
                context: format!("open: {}", abs_path.display()),
                source: e,
            })?;
            let mut hasher = blake3::Hasher::new();
            let size = std::io::copy(&mut file, &mut hasher).map_err(|e| Error::Io {
                context: format!("hash: {}", abs_path.display()),
                source: e,
            })?;
            leaves.push((
                trie_path,
                trie::LeafNode {
                    content_hash: *hasher.finalize().as_bytes(),
                    size,
                    mtime: metadata_mtime_nanos(&metadata),
                },
            ));
        }
        // for_each_included already filters to files and symlinks, so the
        // else branch here is unreachable. Kept as defensive no-op.
        Ok(())
    })?;

    Ok((leaves, lossy_count))
}

// ---------------------------------------------------------------------------
// Walk paths (filtered, no hashing)
// ---------------------------------------------------------------------------

/// Walk a directory with patterns applied, returning matched file paths
/// without hashing. Cheaper than `walk_and_hash` for preview purposes.
///
/// Uses `for_each_included` to guarantee identical filtering behavior.
///
/// Callers must pass a canonicalized root (same requirement as `walk_and_hash`).
///
/// With an empty pattern list, returns all files except `.git` (equivalent
/// to an unfiltered walk; `build_matcher` with `&[]` adds only `.git`).
pub fn walk_paths_filtered(root: &Path, patterns: &[String]) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    let _lossy_count = for_each_included(root, patterns, |_abs, trie_path, _meta| {
        paths.push(trie_path);
        Ok(())
    })?;
    Ok(paths)
}

// ---------------------------------------------------------------------------
// .gitignore import
// ---------------------------------------------------------------------------

/// Import patterns from `.gitignore` files in a directory.
///
/// Uses `ignore::WalkBuilder` with `git_ignore(true)` so the import walk
/// respects `.gitignore` rules as it discovers them (prevents descent into
/// excluded directories like `node_modules/`).
///
/// For each yielded directory, checks `dir.join(".gitignore")` directly
/// (not filtered from walk entries; a `.gitignore` containing `*` would
/// exclude itself and never be yielded as a file entry).
///
/// Patterns from subdirectory `.gitignore` files are prefixed:
/// - Leading `/` anchored: strip `/`, emit `sub/pattern`
/// - Interior `/` (e.g. `src/gen/`): emit `sub/pattern`
/// - Unanchored (no `/` except trailing): emit `sub/**/pattern`
/// - Negation `!`: same rules, `!` stays on the outside
///
/// Prefix uses `trie::path_from_os` for forward-slash joining on all
/// platforms (no backslash-as-escape on Windows).
///
/// Malformed lines (e.g. inverted ranges like `[z-a]`) are silently
/// skipped via `add_line` validation. In practice the `ignore` crate
/// accepts nearly everything git accepts; the skip is defensive.
/// Explicit `--pattern` flags go through `build_matcher` and fail
/// loudly with `InvalidPattern`.
///
/// Returns pattern list in encounter order (no sort, no dedup; gitignore
/// precedence is order-dependent).
pub fn import_gitignore(root: &Path) -> Result<Vec<String>> {
    let mut patterns = Vec::new();

    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .require_git(false)
        .git_global(false)
        .git_exclude(false)
        .parents(false)
        .ignore(false)
        .follow_links(false)
        .sort_by_file_name(|a, b| a.cmp(b))
        .filter_entry(|e| e.file_name() != ".git")
        .build();

    for entry in walker {
        let entry = entry.map_err(|e| Error::Io {
            context: "import walk".into(),
            source: std::io::Error::other(e.to_string()),
        })?;

        if !entry.file_type().is_some_and(|ft| ft.is_dir()) {
            continue;
        }

        let gitignore_path = entry.path().join(".gitignore");
        if !gitignore_path.is_file() {
            continue;
        }

        let rel_dir = entry.path().strip_prefix(root).unwrap_or(Path::new(""));

        let content = std::fs::read_to_string(&gitignore_path).map_err(|e| Error::Io {
            context: format!("read: {}", gitignore_path.display()),
            source: e,
        })?;

        let is_root = rel_dir == Path::new("");
        let prefix = if is_root {
            String::new()
        } else {
            let (p, _) = trie::path_from_os(rel_dir)?;
            p
        };

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // Skip .git in any form
            let bare = trimmed
                .trim_start_matches('!')
                .trim_start_matches('/')
                .trim_end_matches('/');
            if bare == ".git" {
                continue;
            }

            if is_root {
                // Skip malformed lines (matching git behavior)
                if GitignoreBuilder::new("").add_line(None, trimmed).is_err() {
                    continue;
                }
                patterns.push(trimmed.to_string());
            } else {
                let (negated, core) = match trimmed.strip_prefix('!') {
                    Some(rest) => (true, rest),
                    None => (false, trimmed),
                };

                let was_anchored = core.starts_with('/');
                let core = core.strip_prefix('/').unwrap_or(core);

                let has_interior_slash = core.trim_end_matches('/').contains('/');
                let anchored = if was_anchored || has_interior_slash {
                    format!("{prefix}/{core}")
                } else {
                    // Unanchored: match at any depth under the subdir
                    format!("{prefix}/**/{core}")
                };

                let candidate = if negated {
                    format!("!{anchored}")
                } else {
                    anchored
                };

                // Skip malformed lines
                if GitignoreBuilder::new("").add_line(None, &candidate).is_err() {
                    continue;
                }
                patterns.push(candidate);
            }
        }
    }

    // No sort, no dedup. Gitignore precedence is order-dependent.
    Ok(patterns)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract mtime from metadata as nanoseconds since epoch.
///
/// Falls back to 0 on pre-epoch mtimes or platforms where `modified()`
/// is unsupported. Stat-based refresh (WA-001) treats 0 as "unknown,
/// re-hash."
fn metadata_mtime_nanos(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .map(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as i64)
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::test_util::test_dir_with_files;

    /// Helper: collect just the paths from walk results, sorted.
    fn walk_paths(root: &Path, patterns: &[String]) -> Vec<String> {
        let (leaves, _) = walk_and_hash(root, patterns).expect("walk_and_hash failed");
        let mut paths: Vec<String> = leaves.into_iter().map(|(p, _)| p).collect();
        paths.sort();
        paths
    }

    // --- Walk tests ---

    #[test]
    fn walk_excludes_patterns() {
        let dir = test_dir_with_files(&[
            ("src/main.rs", "fn main() {}"),
            ("node_modules/foo.js", "module.exports = {}"),
        ]);
        let patterns = vec!["node_modules/".to_string()];
        let paths = walk_paths(dir.path(), &patterns);

        assert!(
            paths.contains(&"src/main.rs".to_string()),
            "src/main.rs should be included"
        );
        assert!(
            !paths.iter().any(|p| p.starts_with("node_modules")),
            "node_modules/ should be excluded"
        );
    }

    #[test]
    fn walk_excludes_git_dir() {
        let dir = test_dir_with_files(&[
            ("src/main.rs", "fn main() {}"),
            (".git/config", "[core]"),
            (".git/HEAD", "ref: refs/heads/main"),
        ]);
        let patterns: Vec<String> = vec![];
        let paths = walk_paths(dir.path(), &patterns);

        assert!(
            paths.contains(&"src/main.rs".to_string()),
            "src/main.rs should be included"
        );
        assert!(
            !paths.iter().any(|p| p.starts_with(".git")),
            ".git/ should be excluded unconditionally"
        );
    }

    #[test]
    fn walk_excludes_git_file() {
        let dir = test_dir_with_files(&[
            ("src/main.rs", "fn main() {}"),
            (".git", "gitdir: /some/path/to/worktree"),
        ]);
        let patterns: Vec<String> = vec![];
        let paths = walk_paths(dir.path(), &patterns);

        assert!(
            paths.contains(&"src/main.rs".to_string()),
            "src/main.rs should be included"
        );
        assert!(
            !paths.contains(&".git".to_string()),
            ".git gitdir file should be excluded"
        );
    }

    #[test]
    fn walk_includes_unmatched_files() {
        let dir = test_dir_with_files(&[
            ("a.txt", "hello"),
            ("b.rs", "fn main() {}"),
            ("c/d.py", "print('hi')"),
        ]);
        let patterns = vec!["*.log".to_string()];
        let paths = walk_paths(dir.path(), &patterns);

        assert_eq!(paths.len(), 3, "all three files should be included");
        assert!(paths.contains(&"a.txt".to_string()), "a.txt missing");
        assert!(paths.contains(&"b.rs".to_string()), "b.rs missing");
        assert!(paths.contains(&"c/d.py".to_string()), "c/d.py missing");
    }

    #[test]
    fn walk_negation_reincludes() {
        let dir = test_dir_with_files(&[("a.txt", "aaa"), ("b.txt", "bbb")]);
        let patterns = vec!["*.txt".to_string(), "!b.txt".to_string()];
        let paths = walk_paths(dir.path(), &patterns);

        assert!(
            !paths.contains(&"a.txt".to_string()),
            "a.txt should be excluded by *.txt"
        );
        assert!(
            paths.contains(&"b.txt".to_string()),
            "b.txt should be re-included by !b.txt"
        );
    }

    #[test]
    fn walk_negation_under_excluded_dir() {
        let dir = test_dir_with_files(&[
            ("node_modules/keep.js", "keep me"),
            ("node_modules/other.js", "other"),
        ]);
        let patterns = vec![
            "node_modules/".to_string(),
            "!node_modules/keep.js".to_string(),
        ];
        let paths = walk_paths(dir.path(), &patterns);

        assert!(
            !paths.contains(&"node_modules/keep.js".to_string()),
            "cannot re-include a file under an excluded directory"
        );
        assert!(
            !paths.contains(&"node_modules/other.js".to_string()),
            "node_modules/other.js should be excluded"
        );
    }

    #[cfg(unix)]
    #[test]
    fn walk_directory_cut_never_descends() {
        use crate::ingest::test_util::RestorePerms;

        let dir = test_dir_with_files(&[
            ("ok.txt", "visible"),
            ("locked/secret.txt", "hidden"),
        ]);

        let locked = dir.path().join("locked");
        let _guard = RestorePerms::set(&locked, 0o000);

        if _guard.can_still_read() {
            // Running as root; chmod 000 is ineffective. Skip.
            return;
        }

        // With "locked/" excluded, the walker should cut (never descend).
        // If it tried to descend, it would hit a permission error.
        let patterns = vec!["locked/".to_string()];
        let paths = walk_paths(dir.path(), &patterns);

        assert!(
            paths.contains(&"ok.txt".to_string()),
            "ok.txt should be included"
        );
        assert!(
            !paths.iter().any(|p| p.starts_with("locked")),
            "locked/ should be cut, not descended"
        );
    }

    #[cfg(unix)]
    #[test]
    fn walk_symlink_leaf_content() {
        let dir = test_dir_with_files(&[("target.txt", "hello")]);

        // Create a symlink: link.txt -> target.txt
        let link_path = dir.path().join("link.txt");
        std::os::unix::fs::symlink("target.txt", &link_path)
            .expect("failed to create symlink");

        let patterns: Vec<String> = vec![];
        let (leaves, _) = walk_and_hash(dir.path(), &patterns).expect("walk failed");

        let link_leaf = leaves
            .iter()
            .find(|(p, _)| p == "link.txt")
            .expect("link.txt should be in results");

        // Symlink content is the target string "target.txt", hashed as bytes.
        let expected_hash = blake3::hash("target.txt".as_bytes());
        assert_eq!(
            link_leaf.1.content_hash,
            *expected_hash.as_bytes(),
            "symlink leaf should hash the target string, not the target file content"
        );
        assert_eq!(
            link_leaf.1.size,
            "target.txt".len() as u64,
            "symlink size should be the target string length"
        );
    }

    #[test]
    fn walk_empty_patterns_all_except_git() {
        // Fixture has a .gitignore with *.log and a debug.log.
        // With empty patterns, only .git is excluded. The on-disk .gitignore
        // has no effect on walk_and_hash (no live dependency; proves SSOT).
        let dir = test_dir_with_files(&[
            ("a.txt", "hello"),
            (".git/config", "[core]"),
            (".gitignore", "*.log\n"),
            ("debug.log", "log entry"),
        ]);
        let patterns: Vec<String> = vec![];
        let paths = walk_paths(dir.path(), &patterns);

        assert!(paths.contains(&"a.txt".to_string()), "a.txt should be included");
        assert!(
            paths.contains(&".gitignore".to_string()),
            ".gitignore itself should be included"
        );
        assert!(
            paths.contains(&"debug.log".to_string()),
            "debug.log should be included (on-disk .gitignore has no effect)"
        );
        assert!(
            !paths.iter().any(|p| p.starts_with(".git/")),
            ".git/ directory should be excluded"
        );
    }

    #[test]
    fn walk_glob_star_and_doublestar() {
        let dir = test_dir_with_files(&[
            ("a.log", "log"),
            ("b.txt", "text"),
            ("sub/c.tmp", "temp"),
            ("sub/deep/d.tmp", "deep temp"),
        ]);
        let patterns = vec!["*.log".to_string(), "**/*.tmp".to_string()];
        let paths = walk_paths(dir.path(), &patterns);

        assert!(
            paths.contains(&"b.txt".to_string()),
            "b.txt should be included"
        );
        assert!(
            !paths.contains(&"a.log".to_string()),
            "a.log should be excluded by *.log"
        );
        assert!(
            !paths.contains(&"sub/c.tmp".to_string()),
            "sub/c.tmp should be excluded by **/*.tmp"
        );
        assert!(
            !paths.contains(&"sub/deep/d.tmp".to_string()),
            "sub/deep/d.tmp should be excluded by **/*.tmp"
        );
    }

    #[test]
    fn walk_glob_question_mark() {
        let dir = test_dir_with_files(&[("a.txt", "one char"), ("ab.txt", "two chars")]);
        let patterns = vec!["?.txt".to_string()];
        let paths = walk_paths(dir.path(), &patterns);

        assert!(
            !paths.contains(&"a.txt".to_string()),
            "a.txt should be excluded by ?.txt"
        );
        assert!(
            paths.contains(&"ab.txt".to_string()),
            "ab.txt should NOT be excluded by ?.txt (two chars)"
        );
    }

    #[cfg(unix)]
    #[test]
    fn walk_walker_error_fails_ingest() {
        use crate::ingest::test_util::RestorePerms;

        let dir = test_dir_with_files(&[
            ("ok.txt", "visible"),
            ("locked/secret.txt", "hidden"),
        ]);

        let locked = dir.path().join("locked");
        let _guard = RestorePerms::set(&locked, 0o000);

        if _guard.can_still_read() {
            return;
        }

        // No exclusion pattern: the walker WILL try to descend into locked/
        let result = walk_and_hash(dir.path(), &[]);

        assert!(
            result.is_err(),
            "walk should fail on permission denied"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, Error::Io { .. }),
            "error should be Io variant, got: {err}"
        );
    }

    #[test]
    fn walk_anchored_pattern_root_relative() {
        let dir = test_dir_with_files(&[
            ("src/gen.rs", "generated"),
            ("other/src/gen.rs", "also generated"),
        ]);
        // Leading / anchors to the root: only src/gen.rs at root level is excluded
        let patterns = vec!["/src/gen.rs".to_string()];
        let paths = walk_paths(dir.path(), &patterns);

        assert!(
            !paths.contains(&"src/gen.rs".to_string()),
            "src/gen.rs at root should be excluded by /src/gen.rs"
        );
        assert!(
            paths.contains(&"other/src/gen.rs".to_string()),
            "other/src/gen.rs should NOT be excluded (different prefix)"
        );
    }

    // --- walk_paths_filtered tests ---

    #[test]
    fn walk_filtered_empty_patterns_includes_all() {
        let dir = test_dir_with_files(&[
            ("a.txt", "hello"),
            ("sub/b.rs", "fn main() {}"),
            (".git/config", "[core]"),
        ]);
        let paths = walk_paths_filtered(dir.path(), &[]).expect("walk should succeed");

        assert!(paths.contains(&"a.txt".to_string()), "a.txt included");
        assert!(paths.contains(&"sub/b.rs".to_string()), "sub/b.rs included");
        assert!(
            !paths.iter().any(|p| p.starts_with(".git")),
            ".git excluded"
        );
    }

    #[test]
    fn walk_filtered_skips_git() {
        let dir = test_dir_with_files(&[
            ("a.txt", "hello"),
            (".git/HEAD", "ref: refs/heads/main"),
        ]);
        let paths = walk_paths_filtered(dir.path(), &[]).expect("walk should succeed");

        assert!(paths.contains(&"a.txt".to_string()), "a.txt included");
        assert!(
            !paths.iter().any(|p| p.starts_with(".git")),
            ".git excluded with empty patterns"
        );
    }

    #[test]
    fn walk_filtered_empty_dir() {
        let dir = tempfile::TempDir::new().expect("create temp dir");
        let paths = walk_paths_filtered(dir.path(), &[]).expect("walk should succeed");
        assert!(paths.is_empty(), "empty dir returns empty vec");
    }

    #[test]
    fn walk_filtered_nonexistent() {
        let result = walk_paths_filtered(Path::new("/nonexistent/path/abc"), &[]);
        assert!(
            matches!(result, Err(Error::NonExistentPath { .. })),
            "nonexistent path returns NonExistentPath"
        );
    }

    #[test]
    fn walk_filtered_matches_walk_and_hash() {
        // Fixture includes nested excluded dir to ensure both walks agree
        // on directory-level cuts. Symlink tested on unix only (separate test).
        let dir = test_dir_with_files(&[
            ("src/main.rs", "fn main() {}"),
            ("src/lib.rs", "pub fn lib() {}"),
            ("node_modules/foo.js", "module.exports = {}"),
            ("node_modules/bar/baz.js", "nested"),
            ("build/output.o", "binary"),
            ("README.md", "# readme"),
        ]);
        let patterns = vec!["node_modules/".to_string(), "*.o".to_string()];

        let (leaves, _) = walk_and_hash(dir.path(), &patterns).expect("walk_and_hash");
        let mut hash_paths: Vec<String> = leaves.into_iter().map(|(p, _)| p).collect();
        hash_paths.sort();

        let mut filtered_paths = walk_paths_filtered(dir.path(), &patterns)
            .expect("walk_paths_filtered");
        filtered_paths.sort();

        assert_eq!(
            hash_paths, filtered_paths,
            "walk_paths_filtered must return exactly the same paths as walk_and_hash"
        );
    }

    #[test]
    fn walk_filtered_respects_patterns() {
        let dir = test_dir_with_files(&[
            ("a.txt", "hello"),
            ("b.log", "log entry"),
            ("sub/c.log", "nested log"),
        ]);
        let patterns = vec!["*.log".to_string()];
        let paths = walk_paths_filtered(dir.path(), &patterns).expect("walk should succeed");

        assert!(paths.contains(&"a.txt".to_string()), "a.txt included");
        assert!(!paths.contains(&"b.log".to_string()), "b.log excluded");
        assert!(!paths.contains(&"sub/c.log".to_string()), "sub/c.log excluded");
    }

    // --- Import tests ---

    #[test]
    fn import_reads_root_gitignore() {
        let dir = test_dir_with_files(&[(".gitignore", "*.log\nbuild/\n")]);
        let patterns = import_gitignore(dir.path()).expect("import failed");

        assert_eq!(
            patterns,
            vec!["*.log", "build/"],
            "should read root .gitignore patterns"
        );
    }

    #[test]
    fn import_anchors_unanchored_subdir() {
        let dir = test_dir_with_files(&[("sub/.gitignore", "build/\n")]);
        let patterns = import_gitignore(dir.path()).expect("import failed");

        assert_eq!(
            patterns,
            vec!["sub/**/build/"],
            "unanchored pattern from subdir should use **"
        );
    }

    #[test]
    fn import_anchors_slashed_subdir() {
        let dir = test_dir_with_files(&[("sub/.gitignore", "/dist\n")]);
        let patterns = import_gitignore(dir.path()).expect("import failed");

        assert_eq!(
            patterns,
            vec!["sub/dist"],
            "leading-/ pattern from subdir should be directly prefixed"
        );
    }

    #[test]
    fn import_anchors_interior_slash() {
        let dir = test_dir_with_files(&[("sub/.gitignore", "src/gen/\n")]);
        let patterns = import_gitignore(dir.path()).expect("import failed");

        assert_eq!(
            patterns,
            vec!["sub/src/gen/"],
            "interior-slash pattern from subdir should be directly prefixed"
        );
    }

    #[test]
    fn import_skips_comments_blanks() {
        let dir = test_dir_with_files(&[(".gitignore", "# comment\n\n*.log\n  \n")]);
        let patterns = import_gitignore(dir.path()).expect("import failed");

        assert_eq!(patterns, vec!["*.log"], "comments and blanks should be skipped");
    }

    #[test]
    fn import_preserves_order() {
        let dir = test_dir_with_files(&[(".gitignore", "*.txt\n!keep.txt\n")]);
        let patterns = import_gitignore(dir.path()).expect("import failed");

        assert_eq!(
            patterns,
            vec!["*.txt", "!keep.txt"],
            "order must be preserved (gitignore is order-dependent)"
        );
    }

    #[test]
    fn import_no_gitignore_returns_empty() {
        let dir = test_dir_with_files(&[("src/main.rs", "fn main() {}")]);
        let patterns = import_gitignore(dir.path()).expect("import failed");

        assert!(
            patterns.is_empty(),
            "no .gitignore should return empty vec"
        );
    }

    #[test]
    fn import_skips_excluded_dirs() {
        let dir = test_dir_with_files(&[
            (".gitignore", "node_modules/\n"),
            ("node_modules/.gitignore", "*.map\n"),
            ("src/main.rs", "fn main() {}"),
        ]);
        let patterns = import_gitignore(dir.path()).expect("import failed");

        assert_eq!(
            patterns,
            vec!["node_modules/"],
            "should not import patterns from excluded directories"
        );
    }

    #[test]
    fn import_self_ignoring_gitignore() {
        let dir = test_dir_with_files(&[("sub/.gitignore", "*\n")]);
        let patterns = import_gitignore(dir.path()).expect("import failed");

        assert_eq!(
            patterns,
            vec!["sub/**/*"],
            ".gitignore containing * should still be imported (checked via dir entry, not file entry)"
        );
    }

    #[test]
    fn import_skips_malformed_lines() {
        // `[z-a]` is an inverted character class range that globset rejects.
        // The import pipeline silently drops it (matching git's lenient behavior).
        let dir = test_dir_with_files(&[(".gitignore", "*.log\n[z-a]\nkeep.txt\n")]);
        let patterns = import_gitignore(dir.path()).expect("import failed");

        assert_eq!(
            patterns,
            vec!["*.log", "keep.txt"],
            "malformed line '[z-a]' should be silently skipped"
        );
    }

    #[test]
    fn import_subdir_patterns_work_when_walked() {
        // End-to-end: import a subdir .gitignore, then walk with the
        // imported patterns. Proves the anchoring transform produces
        // patterns that actually exclude correctly at walk time.
        let dir = test_dir_with_files(&[
            ("sub/.gitignore", "build/\n"),
            ("sub/build/x.o", "object file"),
            ("sub/deep/build/y.o", "deep object file"),
            ("other/build/z.o", "unrelated build dir"),
            ("root.txt", "root file"),
        ]);

        let patterns = import_gitignore(dir.path()).expect("import failed");
        // Should produce sub/**/build/ (unanchored -> **)
        assert_eq!(patterns, vec!["sub/**/build/"]);

        let paths = walk_paths(dir.path(), &patterns);

        assert!(
            !paths.iter().any(|p| p.starts_with("sub/build/")),
            "sub/build/ should be excluded by imported pattern"
        );
        assert!(
            !paths.iter().any(|p| p.starts_with("sub/deep/build/")),
            "sub/deep/build/ should be excluded (** matches nested)"
        );
        assert!(
            paths.contains(&"other/build/z.o".to_string()),
            "other/build/z.o should be present (pattern is scoped to sub/)"
        );
        assert!(
            paths.contains(&"root.txt".to_string()),
            "root.txt should be present"
        );
    }
}
