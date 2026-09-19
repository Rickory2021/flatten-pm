// src-tauri/src/state.rs
//
// Application state shared across Tauri commands. Holds the database
// Writer and the path for opening per-call reader connections.

use std::path::PathBuf;

use flatten_core::db::writer::Writer;

// Compile-time proof that Writer can be managed as Tauri state.
// Tauri's State<T> requires T: Send + Sync. If a future flatten-core
// change makes Writer !Sync, this fails the build instead of producing
// a confusing trait-bound error on manage().
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Writer>();
};

/// Shared application state managed by Tauri.
///
/// Commands receive this via `State<'_, AppState>`. The `writer` field
/// provides write access through its channel; reader connections are
/// opened per-call via `flatten_core::db::open_reader(&state.db_path)`.
pub struct AppState {
    /// Single writer thread handle. Commands call `writer.call()` or
    /// `writer.call_write()` which send closures to the background thread.
    pub writer: Writer,
    /// Database file path. Used by reader commands to open per-call
    /// read-only connections via `open_reader()`.
    pub db_path: PathBuf,
}

/// Initialize application state: create the data directory, open and
/// migrate the database, return the Writer and path.
///
/// Uses `dirs::data_dir()` (not Tauri's path resolver) so the GUI and
/// CLI share the same database at `{data_dir}/flatten-pm/flatten.db`.
/// Tauri's resolver maps to `{data_dir}/com.flatten-pm.app`, which would
/// create a separate database. Do not switch to Tauri's resolver without
/// also migrating the CLI.
pub fn init_app_state() -> Result<AppState, String> {
    let data_dir = dirs::data_dir()
        .ok_or("could not determine platform data directory")?
        .join("flatten-pm");

    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("failed to create data directory: {e}"))?;

    let db_path = data_dir.join("flatten.db");

    let writer = Writer::open(&db_path)
        .map_err(|e| format!("failed to open database: {e}"))?;

    Ok(AppState { writer, db_path })
}
