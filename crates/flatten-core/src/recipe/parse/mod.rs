// crates/flatten-core/src/recipe/parse/mod.rs
//
// The recipe pipeline internals: lexer -> grammar -> Ast -> resolve.
// Crate-private; the public entry is `recipe::analyze`.

mod ast;
mod grammar;
mod lexer;
mod path;
pub(crate) mod resolve;

use crate::recipe::error::Result;

/// Parse recipe text into an Ast (syntax and structure only; no ARG values).
pub(crate) fn parse(source: &str) -> Result<ast::Ast> {
    grammar::build(lexer::logical_lines(source)?)
}
