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
import { open } from "@tauri-apps/plugin-dialog";
import type {
  Setting,
  QueryResult,
  RepoRow,
  RepoAddResult,
  IngestReport,
  FilePreviewResult,
} from "./types";

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/** List all settings with their current values. */
export async function settingsList(): Promise<Setting[]> {
  return invoke<Setting[]>("settings_list");
}

/** Update a setting value. Validates key and value server-side. */
export async function settingsSet(key: string, value: string): Promise<void> {
  return invoke<void>("settings_set", { key, value });
}

// ---------------------------------------------------------------------------
// Dev tools
// ---------------------------------------------------------------------------

/** List all table names in the database. */
export async function dbTables(): Promise<string[]> {
  return invoke<string[]>("db_tables");
}

/** Execute a read-only SQL query. Write statements are rejected. */
export async function dbQuery(sql: string): Promise<QueryResult> {
  return invoke<QueryResult>("db_query", { sql });
}

// ---------------------------------------------------------------------------
// Repos (APP-003)
// ---------------------------------------------------------------------------

/** Open a native directory picker. Returns null if the user cancels. */
export async function pickDirectory(): Promise<string | null> {
  const selected = await open({ directory: true, multiple: false });
  return selected;
}

/** List all non-deleted repos. */
export async function repoList(): Promise<RepoRow[]> {
  return invoke<RepoRow[]>("repo_list");
}

/** Register a new repo. Returns the ID and ingest report. */
export async function repoAdd(
  path: string,
  name: string,
  patterns: string[],
  importGitignore: boolean,
): Promise<RepoAddResult> {
  return invoke<RepoAddResult>("repo_add", {
    path,
    name,
    patterns,
    importGitignore,
  });
}

/** Return trie paths for a repo. */
export async function repoTree(repoId: number): Promise<string[]> {
  return invoke<string[]>("repo_tree", { repoId });
}

/** Update repo fields. Only provided fields are changed. Triggers re-ingest. */
export async function repoEdit(
  repoId: number,
  name?: string,
  patterns?: string[],
  lineEndingPolicy?: string,
): Promise<IngestReport> {
  return invoke<IngestReport>("repo_edit", {
    repoId,
    name,
    patterns,
    lineEndingPolicy,
  });
}

/** Trigger a full re-ingest of a repo. */
export async function repoReingest(repoId: number): Promise<IngestReport> {
  return invoke<IngestReport>("repo_reingest", { repoId });
}

/** Soft-delete a repo. */
export async function repoDelete(repoId: number): Promise<void> {
  return invoke<void>("repo_delete", { repoId });
}

/** Return files on disk that are NOT in the trie (excluded by patterns). */
export async function repoExcludedFiles(repoId: number): Promise<string[]> {
  return invoke<string[]>("repo_excluded_files", { repoId });
}

/** Preview which files would be included for a directory with given patterns. */
export async function repoPreview(
  path: string,
  patterns: string[],
  importGitignore: boolean,
): Promise<string[]> {
  return invoke<string[]>("repo_preview", { path, patterns, importGitignore });
}

/** Read a file from a registered repo for preview. */
export async function repoReadFile(
  repoId: number,
  relativePath: string,
): Promise<FilePreviewResult> {
  return invoke<FilePreviewResult>("repo_read_file", {
    repoId,
    relativePath,
  });
}

/** Import .gitignore patterns from a directory path. */
export async function repoGitignorePatterns(
  path: string,
): Promise<string[]> {
  return invoke<string[]>("repo_gitignore_patterns", { path });
}
