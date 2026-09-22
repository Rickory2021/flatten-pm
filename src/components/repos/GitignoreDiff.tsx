// src/components/repos/GitignoreDiff.tsx
//
// Gitignore re-import diff view. Shows patterns in .gitignore that are
// not in the stored list (addable with checkboxes), and patterns in the
// stored list that are not in .gitignore (informational only).
// "Add selected" merges checked patterns into the stored list via repoEdit.

import { useEffect, useState, useMemo } from "react";
import { Loader2, Plus, Info } from "lucide-react";
import { cn } from "@/lib/utils";
import { repoGitignorePatterns, repoEdit } from "@/lib/tauri";
import { isCommandError } from "@/lib/types";

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface GitignoreDiffProps {
  repoId: number;
  repoPath: string;
  storedPatterns: string[];
  onClose: () => void;
  /** Called after patterns are merged so parent can refetch. */
  onApplied: () => void;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function GitignoreDiff({
  repoId,
  repoPath,
  storedPatterns,
  onClose,
  onApplied,
}: GitignoreDiffProps) {
  const [gitignorePatterns, setGitignorePatterns] = useState<string[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [applying, setApplying] = useState(false);

  // Checkboxes for addable patterns
  const [checked, setChecked] = useState<Set<string>>(new Set());

  // Fetch gitignore patterns
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError("");

    repoGitignorePatterns(repoPath)
      .then((pats) => {
        if (!cancelled) {
          setGitignorePatterns(pats);
          setLoading(false);
        }
      })
      .catch((e: unknown) => {
        if (!cancelled) {
          setError(isCommandError(e) ? e.error : "Failed to read .gitignore");
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [repoPath]);

  // Compute diffs
  const storedSet = useMemo(() => new Set(storedPatterns), [storedPatterns]);
  const gitignoreSet = useMemo(
    () => new Set(gitignorePatterns ?? []),
    [gitignorePatterns],
  );

  // In .gitignore but not stored (addable)
  const addable = useMemo(
    () => (gitignorePatterns ?? []).filter((p) => !storedSet.has(p)),
    [gitignorePatterns, storedSet],
  );

  // In stored but not in .gitignore (informational)
  const onlyStored = useMemo(
    () => storedPatterns.filter((p) => !gitignoreSet.has(p)),
    [storedPatterns, gitignoreSet],
  );

  // Toggle checkbox
  const toggleChecked = (pattern: string) => {
    setChecked((prev) => {
      const next = new Set(prev);
      if (next.has(pattern)) {
        next.delete(pattern);
      } else {
        next.add(pattern);
      }
      return next;
    });
  };

  // Select/deselect all
  const selectAll = () => setChecked(new Set(addable));
  const selectNone = () => setChecked(new Set());

  // Apply: merge checked patterns into stored list
  const handleApply = async () => {
    if (checked.size === 0) return;
    setApplying(true);
    try {
      const merged = [...storedPatterns, ...addable.filter((p) => checked.has(p))];
      await repoEdit(repoId, undefined, merged, undefined);
      onApplied();
    } catch (e: unknown) {
      setError(isCommandError(e) ? e.error : "Failed to apply patterns.");
      setApplying(false);
    }
  };

  // --- Render ---

  return (
    <div className="p-4 rounded border border-border bg-surface">
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-medium text-text">.gitignore comparison</h3>
        <button
          onClick={onClose}
          className="text-xs text-text-muted hover:text-text"
        >
          Close
        </button>
      </div>

      {loading && (
        <div className="flex items-center gap-2 text-sm text-text-muted py-4">
          <Loader2 className="h-4 w-4 animate-spin" />
          Scanning .gitignore files...
        </div>
      )}

      {error && <p className="text-sm text-danger mb-3">{error}</p>}

      {!loading && gitignorePatterns !== null && (
        <>
          {/* Section 1: Addable patterns */}
          <div className="mb-4">
            <div className="flex items-center justify-between mb-2">
              <p className="text-xs font-medium text-text">
                In .gitignore, not in your list
                {addable.length > 0 && (
                  <span className="text-text-muted ml-1">({addable.length})</span>
                )}
              </p>
              {addable.length > 1 && (
                <div className="flex gap-2">
                  <button
                    onClick={selectAll}
                    className="text-xs text-accent hover:underline"
                  >
                    Select all
                  </button>
                  <button
                    onClick={selectNone}
                    className="text-xs text-text-muted hover:underline"
                  >
                    Clear
                  </button>
                </div>
              )}
            </div>

            {addable.length === 0 ? (
              <p className="text-xs text-text-muted italic py-2">
                Your pattern list already includes everything from .gitignore.
              </p>
            ) : (
              <div className="space-y-1">
                {addable.map((pattern) => (
                  <label
                    key={pattern}
                    className={cn(
                      "flex items-center gap-2 rounded border border-border-muted px-3 py-1.5",
                      "text-sm font-mono cursor-pointer hover:bg-overlay",
                      checked.has(pattern)
                        ? "text-success border-success/30"
                        : "text-text",
                    )}
                  >
                    <input
                      type="checkbox"
                      checked={checked.has(pattern)}
                      onChange={() => toggleChecked(pattern)}
                      className="rounded border-border text-accent focus:ring-accent"
                    />
                    <span className="truncate" title={pattern}>
                      {pattern}
                    </span>
                  </label>
                ))}
              </div>
            )}

            {addable.length > 0 && (
              <button
                onClick={handleApply}
                disabled={checked.size === 0 || applying}
                className={cn(
                  "inline-flex items-center gap-1 rounded px-3 py-1.5 text-sm font-medium mt-3",
                  "focus:outline-none focus:ring-1 focus:ring-accent",
                  checked.size === 0 || applying
                    ? "bg-border text-text-muted cursor-not-allowed"
                    : "bg-accent text-white hover:opacity-90",
                )}
              >
                {applying ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Plus className="h-3.5 w-3.5" />
                )}
                {applying
                  ? "Applying..."
                  : `Add ${checked.size} pattern${checked.size === 1 ? "" : "s"}`}
              </button>
            )}
          </div>

          {/* Section 2: Informational (stored but not in gitignore) */}
          {onlyStored.length > 0 && (
            <div>
              <div className="flex items-center gap-1.5 mb-2">
                <Info className="h-3 w-3 text-text-muted" />
                <p className="text-xs font-medium text-text-muted">
                  In your list, not in .gitignore ({onlyStored.length})
                </p>
              </div>
              <p className="text-xs text-text-muted mb-2">
                These may be patterns you added manually. Remove them from the pattern editor if needed.
              </p>
              <div className="space-y-1">
                {onlyStored.map((pattern) => (
                  <div
                    key={pattern}
                    className="rounded border border-border-muted px-3 py-1.5 text-sm font-mono text-text-muted"
                    title={pattern}
                  >
                    {pattern}
                  </div>
                ))}
              </div>
            </div>
          )}

          {addable.length === 0 && onlyStored.length === 0 && (
            <p className="text-xs text-text-muted italic py-2">
              Your pattern list matches .gitignore exactly.
            </p>
          )}
        </>
      )}
    </div>
  );
}
