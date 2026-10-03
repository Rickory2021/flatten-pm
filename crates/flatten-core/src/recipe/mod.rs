// crates/flatten-core/src/recipe/mod.rs
//
// Recipe language (EX-001): parse and resolve build recipe text.
//
// `analyze` is the only public entry to the pipeline:
//   source text -> lexer -> grammar -> Ast -> resolve -> Recipe
// Everything under `parse/` is crate-private. The output types in
// `types.rs` are the parse output contract (docs/design/3_RECIPES.md).
// Transform lookups go through the `Catalog` trait (`catalog.rs`).
//
// Implemented so far (plan chunk C2): ARG, COPY_DEFAULT_WITH, SOURCE, COPY,
// and RUN; transform chains; path normalization; the canonical COPY shape;
// Open/Bound ARG modes.

mod catalog;
mod error;
mod parse;
mod types;

pub use catalog::{Catalog, TransformInfo, TransformScope};
pub use error::{Error, Location, ParseErrorKind, PathIssue, Result, SourceRef};
pub use parse::resolve::{ArgInput, Resolution};
pub use types::{
    Arg, CopyBlock, Instruction, Position, Recipe, RunInstruction, SourceInstruction, TransformRef,
};

/// The shipped generic recipe (`shipped-default`), embedded from
/// `builtins/recipes/`. The single source for the seed and `recipe new`.
/// Holds the exact text from docs/design/6_VERSIONING.md.
pub const SHIPPED_DEFAULT_RECIPE: &str = strip_prefix_const(
    include_str!("../../builtins/recipes/shipped-default.recipe"),
    SHIPPED_HEADER,
);

// SPEC-DEVIATION(EX-001): the spec says the file holds exactly the shipped
// text. It also carries a directory comment on line 1 so the flatten-sync
// watcher can route it; the comment is stripped here, so the embedded text
// (and every `recipe new` recipe) is still exactly the spec's text.
/// The file's directory comment, stripped from the embedded text.
const SHIPPED_HEADER: &str = "# crates/flatten-core/builtins/recipes/shipped-default.recipe\n";

// Header drift (a missing or edited header line, or a CRLF checkout) would
// leave the comment in the embedded text. Make that a compile error instead
// of a silent change to every `recipe new` recipe.
const _: () = assert!(
    starts_with_const(SHIPPED_DEFAULT_RECIPE, "ARG repo\n"),
    "shipped-default.recipe: the embedded text must start with `ARG repo` (check the line-1 header and LF line endings)"
);

/// `str::starts_with` for const contexts.
const fn starts_with_const(text: &str, prefix: &str) -> bool {
    let (text_bytes, prefix_bytes) = (text.as_bytes(), prefix.as_bytes());
    if text_bytes.len() < prefix_bytes.len() {
        return false;
    }
    let mut i = 0;
    while i < prefix_bytes.len() {
        if text_bytes[i] != prefix_bytes[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// `str::strip_prefix` for const contexts: `text` without a leading
/// `prefix`, or `text` unchanged when it does not start with `prefix`.
const fn strip_prefix_const<'a>(text: &'a str, prefix: &str) -> &'a str {
    if !starts_with_const(text, prefix) {
        return text;
    }
    let (_, rest) = text.as_bytes().split_at(prefix.len());
    match std::str::from_utf8(rest) {
        Ok(rest) => rest,
        Err(_) => text,
    }
}

/// Parse and resolve recipe text.
///
/// `ArgInput::Open` validates at save time: required ARGs without a value
/// stay symbolic (`${name}` survives in output strings), and ARG defaults
/// are checked as final values. `ArgInput::Bound` resolves with concrete
/// values, as export does. Transform names resolve through `catalog`.
pub fn analyze(source: &str, input: &ArgInput, catalog: &dyn Catalog) -> Result<Resolution> {
    let ast = parse::parse(source)?;
    let recipe = parse::resolve::resolve(&ast, input, catalog)?;
    Ok(Resolution { recipe })
}
