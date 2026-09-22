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
}

impl Position {
    /// Build a position from a 1-based line and column.
    pub(crate) fn new(line: u32, col: u32) -> Self {
        Position { line, col }
    }
}

/// A resolved recipe.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Recipe {
    /// The root recipe's ARG declarations, in file order.
    pub args: Vec<Arg>,
    /// Instructions in execution (file) order.
    pub instructions: Vec<Instruction>,
    /// Root ARGs left symbolic in Open mode, in file order. Empty in Bound mode.
    pub unbound: Vec<String>,
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
    /// Where the COPY line starts.
    pub position: Position,
}
