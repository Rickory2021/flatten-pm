// crates/flatten-core/src/recipe/parse/grammar.rs
//
// Grammar: logical lines -> indentation tree -> Ast.
//
// Step 1 builds a tree from indentation alone (relative, no fixed width).
// Step 2 walks it by context and checks each instruction's shape.
// Only SOURCE and COPY consume a trailing `:` (the block colon, F-17). The
// colon is optional on a childless block and required when children follow.
//
// Implemented so far (plan chunk C1): ARG, SOURCE, COPY. Any other first
// token is an unknown instruction until its chunk lands.

use super::ast::{Ast, CopyAst, Item, Segment, Word};
use super::lexer::{LogicalLine, RawSegment, Token};
use super::path::is_arg_name;
use crate::recipe::error::{Error, ParseErrorKind, Result, parse_err};

/// Instruction keywords implemented so far.
const KEYWORDS: &[&str] = &["ARG", "SOURCE", "COPY"];
/// Keywords that open a block and consume a trailing `:`.
const BLOCK_KEYWORDS: &[&str] = &["SOURCE", "COPY"];

/// Build the Ast from logical lines.
pub(crate) fn build(lines: Vec<LogicalLine>) -> Result<Ast> {
    let mut items = Vec::new();
    for node in tree(lines)? {
        items.push(top_level(node)?);
    }
    Ok(Ast { items })
}

/// A logical line and the lines indented under it.
struct Node {
    line: LogicalLine,
    children: Vec<Node>,
    child_indent: Option<u32>,
}

/// Build the indentation tree.
fn tree(lines: Vec<LogicalLine>) -> Result<Vec<Node>> {
    let mut roots = Vec::new();
    let mut stack: Vec<Node> = Vec::new();

    for line in lines {
        while stack
            .last()
            .is_some_and(|top| top.line.indent >= line.indent)
        {
            close(&mut stack, &mut roots);
        }
        match stack.last_mut() {
            None if line.indent != 0 => {
                return Err(parse_err(line.pos, ParseErrorKind::UnexpectedIndent));
            }
            None => {}
            Some(parent) => match parent.child_indent {
                Some(expected) if expected != line.indent => {
                    return Err(parse_err(line.pos, ParseErrorKind::InconsistentIndent));
                }
                Some(_) => {}
                None => parent.child_indent = Some(line.indent),
            },
        }
        stack.push(Node {
            line,
            children: Vec::new(),
            child_indent: None,
        });
    }
    while !stack.is_empty() {
        close(&mut stack, &mut roots);
    }
    Ok(roots)
}

/// Pop the innermost open node and attach it to its parent (or the roots).
fn close(stack: &mut Vec<Node>, roots: &mut Vec<Node>) {
    if let Some(node) = stack.pop() {
        match stack.last_mut() {
            Some(parent) => parent.children.push(node),
            None => roots.push(node),
        }
    }
}

/// Recognize the first token as a keyword. Returns the keyword and whether
/// a block colon was attached to it (`SOURCE:`).
fn keyword_of(token: &Token) -> Option<(&'static str, bool)> {
    let text = token.bare_text()?;
    if let Some(keyword) = KEYWORDS.iter().find(|k| **k == text) {
        return Some((keyword, false));
    }
    let stripped = text.strip_suffix(':')?;
    BLOCK_KEYWORDS
        .iter()
        .find(|k| **k == stripped)
        .map(|keyword| (*keyword, true))
}

fn unknown(token: &Token) -> Error {
    parse_err(
        token.pos,
        ParseErrorKind::UnknownInstruction(token.display()),
    )
}

/// A known keyword in the wrong block, or an unknown instruction.
fn misplaced(child: &Node, parent: &'static str) -> Error {
    match keyword_of(&child.line.first) {
        Some((keyword, _)) => parse_err(
            child.line.pos,
            ParseErrorKind::NotAllowedIn {
                instr: keyword.to_string(),
                parent,
            },
        ),
        None => unknown(&child.line.first),
    }
}

fn top_level(node: Node) -> Result<Item> {
    match keyword_of(&node.line.first) {
        Some(("ARG", _)) => {
            no_children(&node)?;
            arg(&node.line)
        }
        Some(("SOURCE", colon)) => source(node, colon),
        Some(("COPY", _)) => Err(parse_err(node.line.pos, ParseErrorKind::CopyOutsideSource)),
        _ => Err(unknown(&node.line.first)),
    }
}

/// A leaf instruction must not have indented children.
fn no_children(node: &Node) -> Result<()> {
    match node.children.first() {
        Some(child) => Err(parse_err(child.line.pos, ParseErrorKind::UnexpectedIndent)),
        None => Ok(()),
    }
}

/// Read a block line's arguments and its block colon. The colon is either
/// the last bare character of the last token, a standalone `:` token, or
/// attached to the keyword when nothing follows it (`SOURCE:`, then an arity
/// error). A colon on the keyword with arguments after it (`SOURCE: r`) is
/// not a block colon, so the line is malformed.
fn block_args(
    line: &LogicalLine,
    colon_on_keyword: bool,
    malformed: ParseErrorKind,
) -> Result<(Vec<Token>, bool)> {
    let mut args = line.words()?;
    if colon_on_keyword {
        if !args.is_empty() {
            return Err(parse_err(line.pos, malformed));
        }
        return Ok((args, true));
    }
    let colon = strip_block_colon(&mut args);
    Ok((args, colon))
}

fn strip_block_colon(args: &mut Vec<Token>) -> bool {
    let Some(last) = args.last_mut() else {
        return false;
    };
    if last.bare_text() == Some(":") {
        args.pop();
        return true;
    }
    let Some(RawSegment::Bare(text)) = last.segments.last_mut() else {
        return false;
    };
    if !text.ends_with(':') {
        return false;
    }
    text.pop();
    if text.is_empty() {
        last.segments.pop();
    }
    true
}

/// Children require the block colon.
fn require_colon(node: &Node, colon: bool, instr: &'static str) -> Result<()> {
    if !node.children.is_empty() && !colon {
        return Err(parse_err(
            node.line.pos,
            ParseErrorKind::MissingColon { instr },
        ));
    }
    Ok(())
}

fn arg(line: &LogicalLine) -> Result<Item> {
    let args = line.words()?;
    let [token] = args.as_slice() else {
        return Err(parse_err(
            line.pos,
            ParseErrorKind::Syntax {
                instr: "ARG",
                expected: "ARG <name>[=<default>]",
            },
        ));
    };

    let (name_part, default) = match split_assign(token) {
        Some((name_part, value_part)) => (name_part, Some(to_word(&value_part, token))),
        None => (token.segments.clone(), None),
    };
    let name = match name_part.as_slice() {
        [] => String::new(),
        [RawSegment::Bare(text)] => text.clone(),
        _ => {
            let shown = Token {
                segments: name_part.clone(),
                pos: token.pos,
            };
            return Err(parse_err(
                token.pos,
                ParseErrorKind::InvalidArgName {
                    name: shown.display(),
                },
            ));
        }
    };
    if !is_arg_name(&name) {
        return Err(parse_err(
            token.pos,
            ParseErrorKind::InvalidArgName { name },
        ));
    }
    Ok(Item::Arg {
        name,
        default,
        pos: line.pos,
    })
}

/// Split a token at its first `=` inside a bare segment.
fn split_assign(token: &Token) -> Option<(Vec<RawSegment>, Vec<RawSegment>)> {
    for (index, segment) in token.segments.iter().enumerate() {
        let RawSegment::Bare(text) = segment else {
            continue;
        };
        let Some(eq) = text.find('=') else {
            continue;
        };
        let mut name = token.segments[..index].to_vec();
        if eq > 0 {
            name.push(RawSegment::Bare(text[..eq].to_string()));
        }
        let mut value = Vec::new();
        if eq + 1 < text.len() {
            value.push(RawSegment::Bare(text[eq + 1..].to_string()));
        }
        value.extend_from_slice(&token.segments[index + 1..]);
        return Some((name, value));
    }
    None
}

const SOURCE_SYNTAX: ParseErrorKind = ParseErrorKind::Syntax {
    instr: "SOURCE",
    expected: "SOURCE <repo>:",
};
const COPY_SYNTAX: ParseErrorKind = ParseErrorKind::Syntax {
    instr: "COPY",
    expected: "COPY <src> <dest> AS <key>",
};

fn source(node: Node, colon_on_keyword: bool) -> Result<Item> {
    let (args, colon) = block_args(&node.line, colon_on_keyword, SOURCE_SYNTAX)?;
    let [repo] = args.as_slice() else {
        return Err(parse_err(node.line.pos, SOURCE_SYNTAX));
    };
    require_colon(&node, colon, "SOURCE")?;
    if node.children.is_empty() {
        return Err(parse_err(node.line.pos, ParseErrorKind::EmptySource));
    }

    let repo = to_word(&repo.segments, repo);
    let pos = node.line.pos;
    let mut copies = Vec::with_capacity(node.children.len());
    for child in node.children {
        match keyword_of(&child.line.first) {
            Some(("COPY", colon)) => copies.push(copy(child, colon)?),
            _ => return Err(misplaced(&child, "SOURCE")),
        }
    }
    Ok(Item::Source { repo, copies, pos })
}

fn copy(node: Node, colon_on_keyword: bool) -> Result<CopyAst> {
    let (args, colon) = block_args(&node.line, colon_on_keyword, COPY_SYNTAX)?;
    let [src, dest, as_keyword, key] = args.as_slice() else {
        return Err(parse_err(node.line.pos, COPY_SYNTAX));
    };
    if as_keyword.bare_text() != Some("AS") {
        return Err(parse_err(node.line.pos, COPY_SYNTAX));
    }
    require_colon(&node, colon, "COPY")?;
    if let Some(child) = node.children.first() {
        return Err(misplaced(child, "COPY"));
    }
    Ok(CopyAst {
        src: to_word(&src.segments, src),
        dest: to_word(&dest.segments, dest),
        key: to_word(&key.segments, key),
        pos: node.line.pos,
    })
}

/// Convert raw segments to a Word: bare and quoted text merge into literals.
fn to_word(segments: &[RawSegment], token: &Token) -> Word {
    let mut out: Vec<Segment> = Vec::new();
    for segment in segments {
        match segment {
            RawSegment::Bare(text) | RawSegment::Quoted(text) => match out.last_mut() {
                Some(Segment::Lit(existing)) => existing.push_str(text),
                _ => out.push(Segment::Lit(text.clone())),
            },
            RawSegment::Var { name, pos } => out.push(Segment::Var {
                name: name.clone(),
                pos: *pos,
            }),
        }
    }
    Word {
        segments: out,
        pos: token.pos,
    }
}

#[cfg(test)]
mod tests {
    use crate::recipe::error::{Error, ParseErrorKind};
    use crate::recipe::parse::ast::{Ast, Item, Segment};
    use crate::recipe::parse::parse;
    use crate::recipe::types::Position;

    fn ast(source: &str) -> Ast {
        parse(source).expect("source should parse")
    }

    /// Parse and return (kind, line, col) of the expected error.
    fn parse_error(source: &str) -> (ParseErrorKind, u32, u32) {
        match parse(source) {
            Err(Error::Parse { location, kind }) => (kind, location.line, location.col),
            other => panic!("expected a parse error for {source:?}, got {other:?}"),
        }
    }

    fn syntax(instr: &'static str, expected: &'static str) -> ParseErrorKind {
        ParseErrorKind::Syntax { instr, expected }
    }

    const COPY_FORM: &str = "COPY <src> <dest> AS <key>";

    /// Test 18: COPY outside a SOURCE block is an error at the COPY.
    #[test]
    fn copy_outside_source_errors() {
        assert_eq!(
            parse_error("COPY . x/ AS k"),
            (ParseErrorKind::CopyOutsideSource, 1, 1),
            "top-level COPY"
        );
        assert_eq!(
            parse_error("SOURCE r:\n  COPY . a/ AS a\nCOPY . b/ AS b"),
            (ParseErrorKind::CopyOutsideSource, 3, 1),
            "COPY after the SOURCE block has closed"
        );
    }

    /// Test 19 (C1 rows): known instructions nested in the wrong block.
    #[test]
    fn misplaced_instructions_error_with_expected_parent() {
        let cases = [
            ("SOURCE r:\n  ARG a", "ARG", "SOURCE", 2, 3),
            (
                "SOURCE r:\n  COPY . x/ AS k:\n    ARG a",
                "ARG",
                "COPY",
                3,
                5,
            ),
            (
                "SOURCE r:\n  COPY . x/ AS k:\n    SOURCE s:",
                "SOURCE",
                "COPY",
                3,
                5,
            ),
        ];
        for (source, instr, parent, line, col) in cases {
            assert_eq!(
                parse_error(source),
                (
                    ParseErrorKind::NotAllowedIn {
                        instr: instr.into(),
                        parent
                    },
                    line,
                    col
                ),
                "misplaced instruction in {source:?}"
            );
        }
    }

    /// Test 20: an unknown first token is an error at its position.
    #[test]
    fn unknown_instruction_errors_with_line_col() {
        let cases = [
            ("COPIE . x/ AS k", "COPIE", 1, 1),
            ("copy . x/ AS k", "copy", 1, 1),
            ("\"ARG\" a", "\"ARG\"", 1, 1),
            ("ARG a\n\nSOURCE r:\n  FOO bar", "FOO", 4, 3),
        ];
        for (source, word, line, col) in cases {
            assert_eq!(
                parse_error(source),
                (ParseErrorKind::UnknownInstruction(word.into()), line, col),
                "unknown instruction in {source:?}"
            );
        }
    }

    /// Test 21 (F-17): a block with children needs its trailing colon.
    #[test]
    fn block_with_children_missing_colon_errors() {
        assert_eq!(
            parse_error("SOURCE r\n  COPY . x/ AS k"),
            (ParseErrorKind::MissingColon { instr: "SOURCE" }, 1, 1),
            "SOURCE with children and no colon"
        );
        assert_eq!(
            parse_error("SOURCE r:\n  COPY . x/ AS k\n    ARG a"),
            (ParseErrorKind::MissingColon { instr: "COPY" }, 2, 3),
            "COPY with children and no colon"
        );
    }

    /// Test 24: indentation errors.
    #[test]
    fn indentation_errors() {
        let cases = [
            ("ARG a\n  ARG b", ParseErrorKind::UnexpectedIndent, 2, 3),
            (
                "SOURCE r:\n    COPY . a/ AS a\n  COPY . b/ AS b",
                ParseErrorKind::InconsistentIndent,
                3,
                3,
            ),
            ("  ARG a", ParseErrorKind::UnexpectedIndent, 1, 3),
        ];
        for (source, kind, line, col) in cases {
            assert_eq!(
                parse_error(source),
                (kind, line, col),
                "indentation error in {source:?}"
            );
        }
    }

    /// Test 25: a SOURCE block with no COPY is an error, colon or not.
    #[test]
    fn empty_source_errors() {
        for source in ["SOURCE r:", "SOURCE r", "SOURCE r :\nARG a"] {
            assert_eq!(
                parse_error(source),
                (ParseErrorKind::EmptySource, 1, 1),
                "empty SOURCE in {source:?}"
            );
        }
    }

    /// Test 26: empty text and ARG-only recipes parse.
    #[test]
    fn empty_and_arg_only_recipes_parse() {
        assert!(ast("").items.is_empty(), "empty text has no items");
        assert!(
            ast("# only a comment\n\n").items.is_empty(),
            "comments only"
        );
        assert_eq!(ast("ARG a\nARG b=1\n").items.len(), 2, "two ARGs");
    }

    /// Test 27: instructions with the wrong argument shape.
    #[test]
    fn instruction_arity_errors() {
        let cases = [
            ("SOURCE r:\n  COPY . x/ k", syntax("COPY", COPY_FORM), 2, 3),
            ("SOURCE r:\n  COPY . x/ AS", syntax("COPY", COPY_FORM), 2, 3),
            (
                "SOURCE r:\n  COPY . x/ AS k extra",
                syntax("COPY", COPY_FORM),
                2,
                3,
            ),
            (
                "SOURCE r:\n  COPY . x/ as k",
                syntax("COPY", COPY_FORM),
                2,
                3,
            ),
            (
                "SOURCE a b:\n  COPY . x/ AS k",
                syntax("SOURCE", "SOURCE <repo>:"),
                1,
                1,
            ),
            (
                "SOURCE:\n  COPY . x/ AS k",
                syntax("SOURCE", "SOURCE <repo>:"),
                1,
                1,
            ),
            (
                "SOURCE: r\n  COPY . x/ AS k",
                syntax("SOURCE", "SOURCE <repo>:"),
                1,
                1,
            ),
            ("ARG a b", syntax("ARG", "ARG <name>[=<default>]"), 1, 1),
            ("ARG", syntax("ARG", "ARG <name>[=<default>]"), 1, 1),
        ];
        for (source, kind, line, col) in cases {
            assert_eq!(
                parse_error(source),
                (kind, line, col),
                "arity error in {source:?}"
            );
        }
    }

    /// Test 37: ARG with and without defaults.
    #[test]
    fn arg_with_and_without_default() {
        let items = ast("ARG a\nARG b=${a}-x\nARG c=\nARG d=\"x y\"\nARG e==x").items;
        let args: Vec<(String, Option<Vec<Segment>>)> = items
            .into_iter()
            .map(|item| match item {
                Item::Arg { name, default, .. } => (name, default.map(|w| w.segments)),
                other => panic!("expected ARG, got {other:?}"),
            })
            .collect();
        assert_eq!(
            args,
            vec![
                ("a".into(), None),
                (
                    "b".into(),
                    Some(vec![
                        Segment::Var {
                            name: "a".into(),
                            pos: Position::new(2, 7)
                        },
                        Segment::Lit("-x".into())
                    ])
                ),
                ("c".into(), Some(vec![])),
                ("d".into(), Some(vec![Segment::Lit("x y".into())])),
                ("e".into(), Some(vec![Segment::Lit("=x".into())])),
            ],
            "ARG names and defaults"
        );
    }

    /// Test 38: ARG names outside the identifier rule are rejected.
    #[test]
    fn arg_invalid_name_errors() {
        let cases = [
            ("ARG my-arg", "my-arg"),
            ("ARG 1x", "1x"),
            ("ARG \"q\"=1", "\"q\""),
            ("ARG =x", ""),
        ];
        for (source, name) in cases {
            assert_eq!(
                parse_error(source),
                (ParseErrorKind::InvalidArgName { name: name.into() }, 1, 5),
                "invalid ARG name in {source:?}"
            );
        }
    }
}
