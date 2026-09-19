// crates/flatten-core/src/trie/persist.rs
//
// Trie persistence: fixed binary header, MessagePack serialization,
// structural validation, atomic save, and load.
//
// File format:
//   Bytes 0..4:  b"FTRI"           (magic)
//   Bytes 4..8:  u32 LE            (format version, currently 1)
//   Bytes 8..:   MessagePack body  (TriePayload, positional encoding)
//
// Wire format stability: rmp_serde::to_vec encodes struct fields by position
// and enum variants by name. Renaming a NodeKind variant, adding/removing/
// reordering fields within a variant, or changing payload field order changes
// the on-disk format and must bump CURRENT_FORMAT_VERSION.

use super::error::{Error, Result};
use super::{NodeIndex, NodeKind, Trie, ROOT};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::Path;

const MAGIC: [u8; 4] = *b"FTRI";
const CURRENT_FORMAT_VERSION: u32 = 1;

/// Write path: borrows the arena (no clone). Serialize only.
#[derive(Serialize)]
struct TriePayloadRef<'a> {
    arena: &'a [NodeKind],
    free_head: Option<NodeIndex>,
}

/// Read path: owns the deserialized data. Deserialize only.
#[derive(Deserialize)]
struct TriePayload {
    arena: Vec<NodeKind>,
    free_head: Option<NodeIndex>,
}

// ---------------------------------------------------------------------------
// Save
// ---------------------------------------------------------------------------

/// Serialize a trie to disk with a fixed header and atomic write.
///
/// Writes to a tempfile in the same directory, fsyncs, then renames.
/// Caller is responsible for creating the parent directory.
/// All `io::Error` maps to `Error::Io`.
pub(crate) fn save(trie: &Trie, path: &Path) -> Result<()> {
    let path_str = path.display().to_string();

    let parent = path.parent().ok_or_else(|| Error::Io {
        path: path_str.clone(),
        source: std::io::Error::new(std::io::ErrorKind::NotFound, "no parent directory"),
    })?;

    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| Error::Io {
        path: path_str.clone(),
        source: e,
    })?;

    // Header
    tmp.write_all(&MAGIC).map_err(|e| Error::Io {
        path: path_str.clone(),
        source: e,
    })?;
    tmp.write_all(&CURRENT_FORMAT_VERSION.to_le_bytes())
        .map_err(|e| Error::Io {
            path: path_str.clone(),
            source: e,
        })?;

    // Body (positional MessagePack, no clone)
    let payload = TriePayloadRef {
        arena: &trie.arena,
        free_head: trie.free_head,
    };
    let body = rmp_serde::to_vec(&payload).map_err(|e| Error::Corrupt {
        path: path_str.clone(),
        reason: format!("serialization failed: {e}"),
    })?;
    tmp.write_all(&body).map_err(|e| Error::Io {
        path: path_str.clone(),
        source: e,
    })?;

    // fsync then atomic rename
    tmp.as_file().sync_all().map_err(|e| Error::Io {
        path: path_str.clone(),
        source: e,
    })?;
    tmp.persist(path).map_err(|e| Error::Io {
        path: path_str,
        source: e.error,
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Load
// ---------------------------------------------------------------------------

/// Deserialize a trie from disk. Validates header, format version, and
/// structural integrity.
///
/// - `ErrorKind::NotFound` on open → `Error::NotFound`
/// - `ErrorKind::UnexpectedEof` on header read → `Error::Corrupt`
/// - Bad magic → `Error::Corrupt`
/// - Unknown version → `Error::UnsupportedFormat`
/// - Deserialization failure → `Error::Corrupt`
/// - Structural validation failure → `Error::Corrupt`
/// - Other IO → `Error::Io`
pub(crate) fn load(path: &Path) -> Result<Trie> {
    let path_str = path.display().to_string();

    let mut file = std::fs::File::open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::NotFound {
                path: path_str.clone(),
            }
        } else {
            Error::Io {
                path: path_str.clone(),
                source: e,
            }
        }
    })?;

    // Read header
    let mut header = [0u8; 8];
    file.read_exact(&mut header).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            Error::Corrupt {
                path: path_str.clone(),
                reason: "file too short for header".into(),
            }
        } else {
            Error::Io {
                path: path_str.clone(),
                source: e,
            }
        }
    })?;

    // Validate magic
    if header[..4] != MAGIC {
        return Err(Error::Corrupt {
            path: path_str,
            reason: format!(
                "bad magic: expected {:?}, got {:?}",
                MAGIC,
                &header[..4]
            ),
        });
    }

    // Validate version
    let version = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if version != CURRENT_FORMAT_VERSION {
        return Err(Error::UnsupportedFormat {
            path: path_str,
            version,
        });
    }

    // Read body
    let mut body = Vec::new();
    file.read_to_end(&mut body).map_err(|e| Error::Io {
        path: path_str.clone(),
        source: e,
    })?;

    // Deserialize
    let payload: TriePayload =
        rmp_serde::from_slice(&body).map_err(|e| Error::Corrupt {
            path: path_str.clone(),
            reason: format!("deserialization failed: {e}"),
        })?;

    // Structural validation
    validate(&payload.arena, payload.free_head, &path_str)?;

    Ok(Trie {
        arena: payload.arena,
        free_head: payload.free_head,
    })
}

// ---------------------------------------------------------------------------
// Structural validation
// ---------------------------------------------------------------------------

/// O(N) marking pass over the arena. Returns `Error::Corrupt` on any
/// structural violation. Called by `load` after deserialization.
///
/// Rules:
/// - Arena is non-empty and arena[0] is Dir.
/// - Tree walk from root: every child index is in bounds, no child points
///   at ROOT, no Free nodes reachable, no node has two parents (DAG).
/// - Dir children are sorted by name (byte order) with no duplicates.
/// - Free list walk: every node is Free, every index is in bounds,
///   no cycles (visited flag).
/// - After both walks, every slot visited exactly once (no orphans).
fn validate(arena: &[NodeKind], free_head: Option<NodeIndex>, path: &str) -> Result<()> {
    if arena.is_empty() {
        return Err(Error::Corrupt {
            path: path.into(),
            reason: "empty arena".into(),
        });
    }

    if !matches!(&arena[0], NodeKind::Dir { .. }) {
        return Err(Error::Corrupt {
            path: path.into(),
            reason: "arena[0] is not a Dir".into(),
        });
    }

    let mut visited = vec![false; arena.len()];

    // Tree walk from root
    validate_subtree(arena, ROOT, &mut visited, path)?;

    // Free list walk
    let mut current = free_head;
    while let Some(idx) = current {
        let i = idx.0 as usize;
        if i >= arena.len() {
            return Err(Error::Corrupt {
                path: path.into(),
                reason: format!("free list index {i} out of bounds"),
            });
        }
        if visited[i] {
            return Err(Error::Corrupt {
                path: path.into(),
                reason: format!("free list cycle or shared node at index {i}"),
            });
        }
        visited[i] = true;
        match &arena[i] {
            NodeKind::Free { next } => current = *next,
            _ => {
                return Err(Error::Corrupt {
                    path: path.into(),
                    reason: format!("free list node at index {i} is not Free"),
                });
            }
        }
    }

    // Orphan check
    for (i, v) in visited.iter().enumerate() {
        if !v {
            return Err(Error::Corrupt {
                path: path.into(),
                reason: format!("orphan node at index {i}"),
            });
        }
    }

    Ok(())
}

/// Recursive tree walk for validation. Checks bounds, visited (DAG detection),
/// children sort order, ROOT references, and Free nodes in the tree.
fn validate_subtree(
    arena: &[NodeKind],
    idx: NodeIndex,
    visited: &mut [bool],
    path: &str,
) -> Result<()> {
    let i = idx.0 as usize;
    if i >= arena.len() {
        return Err(Error::Corrupt {
            path: path.into(),
            reason: format!("node index {i} out of bounds"),
        });
    }
    if visited[i] {
        return Err(Error::Corrupt {
            path: path.into(),
            reason: format!("node at index {i} has two parents"),
        });
    }
    visited[i] = true;

    match &arena[i] {
        NodeKind::Dir { children, .. } => {
            // Children must be sorted by name with no duplicates
            for window in children.windows(2) {
                if window[0].0 >= window[1].0 {
                    return Err(Error::Corrupt {
                        path: path.into(),
                        reason: format!(
                            "unsorted or duplicate children in Dir at index {i}"
                        ),
                    });
                }
            }
            for (_, child_idx) in children {
                if *child_idx == ROOT {
                    return Err(Error::Corrupt {
                        path: path.into(),
                        reason: "child points at root (index 0)".into(),
                    });
                }
                validate_subtree(arena, *child_idx, visited, path)?;
            }
        }
        NodeKind::Leaf { .. } => {}
        NodeKind::Free { .. } => {
            return Err(Error::Corrupt {
                path: path.into(),
                reason: format!("tree walk reached Free node at index {i}"),
            });
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trie::{empty_merkle_hash, LeafNode};

    fn leaf(hash: [u8; 32]) -> LeafNode {
        LeafNode {
            content_hash: hash,
            size: 0,
            mtime: 0,
        }
    }

    /// Write a raw trie file with arbitrary header and body bytes.
    fn write_raw(path: &Path, magic: &[u8], version: u32, body: &[u8]) {
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(magic).unwrap();
        f.write_all(&version.to_le_bytes()).unwrap();
        f.write_all(body).unwrap();
    }

    /// Serialize an arena and free_head as a valid MessagePack body.
    fn valid_body(arena: &[NodeKind], free_head: Option<NodeIndex>) -> Vec<u8> {
        let payload = TriePayloadRef { arena, free_head };
        rmp_serde::to_vec(&payload).unwrap()
    }

    #[test]
    fn msgpack_serialize_deserialize() {
        let mut trie = Trie::new();
        trie.insert("a/b", leaf([1u8; 32])).unwrap();
        trie.insert("c", leaf([2u8; 32])).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.trie");

        save(&trie, &path).expect("save should succeed");
        let loaded = load(&path).expect("load should succeed");

        assert_eq!(trie, loaded, "save then load should roundtrip exactly");
    }

    #[test]
    fn corrupt_file_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.trie");
        std::fs::write(&path, b"garbage").unwrap();

        let result = load(&path);
        assert!(
            matches!(result, Err(Error::Corrupt { .. })),
            "truncated garbage file should be Corrupt, got {result:?}"
        );
    }

    #[test]
    fn corrupt_file_bad_magic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.trie");
        write_raw(&path, b"NOPE", CURRENT_FORMAT_VERSION, &[0u8; 16]);

        let result = load(&path);
        assert!(
            matches!(result, Err(Error::Corrupt { .. })),
            "bad magic should be Corrupt, got {result:?}"
        );
    }

    #[test]
    fn unsupported_format_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("future.trie");
        // Valid magic, wrong version, some body bytes so the file is long enough
        write_raw(&path, &MAGIC, 999, &[0u8; 16]);

        let result = load(&path);
        assert!(
            matches!(result, Err(Error::UnsupportedFormat { version: 999, .. })),
            "unknown version should be UnsupportedFormat, got {result:?}"
        );
    }

    #[test]
    fn load_rejects_structurally_invalid() {
        let dir = tempfile::tempdir().unwrap();

        // Case 1: child index out of bounds
        {
            let arena = vec![NodeKind::Dir {
                children: vec![("a".into(), NodeIndex(99))],
                merkle_hash: [0u8; 32],
            }];
            let path = dir.path().join("oob.trie");
            write_raw(
                &path,
                &MAGIC,
                CURRENT_FORMAT_VERSION,
                &valid_body(&arena, None),
            );
            let result = load(&path);
            assert!(
                matches!(result, Err(Error::Corrupt { .. })),
                "OOB child index should be Corrupt, got {result:?}"
            );
        }

        // Case 2: root is Leaf
        {
            let arena = vec![NodeKind::Leaf {
                content_hash: [0u8; 32],
                size: 0,
                mtime: 0,
            }];
            let path = dir.path().join("leaf_root.trie");
            write_raw(
                &path,
                &MAGIC,
                CURRENT_FORMAT_VERSION,
                &valid_body(&arena, None),
            );
            let result = load(&path);
            assert!(
                matches!(result, Err(Error::Corrupt { .. })),
                "root=Leaf should be Corrupt, got {result:?}"
            );
        }

        // Case 3: child points to Free node
        {
            let arena = vec![
                NodeKind::Dir {
                    children: vec![("a".into(), NodeIndex(1))],
                    merkle_hash: [0u8; 32],
                },
                NodeKind::Free { next: None },
            ];
            let path = dir.path().join("free_child.trie");
            write_raw(
                &path,
                &MAGIC,
                CURRENT_FORMAT_VERSION,
                &valid_body(&arena, None),
            );
            let result = load(&path);
            assert!(
                matches!(result, Err(Error::Corrupt { .. })),
                "child pointing to Free should be Corrupt, got {result:?}"
            );
        }

        // Case 4: DAG (two parents share a child)
        {
            let arena = vec![
                NodeKind::Dir {
                    children: vec![
                        ("a".into(), NodeIndex(1)),
                        ("b".into(), NodeIndex(2)),
                    ],
                    merkle_hash: [0u8; 32],
                },
                NodeKind::Dir {
                    children: vec![("x".into(), NodeIndex(3))],
                    merkle_hash: [0u8; 32],
                },
                NodeKind::Dir {
                    children: vec![("x".into(), NodeIndex(3))],
                    merkle_hash: [0u8; 32],
                },
                NodeKind::Leaf {
                    content_hash: [0u8; 32],
                    size: 0,
                    mtime: 0,
                },
            ];
            let path = dir.path().join("dag.trie");
            write_raw(
                &path,
                &MAGIC,
                CURRENT_FORMAT_VERSION,
                &valid_body(&arena, None),
            );
            let result = load(&path);
            assert!(
                matches!(result, Err(Error::Corrupt { .. })),
                "DAG (two parents) should be Corrupt, got {result:?}"
            );
        }

        // Case 5: free list contains a Dir node
        {
            let arena = vec![
                NodeKind::Dir {
                    children: vec![],
                    merkle_hash: empty_merkle_hash(),
                },
                NodeKind::Dir {
                    children: vec![],
                    merkle_hash: [0u8; 32],
                },
            ];
            let path = dir.path().join("free_dir.trie");
            write_raw(
                &path,
                &MAGIC,
                CURRENT_FORMAT_VERSION,
                &valid_body(&arena, Some(NodeIndex(1))),
            );
            let result = load(&path);
            assert!(
                matches!(result, Err(Error::Corrupt { .. })),
                "Dir on free list should be Corrupt, got {result:?}"
            );
        }
    }

    #[test]
    fn missing_file_returns_error() {
        let result = load(Path::new("/tmp/nonexistent-flatten-trie-test.trie"));
        assert!(
            matches!(result, Err(Error::NotFound { .. })),
            "missing file should be NotFound, got {result:?}"
        );
    }

    #[test]
    fn save_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.trie");

        let mut trie1 = Trie::new();
        trie1.insert("a", leaf([1u8; 32])).unwrap();
        save(&trie1, &path).unwrap();

        let mut trie2 = Trie::new();
        trie2.insert("b", leaf([2u8; 32])).unwrap();
        save(&trie2, &path).unwrap();

        let loaded = load(&path).unwrap();
        assert_eq!(
            loaded, trie2,
            "second save should overwrite; loaded trie should match trie2"
        );
    }

    #[test]
    fn save_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.trie");

        save(&Trie::new(), &path).unwrap();

        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "directory should contain only the target file"
        );
        assert_eq!(
            entries[0].file_name().to_str().unwrap(),
            "test.trie",
            "the only file should be test.trie"
        );
    }

    #[test]
    fn save_missing_parent_returns_io() {
        let result = save(
            &Trie::new(),
            Path::new("/nonexistent-flatten-dir/test.trie"),
        );
        assert!(
            matches!(result, Err(Error::Io { .. })),
            "missing parent dir should be Io, got {result:?}"
        );
    }
}
