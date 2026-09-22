// src/lib/types.ts
//
// Shared TypeScript types for Tauri IPC. Mirrors the Rust structs
// in src-tauri/src/commands/ and src-tauri/src/error.rs.

/** A single setting row (key-value pair). Mirrors commands::settings::Setting. */
export interface Setting {
  key: string;
  value: string;
}

/** Result of a read-only SQL query. Mirrors commands::db::QueryResult. */
export interface QueryResult {
  columns: string[];
  rows: (string | number | null)[][];
  truncated: boolean;
}

/** Tauri command error shape. Mirrors error::CommandError. */
export interface CommandError {
  error: string;
  kind: "domain" | "database" | "io" | "transform" | "usage";
}

/** Type guard for invoke error handling. Tauri's invoke() rejects with unknown. */
export function isCommandError(e: unknown): e is CommandError {
  return (
    typeof e === "object" &&
    e !== null &&
    "error" in e &&
    "kind" in e &&
    typeof (e as CommandError).error === "string" &&
    typeof (e as CommandError).kind === "string"
  );
}

// ---------------------------------------------------------------------------
// Repo types (APP-003)
// ---------------------------------------------------------------------------

/** A repos table row. Mirrors flatten_core::ingest::RepoRow. */
export interface RepoRow {
  id: number;
  path: string;
  name: string;
  ingest_patterns: string[];
  line_ending_policy: string;
  trie_updated_at: string | null;
  created_at: string;
  deleted_at: string | null;
}

/** Result of an ingest operation. Mirrors commands::repo::IngestReportDto. */
export interface IngestReport {
  file_count: number;
  root_hash: string;
  lossy_count: number;
  enrichment_count: number;
}

/** Returned by repoAdd. Contains the new repo's ID and ingest report. */
export interface RepoAddResult {
  id: number;
  report: IngestReport;
}

/** Returned by repoReadFile. Distinguishes symlinks from regular files. */
export interface FilePreviewResult {
  kind: "file" | "symlink";
  content: string;
}
