// crates/flatten-core/src/db/error.rs
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Rust SQLite Error: {0}")]
    RuSQLite(#[from] rusqlite::Error),

    #[error("Writer Error: {0}")]
    Writer(String),

    #[error("Job Panicked: {0}")]
    Panicked(String),
}

pub type Result<T> = std::result::Result<T, Error>;
