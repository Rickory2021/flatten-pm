// crates/flatten-core/src/ingest/test_util.rs
//
// Shared test helpers for the ingest module.
// Used by patterns::tests and mod::tests.

use std::path::Path;

/// Create a temp directory populated with the given files.
/// Each entry is `(relative_path, content)`. Parent directories
/// are created automatically via `create_dir_all`.
pub(crate) fn test_dir_with_files(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().expect("failed to create tempdir");
    for (path, content) in files {
        let full = dir.path().join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("failed to create parent dirs");
        }
        std::fs::write(&full, content).expect("failed to write file");
    }
    dir
}

/// RAII guard that restores file permissions on drop.
///
/// After setting restrictive permissions (e.g. `chmod 000`), call
/// `can_still_read` to detect whether the test is running as root.
/// If root, the restriction is ineffective and the test should
/// return early rather than produce a false pass.
#[cfg(unix)]
pub(crate) struct RestorePerms {
    pub path: std::path::PathBuf,
    pub perms: std::fs::Permissions,
}

#[cfg(unix)]
impl RestorePerms {
    /// Create a guard and set the given permissions on `path`.
    /// Returns the guard (which restores original perms on drop).
    pub fn set(path: &Path, mode: u32) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let original = std::fs::metadata(path)
            .expect("failed to read metadata")
            .permissions();
        let restrictive = std::fs::Permissions::from_mode(mode);
        std::fs::set_permissions(path, restrictive).expect("failed to set permissions");
        RestorePerms {
            path: path.to_path_buf(),
            perms: original,
        }
    }

    /// Returns true if the restricted path is still readable (running as root).
    /// Caller should return early from the test if true.
    pub fn can_still_read(&self) -> bool {
        std::fs::read_dir(&self.path).is_ok()
    }
}

#[cfg(unix)]
impl Drop for RestorePerms {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.path, self.perms.clone());
    }
}
