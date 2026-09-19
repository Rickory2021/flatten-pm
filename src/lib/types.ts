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
