// crates/flatten-core/src/recipe/mod.rs
//
// Recipe language (EX-001): parse and resolve build recipe text.
//
// `analyze` is the only public entry to the pipeline:
//   source text -> lexer -> grammar -> Ast -> resolve -> Recipe
// Everything under `parse/` is crate-private. The output types in
// `types.rs` are the parse output contract (docs/design/3_RECIPES.md).
//
// Implemented so far (plan chunk C1): ARG, SOURCE, and COPY blocks,
// path normalization, the canonical COPY shape, and Open/Bound ARG modes.

mod error;
mod parse;
mod types;

pub use error::{Error, Location, ParseErrorKind, PathIssue, Result, SourceRef};
pub use parse::resolve::{ArgInput, Resolution};
pub use types::{Arg, CopyBlock, Instruction, Position, Recipe, SourceInstruction};

/// Parse and resolve recipe text.
///
/// `ArgInput::Open` validates at save time: required ARGs without a value
/// stay symbolic (`${name}` survives in output strings), and ARG defaults
/// are checked as final values. `ArgInput::Bound` resolves with concrete
/// values, as export does.
pub fn analyze(source: &str, input: &ArgInput) -> Result<Resolution> {
    let ast = parse::parse(source)?;
    let recipe = parse::resolve::resolve(&ast, input)?;
    Ok(Resolution { recipe })
}
