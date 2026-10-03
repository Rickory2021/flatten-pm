// crates/flatten-core/src/recipe/error.rs
//
// Error types for the recipe module.
//
// Every parse error carries a Location (line and column). Display forms
// are load-bearing: the CLI prints them, and `line:col:` is what the EX-001
// verification checks. All location text goes through `fmt_origin` so error
// and (later) warning lines cannot drift apart.
// See: Error Model in docs/design/1_INFRASTRUCTURE.md.

use std::fmt;

use super::catalog::TransformScope;
use super::types::Position;

/// Recipe operation errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A lexical, structural, or semantic error at a source location.
    // SPEC-DEVIATION(EX-001): the spec has `location: Location`. It is boxed
    // because Location grew INVOKE context (`via`, invoked names) and the
    // unboxed error passed clippy's result_large_err limit (128 bytes). Field
    // access (`location.line`) reads the same through the box.
    #[error("{location}: {kind}")]
    Parse {
        /// Where the error points.
        location: Box<Location>,
        /// What went wrong.
        kind: ParseErrorKind,
    },

    /// Bound mode was given a value for an ARG the recipe never declares.
    #[error("unknown ARG supplied: {name}")]
    UnknownArg {
        /// The undeclared ARG name.
        name: String,
    },
}

/// Convenience alias for recipe operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Build a parse error at a root-recipe position.
pub(crate) fn parse_err(pos: Position, kind: ParseErrorKind) -> Error {
    Error::Parse {
        location: Box::new(Location::root(pos)),
        kind,
    }
}

/// Where an error points: the innermost recipe containing the offending
/// token, and the chain of INVOKE lines that led there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// 1-based line.
    pub line: u32,
    /// 1-based column, in characters.
    pub col: u32,
    /// Which recipe text the line and column refer to.
    pub source: SourceRef,
    /// INVOKE sites from the root down to `source`; empty when `source` is Root.
    pub via: Vec<InvokeSite>,
}

/// Which recipe text a location refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRef {
    /// The recipe being analyzed.
    Root,
    /// A recipe expanded through INVOKE.
    Invoked {
        /// The invoked recipe's name.
        name: String,
        /// Its version number.
        version: u32,
    },
}

/// One INVOKE line on the way to an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvokeSite {
    /// The recipe that contains the INVOKE line: the root's name (or
    /// `<input>` for unsaved text), or an invoked recipe's name.
    pub recipe: String,
    /// The containing recipe's version; `None` for the root.
    pub version: Option<u32>,
    /// 1-based line of the INVOKE.
    pub line: u32,
    /// 1-based column of the INVOKE.
    pub col: u32,
}

impl Location {
    /// A location in the root recipe.
    pub(crate) fn root(pos: Position) -> Self {
        Location {
            line: pos.line,
            col: pos.col,
            source: SourceRef::Root,
            via: Vec::new(),
        }
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.source {
            SourceRef::Root => fmt_origin(f, None, self.line, self.col),
            SourceRef::Invoked { name, version } => {
                fmt_origin(f, Some(&format!("{name}@{version}")), self.line, self.col)?;
                if !self.via.is_empty() {
                    f.write_str(" (via INVOKE at ")?;
                    for (i, site) in self.via.iter().enumerate() {
                        if i > 0 {
                            f.write_str(" -> ")?;
                        }
                        write!(f, "{site}")?;
                    }
                    f.write_str(")")?;
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for InvokeSite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let recipe = match self.version {
            Some(version) => format!("{}@{version}", self.recipe),
            None => self.recipe.clone(),
        };
        fmt_origin(f, Some(&recipe), self.line, self.col)
    }
}

/// Format a source origin as `line:col` (root) or `recipe line:col`
/// (invoked). The one formatter for every location string the recipe module
/// prints.
pub(crate) fn fmt_origin(
    f: &mut fmt::Formatter<'_>,
    recipe: Option<&str>,
    line: u32,
    col: u32,
) -> fmt::Result {
    match recipe {
        Some(recipe) => write!(f, "{recipe} {line}:{col}"),
        None => write!(f, "{line}:{col}"),
    }
}

/// What a path failed on. See the path normalization table in the EX-001 spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PathIssue {
    /// The path is empty after substitution.
    #[error("path is empty")]
    Empty,
    /// The path contains a Unicode control character (including NUL, newline, tab).
    #[error("path contains a control character")]
    ControlChar,
    /// The path contains a backslash.
    #[error("use '/' as the path separator, not '\\'")]
    Backslash,
    /// The path is absolute (leading `/` or a drive letter).
    #[error("path must be relative")]
    Absolute,
    /// The path has a `..` segment.
    #[error("'..' segments are not allowed")]
    ParentSegment,
}

/// Parse error details. Each variant says what went wrong; the enclosing
/// `Error::Parse` says where.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ParseErrorKind {
    // -- lexical ---------------------------------------------------------
    /// A tab in a line's leading whitespace.
    #[error("tab in indentation (indent with spaces)")]
    TabIndent,
    /// A quoted string with no closing quote on its line.
    #[error("unterminated quoted string")]
    UnterminatedQuote,
    /// An escape other than `\"`, `\\`, `\n`, `\t` inside quotes.
    #[error("invalid escape '\\{0}' in quoted string (allowed: \\\" \\\\ \\n \\t)")]
    InvalidEscape(char),
    /// A malformed `${...}` reference.
    #[error("malformed variable reference {found:?}: expected ${{NAME}}")]
    BadVariable {
        /// The text as far as the lexer read it.
        found: String,
    },
    /// A trailing `\` with no following content line.
    #[error("line continuation '\\' is not followed by a line")]
    DanglingContinuation,

    // -- structure -------------------------------------------------------
    /// A line whose first token is not an instruction keyword.
    #[error("unknown instruction '{0}'")]
    UnknownInstruction(String),
    /// A COPY line outside any SOURCE block.
    #[error("COPY must be inside a SOURCE block")]
    CopyOutsideSource,
    /// A COPY-member or WATCH-member instruction outside its block.
    #[error("{instr} must be inside a {expected} block")]
    OutsideBlock {
        /// The misplaced instruction.
        instr: &'static str,
        /// The block it belongs in.
        expected: &'static str,
    },
    /// A known instruction nested under a block that does not allow it.
    #[error("{instr} is not allowed inside {parent}")]
    NotAllowedIn {
        /// The misplaced instruction (always a known keyword).
        instr: &'static str,
        /// The enclosing block.
        parent: &'static str,
    },
    /// A line indented deeper than its context allows.
    #[error("unexpected indentation")]
    UnexpectedIndent,
    /// A line whose indentation matches no enclosing block level.
    #[error("indentation does not match the other lines in this block")]
    InconsistentIndent,
    /// A block instruction with indented children but no trailing `:`.
    #[error("{instr} has an indented block but no trailing ':'")]
    MissingColon {
        /// The block instruction.
        instr: &'static str,
    },
    /// A SOURCE block with no COPY blocks.
    #[error("SOURCE block must contain at least one COPY")]
    EmptySource,
    /// An instruction with the wrong shape of arguments.
    #[error("malformed {instr}: expected `{expected}`")]
    Syntax {
        /// The instruction.
        instr: &'static str,
        /// The accepted form.
        expected: &'static str,
    },
    /// A repeated member: a flag within one transform, a second
    /// OVERRIDE_WITH in a COPY, a repeated INVOKE assignment, a second
    /// WATCH, DEPTH_TOLERANCE, or OVERRIDE, or a repeated OVERRIDE key.
    #[error("duplicate {what}")]
    Duplicate {
        /// What repeated, e.g. `flag --format`.
        what: String,
    },
    /// A bare `--` token an instruction does not recognize. Quote it to
    /// mean literal text (`EXCLUDE "--x"`).
    #[error("unknown {instr} flag {flag} (quote it to use it as a pattern)")]
    UnknownFlag {
        /// The instruction.
        instr: &'static str,
        /// The flag as written.
        flag: String,
    },
    /// An `@N` version pin on a COPY chain element.
    #[error("version pins (@N) are only allowed on INVOKE and RUN")]
    PinNotAllowed,
    /// A number that does not parse, or is out of range.
    #[error("invalid {what}: expected a decimal number")]
    InvalidNumber {
        /// Which number, with its bound when it has one: `version pin (>= 1)`
        /// or `DEPTH_TOLERANCE`.
        what: &'static str,
    },
    /// An ARG name outside `[A-Za-z_][A-Za-z0-9_]*`.
    #[error("invalid ARG name {name:?}: expected [A-Za-z_][A-Za-z0-9_]*")]
    InvalidArgName {
        /// The name as written.
        name: String,
    },

    // -- semantic --------------------------------------------------------
    /// The same ARG declared twice in one recipe.
    #[error("ARG {name} is declared twice")]
    DuplicateArg {
        /// The ARG name.
        name: String,
    },
    /// `${NAME}` used before `ARG NAME` is declared.
    #[error("${{{name}}} is not a declared ARG at this point")]
    UndeclaredArg {
        /// The variable name.
        name: String,
    },
    /// Bound mode: a required ARG has no value.
    #[error("required ARG {name} has no value")]
    MissingRequiredArg {
        /// The ARG name.
        name: String,
    },
    /// Two COPY blocks with the same key after substitution.
    #[error("duplicate COPY key {key:?} (first defined at {first})")]
    DuplicateKey {
        /// The colliding key.
        key: String,
        /// Where the key was first defined.
        first: Location,
    },
    /// A COPY key that is empty or contains a control character.
    #[error("invalid COPY key {key:?}: {reason}")]
    InvalidKey {
        /// The key after substitution.
        key: String,
        /// Why it was rejected.
        reason: &'static str,
    },
    /// A SOURCE repo name that is empty after substitution.
    #[error("SOURCE repo name is empty")]
    InvalidRepoName,
    /// An EXCLUDE pattern the gitignore matcher rejects.
    #[error("invalid EXCLUDE pattern {pattern:?}: {reason}")]
    InvalidPattern {
        /// The pattern after substitution.
        pattern: String,
        /// The matcher's error text.
        reason: String,
    },
    /// A transform name the catalog does not know.
    #[error("unknown transform {name}")]
    UnknownTransform {
        /// The transform name.
        name: String,
    },
    /// A pinned transform version that does not exist.
    #[error("transform {name} has no version {version}")]
    TransformVersionNotFound {
        /// The transform name.
        name: String,
        /// The pinned version.
        version: u32,
    },
    /// A transform used where the other scope is required.
    #[error("{name} is a {found} transform; a {expected} transform is required here")]
    WrongScope {
        /// The transform name.
        name: String,
        /// The scope this position requires.
        expected: TransformScope,
        /// The transform's scope.
        found: TransformScope,
    },
    /// A WATCH OVERRIDE entry for a key no COPY block defines.
    #[error("WATCH OVERRIDE names {key:?}, but no COPY block has that key")]
    UnknownOverrideKey {
        /// The key after substitution.
        key: String,
    },
    /// An INVOKE target the catalog does not know.
    #[error("unknown recipe {name}")]
    UnknownRecipe {
        /// The recipe name.
        name: String,
    },
    /// A pinned recipe version that does not exist.
    #[error("recipe {name} has no version {version}")]
    RecipeVersionNotFound {
        /// The recipe name.
        name: String,
        /// The pinned version.
        version: u32,
    },
    /// A required ARG of an invoked recipe with no value from the INVOKE
    /// line, the caller's ARGs, or a default.
    #[error(
        "INVOKE {invoked}: required ARG {arg} (declared at {}:{}) has no value; {caller} neither passes {arg}= nor declares ARG {arg}",
        .arg_pos.line,
        .arg_pos.col
    )]
    UnboundInvokedArg {
        /// The ARG name.
        arg: String,
        /// The invoked recipe, `name@version`.
        invoked: String,
        /// The calling recipe.
        caller: String,
        /// Where the invoked recipe declares the ARG.
        arg_pos: Position,
    },
    /// An INVOKE assignment for an ARG the invoked recipe never declares.
    #[error("INVOKE {invoked}: the recipe declares no ARG {arg}")]
    UnknownInvokeArg {
        /// The assigned name.
        arg: String,
        /// The invoked recipe, `name@version`.
        invoked: String,
    },
    /// An INVOKE that reaches a recipe version already on the expansion path.
    #[error("INVOKE cycle: {}", .chain.join(" -> "))]
    InvokeCycle {
        /// The expansion path, root first, ending with the repeated version.
        chain: Vec<String>,
    },
    /// INVOKE nesting deeper than 100 levels.
    #[error("INVOKE nesting is deeper than 100 levels")]
    InvokeDepthExceeded,
    /// More than 10,000 SOURCE, COPY, and RUN instructions after expansion.
    #[error("the recipe expands to more than 10000 instructions")]
    ExpansionTooLarge,
    /// A COPY src or dest that fails path normalization.
    #[error("invalid path {path:?}: {issue}")]
    InvalidPath {
        /// The path after substitution.
        path: String,
        /// What it failed on.
        issue: PathIssue,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::lint::{LintCode, LintWarning};

    /// Test 101 (C1, C4, and C5 rows): errors, locations, and warnings share one origin format.
    #[test]
    fn error_warning_and_location_display_formats() {
        let err = parse_err(
            Position::new(3, 5),
            ParseErrorKind::UnknownInstruction("COPIE".into()),
        );
        assert_eq!(
            err.to_string(),
            "3:5: unknown instruction 'COPIE'",
            "root parse error must start with line:col"
        );

        let loc = Location::root(Position::new(12, 1));
        assert_eq!(
            loc.to_string(),
            "12:1",
            "root location displays as line:col"
        );

        let dup = parse_err(
            Position::new(4, 20),
            ParseErrorKind::DuplicateKey {
                key: "k".into(),
                first: Location::root(Position::new(3, 20)),
            },
        );
        assert_eq!(
            dup.to_string(),
            "4:20: duplicate COPY key \"k\" (first defined at 3:20)",
            "nested location in a message uses the same formatter"
        );

        let once = Location {
            line: 3,
            col: 5,
            source: SourceRef::Invoked {
                name: "base".into(),
                version: 2,
            },
            via: vec![InvokeSite {
                recipe: "invoke".into(),
                version: None,
                line: 7,
                col: 1,
            }],
        };
        assert_eq!(
            once.to_string(),
            "base@2 3:5 (via INVOKE at invoke 7:1)",
            "invoked location names the recipe and the INVOKE site"
        );
        let nested = Location {
            line: 2,
            col: 1,
            source: SourceRef::Invoked {
                name: "leaf".into(),
                version: 1,
            },
            via: vec![
                InvokeSite {
                    recipe: "invoke".into(),
                    version: None,
                    line: 7,
                    col: 1,
                },
                InvokeSite {
                    recipe: "base".into(),
                    version: Some(2),
                    line: 4,
                    col: 1,
                },
            ],
        };
        assert_eq!(
            nested.to_string(),
            "leaf@1 2:1 (via INVOKE at invoke 7:1 -> base@2 4:1)",
            "nested via lists every INVOKE root first"
        );
        let invoked_err = Error::Parse {
            location: Box::new(nested),
            kind: ParseErrorKind::UnknownInstruction("COPIE".into()),
        };
        assert_eq!(
            invoked_err.to_string(),
            "leaf@1 2:1 (via INVOKE at invoke 7:1 -> base@2 4:1): unknown instruction 'COPIE'",
            "an invoked error line starts with its location"
        );

        let root_warning = LintWarning {
            code: LintCode::L007,
            message: "replaced".into(),
            position: Position::new(3, 3),
            recipe: None,
        };
        assert_eq!(
            root_warning.to_string(),
            "L007 3:3: replaced",
            "a root warning displays as code line:col: message"
        );
        let invoked_warning = LintWarning {
            code: LintCode::L006,
            message: "ignored".into(),
            position: Position {
                line: 3,
                col: 1,
                recipe_version_id: Some(12),
            },
            recipe: Some("base@2".into()),
        };
        assert_eq!(
            invoked_warning.to_string(),
            "L006 base@2 3:1: ignored",
            "an invoked warning names its recipe through the same formatter"
        );

        let unknown = Error::UnknownArg { name: "zz".into() };
        assert_eq!(
            unknown.to_string(),
            "unknown ARG supplied: zz",
            "non-parse errors carry no location"
        );
    }
}
