// crates/flatten-core/src/db/seed.rs

use super::error::Result;
use rusqlite::Connection;

/// Insert builtin data (no-op if already seeded).
///
/// Source content is placeholder. When real transforms/templates land,
/// move builtin content to a builtins/ directory and use include_str!().
pub(crate) fn run(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;

    // -- transforms (5) --------------------------------------------------
    // Parent rows with curation=builtin, current_version_id initially NULL.
    // Version rows with placeholder source. Update parent to point at version.

    tx.execute(
        "INSERT OR IGNORE INTO transforms (id, name, scope, curation) VALUES (1, 'flatten', 'directory', 'builtin')",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO transform_versions (id, transform_id, version, source) VALUES (1, 1, 1, '// flatten transform placeholder')",
        [],
    )?;
    tx.execute(
        "UPDATE transforms SET current_version_id = 1 WHERE id = 1 AND current_version_id IS NULL",
        [],
    )?;

    tx.execute(
        "INSERT OR IGNORE INTO transforms (id, name, scope, curation) VALUES (2, 'pack', 'directory', 'builtin')",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO transform_versions (id, transform_id, version, source) VALUES (2, 2, 1, '// pack transform placeholder')",
        [],
    )?;
    tx.execute(
        "UPDATE transforms SET current_version_id = 2 WHERE id = 2 AND current_version_id IS NULL",
        [],
    )?;

    tx.execute(
        "INSERT OR IGNORE INTO transforms (id, name, scope, curation) VALUES (3, 'enrichment-injection', 'file', 'builtin')",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO transform_versions (id, transform_id, version, source) VALUES (3, 3, 1, '// enrichment-injection transform placeholder')",
        [],
    )?;
    tx.execute(
        "UPDATE transforms SET current_version_id = 3 WHERE id = 3 AND current_version_id IS NULL",
        [],
    )?;

    tx.execute(
        "INSERT OR IGNORE INTO transforms (id, name, scope, curation, reverses) VALUES (4, 'enrichment-trim', 'file', 'builtin', 'enrichment-injection')",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO transform_versions (id, transform_id, version, source) VALUES (4, 4, 1, '// enrichment-trim transform placeholder')",
        [],
    )?;
    tx.execute(
        "UPDATE transforms SET current_version_id = 4 WHERE id = 4 AND current_version_id IS NULL",
        [],
    )?;

    tx.execute(
        "INSERT OR IGNORE INTO transforms (id, name, scope, curation) VALUES (5, 'context-manifest', 'directory', 'builtin')",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO transform_versions (id, transform_id, version, source) VALUES (5, 5, 1, '// context-manifest transform placeholder')",
        [],
    )?;
    tx.execute(
        "UPDATE transforms SET current_version_id = 5 WHERE id = 5 AND current_version_id IS NULL",
        [],
    )?;

    // -- templates (2) ----------------------------------------------------

    tx.execute(
        "INSERT OR IGNORE INTO templates (id, name, curation, kind) VALUES (1, 'default', 'builtin', 'enrichment')",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO template_versions (id, template_id, version, source) VALUES (1, 1, 1, '// default enrichment template placeholder')",
        [],
    )?;
    tx.execute(
        "UPDATE templates SET current_version_id = 1 WHERE id = 1 AND current_version_id IS NULL",
        [],
    )?;

    tx.execute(
        "INSERT OR IGNORE INTO templates (id, name, curation, kind) VALUES (2, 'context-manifest', 'builtin', 'file')",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO template_versions (id, template_id, version, source) VALUES (2, 2, 1, '// context-manifest file template placeholder')",
        [],
    )?;
    tx.execute(
        "UPDATE templates SET current_version_id = 2 WHERE id = 2 AND current_version_id IS NULL",
        [],
    )?;

    // -- recipe (1) -------------------------------------------------------

    tx.execute(
        "INSERT OR IGNORE INTO build_recipes (id, name, curation) VALUES (1, 'shipped-default', 'builtin')",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO build_recipe_versions (id, build_recipe_id, version, source) VALUES (1, 1, 1, '// shipped-default recipe placeholder')",
        [],
    )?;
    tx.execute(
        "UPDATE build_recipes SET current_version_id = 1 WHERE id = 1 AND current_version_id IS NULL",
        [],
    )?;

    // -- settings (9) -----------------------------------------------------

    tx.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('watch_source_dir', '')",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('watch_poll_interval_ms', '30000')",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('watch_settle_ms', '500')",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('watch_debounce_ms', '250')",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('trie_refresh_interval_ms', '30000')",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('transform_timeout_ms', '10000')",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('transform_memory_mb', '256')",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('copy_size_limit_mb', '64')",
        [],
    )?;
    tx.execute("INSERT OR IGNORE INTO settings (key, value) VALUES ('binary_extensions', '.png,.jpg,.jpeg,.gif,.bmp,.ico,.webp,.pdf,.zip,.gz,.tar,.7z,.rar,.exe,.dll,.so,.dylib,.woff,.woff2,.ttf,.eot,.mp3,.mp4,.wav,.avi,.mov,.db,.sqlite')", [])?;

    // -- change_counter ---------------------------------------------------

    tx.execute(
        "INSERT OR IGNORE INTO change_counter (id, counter) VALUES (1, 0)",
        [],
    )?;

    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::db::open_reader;
    use crate::db::writer::Writer;

    /// Helper: create a seeded database, return tempfile and reader connection.
    fn seeded_db() -> (tempfile::NamedTempFile, rusqlite::Connection) {
        let tmp = tempfile::NamedTempFile::new().expect("failed to create temp file");
        let _writer = Writer::open(tmp.path()).expect("Writer::open failed");
        let conn = open_reader(tmp.path()).expect("open_reader failed");
        (tmp, conn)
    }

    /// Seed inserts exactly 5 builtin transforms.
    #[test]
    fn seeds_5_transforms() {
        let (_tmp, conn) = seeded_db();
        let count: i32 = conn
            .query_row(
                "SELECT count(*) FROM transforms WHERE curation = 'builtin'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 5);
    }

    /// Seed inserts exactly 2 builtin templates.
    #[test]
    fn seeds_2_templates() {
        let (_tmp, conn) = seeded_db();
        let count: i32 = conn
            .query_row(
                "SELECT count(*) FROM templates WHERE curation = 'builtin'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    /// Seed inserts exactly 1 builtin recipe.
    #[test]
    fn seeds_1_recipe() {
        let (_tmp, conn) = seeded_db();
        let count: i32 = conn
            .query_row(
                "SELECT count(*) FROM build_recipes WHERE curation = 'builtin'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    /// Seed inserts exactly 9 settings rows.
    #[test]
    fn seeds_9_settings() {
        let (_tmp, conn) = seeded_db();
        let count: i32 = conn
            .query_row("SELECT count(*) FROM settings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 9);
    }

    /// Seed inserts the change_counter row with counter = 0.
    #[test]
    fn seeds_change_counter() {
        let (_tmp, conn) = seeded_db();
        let counter: i32 = conn
            .query_row(
                "SELECT counter FROM change_counter WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(counter, 0);
    }

    /// All pointer-versioned entities have current_version_id set.
    #[test]
    fn all_builtins_have_current_version() {
        let (_tmp, conn) = seeded_db();

        let null_transforms: i32 = conn.query_row(
            "SELECT count(*) FROM transforms WHERE curation = 'builtin' AND current_version_id IS NULL",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(
            null_transforms, 0,
            "all transforms should have current_version_id set"
        );

        let null_templates: i32 = conn.query_row(
            "SELECT count(*) FROM templates WHERE curation = 'builtin' AND current_version_id IS NULL",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(
            null_templates, 0,
            "all templates should have current_version_id set"
        );

        let null_recipes: i32 = conn.query_row(
            "SELECT count(*) FROM build_recipes WHERE curation = 'builtin' AND current_version_id IS NULL",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(
            null_recipes, 0,
            "all recipes should have current_version_id set"
        );
    }

    /// Seed is idempotent: running open twice produces the same row counts.
    #[test]
    fn seed_idempotent() {
        let tmp = tempfile::NamedTempFile::new().expect("failed to create temp file");

        let w1 = Writer::open(tmp.path()).expect("first open failed");
        drop(w1);
        let w2 = Writer::open(tmp.path()).expect("second open failed");
        drop(w2);

        let conn = open_reader(tmp.path()).expect("open_reader failed");

        let transforms: i32 = conn
            .query_row("SELECT count(*) FROM transforms", [], |row| row.get(0))
            .unwrap();
        let settings: i32 = conn
            .query_row("SELECT count(*) FROM settings", [], |row| row.get(0))
            .unwrap();

        assert_eq!(
            transforms, 5,
            "should still be 5 transforms after double-open"
        );
        assert_eq!(settings, 9, "should still be 9 settings after double-open");
    }
}
