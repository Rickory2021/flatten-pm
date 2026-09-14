// crates/flatten-core/src/db/writer.rs
//
// Writer serializes all write access to a single rusqlite::Connection through a dedicated OS thread.
//
// open() creates an mpsc::channel<Job> and spawns an OS thread (std::thread::spawn)
//  that owns the write Connection and loops: `for job in rx { job(&mut conn) }`.
//
// Writer.sender: Sender<Job>, the write end of the job channel.
//   Callers push Jobs into it. The thread pops them from the other end.
// Writer.handle: JoinHandle<()>, a leash to the spawned thread.
//   Calling .join() on it blocks until the thread exits. Used by Drop for clean shutdown.
//
//  writer.call(f)                            Write Worker (Owns Connection)
//       |                                                |
//       | 1.  creates Job with f and a reply channel     |
//       | 2. Send Job via Sender<Job>  --------------->  |
//       |                                                | 3. run function with connection f(&mut conn)
//       |  <---  4. Send Result<T> Reply via reply_tx    |
//
// Read-only access uses separate Connections via open_reader() in mod.rs.

use std::path::Path;
use std::sync::mpsc;
use std::thread;

use rusqlite::Connection;

use super::error::{Error, Result};
use super::migrations::MIGRATIONS;
use super::seed;

// RuSQLite Connection closure sent from the caller to the worker thread via the job channel.
type Job = Box<dyn FnOnce(&mut Connection) + Send>;

pub struct Writer {
    // Fields are Optional solely for ordered shutdown in impl Drop for Writer
    // Normally they are always Some
    sender: Option<mpsc::Sender<Job>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Writer {
    /// Open, migrate, seed, and spawn the writer thread.
    pub fn open(path: &Path) -> Result<Writer> {
        // 1. Opens the connection
        let mut conn: Connection = Connection::open(path)?;

        // 2. PRAGMA
        // Best practices on connection:
        // - busy_timeout set before migration
        // - journal_mode should set WAL before migration
        // - foreign_keys should be toggled off before migration and on after migration
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| {
            let mode: String = row.get(0)?;
            if mode.to_lowercase() != "wal" {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            Ok(())
        })
        .map_err(|e| Error::Writer(format!("failed to set WAL mode: {e}")))?;

        // 3. Prepare for migration
        // BEGIN IMMEDIATE: grab the write lock up front to prevent deadlock
        conn.set_transaction_behavior(rusqlite::TransactionBehavior::Immediate);

        // FK OFF during migration (table-rewriting migrations need this)
        conn.pragma_update(None, "foreign_keys", "OFF")?;

        // 4. Run schema migrations (no-op if already migrated)
        MIGRATIONS
            .to_latest(&mut conn)
            .map_err(|e| Error::Writer(format!("migration failed: {e}")))?;

        // 5. Restore FK enforcement for all subsequent operations
        conn.pragma_update(None, "foreign_keys", "ON")?;

        // 6. Seed builtin data (no-op if already seeded)
        seed::run(&mut conn)?;

        // 7. Create channel and spawn worker thread
        let (tx, rx) = mpsc::channel::<Job>();
        let handle = thread::spawn(move || {
            // conn move to thread here (open() can no longer use it)
            for job in rx {
                job(&mut conn);
            }
        });

        Ok(Writer {
            sender: Some(tx),
            handle: Some(handle),
        })
    }

    /// Send a closure to the worker thread and block for the result.
    /// f receives &mut Connection (can open own SQLite transactions, savepoints).
    /// For automatic SQLite BEGIN IMMEDIATE transactions, use call_write.
    pub fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let (reply_tx, reply_rx) = mpsc::sync_channel::<Result<T>>(0);

        let job: Job = Box::new(move |conn: &mut Connection| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(conn)));

            let to_send = match result {
                Ok(value) => value,
                Err(panic) => {
                    // TODO: post-panic health check. Run 'SELECT 1' here before attempting next job.
                    // If fail, close the writer (Matches rusqlite-isle's pattern)
                    let msg = panic
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown panic".to_string());
                    Err(Error::Panicked(msg))
                }
            };

            let _ = reply_tx.send(to_send);
        });

        self.sender
            .as_ref()
            .ok_or_else(|| Error::Writer("writer is closed".into()))?
            .send(job)
            .map_err(|_| Error::Writer("writer thread is gone".into()))?;

        reply_rx
            .recv()
            .map_err(|_| Error::Writer("writer dropped without replying".into()))?
    }

    /// Wraps call in a BEGIN IMMEDIATE transaction.
    ///  - Commits on Ok
    ///  - Rolls back on Err or panic (via Transaction's Drop).
    ///
    /// f receives &Connection (not &mut) because Transaction only derefs to &Connection
    /// All rusqlite write methods take &self, so this is sufficient.
    /// Cannot open nested transactions or savepoints inside f (use more generic call instead)
    pub fn call_write<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        self.call(move |conn| {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let result = f(&tx)?;
            tx.commit()?;
            Ok(result)
        })
    }
}

// Ordered shutdown necessary to prevent deadlocks:
//  1. Drop Sender (close channel, worker's `for job in rx` loop exits)
//  2. Join Handle (wait for thread to finish)
//
// JoinHandle's default drop detaches and continues running instead of joining.
// Use take() and join() explicitly to properly drop via Option.
impl Drop for Writer {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{error, open_reader};
 
    /// Helper: create a temp db, open a Writer, return both (tempfile stays alive).
    fn test_writer() -> (tempfile::NamedTempFile, Writer) {
        let tmp = tempfile::NamedTempFile::new().expect("failed to create temp file");
        let writer = Writer::open(tmp.path()).expect("Writer::open failed");
        (tmp, writer)
    }
 
    /// Foreign key constraint rejects a child row referencing a nonexistent parent.
    #[test]
    fn fk_rejects_bad_reference() {
        let (_tmp, writer) = test_writer();
 
        let result = writer.call_write(|conn| {
            conn.execute(
                "INSERT INTO repo_versions (repo_id, version, ingest_patterns, line_ending_policy) VALUES (9999, 1, '[]', 'preserve')",
                [],
            ).map_err(error::Error::from)?;
            Ok(())
        });
 
        assert!(result.is_err(), "expected FK violation, got Ok");
    }
 
    /// Deleting a pipeline_binding cascades to its export_state row.
    #[test]
    fn fk_cascade_binding_to_export_state() {
        let (_tmp, writer) = test_writer();
 
        writer.call_write(|conn| {
            conn.execute(
                "INSERT INTO build_recipes (name, curation) VALUES ('test-cascade', 'custom')",
                [],
            ).map_err(error::Error::from)?;
            let recipe_id = conn.last_insert_rowid();
 
            conn.execute(
                "INSERT INTO pipeline_bindings (build_recipe_id, active) VALUES (?1, 0)",
                [recipe_id],
            ).map_err(error::Error::from)?;
            let binding_id = conn.last_insert_rowid();
 
            conn.execute(
                "INSERT INTO export_state (pipeline_binding_id, output_dir, resolution_rules, runtime_versions, runtime_inputs, repo_root_hashes) VALUES (?1, 'out/', '{}', '{}', '{}', '{}')",
                [binding_id],
            ).map_err(error::Error::from)?;
 
            let count: i32 = conn.query_row(
                "SELECT count(*) FROM export_state WHERE pipeline_binding_id = ?1",
                [binding_id], |row| row.get(0),
            )?;
            assert_eq!(count, 1);
 
            conn.execute(
                "DELETE FROM pipeline_bindings WHERE id = ?1",
                [binding_id],
            ).map_err(error::Error::from)?;
 
            let count: i32 = conn.query_row(
                "SELECT count(*) FROM export_state WHERE pipeline_binding_id = ?1",
                [binding_id], |row| row.get(0),
            )?;
            assert_eq!(count, 0, "export_state should have been cascaded");
 
            Ok(())
        }).expect("cascade test failed");
    }
 
    /// Deleting a build_recipe_version sets file_history.build_recipe_version_id to NULL.
    #[test]
    fn fk_set_null_version_to_file_history() {
        let (_tmp, writer) = test_writer();
 
        writer.call_write(|conn| {
            conn.execute(
                "INSERT INTO build_recipes (name, curation) VALUES ('test-setnull', 'custom')",
                [],
            ).map_err(error::Error::from)?;
            let recipe_id = conn.last_insert_rowid();
 
            conn.execute(
                "INSERT INTO build_recipe_versions (build_recipe_id, version, source) VALUES (?1, 1, 'test')",
                [recipe_id],
            ).map_err(error::Error::from)?;
            let version_id = conn.last_insert_rowid();
 
            conn.execute(
                "INSERT INTO file_history (build_recipe_version_id, file_path, entry_type, content, placed_by) VALUES (?1, 'test.txt', 'snapshot', X'00', 'export')",
                [version_id],
            ).map_err(error::Error::from)?;
 
            conn.execute(
                "DELETE FROM build_recipe_versions WHERE id = ?1",
                [version_id],
            ).map_err(error::Error::from)?;
 
            let fh_version: Option<i64> = conn.query_row(
                "SELECT build_recipe_version_id FROM file_history WHERE file_path = 'test.txt'",
                [], |row| row.get(0),
            )?;
            assert_eq!(fh_version, None, "should be NULL after SET NULL cascade");
 
            Ok(())
        }).expect("set null test failed");
    }
 
    /// Two sequential Writer::open calls on the same database both succeed.
    #[test]
    fn concurrent_init_both_succeed() {
        let tmp = tempfile::NamedTempFile::new().expect("failed to create temp file");
 
        let writer1 = Writer::open(tmp.path());
        assert!(writer1.is_ok(), "first open failed: {:?}", writer1.err());
        drop(writer1);
 
        let writer2 = Writer::open(tmp.path());
        assert!(writer2.is_ok(), "second open failed: {:?}", writer2.err());
    }
 
    /// Opening the same database twice leaves the schema intact.
    #[test]
    fn idempotent_open_preserves_schema() {
        let tmp = tempfile::NamedTempFile::new().expect("failed to create temp file");
 
        let w1 = Writer::open(tmp.path()).expect("first open failed");
        drop(w1);
        let w2 = Writer::open(tmp.path()).expect("second open failed");
        drop(w2);
 
        let conn = open_reader(tmp.path()).expect("open_reader failed");
        let count: i32 = conn
            .prepare("SELECT count(*) FROM sqlite_master WHERE type='table'")
            .unwrap()
            .query_row([], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 16, "schema should still have 16 tables after re-open");
    }

    /// Builtin transforms cannot be hard-deleted.
    #[test]
    fn builtin_transform_rejects_delete() {
        let (_tmp, writer) = test_writer();
 
        let result = writer.call_write(|conn| {
            conn.execute(
                "DELETE FROM transforms WHERE name = 'flatten'",
                [],
            ).map_err(error::Error::from)?;
            Ok(())
        });
 
        assert!(result.is_err(), "should reject delete of builtin transform");
    }
 
    /// Builtin transforms cannot be soft-deleted.
    #[test]
    fn builtin_transform_rejects_soft_delete() {
        let (_tmp, writer) = test_writer();
 
        let result = writer.call_write(|conn| {
            conn.execute(
                "UPDATE transforms SET deleted_at = '2025-01-01T00:00:00Z' WHERE name = 'flatten'",
                [],
            ).map_err(error::Error::from)?;
            Ok(())
        });
 
        assert!(result.is_err(), "should reject soft-delete of builtin transform");
    }
 
    /// Builtin recipes cannot be hard-deleted.
    #[test]
    fn builtin_recipe_rejects_delete() {
        let (_tmp, writer) = test_writer();
 
        let result = writer.call_write(|conn| {
            conn.execute(
                "DELETE FROM build_recipes WHERE name = 'shipped-default'",
                [],
            ).map_err(error::Error::from)?;
            Ok(())
        });
 
        assert!(result.is_err(), "should reject delete of builtin recipe");
    }
 
    /// Custom rows are not affected by builtin protection triggers.
    #[test]
    fn custom_transform_allows_delete() {
        let (_tmp, writer) = test_writer();
 
        writer.call_write(|conn| {
            conn.execute(
                "INSERT INTO transforms (name, scope, curation) VALUES ('test-custom', 'file', 'custom')",
                [],
            ).map_err(error::Error::from)?;
 
            conn.execute(
                "DELETE FROM transforms WHERE name = 'test-custom'",
                [],
            ).map_err(error::Error::from)?;
 
            Ok(())
        }).expect("custom transform delete should succeed");
    }
}