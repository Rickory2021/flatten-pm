// src/components/repos/FilePreview.tsx
//
// Read-only file preview panel. Calls repoReadFile and displays content
// based on the FilePreviewResult.kind discriminator.

import { useEffect, useState } from "react";
import { X, FileSymlink, File, Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { repoReadFile } from "@/lib/tauri";
import type { FilePreviewResult } from "@/lib/types";
import { isCommandError } from "@/lib/types";

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface FilePreviewProps {
  repoId: number;
  /** Relative path from repo root (trie path). */
  relativePath: string;
  onClose: () => void;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function FilePreview({ repoId, relativePath, onClose }: FilePreviewProps) {
  const [result, setResult] = useState<FilePreviewResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError("");
    setResult(null);

    repoReadFile(repoId, relativePath)
      .then((r) => {
        if (!cancelled) {
          setResult(r);
          setLoading(false);
        }
      })
      .catch((e: unknown) => {
        if (!cancelled) {
          setError(isCommandError(e) ? e.error : "Failed to read file.");
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [repoId, relativePath]);

  return (
    <div className="border border-border rounded bg-surface overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-border-muted bg-surface">
        <div className="flex items-center gap-2 min-w-0">
          {result?.kind === "symlink" ? (
            <FileSymlink className="h-3.5 w-3.5 text-warning shrink-0" />
          ) : (
            <File className="h-3.5 w-3.5 text-text-muted shrink-0" />
          )}
          <span
            className="text-sm font-mono text-text truncate"
            title={relativePath}
          >
            {relativePath}
          </span>
        </div>
        <button
          onClick={onClose}
          className="shrink-0 text-text-muted hover:text-text ml-2"
          title="Close preview"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      {/* Content */}
      <div className="max-h-80 overflow-auto">
        {loading && (
          <div className="flex items-center gap-2 px-3 py-6 text-sm text-text-muted">
            <Loader2 className="h-4 w-4 animate-spin" />
            Loading...
          </div>
        )}

        {error && (
          <p className="px-3 py-4 text-sm text-danger">{error}</p>
        )}

        {result && !loading && (
          <>
            {result.kind === "symlink" ? (
              <div className="px-3 py-4">
                <p className="text-sm text-text-muted mb-1">Symlink target:</p>
                <p className="text-sm font-mono text-text">{result.content}</p>
              </div>
            ) : (
              <pre
                className={cn(
                  "px-3 py-3 text-xs font-mono text-text whitespace-pre overflow-x-auto",
                  "leading-relaxed",
                )}
              >
                {result.content}
              </pre>
            )}
          </>
        )}
      </div>
    </div>
  );
}
