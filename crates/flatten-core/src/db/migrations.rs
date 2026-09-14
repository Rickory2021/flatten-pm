// crates/flatten-core/src/db/migrations.rs
//
// Schema migrations for flatten-core's SQLite database.
//
// Use rusqlite_migration with PRAGMA user_version tracking.
// One M::up() per schema version. M::down() after v1.
//
// PRAGMAs must not go in migrations (no-op inside transactions).
// IF NOT EXISTS on all DDL for concurrent-init safety.

use rusqlite_migration::{M, Migrations};

// SQLite does not enforce FK targets at DDL time (only on DML),
//  so circular FKs between parent and version tables are safe.
const MIGRATIONS_SLICE: &[M<'_>] = &[
    M::up(r#"
-- v1: initial schema
-- All _at columns are TEXT in ISO 8601 (UTC).
-- All JSON columns are TEXT (no JSONB).

CREATE TABLE IF NOT EXISTS repos (
    id                  INTEGER PRIMARY KEY,
    path                TEXT    NOT NULL,
    name                TEXT    NOT NULL UNIQUE,
    ingest_patterns     TEXT    NOT NULL,       -- JSON array
    line_ending_policy  TEXT    NOT NULL DEFAULT 'preserve',    -- 'preserve' | 'lf'
    safety_allowlist    TEXT,       -- JSON, nullable
    trie_updated_at     TEXT,
    created_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    deleted_at          TEXT        -- soft delete
);
 
CREATE TABLE IF NOT EXISTS repo_versions (
    id                 INTEGER  PRIMARY KEY,
    repo_id            INTEGER  NOT NULL REFERENCES repos(id),
    version            INTEGER  NOT NULL,
    ingest_patterns    TEXT     NOT NULL,       -- JSON
    line_ending_policy TEXT     NOT NULL,
    safety_allowlist   TEXT,        -- JSON, nullable
    created_at         TEXT     NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
 
-- current_version_id nullable to break circular FK insert cycle.
-- curation column required by DA-001 AC (missing from ER diagram).
CREATE TABLE IF NOT EXISTS build_recipes (
    id                 INTEGER  PRIMARY KEY,
    name               TEXT     NOT NULL UNIQUE,
    curation           TEXT     NOT NULL DEFAULT 'custom',  -- 'builtin' | 'custom'
    current_version_id INTEGER  REFERENCES build_recipe_versions(id),
    deleted_at         TEXT
);
 
CREATE TABLE IF NOT EXISTS build_recipe_versions (
    id              INTEGER PRIMARY KEY,
    build_recipe_id INTEGER NOT NULL REFERENCES build_recipes(id),
    version         INTEGER NOT NULL,
    source          TEXT    NOT NULL,
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    deleted_at      TEXT,
    UNIQUE(build_recipe_id, version)
);
 
CREATE TABLE IF NOT EXISTS transforms (
    id                 INTEGER  PRIMARY KEY,
    name               TEXT     NOT NULL UNIQUE,
    scope              TEXT     NOT NULL,       -- 'file' | 'directory'
    curation           TEXT     NOT NULL DEFAULT 'custom',      -- 'builtin' | 'custom'
    reverses           TEXT,        -- references transforms.name (logical, not FK)
    current_version_id INTEGER  REFERENCES transform_versions(id),
    deleted_at         TEXT
);
 
CREATE TABLE IF NOT EXISTS transform_versions (
    id           INTEGER    PRIMARY KEY,
    transform_id INTEGER    NOT NULL REFERENCES transforms(id),
    version      INTEGER    NOT NULL,
    source       TEXT       NOT NULL,
    created_at   TEXT       NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    deleted_at   TEXT,
    UNIQUE(transform_id, version)
);
 
CREATE TABLE IF NOT EXISTS templates (
    id                 INTEGER  PRIMARY KEY,
    name               TEXT     NOT NULL UNIQUE,
    curation           TEXT     NOT NULL DEFAULT 'custom',      -- 'builtin' | 'custom'
    kind               TEXT     NOT NULL,       -- 'enrichment' | 'file'
    current_version_id INTEGER  REFERENCES template_versions(id),
    deleted_at         TEXT
);
 
CREATE TABLE IF NOT EXISTS template_versions (
    id          INTEGER PRIMARY KEY,
    template_id INTEGER NOT NULL REFERENCES templates(id),
    version     INTEGER NOT NULL,
    source      TEXT    NOT NULL,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    deleted_at  TEXT,
    UNIQUE(template_id, version)
);
 
CREATE TABLE IF NOT EXISTS pipeline_bindings (
    id              INTEGER PRIMARY KEY,
    build_recipe_id INTEGER NOT NULL REFERENCES build_recipes(id),
    version_pin     INTEGER,    -- null = follow current
    active          INTEGER NOT NULL DEFAULT 0,     -- boolean: 0 = inactive, 1 = active
    arg_values      TEXT    NOT NULL DEFAULT '{}',      -- JSON
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_used_at    TEXT
);
 
-- ON DELETE CASCADE: binding deletion removes export_state,
-- which cascades to safety_findings.
CREATE TABLE IF NOT EXISTS export_state (
    id                  INTEGER PRIMARY KEY,
    pipeline_binding_id INTEGER NOT NULL UNIQUE
        REFERENCES pipeline_bindings(id) ON DELETE CASCADE,
    output_dir          TEXT    NOT NULL,
    resolution_rules    TEXT    NOT NULL,       -- JSON
    runtime_versions    TEXT    NOT NULL,       -- JSON
    runtime_inputs      TEXT    NOT NULL,       -- JSON
    repo_root_hashes    TEXT    NOT NULL        -- JSON
);
 
CREATE TABLE IF NOT EXISTS watch_match_flags (
    id             INTEGER  PRIMARY KEY,
    flag_type      TEXT     NOT NULL,        -- 'ambiguous' | 'new_directory' | 'unroutable'
    content        BLOB     NOT NULL,
    source_path    TEXT     NOT NULL,
    extracted_path TEXT     NOT NULL,
    created_at     TEXT     NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
 
-- FK actions: flag -> CASCADE, binding -> SET NULL, version -> SET NULL
CREATE TABLE IF NOT EXISTS watch_match_candidates (
    id                      INTEGER PRIMARY KEY,
    watch_match_flag_id     INTEGER NOT NULL
        REFERENCES watch_match_flags(id) ON DELETE CASCADE,
    pipeline_binding_id     INTEGER
        REFERENCES pipeline_bindings(id) ON DELETE SET NULL,
    build_recipe_version_id INTEGER
        REFERENCES build_recipe_versions(id) ON DELETE SET NULL,
    repo_id                 INTEGER NOT NULL REFERENCES repos(id),
    target_path             TEXT,
    copy_key                TEXT,
    status                  TEXT    NOT NULL,      -- 'ok' | 'exceeds_depth' | 'excluded' | 'no_match'
    resolved_reverse_chain  TEXT    NOT NULL,      -- JSON
    template_set_version    INTEGER,
    UNIQUE(watch_match_flag_id, pipeline_binding_id, repo_id)
);
 
-- CASCADE from export_state.
CREATE TABLE IF NOT EXISTS safety_findings (
    id              INTEGER PRIMARY KEY,
    export_state_id INTEGER NOT NULL
        REFERENCES export_state(id) ON DELETE CASCADE,
    repo_id         INTEGER NOT NULL REFERENCES repos(id),
    file_path       TEXT    NOT NULL,
    finding_type    TEXT    NOT NULL,       -- 'secret_detected' | 'pii_detected'
    rule_id         TEXT    NOT NULL,
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(export_state_id, repo_id, file_path, rule_id)
);
 
-- repo_id nullable (packed/emitted files).
-- build_recipe_version_id ON DELETE SET NULL.
CREATE TABLE IF NOT EXISTS file_history (
    id                      INTEGER PRIMARY KEY,
    repo_id                 INTEGER REFERENCES repos(id),
    build_recipe_version_id INTEGER
        REFERENCES build_recipe_versions(id) ON DELETE SET NULL,
    file_path               TEXT    NOT NULL,
    entry_type              TEXT    NOT NULL,   -- 'snapshot' | 'diff'
    content                 BLOB   NOT NULL,
    placed_by               TEXT    NOT NULL,   -- 'export' | 'watch_auto' | 'watch_manual'
    source_path             TEXT,
    created_at              TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
 
CREATE INDEX IF NOT EXISTS idx_file_history_lookup
    ON file_history(repo_id, file_path, created_at);
 
CREATE TABLE IF NOT EXISTS settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
 
CREATE TABLE IF NOT EXISTS change_counter (
    id      INTEGER PRIMARY KEY,        -- single row, id = 1
    counter INTEGER NOT NULL DEFAULT 0
);"#,)
    .foreign_key_check(),
    // v2 example:
    // M::up("ALTER TABLE repos ADD COLUMN new_col TEXT;")
    //     .down("ALTER TABLE repos DROP COLUMN new_col;")
    //     .foreign_key_check(),
];
pub(crate) const MIGRATIONS: Migrations<'_> = Migrations::from_slice(MIGRATIONS_SLICE);

#[cfg(test)]
mod tests {
    use super::*;

    ///MIGRATIONS SQL parses without error.
    #[test]
    fn validates() {
        MIGRATIONS.validate().expect("migration validation failed")
    }
}
