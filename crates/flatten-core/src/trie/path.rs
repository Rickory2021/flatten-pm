// crates/flatten-core/src/trie/path.rs
//
// Trie path validation and OS path conversion.
//
// These are pure string/Path functions with no arena dependency.
// Used by trie operations and by ingest (path_from_os).

use super::error::{Error, Result};

/// Validate a trie path: relative, forward-slash separated, no `.`/`..`,
/// no empty segments, no leading/trailing slash.
pub fn validate_path(path: &str) -> Result<()> {
    if path.is_empty() {
        return Err(Error::InvalidPath("empty path".into()));
    }
    if path.starts_with('/') {
        return Err(Error::InvalidPath(format!("leading slash: {path}")));
    }
    if path.ends_with('/') {
        return Err(Error::InvalidPath(format!("trailing slash: {path}")));
    }
    for segment in path.split('/') {
        if segment.is_empty() {
            return Err(Error::InvalidPath(format!("empty segment in: {path}")));
        }
        if segment == "." {
            return Err(Error::InvalidPath(format!("'.' segment in: {path}")));
        }
        if segment == ".." {
            return Err(Error::InvalidPath(format!("'..' segment in: {path}")));
        }
    }
    Ok(())
}

/// Convert a `Path` to a trie-compatible `String`.
///
/// Iterates `std::path::Component` variants. `Normal` components are converted
/// lossily. `RootDir`, `ParentDir`, and `Prefix` return `Error::InvalidPath`.
/// Leading `CurDir` (from a `./` prefix) also returns `Error::InvalidPath`;
/// interior `.` segments are normalized away by `Path::components()`.
///
/// Returns `Ok((trie_path, was_lossy))`. Caller decides how to surface the
/// lossy warning.
pub fn path_from_os(path: &std::path::Path) -> Result<(String, bool)> {
    use std::path::Component;

    let mut parts = Vec::new();
    let mut was_lossy = false;

    for component in path.components() {
        match component {
            Component::Normal(os_str) => match os_str.to_str() {
                Some(s) => parts.push(s.to_string()),
                None => {
                    was_lossy = true;
                    parts.push(os_str.to_string_lossy().into_owned());
                }
            },
            Component::CurDir => {
                return Err(Error::InvalidPath(
                    "path contains leading '.' component".into(),
                ));
            }
            Component::ParentDir => {
                return Err(Error::InvalidPath(
                    "path contains '..' component".into(),
                ));
            }
            Component::RootDir => {
                return Err(Error::InvalidPath("path is absolute".into()));
            }
            Component::Prefix(_) => {
                return Err(Error::InvalidPath(
                    "path contains Windows prefix".into(),
                ));
            }
        }
    }

    if parts.is_empty() {
        return Err(Error::InvalidPath("empty path".into()));
    }

    Ok((parts.join("/"), was_lossy))
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- validate_path ---

    #[test]
    fn validate_path_valid() {
        assert!(validate_path("a").is_ok(), "single segment should be valid");
        assert!(validate_path("a/b/c").is_ok(), "multi-segment should be valid");
    }

    #[test]
    fn validate_path_empty() {
        assert!(
            matches!(validate_path(""), Err(Error::InvalidPath(_))),
            "empty path should be invalid"
        );
    }

    #[test]
    fn validate_path_leading_slash() {
        assert!(
            matches!(validate_path("/a"), Err(Error::InvalidPath(_))),
            "leading slash should be invalid"
        );
    }

    #[test]
    fn validate_path_trailing_slash() {
        assert!(
            matches!(validate_path("a/"), Err(Error::InvalidPath(_))),
            "trailing slash should be invalid"
        );
    }

    #[test]
    fn validate_path_double_slash() {
        assert!(
            matches!(validate_path("a//b"), Err(Error::InvalidPath(_))),
            "double slash should be invalid"
        );
    }

    #[test]
    fn validate_path_dot() {
        assert!(
            matches!(validate_path("./a"), Err(Error::InvalidPath(_))),
            "dot segment should be invalid"
        );
    }

    #[test]
    fn validate_path_dotdot() {
        assert!(
            matches!(validate_path("a/../b"), Err(Error::InvalidPath(_))),
            "dotdot segment should be invalid"
        );
    }

    // --- path_from_os ---

    #[cfg(unix)]
    #[test]
    fn lossy_path_conversion() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::path::Path;

        let os_str = OsStr::from_bytes(&[0xFF]);
        let path = Path::new(os_str);
        let (result, was_lossy) = path_from_os(path).expect("should not error");
        assert!(was_lossy, "non-UTF-8 should be flagged as lossy");
        assert!(
            result.contains('\u{FFFD}'),
            "lossy conversion should contain replacement character"
        );
    }

    #[test]
    fn path_from_os_joins_components() {
        let path = std::path::Path::new("a").join("b").join("c");
        let (result, was_lossy) = path_from_os(&path).expect("should succeed");
        assert_eq!(result, "a/b/c", "components should be joined with /");
        assert!(!was_lossy, "valid UTF-8 should not be lossy");
    }

    #[test]
    fn path_from_os_rejects_parent_dir() {
        let path = std::path::Path::new("../a");
        let result = path_from_os(path);
        assert!(
            matches!(result, Err(Error::InvalidPath(_))),
            ".. component should return InvalidPath"
        );
    }
}
