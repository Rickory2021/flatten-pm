// src-tauri/src/commands/repo.rs
//
// Repo commands: registration, CRUD, tree, preview, file read, excluded
// files, gitignore import. Wraps flatten-core::ingest functions.
// See: APP-003 spec and docs/design/2_INGEST.md.

use std::path::{Path, PathBuf};

use crate::error::CommandError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// DTO for `IngestReport`. Converts `root_hash` to hex for frontend display.
/// `IngestReport` derives `Debug, Clone` only; this provides `Serialize`.
#[derive(serde::Serialize)]
pub struct IngestReportDto {
    pub file_count: usize,
    pub root_hash: String,
    pub lossy_count: usize,
    pub enrichment_count: usize,
}

impl From<flatten_core::ingest::IngestReport> for IngestReportDto {
    fn from(r: flatten_core::ingest::IngestReport) -> Self {
        Self {
            file_count: r.file_count,
            root_hash: r.root_hash_hex(),
            lossy_count: r.lossy_count,
            enrichment_count: r.enrichment_count,
        }
    }
}

/// Returned by `repo_add` so the frontend has the new repo's ID for
/// navigation to the detail view.
#[derive(serde::Serialize)]
pub struct RepoAddResult {
    pub id: i64,
    pub report: IngestReportDto,
}

/// Returned by `repo_read_file`. Distinguishes symlinks from regular files
/// so the frontend can display them differently.
#[derive(Debug, serde::Serialize)]
pub struct FilePreviewDto {
    /// "file" or "symlink"
    pub kind: &'static str,
    /// File content (UTF-8 text) or symlink target path.
    pub content: String,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// List all non-deleted repos.
#[tauri::command]
pub async fn repo_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<flatten_core::ingest::RepoRow>, CommandError> {
    let conn = flatten_core::db::open_reader(&state.db_path)?;
    Ok(flatten_core::ingest::list_repos(&conn)?)
}

/// Register a new repo. Returns the ID and ingest report.
#[tauri::command]
pub async fn repo_add(
    path: String,
    name: String,
    patterns: Vec<String>,
    import_gitignore: bool,
    state: tauri::State<'_, AppState>,
) -> Result<RepoAddResult, CommandError> {
    let (id, report) = flatten_core::ingest::register_repo(
        &state.writer,
        &state.data_dir,
        &PathBuf::from(&path),
        &name,
        &patterns,
        "preserve",
        import_gitignore,
    )?;
    Ok(RepoAddResult {
        id,
        report: IngestReportDto::from(report),
    })
}

/// Return trie paths for a repo. Validates the repo exists first.
#[tauri::command]
pub async fn repo_tree(
    repo_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<String>, CommandError> {
    let conn = flatten_core::db::open_reader(&state.db_path)?;
    let _repo = flatten_core::ingest::get_repo(&conn, repo_id)?;
    let (trie, _) = flatten_core::ingest::load_or_reingest(
        &state.writer,
        &state.data_dir,
        repo_id,
    )?;
    Ok(trie.list(""))
}

/// Update repo fields. Only `Some` fields are changed. Triggers re-ingest.
///
/// Rename is not exposed in v1 (needs empty-name guard in core).
/// The `name` parameter exists for forward compatibility.
#[tauri::command]
pub async fn repo_edit(
    repo_id: i64,
    name: Option<String>,
    patterns: Option<Vec<String>>,
    line_ending_policy: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<IngestReportDto, CommandError> {
    let (_, report) = flatten_core::ingest::edit_repo(
        &state.writer,
        &state.data_dir,
        repo_id,
        name.as_deref(),
        patterns.as_deref(),
        line_ending_policy.as_deref(),
    )?;
    Ok(IngestReportDto::from(report))
}

/// Trigger a full re-ingest of a repo.
#[tauri::command]
pub async fn repo_reingest(
    repo_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<IngestReportDto, CommandError> {
    let (_, report) = flatten_core::ingest::reingest(
        &state.writer,
        &state.data_dir,
        repo_id,
    )?;
    Ok(IngestReportDto::from(report))
}

/// Soft-delete a repo. Cascade UX deferred to DA-005.
#[tauri::command]
pub async fn repo_delete(
    repo_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), CommandError> {
    flatten_core::ingest::soft_delete_repo(&state.writer, repo_id)?;
    Ok(())
}

/// Return files on disk that are NOT in the trie (excluded by patterns).
/// Uses an unfiltered walk (empty pattern list) and diffs against the trie.
/// Opt-in call from the frontend "show excluded" toggle.
#[tauri::command]
pub async fn repo_excluded_files(
    repo_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<String>, CommandError> {
    let conn = flatten_core::db::open_reader(&state.db_path)?;
    let repo = flatten_core::ingest::get_repo(&conn, repo_id)?;

    let root = PathBuf::from(&repo.path);
    let all_paths = flatten_core::ingest::walk_paths_filtered(&root, &[])?;

    let (trie, _) = flatten_core::ingest::load_or_reingest(
        &state.writer,
        &state.data_dir,
        repo_id,
    )?;
    let trie_paths: std::collections::HashSet<String> =
        trie.list("").into_iter().collect();

    Ok(all_paths
        .into_iter()
        .filter(|p| !trie_paths.contains(p))
        .collect())
}

/// Preview which files would be included for a directory with given patterns.
/// Does not touch the database. Uses `canonical_root` + `assemble_patterns`
/// from core for identical canonicalization and merge order to `register_repo`.
#[tauri::command]
pub async fn repo_preview(
    path: String,
    patterns: Vec<String>,
    import_gitignore: bool,
) -> Result<Vec<String>, CommandError> {
    let root = PathBuf::from(&path);
    let canonical = flatten_core::ingest::canonical_root(&root)?;
    let final_patterns = flatten_core::ingest::assemble_patterns(
        &canonical,
        &patterns,
        import_gitignore,
    )?;
    Ok(flatten_core::ingest::walk_paths_filtered(&canonical, &final_patterns)?)
}

/// Read a file from a repo for preview. Validates path safety, size, and
/// encoding. Returns structured result distinguishing files from symlinks.
///
/// Extracted as a free function for testability. The Tauri command is a
/// thin wrapper.
///
/// Security model:
///   1. `validate_path` rejects absolute, `..`, `.`, empty, double-slash.
///   2. Parent directory containment: canonicalize the parent of the target
///      and verify it is under the canonical repo root. This catches
///      intermediate symlinks that escape the repo (e.g. `repo/linkdir`
///      pointing outside, then requesting `linkdir/x`).
///   3. Symlinks at the leaf level return the link target string (what
///      ingest hashed), not the resolved content.
///   4. Regular files get a belt-and-braces canonicalize + starts_with check,
///      plus size guard, binary check, and UTF-8 validation.
fn read_repo_file(
    repo_root: &Path,
    relative: &str,
) -> std::result::Result<FilePreviewDto, CommandError> {
    // 1. Lexical validation: reject any non-trie path before fs access
    flatten_core::trie::validate_path(relative)
        .map_err(|e| CommandError::domain(format!("invalid path: {e}")))?;

    let full_path = repo_root.join(relative);

    // 2. Parent directory containment check: canonicalize the parent and
    // verify it is under the repo root. This catches intermediate symlinks
    // that escape the repo before we touch the target file at all.
    let parent = full_path.parent().ok_or_else(|| {
        CommandError::domain("invalid path: no parent".to_string())
    })?;
    let parent_canonical = std::fs::canonicalize(parent)
        .map_err(|e| CommandError {
            error: format!("file not found: {e}"),
            kind: "io",
        })?;
    let repo_canonical = std::fs::canonicalize(repo_root)
        .map_err(|e| CommandError {
            error: format!("repo path invalid: {e}"),
            kind: "io",
        })?;
    if !parent_canonical.starts_with(&repo_canonical) {
        return Err(CommandError::domain("path traversal denied".to_string()));
    }

    // 3. Type check
    let link_meta = std::fs::symlink_metadata(&full_path)
        .map_err(|e| CommandError {
            error: format!("file not found: {e}"),
            kind: "io",
        })?;

    // 4. Symlink: return the link target string (what ingest hashed).
    // Parent containment already verified above, so this is safe.
    if link_meta.file_type().is_symlink() {
        let target = std::fs::read_link(&full_path)
            .map_err(|e| CommandError {
                error: format!("cannot read symlink: {e}"),
                kind: "io",
            })?;
        return Ok(FilePreviewDto {
            kind: "symlink",
            content: target.to_string_lossy().into_owned(),
        });
    }

    // 5. Regular file: belt-and-braces canonicalize + containment
    let canonical = std::fs::canonicalize(&full_path)
        .map_err(|e| CommandError {
            error: format!("file not found: {e}"),
            kind: "io",
        })?;
    if !canonical.starts_with(&repo_canonical) {
        return Err(CommandError::domain("path traversal denied".to_string()));
    }

    // 6. Size guard: reject files over 1MB
    let metadata = std::fs::metadata(&canonical)
        .map_err(|e| CommandError {
            error: format!("cannot stat file: {e}"),
            kind: "io",
        })?;
    if metadata.len() > 1_048_576 {
        return Err(CommandError::domain(format!(
            "file too large for preview ({} bytes, max 1MB)",
            metadata.len()
        )));
    }

    // 7. Binary check: null byte in first 8KB
    let content = std::fs::read(&canonical)
        .map_err(|e| CommandError {
            error: format!("cannot read file: {e}"),
            kind: "io",
        })?;
    if content.iter().take(8192).any(|&b| b == 0) {
        return Err(CommandError::domain(
            "binary file; preview not available".to_string(),
        ));
    }

    // 8. UTF-8 decode
    let text = String::from_utf8(content)
        .map_err(|_| CommandError::domain("file is not valid UTF-8".to_string()))?;

    Ok(FilePreviewDto {
        kind: "file",
        content: text,
    })
}

/// Read a file from a registered repo for preview.
#[tauri::command]
pub async fn repo_read_file(
    repo_id: i64,
    relative_path: String,
    state: tauri::State<'_, AppState>,
) -> Result<FilePreviewDto, CommandError> {
    let conn = flatten_core::db::open_reader(&state.db_path)?;
    let repo = flatten_core::ingest::get_repo(&conn, repo_id)?;
    read_repo_file(&PathBuf::from(&repo.path), &relative_path)
}

/// Import .gitignore patterns from a directory. Takes a path (not repo_id)
/// so it works in the wizard (before registration) and in the detail view.
/// Calls `canonical_root` first for consistent error kinds.
#[tauri::command]
pub async fn repo_gitignore_patterns(
    path: String,
) -> Result<Vec<String>, CommandError> {
    let root = PathBuf::from(&path);
    let canonical = flatten_core::ingest::canonical_root(&root)?;
    Ok(flatten_core::ingest::import_gitignore(&canonical)?)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Create a temp directory with files at given relative paths.
    fn test_dir_with_files(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().expect("create temp dir");
        for (path, content) in files {
            let full = dir.path().join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("create parent dirs");
            }
            std::fs::write(&full, content).expect("write file");
        }
        dir
    }

    // --- Error mapping tests (T1-T3) ---

    #[test]
    fn ingest_error_maps_domain_variants() {
        use flatten_core::ingest::error::Error as IE;

        let cases: Vec<IE> = vec![
            IE::NonExistentPath {
                path: "/tmp".into(),
            },
            IE::DuplicateName {
                name: "dup".into(),
            },
            IE::RepoNotFound("id 1".into()),
            IE::InvalidLineEndingPolicy {
                value: "crlf".into(),
            },
        ];

        for e in cases {
            let cmd_err = CommandError::from(e);
            assert_eq!(
                cmd_err.kind, "domain",
                "expected domain, got {} for: {}",
                cmd_err.kind, cmd_err.error
            );
        }
    }

    #[test]
    fn ingest_error_maps_io() {
        use flatten_core::ingest::error::Error as IE;

        let e = IE::Io {
            context: "test".into(),
            source: std::io::Error::other("test io"),
        };
        let cmd_err = CommandError::from(e);
        assert_eq!(cmd_err.kind, "io", "Io should map to io");
    }

    #[test]
    fn ingest_error_maps_database() {
        use flatten_core::ingest::error::Error as IE;

        // Json variant
        let json_err = IE::Json(
            serde_json::from_str::<String>("not json").unwrap_err(),
        );
        let cmd_err = CommandError::from(json_err);
        assert_eq!(cmd_err.kind, "database", "Json should map to database");

        // Trie variant
        let trie_err = IE::Trie(
            flatten_core::trie::error::Error::InvalidPath("test".into()),
        );
        let cmd_err2 = CommandError::from(trie_err);
        assert_eq!(cmd_err2.kind, "database", "Trie should map to database");

        // Database variant
        let db_err = IE::Database(
            flatten_core::db::error::Error::Writer("test".into()),
        );
        let cmd_err3 = CommandError::from(db_err);
        assert_eq!(cmd_err3.kind, "database", "Database should map to database");
    }

    // --- DTO tests (T4) ---

    #[test]
    fn ingest_report_dto_hex() {
        let report = flatten_core::ingest::IngestReport {
            file_count: 5,
            root_hash: [0xab; 32],
            lossy_count: 0,
            enrichment_count: 0,
        };
        let dto = IngestReportDto::from(report);
        assert_eq!(dto.root_hash.len(), 64, "hex should be 64 chars");
        assert_eq!(&dto.root_hash[..4], "abab", "first two bytes 0xab -> 'abab'");
        assert_eq!(dto.file_count, 5, "file_count preserved");
    }

    // --- File preview tests (T5-T10+) ---

    #[test]
    fn read_repo_file_rejects_absolute() {
        let dir = test_dir_with_files(&[("a.txt", "hello")]);
        let result = read_repo_file(dir.path(), "/etc/passwd");
        assert!(result.is_err(), "absolute path should be rejected");
        let err = result.unwrap_err();
        assert_eq!(err.kind, "domain", "should be domain error");
        assert!(
            err.error.contains("invalid path"),
            "error should mention invalid path: {}",
            err.error
        );
    }

    #[test]
    fn read_repo_file_rejects_dotdot() {
        let dir = test_dir_with_files(&[("a.txt", "hello")]);
        let result = read_repo_file(dir.path(), "../secret");
        assert!(result.is_err(), ".. path should be rejected");
        let err = result.unwrap_err();
        assert_eq!(err.kind, "domain", "should be domain error");
    }

    #[test]
    fn read_repo_file_rejects_large() {
        let dir = tempfile::TempDir::new().unwrap();
        let big_file = dir.path().join("big.txt");
        // Write 1MB + 1 byte
        let data = vec![b'x'; 1_048_577];
        std::fs::write(&big_file, &data).unwrap();

        let result = read_repo_file(dir.path(), "big.txt");
        assert!(result.is_err(), "large file should be rejected");
        let err = result.unwrap_err();
        assert_eq!(err.kind, "domain", "should be domain error");
        assert!(
            err.error.contains("too large"),
            "error should mention size: {}",
            err.error
        );
    }

    #[test]
    fn read_repo_file_rejects_binary() {
        let dir = tempfile::TempDir::new().unwrap();
        let bin_file = dir.path().join("image.bin");
        // Mostly printable bytes with a single null byte at index 50
        let mut data = vec![b'x'; 100];
        data[50] = 0;
        std::fs::write(&bin_file, &data).unwrap();

        let result = read_repo_file(dir.path(), "image.bin");
        assert!(result.is_err(), "binary file should be rejected");
        let err = result.unwrap_err();
        assert_eq!(err.kind, "domain", "should be domain error");
        assert!(
            err.error.contains("binary"),
            "error should mention binary: {}",
            err.error
        );
    }

    #[cfg(unix)]
    #[test]
    fn read_repo_file_symlink() {
        let dir = test_dir_with_files(&[("target.txt", "hello")]);
        let link_path = dir.path().join("link.txt");
        std::os::unix::fs::symlink("target.txt", &link_path).unwrap();

        let result = read_repo_file(dir.path(), "link.txt");
        assert!(result.is_ok(), "symlink should succeed");
        let dto = result.unwrap();
        assert_eq!(dto.kind, "symlink", "kind should be symlink");
        assert_eq!(dto.content, "target.txt", "content should be link target");
    }

    #[test]
    fn read_repo_file_normal() {
        let dir = test_dir_with_files(&[("hello.txt", "hello world")]);

        let result = read_repo_file(dir.path(), "hello.txt");
        assert!(result.is_ok(), "normal file should succeed");
        let dto = result.unwrap();
        assert_eq!(dto.kind, "file", "kind should be file");
        assert_eq!(dto.content, "hello world", "content should match");
    }

    /// A symlinked parent directory that escapes the repo should be caught
    /// by the parent containment check, even for valid trie paths.
    #[cfg(unix)]
    #[test]
    fn read_repo_file_rejects_symlinked_parent_escape() {
        // Create "outside" dir with a file
        let outside = test_dir_with_files(&[("secret.txt", "sensitive data")]);

        // Create "repo" dir with a symlink to outside
        let repo = tempfile::TempDir::new().unwrap();
        let link_dir = repo.path().join("linkdir");
        std::os::unix::fs::symlink(outside.path(), &link_dir).unwrap();

        // "linkdir/secret.txt" is a valid trie path, but linkdir resolves
        // outside the repo. The parent containment check should catch it.
        let result = read_repo_file(repo.path(), "linkdir/secret.txt");
        assert!(result.is_err(), "symlinked parent escape should be rejected");
        let err = result.unwrap_err();
        assert_eq!(err.kind, "domain", "should be domain error");
        assert!(
            err.error.contains("path traversal denied"),
            "error should mention traversal: {}",
            err.error
        );
    }
}
