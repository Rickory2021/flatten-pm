// crates/flatten-core/src/ingest/mod.rs
//
// Repo registration, ingest, and CRUD.
// See: Repos and Ingest Flow contracts in docs/design/2_INGEST.md.

pub mod error;
mod patterns;

pub use patterns::{import_gitignore, walk_and_hash, walk_paths_filtered};

#[cfg(test)]
mod test_util;

use std::path::{Path, PathBuf};

use error::{Error, Result};
use crate::db;
use crate::trie;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

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

impl IngestReport {
    /// Format `root_hash` as a lowercase hex string (64 characters).
    pub fn root_hash_hex(&self) -> String {
        self.root_hash.iter().map(|b| format!("{b:02x}")).collect()
    }
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

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Validate a line ending policy string ("preserve" or "lf").
fn validate_line_ending_policy(value: &str) -> Result<()> {
    match value {
        "preserve" | "lf" => Ok(()),
        _ => Err(Error::InvalidLineEndingPolicy {
            value: value.to_string(),
        }),
    }
}

/// Create `{data_dir}/tries/` if missing.
fn ensure_tries_dir(data_dir: &Path) -> Result<()> {
    let tries_dir = data_dir.join("tries");
    std::fs::create_dir_all(&tries_dir).map_err(|e| Error::Io {
        context: format!("create tries dir: {}", tries_dir.display()),
        source: e,
    })
}

/// Trie file path for a repo: `{data_dir}/tries/{repo_id}.trie`.
fn trie_file_path(data_dir: &Path, repo_id: i64) -> PathBuf {
    data_dir.join("tries").join(format!("{repo_id}.trie"))
}

/// Build an `IngestReport` from a completed trie and the walk's lossy count.
fn build_report(trie: &trie::Trie, lossy_count: usize) -> IngestReport {
    IngestReport {
        file_count: trie.list("").len(),
        root_hash: trie.root_hash(),
        lossy_count,
        enrichment_count: 0,
    }
}

/// Update `repos.trie_updated_at` to now.
fn update_trie_timestamp(writer: &db::writer::Writer, repo_id: i64) -> Result<()> {
    let id = repo_id;
    writer.call_write(move |conn| {
        conn.execute(
            "UPDATE repos SET trie_updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
            [id],
        ).map_err(db::error::Error::from)?;
        Ok(())
    })?;
    Ok(())
}

/// Raw column tuple from the repos table. The JSON parse happens outside
/// the rusqlite callback to preserve the `Error::Json` variant on corrupt
/// `ingest_patterns`.
type RawRepoRow = (i64, String, String, String, String, Option<String>, String, Option<String>);

/// Extract a `RawRepoRow` from a rusqlite `Row`.
/// Used inside `query_row`/`query_map` callbacks.
fn row_to_raw(row: &rusqlite::Row) -> rusqlite::Result<RawRepoRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
    ))
}

/// Convert a `RawRepoRow` to `RepoRow`. Parses `ingest_patterns` JSON.
/// Returns `Error::Json` on corrupt JSON (not `Error::Database`).
fn raw_to_repo(raw: RawRepoRow) -> Result<RepoRow> {
    let (id, path, name, patterns_json, policy, trie_ts, created, deleted) = raw;
    let patterns: Vec<String> = serde_json::from_str(&patterns_json)?;
    Ok(RepoRow {
        id,
        path,
        name,
        ingest_patterns: patterns,
        line_ending_policy: policy,
        trie_updated_at: trie_ts,
        created_at: created,
        deleted_at: deleted,
    })
}

// ---------------------------------------------------------------------------
// Public API -- path and pattern helpers
// ---------------------------------------------------------------------------

/// Canonicalize a repo root path. Returns `NonExistentPath` if the path
/// doesn't exist or isn't a directory.
pub fn canonical_root(root: &Path) -> Result<PathBuf> {
    let canonical = std::fs::canonicalize(root).map_err(|_| Error::NonExistentPath {
        path: root.display().to_string(),
    })?;
    if !canonical.is_dir() {
        return Err(Error::NonExistentPath {
            path: root.display().to_string(),
        });
    }
    Ok(canonical)
}

/// Assemble the final pattern list from explicit patterns and optional
/// `.gitignore` import. Merge order: imported first, explicit after.
///
/// Takes an already-canonicalized root (from `canonical_root`). This split
/// preserves `register_repo`'s error ordering: canonicalize at step 1,
/// cheap checks (UTF-8, policy, name uniqueness) at steps 2-4, pattern
/// assembly at step 5.
pub fn assemble_patterns(
    canonical_root: &Path,
    explicit_patterns: &[String],
    import_gitignore: bool,
) -> Result<Vec<String>> {
    let mut final_patterns = if import_gitignore {
        patterns::import_gitignore(canonical_root)?
    } else {
        Vec::new()
    };
    final_patterns.extend(explicit_patterns.iter().cloned());
    Ok(final_patterns)
}

// ---------------------------------------------------------------------------
// Public API -- registration and ingest
// ---------------------------------------------------------------------------

/// Register a new repo.
///
/// Sequence:
///   1. `canonical_root(path)` -> `NonExistentPath` if fails
///   2. `path.to_str()` -> `NonExistentPath` if non-UTF-8 (can't store in TEXT)
///   3. `validate_line_ending_policy` (cheap local check before DB round-trip)
///   4. Pre-check name uniqueness (`SELECT`, no `deleted_at` filter)
///   5. `assemble_patterns` (optionally `import_gitignore`; merge: imported first, explicit after)
///   6. `walk_and_hash` -> `(leaves, lossy_count)` (validates patterns internally)
///   7. `Trie::from_leaves` -> trie
///   8. Insert `repos` row (UNIQUE is authoritative guard against races)
///   9. `ensure_tries_dir`, `trie.save`, update `trie_updated_at`
///
/// If `trie.save` fails after row insert (disk full), the repo has a row
/// and no trie. Self-healing: `load_or_reingest` -> `NotFound` -> rebuild.
pub fn register_repo(
    writer: &db::writer::Writer,
    data_dir: &Path,
    path: &Path,
    name: &str,
    explicit_patterns: &[String],
    line_ending_policy: &str,
    import_gitignore_flag: bool,
) -> Result<(i64, IngestReport)> {
    // 1. Canonicalize and validate directory
    let canonical = canonical_root(path)?;

    // 2. Ensure path is UTF-8 for TEXT storage
    let path_str = canonical
        .to_str()
        .ok_or_else(|| Error::NonExistentPath {
            path: format!("{} (non-UTF-8)", canonical.display()),
        })?
        .to_string();

    // 3. Validate line ending policy (cheap local check before DB round-trip)
    validate_line_ending_policy(line_ending_policy)?;

    // 4. Pre-check name uniqueness (fast fail; UNIQUE is the real guard)
    let name_for_check = name.to_string();
    let exists: bool = writer.call(move |conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM repos WHERE name = ?1",
            rusqlite::params![name_for_check],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    })?;
    if exists {
        return Err(Error::DuplicateName {
            name: name.to_string(),
        });
    }

    // 5. Assemble patterns (import first, explicit after)
    let final_patterns = assemble_patterns(&canonical, explicit_patterns, import_gitignore_flag)?;

    // 6. Walk and hash (validates patterns internally via build_matcher)
    let (leaves, lossy_count) = patterns::walk_and_hash(&canonical, &final_patterns)?;

    // 7. Build trie
    let trie = trie::Trie::from_leaves(leaves)?;

    // 8. Insert repos row
    let patterns_json = serde_json::to_string(&final_patterns)?;
    let policy = line_ending_policy.to_string();
    let path_for_insert = path_str;
    let name_for_insert = name.to_string();

    let repo_id = writer
        .call_write(move |conn| {
            conn.execute(
                "INSERT INTO repos (path, name, ingest_patterns, line_ending_policy) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![path_for_insert, name_for_insert, patterns_json, policy],
            )
            .map_err(db::error::Error::from)?;
            Ok(conn.last_insert_rowid())
        })
        .map_err(|e| {
            if let db::error::Error::RuSQLite(rusqlite::Error::SqliteFailure(ref err, _)) = e
                && err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
            {
                return Error::DuplicateName {
                    name: name.to_string(),
                };
            }
            Error::Database(e)
        })?;

    // 9. Save trie and update timestamp
    ensure_tries_dir(data_dir)?;
    let trie_path = trie_file_path(data_dir, repo_id);
    trie.save(&trie_path)?;
    update_trie_timestamp(writer, repo_id)?;

    Ok((repo_id, build_report(&trie, lossy_count)))
}

/// Re-ingest an existing repo: walk, rebuild trie, save.
///
/// Reads `ingest_patterns` and `path` from the repos row
/// (`WHERE id = ? AND deleted_at IS NULL`). Full re-hash. Idempotent.
///
/// Row lookup uses `query_row(..).optional()` inside the closure;
/// `None` mapped to `RepoNotFound` outside.
///
/// Returns the in-memory trie and the report. The design doc says
/// "produces the trie; the caller decides what to do with it."
pub fn reingest(
    writer: &db::writer::Writer,
    data_dir: &Path,
    repo_id: i64,
) -> Result<(trie::Trie, IngestReport)> {
    use rusqlite::OptionalExtension;

    // Read row
    let id = repo_id;
    let maybe_row = writer.call(move |conn| {
        conn.query_row(
            "SELECT path, ingest_patterns FROM repos WHERE id = ?1 AND deleted_at IS NULL",
            [id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(db::error::Error::from)
    })?;

    let (path_str, patterns_json) =
        maybe_row.ok_or_else(|| Error::RepoNotFound(format!("id {repo_id}")))?;

    let patterns: Vec<String> = serde_json::from_str(&patterns_json)?;
    let root = PathBuf::from(&path_str);

    // Walk and hash
    let (leaves, lossy_count) = patterns::walk_and_hash(&root, &patterns)?;

    // Build trie
    let trie = trie::Trie::from_leaves(leaves)?;

    // Save and update timestamp
    ensure_tries_dir(data_dir)?;
    let trie_path = trie_file_path(data_dir, repo_id);
    trie.save(&trie_path)?;
    update_trie_timestamp(writer, repo_id)?;

    let report = build_report(&trie, lossy_count);
    Ok((trie, report))
}

/// Load a trie from disk, or re-ingest if missing/corrupt.
///
/// Calls `Trie::load`. On recovery variants (`NotFound`, `Corrupt`,
/// `UnsupportedFormat`), calls `reingest` and returns the rebuilt trie
/// with `Some(report)`. On successful load, returns `None` for the report.
/// On `Io`, propagates.
///
/// Single implementation of the design contract:
/// "callers match `NotFound | Corrupt | UnsupportedFormat` to re-ingest."
///
/// Note: the load path does no DB read, so a soft-deleted repo with a
/// surviving trie file loads fine. Every caller (CLI via `get_repo_by_name`,
/// export/watch via resolved ids) pre-validates. Don't add a query here.
pub fn load_or_reingest(
    writer: &db::writer::Writer,
    data_dir: &Path,
    repo_id: i64,
) -> Result<(trie::Trie, Option<IngestReport>)> {
    let trie_path = trie_file_path(data_dir, repo_id);

    match trie::Trie::load(&trie_path) {
        Ok(trie) => Ok((trie, None)),
        Err(trie::error::Error::NotFound { .. })
        | Err(trie::error::Error::Corrupt { .. })
        | Err(trie::error::Error::UnsupportedFormat { .. }) => {
            let (trie, report) = reingest(writer, data_dir, repo_id)?;
            Ok((trie, Some(report)))
        }
        Err(e) => Err(Error::Trie(e)),
    }
}

// ---------------------------------------------------------------------------
// Public API -- repo CRUD
// ---------------------------------------------------------------------------

/// List all non-deleted repos.
pub fn list_repos(conn: &rusqlite::Connection) -> Result<Vec<RepoRow>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, path, name, ingest_patterns, line_ending_policy, \
             trie_updated_at, created_at, deleted_at \
             FROM repos WHERE deleted_at IS NULL ORDER BY name",
        )
        .map_err(db::error::Error::from)?;

    let rows = stmt
        .query_map([], row_to_raw)
        .map_err(db::error::Error::from)?;

    let mut result = Vec::new();
    for row in rows {
        let raw = row.map_err(db::error::Error::from)?;
        result.push(raw_to_repo(raw)?);
    }
    Ok(result)
}

/// Get a repo by name. Returns `RepoNotFound` if missing or soft-deleted.
pub fn get_repo_by_name(conn: &rusqlite::Connection, name: &str) -> Result<RepoRow> {
    use rusqlite::OptionalExtension;

    let raw = conn
        .query_row(
            "SELECT id, path, name, ingest_patterns, line_ending_policy, \
             trie_updated_at, created_at, deleted_at \
             FROM repos WHERE name = ?1 AND deleted_at IS NULL",
            [name],
            row_to_raw,
        )
        .optional()
        .map_err(db::error::Error::from)?
        .ok_or_else(|| Error::RepoNotFound(name.to_string()))?;

    raw_to_repo(raw)
}

/// Get a repo by ID. Returns `RepoNotFound` if missing or soft-deleted.
pub fn get_repo(conn: &rusqlite::Connection, repo_id: i64) -> Result<RepoRow> {
    use rusqlite::OptionalExtension;

    let raw = conn
        .query_row(
            "SELECT id, path, name, ingest_patterns, line_ending_policy, \
             trie_updated_at, created_at, deleted_at \
             FROM repos WHERE id = ?1 AND deleted_at IS NULL",
            [repo_id],
            row_to_raw,
        )
        .optional()
        .map_err(db::error::Error::from)?
        .ok_or_else(|| Error::RepoNotFound(format!("id {repo_id}")))?;

    raw_to_repo(raw)
}

/// Soft-delete a repo (set `deleted_at`). Returns `RepoNotFound` if the
/// id doesn't exist or is already deleted.
///
/// Trie file left on disk; s2 reaps orphaned files.
///
/// [DEFERRED: DA-005] Active binding guard: soft-deleting a repo with
/// active bindings is a domain error naming the bindings.
pub fn soft_delete_repo(writer: &db::writer::Writer, repo_id: i64) -> Result<()> {
    let id = repo_id;
    let changed = writer.call_write(move |conn| {
        let rows = conn
            .execute(
                "UPDATE repos SET deleted_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                 WHERE id = ?1 AND deleted_at IS NULL",
                [id],
            )
            .map_err(db::error::Error::from)?;
        Ok(rows)
    })?;

    if changed == 0 {
        return Err(Error::RepoNotFound(format!("id {repo_id}")));
    }
    Ok(())
}

/// Edit repo fields (name, patterns, line_ending_policy).
///
/// Only updates fields where the `Option` is `Some`. Uses `COALESCE`
/// for partial update. Re-ingests after any edit (including pure rename).
///
/// Sequence: validate policy -> validate patterns via `build_matcher` ->
/// `UPDATE` with `COALESCE` -> `reingest`.
pub fn edit_repo(
    writer: &db::writer::Writer,
    data_dir: &Path,
    repo_id: i64,
    new_name: Option<&str>,
    new_patterns: Option<&[String]>,
    new_line_ending_policy: Option<&str>,
) -> Result<(trie::Trie, IngestReport)> {
    use rusqlite::OptionalExtension;

    // Validate line ending policy if provided
    if let Some(policy) = new_line_ending_policy {
        validate_line_ending_policy(policy)?;
    }

    // If new patterns, validate via build_matcher (need repo path for root)
    if let Some(pats) = new_patterns {
        let id = repo_id;
        let maybe_path = writer.call(move |conn| {
            conn.query_row(
                "SELECT path FROM repos WHERE id = ?1 AND deleted_at IS NULL",
                [id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(db::error::Error::from)
        })?;
        let path_str =
            maybe_path.ok_or_else(|| Error::RepoNotFound(format!("id {repo_id}")))?;
        let root = PathBuf::from(&path_str);
        patterns::build_matcher(&root, pats)?;
    }

    // UPDATE with COALESCE
    let new_name_owned = new_name.map(|s| s.to_string());
    let new_patterns_json = new_patterns
        .map(serde_json::to_string)
        .transpose()?;
    let new_policy_owned = new_line_ending_policy.map(|s| s.to_string());
    let name_for_err = new_name.unwrap_or("").to_string();

    let id = repo_id;
    let changed = writer
        .call_write(move |conn| {
            let rows = conn
                .execute(
                    "UPDATE repos SET \
                     name = COALESCE(?1, name), \
                     ingest_patterns = COALESCE(?2, ingest_patterns), \
                     line_ending_policy = COALESCE(?3, line_ending_policy) \
                     WHERE id = ?4 AND deleted_at IS NULL",
                    rusqlite::params![new_name_owned, new_patterns_json, new_policy_owned, id],
                )
                .map_err(db::error::Error::from)?;
            Ok(rows)
        })
        .map_err(|e| {
            if let db::error::Error::RuSQLite(rusqlite::Error::SqliteFailure(ref err, _)) = e
                && err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
            {
                return Error::DuplicateName {
                    name: name_for_err,
                };
            }
            Error::Database(e)
        })?;

    if changed == 0 {
        return Err(Error::RepoNotFound(format!("id {repo_id}")));
    }

    // Reingest with updated config
    reingest(writer, data_dir, repo_id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::ingest::test_util::{test_db, test_dir_with_files};

    /// Helper: open a reader on the test DB.
    fn test_reader(data_dir: &Path) -> rusqlite::Connection {
        db::open_reader(&data_dir.join("flatten.db")).expect("open_reader failed")
    }

    // --- Registration tests (1-9) ---

    #[test]
    fn register_repo_inserts_row() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("src/main.rs", "fn main() {}")]);
        let writer = test_db(data_dir.path());

        let (id, report) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "test-repo",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        assert!(id > 0, "repo id should be positive");
        assert_eq!(report.file_count, 1, "should have 1 file");

        // Read back row
        let conn = test_reader(data_dir.path());
        let name: String = conn
            .query_row("SELECT name FROM repos WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .expect("row should exist");
        assert_eq!(name, "test-repo", "name should match");
    }

    #[test]
    fn register_repo_stores_absolute_path() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        let (id, _) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "abs-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        let conn = test_reader(data_dir.path());
        let stored_path: String = conn
            .query_row("SELECT path FROM repos WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .expect("row should exist");

        // On macOS, tempdir is /var/... but canonicalize yields /private/var/...
        let expected = repo_dir
            .path()
            .canonicalize()
            .expect("canonicalize should succeed");
        assert_eq!(
            stored_path,
            expected.to_str().unwrap(),
            "stored path should be the canonical absolute path"
        );
    }

    #[test]
    fn register_repo_duplicate_name_error() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "dup-name",
            &[],
            "preserve",
            false,
        )
        .expect("first register should succeed");

        let result = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "dup-name",
            &[],
            "preserve",
            false,
        );

        assert!(
            matches!(result, Err(Error::DuplicateName { .. })),
            "second register with same name should return DuplicateName, got: {result:?}"
        );
    }

    #[test]
    fn register_repo_nonexistent_path_error() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let writer = test_db(data_dir.path());

        let result = register_repo(
            &writer,
            data_dir.path(),
            Path::new("/nonexistent/path/that/does/not/exist"),
            "ghost",
            &[],
            "preserve",
            false,
        );

        assert!(
            matches!(result, Err(Error::NonExistentPath { .. })),
            "nonexistent path should return NonExistentPath, got: {result:?}"
        );
    }

    #[test]
    fn register_repo_not_a_directory_error() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        // Point at a file, not a directory
        let file_path = repo_dir.path().join("a.txt");
        let result = register_repo(
            &writer,
            data_dir.path(),
            &file_path,
            "not-a-dir",
            &[],
            "preserve",
            false,
        );

        assert!(
            matches!(result, Err(Error::NonExistentPath { .. })),
            "file path should return NonExistentPath, got: {result:?}"
        );
    }

    #[test]
    fn register_repo_writes_trie_file() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("src/main.rs", "fn main() {}")]);
        let writer = test_db(data_dir.path());

        let (id, _) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "trie-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        let trie_path = data_dir.path().join("tries").join(format!("{id}.trie"));
        assert!(
            trie_path.exists(),
            "trie file should exist at {}", trie_path.display()
        );
    }

    #[test]
    fn register_repo_sets_trie_updated_at() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        let (id, _) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "ts-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        let conn = test_reader(data_dir.path());
        let ts: Option<String> = conn
            .query_row(
                "SELECT trie_updated_at FROM repos WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .expect("row should exist");

        assert!(
            ts.is_some(),
            "trie_updated_at should be set after registration"
        );
    }

    #[test]
    fn register_repo_invalid_pattern_error() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        // `[z-a]` is an inverted character class range that globset rejects.
        let result = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "bad-pattern",
            &["[z-a]".to_string()],
            "preserve",
            false,
        );

        assert!(
            matches!(result, Err(Error::InvalidPattern { .. })),
            "inverted range [z-a] should return InvalidPattern, got: {result:?}"
        );
    }

    #[test]
    fn register_repo_invalid_line_ending_policy() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        let result = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "bad-policy",
            &[],
            "crlf",
            false,
        );

        assert!(
            matches!(result, Err(Error::InvalidLineEndingPolicy { .. })),
            "invalid policy should return InvalidLineEndingPolicy, got: {result:?}"
        );
    }

    // --- Reingest tests (33-41) ---

    #[test]
    fn reingest_idempotent() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello"), ("b.txt", "world")]);
        let writer = test_db(data_dir.path());

        let (id, report1) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "idem-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        let (_, report2) = reingest(&writer, data_dir.path(), id).expect("reingest should succeed");

        assert_eq!(
            report1.root_hash, report2.root_hash,
            "root hash should be identical on reingest of unchanged repo"
        );
        assert_eq!(
            report1.file_count, report2.file_count,
            "file count should be identical"
        );
    }

    #[test]
    fn reingest_detects_change() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        let (id, report1) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "change-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        // Add a new file
        std::fs::write(repo_dir.path().join("b.txt"), "new file").expect("write should succeed");

        let (_, report2) = reingest(&writer, data_dir.path(), id).expect("reingest should succeed");

        assert_ne!(
            report1.root_hash, report2.root_hash,
            "root hash should change after adding a file"
        );
        assert_eq!(
            report2.file_count,
            report1.file_count + 1,
            "file count should increase by 1"
        );
    }

    #[test]
    fn reingest_unknown_id_error() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let writer = test_db(data_dir.path());

        let result = reingest(&writer, data_dir.path(), 99999);

        assert!(
            matches!(result, Err(Error::RepoNotFound(_))),
            "unknown id should return RepoNotFound, got: {result:?}"
        );
    }

    #[test]
    fn reingest_path_gone_error() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        let (id, _) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "gone-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        // Delete the repo directory
        drop(repo_dir);

        let result = reingest(&writer, data_dir.path(), id);

        assert!(
            matches!(result, Err(Error::NonExistentPath { .. })),
            "deleted path should return NonExistentPath, got: {result:?}"
        );
    }

    #[test]
    fn reingest_returns_trie() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[
            ("a.txt", "hello"),
            ("b.txt", "world"),
            ("sub/c.txt", "nested"),
        ]);
        let writer = test_db(data_dir.path());

        let (id, _) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "trie-return-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        let (trie, report) =
            reingest(&writer, data_dir.path(), id).expect("reingest should succeed");

        assert_eq!(
            trie.list("").len(),
            3,
            "trie should have 3 leaves"
        );
        assert_eq!(
            report.file_count, 3,
            "report should match trie leaf count"
        );
    }

    #[test]
    fn reingest_soft_deleted_not_found() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        let (id, _) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "soft-del-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        // Soft-delete via raw SQL (soft_delete_repo is in chunk 4)
        let del_id = id;
        writer
            .call_write(move |conn| {
                conn.execute(
                    "UPDATE repos SET deleted_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                    [del_id],
                )
                .map_err(db::error::Error::from)?;
                Ok(())
            })
            .expect("soft delete should succeed");

        let result = reingest(&writer, data_dir.path(), id);

        assert!(
            matches!(result, Err(Error::RepoNotFound(_))),
            "reingest on soft-deleted repo should return RepoNotFound, got: {result:?}"
        );
    }

    #[test]
    fn load_or_reingest_corrupt_trie() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        let (id, _) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "corrupt-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        // Truncate the trie file
        let trie_path = data_dir.path().join("tries").join(format!("{id}.trie"));
        std::fs::write(&trie_path, b"").expect("truncate should succeed");

        let (trie, report) =
            load_or_reingest(&writer, data_dir.path(), id).expect("recovery should succeed");

        assert!(
            report.is_some(),
            "report should be Some (re-ingest ran)"
        );
        assert_eq!(
            trie.list("").len(),
            1,
            "recovered trie should have 1 leaf"
        );
    }

    #[test]
    fn load_or_reingest_missing_trie() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        let (id, _) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "missing-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        // Delete the trie file
        let trie_path = data_dir.path().join("tries").join(format!("{id}.trie"));
        std::fs::remove_file(&trie_path).expect("delete should succeed");

        let (trie, report) =
            load_or_reingest(&writer, data_dir.path(), id).expect("recovery should succeed");

        assert!(
            report.is_some(),
            "report should be Some (re-ingest ran)"
        );
        assert_eq!(
            trie.list("").len(),
            1,
            "recovered trie should have 1 leaf"
        );
    }

    #[test]
    fn load_or_reingest_valid_trie() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        let (id, _) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "valid-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        let (trie, report) =
            load_or_reingest(&writer, data_dir.path(), id).expect("load should succeed");

        assert!(
            report.is_none(),
            "report should be None (loaded from file, no re-ingest)"
        );
        assert_eq!(
            trie.list("").len(),
            1,
            "loaded trie should have 1 leaf"
        );
    }

    // --- CRUD tests (42-52) ---

    #[test]
    fn list_repos_returns_registered() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "list-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        let conn = test_reader(data_dir.path());
        let repos = list_repos(&conn).expect("list should succeed");

        assert!(
            repos.iter().any(|r| r.name == "list-test"),
            "registered repo should appear in list"
        );
    }

    #[test]
    fn list_repos_excludes_deleted() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        let (id, _) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "del-list-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        soft_delete_repo(&writer, id).expect("soft delete should succeed");

        let conn = test_reader(data_dir.path());
        let repos = list_repos(&conn).expect("list should succeed");

        assert!(
            !repos.iter().any(|r| r.name == "del-list-test"),
            "soft-deleted repo should not appear in list"
        );
    }

    #[test]
    fn get_repo_by_name_found_and_not_found() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "get-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        let conn = test_reader(data_dir.path());

        // Happy path
        let row = get_repo_by_name(&conn, "get-test").expect("get should succeed");
        assert_eq!(row.name, "get-test", "name should match");
        assert_eq!(
            row.line_ending_policy, "preserve",
            "policy should match"
        );

        // Error path
        let result = get_repo_by_name(&conn, "nonexistent");
        assert!(
            matches!(result, Err(Error::RepoNotFound(_))),
            "unknown name should return RepoNotFound, got: {result:?}"
        );
    }

    #[test]
    fn soft_delete_sets_deleted_at() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        let (id, _) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "soft-del-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        soft_delete_repo(&writer, id).expect("soft delete should succeed");

        let conn = test_reader(data_dir.path());
        let deleted_at: Option<String> = conn
            .query_row(
                "SELECT deleted_at FROM repos WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .expect("row should exist");

        assert!(
            deleted_at.is_some(),
            "deleted_at should be set after soft delete"
        );
    }

    #[test]
    fn soft_delete_unknown_not_found() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let writer = test_db(data_dir.path());

        let result = soft_delete_repo(&writer, 99999);

        assert!(
            matches!(result, Err(Error::RepoNotFound(_))),
            "unknown id should return RepoNotFound, got: {result:?}"
        );
    }

    #[test]
    fn edit_repo_renames() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        let (id, _) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "old-name",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        edit_repo(&writer, data_dir.path(), id, Some("new-name"), None, None)
            .expect("edit should succeed");

        let conn = test_reader(data_dir.path());

        // Old name gone
        let old = get_repo_by_name(&conn, "old-name");
        assert!(
            matches!(old, Err(Error::RepoNotFound(_))),
            "old name should not be found"
        );

        // New name works
        let new = get_repo_by_name(&conn, "new-name").expect("new name should be found");
        assert_eq!(new.id, id, "id should be unchanged after rename");
    }

    #[test]
    fn edit_repo_rename_collision() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_a = test_dir_with_files(&[("a.txt", "hello")]);
        let repo_b = test_dir_with_files(&[("b.txt", "world")]);
        let writer = test_db(data_dir.path());

        let (id_a, _) = register_repo(
            &writer,
            data_dir.path(),
            repo_a.path(),
            "repo-a",
            &[],
            "preserve",
            false,
        )
        .expect("register a should succeed");

        register_repo(
            &writer,
            data_dir.path(),
            repo_b.path(),
            "repo-b",
            &[],
            "preserve",
            false,
        )
        .expect("register b should succeed");

        // Try to rename a to b's name
        let result = edit_repo(
            &writer,
            data_dir.path(),
            id_a,
            Some("repo-b"),
            None,
            None,
        );

        assert!(
            matches!(result, Err(Error::DuplicateName { .. })),
            "rename collision should return DuplicateName, got: {result:?}"
        );
    }

    #[test]
    fn edit_repo_updates_patterns_reingests() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[
            ("a.txt", "hello"),
            ("b.log", "log entry"),
        ]);
        let writer = test_db(data_dir.path());

        let (id, report1) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "edit-pat-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        assert_eq!(report1.file_count, 2, "both files initially ingested");

        // Add *.log exclusion
        let (_, report2) = edit_repo(
            &writer,
            data_dir.path(),
            id,
            None,
            Some(&["*.log".to_string()]),
            None,
        )
        .expect("edit should succeed");

        assert_eq!(
            report2.file_count, 1,
            "after adding *.log pattern, only a.txt should remain"
        );
        assert_ne!(
            report1.root_hash, report2.root_hash,
            "root hash should change after pattern edit"
        );
    }

    #[test]
    fn edit_repo_validates_before_storing() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        let (id, _) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "val-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        // Invalid line ending policy should fail without changing the row
        let result = edit_repo(
            &writer,
            data_dir.path(),
            id,
            None,
            None,
            Some("crlf"),
        );

        assert!(
            matches!(result, Err(Error::InvalidLineEndingPolicy { .. })),
            "invalid policy should error, got: {result:?}"
        );

        // Verify row is unchanged
        let conn = test_reader(data_dir.path());
        let policy: String = conn
            .query_row(
                "SELECT line_ending_policy FROM repos WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .expect("row should exist");
        assert_eq!(
            policy, "preserve",
            "policy should be unchanged after failed edit"
        );
    }

    #[test]
    fn list_repos_corrupt_json_error() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        let (id, _) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "corrupt-json-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        // Corrupt the ingest_patterns column
        let corrupt_id = id;
        writer
            .call_write(move |conn| {
                conn.execute(
                    "UPDATE repos SET ingest_patterns = 'not-valid-json' WHERE id = ?1",
                    [corrupt_id],
                )
                .map_err(db::error::Error::from)?;
                Ok(())
            })
            .expect("corrupt should succeed");

        let conn = test_reader(data_dir.path());
        let result = list_repos(&conn);

        assert!(
            matches!(result, Err(Error::Json(_))),
            "corrupt JSON should return Json error, got: {result:?}"
        );
    }

    #[test]
    fn stored_patterns_is_ssot() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[
            (".gitignore", "*.log\n"),
            ("a.txt", "hello"),
            ("debug.log", "log entry"),
        ]);
        let writer = test_db(data_dir.path());

        // Register with gitignore import: *.log is imported, debug.log excluded
        let (id, report1) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "ssot-test",
            &[],
            "preserve",
            true,
        )
        .expect("register should succeed");

        // .gitignore + a.txt (debug.log excluded by *.log)
        assert_eq!(
            report1.file_count, 2,
            "initial: .gitignore and a.txt (debug.log excluded)"
        );

        // (a) Edit the on-disk .gitignore: change to *.txt
        std::fs::write(repo_dir.path().join(".gitignore"), "*.txt\n")
            .expect("write should succeed");

        // Reingest: reads STORED patterns (["*.log"]), not the disk .gitignore.
        // debug.log should still be excluded (stored *.log still applies).
        // a.txt should still be included (stored patterns don't exclude it).
        // Root hash WILL differ because .gitignore content changed, but the
        // same set of files is included, proving SSOT.
        let (trie2, report2) =
            reingest(&writer, data_dir.path(), id).expect("reingest should succeed");

        assert_eq!(
            report2.file_count, 2,
            "after .gitignore edit: still 2 files (.gitignore + a.txt; debug.log still excluded by stored *.log)"
        );
        assert!(
            trie2.has("a.txt"),
            "a.txt should still be included (stored patterns don't exclude *.txt)"
        );
        assert!(
            !trie2.has("debug.log"),
            "debug.log should still be excluded (stored *.log is SSOT)"
        );

        // (b) Edit stored patterns: change to *.txt
        let (_, report3) = edit_repo(
            &writer,
            data_dir.path(),
            id,
            None,
            Some(&["*.txt".to_string()]),
            None,
        )
        .expect("edit should succeed");

        assert_ne!(
            report1.root_hash, report3.root_hash,
            "root hash should change after editing stored patterns"
        );
        // Now .gitignore + debug.log (a.txt excluded by *.txt)
        assert_eq!(
            report3.file_count, 2,
            "after *.txt: .gitignore and debug.log (a.txt excluded)"
        );
    }

    // --- New tests for APP-003 ---

    #[test]
    fn get_repo_returns_row() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        let (id, _) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "get-id-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        let conn = test_reader(data_dir.path());
        let row = get_repo(&conn, id).expect("get_repo should succeed");

        assert_eq!(row.id, id, "id should match");
        assert_eq!(row.name, "get-id-test", "name should match");
        assert_eq!(row.line_ending_policy, "preserve", "policy should match");
    }

    #[test]
    fn get_repo_not_found() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let writer = test_db(data_dir.path());
        // Need to initialize DB even if no repos
        let _ = writer;

        let conn = test_reader(data_dir.path());
        let result = get_repo(&conn, 99999);

        assert!(
            matches!(result, Err(Error::RepoNotFound(_))),
            "missing ID should return RepoNotFound, got: {result:?}"
        );
    }

    #[test]
    fn get_repo_skips_deleted() {
        let data_dir = tempfile::TempDir::new().unwrap();
        let repo_dir = test_dir_with_files(&[("a.txt", "hello")]);
        let writer = test_db(data_dir.path());

        let (id, _) = register_repo(
            &writer,
            data_dir.path(),
            repo_dir.path(),
            "del-get-test",
            &[],
            "preserve",
            false,
        )
        .expect("register should succeed");

        soft_delete_repo(&writer, id).expect("soft delete should succeed");

        let conn = test_reader(data_dir.path());
        let result = get_repo(&conn, id);

        assert!(
            matches!(result, Err(Error::RepoNotFound(_))),
            "soft-deleted repo should return RepoNotFound, got: {result:?}"
        );
    }

    #[test]
    fn assemble_patterns_merge_order() {
        let dir = test_dir_with_files(&[
            (".gitignore", "from_git\n"),
            ("a.txt", "hello"),
        ]);

        let canonical = canonical_root(dir.path()).expect("canonical_root");
        let result = assemble_patterns(&canonical, &["explicit".to_string()], true)
            .expect("assemble should succeed");

        // Imported patterns first, explicit after
        assert_eq!(result.len(), 2, "should have 2 patterns");
        assert_eq!(result[0], "from_git", "imported pattern first");
        assert_eq!(result[1], "explicit", "explicit pattern second");
    }

    #[test]
    fn assemble_patterns_no_import() {
        let dir = test_dir_with_files(&[
            (".gitignore", "from_git\n"),
            ("a.txt", "hello"),
        ]);

        let canonical = canonical_root(dir.path()).expect("canonical_root");
        let result = assemble_patterns(&canonical, &["only_this".to_string()], false)
            .expect("assemble should succeed");

        assert_eq!(result, vec!["only_this"], "import=false returns only explicit");
    }

    #[test]
    fn canonical_root_nonexistent() {
        let result = canonical_root(Path::new("/nonexistent/path/abc"));
        assert!(
            matches!(result, Err(Error::NonExistentPath { .. })),
            "nonexistent path returns NonExistentPath, got: {result:?}"
        );
    }

    #[test]
    fn canonical_root_file_not_dir() {
        let dir = test_dir_with_files(&[("a.txt", "hello")]);
        let file_path = dir.path().join("a.txt");
        let result = canonical_root(&file_path);
        assert!(
            matches!(result, Err(Error::NonExistentPath { .. })),
            "file path returns NonExistentPath, got: {result:?}"
        );
    }

    #[test]
    fn root_hash_hex_format() {
        let report = IngestReport {
            file_count: 0,
            root_hash: [0xab; 32],
            lossy_count: 0,
            enrichment_count: 0,
        };
        let hex = report.root_hash_hex();
        assert_eq!(hex.len(), 64, "hex string should be 64 chars");
        assert!(
            hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "hex string should be lowercase hex, got: {hex}"
        );
        assert_eq!(&hex[..4], "abab", "first two bytes should be 'abab'");
    }
}
