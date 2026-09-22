// crates/flatten-core/src/recipe/parse/ast.rs
//
// Syntax tree: the grammar's output and resolve's input.
//
// Pre-substitution. Words still hold `${NAME}` references; quoting has been
// consumed. Item order is file order.

use crate::recipe::types::Position;

/// A parsed recipe.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Ast {
    /// Top-level instructions in file order.
    pub items: Vec<Item>,
}

/// A top-level instruction.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Item {
    /// `ARG name[=default]`.
    Arg {
        name: String,
        default: Option<Word>,
        pos: Position,
    },
    /// `COPY_DEFAULT_WITH <chain>`.
    CopyDefaultWith {
        chain: Vec<ChainElem>,
        pos: Position,
    },
    /// `SOURCE <repo>:` with its COPY blocks.
    Source {
        repo: Word,
        copies: Vec<CopyAst>,
        pos: Position,
    },
    /// `RUN <transform>[@N] [flags] [--only <glob> ...]`.
    Run {
        name: String,
        pin: Option<u32>,
        flags: Vec<Flag>,
        only: Vec<Word>,
        pos: Position,
    },
}

/// One element of a transform chain: `name --flag value ...`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChainElem {
    pub name: String,
    pub flags: Vec<Flag>,
    /// Position of the transform name.
    pub pos: Position,
}

/// A `--name value` flag. A bare `--name` has the value `true`.
// SPEC-DEVIATION(EX-001): the spec's Flag also has `pos`. Nothing reads it
// (errors point at the element or the value word, which has its own pos),
// so it is left out per plan rule 1.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Flag {
    pub name: String,
    pub value: Word,
}

/// `COPY <src> <dest> AS <key>` inside a SOURCE.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CopyAst {
    pub src: Word,
    pub dest: Word,
    pub key: Word,
    pub pos: Position,
}

/// A token awaiting substitution.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Word {
    /// Literal text and variables, in order; adjacent literals are merged.
    pub segments: Vec<Segment>,
    /// Position of the token.
    pub pos: Position,
}

/// One piece of a Word.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Segment {
    /// Literal text (bare or quoted in the source).
    Lit(String),
    /// A `${NAME}` reference.
    Var { name: String, pos: Position },
}

impl Word {
    /// The word as written, with variables shown as `${NAME}`.
    pub fn display(&self) -> String {
        let mut out = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Lit(text) => out.push_str(text),
                Segment::Var { name, .. } => {
                    out.push_str("${");
                    out.push_str(name);
                    out.push('}');
                }
            }
        }
        out
    }
}
