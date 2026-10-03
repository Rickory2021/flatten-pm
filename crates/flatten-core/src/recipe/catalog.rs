// crates/flatten-core/src/recipe/catalog.rs
//
// The lookups resolve needs from stored entities, behind a trait.
//
// Resolve asks the catalog for transforms by name and for recipe versions
// (INVOKE). `DbCatalog` reads SQLite through a reader or writer connection;
// `MemCatalog` is the in-memory test double. The trait is object-safe:
// resolve takes `&dyn Catalog`, so there is one instantiation and no generic
// plumbing.
//
// `DbCatalog` hides soft-deleted rows: a parent or version row with
// `deleted_at` set is absent. A `transforms.scope` value other than `file` or
// `directory` is a corrupt row and surfaces as `Error::Database`.
//
// `PendingOverlay` wraps a catalog while unsaved text is analyzed under a
// recipe name (add, edit): an unpinned lookup of that name returns the
// pending text, so a self-INVOKE is caught as the cycle export would see.
// Only `analyze` builds it.
//
use std::fmt;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ValueRef};
use rusqlite::{Connection, OptionalExtension, Row};

use super::error::Result;
use crate::db;

/// A transform's scope. File transforms run in COPY chains; directory
/// transforms run through RUN.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformScope {
    /// Operates on one file at a time.
    File,
    /// Reshapes the whole run folder.
    Directory,
}

/// `transforms.scope` text: `file` or `directory`; anything else is a
/// conversion failure (a corrupt row).
impl FromSql for TransformScope {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value.as_str()? {
            "file" => Ok(TransformScope::File),
            "directory" => Ok(TransformScope::Directory),
            other => Err(FromSqlError::Other(
                format!("unknown transform scope {other:?}").into(),
            )),
        }
    }
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
    /// The transform this one undoes, by name (`transforms.reverses`).
    pub reverses: Option<String>,
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

    /// Non-deleted transforms whose `reverses` is `name`, at their current
    /// versions, ordered by id.
    fn reversers_of(&self, name: &str) -> Result<Vec<TransformInfo>>;
}

/// The catalog over a SQLite connection (reader or writer).
pub struct DbCatalog<'c> {
    conn: &'c Connection,
}

impl<'c> DbCatalog<'c> {
    /// A catalog that reads through `conn`.
    pub fn new(conn: &'c Connection) -> Self {
        DbCatalog { conn }
    }
}

/// Columns: t.id, t.name, t.scope, t.reverses, v.id, v.version.
const TRANSFORM_COLUMNS: &str = "SELECT t.id, t.name, t.scope, t.reverses, v.id, v.version \
     FROM transforms t JOIN transform_versions v ON v.transform_id = t.id \
     WHERE t.deleted_at IS NULL AND v.deleted_at IS NULL";

fn transform_row(row: &Row<'_>) -> rusqlite::Result<TransformInfo> {
    Ok(TransformInfo {
        transform_id: row.get(0)?,
        name: row.get(1)?,
        scope: row.get(2)?,
        reverses: row.get(3)?,
        version_id: row.get(4)?,
        version: row.get(5)?,
    })
}

impl Catalog for DbCatalog<'_> {
    fn transform(&self, name: &str, version: Option<u32>) -> Result<Option<TransformInfo>> {
        let sql = format!(
            "{TRANSFORM_COLUMNS} AND t.name = ?1 \
             AND CASE WHEN ?2 IS NULL THEN v.id = t.current_version_id ELSE v.version = ?2 END"
        );
        Ok(self
            .conn
            .query_row(&sql, rusqlite::params![name, version], transform_row)
            .optional()
            .map_err(db::error::Error::from)?)
    }

    fn reversers_of(&self, name: &str) -> Result<Vec<TransformInfo>> {
        let sql = format!(
            "{TRANSFORM_COLUMNS} AND t.reverses = ?1 AND v.id = t.current_version_id \
             ORDER BY t.id"
        );
        let mut stmt = self.conn.prepare(&sql).map_err(db::error::Error::from)?;
        let rows = stmt
            .query_map([name], transform_row)
            .map_err(db::error::Error::from)?;
        Ok(rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db::error::Error::from)?)
    }

    fn recipe(&self, name: &str, version: Option<u32>) -> Result<Option<RecipeSource>> {
        Ok(self
            .conn
            .query_row(
                "SELECT r.id, r.name, v.id, v.version, v.source \
                 FROM build_recipes r JOIN build_recipe_versions v ON v.build_recipe_id = r.id \
                 WHERE r.deleted_at IS NULL AND v.deleted_at IS NULL AND r.name = ?1 \
                 AND CASE WHEN ?2 IS NULL THEN v.id = r.current_version_id ELSE v.version = ?2 END",
                rusqlite::params![name, version],
                |row| {
                    Ok(RecipeSource {
                        recipe_id: Some(row.get(0)?),
                        name: row.get(1)?,
                        version_id: Some(row.get(2)?),
                        version: Some(row.get(3)?),
                        source: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(db::error::Error::from)?)
    }
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

    fn reversers_of(&self, name: &str) -> Result<Vec<TransformInfo>> {
        self.inner.reversers_of(name)
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
    reverses: Option<String>,
    current: u32,
    /// `(version, version_id)` pairs.
    versions: Vec<(u32, i64)>,
}

#[cfg(test)]
impl MemCatalog {
    /// The five seeded builtins, each at version 1, with the seed's ids and
    /// `reverses` (enrichment-trim undoes enrichment-injection).
    pub(crate) fn builtins() -> Self {
        let rows = [
            (1, "flatten", TransformScope::Directory, None),
            (2, "pack", TransformScope::Directory, None),
            (3, "enrichment-injection", TransformScope::File, None),
            (
                4,
                "enrichment-trim",
                TransformScope::File,
                Some("enrichment-injection"),
            ),
            (5, "context-manifest", TransformScope::Directory, None),
        ];
        MemCatalog {
            recipes: Vec::new(),
            transforms: rows
                .into_iter()
                .map(|(id, name, scope, reverses)| MemTransform {
                    transform_id: id,
                    name: name.to_string(),
                    scope,
                    reverses: reverses.map(str::to_string),
                    current: 1,
                    versions: vec![(1, id)],
                })
                .collect(),
        }
    }

    /// Add a transform at version 1 with an optional `reverses`.
    pub(crate) fn with_transform(
        mut self,
        name: &str,
        scope: TransformScope,
        reverses: Option<&str>,
    ) -> Self {
        let transform_id = self.transforms.len() as i64 + 1;
        self.transforms.push(MemTransform {
            transform_id,
            name: name.to_string(),
            scope,
            reverses: reverses.map(str::to_string),
            current: 1,
            versions: vec![(1, 100 + transform_id)],
        });
        self
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
                    reverses: None,
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
        Ok(t.info(version.unwrap_or(t.current)))
    }

    fn reversers_of(&self, name: &str) -> Result<Vec<TransformInfo>> {
        Ok(self
            .transforms
            .iter()
            .filter(|t| t.reverses.as_deref() == Some(name))
            .filter_map(|t| t.info(t.current))
            .collect())
    }
}

#[cfg(test)]
impl MemTransform {
    fn info(&self, wanted: u32) -> Option<TransformInfo> {
        self.versions
            .iter()
            .find(|(v, _)| *v == wanted)
            .map(|&(version, version_id)| TransformInfo {
                transform_id: self.transform_id,
                name: self.name.clone(),
                scope: self.scope,
                reverses: self.reverses.clone(),
                version_id,
                version,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{Catalog, DbCatalog, TransformInfo, TransformScope};
    use crate::db::writer::Writer;
    use crate::db::{error as db_error, open_reader};
    use crate::recipe::{
        ArgInput, Error, ParseErrorKind, RootRef, SHIPPED_DEFAULT_RECIPE, analyze,
    };

    /// A seeded database with a writer for setup and a reader for lookups.
    fn seeded() -> (tempfile::NamedTempFile, Writer, rusqlite::Connection) {
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        let writer = Writer::open(tmp.path()).expect("Writer::open");
        let reader = open_reader(tmp.path()).expect("open_reader");
        (tmp, writer, reader)
    }

    /// Run setup SQL in one write transaction.
    fn exec(writer: &Writer, sql: &'static str) {
        writer
            .call_write(move |conn| {
                conn.execute_batch(sql).map_err(db_error::Error::from)?;
                Ok(())
            })
            .unwrap_or_else(|e| panic!("setup SQL failed: {e}\n{sql}"));
    }

    /// Test 80: builtin rows round-trip; pins and current pointers resolve;
    /// a corrupt scope is a database error.
    #[test]
    fn db_catalog_reads_scope_reverses_and_versions() {
        let (_tmp, writer, reader) = seeded();
        let catalog = DbCatalog::new(&reader);

        assert_eq!(
            catalog.transform("enrichment-trim", None).expect("query"),
            Some(TransformInfo {
                transform_id: 4,
                name: "enrichment-trim".into(),
                scope: TransformScope::File,
                reverses: Some("enrichment-injection".into()),
                version_id: 4,
                version: 1,
            }),
            "the seeded enrichment-trim row"
        );
        assert_eq!(
            catalog
                .transform("pack", None)
                .expect("query")
                .map(|t| t.scope),
            Some(TransformScope::Directory),
            "pack is a directory transform"
        );
        assert!(
            catalog.transform("nope", None).expect("query").is_none(),
            "an unknown name is None"
        );
        assert!(
            catalog.transform("pack", Some(2)).expect("query").is_none(),
            "a missing pinned version is None"
        );
        assert_eq!(
            catalog
                .reversers_of("enrichment-injection")
                .expect("query")
                .into_iter()
                .map(|t| t.name)
                .collect::<Vec<_>>(),
            vec!["enrichment-trim".to_string()],
            "enrichment-trim reverses enrichment-injection"
        );
        assert!(
            catalog.reversers_of("flatten").expect("query").is_empty(),
            "nothing reverses flatten"
        );

        let shipped = catalog
            .recipe("shipped-default", None)
            .expect("query")
            .expect("the shipped recipe exists");
        assert_eq!(
            (shipped.recipe_id, shipped.version_id, shipped.version),
            (Some(1), Some(1), Some(1)),
            "shipped-default ids and version"
        );
        assert_eq!(shipped.source, SHIPPED_DEFAULT_RECIPE, "the seeded text");
        assert!(
            catalog
                .recipe("shipped-default", Some(2))
                .expect("query")
                .is_none(),
            "a missing pinned recipe version is None"
        );

        exec(
            &writer,
            "INSERT INTO transform_versions (id, transform_id, version, source) VALUES (20, 2, 2, 'v2');
             UPDATE transforms SET current_version_id = 20 WHERE id = 2;",
        );
        assert_eq!(
            catalog
                .transform("pack", None)
                .expect("query")
                .map(|t| (t.version, t.version_id)),
            Some((2, 20)),
            "unpinned follows the current pointer"
        );
        assert_eq!(
            catalog
                .transform("pack", Some(1))
                .expect("query")
                .map(|t| (t.version, t.version_id)),
            Some((1, 2)),
            "a pin reads the older version"
        );

        exec(
            &writer,
            "INSERT INTO transforms (id, name, scope) VALUES (30, 'bad', 'bogus');
             INSERT INTO transform_versions (id, transform_id, version, source) VALUES (30, 30, 1, 'x');
             UPDATE transforms SET current_version_id = 30 WHERE id = 30;",
        );
        assert!(
            matches!(catalog.transform("bad", None), Err(Error::Database(_))),
            "an unknown scope value is a corrupt row"
        );
    }

    /// Test 81: soft-deleted parents and versions are absent, so INVOKE and
    /// pinned lookups report them as missing.
    #[test]
    fn db_catalog_excludes_soft_deleted() {
        let (_tmp, writer, reader) = seeded();
        exec(
            &writer,
            "INSERT INTO transforms (id, name, scope, reverses) VALUES (40, 'gone-t', 'file', 'x');
             INSERT INTO transform_versions (id, transform_id, version, source) VALUES (40, 40, 1, 's');
             UPDATE transforms SET current_version_id = 40, deleted_at = '2026-01-01T00:00:00Z' WHERE id = 40;
             INSERT INTO transforms (id, name, scope) VALUES (41, 'half-t', 'file');
             INSERT INTO transform_versions (id, transform_id, version, source) VALUES (41, 41, 1, 's');
             INSERT INTO transform_versions (id, transform_id, version, source, deleted_at)
                 VALUES (42, 41, 2, 's', '2026-01-01T00:00:00Z');
             UPDATE transforms SET current_version_id = 41 WHERE id = 41;
             INSERT INTO build_recipes (id, name) VALUES (10, 'gone');
             INSERT INTO build_recipe_versions (id, build_recipe_id, version, source)
                 VALUES (10, 10, 1, 'SOURCE r:\n  COPY . x/ AS k');
             UPDATE build_recipes SET current_version_id = 10, deleted_at = '2026-01-01T00:00:00Z' WHERE id = 10;
             INSERT INTO build_recipes (id, name) VALUES (11, 'half');
             INSERT INTO build_recipe_versions (id, build_recipe_id, version, source) VALUES (11, 11, 1, '');
             INSERT INTO build_recipe_versions (id, build_recipe_id, version, source, deleted_at)
                 VALUES (12, 11, 2, '', '2026-01-01T00:00:00Z');
             UPDATE build_recipes SET current_version_id = 11 WHERE id = 11;",
        );
        let catalog = DbCatalog::new(&reader);

        assert!(
            catalog.transform("gone-t", None).expect("query").is_none(),
            "a soft-deleted transform is absent"
        );
        assert!(
            catalog.reversers_of("x").expect("query").is_empty(),
            "a soft-deleted transform reverses nothing"
        );
        assert!(
            catalog
                .transform("half-t", Some(2))
                .expect("query")
                .is_none(),
            "a soft-deleted version is absent"
        );
        assert!(
            catalog
                .transform("half-t", Some(1))
                .expect("query")
                .is_some(),
            "the live version still resolves"
        );
        assert!(
            catalog.recipe("gone", None).expect("query").is_none(),
            "a soft-deleted recipe is absent"
        );

        let input = RootRef::Pending { name: None };
        let kind_of = |source: &str| match analyze(source, &ArgInput::Open, &catalog, &input) {
            Err(Error::Parse { kind, .. }) => kind,
            other => panic!("{source:?}: expected a parse error, got {other:?}"),
        };
        assert_eq!(
            kind_of("INVOKE gone"),
            ParseErrorKind::UnknownRecipe {
                name: "gone".into()
            },
            "INVOKE of a soft-deleted recipe is UnknownRecipe"
        );
        assert_eq!(
            kind_of("INVOKE half@2"),
            ParseErrorKind::RecipeVersionNotFound {
                name: "half".into(),
                version: 2
            },
            "INVOKE of a soft-deleted version is RecipeVersionNotFound"
        );
    }
}
