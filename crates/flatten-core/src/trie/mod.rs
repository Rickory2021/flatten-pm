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
