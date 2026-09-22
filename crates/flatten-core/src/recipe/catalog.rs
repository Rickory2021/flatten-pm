// crates/flatten-core/src/recipe/catalog.rs
//
// The lookups resolve needs from stored entities, behind a trait.
//
// Resolve asks the catalog for transforms by name (and, from plan chunk C4,
// recipes for INVOKE). `DbCatalog` (C7) reads SQLite; `MemCatalog` is the
// in-memory test double. The trait is object-safe: resolve takes
// `&dyn Catalog`, so there is one instantiation and no generic plumbing.
//
// Implemented so far (plan chunk C2): transform lookup.

use std::fmt;

use super::error::Result;

/// A transform's scope. File transforms run in COPY chains; directory
/// transforms run through RUN.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformScope {
    /// Operates on one file at a time.
    File,
    /// Reshapes the whole run folder.
    Directory,
}

impl fmt::Display for TransformScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TransformScope::File => "file",
            TransformScope::Directory => "directory",
        })
    }
}

/// A transform at a resolved version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransformInfo {
    /// `transforms.id`.
    pub transform_id: i64,
    /// The transform name.
    pub name: String,
    /// File or directory.
    pub scope: TransformScope,
    /// `transform_versions.id` of the resolved version.
    pub version_id: i64,
    /// The resolved version number.
    pub version: u32,
}

/// Lookups resolve needs.
pub trait Catalog {
    /// A non-deleted transform by name. `version` None means the current
    /// version; `Some(n)` means version `n`. Returns `Ok(None)` when the
    /// transform, or that version of it, does not exist.
    fn transform(&self, name: &str, version: Option<u32>) -> Result<Option<TransformInfo>>;
}

/// In-memory catalog for tests.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub(crate) struct MemCatalog {
    transforms: Vec<MemTransform>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct MemTransform {
    transform_id: i64,
    name: String,
    scope: TransformScope,
    current: u32,
    /// `(version, version_id)` pairs.
    versions: Vec<(u32, i64)>,
}

#[cfg(test)]
impl MemCatalog {
    /// The five seeded builtins, each at version 1, with the seed's ids.
    pub(crate) fn builtins() -> Self {
        let rows = [
            (1, "flatten", TransformScope::Directory),
            (2, "pack", TransformScope::Directory),
            (3, "enrichment-injection", TransformScope::File),
            (4, "enrichment-trim", TransformScope::File),
            (5, "context-manifest", TransformScope::Directory),
        ];
        MemCatalog {
            transforms: rows
                .into_iter()
                .map(|(id, name, scope)| MemTransform {
                    transform_id: id,
                    name: name.to_string(),
                    scope,
                    current: 1,
                    versions: vec![(1, id)],
                })
                .collect(),
        }
    }

    /// Replace a transform's versions (adding the transform if it is new).
    pub(crate) fn with_versions(
        mut self,
        name: &str,
        scope: TransformScope,
        current: u32,
        versions: &[(u32, i64)],
    ) -> Self {
        let next_id = self.transforms.len() as i64 + 1;
        let entry = MemTransform {
            transform_id: next_id,
            name: name.to_string(),
            scope,
            current,
            versions: versions.to_vec(),
        };
        match self.transforms.iter_mut().find(|t| t.name == name) {
            Some(existing) => {
                existing.scope = scope;
                existing.current = current;
                existing.versions = versions.to_vec();
            }
            None => self.transforms.push(entry),
        }
        self
    }
}

#[cfg(test)]
impl Catalog for MemCatalog {
    fn transform(&self, name: &str, version: Option<u32>) -> Result<Option<TransformInfo>> {
        let Some(t) = self.transforms.iter().find(|t| t.name == name) else {
            return Ok(None);
        };
        let wanted = version.unwrap_or(t.current);
        Ok(t.versions
            .iter()
            .find(|(v, _)| *v == wanted)
            .map(|&(version, version_id)| TransformInfo {
                transform_id: t.transform_id,
                name: t.name.clone(),
                scope: t.scope,
                version_id,
                version,
            }))
    }
}
