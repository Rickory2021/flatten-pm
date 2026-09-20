// src-tauri/src/commands/db.rs
//
// Dev Tools commands: list tables and run read-only SQL queries.
// Intended for development inspection of the flatten-core database.

use crate::error::CommandError;
use crate::state::AppState;

/// Maximum rows returned by `db_query`. Prevents large result sets from
/// blocking IPC serialization. Does not bound query execution time: an
/// aggregate over an unbounded CTE runs until the connection is closed.
const QUERY_ROW_LIMIT: usize = 1000;

/// Result of a read-only SQL query.
#[derive(Debug, serde::Serialize)]
pub struct QueryResult {
    /// Column names from the result set.
    pub columns: Vec<String>,
    /// Row data. Each row is a vec of JSON values matching `columns`.
    pub rows: Vec<Vec<serde_json::Value>>,
    /// True if the result set exceeded `QUERY_ROW_LIMIT` and was truncated.
    pub truncated: bool,
}

/// List all table names in the database.
///
/// Uses a per-call reader connection (per ADR-038, no connection pool).
#[tauri::command]
pub async fn db_tables(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<String>, CommandError> {
    let conn = flatten_core::db::open_reader(&state.db_path)?;
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let tables: Vec<String> = rows.collect::<std::result::Result<_, _>>()?;
    Ok(tables)
}

/// Execute a read-only SQL query and return results as a table.
///
/// Write statements are rejected via `sqlite3_stmt_readonly` before
/// execution. Results are capped at [`QUERY_ROW_LIMIT`] rows.
///
/// Uses a per-call reader connection (per ADR-038, no connection pool).
#[tauri::command]
pub async fn db_query(
    sql: String,
    state: tauri::State<'_, AppState>,
) -> Result<QueryResult, CommandError> {
    let conn = flatten_core::db::open_reader(&state.db_path)?;
    execute_query(&conn, &sql)
}

/// Run a prepared statement against the given connection and collect results.
///
/// Separated from `db_query` so unit tests can call it with an in-memory
/// connection without needing Tauri state or a database file.
fn execute_query(
    conn: &rusqlite::Connection,
    sql: &str,
) -> Result<QueryResult, CommandError> {
    let mut stmt = conn.prepare(sql)?;

    // sqlite3_stmt_readonly: rejects INSERT, UPDATE, DELETE, DROP, ALTER,
    // CREATE, and also handles CTEs (WITH ... INSERT), PRAGMAs that write,
    // VACUUM, etc. correctly.
    //
    // readonly() returns true for BEGIN, COMMIT, ATTACH (read-only), DETACH.
    // On a read-only connection these are harmless (a BEGIN that is then
    // dropped rolls back; ATTACH inherits the read-only flag). This is
    // consistent with ADR-038: "the write thread is the only code that calls
    // BEGIN" applies to write connections; a reader's BEGIN is a no-op.
    //
    // The read-only connection would reject writes anyway, but this gives
    // a clear "not allowed" message instead of "attempt to write a readonly
    // database".
    //
    // rusqlite's prepare() returns Error::MultipleStatement on
    // "SELECT 1; SELECT 2", so multi-statement input gets a clear error
    // rather than silent truncation.
    if !stmt.readonly() {
        return Err(CommandError::domain("write statements are not allowed"));
    }

    let columns: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let col_count = columns.len();
    let mut result_rows = Vec::new();
    let mut truncated = false;

    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        if result_rows.len() >= QUERY_ROW_LIMIT {
            truncated = true;
            break;
        }
        let mut vals = Vec::with_capacity(col_count);
        for i in 0..col_count {
            let val: rusqlite::types::Value = row.get(i)?;
            let json_val = match val {
                rusqlite::types::Value::Null => serde_json::Value::Null,
                rusqlite::types::Value::Integer(n) => serde_json::json!(n),
                rusqlite::types::Value::Real(f) => serde_json::json!(f),
                rusqlite::types::Value::Text(s) => serde_json::json!(s),
                rusqlite::types::Value::Blob(b) => {
                    serde_json::json!(format!("<blob {} bytes>", b.len()))
                }
            };
            vals.push(json_val);
        }
        result_rows.push(vals);
    }

    Ok(QueryResult {
        columns,
        rows: result_rows,
        truncated,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Helper: open an in-memory read-write connection.
    /// Read-write on purpose: proves our readonly() check rejects writes,
    /// not SQLite's read-only flag.
    fn mem_conn() -> rusqlite::Connection {
        rusqlite::Connection::open_in_memory().unwrap()
    }

    /// INSERT is rejected by the readonly() check with kind "domain".
    #[test]
    fn query_rejects_write_statement() {
        let conn = mem_conn();
        conn.execute("CREATE TABLE t (x INTEGER)", []).unwrap();
        let result = execute_query(&conn, "INSERT INTO t VALUES (1)");
        assert!(result.is_err(), "write statement should be rejected");
        let err = result.unwrap_err();
        assert_eq!(err.kind, "domain", "error kind should be domain");
        assert!(
            err.error.contains("write statements are not allowed"),
            "error message should explain the rejection"
        );
    }

    /// A query returning more than QUERY_ROW_LIMIT rows is truncated.
    #[test]
    fn query_caps_rows_and_sets_truncated() {
        let conn = mem_conn();
        let result = execute_query(
            &conn,
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x < 2000) SELECT x FROM n",
        );
        let qr = result.expect("recursive CTE should succeed");
        assert_eq!(qr.rows.len(), QUERY_ROW_LIMIT, "should cap at QUERY_ROW_LIMIT");
        assert!(qr.truncated, "truncated flag should be true");
        assert_eq!(qr.columns, vec!["x"], "column name should be x");
    }

    /// A simple SELECT returns the expected columns and rows.
    #[test]
    fn query_returns_columns_and_rows() {
        let conn = mem_conn();
        let result = execute_query(&conn, "SELECT 1 AS a, 'hello' AS b");
        let qr = result.expect("simple SELECT should succeed");
        assert_eq!(qr.columns, vec!["a", "b"], "columns should match");
        assert_eq!(qr.rows.len(), 1, "should have one row");
        assert_eq!(qr.rows[0][0], serde_json::json!(1), "first column should be 1");
        assert_eq!(qr.rows[0][1], serde_json::json!("hello"), "second column should be 'hello'");
        assert!(!qr.truncated, "truncated should be false");
    }
}
