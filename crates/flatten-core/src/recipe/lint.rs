// crates/flatten-core/src/recipe/lint.rs
//
// Recipe warnings: codes, the warning type, and their order.
//
// Warnings never stop a save. The resolver emits two (L006, L007) while it
// expands INVOKE; the lint pass (plan chunk C6) adds the rest. `analyze`
// returns them merged and sorted: the root recipe first, then invoked
// recipes in `invoked_versions` order, then by position, then by code.
// Display goes through the same `fmt_origin` as error locations.

use std::fmt;

use serde::Serialize;

use super::error::fmt_origin;
use super::types::{Position, Recipe};

/// A warning code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum LintCode {
    /// An invoked recipe's DEPTH_TOLERANCE was ignored (ADR-041).
    L006,
    /// A caller's WATCH OVERRIDE replaced an invoked recipe's entry for the
    /// same key.
    L007,
}

impl fmt::Display for LintCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// One warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LintWarning {
    /// The code.
    pub code: LintCode,
    /// What the warning means here.
    pub message: String,
    /// Where it points; `recipe_version_id` is set inside invoked recipes.
    pub position: Position,
    /// `name@version` when the position is inside an invoked recipe; `None`
    /// for the root.
    pub recipe: Option<String>,
}

impl fmt::Display for LintWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ", self.code)?;
        fmt_origin(
            f,
            self.recipe.as_deref(),
            self.position.line,
            self.position.col,
        )?;
        write!(f, ": {}", self.message)
    }
}

/// Sort warnings: root first, then invoked recipes in `invoked_versions`
/// order, then by line, column, and code.
pub(crate) fn sort_warnings(warnings: &mut [LintWarning], recipe: &Recipe) {
    let origin = |w: &LintWarning| match w.position.recipe_version_id {
        None => 0,
        Some(id) => recipe
            .invoked_versions
            .iter()
            .position(|v| v.version_id == id)
            .map_or(usize::MAX, |i| i + 1),
    };
    warnings.sort_by_key(|w| (origin(w), w.position.line, w.position.col, w.code));
}
