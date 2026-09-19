// crates/flatten-core/src/ingest/error.rs
//
// Error types for the ingest module.
// See: Error Model in docs/design/1_INFRASTRUCTURE.md.

/// Ingest operation errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("path does not exist or is not a directory: {path}")]
    NonExistentPath { path: String },

    #[error("duplicate repo name: {name} (including soft-deleted repos)")]
    DuplicateName { name: String },

    #[error("repo not found: {0}")]
    RepoNotFound(String),

    #[error("invalid pattern {pattern:?}: {source}")]
    InvalidPattern {
        pattern: String,
        source: ignore::Error,
    },

    #[error("invalid line ending policy {value:?}: expected \"preserve\" or \"lf\"")]
    InvalidLineEndingPolicy { value: String },

    #[error("ingest IO error: {context}: {source}")]
    Io {
        context: String,
        source: std::io::Error,
    },

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Trie(#[from] crate::trie::error::Error),

    #[error(transparent)]
    Database(#[from] crate::db::error::Error),
}

/// Convenience alias for ingest operations.
pub type Result<T> = std::result::Result<T, Error>;
