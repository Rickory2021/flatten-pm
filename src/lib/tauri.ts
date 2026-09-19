// src/lib/tauri.ts
//
// Typed wrappers over Tauri's invoke(). Each function maps to a
// #[tauri::command] in src-tauri/src/commands/.
//
// Note: Tauri converts multi-word Rust argument names to camelCase
// on the JS side by default (e.g. repo_name becomes repoName). No
// Phase 3 args are multi-word, but Phase 4 args will be. Keep this
// in mind when adding wrappers.

import { invoke } from "@tauri-apps/api/core";
import type { Setting, QueryResult } from "./types";

/** List all settings with their current values. */
export async function settingsList(): Promise<Setting[]> {
  return invoke<Setting[]>("settings_list");
}

/** Update a setting value. Validates key and value server-side. */
export async function settingsSet(key: string, value: string): Promise<void> {
  return invoke<void>("settings_set", { key, value });
}

/** List all table names in the database. */
export async function dbTables(): Promise<string[]> {
  return invoke<string[]>("db_tables");
}

/** Execute a read-only SQL query. Write statements are rejected. */
export async function dbQuery(sql: string): Promise<QueryResult> {
  return invoke<QueryResult>("db_query", { sql });
}
