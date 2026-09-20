// src/pages/SettingsPage.tsx
//
// Settings page: lists all 9 settings, edits with validation feedback.
// Calls settings_list on mount and settings_set on save.

import { useEffect, useState } from "react";
import { Settings } from "lucide-react";
import { cn } from "@/lib/utils";
import { settingsList, settingsSet } from "@/lib/tauri";
import type { Setting } from "@/lib/types";
import { isCommandError } from "@/lib/types";

/** Type hints shown as placeholder text per setting key. */
const TYPE_HINTS: Record<string, string> = {
  watch_source_dir: "absolute path to directory (empty = unset)",
  watch_poll_interval_ms: "positive integer (ms)",
  watch_settle_ms: "positive integer (ms)",
  watch_debounce_ms: "positive integer (ms)",
  trie_refresh_interval_ms: "positive integer (ms)",
  transform_timeout_ms: "positive integer (ms)",
  transform_memory_mb: "positive integer (MB)",
  copy_size_limit_mb: "positive integer (MB)",
  binary_extensions: "comma-separated list (e.g. .png,.jpg)",
};

/** Per-row edit state. */
interface SettingRow {
  key: string;
  saved: string;
  draft: string;
  status: "idle" | "saving" | "saved" | "error";
  error: string;
}

export function SettingsPage() {
  const [rows, setRows] = useState<SettingRow[]>([]);
  const [loadError, setLoadError] = useState<string>("");

  // Load settings on mount.
  useEffect(() => {
    let cancelled = false;
    settingsList()
      .then((settings: Setting[]) => {
        if (cancelled) return;
        setRows(
          settings.map((s) => ({
            key: s.key,
            saved: s.value,
            draft: s.value,
            status: "idle",
            error: "",
          }))
        );
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setLoadError(
          isCommandError(e) ? e.error : "Failed to load settings"
        );
      });
    return () => {
      cancelled = true;
    };
  }, []);

  /** Update a row's draft value. */
  const onDraftChange = (key: string, value: string) => {
    setRows((prev) =>
      prev.map((r) =>
        r.key === key ? { ...r, draft: value, status: "idle", error: "" } : r
      )
    );
  };

  /** Save a single setting. */
  const onSave = async (key: string) => {
    const row = rows.find((r) => r.key === key);
    if (!row || row.draft === row.saved) return;

    setRows((prev) =>
      prev.map((r) => (r.key === key ? { ...r, status: "saving", error: "" } : r))
    );

    try {
      await settingsSet(key, row.draft);
      setRows((prev) =>
        prev.map((r) =>
          r.key === key
            ? { ...r, saved: r.draft, status: "saved", error: "" }
            : r
        )
      );
      // Clear "saved" indicator after 2 seconds.
      setTimeout(() => {
        setRows((prev) =>
          prev.map((r) =>
            r.key === key && r.status === "saved"
              ? { ...r, status: "idle" }
              : r
          )
        );
      }, 2000);
    } catch (e: unknown) {
      setRows((prev) =>
        prev.map((r) =>
          r.key === key
            ? {
                ...r,
                status: "error",
                error: isCommandError(e) ? e.error : "Save failed",
              }
            : r
        )
      );
    }
  };

  /** Handle Enter key on an input. */
  const onKeyDown = (e: React.KeyboardEvent, key: string) => {
    if (e.key === "Enter") {
      e.preventDefault();
      onSave(key);
    }
  };

  // Mount failure: show page-level error, no rows.
  if (loadError) {
    return (
      <div className="p-6">
        <div className="flex items-center gap-2">
          <Settings className="h-5 w-5 text-accent" />
          <h1 className="text-xl font-semibold text-text">Settings</h1>
        </div>
        <p className="text-danger mt-4">{loadError}</p>
      </div>
    );
  }

  return (
    <div className="p-6">
      <div className="flex items-center gap-2">
        <Settings className="h-5 w-5 text-accent" />
        <h1 className="text-xl font-semibold text-text">Settings</h1>
      </div>
      <p className="text-text-muted mt-1">
        Global application settings.
      </p>

      <div className="mt-6 space-y-4">
        {rows.map((row) => (
          <div key={row.key} className="space-y-1">
            <label
              htmlFor={`setting-${row.key}`}
              className="block text-sm font-medium text-text"
            >
              {row.key}
              {TYPE_HINTS[row.key] && (
                <span className="ml-2 font-normal text-xs text-text-muted">
                  ({TYPE_HINTS[row.key]})
                </span>
              )}
            </label>
            <div className="flex items-center gap-2">
              <input
                id={`setting-${row.key}`}
                type="text"
                value={row.draft}
                onChange={(e) => onDraftChange(row.key, e.target.value)}
                onKeyDown={(e) => onKeyDown(e, row.key)}
                placeholder={row.key === "watch_source_dir" ? "/absolute/path/to/directory" : ""}
                className={cn(
                  "flex-1 rounded border px-3 py-1.5 text-sm bg-surface text-text",
                  "placeholder:text-text-muted",
                  "focus:outline-none focus:ring-1 focus:ring-accent",
                  row.status === "error"
                    ? "border-danger"
                    : "border-border"
                )}
              />
              <button
                onClick={() => onSave(row.key)}
                disabled={row.draft === row.saved || row.status === "saving"}
                className={cn(
                  "rounded px-3 py-1.5 text-sm font-medium",
                  "focus:outline-none focus:ring-1 focus:ring-accent",
                  row.draft === row.saved || row.status === "saving"
                    ? "bg-border text-text-muted cursor-not-allowed"
                    : "bg-accent text-white hover:opacity-90"
                )}
              >
                {row.status === "saving" ? "Saving..." : "Save"}
              </button>
            </div>
            {row.status === "saved" && (
              <p className="text-xs text-success">Saved</p>
            )}
            {row.status === "error" && (
              <p className="text-xs text-danger">{row.error}</p>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
