// src-tauri/src/lib.rs
//
// Tauri application entry point. Initializes managed state and registers
// commands. All pipeline logic lives in flatten-core; this layer is thin
// dispatch.

mod commands;
mod error;
mod state;

/// Build and run the Tauri application.
///
/// State is initialized before the builder so that startup failures
/// (data directory, database open) produce a message on stderr instead
/// of panicking. Tauri's `.setup()` hook panics on `Err`, and with
/// `panic = "abort"` in the release profile that would abort silently.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = match state::init_app_state() {
        Ok(s) => s,
        Err(e) => {
            // Windows release builds hide the console (windows_subsystem = "windows"),
            // so this is invisible there. A native error dialog needs the dialog
            // plugin, deferred to Phase 4. Dev builds show the console.
            eprintln!("flatten-pm: {e}");
            std::process::exit(1);
        }
    };

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::settings::settings_list,
            commands::settings::settings_set,
            commands::db::db_tables,
            commands::db::db_query,
        ])
        .run(tauri::generate_context!());

    if let Err(e) = app {
        eprintln!("flatten-pm: {e}");
        std::process::exit(1);
    }
}
