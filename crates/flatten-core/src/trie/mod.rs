// crates/flatten-core/src/trie/mod.rs
//
// Arena-backed file trie with BLAKE3 Merkle hashes.
//
// The trie is the hashed picture of a registered repo. Built by ingest,
// read by export and watch, updated by watch on placement.
//
// Merkle encoding (pinned, do not change without bumping format version):
// Per child in sorted-by-name order (byte order, String::cmp):
//   [name_len: u32 LE] [name_bytes: name_len bytes] [child_hash: 32 bytes]
// Concatenated, then hashed with BLAKE3.
// Empty directory: BLAKE3 of empty input.

pub mod error;
mod path;
mod persist;

pub use path::path_from_os;
use error::{Error, Result};
use path::validate_path;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Index into the arena Vec. u32 covers ~4 billion nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct NodeIndex(u32);

/// Root is always arena[0].
const ROOT: NodeIndex = NodeIndex(0);

/// Arena node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum NodeKind {
    Leaf {
        content_hash: [u8; 32],
        size: u64,
        mtime: i64, // nanoseconds since epoch
    },
    Dir {
        /// Sorted by name in byte order (String::cmp, locale-independent).
        children: Vec<(String, NodeIndex)>,
        merkle_hash: [u8; 32],
    },
    Free {
        next: Option<NodeIndex>, // free list pointer
    },
}

/// Leaf metadata returned by `Trie::get()` and accepted by `insert` / `from_leaves`.
#[derive(Clone, Debug, PartialEq)]
pub struct LeafNode {
    pub content_hash: [u8; 32],
    pub size: u64,
    pub mtime: i64, // nanoseconds since epoch
}

/// Arena-backed file trie with BLAKE3 Merkle hashes.
///
/// Always-clean invariant: every insert and remove eagerly recomputes
/// ancestor Merkle hashes. All accessors are `&self` and infallible.
///
/// `PartialEq` is structural (arena layout), not logical. Two tries with
/// identical leaves but different insertion orders may compare unequal.
/// Use `root_hash()` for logical equality.
#[derive(Clone, Debug, PartialEq)]
pub struct Trie {
    arena: Vec<NodeKind>,
    free_head: Option<NodeIndex>,
}

impl Default for Trie {
    /// Same as `Trie::new()`. Satisfies `clippy::new_without_default`.
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Merkle hashing
// ---------------------------------------------------------------------------

/// Compute the Merkle hash for a directory from its children.
///
/// Encoding (pinned): per child in sorted order,
///   `[name_len: u32 LE] [name_bytes] [child_hash: 32 bytes]`
/// concatenated, then BLAKE3-hashed.
fn compute_merkle_hash(children: &[(String, NodeIndex)], arena: &[NodeKind]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    for (name, child_idx) in children {
        let name_bytes = name.as_bytes();
        hasher.update(&(name_bytes.len() as u32).to_le_bytes());
        hasher.update(name_bytes);
        let child_hash = match &arena[child_idx.0 as usize] {
            NodeKind::Leaf { content_hash, .. } => *content_hash,
            NodeKind::Dir { merkle_hash, .. } => *merkle_hash,
            NodeKind::Free { .. } => [0u8; 32], // Defensive; validated trees never hit this
        };
        hasher.update(&child_hash);
    }
    *hasher.finalize().as_bytes()
}

/// BLAKE3 hash of empty input. Used for empty directories.
fn empty_merkle_hash() -> [u8; 32] {
    *blake3::hash(b"").as_bytes()
}

// ---------------------------------------------------------------------------
// Trie implementation
// ---------------------------------------------------------------------------

impl Trie {
    /// Create an empty trie. Root dir is allocated at `arena[0]`.
    pub fn new() -> Self {
        Trie {
            arena: vec![NodeKind::Dir {
                children: Vec::new(),
                merkle_hash: empty_merkle_hash(),
            }],
            free_head: None,
        }
    }

    // --- Private helpers ---

    /// Allocate a new arena slot, reusing from the free list if available.
    fn alloc_node(&mut self, kind: NodeKind) -> Result<NodeIndex> {
        if let Some(free_idx) = self.free_head {
            if let NodeKind::Free { next } = self.arena[free_idx.0 as usize] {
                self.free_head = next;
                self.arena[free_idx.0 as usize] = kind;
                return Ok(free_idx);
            }
            // Defensive: free_head pointed at a non-Free node. Clear and fall through.
            self.free_head = None;
        }
        let idx = u32::try_from(self.arena.len()).map_err(|_| Error::ArenaFull)?;
        self.arena.push(kind);
        Ok(NodeIndex(idx))
    }

    /// Add a node to the free list.
    fn free_node(&mut self, idx: NodeIndex) {
        self.arena[idx.0 as usize] = NodeKind::Free {
            next: self.free_head,
        };
        self.free_head = Some(idx);
    }

    /// Recursively free a node and all its descendants.
    fn free_subtree(&mut self, idx: NodeIndex) {
        let child_indices: Vec<NodeIndex> = match &self.arena[idx.0 as usize] {
            NodeKind::Dir { children, .. } => children.iter().map(|(_, ci)| *ci).collect(),
            _ => Vec::new(),
        };
        for ci in child_indices {
            self.free_subtree(ci);
        }
        self.free_node(idx);
    }

    /// Find a child by name in a Dir node. Returns None if the parent is not
    /// a Dir or the child is not found.
    fn find_child(&self, parent: NodeIndex, name: &str) -> Option<NodeIndex> {
        match &self.arena[parent.0 as usize] {
            NodeKind::Dir { children, .. } => children
                .binary_search_by(|(n, _)| n.as_str().cmp(name))
                .ok()
                .map(|pos| children[pos].1),
            _ => None,
        }
    }

    /// Insert a child into a Dir node's children list in sorted position.
    fn add_child(&mut self, parent: NodeIndex, name: String, child: NodeIndex) {
        if let NodeKind::Dir { children, .. } = &mut self.arena[parent.0 as usize] {
            match children.binary_search_by(|(n, _)| n.as_str().cmp(&name)) {
                Ok(pos) => children[pos].1 = child, // Replace existing
                Err(pos) => children.insert(pos, (name, child)),
            }
        }
    }

    /// Remove a child by name from a Dir node's children list.
    fn remove_child(&mut self, parent: NodeIndex, name: &str) {
        if let NodeKind::Dir { children, .. } = &mut self.arena[parent.0 as usize]
            && let Ok(pos) = children.binary_search_by(|(n, _)| n.as_str().cmp(name))
        {
            children.remove(pos);
        }
    }

    /// Recompute the Merkle hash for a single Dir node from its children.
    fn recompute_merkle(&mut self, idx: NodeIndex) {
        let new_hash = {
            match &self.arena[idx.0 as usize] {
                NodeKind::Dir { children, .. } => compute_merkle_hash(children, &self.arena),
                _ => return, // Not a dir (Leaf or Free)
            }
        };
        if let NodeKind::Dir { merkle_hash, .. } = &mut self.arena[idx.0 as usize] {
            *merkle_hash = new_hash;
        }
    }

    /// Resolve a path to a node index. Returns None for invalid or missing paths.
    /// Empty string resolves to ROOT.
    fn resolve_path(&self, path: &str) -> Option<NodeIndex> {
        if path.is_empty() {
            return Some(ROOT);
        }
        if validate_path(path).is_err() {
            return None;
        }
        let mut current = ROOT;
        for segment in path.split('/') {
            current = self.find_child(current, segment)?;
        }
        Some(current)
    }

    // --- Public mutation ---

    /// Insert a leaf without recomputing Merkle hashes. Returns the ancestor
    /// indices so the caller can recompute selectively.
    ///
    /// Used by `insert` (recomputes the returned ancestors) and `from_leaves`
    /// (skips per-insert recompute, does one post-order pass at the end).
    fn insert_unhashed(&mut self, path: &str, leaf: LeafNode) -> Result<Vec<NodeIndex>> {
        validate_path(path)?;

        let segments: Vec<&str> = path.split('/').collect();
        let mut ancestors: Vec<NodeIndex> = vec![ROOT];
        let mut current = ROOT;

        // Navigate/create intermediate directories
        for &segment in &segments[..segments.len() - 1] {
            let child = self.find_child(current, segment);
            match child {
                Some(idx) => {
                    if matches!(&self.arena[idx.0 as usize], NodeKind::Dir { .. }) {
                        current = idx;
                    } else {
                        // Leaf-becomes-dir: overwrite with empty dir
                        self.arena[idx.0 as usize] = NodeKind::Dir {
                            children: Vec::new(),
                            merkle_hash: empty_merkle_hash(),
                        };
                        current = idx;
                    }
                }
                None => {
                    let new_idx = self.alloc_node(NodeKind::Dir {
                        children: Vec::new(),
                        merkle_hash: empty_merkle_hash(),
                    })?;
                    self.add_child(current, segment.to_string(), new_idx);
                    current = new_idx;
                }
            }
            ancestors.push(current);
        }

        // Insert or replace the leaf at the final segment
        let last_segment = segments[segments.len() - 1];
        let child = self.find_child(current, last_segment);
        match child {
            Some(idx) => {
                // If replacing a directory, free its entire subtree first
                let child_indices: Vec<NodeIndex> = match &self.arena[idx.0 as usize] {
                    NodeKind::Dir { children, .. } => {
                        children.iter().map(|(_, ci)| *ci).collect()
                    }
                    _ => Vec::new(),
                };
                for ci in child_indices {
                    self.free_subtree(ci);
                }
                self.arena[idx.0 as usize] = NodeKind::Leaf {
                    content_hash: leaf.content_hash,
                    size: leaf.size,
                    mtime: leaf.mtime,
                };
            }
            None => {
                let new_idx = self.alloc_node(NodeKind::Leaf {
                    content_hash: leaf.content_hash,
                    size: leaf.size,
                    mtime: leaf.mtime,
                })?;
                self.add_child(current, last_segment.to_string(), new_idx);
            }
        }

        Ok(ancestors)
    }

    /// Recursively recompute Merkle hashes for a subtree, post-order.
    /// Children are recomputed before their parent, so each directory sees
    /// up-to-date child hashes. O(N) in subtree size.
    fn recompute_subtree(&mut self, idx: NodeIndex) {
        let child_indices: Vec<NodeIndex> = match &self.arena[idx.0 as usize] {
            NodeKind::Dir { children, .. } => children.iter().map(|(_, ci)| *ci).collect(),
            _ => return,
        };
        for ci in child_indices {
            self.recompute_subtree(ci);
        }
        self.recompute_merkle(idx);
    }

    // --- Public construction ---

    /// Bulk-build a trie from an iterator of `(path, leaf)` pairs.
    ///
    /// Inserts all leaves without per-insert Merkle recomputation, then does
    /// one post-order pass over the entire tree. O(N) total hashing instead
    /// of O(N x depth). Duplicate paths: last wins.
    pub fn from_leaves(
        leaves: impl IntoIterator<Item = (String, LeafNode)>,
    ) -> Result<Trie> {
        let mut trie = Trie::new();
        for (path, leaf) in leaves {
            trie.insert_unhashed(&path, leaf)?;
        }
        trie.recompute_subtree(ROOT);
        Ok(trie)
    }

    // --- Public mutation ---

    /// Insert or replace a leaf. Intermediate directories created as needed.
    ///
    /// If the path currently names a directory, it is replaced (leaf wins);
    /// the old subtree is freed. If a prefix of the path is currently a leaf,
    /// it becomes a directory. Eagerly recomputes ancestor Merkle hashes.
    pub fn insert(&mut self, path: &str, leaf: LeafNode) -> Result<()> {
        let ancestors = self.insert_unhashed(path, leaf)?;
        for &idx in ancestors.iter().rev() {
            self.recompute_merkle(idx);
        }
        Ok(())
    }

    /// Remove a leaf. Returns `Ok(true)` if removed, `Ok(false)` if the path
    /// did not exist or names a directory (leaf-only, no subtree removal).
    ///
    /// Prunes empty ancestor directories up to (not including) root.
    /// Eagerly recomputes ancestor Merkle hashes.
    pub fn remove(&mut self, path: &str) -> Result<bool> {
        validate_path(path)?;

        let segments: Vec<&str> = path.split('/').collect();

        // Walk the path, collecting node indices.
        // path_nodes[0] = ROOT, path_nodes[i+1] = node for segments[i].
        let mut path_nodes: Vec<NodeIndex> = vec![ROOT];
        let mut current = ROOT;

        for &segment in &segments[..segments.len() - 1] {
            match self.find_child(current, segment) {
                Some(idx)
                    if matches!(&self.arena[idx.0 as usize], NodeKind::Dir { .. }) =>
                {
                    path_nodes.push(idx);
                    current = idx;
                }
                _ => return Ok(false), // Path doesn't exist or intermediate is not a dir
            }
        }

        // Check the target (last segment)
        let last = segments[segments.len() - 1];
        let target = match self.find_child(current, last) {
            Some(idx) if matches!(&self.arena[idx.0 as usize], NodeKind::Leaf { .. }) => idx,
            _ => return Ok(false), // Not found or is a directory
        };

        // Remove the leaf from its parent
        self.remove_child(current, last);
        self.free_node(target);

        // Prune empty ancestor dirs bottom-up (skip root, which is path_nodes[0]).
        // path_nodes[i+1] is the node for segments[i], parented by path_nodes[i].
        for i in (0..segments.len() - 1).rev() {
            let dir_idx = path_nodes[i + 1];
            let is_empty = matches!(
                &self.arena[dir_idx.0 as usize],
                NodeKind::Dir { children, .. } if children.is_empty()
            );
            if !is_empty {
                break;
            }
            let parent_idx = path_nodes[i];
            self.remove_child(parent_idx, segments[i]);
            self.free_node(dir_idx);
        }

        // Recompute Merkle for surviving path nodes bottom-up
        for &idx in path_nodes.iter().rev() {
            if matches!(&self.arena[idx.0 as usize], NodeKind::Dir { .. }) {
                self.recompute_merkle(idx);
            }
        }

        Ok(true)
    }

    // --- Public queries ---

    /// Get leaf metadata. None if not found or is a directory.
    /// Invalid paths treated as not-found.
    pub fn get(&self, path: &str) -> Option<LeafNode> {
        let idx = self.resolve_path(path)?;
        match &self.arena[idx.0 as usize] {
            NodeKind::Leaf {
                content_hash,
                size,
                mtime,
            } => Some(LeafNode {
                content_hash: *content_hash,
                size: *size,
                mtime: *mtime,
            }),
            _ => None, // Dir or Free
        }
    }

    /// True if the path is a leaf (not a directory). Invalid paths return false.
    pub fn has(&self, path: &str) -> bool {
        self.resolve_path(path)
            .map(|idx| matches!(&self.arena[idx.0 as usize], NodeKind::Leaf { .. }))
            .unwrap_or(false)
    }

    /// Content hash of a leaf. None if not found or is a directory.
    /// Invalid paths return None.
    pub fn leaf_hash(&self, path: &str) -> Option<[u8; 32]> {
        self.get(path).map(|n| n.content_hash)
    }

    /// Merkle hash of a directory subtree. None if path is a leaf or not found.
    /// Empty string returns `Some(root_hash())`. Invalid paths return None.
    pub fn subtree_hash(&self, dir: &str) -> Option<[u8; 32]> {
        let idx = self.resolve_path(dir)?;
        match &self.arena[idx.0 as usize] {
            NodeKind::Dir { merkle_hash, .. } => Some(*merkle_hash),
            _ => None, // Leaf or Free
        }
    }

    /// Root Merkle hash. Identifies the whole repo state.
    pub fn root_hash(&self) -> [u8; 32] {
        match &self.arena[ROOT.0 as usize] {
            NodeKind::Dir { merkle_hash, .. } => *merkle_hash,
            _ => [0u8; 32], // Defensive; root is always a Dir
        }
    }

    /// True if the leaf exists and its `(size, mtime)` match the given values.
    /// Used by stat-based refresh to decide whether to re-hash a file.
    pub fn stat_matches(&self, path: &str, size: u64, mtime: i64) -> bool {
        self.get(path)
            .map(|n| n.size == size && n.mtime == mtime)
            .unwrap_or(false)
    }

    /// All leaf paths under the given directory prefix, sorted by byte order
    /// of the full `/`-joined path. Subtree walk, then `sort_unstable`.
    ///
    /// Empty prefix returns all leaves. Prefix naming a leaf returns that one
    /// path. Invalid prefix returns empty `Vec`.
    pub fn list(&self, prefix: &str) -> Vec<String> {
        let (start_idx, path_prefix) = if prefix.is_empty() {
            (ROOT, String::new())
        } else {
            if validate_path(prefix).is_err() {
                return Vec::new();
            }
            match self.resolve_path(prefix) {
                Some(idx) => (idx, prefix.to_string()),
                None => return Vec::new(),
            }
        };

        // If start is a leaf, return just that path
        if matches!(&self.arena[start_idx.0 as usize], NodeKind::Leaf { .. }) {
            return vec![path_prefix];
        }

        let mut result = Vec::new();
        self.collect_leaves(start_idx, &path_prefix, &mut result);
        result.sort_unstable();
        result
    }

    /// Recursively collect leaf paths from a subtree.
    fn collect_leaves(&self, idx: NodeIndex, prefix: &str, result: &mut Vec<String>) {
        match &self.arena[idx.0 as usize] {
            NodeKind::Dir { children, .. } => {
                for (name, child_idx) in children {
                    let child_path = if prefix.is_empty() {
                        name.clone()
                    } else {
                        format!("{prefix}/{name}")
                    };
                    self.collect_leaves(*child_idx, &child_path, result);
                }
            }
            NodeKind::Leaf { .. } => {
                result.push(prefix.to_string());
            }
            NodeKind::Free { .. } => {} // Defensive; never reached in valid tree
        }
    }

    // --- Persistence ---

    /// Serialize to MessagePack with a fixed binary header.
    /// Atomic write: tempfile in the same directory, fsync, then rename.
    /// Caller is responsible for creating the parent directory.
    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        persist::save(self, path)
    }

    /// Deserialize from MessagePack. Validates header, format version,
    /// and structural integrity.
    pub fn load(path: &std::path::Path) -> Result<Trie> {
        persist::load(path)
    }

    /// Number of arena slots (including free). Test-only.
    #[cfg(test)]
    fn arena_len(&self) -> usize {
        self.arena.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Helper: leaf with given hash, zero stat metadata.
    fn leaf(hash: [u8; 32]) -> LeafNode {
        LeafNode {
            content_hash: hash,
            size: 0,
            mtime: 0,
        }
    }

    /// Helper: leaf with given hash, size, and mtime.
    fn leaf_with_stat(hash: [u8; 32], size: u64, mtime: i64) -> LeafNode {
        LeafNode {
            content_hash: hash,
            size,
            mtime,
        }
    }

    // --- Core CRUD ---

    #[test]
    fn insert_get_roundtrip() {
        let mut trie = Trie::new();
        let h = [42u8; 32];
        let l = leaf_with_stat(h, 1024, 1_000_000_000);
        trie.insert("src/main.rs", l.clone())
            .expect("insert should succeed");

        let got = trie.get("src/main.rs").expect("should find the leaf");
        assert_eq!(got.content_hash, h, "content_hash mismatch");
        assert_eq!(got.size, 1024, "size mismatch");
        assert_eq!(got.mtime, 1_000_000_000, "mtime mismatch");
        assert_eq!(got, l, "full LeafNode mismatch");
    }

    #[test]
    fn insert_remove_absent() {
        let mut trie = Trie::new();
        trie.insert("a/b", leaf([1u8; 32])).unwrap();
        assert!(trie.has("a/b"), "should exist before remove");

        let removed = trie.remove("a/b").unwrap();
        assert!(removed, "remove should return true");
        assert_eq!(trie.get("a/b"), None, "should be absent after remove");
    }

    #[test]
    fn insert_replaces_existing_leaf() {
        let mut trie = Trie::new();
        let h1 = [1u8; 32];
        let h2 = [2u8; 32];
        trie.insert("f", leaf(h1)).unwrap();
        let root_before = trie.root_hash();
        let arena_before = trie.arena_len();

        trie.insert("f", leaf(h2)).unwrap();
        assert_eq!(
            trie.leaf_hash("f"),
            Some(h2),
            "leaf should have the new hash after re-insert"
        );
        assert_ne!(
            trie.root_hash(),
            root_before,
            "root_hash should change after replacing a leaf"
        );
        assert_eq!(
            trie.arena_len(),
            arena_before,
            "arena should not grow when replacing an existing leaf"
        );
    }

    #[test]
    fn remove_prunes_empty_ancestors() {
        let mut trie = Trie::new();
        let empty_root = trie.root_hash();

        trie.insert("a/b/c", leaf([1u8; 32])).unwrap();
        assert!(trie.subtree_hash("a").is_some(), "a should be a dir");
        assert!(trie.subtree_hash("a/b").is_some(), "a/b should be a dir");

        trie.remove("a/b/c").unwrap();

        assert_eq!(
            trie.subtree_hash("a"),
            None,
            "a should be pruned (not found)"
        );
        assert_eq!(
            trie.subtree_hash("a/b"),
            None,
            "a/b should be pruned (not found)"
        );
        assert_eq!(
            trie.root_hash(),
            empty_root,
            "root hash should match empty trie after removing all leaves"
        );
    }

    #[test]
    fn remove_missing_returns_false() {
        let mut trie = Trie::new();
        let result = trie.remove("nonexistent").unwrap();
        assert!(!result, "remove on missing path should return Ok(false)");

        // Also test removing through a leaf intermediate: "a" is a leaf,
        // "a/b" does not exist because "a" is not a directory.
        trie.insert("a", leaf([1u8; 32])).unwrap();
        let result = trie.remove("a/b").unwrap();
        assert!(
            !result,
            "remove through a leaf intermediate should return Ok(false)"
        );
        assert!(trie.has("a"), "the leaf at 'a' should be untouched");
    }

    #[test]
    fn remove_directory_returns_false() {
        let mut trie = Trie::new();
        trie.insert("dir/file", leaf([1u8; 32])).unwrap();
        let result = trie.remove("dir").unwrap();
        assert!(
            !result,
            "remove on a directory path should return Ok(false)"
        );
        assert!(trie.has("dir/file"), "file should still exist");
    }

    #[test]
    fn remove_invalid_path() {
        let mut trie = Trie::new();
        let result = trie.remove("a/../b");
        assert!(
            matches!(result, Err(Error::InvalidPath(_))),
            "remove with .. should return InvalidPath"
        );
    }

    #[test]
    fn merkle_changes_on_remove() {
        let mut trie = Trie::new();
        trie.insert("file", leaf([1u8; 32])).unwrap();
        let hash_before = trie.root_hash();

        trie.remove("file").unwrap();
        assert_ne!(
            trie.root_hash(),
            hash_before,
            "root_hash should change after remove"
        );
    }

    #[test]
    fn remove_recomputes_intermediate_dirs() {
        // Remove a/b while a/c survives. Proves intermediate dir "a" and root
        // are both recomputed, not just root.
        let mut trie = Trie::new();
        trie.insert("a/b", leaf([1u8; 32])).unwrap();
        trie.insert("a/c", leaf([2u8; 32])).unwrap();

        trie.remove("a/b").unwrap();

        // Build a reference trie with only a/c
        let mut expected = Trie::new();
        expected.insert("a/c", leaf([2u8; 32])).unwrap();

        assert_eq!(
            trie.root_hash(),
            expected.root_hash(),
            "root_hash after removing a/b should equal a trie with only a/c"
        );
    }

    #[test]
    fn replace_directory_with_leaf() {
        let mut trie = Trie::new();
        trie.insert("a/b", leaf([1u8; 32])).unwrap();
        trie.insert("a/c", leaf([2u8; 32])).unwrap();
        assert!(trie.subtree_hash("a").is_some(), "a should be a dir");

        let arena_before = trie.arena_len();

        // Replace directory "a" with a leaf
        trie.insert("a", leaf([3u8; 32])).unwrap();
        assert!(trie.has("a"), "a should now be a leaf");
        assert_eq!(trie.get("a/b"), None, "a/b should be gone");
        assert_eq!(trie.get("a/c"), None, "a/c should be gone");

        // Insert two more leaves; both freed slots should be reused
        trie.insert("x", leaf([4u8; 32])).unwrap();
        trie.insert("y", leaf([5u8; 32])).unwrap();
        assert_eq!(
            trie.arena_len(),
            arena_before,
            "arena should not grow; freed slots should be reused (before={arena_before}, after={})",
            trie.arena_len()
        );
    }

    #[test]
    fn free_list_reuse() {
        let mut trie = Trie::new();
        trie.insert("a", leaf([1u8; 32])).unwrap();
        trie.insert("b", leaf([2u8; 32])).unwrap();
        let size_after_insert = trie.arena_len();

        trie.remove("a").unwrap();
        trie.remove("b").unwrap();

        // Re-insert: should reuse freed slots
        trie.insert("c", leaf([3u8; 32])).unwrap();
        trie.insert("d", leaf([4u8; 32])).unwrap();
        assert_eq!(
            trie.arena_len(),
            size_after_insert,
            "arena should not grow after reusing freed slots"
        );
    }

    #[test]
    fn arc_clone_preserves_old_state() {
        let mut trie = Trie::new();
        trie.insert("file", leaf([1u8; 32])).unwrap();

        let mut shared = Arc::new(trie);
        let snapshot = Arc::clone(&shared);

        // Mutate via make_mut (copy-on-write)
        let trie_mut = Arc::make_mut(&mut shared);
        trie_mut.insert("new_file", leaf([2u8; 32])).unwrap();

        // Snapshot should be unchanged
        assert!(snapshot.has("file"), "snapshot should still have 'file'");
        assert!(
            !snapshot.has("new_file"),
            "snapshot should not have 'new_file'"
        );
    }

    // --- Queries (from chunk 2, unchanged) ---

    #[test]
    fn has_leaf_true() {
        let mut trie = Trie::new();
        trie.insert("a.txt", leaf([1u8; 32]))
            .expect("insert should succeed");
        assert!(trie.has("a.txt"), "has should return true for a leaf");
    }

    #[test]
    fn has_directory_false() {
        let mut trie = Trie::new();
        trie.insert("dir/file.txt", leaf([1u8; 32]))
            .expect("insert should succeed");
        assert!(
            !trie.has("dir"),
            "has should return false for a directory"
        );
    }

    #[test]
    fn leaf_hash_returns_content_hash() {
        let mut trie = Trie::new();
        let h = [99u8; 32];
        trie.insert("f.txt", leaf(h)).expect("insert should succeed");
        assert_eq!(
            trie.leaf_hash("f.txt"),
            Some(h),
            "leaf_hash should return the content_hash"
        );
    }

    #[test]
    fn subtree_hash_returns_merkle() {
        let h = [1u8; 32];
        let mut trie = Trie::new();
        trie.insert("dir/a.txt", leaf(h))
            .expect("insert should succeed");

        let hash = trie.subtree_hash("dir");
        assert!(hash.is_some(), "subtree_hash should return Some for a directory");

        // Cross-check: subtree_hash("dir") must equal root_hash() of a fresh
        // trie containing only "a.txt" with the same content hash, because both
        // are the Merkle of the single child ("a.txt", h).
        let mut fresh = Trie::new();
        fresh.insert("a.txt", leaf(h)).expect("insert should succeed");
        assert_eq!(
            hash.unwrap(),
            fresh.root_hash(),
            "subtree Merkle should equal root Merkle of equivalent standalone trie"
        );
    }

    #[test]
    fn subtree_hash_empty_string_is_root() {
        let mut trie = Trie::new();
        trie.insert("a.txt", leaf([1u8; 32]))
            .expect("insert should succeed");
        assert_eq!(
            trie.subtree_hash(""),
            Some(trie.root_hash()),
            "subtree_hash('') should equal root_hash()"
        );
    }

    #[test]
    fn subtree_hash_on_leaf_returns_none() {
        let mut trie = Trie::new();
        trie.insert("f.txt", leaf([1u8; 32]))
            .expect("insert should succeed");
        assert_eq!(
            trie.subtree_hash("f.txt"),
            None,
            "subtree_hash on a leaf should return None"
        );
    }

    #[test]
    fn stat_metadata_roundtrip() {
        let mut trie = Trie::new();
        let l = leaf_with_stat([0u8; 32], 4096, 1_718_000_000_123_456_789);
        trie.insert("file", l.clone()).expect("insert should succeed");

        let got = trie.get("file").expect("should find the leaf");
        assert_eq!(got.size, 4096, "size should roundtrip");
        assert_eq!(got.mtime, 1_718_000_000_123_456_789, "mtime should roundtrip");

        // Two mtimes differing only in sub-second portion are distinct
        let l2 = leaf_with_stat([0u8; 32], 4096, 1_718_000_000_123_456_790);
        trie.insert("file2", l2).expect("insert should succeed");
        let got2 = trie.get("file2").expect("should find file2");
        assert_ne!(
            got.mtime, got2.mtime,
            "sub-second mtime differences should be preserved"
        );
    }

    #[test]
    fn stat_matches_true_when_equal() {
        let mut trie = Trie::new();
        trie.insert("f", leaf_with_stat([0u8; 32], 100, 999))
            .expect("insert should succeed");
        assert!(
            trie.stat_matches("f", 100, 999),
            "stat_matches should return true for matching size and mtime"
        );
    }

    #[test]
    fn stat_matches_false_when_different() {
        let mut trie = Trie::new();
        trie.insert("f", leaf_with_stat([0u8; 32], 100, 999))
            .expect("insert should succeed");
        assert!(
            !trie.stat_matches("f", 101, 999),
            "stat_matches should return false for different size"
        );
        assert!(
            !trie.stat_matches("f", 100, 1000),
            "stat_matches should return false for different mtime"
        );
        assert!(
            !trie.stat_matches("nonexistent", 100, 999),
            "stat_matches should return false for missing path"
        );
    }

    #[test]
    fn case_sensitive_paths() {
        let mut trie = Trie::new();
        trie.insert("Foo.rs", leaf([1u8; 32]))
            .expect("insert Foo.rs should succeed");
        trie.insert("foo.rs", leaf([2u8; 32]))
            .expect("insert foo.rs should succeed");
        assert_ne!(
            trie.leaf_hash("Foo.rs"),
            trie.leaf_hash("foo.rs"),
            "Foo.rs and foo.rs should be distinct"
        );
        assert!(trie.has("Foo.rs"), "Foo.rs should exist");
        assert!(trie.has("foo.rs"), "foo.rs should exist");
    }

    #[test]
    fn query_invalid_path_returns_none() {
        let mut trie = Trie::new();
        trie.insert("a", leaf([1u8; 32]))
            .expect("insert should succeed");
        assert_eq!(trie.get("a/../b"), None, "get with .. should return None");
        assert!(!trie.has("a/../b"), "has with .. should return false");
        assert_eq!(
            trie.leaf_hash("/a"),
            None,
            "leaf_hash with leading slash should return None"
        );
        assert_eq!(
            trie.subtree_hash("a/"),
            None,
            "subtree_hash with trailing slash should return None"
        );
    }

    // --- Merkle integrity ---

    #[test]
    fn merkle_changes_on_insert() {
        let trie_empty = Trie::new();
        let mut trie = Trie::new();
        trie.insert("file", leaf([1u8; 32]))
            .expect("insert should succeed");
        assert_ne!(
            trie.root_hash(),
            trie_empty.root_hash(),
            "root_hash should change after insert"
        );
    }

    #[test]
    fn merkle_deterministic() {
        let mut t1 = Trie::new();
        t1.insert("b/x", leaf([1u8; 32])).unwrap();
        t1.insert("a/y", leaf([2u8; 32])).unwrap();

        let mut t2 = Trie::new();
        t2.insert("a/y", leaf([2u8; 32])).unwrap();
        t2.insert("b/x", leaf([1u8; 32])).unwrap();

        assert_eq!(
            t1.root_hash(),
            t2.root_hash(),
            "same inserts in different order should produce the same root_hash"
        );
    }

    #[test]
    fn merkle_golden_hash() {
        // Case 1: two root-level leaves.
        // Hand-compute expected root hash from the encoding spec.
        let mut trie = Trie::new();
        trie.insert("a", leaf([0u8; 32])).unwrap();
        trie.insert("b", leaf([1u8; 32])).unwrap();

        let mut expected_input = Vec::new();
        // Child "a": len=1, name=0x61, hash=[0;32]
        expected_input.extend_from_slice(&1u32.to_le_bytes());
        expected_input.extend_from_slice(b"a");
        expected_input.extend_from_slice(&[0u8; 32]);
        // Child "b": len=1, name=0x62, hash=[1;32]
        expected_input.extend_from_slice(&1u32.to_le_bytes());
        expected_input.extend_from_slice(b"b");
        expected_input.extend_from_slice(&[1u8; 32]);

        let expected: [u8; 32] = *blake3::hash(&expected_input).as_bytes();
        assert_eq!(
            trie.root_hash(),
            expected,
            "golden hash case 1: two root-level leaves"
        );

        // Case 2: nested directory. Locks that a Dir child contributes its
        // merkle_hash (not its name, not zeros).
        let mut trie2 = Trie::new();
        let leaf_hash = [7u8; 32];
        trie2.insert("d/x", leaf(leaf_hash)).unwrap();

        // Inner: dir "d" has one child "x" with content_hash leaf_hash.
        let mut inner_input = Vec::new();
        inner_input.extend_from_slice(&1u32.to_le_bytes());
        inner_input.extend_from_slice(b"x");
        inner_input.extend_from_slice(&leaf_hash);
        let inner_hash: [u8; 32] = *blake3::hash(&inner_input).as_bytes();

        // Outer: root has one child "d" with merkle_hash = inner_hash.
        let mut outer_input = Vec::new();
        outer_input.extend_from_slice(&1u32.to_le_bytes());
        outer_input.extend_from_slice(b"d");
        outer_input.extend_from_slice(&inner_hash);
        let outer_hash: [u8; 32] = *blake3::hash(&outer_input).as_bytes();

        assert_eq!(
            trie2.root_hash(),
            outer_hash,
            "golden hash case 2: nested directory"
        );
    }

    // --- Replace semantics ---

    #[test]
    fn replace_leaf_with_directory() {
        let mut trie = Trie::new();
        trie.insert("a", leaf([1u8; 32])).unwrap();
        assert!(trie.has("a"), "a should be a leaf");

        trie.insert("a/b", leaf([2u8; 32])).unwrap();
        assert!(!trie.has("a"), "a should no longer be a leaf");
        assert!(trie.has("a/b"), "a/b should be a leaf");
        assert!(
            trie.subtree_hash("a").is_some(),
            "a should now be a directory"
        );
    }

    // --- Path validation ---

    #[test]
    fn invalid_path_empty() {
        let mut trie = Trie::new();
        let result = trie.insert("", leaf([0u8; 32]));
        assert!(
            matches!(result, Err(Error::InvalidPath(_))),
            "empty path should return InvalidPath"
        );
    }

    #[test]
    fn invalid_path_dotdot() {
        let mut trie = Trie::new();
        let result = trie.insert("a/../b", leaf([0u8; 32]));
        assert!(
            matches!(result, Err(Error::InvalidPath(_))),
            ".. in path should return InvalidPath"
        );
    }

    #[test]
    fn invalid_path_dot() {
        let mut trie = Trie::new();
        let result = trie.insert("./a", leaf([0u8; 32]));
        assert!(
            matches!(result, Err(Error::InvalidPath(_))),
            ". in path should return InvalidPath"
        );
    }

    #[test]
    fn invalid_path_leading_slash() {
        let mut trie = Trie::new();
        let result = trie.insert("/a", leaf([0u8; 32]));
        assert!(
            matches!(result, Err(Error::InvalidPath(_))),
            "leading slash should return InvalidPath"
        );
    }

    #[test]
    fn invalid_path_trailing_slash() {
        let mut trie = Trie::new();
        let result = trie.insert("a/", leaf([0u8; 32]));
        assert!(
            matches!(result, Err(Error::InvalidPath(_))),
            "trailing slash should return InvalidPath"
        );
    }

    #[test]
    fn invalid_path_double_slash() {
        let mut trie = Trie::new();
        let result = trie.insert("a//b", leaf([0u8; 32]));
        assert!(
            matches!(result, Err(Error::InvalidPath(_))),
            "double slash should return InvalidPath"
        );
    }

    // --- Bulk construction ---

    #[test]
    fn from_leaves_bulk() {
        // Build via from_leaves and via sequential insert, compare root hashes.
        let entries: Vec<(String, LeafNode)> = vec![
            ("a/x".into(), leaf([1u8; 32])),
            ("b".into(), leaf([2u8; 32])),
            ("a/y".into(), leaf([3u8; 32])),
        ];

        let bulk = Trie::from_leaves(entries.clone()).expect("from_leaves should succeed");

        let mut sequential = Trie::new();
        for (path, l) in &entries {
            sequential.insert(path, l.clone()).unwrap();
        }

        assert_eq!(
            bulk.root_hash(),
            sequential.root_hash(),
            "from_leaves root_hash should match sequential inserts"
        );
    }

    #[test]
    fn from_leaves_duplicate_last_wins() {
        let entries: Vec<(String, LeafNode)> = vec![
            ("f".into(), leaf([1u8; 32])),
            ("f".into(), leaf([2u8; 32])),
        ];

        let trie = Trie::from_leaves(entries).expect("from_leaves should succeed");
        assert_eq!(
            trie.leaf_hash("f"),
            Some([2u8; 32]),
            "duplicate paths: last value should win"
        );
    }

    #[test]
    fn from_leaves_invalid_path() {
        let entries: Vec<(String, LeafNode)> = vec![
            ("good".into(), leaf([1u8; 32])),
            ("../bad".into(), leaf([2u8; 32])),
        ];

        let result = Trie::from_leaves(entries);
        assert!(
            matches!(result, Err(Error::InvalidPath(_))),
            "from_leaves with invalid path should return InvalidPath"
        );
    }

    // --- List ---

    #[test]
    fn list_prefix_sorted() {
        let mut trie = Trie::new();
        // "a-b" and "a/c" distinguish byte-order sort from traversal order:
        // traversal visits child "a" (→ src/a/c) before "a-b" (→ src/a-b),
        // but byte order has '-' (0x2D) < '/' (0x2F), so src/a-b < src/a/c.
        trie.insert("src/a-b", leaf([1u8; 32])).unwrap();
        trie.insert("src/a/c", leaf([2u8; 32])).unwrap();

        let paths = trie.list("src");
        assert_eq!(
            paths,
            vec!["src/a-b", "src/a/c"],
            "list should sort by byte order of full path, not traversal order"
        );
    }

    #[test]
    fn list_empty_prefix_returns_all() {
        let mut trie = Trie::new();
        trie.insert("b", leaf([1u8; 32])).unwrap();
        trie.insert("a/x", leaf([2u8; 32])).unwrap();
        trie.insert("a/y", leaf([3u8; 32])).unwrap();

        let paths = trie.list("");
        assert_eq!(
            paths,
            vec!["a/x", "a/y", "b"],
            "empty prefix should return all leaves, sorted"
        );
    }

    #[test]
    fn list_prefix_is_leaf() {
        let mut trie = Trie::new();
        trie.insert("single-file", leaf([1u8; 32])).unwrap();
        trie.insert("other", leaf([2u8; 32])).unwrap();

        let paths = trie.list("single-file");
        assert_eq!(
            paths,
            vec!["single-file"],
            "list on a leaf should return that one path"
        );
    }

    #[test]
    fn list_invalid_prefix_returns_empty() {
        let mut trie = Trie::new();
        trie.insert("a", leaf([1u8; 32])).unwrap();

        let paths = trie.list("a/../b");
        assert!(
            paths.is_empty(),
            "list with invalid prefix should return empty Vec"
        );
    }

    // --- Concurrency and construction ---

    #[test]
    fn default_equals_new() {
        assert_eq!(
            Trie::default(),
            Trie::new(),
            "Default should produce the same trie as new()"
        );
    }

    #[test]
    fn send_sync_static_assertion() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Trie>();
    }
}
