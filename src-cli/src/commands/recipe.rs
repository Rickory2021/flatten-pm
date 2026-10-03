// src-cli/src/commands/recipe.rs
//
// `flatten recipe ...`: add, new, show, args, edit, history, rollback, ls,
// rm, lint. Each subcommand calls one `flatten_core::recipe` store function.
//
// Output streams: for `lint`, warnings are the product and go to stdout. For
// add, new, edit, show, and rollback they are side information and go to
// stderr as `warning: ...` in text mode. With `--json`, everything that is
// not an error is in the stdout object; errors are JSON on stderr with a
// `kind` and, for parse errors, a `detail` location.
//
// `show --raw` prints the stored text byte for byte (no added newline, no
// resolve, so a broken stored version still prints) and wins over `--json`.
// `lint <target>` lints a file when `target` is an existing file path, and a
// stored recipe by name otherwise.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use flatten_core::db;
use flatten_core::recipe::{self, ArgInput, LintWarning, Recipe};
use serde_json::{Value, json};

use crate::{Cli, CliError, open_writer, print_ok, print_rows, resolve_db_path};

#[derive(Args)]
#[command(arg_required_else_help = true)]
pub struct RecipeArgs {
    #[command(subcommand)]
    command: RecipeCommand,
}

#[derive(Subcommand)]
enum RecipeCommand {
    /// Save a new recipe from a file
    Add {
        /// Recipe file (UTF-8)
        file: PathBuf,
        /// Unique recipe name
        #[arg(long)]
        name: String,
    },
    /// Save a new recipe from the shipped default text
    New {
        /// Unique recipe name
        name: String,
    },
    /// Show a recipe's resolved instructions (or its text, with --raw)
    Show {
        /// Recipe name
        name: String,
        /// Print the stored text exactly (wins over --json)
        #[arg(long)]
        raw: bool,
    },
    /// List a recipe's ARG declarations
    Args {
        /// Recipe name
        name: String,
    },
    /// Save a new version of a recipe from a file
    Edit {
        /// Recipe name
        name: String,
        /// Recipe file (UTF-8)
        file: PathBuf,
    },
    /// List a recipe's versions
    History {
        /// Recipe name
        name: String,
    },
    /// Make an earlier version current
    Rollback {
        /// Recipe name
        name: String,
        /// Version to make current
        version: u32,
    },
    /// List recipes
    Ls,
    /// Remove a recipe (soft delete; the name stays taken)
    Rm {
        /// Recipe name
        name: String,
    },
    /// Lint a recipe file, or a stored recipe by name
    Lint {
        /// A file path, or a recipe name
        target: String,
    },
}

/// Run a `recipe` subcommand.
pub fn run(cli: &Cli, args: &RecipeArgs) -> Result<(), CliError> {
    // Opening the writer creates, migrates, and seeds the database on first
    // use, so read-only subcommands work on a fresh data directory too.
    let writer = open_writer(cli)?;
    let reader = || db::open_reader(&resolve_db_path(cli)).map_err(CliError::from);

    match &args.command {
        RecipeCommand::Add { file, name } => {
            let source = read_file(file)?;
            let report = recipe::add_recipe(&writer, name, &source)?;
            print_save(cli.json, name, &report)
        }
        RecipeCommand::New { name } => {
            let report = recipe::new_recipe(&writer, name)?;
            print_save(cli.json, name, &report)
        }
        RecipeCommand::Show { name, raw } => {
            let conn = reader()?;
            if *raw {
                print!("{}", recipe::get_recipe(&conn, name)?.source);
                return Ok(());
            }
            // One snapshot for the detail and the resolution, so a pointer
            // move between the two reads cannot mix versions.
            let tx = conn.unchecked_transaction().map_err(db_err)?;
            let detail = recipe::get_recipe(&tx, name)?;
            let resolution = recipe::resolve_stored(&tx, name, &ArgInput::Open)?;
            drop(tx);
            if cli.json {
                print_ok(
                    true,
                    json!({
                        "name": detail.row.name,
                        "version": detail.row.current_version,
                        "version_id": detail.version_id,
                        "recipe": to_json(&resolution.recipe)?,
                        "warnings": to_json(&resolution.warnings)?,
                    }),
                );
            } else {
                println!("{} v{}", detail.row.name, detail.row.current_version);
                print_outline(&resolution.recipe);
                warn_stderr(&resolution.warnings);
            }
            Ok(())
        }
        RecipeCommand::Args { name } => {
            let args = recipe::recipe_args(&reader()?, name)?;
            if cli.json {
                print_ok(true, json!({ "args": to_json(&args)? }));
            } else {
                let rows = args
                    .iter()
                    .map(|a| {
                        vec![
                            a.name.clone(),
                            a.default.clone().unwrap_or_default(),
                            a.required.to_string(),
                        ]
                    })
                    .collect();
                print_rows(false, &columns(&["name", "default", "required"]), rows);
            }
            Ok(())
        }
        RecipeCommand::Edit { name, file } => {
            let source = read_file(file)?;
            let report = recipe::edit_recipe(&writer, name, &source)?;
            if cli.json {
                print_ok(
                    true,
                    json!({
                        "version": report.version,
                        "version_id": report.version_id,
                        "unchanged": report.unchanged,
                        "warnings": to_json(&report.warnings)?,
                    }),
                );
            } else {
                print_ok(
                    false,
                    json!({ "version": report.version, "unchanged": report.unchanged }),
                );
                warn_stderr(&report.warnings);
            }
            Ok(())
        }
        RecipeCommand::History { name } => {
            let versions = recipe::recipe_history(&reader()?, name)?;
            if cli.json {
                let rows: Vec<Value> = versions
                    .iter()
                    .map(|v| {
                        json!({
                            "version": v.version,
                            "version_id": v.version_id,
                            "created_at": v.created_at,
                            "current": v.current,
                        })
                    })
                    .collect();
                print_ok(true, json!({ "versions": rows }));
            } else {
                let rows = versions
                    .iter()
                    .map(|v| {
                        vec![
                            v.version.to_string(),
                            v.version_id.to_string(),
                            v.created_at.clone(),
                            v.current.to_string(),
                        ]
                    })
                    .collect();
                print_rows(
                    false,
                    &columns(&["version", "version_id", "created_at", "current"]),
                    rows,
                );
            }
            Ok(())
        }
        RecipeCommand::Rollback { name, version } => {
            let report = recipe::rollback_recipe(&writer, name, *version)?;
            if cli.json {
                print_ok(
                    true,
                    json!({
                        "name": name,
                        "current_version": report.version,
                        "moved": report.moved,
                        "problem": report.problem.as_ref().map(|e| e.to_string()),
                        "warnings": to_json(&report.warnings)?,
                    }),
                );
            } else {
                print_ok(false, json!({ "current_version": report.version }));
                if let Some(problem) = &report.problem {
                    eprintln!(
                        "warning: version {} does not resolve: {problem}",
                        report.version
                    );
                }
                warn_stderr(&report.warnings);
            }
            Ok(())
        }
        RecipeCommand::Ls => {
            let recipes = recipe::list_recipes(&reader()?)?;
            if cli.json {
                let rows: Vec<Value> = recipes
                    .iter()
                    .map(|r| {
                        json!({
                            "id": r.id,
                            "name": r.name,
                            "curation": r.curation,
                            "current_version": r.current_version,
                            "version_count": r.version_count,
                        })
                    })
                    .collect();
                print_ok(true, json!({ "recipes": rows }));
            } else {
                let rows = recipes
                    .iter()
                    .map(|r| {
                        vec![
                            r.id.to_string(),
                            r.name.clone(),
                            r.curation.clone(),
                            r.current_version.to_string(),
                            r.version_count.to_string(),
                        ]
                    })
                    .collect();
                print_rows(
                    false,
                    &columns(&["id", "name", "curation", "current_version", "versions"]),
                    rows,
                );
            }
            Ok(())
        }
        RecipeCommand::Rm { name } => {
            recipe::soft_delete_recipe(&writer, name)?;
            print_ok(cli.json, json!({ "deleted": name }));
            Ok(())
        }
        RecipeCommand::Lint { target } => {
            let conn = reader()?;
            let resolution = if Path::new(target).is_file() {
                recipe::lint_text(&conn, &read_file(Path::new(target))?)?
            } else {
                recipe::resolve_stored(&conn, target, &ArgInput::Open)?
            };
            if cli.json {
                print_ok(true, json!({ "warnings": to_json(&resolution.warnings)? }));
            } else if resolution.warnings.is_empty() {
                println!("no warnings");
            } else {
                for w in &resolution.warnings {
                    println!("{w}");
                }
            }
            Ok(())
        }
    }
}

/// Text or JSON for add and new.
fn print_save(json: bool, name: &str, report: &recipe::SaveReport) -> Result<(), CliError> {
    if json {
        print_ok(
            true,
            json!({
                "id": report.recipe_id,
                "name": name,
                "version": report.version,
                "version_id": report.version_id,
                "warnings": to_json(&report.warnings)?,
            }),
        );
    } else {
        print_ok(
            false,
            json!({ "id": report.recipe_id, "name": name, "version": report.version }),
        );
        warn_stderr(&report.warnings);
    }
    Ok(())
}

/// Text-mode side warnings: one `warning: <code> <origin>: <message>` line each.
fn warn_stderr(warnings: &[LintWarning]) {
    for w in warnings {
        eprintln!("warning: {w}");
    }
}

/// The text outline: SOURCE, its COPY blocks (key, src -> dest, chain), RUN.
fn print_outline(recipe: &Recipe) {
    let names = |chain: &[recipe::TransformRef]| {
        chain
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    for instruction in &recipe.instructions {
        match instruction {
            recipe::Instruction::Source(source) => {
                println!("SOURCE {}", source.repo_name);
                for copy in &source.copies {
                    let src = if copy.src.is_empty() { "." } else { &copy.src };
                    println!(
                        "  COPY {}  {src} -> {}  [{}]",
                        copy.key,
                        copy.dest,
                        names(&copy.forward_chain)
                    );
                }
            }
            recipe::Instruction::Run(run) => {
                let scope = if run.scope.is_empty() {
                    String::new()
                } else {
                    format!("  --only {}", run.scope.join(" "))
                };
                println!("RUN {}{scope}", run.transform.name);
            }
        }
    }
}

fn read_file(path: &Path) -> Result<String, CliError> {
    std::fs::read_to_string(path)
        .map_err(|e| CliError::new("io", format!("cannot read {}: {e}", path.display())))
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<Value, CliError> {
    serde_json::to_value(value).map_err(|e| CliError::new("database", e.to_string()))
}

fn db_err(e: rusqlite::Error) -> CliError {
    CliError::from(db::error::Error::from(e))
}

fn columns(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}
