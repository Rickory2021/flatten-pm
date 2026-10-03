// crates/flatten-core/src/recipe/types.rs
//
// Resolved recipe types: the parse output contract.
//
// A derived view of the recipe text (ADR-008). Every string here is
// post-substitution; in Open mode an unbound required ARG appears as
// `${name}`. See the Parse Output contract in docs/design/3_RECIPES.md.

use serde::Serialize;

/// A 1-based source position. Columns count Unicode scalar values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Position {
    /// 1-based physical line.
    pub line: u32,
    /// 1-based column, in characters.
    pub col: u32,
    /// `build_recipe_versions.id` of the recipe the position is in, for
    /// instructions expanded from an INVOKE; `None` for the root recipe.
    pub recipe_version_id: Option<i64>,
}

impl Position {
    /// Build a root-recipe position from a 1-based line and column.
    pub(crate) fn new(line: u32, col: u32) -> Self {
        Position {
            line,
            col,
            recipe_version_id: None,
        }
    }
}

/// A resolved recipe.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Recipe {
    /// The root recipe's ARG declarations, in file order.
    pub args: Vec<Arg>,
    /// Instructions in execution (file) order.
    pub instructions: Vec<Instruction>,
    /// The WATCH configuration watch will record for this recipe.
    pub watch_config: WatchConfig,
    /// Every recipe version expanded through INVOKE, nested ones included,
    /// in first-seen order and without duplicates.
    pub invoked_versions: Vec<InvokedVersion>,
    /// Root ARGs left symbolic in Open mode, in file order. Empty in Bound mode.
    pub unbound: Vec<String>,
}

/// The recipe's WATCH block, merged across INVOKE.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WatchConfig {
    /// How far a returned file's path may drift; 2 unless the root's WATCH
    /// sets it. Invoked recipes' values are ignored (ADR-041).
    pub depth_tolerance: u32,
    /// Return-chain overrides by COPY key. The root's entries win over an
    /// invoked recipe's for the same key.
    pub overrides: std::collections::BTreeMap<String, Vec<TransformRef>>,
}

impl Default for WatchConfig {
    fn default() -> Self {
        WatchConfig {
            depth_tolerance: 2,
            overrides: std::collections::BTreeMap::new(),
        }
    }
}

/// A recipe version expanded through INVOKE.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InvokedVersion {
    /// `build_recipe_versions.id`.
    pub version_id: i64,
    /// The invoked recipe's name.
    pub name: String,
    /// The version number.
    pub version: u32,
}

/// One `ARG name[=default]` declaration.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Arg {
    /// The ARG name.
    pub name: String,
    /// The default as written: quotes decoded, variables shown as `${NAME}`.
    pub default: Option<String>,
    /// True when the ARG has no default, so a binding must supply it.
    pub required: bool,
    /// Where the ARG is declared.
    pub position: Position,
}

/// A recipe-level instruction.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Instruction {
    /// A `SOURCE <repo>:` block with its COPY blocks.
    Source(SourceInstruction),
    /// A `RUN <transform>` directory transform.
    Run(RunInstruction),
}

/// A `SOURCE <repo>:` block.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceInstruction {
    /// The repo name after substitution.
    pub repo_name: String,
    /// The COPY blocks inside this SOURCE, in file order.
    pub copies: Vec<CopyBlock>,
    /// Where the SOURCE line starts.
    pub position: Position,
}

/// A `COPY <src> <dest> AS <key>` block.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CopyBlock {
    /// Source prefix: normalized, in canonical COPY shape. Recorded as `src_prefix`.
    pub src: String,
    /// Destination prefix: normalized, in canonical COPY shape. Recorded as `dest_prefix`.
    pub dest: String,
    /// The `AS` key after substitution; unique across the recipe.
    pub key: String,
    /// EXCLUDE entries in file order: gitignore patterns (after
    /// substitution) and the `--binary` marker.
    pub excludes: Vec<Exclude>,
    /// The per-file transform chain: this block's OVERRIDE_WITH when present,
    /// otherwise the recipe's COPY_DEFAULT_WITH in effect at this COPY (empty
    /// when none has been declared).
    pub forward_chain: Vec<TransformRef>,
    /// Where the COPY line starts.
    pub position: Position,
}

/// One EXCLUDE entry.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Exclude {
    /// A gitignore-syntax pattern, after substitution.
    Pattern(String),
    /// `--binary`: exclude files whose extension is in the
    /// `binary_extensions` setting (expanded at export, EX-004).
    Binary,
}

/// A transform at a resolved version, with its arguments.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TransformRef {
    /// The transform name.
    pub name: String,
    /// `transforms.id`.
    pub transform_id: i64,
    /// `transform_versions.id` of the resolved version.
    pub version_id: i64,
    /// The resolved version number (pinned or current).
    pub version: u32,
    /// Flags as given, after substitution. A bare `--flag` is `"true"`.
    /// Defaults from the transform's arg schema are not applied yet
    /// ([DEFERRED: EX-002B]).
    pub args: std::collections::BTreeMap<String, String>,
}

/// A `RUN <transform>[@N] [--flag value ...] [--only <glob> ...]` line.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunInstruction {
    /// The directory transform at its resolved version.
    pub transform: TransformRef,
    /// The `--only` globs after substitution; empty means everything.
    pub scope: Vec<String>,
    /// Where the RUN line starts.
    pub position: Position,
}
