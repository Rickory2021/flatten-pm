// src-cli/src/main.rs
//
// Flatten PM CLI. Shell access to all flatten-core operations.
//
// Exit codes: 0 success, 1 domain error, 2 usage error (clap handles 2).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use flatten_core::db;
use flatten_core::ingest;

// ---------------------------------------------------------------------------
// CLI structure
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "flatten", about = "Flatten PM CLI")]
struct Cli {
    /// Override the app data directory (tries, exports, database, PID file)
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,

    /// Override the database path (default: {data-dir}/flatten.db)
    #[arg(long, global = true)]
    db: Option<PathBuf>,

    /// Machine-readable JSON output on stdout
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Subcommand)]
enum CliCommand {
    /// Database management
    Db(DbArgs),
    /// Application settings
    Settings(SettingsArgs),
    /// Repository management
    Repo(RepoArgs),
    /// Recipe management
    Recipe,
    /// Transform management
    Transform,
    /// Template management
    Template,
    /// Binding management
    Binding,
    /// Export pipeline
    Export,
    /// Watch pipeline
    Watch,
    /// Watch match flags
    Flag,
    /// File history
    History,
}

// db subcommands

#[derive(Args)]
#[command(arg_required_else_help = true)]
struct DbArgs {
    #[command(subcommand)]
    command: DbCommand,
}

#[derive(Subcommand)]
enum DbCommand {
    /// Create and migrate the database
    Init,
    /// List all tables
    Tables,
    /// Run a read-only SQL query
    Query {
        /// SQL statement to execute (write statements are rejected)
        sql: String,
    },
}

// settings subcommands

#[derive(Args)]
#[command(arg_required_else_help = true)]
struct SettingsArgs {
    #[command(subcommand)]
    command: SettingsCommand,
}

#[derive(Subcommand)]
enum SettingsCommand {
    /// Get a setting value
    Get {
        /// Setting key
        key: String,
    },
    /// Set a setting value
    Set {
        /// Setting key
        key: String,
        /// New value
        value: String,
    },
}

// repo subcommands

#[derive(Args)]
#[command(arg_required_else_help = true)]
struct RepoArgs {
    #[command(subcommand)]
    command: RepoCommand,
}

#[derive(Subcommand)]
enum RepoCommand {
    /// Register a new source directory
    Add {
        /// Path to the source directory
        path: PathBuf,
        /// Unique name for the repo
        #[arg(long)]
        name: String,
        /// Gitignore-style exclude patterns (repeatable)
        #[arg(long = "pattern", num_args = 1)]
        patterns: Vec<String>,
        /// Import patterns from .gitignore files in the source
        #[arg(long)]
        import_gitignore: bool,
        /// Line ending policy: preserve (default) or lf
        #[arg(long, default_value = "preserve")]
        line_ending_policy: String,
    },
    /// List registered repos
    Ls,
    /// Print trie paths for a repo
    Tree {
        /// Repo name
        name: String,
    },
    /// Edit repo configuration
    Edit {
        /// Current repo name
        name: String,
        /// New name
        #[arg(long)]
        new_name: Option<String>,
        /// Replace patterns (repeatable; replaces entire list)
        #[arg(long = "pattern", num_args = 1)]
        patterns: Vec<String>,
        /// Line ending policy
        #[arg(long)]
        line_ending_policy: Option<String>,
    },
    /// Re-ingest a repo (full re-hash)
    Reingest {
        /// Repo name
        name: String,
    },
    /// Remove a repo (soft delete)
    Rm {
        /// Repo name
        name: String,
    },
    /// Show repo config history (requires DA-004)
    History {
        /// Repo name
        name: String,
    },
    /// Roll back repo config to a previous version (requires DA-004)
    Rollback {
        /// Repo name
        name: String,
        /// Version to roll back to
        version: i64,
    },
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// CLI error with a `kind` discriminator for JSON output.
struct CliError {
    kind: &'static str,
    msg: String,
}

impl From<ingest::error::Error> for CliError {
    fn from(e: ingest::error::Error) -> Self {
        let kind = match &e {
            ingest::error::Error::NonExistentPath { .. }
            | ingest::error::Error::DuplicateName { .. }
            | ingest::error::Error::RepoNotFound(_)
            | ingest::error::Error::InvalidPattern { .. }
            | ingest::error::Error::InvalidLineEndingPolicy { .. } => "domain",
            ingest::error::Error::Io { .. }
            | ingest::error::Error::Trie(flatten_core::trie::error::Error::Io { .. }) => "io",
            _ => "database",
        };
        CliError {
            kind,
            msg: e.to_string(),
        }
    }
}

impl From<db::error::Error> for CliError {
    fn from(e: db::error::Error) -> Self {
        CliError {
            kind: "database",
            msg: e.to_string(),
        }
    }
}

/// Backward compatibility: existing handlers return `Result<(), String>`.
impl From<String> for CliError {
    fn from(msg: String) -> Self {
        CliError {
            kind: "database",
            msg,
        }
    }
}

// ---------------------------------------------------------------------------
// Path resolution
// ---------------------------------------------------------------------------

fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .expect("could not determine platform data directory")
        .join("flatten-pm")
}

/// Resolve the data directory (for tries, exports, etc.).
fn resolve_data_dir(cli: &Cli) -> PathBuf {
    if let Some(dir) = &cli.data_dir {
        dir.clone()
    } else {
        default_data_dir()
    }
}

fn resolve_db_path(cli: &Cli) -> PathBuf {
    if let Some(db) = &cli.db {
        db.clone()
    } else {
        resolve_data_dir(cli).join("flatten.db")
    }
}

/// Open a writer, creating the DB's parent directory if needed.
/// First-run safety: `flatten repo add` is the natural first command
/// on a fresh machine; without this, Writer::open fails with an opaque
/// SQLite error.
fn open_writer(cli: &Cli) -> Result<db::writer::Writer, CliError> {
    let path = resolve_db_path(cli);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CliError {
            kind: "io",
            msg: format!("failed to create data directory: {e}"),
        })?;
    }
    Ok(db::writer::Writer::open(&path)?)
}

// ---------------------------------------------------------------------------
// Output helpers
// ---------------------------------------------------------------------------

fn print_ok(json: bool, data: serde_json::Value) {
    if json {
        let mut out = data;
        out.as_object_mut()
            .expect("data must be an object")
            .insert("ok".to_string(), serde_json::Value::Bool(true));
        println!("{}", serde_json::to_string(&out).unwrap());
    } else if let Some(obj) = data.as_object() {
        for (k, v) in obj {
            println!("{k}: {v}");
        }
    } else {
        println!("{data}");
    }
}

fn print_rows(json: bool, columns: &[String], rows: Vec<Vec<String>>) {
    if json {
        let json_rows: Vec<serde_json::Value> = rows
            .iter()
            .map(|row| {
                let obj: serde_json::Map<String, serde_json::Value> = columns
                    .iter()
                    .zip(row.iter())
                    .map(|(col, val)| (col.clone(), serde_json::Value::String(val.clone())))
                    .collect();
                serde_json::Value::Object(obj)
            })
            .collect();
        let out = serde_json::json!({ "ok": true, "rows": json_rows });
        println!("{}", serde_json::to_string(&out).unwrap());
    } else {
        if rows.is_empty() {
            println!("(no rows)");
            return;
        }
        println!("{}", columns.join("\t"));
        for row in &rows {
            println!("{}", row.join("\t"));
        }
    }
}

fn print_err(json: bool, kind: &str, msg: &str) {
    if json {
        let out = serde_json::json!({ "error": msg, "kind": kind });
        eprintln!("{}", serde_json::to_string(&out).unwrap());
    } else {
        eprintln!("error: {msg}");
    }
}

/// Format a root hash as a hex string.
fn hex_hash(hash: &[u8; 32]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Command handlers — db
// ---------------------------------------------------------------------------

fn cmd_db_init(cli: &Cli) -> Result<(), String> {
    let path = resolve_db_path(cli);

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create data directory: {e}"))?;
    }

    let _writer =
        db::writer::Writer::open(&path).map_err(|e| format!("{e}"))?;

    // Read back user_version to confirm
    let conn =
        db::open_reader(&path).map_err(|e| format!("{e}"))?;
    let version: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| format!("{e}"))?;

    print_ok(cli.json, serde_json::json!({ "user_version": version }));
    Ok(())
}

fn cmd_db_tables(cli: &Cli) -> Result<(), String> {
    let path = resolve_db_path(cli);
    let conn =
        db::open_reader(&path).map_err(|e| format!("{e}"))?;

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .map_err(|e| format!("{e}"))?;

    let columns = vec!["name".to_string()];
    let rows: Vec<Vec<String>> = stmt
        .query_map([], |row| {
            let name: String = row.get(0)?;
            Ok(vec![name])
        })
        .map_err(|e| format!("{e}"))?
        .filter_map(|r| r.ok())
        .collect();

    print_rows(cli.json, &columns, rows);
    Ok(())
}

fn cmd_db_query(cli: &Cli, sql: &str) -> Result<(), String> {
    let path = resolve_db_path(cli);
    let conn =
        db::open_reader(&path).map_err(|e| format!("{e}"))?;

    let mut stmt = conn.prepare(sql).map_err(|e| format!("{e}"))?;

    let columns: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let rows: Vec<Vec<String>> = stmt
        .query_map([], |row| {
            let mut vals = Vec::new();
            for i in 0..columns.len() {
                let val: String = row
                    .get::<_, rusqlite::types::Value>(i)
                    .map(|v| match v {
                        rusqlite::types::Value::Null => "NULL".to_string(),
                        rusqlite::types::Value::Integer(n) => n.to_string(),
                        rusqlite::types::Value::Real(f) => f.to_string(),
                        rusqlite::types::Value::Text(s) => s,
                        rusqlite::types::Value::Blob(b) => format!("<blob {} bytes>", b.len()),
                    })
                    .unwrap_or_else(|_| "ERROR".to_string());
                vals.push(val);
            }
            Ok(vals)
        })
        .map_err(|e| format!("{e}"))?
        .filter_map(|r| r.ok())
        .collect();

    print_rows(cli.json, &columns, rows);
    Ok(())
}

// ---------------------------------------------------------------------------
// Command handlers — settings
// ---------------------------------------------------------------------------

fn cmd_settings_get(cli: &Cli, key: &str) -> Result<(), String> {
    let path = resolve_db_path(cli);
    let conn =
        db::open_reader(&path).map_err(|e| format!("{e}"))?;

    let result: Result<String, _> = conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        [key],
        |row| row.get(0),
    );

    match result {
        Ok(v) => {
            print_ok(cli.json, serde_json::json!({ "key": key, "value": v }));
            Ok(())
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(format!("unknown key: {key}")),
        Err(e) => Err(format!("{e}")),
    }
}

fn cmd_settings_set(cli: &Cli, key: &str, value: &str) -> Result<(), String> {
    let path = resolve_db_path(cli);
    let writer =
        db::writer::Writer::open(&path).map_err(|e| format!("{e}"))?;

    let key_owned = key.to_string();
    let value_owned = value.to_string();

    writer
        .call_write(move |conn| {
            let changed = conn
                .execute(
                    "UPDATE settings SET value = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE key = ?2",
                    rusqlite::params![value_owned, key_owned],
                )
                .map_err(db::error::Error::from)?;

            if changed == 0 {
                return Err(db::error::Error::Writer(format!(
                    "unknown key: {key_owned}"
                )));
            }
            Ok(())
        })
        .map_err(|e| format!("{e}"))?;

    print_ok(cli.json, serde_json::json!({ "key": key, "value": value }));
    Ok(())
}

// ---------------------------------------------------------------------------
// Command handlers — repo
// ---------------------------------------------------------------------------

fn cmd_repo_add(cli: &Cli, args: &RepoAddArgs) -> Result<(), CliError> {
    let writer = open_writer(cli)?;
    let data_dir = resolve_data_dir(cli);

    let (id, report) = ingest::register_repo(
        &writer,
        &data_dir,
        &args.path,
        &args.name,
        &args.patterns,
        &args.line_ending_policy,
        args.import_gitignore,
    )?;

    print_ok(
        cli.json,
        serde_json::json!({
            "id": id,
            "file_count": report.file_count,
            "root_hash": hex_hash(&report.root_hash),
            "lossy_count": report.lossy_count,
        }),
    );
    Ok(())
}

/// Destructured `RepoCommand::Add` fields for `cmd_repo_add`.
struct RepoAddArgs {
    path: PathBuf,
    name: String,
    patterns: Vec<String>,
    import_gitignore: bool,
    line_ending_policy: String,
}

fn cmd_repo_ls(cli: &Cli) -> Result<(), CliError> {
    let writer = open_writer(cli)?;
    drop(writer); // only needed to ensure DB exists

    let conn = db::open_reader(&resolve_db_path(cli))?;
    let repos = ingest::list_repos(&conn)?;

    if cli.json {
        let out = serde_json::json!({
            "ok": true,
            "repos": serde_json::to_value(&repos).map_err(|e| CliError {
                kind: "database",
                msg: e.to_string(),
            })?,
        });
        println!("{}", serde_json::to_string(&out).unwrap());
    } else {
        let columns = vec![
            "id".to_string(),
            "name".to_string(),
            "path".to_string(),
            "trie_updated_at".to_string(),
        ];
        let rows: Vec<Vec<String>> = repos
            .iter()
            .map(|r| {
                vec![
                    r.id.to_string(),
                    r.name.clone(),
                    r.path.clone(),
                    r.trie_updated_at.clone().unwrap_or_default(),
                ]
            })
            .collect();
        print_rows(cli.json, &columns, rows);
    }
    Ok(())
}

fn cmd_repo_tree(cli: &Cli, name: &str) -> Result<(), CliError> {
    let writer = open_writer(cli)?;
    let data_dir = resolve_data_dir(cli);
    let conn = db::open_reader(&resolve_db_path(cli))?;

    let repo = ingest::get_repo_by_name(&conn, name)?;
    let (trie, report) = ingest::load_or_reingest(&writer, &data_dir, repo.id)?;

    if let Some(_report) = report {
        eprintln!("trie regenerated");
    }

    let paths = trie.list("");

    if cli.json {
        print_ok(cli.json, serde_json::json!({ "paths": paths }));
    } else {
        for p in &paths {
            println!("{p}");
        }
    }
    Ok(())
}

fn cmd_repo_edit(
    cli: &Cli,
    name: &str,
    new_name: Option<&str>,
    patterns: &[String],
    line_ending_policy: Option<&str>,
) -> Result<(), CliError> {
    let writer = open_writer(cli)?;
    let data_dir = resolve_data_dir(cli);
    let conn = db::open_reader(&resolve_db_path(cli))?;

    let repo = ingest::get_repo_by_name(&conn, name)?;

    // Empty patterns vec means "don't change"
    let new_patterns = if patterns.is_empty() {
        None
    } else {
        Some(patterns)
    };

    let (_, report) = ingest::edit_repo(
        &writer,
        &data_dir,
        repo.id,
        new_name,
        new_patterns,
        line_ending_policy,
    )?;

    print_ok(
        cli.json,
        serde_json::json!({
            "file_count": report.file_count,
            "root_hash": hex_hash(&report.root_hash),
            "lossy_count": report.lossy_count,
        }),
    );
    Ok(())
}

fn cmd_repo_reingest(cli: &Cli, name: &str) -> Result<(), CliError> {
    let writer = open_writer(cli)?;
    let data_dir = resolve_data_dir(cli);
    let conn = db::open_reader(&resolve_db_path(cli))?;

    let repo = ingest::get_repo_by_name(&conn, name)?;
    let (_, report) = ingest::reingest(&writer, &data_dir, repo.id)?;

    print_ok(
        cli.json,
        serde_json::json!({
            "file_count": report.file_count,
            "root_hash": hex_hash(&report.root_hash),
            "lossy_count": report.lossy_count,
        }),
    );
    Ok(())
}

fn cmd_repo_rm(cli: &Cli, name: &str) -> Result<(), CliError> {
    let writer = open_writer(cli)?;
    let conn = db::open_reader(&resolve_db_path(cli))?;

    let repo = ingest::get_repo_by_name(&conn, name)?;
    ingest::soft_delete_repo(&writer, repo.id)?;

    print_ok(
        cli.json,
        serde_json::json!({
            "id": repo.id,
            "name": repo.name,
        }),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result: Result<(), CliError> = match &cli.command {
        CliCommand::Db(args) => match &args.command {
            DbCommand::Init => cmd_db_init(&cli).map_err(CliError::from),
            DbCommand::Tables => cmd_db_tables(&cli).map_err(CliError::from),
            DbCommand::Query { sql } => cmd_db_query(&cli, sql).map_err(CliError::from),
        },
        CliCommand::Settings(args) => match &args.command {
            SettingsCommand::Get { key } => {
                cmd_settings_get(&cli, key).map_err(CliError::from)
            }
            SettingsCommand::Set { key, value } => {
                cmd_settings_set(&cli, key, value).map_err(CliError::from)
            }
        },

        CliCommand::Repo(args) => match &args.command {
            RepoCommand::Add {
                path,
                name,
                patterns,
                import_gitignore,
                line_ending_policy,
            } => cmd_repo_add(
                &cli,
                &RepoAddArgs {
                    path: path.clone(),
                    name: name.clone(),
                    patterns: patterns.clone(),
                    import_gitignore: *import_gitignore,
                    line_ending_policy: line_ending_policy.clone(),
                },
            ),
            RepoCommand::Ls => cmd_repo_ls(&cli),
            RepoCommand::Tree { name } => cmd_repo_tree(&cli, name),
            RepoCommand::Edit {
                name,
                new_name,
                patterns,
                line_ending_policy,
            } => cmd_repo_edit(
                &cli,
                name,
                new_name.as_deref(),
                patterns,
                line_ending_policy.as_deref(),
            ),
            RepoCommand::Reingest { name } => cmd_repo_reingest(&cli, name),
            RepoCommand::Rm { name } => cmd_repo_rm(&cli, name),
            RepoCommand::History { .. } | RepoCommand::Rollback { .. } => {
                print_err(cli.json, "usage", "subcommand not yet implemented");
                return ExitCode::from(2);
            }
        },

        // Reserved groups (other than Repo, which is now wired)
        CliCommand::Recipe
        | CliCommand::Transform
        | CliCommand::Template
        | CliCommand::Binding
        | CliCommand::Export
        | CliCommand::Watch
        | CliCommand::Flag
        | CliCommand::History => {
            print_err(cli.json, "usage", "subcommand not yet implemented");
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            print_err(cli.json, e.kind, &e.msg);
            ExitCode::from(1)
        }
    }
}
