// crates/flatten-core/src/trie/error.rs
//
// Error types for the trie module. Each variant carries enough context
// to diagnose without unwinding the call stack.

/// Trie operation errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error at {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },

    #[error("corrupt trie file at {path}: {reason}")]
    Corrupt { path: String, reason: String },

    #[error("unsupported trie format version {version} at {path}")]
    UnsupportedFormat { path: String, version: u32 },

    #[error("trie file not found: {path}")]
    NotFound { path: String },

    #[error("invalid trie path: {0}")]
    InvalidPath(String),

    #[error("arena capacity exceeded (>= 2^32 nodes)")]
    ArenaFull,
}

/// Convenience alias for trie operations.
pub type Result<T> = std::result::Result<T, Error>;
