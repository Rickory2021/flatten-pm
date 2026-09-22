// src/components/repos/GraphTree.tsx
//
// Node-and-edge tree diagram of the trie structure. Custom SVG with
// recursive horizontal layout. Pan via mouse drag, zoom via wheel.
//
// WebKit note: wheel events use { passive: false } via addEventListener
// (not React's onWheel) so preventDefault works.

import React, { useRef, useEffect, useState, useCallback } from "react";
import type { TreeNode } from "@/lib/tree";

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface GraphTreeProps {
  root: TreeNode;
  onSelect?: (node: TreeNode) => void;
}

// ---------------------------------------------------------------------------
// Layout constants
// ---------------------------------------------------------------------------

const NODE_H = 24;
const NODE_PAD_X = 12;
const ROW_GAP = 4;
const COL_GAP = 32;
const FONT_SIZE = 12;
const CHAR_WIDTH = 7.2; // approximate monospace char width at 12px
const CORNER_R = 4;

// ---------------------------------------------------------------------------
// Layout types
// ---------------------------------------------------------------------------

interface LayoutNode {
  node: TreeNode;
  x: number;
  y: number;
  w: number;
  h: number;
  children: LayoutNode[];
}

// ---------------------------------------------------------------------------
// Layout algorithm
// ---------------------------------------------------------------------------

/**
 * Recursively lay out the tree horizontally. Root on the left, children
 * branch to the right. Returns the LayoutNode with computed positions.
 * `yOffset` tracks the current vertical cursor.
 */
function layoutTree(
  node: TreeNode,
  x: number,
  yOffset: { value: number },
  collapsed: Set<string>,
): LayoutNode {
  const label = node.name || "(root)";
  const w = label.length * CHAR_WIDTH + NODE_PAD_X * 2;
  const h = NODE_H;

  // If leaf or collapsed dir, single row
  if (node.kind === "file" || collapsed.has(node.path)) {
    const y = yOffset.value;
    yOffset.value += h + ROW_GAP;
    return { node, x, y, w, h, children: [] };
  }

  // Directory: lay out children recursively
  const childX = x + w + COL_GAP;
  const childLayouts: LayoutNode[] = [];

  if (node.children.length === 0) {
    const y = yOffset.value;
    yOffset.value += h + ROW_GAP;
    return { node, x, y, w, h, children: [] };
  }

  for (const child of node.children) {
    childLayouts.push(layoutTree(child, childX, yOffset, collapsed));
  }

  // Center this node vertically among its children
  const firstChild = childLayouts[0];
  const lastChild = childLayouts[childLayouts.length - 1];
  const childTop = firstChild.y;
  const childBottom = lastChild.y + lastChild.h;
  const y = childTop + (childBottom - childTop - h) / 2;

  return { node, x, y, w, h, children: childLayouts };
}

// ---------------------------------------------------------------------------
// SVG renderers
// ---------------------------------------------------------------------------

function renderEdges(layout: LayoutNode): React.ReactElement[] {
  const edges: React.ReactElement[] = [];

  for (const child of layout.children) {
    const x1 = layout.x + layout.w;
    const y1 = layout.y + layout.h / 2;
    const x2 = child.x;
    const y2 = child.y + child.h / 2;
    const mx = x1 + (x2 - x1) / 2;

    edges.push(
      <path
        key={`edge-${layout.node.path}-${child.node.path}`}
        d={`M ${x1} ${y1} C ${mx} ${y1}, ${mx} ${y2}, ${x2} ${y2}`}
        fill="none"
        stroke="var(--app-border)"
        strokeWidth={1}
      />,
    );

    edges.push(...renderEdges(child));
  }

  return edges;
}

function renderNodes(
  layout: LayoutNode,
  onSelect?: (node: TreeNode) => void,
  onToggle?: (path: string) => void,
): React.ReactElement[] {
  const nodes: React.ReactElement[] = [];
  const isDir = layout.node.kind === "dir";
  const label = layout.node.name || "(root)";

  nodes.push(
    <g
      key={`node-${layout.node.path}`}
      onClick={() => {
        if (isDir) onToggle?.(layout.node.path);
        onSelect?.(layout.node);
      }}
      style={{ cursor: "pointer" }}
    >
      <rect
        x={layout.x}
        y={layout.y}
        width={layout.w}
        height={layout.h}
        rx={CORNER_R}
        ry={CORNER_R}
        fill={
          layout.node.excluded
            ? "var(--app-border-muted)"
            : isDir
              ? "var(--app-surface)"
              : "var(--app-overlay)"
        }
        stroke={isDir ? "var(--app-accent)" : "var(--app-border)"}
        strokeWidth={1}
      />
      <text
        x={layout.x + NODE_PAD_X}
        y={layout.y + layout.h / 2 + FONT_SIZE * 0.35}
        fontSize={FONT_SIZE}
        fontFamily="monospace"
        fill={
          layout.node.excluded
            ? "var(--app-text-muted)"
            : "var(--app-text)"
        }
        textDecoration={layout.node.excluded ? "line-through" : undefined}
      >
        {label}
      </text>
    </g>,
  );

  for (const child of layout.children) {
    nodes.push(...renderNodes(child, onSelect, onToggle));
  }

  return nodes;
}

/**
 * Compute the bounding box of the entire layout tree.
 */
function bounds(layout: LayoutNode): { maxX: number; maxY: number } {
  let maxX = layout.x + layout.w;
  let maxY = layout.y + layout.h;

  for (const child of layout.children) {
    const cb = bounds(child);
    if (cb.maxX > maxX) maxX = cb.maxX;
    if (cb.maxY > maxY) maxY = cb.maxY;
  }

  return { maxX, maxY };
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function GraphTree({ root, onSelect }: GraphTreeProps) {
  const svgRef = useRef<SVGSVGElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  // Pan/zoom state
  const [pan, setPan] = useState({ x: 20, y: 20 });
  const [zoom, setZoom] = useState(1);
  const [dragging, setDragging] = useState(false);
  const dragStart = useRef({ x: 0, y: 0, panX: 0, panY: 0 });

  // Collapsed directories
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  const onToggle = useCallback((path: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  }, []);

  // Layout
  const yOffset = { value: 0 };
  const layout = layoutTree(root, 0, yOffset, collapsed);
  const { maxX, maxY } = bounds(layout);
  const svgW = maxX + 40;
  const svgH = maxY + 40;

  // Wheel zoom (non-passive for preventDefault on WebKit)
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const delta = e.deltaY > 0 ? 0.9 : 1.1;
      setZoom((z) => Math.min(3, Math.max(0.2, z * delta)));
    };

    container.addEventListener("wheel", onWheel, { passive: false });
    return () => container.removeEventListener("wheel", onWheel);
  }, []);

  // Mouse drag for panning
  const onMouseDown = (e: React.MouseEvent) => {
    setDragging(true);
    dragStart.current = { x: e.clientX, y: e.clientY, panX: pan.x, panY: pan.y };
  };

  const onMouseMove = (e: React.MouseEvent) => {
    if (!dragging) return;
    const dx = e.clientX - dragStart.current.x;
    const dy = e.clientY - dragStart.current.y;
    setPan({ x: dragStart.current.panX + dx, y: dragStart.current.panY + dy });
  };

  const onMouseUp = () => {
    setDragging(false);
  };

  return (
    <div
      ref={containerRef}
      className="relative overflow-hidden rounded border border-border-muted bg-surface"
      style={{ height: "400px", cursor: dragging ? "grabbing" : "grab" }}
      onMouseDown={onMouseDown}
      onMouseMove={onMouseMove}
      onMouseUp={onMouseUp}
      onMouseLeave={onMouseUp}
    >
      <svg
        ref={svgRef}
        width={svgW * zoom}
        height={svgH * zoom}
        viewBox={`0 0 ${svgW} ${svgH}`}
        style={{
          transform: `translate(${pan.x}px, ${pan.y}px)`,
          transformOrigin: "0 0",
        }}
      >
        {renderEdges(layout)}
        {renderNodes(layout, onSelect, onToggle)}
      </svg>

      {/* Zoom indicator */}
      <div className="absolute bottom-2 right-2 text-xs text-text-muted bg-surface/80 rounded px-1.5 py-0.5">
        {Math.round(zoom * 100)}%
      </div>
    </div>
  );
}
