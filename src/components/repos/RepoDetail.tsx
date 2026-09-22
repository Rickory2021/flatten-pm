// src/components/repos/RepoDetail.tsx
//
// Repo detail view: header, view mode toggle (accordion/flat/graph),
// action bar, pattern editing with contextual suggestions, file preview,
// excluded toggle, gitignore re-import diff, delete with inline confirm.

import { useEffect, useState, useCallback } from "react";
import {
  ArrowLeft,
  TreePine,
  List,
  GitGraph,
  RefreshCw,
  Trash2,
  Loader2,
  Eye,
  EyeOff,
  FileDown,
} from "lucide-react";
import { cn } from "@/lib/utils";
import {
  repoList,
  repoTree,
  repoEdit,
  repoReingest,
  repoDelete,
  repoExcludedFiles,
} from "@/lib/tauri";
import type { RepoRow } from "@/lib/types";
import { isCommandError } from "@/lib/types";
import { buildTree, type TreeNode } from "@/lib/tree";
import { AccordionTree } from "@/components/repos/AccordionTree";
import { FlatList } from "@/components/repos/FlatList";
import { GraphTree } from "@/components/repos/GraphTree";
import { FilePreview } from "@/components/repos/FilePreview";
import { PatternEditor } from "@/components/repos/PatternEditor";
import { GitignoreDiff } from "@/components/repos/GitignoreDiff";

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface RepoDetailProps {
  repoId: number;
  onBack: () => void;
  onDeleted: () => void;
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type ViewMode = "accordion" | "flat" | "graph";

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function RepoDetail({ repoId, onBack, onDeleted }: RepoDetailProps) {
  // Repo data
  const [repo, setRepo] = useState<RepoRow | null>(null);
  const [treePaths, setTreePaths] = useState<string[]>([]);
  const [loadError, setLoadError] = useState("");

  // View mode
  const [viewMode, setViewMode] = useState<ViewMode>("accordion");

  // Excluded files
  const [showExcluded, setShowExcluded] = useState(false);
  const [excludedPaths, setExcludedPaths] = useState<string[]>([]);
  const [excludedLoading, setExcludedLoading] = useState(false);

  // File preview
  const [previewPath, setPreviewPath] = useState<string | null>(null);

  // Pattern editing
  const [editingPatterns, setEditingPatterns] = useState(false);
  const [draftPatterns, setDraftPatterns] = useState<string[]>([]);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState("");

  // Gitignore diff
  const [showGitignoreDiff, setShowGitignoreDiff] = useState(false);

  // Re-ingest
  const [reingesting, setReingesting] = useState(false);

  // Delete
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);

  // Selected node for contextual pattern suggestions
  const [selectedNode, setSelectedNode] = useState<TreeNode | null>(null);

  // --- Fetch data ---

  const fetchData = useCallback(async () => {
    try {
      const repos = await repoList();
      const found = repos.find((r) => r.id === repoId);
      if (!found) {
        setLoadError("Repo not found.");
        return;
      }
      setRepo(found);

      const paths = await repoTree(repoId);
      setTreePaths(paths);
    } catch (e: unknown) {
      setLoadError(isCommandError(e) ? e.error : "Failed to load repo.");
    }
  }, [repoId]);

  useEffect(() => {
    fetchData();
  }, [fetchData]);

  // --- Excluded files ---

  useEffect(() => {
    if (!showExcluded) {
      setExcludedPaths([]);
      return;
    }
    let cancelled = false;
    setExcludedLoading(true);
    repoExcludedFiles(repoId)
      .then((paths) => {
        if (!cancelled) {
          setExcludedPaths(paths);
          setExcludedLoading(false);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setExcludedPaths([]);
          setExcludedLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [showExcluded, repoId]);

  // --- Node selection -> file preview ---

  const handleNodeSelect = (node: TreeNode) => {
    setSelectedNode(node);
    if (node.kind === "file" && !node.excluded) {
      setPreviewPath(node.path);
    }
  };

  // --- Pattern editing ---

  const startEditing = () => {
    if (!repo) return;
    setDraftPatterns([...repo.ingest_patterns]);
    setEditingPatterns(true);
    setShowGitignoreDiff(false);
    setSaveError("");
  };

  const cancelEditing = () => {
    setEditingPatterns(false);
    setSaveError("");
    setSelectedNode(null);
  };

  const savePatterns = async () => {
    setSaving(true);
    setSaveError("");
    try {
      await repoEdit(repoId, undefined, draftPatterns, undefined);
      setSaving(false);
      setEditingPatterns(false);
      setSelectedNode(null);
      await fetchData();
    } catch (e: unknown) {
      setSaveError(isCommandError(e) ? e.error : "Save failed.");
      setSaving(false);
    }
  };

  // --- Gitignore diff ---

  const handleGitignoreApplied = async () => {
    setShowGitignoreDiff(false);
    await fetchData();
  };

  // --- Re-ingest ---

  const handleReingest = async () => {
    setReingesting(true);
    try {
      await repoReingest(repoId);
      await fetchData();
    } catch {
      // Reingest errors are transient
    }
    setReingesting(false);
  };

  // --- Delete ---

  const handleDelete = async () => {
    setDeleting(true);
    try {
      await repoDelete(repoId);
      onDeleted();
      onBack();
    } catch {
      setDeleting(false);
      setConfirmingDelete(false);
    }
  };

  // --- Build tree ---

  const tree = buildTree(
    treePaths,
    showExcluded ? excludedPaths : undefined,
  );

  // --- Loading/error ---

  if (loadError) {
    return (
      <div>
        <button
          onClick={onBack}
          className="inline-flex items-center gap-1 text-sm text-text-muted hover:text-text mb-4"
        >
          <ArrowLeft className="h-3.5 w-3.5" />
          Back to list
        </button>
        <p className="text-danger text-sm">{loadError}</p>
      </div>
    );
  }

  if (!repo) {
    return <p className="text-text-muted text-sm">Loading...</p>;
  }

  // --- Render ---

  return (
    <div>
      {/* Back link */}
      <button
        onClick={onBack}
        className="inline-flex items-center gap-1 text-sm text-text-muted hover:text-text mb-4"
      >
        <ArrowLeft className="h-3.5 w-3.5" />
        Back to list
      </button>

      {/* Header */}
      <div className="mb-4">
        <h2 className="text-lg font-semibold text-text">{repo.name}</h2>
        <p className="text-sm text-text-muted font-mono truncate" title={repo.path}>
          {repo.path}
        </p>
        <p className="text-xs text-text-muted mt-0.5">
          {treePaths.length} file{treePaths.length === 1 ? "" : "s"}
          {showExcluded && excludedPaths.length > 0 && (
            <span className="text-warning">
              , {excludedPaths.length} excluded
            </span>
          )}
          {repo.trie_updated_at && (
            <span>
              {" -- "}last ingested{" "}
              {new Date(repo.trie_updated_at).toLocaleString(undefined, {
                month: "short",
                day: "numeric",
                hour: "2-digit",
                minute: "2-digit",
              })}
            </span>
          )}
        </p>
      </div>

      {/* Action bar */}
      <div className="flex items-center gap-2 mb-4 flex-wrap">
        {/* View mode toggle */}
        <div className="flex items-center border border-border rounded overflow-hidden">
          <button
            onClick={() => setViewMode("accordion")}
            className={cn(
              "px-2.5 py-1 text-xs flex items-center gap-1",
              viewMode === "accordion"
                ? "bg-accent text-white"
                : "bg-surface text-text-muted hover:text-text",
            )}
            title="Accordion tree"
          >
            <TreePine className="h-3 w-3" />
            Tree
          </button>
          <button
            onClick={() => setViewMode("flat")}
            className={cn(
              "px-2.5 py-1 text-xs flex items-center gap-1",
              viewMode === "flat"
                ? "bg-accent text-white"
                : "bg-surface text-text-muted hover:text-text",
            )}
            title="Flat list"
          >
            <List className="h-3 w-3" />
            Flat
          </button>
          <button
            onClick={() => setViewMode("graph")}
            className={cn(
              "px-2.5 py-1 text-xs flex items-center gap-1",
              viewMode === "graph"
                ? "bg-accent text-white"
                : "bg-surface text-text-muted hover:text-text",
            )}
            title="Graph tree"
          >
            <GitGraph className="h-3 w-3" />
            Graph
          </button>
        </div>

        {/* Show excluded toggle */}
        <button
          onClick={() => setShowExcluded(!showExcluded)}
          disabled={excludedLoading}
          className={cn(
            "inline-flex items-center gap-1 rounded px-2.5 py-1 text-xs",
            "border border-border",
            showExcluded
              ? "text-warning border-warning"
              : "text-text-muted hover:text-text hover:border-accent",
          )}
        >
          {excludedLoading ? (
            <Loader2 className="h-3 w-3 animate-spin" />
          ) : showExcluded ? (
            <EyeOff className="h-3 w-3" />
          ) : (
            <Eye className="h-3 w-3" />
          )}
          {showExcluded ? "Hide excluded" : "Show excluded"}
        </button>

        {/* Re-ingest */}
        <button
          onClick={handleReingest}
          disabled={reingesting}
          className={cn(
            "inline-flex items-center gap-1 rounded px-2.5 py-1 text-xs",
            "border border-border text-text-muted hover:text-text hover:border-accent",
          )}
        >
          {reingesting ? (
            <Loader2 className="h-3 w-3 animate-spin" />
          ) : (
            <RefreshCw className="h-3 w-3" />
          )}
          Re-ingest
        </button>

        {/* Edit patterns */}
        {!editingPatterns && (
          <button
            onClick={startEditing}
            className={cn(
              "rounded px-2.5 py-1 text-xs",
              "border border-border text-text-muted hover:text-text hover:border-accent",
            )}
          >
            Edit patterns
          </button>
        )}

        {/* Re-import .gitignore */}
        {!showGitignoreDiff && !editingPatterns && (
          <button
            onClick={() => setShowGitignoreDiff(true)}
            className={cn(
              "inline-flex items-center gap-1 rounded px-2.5 py-1 text-xs",
              "border border-border text-text-muted hover:text-text hover:border-accent",
            )}
          >
            <FileDown className="h-3 w-3" />
            Re-import .gitignore
          </button>
        )}

        {/* Spacer */}
        <div className="flex-1" />

        {/* Delete */}
        {!confirmingDelete ? (
          <button
            onClick={() => setConfirmingDelete(true)}
            className={cn(
              "inline-flex items-center gap-1 rounded px-2.5 py-1 text-xs",
              "border border-border text-text-muted hover:text-danger hover:border-danger",
            )}
          >
            <Trash2 className="h-3 w-3" />
            Delete
          </button>
        ) : (
          <div className="flex items-center gap-2">
            <span className="text-xs text-danger">Are you sure?</span>
            <button
              onClick={handleDelete}
              disabled={deleting}
              className={cn(
                "rounded px-2.5 py-1 text-xs font-medium",
                "bg-danger text-white hover:opacity-90",
              )}
            >
              {deleting ? "Deleting..." : "Confirm"}
            </button>
            <button
              onClick={() => setConfirmingDelete(false)}
              className="rounded px-2.5 py-1 text-xs text-text-muted hover:text-text"
            >
              Cancel
            </button>
          </div>
        )}
      </div>

      {/* Gitignore diff (when shown) */}
      {showGitignoreDiff && (
        <div className="mb-4">
          <GitignoreDiff
            repoId={repoId}
            repoPath={repo.path}
            storedPatterns={repo.ingest_patterns}
            onClose={() => setShowGitignoreDiff(false)}
            onApplied={handleGitignoreApplied}
          />
        </div>
      )}

      {/* Pattern editor (when editing) */}
      {editingPatterns && (
        <div className="mb-4 p-4 rounded border border-border bg-surface">
          <PatternEditor
            patterns={draftPatterns}
            onChange={setDraftPatterns}
            selectedNode={selectedNode}
          />
          {saveError && (
            <p className="text-xs text-danger mt-2">{saveError}</p>
          )}
          <div className="flex items-center gap-2 mt-3">
            <button
              onClick={savePatterns}
              disabled={saving}
              className={cn(
                "inline-flex items-center gap-1 rounded px-3 py-1.5 text-sm font-medium",
                "focus:outline-none focus:ring-1 focus:ring-accent",
                saving
                  ? "bg-border text-text-muted cursor-not-allowed"
                  : "bg-accent text-white hover:opacity-90",
              )}
            >
              {saving && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              {saving ? "Saving..." : "Save patterns"}
            </button>
            <button
              onClick={cancelEditing}
              className="text-sm text-text-muted hover:text-text"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Main content: view + file preview side by side */}
      <div className={cn("flex gap-4", previewPath ? "items-start" : "")}>
        {/* View content */}
        <div className={cn(previewPath ? "flex-1 min-w-0" : "w-full")}>
          {viewMode === "accordion" && (
            <AccordionTree
              root={tree}
              onSelect={handleNodeSelect}
            />
          )}

          {viewMode === "flat" && (
            <FlatList paths={treePaths} />
          )}

          {viewMode === "graph" && (
            <GraphTree
              root={tree}
              onSelect={handleNodeSelect}
            />
          )}
        </div>

        {/* File preview panel */}
        {previewPath && (
          <div className="w-96 shrink-0">
            <FilePreview
              repoId={repoId}
              relativePath={previewPath}
              onClose={() => setPreviewPath(null)}
            />
          </div>
        )}
      </div>
    </div>
  );
}
