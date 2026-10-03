// crates/flatten-core/src/recipe/catalog.rs
//
// The lookups resolve needs from stored entities, behind a trait.
//
// Resolve asks the catalog for transforms by name and for recipe versions
// (INVOKE). `DbCatalog` (C7) reads SQLite; `MemCatalog` is the in-memory test
// double. The trait is object-safe: resolve takes `&dyn Catalog`, so there is
// one instantiation and no generic plumbing.
//
// `PendingOverlay` wraps a catalog while unsaved text is analyzed under a
// recipe name (add, edit): an unpinned lookup of that name returns the
// pending text, so a self-INVOKE is caught as the cycle export would see.
// Only `analyze` builds it.
//
// Implemented so far (plan chunk C4): transform and recipe lookup.

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

/// A recipe's text at a resolved version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeSource {
    /// `build_recipes.id`; `None` only for pending text with no row yet.
    pub recipe_id: Option<i64>,
    /// The recipe name.
    pub name: String,
    /// `build_recipe_versions.id`; `None` for pending (unsaved) text.
    pub version_id: Option<i64>,
    /// The version number; `None` for pending text.
    pub version: Option<u32>,
    /// The recipe text, verbatim.
    pub source: String,
}

/// Lookups resolve needs.
pub trait Catalog {
    /// A non-deleted transform by name. `version` None means the current
    /// version; `Some(n)` means version `n`. Returns `Ok(None)` when the
    /// transform, or that version of it, does not exist.
    fn transform(&self, name: &str, version: Option<u32>) -> Result<Option<TransformInfo>>;

    /// A non-deleted recipe by name. `version` None means the current
    /// version; `Some(n)` means version `n`. Returns `Ok(None)` when the
    /// recipe, or that version of it, does not exist.
    fn recipe(&self, name: &str, version: Option<u32>) -> Result<Option<RecipeSource>>;
}

/// A catalog with one recipe name answered from pending (unsaved) text.
// SPEC-DEVIATION(EX-001): the spec gives `name: String`. It is borrowed: the
// overlay lives only inside `analyze`, next to the name it borrows.
pub(crate) struct PendingOverlay<'a> {
    inner: &'a dyn Catalog,
    name: &'a str,
    source: &'a str,
}

impl<'a> PendingOverlay<'a> {
    /// Answer unpinned lookups of `name` with `source`; delegate the rest.
    pub(crate) fn new(inner: &'a dyn Catalog, name: &'a str, source: &'a str) -> Self {
        PendingOverlay {
            inner,
            name,
            source,
        }
    }
}

impl Catalog for PendingOverlay<'_> {
    fn transform(&self, name: &str, version: Option<u32>) -> Result<Option<TransformInfo>> {
        self.inner.transform(name, version)
    }

    fn recipe(&self, name: &str, version: Option<u32>) -> Result<Option<RecipeSource>> {
        if name != self.name || version.is_some() {
            return self.inner.recipe(name, version);
        }
        let recipe_id = self.inner.recipe(name, None)?.and_then(|r| r.recipe_id);
        Ok(Some(RecipeSource {
            recipe_id,
            name: name.to_string(),
            version_id: None,
            version: None,
            source: self.source.to_string(),
        }))
    }
}

/// In-memory catalog for tests.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub(crate) struct MemCatalog {
    transforms: Vec<MemTransform>,
    recipes: Vec<MemRecipe>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct MemRecipe {
    recipe_id: i64,
    name: String,
    current: u32,
    /// `(version, version_id, source)` triples.
    versions: Vec<(u32, i64, String)>,
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
            recipes: Vec::new(),
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
    /// `current` must be one of the versions in `versions`; otherwise the
    /// unpinned lookup finds no current version.
    pub(crate) fn with_versions(
        mut self,
        name: &str,
        scope: TransformScope,
        current: u32,
        versions: &[(u32, i64)],
    ) -> Self {
        match self.transforms.iter_mut().find(|t| t.name == name) {
            Some(existing) => {
                existing.scope = scope;
                existing.current = current;
                existing.versions = versions.to_vec();
            }
            None => {
                let transform_id = self.transforms.len() as i64 + 1;
                self.transforms.push(MemTransform {
                    transform_id,
                    name: name.to_string(),
                    scope,
                    current,
                    versions: versions.to_vec(),
                });
            }
        }
        self
    }

    /// Add a recipe with its versions. `current` must be one of them.
    pub(crate) fn with_recipe(
        mut self,
        name: &str,
        recipe_id: i64,
        current: u32,
        versions: &[(u32, i64, &str)],
    ) -> Self {
        self.recipes.push(MemRecipe {
            recipe_id,
            name: name.to_string(),
            current,
            versions: versions
                .iter()
                .map(|&(v, id, src)| (v, id, src.to_string()))
                .collect(),
        });
        self
    }
}

#[cfg(test)]
impl Catalog for MemCatalog {
    fn recipe(&self, name: &str, version: Option<u32>) -> Result<Option<RecipeSource>> {
        let Some(r) = self.recipes.iter().find(|r| r.name == name) else {
            return Ok(None);
        };
        let wanted = version.unwrap_or(r.current);
        Ok(r.versions
            .iter()
            .find(|(v, _, _)| *v == wanted)
            .map(|(version, version_id, source)| RecipeSource {
                recipe_id: Some(r.recipe_id),
                name: r.name.clone(),
                version_id: Some(*version_id),
                version: Some(*version),
                source: source.clone(),
            }))
    }

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
