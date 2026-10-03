// crates/flatten-core/src/recipe/parse/path.rs
//
// Path normalization, the canonical COPY shape, and identifier rules.
//
// Pure string functions. The normalized COPY `src` and `dest` are what
// export records as `src_prefix` and `dest_prefix`, and watch picks prefix
// vs exact matching from them (docs/design/5_WATCH.md, COPY matching rules),
// so the trailing slash is load-bearing and normalization preserves it.

use crate::recipe::error::PathIssue;

/// Normalize a COPY `src` or `dest` lexically.
///
/// Rejects empty paths, control characters, backslashes, absolute paths
/// (leading `/` or a drive letter), and `..` segments. Strips `.` segments
/// and collapses repeated `/`. Keeps a trailing `/`; a trailing `.` segment
/// also means directory intent (`src/.` -> `src/`). The root (`.`, `./`)
/// normalizes to `""`. Case and Unicode are preserved.
pub(crate) fn normalize_rel_path(path: &str) -> Result<String, PathIssue> {
    if path.is_empty() {
        return Err(PathIssue::Empty);
    }
    if path.chars().any(char::is_control) {
        return Err(PathIssue::ControlChar);
    }
    if path.contains('\\') {
        return Err(PathIssue::Backslash);
    }
    if path.starts_with('/') || has_drive_prefix(path) {
        return Err(PathIssue::Absolute);
    }

    let raw: Vec<&str> = path.split('/').collect();
    let directory = path.ends_with('/') || raw.last() == Some(&".");
    let mut kept: Vec<&str> = Vec::with_capacity(raw.len());
    for segment in raw {
        match segment {
            "" | "." => {}
            ".." => return Err(PathIssue::ParentSegment),
            other => kept.push(other),
        }
    }

    let mut out = kept.join("/");
    if directory && !out.is_empty() {
        out.push('/');
    }
    Ok(out)
}

/// `C:` style prefix (an ASCII letter followed by a colon).
fn has_drive_prefix(path: &str) -> bool {
    let mut chars = path.chars();
    matches!(
        (chars.next(), chars.next()),
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic()
    )
}

/// Put a normalized `(src, dest)` pair in canonical COPY shape.
///
/// A pair is a prefix pair when either side is empty or either side ends
/// with `/`; then every non-empty side gets a trailing `/`, so prefix
/// concatenation always lands on a segment boundary. Otherwise the pair is
/// exact and is returned unchanged.
pub(crate) fn canonical_copy_shape(mut src: String, mut dest: String) -> (String, String) {
    let prefix = src.is_empty() || dest.is_empty() || src.ends_with('/') || dest.ends_with('/');
    if prefix {
        for side in [&mut src, &mut dest] {
            if !side.is_empty() && !side.ends_with('/') {
                side.push('/');
            }
        }
    }
    (src, dest)
}

/// Check a COPY key after substitution: any non-empty text without control
/// characters. Keys never touch the filesystem, so there is no charset.
pub(crate) fn check_key(key: &str) -> Result<(), &'static str> {
    if key.is_empty() {
        return Err("key is empty");
    }
    if key.chars().any(char::is_control) {
        return Err("key contains a control character");
    }
    Ok(())
}

/// Transform and recipe names: `[A-Za-z0-9][A-Za-z0-9_.-]*`.
pub(crate) fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphanumeric() => {
            chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        }
        _ => false,
    }
}

/// Transform flag names (after the `--`): `[a-z0-9][a-z0-9-]*`.
pub(crate) fn is_flag_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() || first.is_ascii_digit() => {
            chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        }
        _ => false,
    }
}

/// ARG and variable names: `[A-Za-z_][A-Za-z0-9_]*`.
pub(crate) fn is_arg_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test 68: `.` segments drop, repeated slashes collapse, directory intent survives.
    #[test]
    fn normalize_collapses_dot_and_slashes() {
        let cases = [
            ("./src//app/", "src/app/"),
            ("src/./app", "src/app"),
            ("src/.", "src/"),
            ("src/app", "src/app"),
            ("a//b", "a/b"),
            ("${repo}/", "${repo}/"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                normalize_rel_path(input),
                Ok(expected.to_string()),
                "normalize({input:?})"
            );
        }
    }

    /// Test 69: every root form normalizes to the empty prefix.
    #[test]
    fn normalize_root_forms_to_empty() {
        for input in [".", "./", ".//", "./."] {
            assert_eq!(
                normalize_rel_path(input),
                Ok(String::new()),
                "normalize({input:?}) should be the root"
            );
        }
    }

    /// Test 70: each rejected form maps to its issue.
    #[test]
    fn normalize_rejects_bad_forms() {
        let cases = [
            ("", PathIssue::Empty),
            ("a\u{0}b", PathIssue::ControlChar),
            ("a\nb", PathIssue::ControlChar),
            ("a\tb", PathIssue::ControlChar),
            ("a\\b", PathIssue::Backslash),
            ("/abs", PathIssue::Absolute),
            ("C:x", PathIssue::Absolute),
            ("..", PathIssue::ParentSegment),
            ("a/../b", PathIssue::ParentSegment),
            ("${repo}/../x", PathIssue::ParentSegment),
        ];
        for (input, issue) in cases {
            assert_eq!(
                normalize_rel_path(input),
                Err(issue),
                "normalize({input:?})"
            );
        }
    }

    /// Test 71: keys accept any visible text; empty and control characters are rejected.
    #[test]
    fn key_rule_accepts_any_text_rejects_empty_and_control() {
        for key in [
            "all-files",
            "my key",
            "\u{43a}\u{43b}\u{44e}\u{447}",
            "${repo}-files",
            "a:b",
        ] {
            assert!(check_key(key).is_ok(), "key {key:?} should be accepted");
        }
        for key in ["", "a\tb", "a\u{7}"] {
            assert!(check_key(key).is_err(), "key {key:?} should be rejected");
        }
    }

    /// Test 72: every row of the spec's canonical-shape examples table.
    #[test]
    fn copy_shape_canonicalizes_prefix_pairs() {
        let cases = [
            (("", "${repo}/"), ("", "${repo}/")),
            (("", "out"), ("", "out/")),
            (("src", "dest/"), ("src/", "dest/")),
            (("src/", "dest"), ("src/", "dest/")),
            (("src", ""), ("src/", "")),
            (("README.md", ""), ("README.md/", "")),
            (("README.md", "docs/"), ("README.md/", "docs/")),
            (
                ("README.md", "docs/README.md"),
                ("README.md", "docs/README.md"),
            ),
        ];
        for ((src, dest), (want_src, want_dest)) in cases {
            assert_eq!(
                canonical_copy_shape(src.to_string(), dest.to_string()),
                (want_src.to_string(), want_dest.to_string()),
                "canonical_copy_shape({src:?}, {dest:?})"
            );
        }
    }
}
