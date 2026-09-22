// src/components/repos/PatternEditor.tsx
//
// Pattern editor: displays a list of glob patterns with add/remove.
// When a selectedNode is provided, shows contextual action suggestions
// (exclude file, extension, folder; include with negation).

import { useState } from "react";
import { Plus, X, Crosshair } from "lucide-react";
import { cn } from "@/lib/utils";
import type { TreeNode } from "@/lib/tree";

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
  /** When provided, shows contextual action suggestions. */
  selectedNode?: TreeNode | null;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Extract file extension from a path. Returns null if none. */
function getExtension(path: string): string | null {
  const dot = path.lastIndexOf(".");
  if (dot <= 0 || dot === path.length - 1) return null;
  const name = path.split("/").pop() ?? path;
  const nameDot = name.lastIndexOf(".");
  if (nameDot <= 0) return null;
  return name.substring(nameDot + 1);
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function PatternEditor({
  patterns,
  onChange,
  label = "Ingest patterns",
  selectedNode,
}: PatternEditorProps) {
  const [draft, setDraft] = useState("");

  const addPattern = (pattern?: string) => {
    const toAdd = pattern ?? draft.trim();
    if (!toAdd) return;
    if (patterns.includes(toAdd)) {
      if (!pattern) setDraft("");
      return;
    }
    onChange([...patterns, toAdd]);
    if (!pattern) setDraft("");
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

  // --- Contextual suggestions ---

  const suggestions: { label: string; pattern: string; hint?: string }[] = [];

  if (selectedNode && selectedNode.path) {
    const node = selectedNode;
    const ext = node.kind === "file" ? getExtension(node.path) : null;

    if (!node.excluded) {
      // Exclude suggestions
      if (node.kind === "file") {
        // Exclude by extension (unanchored, global)
        if (ext) {
          suggestions.push({
            label: `Exclude *.${ext}`,
            pattern: `*.${ext}`,
          });
        }
        // Exclude this specific file (anchored)
        suggestions.push({
          label: "Exclude this file",
          pattern: `/${node.path}`,
        });
      } else if (node.kind === "dir") {
        // Exclude this folder (anchored)
        suggestions.push({
          label: "Exclude this folder",
          pattern: `/${node.path}/`,
        });
      }
    } else {
      // Include suggestion for excluded nodes (negation)
      suggestions.push({
        label: "Include this",
        pattern: `!/${node.path}`,
        hint: "Negation cannot re-include files under an excluded directory.",
      });
    }
  }

  return (
    <div>
      <p className="text-sm font-medium text-text mb-2">{label}</p>

      {/* Contextual suggestions */}
      {suggestions.length > 0 && (
        <div className="mb-3 p-2.5 rounded border border-accent/30 bg-accent/5">
          <div className="flex items-center gap-1.5 mb-2">
            <Crosshair className="h-3 w-3 text-accent" />
            <span className="text-xs font-medium text-accent">
              {selectedNode?.path}
            </span>
          </div>
          <div className="flex flex-wrap gap-1.5">
            {suggestions.map((s) => (
              <button
                key={s.pattern}
                onClick={() => addPattern(s.pattern)}
                disabled={patterns.includes(s.pattern)}
                className={cn(
                  "rounded border px-2 py-1 text-xs font-mono",
                  patterns.includes(s.pattern)
                    ? "border-border-muted text-text-muted cursor-not-allowed"
                    : "border-accent/40 text-accent hover:bg-accent/10",
                )}
                title={s.hint ?? `Add pattern: ${s.pattern}`}
              >
                {s.label}
              </button>
            ))}
          </div>
          {suggestions.some((s) => s.hint) && (
            <p className="text-xs text-text-muted mt-1.5 italic">
              {suggestions.find((s) => s.hint)?.hint}
            </p>
          )}
        </div>
      )}

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
          onClick={() => addPattern()}
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
