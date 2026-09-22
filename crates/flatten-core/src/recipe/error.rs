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
    #[error("{location}: {kind}")]
    Parse {
        /// Where the error points.
        location: Location,
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
        location: Location::root(pos),
        kind,
    }
}

/// Where an error points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// 1-based line.
    pub line: u32,
    /// 1-based column, in characters.
    pub col: u32,
    /// Which recipe text the line and column refer to.
    pub source: SourceRef,
}

/// Which recipe text a location refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRef {
    /// The recipe being analyzed.
    Root,
}

impl Location {
    /// A location in the root recipe.
    pub(crate) fn root(pos: Position) -> Self {
        Location {
            line: pos.line,
            col: pos.col,
            source: SourceRef::Root,
        }
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.source {
            SourceRef::Root => fmt_origin(f, self.line, self.col),
        }
    }
}

/// Format a source origin as `line:col`. The one formatter for every
/// location string the recipe module prints.
pub(crate) fn fmt_origin(f: &mut fmt::Formatter<'_>, line: u32, col: u32) -> fmt::Result {
    write!(f, "{line}:{col}")
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
    /// A repeated member: a flag within one transform, and (from later
    /// chunks) block members such as a second OVERRIDE_WITH.
    #[error("duplicate {what}")]
    Duplicate {
        /// What repeated, e.g. `flag --format`.
        what: String,
    },
    /// An `@N` version pin on a COPY chain element.
    #[error("version pins (@N) are only allowed on INVOKE and RUN")]
    PinNotAllowed,
    /// A number that does not parse, or is out of range.
    #[error("invalid {what}: expected a decimal number >= 1")]
    InvalidNumber {
        /// Which number, e.g. `version pin`.
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

    /// Test 101 (C1 rows): root errors display as `line:col: message`.
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

        let unknown = Error::UnknownArg { name: "zz".into() };
        assert_eq!(
            unknown.to_string(),
            "unknown ARG supplied: zz",
            "non-parse errors carry no location"
        );
    }
}
