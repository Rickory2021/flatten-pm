// src-cli/src/main.rs
//
// Flatten PM CLI. Shell access to all flatten-core operations.
//
// Exit codes: 0 success, 1 domain error, 2 usage error (clap handles 2).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use flatten_core::db;

// CLI Structure

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
    Repo,
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

// Path resolution

fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .expect("could not determine platform data directory")
        .join("flatten-pm")
}

fn resolve_db_path(cli: &Cli) -> PathBuf {
    if let Some(db) = &cli.db {
        db.clone()
    } else if let Some(dir) = &cli.data_dir {
        dir.join("flatten.db")
    } else {
        default_data_dir().join("flatten.db")
    }
}

// Output helpers

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

// Command handlers

fn cmd_db_init(cli: &Cli) -> Result<(), String> {
    let path = resolve_db_path(cli);
 
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create data directory: {e}"))?;
    }
 
    let _writer = db::writer::Writer::open(&path)
        .map_err(|e| format!("{e}"))?;
 
    // Read back user_version to confirm
    let conn = db::open_reader(&path)
        .map_err(|e| format!("{e}"))?;
    let version: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| format!("{e}"))?;
 
    print_ok(cli.json, serde_json::json!({ "user_version": version }));
    Ok(())
}
 
fn cmd_db_tables(cli: &Cli) -> Result<(), String> {
    let path = resolve_db_path(cli);
    let conn = db::open_reader(&path)
        .map_err(|e| format!("{e}"))?;
 
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
    let conn = db::open_reader(&path)
        .map_err(|e| format!("{e}"))?;
 
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| format!("{e}"))?;
 
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
 
fn cmd_settings_get(cli: &Cli, key: &str) -> Result<(), String> {
    let path = resolve_db_path(cli);
    let conn = db::open_reader(&path)
        .map_err(|e| format!("{e}"))?;
 
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
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            Err(format!("unknown key: {key}"))
        }
        Err(e) => Err(format!("{e}")),
    }
}
 
fn cmd_settings_set(cli: &Cli, key: &str, value: &str) -> Result<(), String> {
    let path = resolve_db_path(cli);
    let writer = db::writer::Writer::open(&path)
        .map_err(|e| format!("{e}"))?;
    
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
                return Err(db::error::Error::Writer(format!("unknown key: {key_owned}")));
            }
            Ok(())
        })
        .map_err(|e| format!("{e}"))?;
 
    print_ok(cli.json, serde_json::json!({ "key": key, "value": value }));
    Ok(())

}

// main

fn main() -> ExitCode {
    let cli = Cli::parse();
 
    let result = match &cli.command {
        CliCommand::Db(args) => match &args.command {
            DbCommand::Init => cmd_db_init(&cli),
            DbCommand::Tables => cmd_db_tables(&cli),
            DbCommand::Query { sql } => cmd_db_query(&cli, sql),
        },
        CliCommand::Settings(args) => match &args.command {
            SettingsCommand::Get { key } => cmd_settings_get(&cli, key),
            SettingsCommand::Set { key, value } => cmd_settings_set(&cli, key, value),
        },
 
        // Reserved groups
        CliCommand::Repo
        | CliCommand::Recipe
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
        Err(msg) => {
            print_err(cli.json, "database", &msg);
            ExitCode::from(1)
        }
    }
}
