// crates/flatten-core/src/recipe/parse/mod.rs
//
// The recipe pipeline internals: lexer -> grammar -> Ast -> resolve.
// Crate-private; the public entry is `recipe::analyze`.

mod ast;
mod grammar;
mod lexer;
mod path;
pub(crate) mod resolve;

pub(crate) use path::is_name;

use crate::recipe::error::Result;
use crate::recipe::types::Arg;

/// Parse recipe text into an Ast (syntax and structure only; no ARG values).
pub(crate) fn parse(source: &str) -> Result<ast::Ast> {
    grammar::build(lexer::logical_lines(source)?)
}

/// The root recipe's ARG declarations, from parsing alone (no resolution,
/// no catalog). Defaults are shown as written.
pub(crate) fn declared_args(source: &str) -> Result<Vec<Arg>> {
    Ok(parse(source)?
        .items
        .into_iter()
        .filter_map(|item| match item {
            ast::Item::Arg { name, default, pos } => Some(Arg {
                required: default.is_none(),
                default: default.as_ref().map(ast::Word::display),
                name,
                position: pos,
            }),
            _ => None,
        })
        .collect())
}
