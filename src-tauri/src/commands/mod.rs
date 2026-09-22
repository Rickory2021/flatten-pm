// src-tauri/src/commands/mod.rs
//
// Tauri command modules. Each module contains pub async fn commands
// registered in lib.rs via generate_handler!.

pub mod db;
pub mod repo;
pub mod settings;
