// crates/flatten-core/src/recipe/store.rs
//
// Recipe storage: add, new, edit, rollback, soft delete, and the read side
// (list, get, history, args, resolve_stored, lint_text).
//
// Every write runs in one IMMEDIATE transaction on the writer thread and
// validates inside it, so the checks and the write see the same rows. The
// transaction commits only when the operation succeeds; any error, domain or
// database, rolls it back, so a failure part way through leaves nothing. Text is stored
// verbatim (comments, CRLF, BOM, and a missing final newline survive).
// Analysis uses a `DbCatalog` over the same connection, with
// `RootRef::Pending { name }` for unsaved text so a self-INVOKE is the cycle
// it will be once saved.
//
// Versioning (docs/design/6_VERSIONING.md, ADR-037): a save inserts version
// MAX(version) + 1 over every version row, soft-deleted ones included, and
// moves `current_version_id`. Each pointer move bumps the change counter in
// the same transaction (DA-004 [D10]). An edit whose text is byte-identical
// to the current version writes nothing and does not analyze. Rollback moves
// the pointer first, then checks the target: an error becomes `problem`
// (export and show will hit it) and never fails the call; a clean check
// returns its warnings, like add and edit.
//
// Names follow the transform rule, [A-Za-z0-9][A-Za-z0-9_.-]*, and stay
// taken after a soft delete. The active-binding guard on delete is
// [DEFERRED: DA-005].

use rusqlite::{Connection, OptionalExtension};

use super::catalog::DbCatalog;
use super::error::{Error, Result};
use super::lint::LintWarning;
use super::parse::resolve::{ArgInput, Resolution, RootRef};
use super::parse::{declared_args, is_name};
use super::types::Arg;
use super::{SHIPPED_DEFAULT_RECIPE, analyze};
use crate::db;
use crate::db::writer::Writer;

/// One live recipe, as `recipe list` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeRow {
    /// `build_recipes.id`.
    pub id: i64,
    /// The recipe name.
    pub name: String,
    /// `builtin` or `custom`.
    pub curation: String,
    /// The version the current pointer names.
    pub current_version: u32,
    /// Live (not soft-deleted) versions.
    pub version_count: u32,
}

/// A live recipe with its current version's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeDetail {
    /// The recipe row.
    pub row: RecipeRow,
    /// `build_recipe_versions.id` of the current version.
    pub version_id: i64,
    /// The current version's text, verbatim.
    pub source: String,
}

/// One live version, as `recipe history` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRow {
    /// The version number.
    pub version: u32,
    /// `build_recipe_versions.id`.
    pub version_id: i64,
    /// When the version was saved (ISO 8601, UTC).
    pub created_at: String,
    /// Whether the current pointer names this version.
    pub current: bool,
}

/// The result of add, new, or edit.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveReport {
    /// `build_recipes.id`.
    pub recipe_id: i64,
    /// The current version's id after the call.
    pub version_id: i64,
    /// The current version's number after the call.
    pub version: u32,
    /// True when an edit's text matched the current version, so nothing
    /// was written.
    pub unchanged: bool,
    /// Warnings from analyzing the saved text; empty when `unchanged`.
    pub warnings: Vec<LintWarning>,
}

/// The result of rollback.
#[derive(Debug)]
pub struct RollbackReport {
    /// The version now current.
    pub version: u32,
    /// False when the target was already current (nothing written).
    pub moved: bool,
    /// The error analysis of the now-current version reports, if any. A
    /// rollback never fails because of it.
    pub problem: Option<Error>,
    /// Warnings from that analysis; empty when `problem` is set.
    pub warnings: Vec<LintWarning>,
}

/// A live recipe's head row.
struct Head {
    id: i64,
    curation: String,
    current_version_id: i64,
}

/// The live recipe named `name`, or RecipeNotFound.
fn live_head(conn: &Connection, name: &str) -> Result<Head> {
    conn.query_row(
        "SELECT id, curation, current_version_id FROM build_recipes \
         WHERE name = ?1 AND deleted_at IS NULL",
        [name],
        |row| {
            Ok(Head {
                id: row.get(0)?,
                curation: row.get(1)?,
                current_version_id: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(db::error::Error::from)?
    .ok_or_else(|| Error::RecipeNotFound(name.to_string()))
}

/// `(version, source)` of a version row.
fn version_row(conn: &Connection, version_id: i64) -> Result<(u32, String)> {
    Ok(conn
        .query_row(
            "SELECT version, source FROM build_recipe_versions WHERE id = ?1",
            [version_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(db::error::Error::from)?)
}

/// Analyze unsaved text under `name` against the stored catalog.
fn analyze_pending(conn: &Connection, name: &str, source: &str) -> Result<Resolution> {
    analyze(
        source,
        &ArgInput::Open,
        &DbCatalog::new(conn),
        &RootRef::Pending {
            name: Some(name.to_string()),
        },
    )
}

/// Insert version `MAX(version) + 1`, move the pointer, and bump the counter.
fn insert_version(conn: &Connection, recipe_id: i64, source: &str) -> Result<(i64, u32)> {
    let next: u32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM build_recipe_versions \
             WHERE build_recipe_id = ?1",
            [recipe_id],
            |row| row.get(0),
        )
        .map_err(db::error::Error::from)?;
    conn.execute(
        "INSERT INTO build_recipe_versions (build_recipe_id, version, source) \
         VALUES (?1, ?2, ?3)",
        rusqlite::params![recipe_id, next, source],
    )
    .map_err(db::error::Error::from)?;
    let version_id = conn.last_insert_rowid();
    conn.execute(
        "UPDATE build_recipes SET current_version_id = ?1 WHERE id = ?2",
        [version_id, recipe_id],
    )
    .map_err(db::error::Error::from)?;
    db::bump_change_counter(conn)?;
    Ok((version_id, next))
}

/// Run `op` in one IMMEDIATE transaction on the writer thread. Commit only
/// when `op` succeeds; on any error the transaction is dropped, which rolls
/// it back. (`Writer::call_write` would commit an `Ok(Err(..))`.)
fn in_write_tx<T, F>(writer: &Writer, op: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> Result<T> + Send + 'static,
{
    writer.call(move |conn| {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let result = op(&tx);
        if result.is_ok() {
            tx.commit()?;
        }
        Ok(result)
    })?
}

/// True for a SQLite constraint failure with this extended code.
fn is_constraint(e: &db::error::Error, code: i32) -> bool {
    matches!(
        e,
        db::error::Error::RuSQLite(rusqlite::Error::SqliteFailure(err, _))
            if err.extended_code == code
    )
}

/// Save a new recipe at version 1. Fails with InvalidName, DuplicateName
/// (soft-deleted names included), or any analysis error, writing nothing.
pub fn add_recipe(writer: &Writer, name: &str, source: &str) -> Result<SaveReport> {
    let name = name.to_string();
    let source = source.to_string();
    in_write_tx(writer, move |conn| add_in(conn, &name, &source))
}

fn add_in(conn: &Connection, name: &str, source: &str) -> Result<SaveReport> {
    if !is_name(name) {
        return Err(Error::InvalidName {
            name: name.to_string(),
        });
    }
    let taken: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM build_recipes WHERE name = ?1)",
            [name],
            |row| row.get(0),
        )
        .map_err(db::error::Error::from)?;
    if taken {
        return Err(Error::DuplicateName {
            name: name.to_string(),
        });
    }
    let resolution = analyze_pending(conn, name, source)?;

    conn.execute(
        "INSERT INTO build_recipes (name, curation) VALUES (?1, 'custom')",
        [name],
    )
    .map_err(|e| {
        let e = db::error::Error::from(e);
        if is_constraint(&e, rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE) {
            Error::DuplicateName {
                name: name.to_string(),
            }
        } else {
            Error::Database(e)
        }
    })?;
    let recipe_id = conn.last_insert_rowid();
    let (version_id, version) = insert_version(conn, recipe_id, source)?;
    Ok(SaveReport {
        recipe_id,
        version_id,
        version,
        unchanged: false,
        warnings: resolution.warnings,
    })
}

/// Save a new recipe whose text is the shipped default.
pub fn new_recipe(writer: &Writer, name: &str) -> Result<SaveReport> {
    add_recipe(writer, name, SHIPPED_DEFAULT_RECIPE)
}

/// Save a new version of a live recipe. Identical text is a no-op
/// (`unchanged`, no analysis, no write, no counter bump). Builtin recipes
/// are editable.
pub fn edit_recipe(writer: &Writer, name: &str, source: &str) -> Result<SaveReport> {
    let name = name.to_string();
    let source = source.to_string();
    in_write_tx(writer, move |conn| edit_in(conn, &name, &source))
}

fn edit_in(conn: &Connection, name: &str, source: &str) -> Result<SaveReport> {
    let head = live_head(conn, name)?;
    let (current, current_source) = version_row(conn, head.current_version_id)?;
    if current_source == source {
        return Ok(SaveReport {
            recipe_id: head.id,
            version_id: head.current_version_id,
            version: current,
            unchanged: true,
            warnings: Vec::new(),
        });
    }
    let resolution = analyze_pending(conn, name, source)?;
    let (version_id, version) = insert_version(conn, head.id, source)?;
    Ok(SaveReport {
        recipe_id: head.id,
        version_id,
        version,
        unchanged: false,
        warnings: resolution.warnings,
    })
}

/// Move the current pointer to an existing live version. Then analyze the
/// now-current version: an error is reported as `problem` (the call still
/// succeeds); otherwise its warnings are returned.
pub fn rollback_recipe(writer: &Writer, name: &str, version: u32) -> Result<RollbackReport> {
    let name = name.to_string();
    in_write_tx(writer, move |conn| rollback_in(conn, &name, version))
}

fn rollback_in(conn: &Connection, name: &str, version: u32) -> Result<RollbackReport> {
    let head = live_head(conn, name)?;
    let target: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, source FROM build_recipe_versions \
             WHERE build_recipe_id = ?1 AND version = ?2 AND deleted_at IS NULL",
            rusqlite::params![head.id, version],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(db::error::Error::from)?;
    let Some((version_id, source)) = target else {
        return Err(Error::VersionNotFound {
            name: name.to_string(),
            version,
        });
    };

    let moved = version_id != head.current_version_id;
    if moved {
        conn.execute(
            "UPDATE build_recipes SET current_version_id = ?1 WHERE id = ?2",
            [version_id, head.id],
        )
        .map_err(db::error::Error::from)?;
        db::bump_change_counter(conn)?;
    }

    let root = RootRef::Stored {
        recipe_id: head.id,
        version_id,
        name: name.to_string(),
        version,
    };
    let (problem, warnings) = match analyze(&source, &ArgInput::Open, &DbCatalog::new(conn), &root)
    {
        Ok(resolution) => (None, resolution.warnings),
        Err(e) => (Some(e), Vec::new()),
    };
    Ok(RollbackReport {
        version,
        moved,
        problem,
        warnings,
    })
}

/// Soft-delete a live custom recipe. Its name stays taken.
pub fn soft_delete_recipe(writer: &Writer, name: &str) -> Result<()> {
    let name = name.to_string();
    in_write_tx(writer, move |conn| soft_delete_in(conn, &name))
}

fn soft_delete_in(conn: &Connection, name: &str) -> Result<()> {
    let head = live_head(conn, name)?;
    let protected = || Error::BuiltinProtected {
        name: name.to_string(),
    };
    if head.curation == "builtin" {
        return Err(protected());
    }
    conn.execute(
        "UPDATE build_recipes SET deleted_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE id = ?1",
        [head.id],
    )
    .map_err(|e| {
        let e = db::error::Error::from(e);
        if is_constraint(&e, rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER) {
            protected()
        } else {
            Error::Database(e)
        }
    })?;
    Ok(())
}

/// Live recipes, by name.
pub fn list_recipes(conn: &Connection) -> Result<Vec<RecipeRow>> {
    let mut stmt = conn
        .prepare(
            "SELECT r.id, r.name, r.curation, v.version, \
                    (SELECT COUNT(*) FROM build_recipe_versions c \
                     WHERE c.build_recipe_id = r.id AND c.deleted_at IS NULL) \
             FROM build_recipes r JOIN build_recipe_versions v ON v.id = r.current_version_id \
             WHERE r.deleted_at IS NULL ORDER BY r.name",
        )
        .map_err(db::error::Error::from)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(RecipeRow {
                id: row.get(0)?,
                name: row.get(1)?,
                curation: row.get(2)?,
                current_version: row.get(3)?,
                version_count: row.get(4)?,
            })
        })
        .map_err(db::error::Error::from)?;
    Ok(rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db::error::Error::from)?)
}

/// A live recipe with its current text.
pub fn get_recipe(conn: &Connection, name: &str) -> Result<RecipeDetail> {
    let head = live_head(conn, name)?;
    let (current_version, source) = version_row(conn, head.current_version_id)?;
    let version_count: u32 = conn
        .query_row(
            "SELECT COUNT(*) FROM build_recipe_versions \
             WHERE build_recipe_id = ?1 AND deleted_at IS NULL",
            [head.id],
            |row| row.get(0),
        )
        .map_err(db::error::Error::from)?;
    Ok(RecipeDetail {
        row: RecipeRow {
            id: head.id,
            name: name.to_string(),
            curation: head.curation,
            current_version,
            version_count,
        },
        version_id: head.current_version_id,
        source,
    })
}

/// A live recipe's live versions, oldest first.
pub fn recipe_history(conn: &Connection, name: &str) -> Result<Vec<VersionRow>> {
    let head = live_head(conn, name)?;
    let mut stmt = conn
        .prepare(
            "SELECT version, id, created_at FROM build_recipe_versions \
             WHERE build_recipe_id = ?1 AND deleted_at IS NULL ORDER BY version",
        )
        .map_err(db::error::Error::from)?;
    let rows = stmt
        .query_map([head.id], |row| {
            let version_id: i64 = row.get(1)?;
            Ok(VersionRow {
                version: row.get(0)?,
                version_id,
                created_at: row.get(2)?,
                current: version_id == head.current_version_id,
            })
        })
        .map_err(db::error::Error::from)?;
    Ok(rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db::error::Error::from)?)
}

/// The current version's root ARG declarations, from parsing alone.
pub fn recipe_args(conn: &Connection, name: &str) -> Result<Vec<Arg>> {
    declared_args(&get_recipe(conn, name)?.source)
}

/// Analyze a live recipe's current version against the stored catalog.
pub fn resolve_stored(conn: &Connection, name: &str, input: &ArgInput) -> Result<Resolution> {
    let detail = get_recipe(conn, name)?;
    let root = RootRef::Stored {
        recipe_id: detail.row.id,
        version_id: detail.version_id,
        name: detail.row.name,
        version: detail.row.current_version,
    };
    analyze(&detail.source, input, &DbCatalog::new(conn), &root)
}

/// Analyze unsaved text (a file on disk) in open mode against the stored
/// catalog. Nothing is written.
pub fn lint_text(conn: &Connection, source: &str) -> Result<Resolution> {
    analyze(
        source,
        &ArgInput::Open,
        &DbCatalog::new(conn),
        &RootRef::Pending { name: None },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_reader;
    use crate::recipe::{LintCode, ParseErrorKind, Position};

    /// A seeded database: the temp file, a writer, and a reader.
    fn setup() -> (tempfile::NamedTempFile, Writer, Connection) {
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        let writer = Writer::open(tmp.path()).expect("Writer::open");
        let reader = open_reader(tmp.path()).expect("open_reader");
        (tmp, writer, reader)
    }

    fn counter(conn: &Connection) -> i64 {
        conn.query_row("SELECT counter FROM change_counter WHERE id = 1", [], |r| {
            r.get(0)
        })
        .expect("counter row")
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).expect("count query")
    }

    fn add(writer: &Writer, name: &str, source: &str) -> SaveReport {
        add_recipe(writer, name, source).unwrap_or_else(|e| panic!("add {name}: {e}"))
    }

    fn kind(e: Error) -> ParseErrorKind {
        match e {
            Error::Parse { kind, .. } => kind,
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    fn codes(warnings: &[LintWarning]) -> Vec<(LintCode, u32, Option<&str>)> {
        warnings
            .iter()
            .map(|w| (w.code, w.position.line, w.recipe.as_deref()))
            .collect()
    }

    /// Test 82: stored and unsaved text resolve against the DB catalog, with
    /// resolver and lint warnings merged and sorted (root first).
    #[test]
    fn resolve_stored_and_lint_text_merge_sorted_warnings() {
        let (_tmp, writer, reader) = setup();
        add(
            &writer,
            "base",
            "SOURCE r:\n  COPY . b/ AS b\nWATCH:\n  DEPTH_TOLERANCE 4",
        );
        let top = "INVOKE base\nSOURCE r:\n  COPY . t/ AS t";
        let saved = add(&writer, "top", top);
        let expected = vec![
            (LintCode::L005, 3, None),
            (LintCode::L005, 2, Some("base@1")),
            (LintCode::L006, 4, Some("base@1")),
        ];
        assert_eq!(codes(&saved.warnings), expected, "add reports the warnings");

        let stored = resolve_stored(&reader, "top", &ArgInput::Open).expect("resolve_stored");
        assert_eq!(
            codes(&stored.warnings),
            expected,
            "resolve_stored sorts the same way"
        );
        assert_eq!(
            stored.recipe.invoked_versions[0].name, "base",
            "the stored root expands its INVOKE"
        );
        let linted = lint_text(&reader, top).expect("lint_text");
        assert_eq!(codes(&linted.warnings), expected, "lint_text matches");
    }

    /// Test 83: add inserts the parent, version 1, the pointer, and one bump.
    #[test]
    fn add_inserts_parent_version_1_pointer_and_bumps_counter() {
        let (_tmp, writer, reader) = setup();
        let before = counter(&reader);
        let report = add(&writer, "mine", "ARG repo");
        assert_eq!(
            (report.version, report.unchanged),
            (1, false),
            "a new recipe starts at version 1"
        );
        let detail = get_recipe(&reader, "mine").expect("get");
        assert_eq!(
            (
                detail.row.id,
                detail.version_id,
                detail.row.curation.as_str()
            ),
            (report.recipe_id, report.version_id, "custom"),
            "the pointer names the new version; curation is custom"
        );
        assert_eq!(counter(&reader), before + 1, "one counter bump");
    }

    /// Test 84: name errors: invalid names, and names already taken (soft
    /// deleted ones included).
    #[test]
    fn add_name_errors() {
        let (_tmp, writer, _reader) = setup();
        for name in ["", "has space", "a@1", "-lead", "dot/slash"] {
            assert!(
                matches!(add_recipe(&writer, name, ""), Err(Error::InvalidName { name: n }) if n == name),
                "{name:?} is an invalid name"
            );
        }
        add(&writer, "dup", "");
        assert!(
            matches!(
                add_recipe(&writer, "dup", ""),
                Err(Error::DuplicateName { .. })
            ),
            "a live name is taken"
        );
        add(&writer, "old", "");
        soft_delete_recipe(&writer, "old").expect("delete");
        assert!(
            matches!(
                add_recipe(&writer, "old", ""),
                Err(Error::DuplicateName { .. })
            ),
            "a soft-deleted name stays taken"
        );
        assert!(
            matches!(
                add_recipe(&writer, "shipped-default", ""),
                Err(Error::DuplicateName { .. })
            ),
            "the builtin name is taken"
        );
    }

    /// Test 85: an add that fails analysis writes nothing.
    #[test]
    fn add_parse_error_writes_nothing() {
        let (_tmp, writer, reader) = setup();
        let rows = count(&reader, "SELECT COUNT(*) FROM build_recipes");
        let versions = count(&reader, "SELECT COUNT(*) FROM build_recipe_versions");
        let before = counter(&reader);
        assert_eq!(
            kind(add_recipe(&writer, "bad", "COPIE . x/ AS k").expect_err("parse error")),
            ParseErrorKind::UnknownInstruction("COPIE".into()),
            "the analysis error is returned"
        );
        assert_eq!(
            (
                count(&reader, "SELECT COUNT(*) FROM build_recipes"),
                count(&reader, "SELECT COUNT(*) FROM build_recipe_versions"),
                counter(&reader)
            ),
            (rows, versions, before),
            "no rows and no bump"
        );
    }

    /// A write that fails after the parent row is inserted leaves nothing:
    /// the store commits only when the whole operation succeeds. (With no
    /// change_counter row, `add` fails at the bump, after its inserts.)
    #[test]
    fn add_failure_after_parent_insert_writes_nothing() {
        let (_tmp, writer, reader) = setup();
        writer
            .call_write(|conn| {
                conn.execute("DELETE FROM change_counter", [])?;
                Ok(())
            })
            .expect("remove the counter row");
        assert!(
            matches!(
                add_recipe(&writer, "partial", "ARG a"),
                Err(Error::Database(_))
            ),
            "the failed bump surfaces as a database error"
        );
        assert_eq!(
            (
                count(
                    &reader,
                    "SELECT COUNT(*) FROM build_recipes WHERE name = 'partial'"
                ),
                count(
                    &reader,
                    "SELECT COUNT(*) FROM build_recipe_versions v JOIN build_recipes r \
                     ON r.id = v.build_recipe_id WHERE r.name = 'partial'"
                )
            ),
            (0, 0),
            "neither the parent nor the version row was committed"
        );

        writer
            .call_write(|conn| {
                conn.execute("INSERT INTO change_counter (id, counter) VALUES (1, 0)", [])?;
                Ok(())
            })
            .expect("restore the counter row");
        assert_eq!(
            add(&writer, "partial", "ARG a").version,
            1,
            "the writer is healthy and the name is still free"
        );
    }

    /// Test 86: text is stored verbatim: comments, CRLF, BOM, no final newline.
    #[test]
    fn add_stores_source_verbatim() {
        let (_tmp, writer, reader) = setup();
        let source = "\u{FEFF}# keep me\r\nARG a\r\nSOURCE r:\r\n  COPY . x/ AS k";
        add(&writer, "verbatim", source);
        assert_eq!(
            get_recipe(&reader, "verbatim").expect("get").source,
            source,
            "byte-for-byte"
        );
    }

    /// Test 87: new uses the shipped text, whose one ARG is required `repo`.
    #[test]
    fn new_uses_shipped_text_and_repo_required() {
        let (_tmp, writer, reader) = setup();
        let report = new_recipe(&writer, "fresh").expect("new");
        assert_eq!(report.version, 1, "version 1");
        assert!(report.warnings.is_empty(), "the shipped text lints clean");
        assert_eq!(
            get_recipe(&reader, "fresh").expect("get").source,
            SHIPPED_DEFAULT_RECIPE,
            "the shipped text"
        );
        assert_eq!(
            recipe_args(&reader, "fresh").expect("args"),
            vec![Arg {
                name: "repo".into(),
                default: None,
                required: true,
                position: Position::new(1, 1),
            }],
            "repo is required"
        );
    }

    /// Test 88: edit inserts the next version, moves the pointer, and bumps.
    #[test]
    fn edit_inserts_next_version_moves_pointer_and_bumps() {
        let (_tmp, writer, reader) = setup();
        add(&writer, "e", "ARG a");
        let before = counter(&reader);
        let report = edit_recipe(&writer, "e", "ARG b").expect("edit");
        assert_eq!(
            (report.version, report.unchanged),
            (2, false),
            "the next version"
        );
        let detail = get_recipe(&reader, "e").expect("get");
        assert_eq!(
            (
                detail.source.as_str(),
                detail.version_id,
                detail.row.current_version
            ),
            ("ARG b", report.version_id, 2),
            "the pointer moved"
        );
        assert_eq!(counter(&reader), before + 1, "one bump");
        assert_eq!(
            edit_recipe(&writer, "shipped-default", "ARG repo")
                .expect("edit builtin")
                .version,
            2,
            "builtin recipes are editable"
        );
    }

    /// Test 89: identical text is a no-op, and skips analysis entirely.
    #[test]
    fn edit_identical_source_is_noop() {
        let (_tmp, writer, reader) = setup();
        add(&writer, "base", "");
        add(&writer, "top", "INVOKE base");
        soft_delete_recipe(&writer, "base").expect("delete base");
        let before = counter(&reader);
        let report = edit_recipe(&writer, "top", "INVOKE base").expect("identical edit");
        assert_eq!(
            (report.unchanged, report.version, report.warnings.len()),
            (true, 1, 0),
            "unchanged, still version 1, no warnings"
        );
        assert_eq!(counter(&reader), before, "no bump");
        assert_eq!(
            kind(edit_recipe(&writer, "top", "INVOKE base\n").expect_err("changed text")),
            ParseErrorKind::UnknownRecipe {
                name: "base".into()
            },
            "changed text is analyzed (and base is gone)"
        );
    }

    /// Test 90: an edit that fails analysis keeps the pointer.
    #[test]
    fn edit_parse_error_keeps_pointer() {
        let (_tmp, writer, reader) = setup();
        add(&writer, "e", "ARG a");
        let before = counter(&reader);
        assert!(
            matches!(edit_recipe(&writer, "e", "COPIE"), Err(Error::Parse { .. })),
            "the analysis error is returned"
        );
        let detail = get_recipe(&reader, "e").expect("get");
        assert_eq!(
            (
                detail.row.current_version,
                detail.row.version_count,
                counter(&reader)
            ),
            (1, 1, before),
            "no new version, no bump"
        );
    }

    /// Test 91: rollback moves the pointer and bumps; rolling back to the
    /// current version writes nothing but still checks it; a clean check
    /// returns its warnings.
    #[test]
    fn rollback_moves_pointer_bumps_and_current_is_noop() {
        let (_tmp, writer, reader) = setup();
        add(&writer, "r", "ARG a");
        edit_recipe(&writer, "r", "ARG b").expect("edit");
        let before = counter(&reader);
        let report = rollback_recipe(&writer, "r", 1).expect("rollback");
        assert_eq!(
            (report.version, report.moved, report.problem.is_none()),
            (1, true, true),
            "moved to version 1 with no problem"
        );
        assert_eq!(
            get_recipe(&reader, "r").expect("get").source,
            "ARG a",
            "v1 is current"
        );
        assert_eq!(counter(&reader), before + 1, "one bump");

        let again = rollback_recipe(&writer, "r", 1).expect("rollback to current");
        assert_eq!(
            (again.moved, again.problem.is_none()),
            (false, true),
            "already current: nothing moved"
        );
        assert_eq!(counter(&reader), before + 1, "no bump for a no-op");

        add(&writer, "warned", "SOURCE r:\n  COPY . x/ AS k");
        edit_recipe(&writer, "warned", "").expect("edit");
        let warned = rollback_recipe(&writer, "warned", 1).expect("rollback");
        assert_eq!(
            codes(&warned.warnings),
            vec![(LintCode::L005, 2, None)],
            "a clean check returns the target's warnings"
        );

        add(&writer, "base", "");
        add(&writer, "top", "INVOKE base");
        soft_delete_recipe(&writer, "base").expect("delete base");
        let stale = rollback_recipe(&writer, "top", 1).expect("rollback to current");
        assert!(!stale.moved, "already current");
        assert_eq!(
            kind(stale.problem.expect("the check ran")),
            ParseErrorKind::UnknownRecipe {
                name: "base".into()
            },
            "the check runs even when nothing moved"
        );
        assert!(stale.warnings.is_empty(), "no warnings with a problem");
    }

    /// Test 92: rollback to a missing or soft-deleted version errors; the next
    /// version number counts soft-deleted rows.
    #[test]
    fn rollback_unknown_version_errors() {
        let (_tmp, writer, _reader) = setup();
        add(&writer, "r", "ARG a");
        edit_recipe(&writer, "r", "ARG b").expect("edit");
        edit_recipe(&writer, "r", "ARG c").expect("edit");
        writer
            .call_write(|conn| {
                conn.execute(
                    "UPDATE build_recipe_versions SET deleted_at = 'x' WHERE version = 2 \
                     AND build_recipe_id = (SELECT id FROM build_recipes WHERE name = 'r')",
                    [],
                )?;
                Ok(())
            })
            .expect("soft-delete v2");
        for version in [9, 2] {
            assert!(
                matches!(
                    rollback_recipe(&writer, "r", version),
                    Err(Error::VersionNotFound { version: v, .. }) if v == version
                ),
                "version {version} is not a rollback target"
            );
        }

        rollback_recipe(&writer, "r", 1).expect("rollback to 1");
        writer
            .call_write(|conn| {
                conn.execute(
                    "UPDATE build_recipe_versions SET deleted_at = 'x' WHERE version = 3 \
                     AND build_recipe_id = (SELECT id FROM build_recipes WHERE name = 'r')",
                    [],
                )?;
                Ok(())
            })
            .expect("soft-delete v3, the latest");
        assert_eq!(
            edit_recipe(&writer, "r", "ARG d").expect("edit").version,
            4,
            "numbering counts soft-deleted versions, so UNIQUE never collides"
        );
    }

    /// Test 93: rollback to a version that no longer resolves still moves
    /// the pointer, and reports the problem.
    #[test]
    fn rollback_to_stale_version_reports_problem() {
        let (_tmp, writer, reader) = setup();
        add(&writer, "base", "");
        add(&writer, "top", "INVOKE base");
        edit_recipe(&writer, "top", "").expect("edit");
        soft_delete_recipe(&writer, "base").expect("delete base");
        let report = rollback_recipe(&writer, "top", 1).expect("rollback");
        assert!(report.moved, "the pointer moved");
        assert_eq!(
            kind(report.problem.expect("a problem is reported")),
            ParseErrorKind::UnknownRecipe {
                name: "base".into()
            },
            "the now-current version no longer resolves"
        );
        assert_eq!(
            get_recipe(&reader, "top").expect("get").row.current_version,
            1,
            "version 1 is current"
        );
    }

    /// Test 94: history lists live versions, oldest first, marking the current one.
    #[test]
    fn history_lists_versions_with_current_marker() {
        let (_tmp, writer, reader) = setup();
        add(&writer, "h", "ARG a");
        edit_recipe(&writer, "h", "ARG b").expect("edit");
        edit_recipe(&writer, "h", "ARG c").expect("edit");
        rollback_recipe(&writer, "h", 2).expect("rollback");
        let history = recipe_history(&reader, "h").expect("history");
        assert_eq!(
            history
                .iter()
                .map(|v| (v.version, v.current))
                .collect::<Vec<_>>(),
            vec![(1, false), (2, true), (3, false)],
            "every version, current marked"
        );
        assert!(
            history
                .windows(2)
                .all(|w| w[0].version_id < w[1].version_id),
            "version ids ascend with version numbers"
        );
        assert!(
            history.iter().all(|v| v.created_at.ends_with('Z')),
            "created_at is UTC ISO 8601"
        );
    }

    /// Test 95: list shows live recipes only, by name, with counts.
    #[test]
    fn list_excludes_soft_deleted() {
        let (_tmp, writer, reader) = setup();
        add(&writer, "b-recipe", "ARG a");
        edit_recipe(&writer, "b-recipe", "ARG b").expect("edit");
        add(&writer, "a-recipe", "");
        soft_delete_recipe(&writer, "a-recipe").expect("delete");
        let rows: Vec<(String, String, u32, u32)> = list_recipes(&reader)
            .expect("list")
            .into_iter()
            .map(|r| (r.name, r.curation, r.current_version, r.version_count))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("b-recipe".into(), "custom".into(), 2, 2),
                ("shipped-default".into(), "builtin".into(), 1, 1),
            ],
            "soft-deleted recipes are absent; rows sort by name"
        );
    }

    /// Test 96: builtin recipes cannot be deleted.
    #[test]
    fn soft_delete_builtin_errors() {
        let (_tmp, writer, reader) = setup();
        assert!(
            matches!(
                soft_delete_recipe(&writer, "shipped-default"),
                Err(Error::BuiltinProtected { name }) if name == "shipped-default"
            ),
            "the shipped recipe is protected"
        );
        assert!(
            get_recipe(&reader, "shipped-default").is_ok(),
            "it is still there"
        );
    }

    /// Test 97: after delete (and for names never created), every operation
    /// on the name is RecipeNotFound.
    #[test]
    fn soft_delete_then_lookup_not_found() {
        let (_tmp, writer, reader) = setup();
        add(&writer, "gone", "ARG a");
        soft_delete_recipe(&writer, "gone").expect("delete");
        for name in ["gone", "never"] {
            let not_found = |r: Result<()>, op: &str| {
                assert!(
                    matches!(r, Err(Error::RecipeNotFound(ref n)) if n == name),
                    "{op} {name}: expected RecipeNotFound, got {r:?}"
                );
            };
            not_found(get_recipe(&reader, name).map(|_| ()), "get");
            not_found(edit_recipe(&writer, name, "ARG z").map(|_| ()), "edit");
            not_found(rollback_recipe(&writer, name, 1).map(|_| ()), "rollback");
            not_found(recipe_history(&reader, name).map(|_| ()), "history");
            not_found(recipe_args(&reader, name).map(|_| ()), "args");
            not_found(
                resolve_stored(&reader, name, &ArgInput::Open).map(|_| ()),
                "resolve_stored",
            );
            not_found(soft_delete_recipe(&writer, name), "rm");
        }
    }

    /// Test 98: recipe_args lists required ARGs and defaults as written.
    #[test]
    fn recipe_args_lists_required_and_defaults() {
        let (_tmp, writer, reader) = setup();
        add(&writer, "args", "ARG a\nARG b=x\nARG c=${b}-y");
        let shown: Vec<(String, Option<String>, bool, u32)> = recipe_args(&reader, "args")
            .expect("args")
            .into_iter()
            .map(|a| (a.name, a.default, a.required, a.position.line))
            .collect();
        assert_eq!(
            shown,
            vec![
                ("a".into(), None, true, 1),
                ("b".into(), Some("x".into()), false, 2),
                ("c".into(), Some("${b}-y".into()), false, 3),
            ],
            "file order; defaults unsubstituted"
        );
    }
}
