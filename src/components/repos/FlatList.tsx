// src/components/repos/FlatList.tsx
//
// Flat sorted list of trie paths with a search/filter input.
// No virtualization in v1; revisit if performance degrades above ~2000 items.

import { useState, useMemo } from "react";
import { Search } from "lucide-react";
import { cn } from "@/lib/utils";

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface FlatListProps {
  /** Sorted list of trie paths. */
  paths: string[];
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function FlatList({ paths }: FlatListProps) {
  const [filter, setFilter] = useState("");

  const filtered = useMemo(() => {
    if (!filter.trim()) return paths;
    const lower = filter.toLowerCase();
    return paths.filter((p) => p.toLowerCase().includes(lower));
  }, [paths, filter]);

  return (
    <div>
      {/* Search input */}
      <div className="relative mb-3">
        <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-text-muted" />
        <input
          type="text"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter paths..."
          className={cn(
            "w-full rounded border border-border pl-8 pr-3 py-1.5 text-sm",
            "bg-surface text-text font-mono",
            "placeholder:text-text-muted",
            "focus:outline-none focus:ring-1 focus:ring-accent",
          )}
        />
      </div>

      {/* Count */}
      <p className="text-xs text-text-muted mb-2">
        {filtered.length === paths.length
          ? `${paths.length} file${paths.length === 1 ? "" : "s"}`
          : `${filtered.length} of ${paths.length} files`}
      </p>

      {/* Path list */}
      <div className="max-h-96 overflow-y-auto rounded border border-border-muted bg-surface">
        {filtered.length === 0 ? (
          <p className="px-3 py-4 text-sm text-text-muted text-center italic">
            {paths.length === 0 ? "No files." : "No matches."}
          </p>
        ) : (
          filtered.map((p) => (
            <div
              key={p}
              className="px-3 py-1 text-xs font-mono text-text border-b border-border-muted last:border-b-0"
              title={p}
            >
              {p}
            </div>
          ))
        )}
      </div>
    </div>
  );
}
