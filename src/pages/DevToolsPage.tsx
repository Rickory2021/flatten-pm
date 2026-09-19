// src/pages/DevToolsPage.tsx
//
// Dev Tools: database table list and read-only SQL query runner.
// Calls db_tables on mount and db_query on execute.

import { useEffect, useState } from "react";
import { Terminal } from "lucide-react";
import { cn } from "@/lib/utils";
import { dbTables, dbQuery } from "@/lib/tauri";
import type { QueryResult } from "@/lib/types";
import { isCommandError } from "@/lib/types";

export function DevToolsPage() {
  const [tables, setTables] = useState<string[]>([]);
  const [tableError, setTableError] = useState("");
  const [sql, setSql] = useState("");
  const [result, setResult] = useState<QueryResult | null>(null);
  const [queryError, setQueryError] = useState("");
  const [running, setRunning] = useState(false);

  // Load table list on mount.
  useEffect(() => {
    let cancelled = false;
    dbTables()
      .then((names) => {
        if (!cancelled) setTables(names);
      })
      .catch((e: unknown) => {
        if (!cancelled)
          setTableError(
            isCommandError(e) ? e.error : "Failed to load tables"
          );
      });
    return () => {
      cancelled = true;
    };
  }, []);

  /** Populate query input when a table name is clicked. */
  const onTableClick = (name: string) => {
    setSql(`SELECT * FROM "${name}" LIMIT 50`);
    setResult(null);
    setQueryError("");
  };

  /** Execute the current SQL query. */
  const onExecute = async () => {
    const trimmed = sql.trim();
    if (!trimmed) return;

    setRunning(true);
    setResult(null);
    setQueryError("");

    try {
      const qr = await dbQuery(trimmed);
      setResult(qr);
    } catch (e: unknown) {
      setQueryError(isCommandError(e) ? e.error : "Query failed");
    } finally {
      setRunning(false);
    }
  };

  /** Handle Ctrl+Enter in the textarea. */
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      onExecute();
    }
  };

  return (
    <div className="p-6 flex flex-col gap-6 h-full">
      {/* Header */}
      <div>
        <div className="flex items-center gap-2">
          <Terminal className="h-5 w-5 text-accent" />
          <h1 className="text-xl font-semibold text-text">Dev Tools</h1>
        </div>
        <p className="text-text-muted mt-1">
          Database inspector and query runner.
        </p>
      </div>

      {/* Table list */}
      <div>
        <h2 className="text-sm font-medium text-text mb-2">Tables</h2>
        {tableError ? (
          <p className="text-xs text-danger">{tableError}</p>
        ) : (
          <div className="flex flex-wrap gap-1.5">
            {tables.map((name) => (
              <button
                key={name}
                onClick={() => onTableClick(name)}
                className={cn(
                  "rounded border border-border px-2 py-0.5 text-xs",
                  "text-text-muted hover:text-text hover:border-accent",
                  "focus:outline-none focus:ring-1 focus:ring-accent"
                )}
              >
                {name}
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Query runner */}
      <div className="flex flex-col gap-2 flex-1 min-h-0">
        <h2 className="text-sm font-medium text-text">Query</h2>
        <textarea
          value={sql}
          onChange={(e) => setSql(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder="SELECT * FROM settings LIMIT 10"
          rows={4}
          className={cn(
            "w-full rounded border border-border bg-surface text-text",
            "px-3 py-2 text-sm font-mono",
            "placeholder:text-text-muted",
            "focus:outline-none focus:ring-1 focus:ring-accent",
            "resize-y"
          )}
        />
        <div className="flex items-center gap-3">
          <button
            onClick={onExecute}
            disabled={running || !sql.trim()}
            className={cn(
              "rounded px-3 py-1.5 text-sm font-medium",
              "focus:outline-none focus:ring-1 focus:ring-accent",
              running || !sql.trim()
                ? "bg-border text-text-muted cursor-not-allowed"
                : "bg-accent text-white hover:opacity-90"
            )}
          >
            {running ? "Running..." : "Execute"}
          </button>
          <span className="text-xs text-text-muted">Ctrl+Enter to run</span>
        </div>

        {/* Error */}
        {queryError && (
          <p className="text-xs text-danger">{queryError}</p>
        )}

        {/* Results */}
        {result && (
          <div className="flex flex-col gap-1 flex-1 min-h-0">
            <div className="flex items-center gap-2 text-xs text-text-muted">
              <span>
                {result.rows.length} row{result.rows.length !== 1 ? "s" : ""}
              </span>
              {result.truncated && (
                <span className="text-warning">
                  Results capped at 1000 rows.
                </span>
              )}
            </div>
            <div className="overflow-auto border border-border rounded flex-1 max-h-96">
              <table className="w-full text-xs">
                <thead>
                  <tr className="bg-surface sticky top-0">
                    {result.columns.map((col) => (
                      <th
                        key={col}
                        className="px-2 py-1.5 text-left font-medium text-text border-b border-border whitespace-nowrap"
                      >
                        {col}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {result.rows.map((row, rowIdx) => (
                    <tr
                      key={rowIdx}
                      className="border-b border-border-muted hover:bg-surface"
                    >
                      {row.map((val, colIdx) => (
                        <td
                          key={colIdx}
                          className={cn(
                            "px-2 py-1 whitespace-nowrap",
                            val === null ? "text-text-muted italic" : "text-text"
                          )}
                        >
                          {val === null ? "NULL" : String(val)}
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
