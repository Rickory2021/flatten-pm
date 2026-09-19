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
mod persist;

use error::{Error, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Index into the arena Vec. u32 covers ~4 billion nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct NodeIndex(u32);

/// Root is always arena[0].
const ROOT: NodeIndex = NodeIndex(0);

/// Arena node. Leaf and Dir only in this chunk; Free added in chunk 3.
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
/// Always-clean invariant: every insert eagerly recomputes ancestor Merkle
/// hashes. All accessors are `&self` and infallible.
///
/// `PartialEq` is structural (arena layout), not logical. Two tries with
/// identical leaves but different insertion orders may compare unequal.
/// Use `root_hash()` for logical equality.
#[derive(Clone, Debug, PartialEq)]
pub struct Trie {
    arena: Vec<NodeKind>,
    // free_head added in chunk 3 with the Free variant
}

impl Default for Trie {
    /// Same as `Trie::new()`. Satisfies `clippy::new_without_default`.
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Path validation
// ---------------------------------------------------------------------------

/// Validate a trie path: relative, forward-slash separated, no `.`/`..`,
/// no empty segments, no leading/trailing slash.
fn validate_path(path: &str) -> Result<()> {
    if path.is_empty() {
        return Err(Error::InvalidPath("empty path".into()));
    }
    if path.starts_with('/') {
        return Err(Error::InvalidPath(format!("leading slash: {path}")));
    }
    if path.ends_with('/') {
        return Err(Error::InvalidPath(format!("trailing slash: {path}")));
    }
    for segment in path.split('/') {
        if segment.is_empty() {
            return Err(Error::InvalidPath(format!("empty segment in: {path}")));
        }
        if segment == "." {
            return Err(Error::InvalidPath(format!("'.' segment in: {path}")));
        }
        if segment == ".." {
            return Err(Error::InvalidPath(format!("'..' segment in: {path}")));
        }
    }
    Ok(())
}

/// Convert a `Path` to a trie-compatible `String`.
///
/// Iterates `std::path::Component` variants. `Normal` components are converted
/// lossily. `RootDir`, `ParentDir`, and `Prefix` return `Error::InvalidPath`.
/// Leading `CurDir` (from a `./` prefix) also returns `Error::InvalidPath`;
/// interior `.` segments are normalized away by `Path::components()`.
///
/// Returns `Ok((trie_path, was_lossy))`. Caller decides how to surface the
/// lossy warning.
pub fn path_from_os(path: &std::path::Path) -> Result<(String, bool)> {
    use std::path::Component;

    let mut parts = Vec::new();
    let mut was_lossy = false;

    for component in path.components() {
        match component {
            Component::Normal(os_str) => match os_str.to_str() {
                Some(s) => parts.push(s.to_string()),
                None => {
                    was_lossy = true;
                    parts.push(os_str.to_string_lossy().into_owned());
                }
            },
            Component::CurDir => {
                return Err(Error::InvalidPath(
                    "path contains leading '.' component".into(),
                ));
            }
            Component::ParentDir => {
                return Err(Error::InvalidPath(
                    "path contains '..' component".into(),
                ));
            }
            Component::RootDir => {
                return Err(Error::InvalidPath("path is absolute".into()));
            }
            Component::Prefix(_) => {
                return Err(Error::InvalidPath(
                    "path contains Windows prefix".into(),
                ));
            }
        }
    }

    if parts.is_empty() {
        return Err(Error::InvalidPath("empty path".into()));
    }

    Ok((parts.join("/"), was_lossy))
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
        }
    }

    // --- Private helpers ---

    /// Allocate a new arena slot. Returns the index.
    fn alloc_node(&mut self, kind: NodeKind) -> Result<NodeIndex> {
        let idx = u32::try_from(self.arena.len()).map_err(|_| Error::ArenaFull)?;
        self.arena.push(kind);
        Ok(NodeIndex(idx))
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
    /// Caller must ensure the name is not already present.
    fn add_child(&mut self, parent: NodeIndex, name: String, child: NodeIndex) {
        if let NodeKind::Dir { children, .. } = &mut self.arena[parent.0 as usize] {
            let pos = children
                .binary_search_by(|(n, _)| n.as_str().cmp(&name))
                .unwrap_or_else(|pos| pos);
            children.insert(pos, (name, child));
        }
    }

    /// Recompute the Merkle hash for a single Dir node from its children.
    fn recompute_merkle(&mut self, idx: NodeIndex) {
        let new_hash = {
            match &self.arena[idx.0 as usize] {
                NodeKind::Dir { children, .. } => compute_merkle_hash(children, &self.arena),
                _ => return, // Not a dir
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

    /// Insert or replace a leaf. Intermediate directories created as needed.
    ///
    /// If the path currently names a directory, it is replaced (leaf wins).
    /// If a prefix of the path is currently a leaf, it becomes a directory.
    /// Eagerly recomputes ancestor Merkle hashes.
    pub fn insert(&mut self, path: &str, leaf: LeafNode) -> Result<()> {
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
                // Replace existing node (dir-becomes-leaf orphans subtree;
                // chunk 3 adds free_subtree before this overwrite)
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

        // Recompute Merkle hashes bottom-up
        for &idx in ancestors.iter().rev() {
            self.recompute_merkle(idx);
        }

        Ok(())
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
            _ => None,
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
            _ => None,
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
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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

    // --- path_from_os ---

    #[cfg(unix)]
    #[test]
    fn lossy_path_conversion() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::path::Path;

        // 0xFF is not valid UTF-8
        let os_str = OsStr::from_bytes(&[0xFF]);
        let path = Path::new(os_str);
        let (result, was_lossy) = path_from_os(path).expect("should not error");
        assert!(was_lossy, "non-UTF-8 should be flagged as lossy");
        assert!(
            result.contains('\u{FFFD}'),
            "lossy conversion should contain replacement character"
        );
    }

    #[test]
    fn path_from_os_joins_components() {
        let path = std::path::Path::new("a").join("b").join("c");
        let (result, was_lossy) = path_from_os(&path).expect("should succeed");
        assert_eq!(result, "a/b/c", "components should be joined with /");
        assert!(!was_lossy, "valid UTF-8 should not be lossy");
    }

    #[test]
    fn path_from_os_rejects_parent_dir() {
        let path = std::path::Path::new("../a");
        let result = path_from_os(path);
        assert!(
            matches!(result, Err(Error::InvalidPath(_))),
            ".. component should return InvalidPath"
        );
    }

    // --- Concurrency ---

    #[test]
    fn send_sync_static_assertion() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Trie>();
    }
}
