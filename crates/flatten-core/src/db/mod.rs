// crates/flatten-core/src/db/mod.rs
pub mod error;
mod migrations;
mod seed;
pub mod writer;

use error::{Error, Result};
use rusqlite::{Connection, OpenFlags};

/// Open a read-only connection to the database.
/// The writer must have run at least once to create the schema and set WAL mode.
pub fn open_reader(path: &std::path::Path) -> Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn: Connection = Connection::open_with_flags(path, flags)?;

    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    // Verify WAL mode
    let mode: String = conn.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    if mode.to_lowercase() != "wal" {
        return Err(Error::Writer(format!(
            "expected WAL journal mode, got '{mode}'"
        )));
    }

    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Helper: create a temp db with a Writer, return both.
    fn test_db() -> (tempfile::NamedTempFile, writer::Writer) {
        let tmp = tempfile::NamedTempFile::new().expect("failed to create temp file");
        let w = writer::Writer::open(tmp.path()).expect("Writer::open failed");
        (tmp, w)
    }

    /// Schema contains exactly 16 tables after initialization.
    #[test]
    fn schema_has_16_tables() {
        let (tmp, _writer) = test_db();

        let conn = open_reader(tmp.path()).expect("open_reader failed");
        let count: i32 = conn
            .prepare("SELECT count(*) FROM sqlite_master WHERE type='table'")
            .unwrap()
            .query_row([], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 16, "expected 16 tables, got {count}");
    }

    /// PRAGMA user_version is 1 after migration.
    #[test]
    fn user_version_is_1() {
        let (tmp, _writer) = test_db();

        let conn = open_reader(tmp.path()).expect("open_reader failed");
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1, "expected user_version 1, got {version}");
    }

    /// open_reader fails on a nonexistent database file.
    #[test]
    fn reader_missing_file_errors() {
        let result = open_reader(Path::new("/tmp/flatten-nonexistent-db-test.db"));
        assert!(result.is_err(), "open_reader should fail on missing file");
    }

    /// open_reader rejects a database that is not in WAL mode.
    #[test]
    fn reader_rejects_non_wal_db() {
        let tmp = tempfile::NamedTempFile::new().expect("failed to create temp file");

        // Create a raw database without setting WAL mode
        let conn = rusqlite::Connection::open(tmp.path()).unwrap();
        conn.execute("CREATE TABLE dummy (id INTEGER)", []).unwrap();
        drop(conn);

        let result = open_reader(tmp.path());
        assert!(
            result.is_err(),
            "open_reader should reject non-WAL database"
        );
    }

    /// open_reader connection rejects write statements.
    #[test]
    fn reader_rejects_writes() {
        let (tmp, _writer) = test_db();

        let conn = open_reader(tmp.path()).expect("open_reader failed");
        let result = conn.execute(
            "INSERT INTO settings (key, value) VALUES ('test', 'val')",
            [],
        );
        assert!(result.is_err(), "read-only connection should reject writes");
    }
}
