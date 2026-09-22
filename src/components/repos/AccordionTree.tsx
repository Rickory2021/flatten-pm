// src/components/repos/AccordionTree.tsx
//
// VS Code-style collapsible folder hierarchy. Built from a TreeNode root
// via buildTree. Shows fileCount/excludedCount per directory.

import { useState } from "react";
import { ChevronRight, ChevronDown, Folder, File } from "lucide-react";
import { cn } from "@/lib/utils";
import type { TreeNode } from "@/lib/tree";

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface AccordionTreeProps {
  /** Root TreeNode from buildTree. */
  root: TreeNode;
  /** Called when a node is selected (click). */
  onSelect?: (node: TreeNode) => void;
}

// ---------------------------------------------------------------------------
// Recursive node renderer
// ---------------------------------------------------------------------------

function TreeNodeRow({
  node,
  depth,
  expanded,
  onToggle,
  onSelect,
}: {
  node: TreeNode;
  depth: number;
  expanded: Set<string>;
  onToggle: (path: string) => void;
  onSelect?: (node: TreeNode) => void;
}) {
  const isDir = node.kind === "dir";
  const isOpen = expanded.has(node.path);

  const handleClick = () => {
    if (isDir) {
      onToggle(node.path);
    }
    onSelect?.(node);
  };

  return (
    <>
      <div
        onClick={handleClick}
        className={cn(
          "flex items-center gap-1 py-0.5 pr-2 text-sm cursor-pointer",
          "hover:bg-surface rounded",
          node.excluded && "opacity-50 line-through",
        )}
        style={{ paddingLeft: `${depth * 16 + 4}px` }}
        title={node.path}
      >
        {/* Expand/collapse icon for directories */}
        {isDir ? (
          <span className="shrink-0 w-4 h-4 flex items-center justify-center">
            {isOpen ? (
              <ChevronDown className="h-3.5 w-3.5 text-text-muted" />
            ) : (
              <ChevronRight className="h-3.5 w-3.5 text-text-muted" />
            )}
          </span>
        ) : (
          <span className="shrink-0 w-4" />
        )}

        {/* Icon */}
        {isDir ? (
          <Folder className="h-3.5 w-3.5 text-accent shrink-0" />
        ) : (
          <File className="h-3.5 w-3.5 text-text-muted shrink-0" />
        )}

        {/* Name */}
        <span className={cn("truncate", node.excluded ? "text-text-muted" : "text-text")}>
          {node.name}
        </span>

        {/* Counts for directories */}
        {isDir && (
          <span className="ml-auto text-xs text-text-muted shrink-0">
            {node.fileCount}
            {node.excludedCount > 0 && (
              <span className="text-warning"> +{node.excludedCount} excluded</span>
            )}
          </span>
        )}
      </div>

      {/* Render children if expanded */}
      {isDir && isOpen && (
        <>
          {node.children.map((child) => (
            <TreeNodeRow
              key={child.path}
              node={child}
              depth={depth + 1}
              expanded={expanded}
              onToggle={onToggle}
              onSelect={onSelect}
            />
          ))}
          {node.children.length === 0 && (
            <div
              className="text-xs text-text-muted italic py-0.5"
              style={{ paddingLeft: `${(depth + 1) * 16 + 4}px` }}
            >
              (empty)
            </div>
          )}
        </>
      )}
    </>
  );
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function AccordionTree({ root, onSelect }: AccordionTreeProps) {
  const [expanded, setExpanded] = useState<Set<string>>(() => {
    // Start with root expanded
    return new Set([""]);
  });

  const onToggle = (path: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  };

  return (
    <div className="font-mono text-sm">
      {root.children.map((child) => (
        <TreeNodeRow
          key={child.path}
          node={child}
          depth={0}
          expanded={expanded}
          onToggle={onToggle}
          onSelect={onSelect}
        />
      ))}
      {root.children.length === 0 && (
        <p className="text-text-muted text-sm italic">No files.</p>
      )}
    </div>
  );
}
