// crates/flatten-core/src/ingest/mod.rs
//
// Repo registration, ingest, and CRUD.
// See: Repos and Ingest Flow contracts in docs/design/2_INGEST.md.

pub mod error;
mod patterns;

pub use patterns::{import_gitignore, walk_and_hash};

#[cfg(test)]
mod test_util;

/// Result of an ingest operation.
#[derive(Debug, Clone)]
pub struct IngestReport {
    /// Number of leaf nodes in the trie (files ingested).
    /// Taken from `trie.list("").len()` after build.
    pub file_count: usize,
    /// Root hash of the built trie.
    pub root_hash: [u8; 32],
    /// Number of paths where `path_from_os` returned `was_lossy = true`.
    pub lossy_count: usize,
    /// Files matching extraction patterns (committed enrichments).
    /// Always 0 until extraction patterns exist (EX-006).
    pub enrichment_count: usize,
}

/// A `repos` table row, returned by list/get operations.
#[derive(Debug, serde::Serialize)]
pub struct RepoRow {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub ingest_patterns: Vec<String>,
    pub line_ending_policy: String,
    pub trie_updated_at: Option<String>,
    pub created_at: String,
    pub deleted_at: Option<String>,
}
