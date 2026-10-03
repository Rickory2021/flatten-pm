// src/lib/tree.ts
//
// Tree data structure for repo file views. Shared by AccordionTree,
// GraphTree, and any component that needs a nested tree from flat paths.

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** A node in the repo file tree. */
export interface TreeNode {
  /** Segment name (e.g. "src", "main.rs"). */
  name: string;
  /** Full relative path from repo root (e.g. "src/main.rs"). */
  path: string;
  /** "file" for leaves, "dir" for intermediate directories. */
  kind: "file" | "dir";
  /** Sorted children by name. Empty for files. */
  children: TreeNode[];
  /** Recursive count of included (non-excluded) files under this dir. */
  fileCount: number;
  /** Recursive count of excluded files under this dir. */
  excludedCount: number;
  /** True if this path was in the excluded set (not in the trie). */
  excluded: boolean;
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/**
 * Build a nested tree from flat path lists.
 *
 * @param includedPaths - Paths in the trie (from repoTree or repoPreview).
 * @param excludedPaths - Paths on disk but not in the trie (from repoExcludedFiles). Optional.
 * @returns A root TreeNode representing the repo root directory.
 *
 * The root node has `name: ""`, `path: ""`, `kind: "dir"`.
 * `fileCount` on a directory counts only non-excluded leaves in its subtree.
 * `excludedCount` counts only excluded leaves. Both are computed in a
 * single bottom-up pass after tree construction.
 */
export function buildTree(
  includedPaths: string[],
  excludedPaths?: string[],
): TreeNode {
  // Root node
  const root: TreeNode = {
    name: "",
    path: "",
    kind: "dir",
    children: [],
    fileCount: 0,
    excludedCount: 0,
    excluded: false,
  };

  // Insert all included paths
  for (const p of includedPaths) {
    insertPath(root, p, false);
  }

  // Insert excluded paths
  if (excludedPaths) {
    for (const p of excludedPaths) {
      insertPath(root, p, true);
    }
  }

  // Compute counts bottom-up
  computeCounts(root);

  return root;
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/**
 * Insert a path into the tree, creating intermediate directory nodes as
 * needed. Maintains sorted children order via binary search insertion.
 */
function insertPath(root: TreeNode, path: string, excluded: boolean): void {
  const segments = path.split("/");
  let current = root;

  for (let i = 0; i < segments.length; i++) {
    const segment = segments[i];
    const isLast = i === segments.length - 1;
    const childPath = current.path ? `${current.path}/${segment}` : segment;

    // Find or create child
    const existingIdx = binarySearch(current.children, segment);

    if (existingIdx >= 0) {
      // Child exists
      const existing = current.children[existingIdx];
      if (isLast) {
        // If the existing node is a dir and we're inserting a file at the
        // same path, this shouldn't happen in practice (a path can't be
        // both a file and a directory). Skip to avoid corrupting the tree.
        if (existing.kind === "file") {
          existing.excluded = excluded;
        }
      } else {
        // Navigate into existing directory
        current = existing;
      }
    } else {
      // Create new node
      const insertIdx = -(existingIdx + 1);
      if (isLast) {
        // Leaf (file)
        const leaf: TreeNode = {
          name: segment,
          path: childPath,
          kind: "file",
          children: [],
          fileCount: 0,
          excludedCount: 0,
          excluded,
        };
        current.children.splice(insertIdx, 0, leaf);
      } else {
        // Intermediate directory
        const dir: TreeNode = {
          name: segment,
          path: childPath,
          kind: "dir",
          children: [],
          fileCount: 0,
          excludedCount: 0,
          excluded: false,
        };
        current.children.splice(insertIdx, 0, dir);
        current = dir;
      }
    }
  }
}

/**
 * Binary search for a child by name. Returns the index if found (>= 0),
 * or -(insertionPoint + 1) if not found (matching Java's Arrays.binarySearch
 * convention).
 */
function binarySearch(children: TreeNode[], name: string): number {
  let lo = 0;
  let hi = children.length - 1;

  while (lo <= hi) {
    const mid = (lo + hi) >>> 1;
    const cmp = children[mid].name.localeCompare(name);
    if (cmp < 0) {
      lo = mid + 1;
    } else if (cmp > 0) {
      hi = mid - 1;
    } else {
      return mid;
    }
  }

  return -(lo + 1);
}

/**
 * Bottom-up pass to compute fileCount and excludedCount on every directory.
 * fileCount = included files only. excludedCount = excluded files only.
 */
function computeCounts(node: TreeNode): void {
  if (node.kind === "file") {
    // Leaf: counts are 0 on files (they don't have children).
    // The parent aggregates based on the excluded flag.
    return;
  }

  let included = 0;
  let excluded = 0;

  for (const child of node.children) {
    computeCounts(child);
    if (child.kind === "file") {
      if (child.excluded) {
        excluded += 1;
      } else {
        included += 1;
      }
    } else {
      included += child.fileCount;
      excluded += child.excludedCount;
    }
  }

  node.fileCount = included;
  node.excludedCount = excluded;
}
