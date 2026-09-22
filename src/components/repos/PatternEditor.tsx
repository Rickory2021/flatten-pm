// src/components/repos/PatternEditor.tsx
//
// Pattern editor: displays a list of glob patterns with add/remove.
// Used in the wizard (step 3) and detail view. Contextual suggestions
// from tree selection are added in chunk 7c.

import { useState } from "react";
import { Plus, X } from "lucide-react";
import { cn } from "@/lib/utils";

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export interface PatternEditorProps {
  /** Current pattern list. */
  patterns: string[];
  /** Called when the pattern list changes. */
  onChange: (patterns: string[]) => void;
  /** Label shown above the pattern list. */
  label?: string;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function PatternEditor({
  patterns,
  onChange,
  label = "Ingest patterns",
}: PatternEditorProps) {
  const [draft, setDraft] = useState("");

  const addPattern = () => {
    const trimmed = draft.trim();
    if (!trimmed) return;
    // Avoid duplicates
    if (patterns.includes(trimmed)) {
      setDraft("");
      return;
    }
    onChange([...patterns, trimmed]);
    setDraft("");
  };

  const removePattern = (index: number) => {
    onChange(patterns.filter((_, i) => i !== index));
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      addPattern();
    }
  };

  return (
    <div>
      <p className="text-sm font-medium text-text mb-2">{label}</p>

      {/* Pattern list */}
      {patterns.length > 0 && (
        <ul className="mb-3 space-y-1">
          {patterns.map((pattern, i) => (
            <li
              key={`${pattern}-${i}`}
              className={cn(
                "flex items-center justify-between gap-2",
                "rounded border border-border-muted px-3 py-1.5",
                "bg-surface text-sm text-text font-mono",
              )}
            >
              <span className="truncate" title={pattern}>
                {pattern}
              </span>
              <button
                onClick={() => removePattern(i)}
                className="shrink-0 text-text-muted hover:text-danger"
                title="Remove pattern"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </li>
          ))}
        </ul>
      )}

      {/* Add input */}
      <div className="flex items-center gap-2">
        <input
          type="text"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder="*.log, node_modules/, etc."
          className={cn(
            "flex-1 rounded border border-border px-3 py-1.5 text-sm",
            "bg-surface text-text font-mono",
            "placeholder:text-text-muted",
            "focus:outline-none focus:ring-1 focus:ring-accent",
          )}
        />
        <button
          onClick={addPattern}
          disabled={!draft.trim()}
          className={cn(
            "inline-flex items-center gap-1 rounded px-3 py-1.5 text-sm font-medium",
            "focus:outline-none focus:ring-1 focus:ring-accent",
            draft.trim()
              ? "bg-accent text-white hover:opacity-90"
              : "bg-border text-text-muted cursor-not-allowed",
          )}
        >
          <Plus className="h-3.5 w-3.5" />
          Add
        </button>
      </div>
    </div>
  );
}
