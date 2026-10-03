// src/components/repos/RepoList.tsx
//
// Repo list view: table of registered repos, empty state, "Add repo" button.
// Fetches repoList on mount.

import { useEffect, useState } from "react";
import { Plus } from "lucide-react";
import { cn } from "@/lib/utils";
import { repoList } from "@/lib/tauri";
import type { RepoRow } from "@/lib/types";
import { isCommandError } from "@/lib/types";

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface RepoListProps {
  onAdd: () => void;
  onSelect: (repoId: number) => void;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Format an ISO timestamp to a short local string, or "--" if null. */
function formatTimestamp(ts: string | null): string {
  if (!ts) return "--";
  try {
    const d = new Date(ts);
    return d.toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return ts;
  }
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function RepoList({ onAdd, onSelect }: RepoListProps) {
  const [repos, setRepos] = useState<RepoRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    let cancelled = false;
    repoList()
      .then((rows) => {
        if (!cancelled) {
          setRepos(rows);
          setLoading(false);
        }
      })
      .catch((e: unknown) => {
        if (!cancelled) {
          setError(isCommandError(e) ? e.error : "Failed to load repos");
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Loading state
  if (loading) {
    return <p className="text-text-muted text-sm">Loading repos...</p>;
  }

  // Error state
  if (error) {
    return <p className="text-danger text-sm">{error}</p>;
  }

  // Empty state
  if (repos.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <p className="text-text-muted mb-4">
          No repositories registered yet.
        </p>
        <button
          onClick={onAdd}
          className={cn(
            "inline-flex items-center gap-1.5 rounded px-4 py-2 text-sm font-medium",
            "bg-accent text-white hover:opacity-90",
            "focus:outline-none focus:ring-1 focus:ring-accent",
          )}
        >
          <Plus className="h-4 w-4" />
          Add repo
        </button>
      </div>
    );
  }

  // Table
  return (
    <div>
      <div className="flex items-center justify-between mb-4">
        <p className="text-text-muted text-sm">
          {repos.length} {repos.length === 1 ? "repo" : "repos"}
        </p>
        <button
          onClick={onAdd}
          className={cn(
            "inline-flex items-center gap-1.5 rounded px-3 py-1.5 text-sm font-medium",
            "bg-accent text-white hover:opacity-90",
            "focus:outline-none focus:ring-1 focus:ring-accent",
          )}
        >
          <Plus className="h-4 w-4" />
          Add repo
        </button>
      </div>

      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-border text-left text-text-muted">
            <th className="pb-2 pr-4 font-medium">Name</th>
            <th className="pb-2 pr-4 font-medium">Path</th>
            <th className="pb-2 font-medium">Last ingested</th>
          </tr>
        </thead>
        <tbody>
          {repos.map((repo) => (
            <tr
              key={repo.id}
              onClick={() => onSelect(repo.id)}
              className={cn(
                "border-b border-border-muted cursor-pointer",
                "hover:bg-surface transition-colors",
              )}
            >
              <td className="py-2.5 pr-4 font-medium text-text">
                {repo.name}
              </td>
              <td
                className="py-2.5 pr-4 text-text-muted max-w-xs truncate"
                title={repo.path}
              >
                {repo.path}
              </td>
              <td className="py-2.5 text-text-muted">
                {formatTimestamp(repo.trie_updated_at)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
