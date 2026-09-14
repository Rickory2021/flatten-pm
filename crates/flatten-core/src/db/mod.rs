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

    /// Helper: create a seeded database, return tempfile and reader connection.
    fn test_reader() -> (tempfile::NamedTempFile, rusqlite::Connection) {
        let tmp = tempfile::NamedTempFile::new().expect("failed to create temp file");
        let _w = writer::Writer::open(tmp.path()).expect("Writer::open failed");
        let conn = open_reader(tmp.path()).expect("open_reader failed");
        (tmp, conn)
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
        let (_tmp, conn) = test_reader();

        let result = conn.execute(
            "INSERT INTO settings (key, value) VALUES ('test', 'val')",
            [],
        );
        assert!(result.is_err(), "read-only connection should reject writes");
    }
}
