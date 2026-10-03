// crates/flatten-core/src/recipe/parse/grammar.rs
//
// Grammar: logical lines -> indentation tree -> Ast.
//
// Step 1 builds a tree from indentation alone (relative, no fixed width).
// Step 2 walks it by context and checks each instruction's shape.
// Only SOURCE and COPY consume a trailing `:` (the block colon, F-17). The
// colon is optional on a childless block and required when children follow.
//
// Transform chains (`[t1 --flag v, t2]`) are read in chain mode; RUN flags
// in word mode. Flags are `--name value`, `--name=value`, or a bare `--name`
// (value `true`). On RUN, `--only` is reserved and takes every following
// non-flag token as a glob.
//
// COPY blocks take two kinds of members: EXCLUDE (gitignore patterns and the
// `--binary` marker; any other bare `--` token is an unknown flag) and at
// most one OVERRIDE_WITH chain.
//
// INVOKE takes a recipe name, an optional `@N` pin, and `name=value`
// assignments for the invoked recipe's ARGs.
//
// Implemented so far (plan chunk C4): ARG, COPY_DEFAULT_WITH, SOURCE, COPY
// with EXCLUDE and OVERRIDE_WITH, RUN, INVOKE. Any other first token is an
// unknown instruction until its chunk lands.

use std::collections::HashSet;

use super::ast::{Ast, ChainElem, CopyAst, ExcludeAst, Flag, Item, Segment, Word};
use super::lexer::{ChainToken, LogicalLine, RawSegment, Token};
use super::path::{is_arg_name, is_flag_name, is_name};
use crate::recipe::error::{Error, ParseErrorKind, Result, parse_err};
use crate::recipe::types::Position;

/// Instruction keywords implemented so far.
const KEYWORDS: &[&str] = &[
    "ARG",
    "COPY_DEFAULT_WITH",
    "SOURCE",
    "RUN",
    "COPY",
    "EXCLUDE",
    "OVERRIDE_WITH",
    "INVOKE",
];
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
                instr: keyword,
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
        Some(("COPY_DEFAULT_WITH", _)) => {
            no_children(&node)?;
            let chain = chain(&node.line, "COPY_DEFAULT_WITH")?;
            Ok(Item::CopyDefaultWith {
                chain,
                pos: node.line.pos,
            })
        }
        Some(("SOURCE", colon)) => source(node, colon),
        Some(("RUN", _)) => {
            no_children(&node)?;
            run(&node.line)
        }
        Some(("INVOKE", _)) => {
            no_children(&node)?;
            invoke(&node.line)
        }
        Some(("COPY", _)) => Err(parse_err(node.line.pos, ParseErrorKind::CopyOutsideSource)),
        Some((instr @ ("EXCLUDE" | "OVERRIDE_WITH"), _)) => Err(parse_err(
            node.line.pos,
            ParseErrorKind::OutsideBlock {
                instr,
                expected: "COPY",
            },
        )),
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
        // SPEC-DEVIATION(EX-001): `SOURCE: r` reports Syntax (a malformed
        // SOURCE), not the UnknownInstruction the spec's colon rule implies
        // (`SOURCE:` is only a keyword when nothing follows it). Syntax names
        // the instruction the author meant.
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

    let mut excludes = Vec::new();
    let mut override_with = None;
    for child in &node.children {
        match keyword_of(&child.line.first) {
            Some(("EXCLUDE", _)) => {
                no_children(child)?;
                excludes.extend(exclude(&child.line)?);
            }
            Some(("OVERRIDE_WITH", _)) => {
                no_children(child)?;
                if override_with.is_some() {
                    return Err(parse_err(
                        child.line.pos,
                        ParseErrorKind::Duplicate {
                            what: "OVERRIDE_WITH".to_string(),
                        },
                    ));
                }
                override_with = Some(chain(&child.line, "OVERRIDE_WITH")?);
            }
            _ => return Err(misplaced(child, "COPY")),
        }
    }

    Ok(CopyAst {
        src: to_word(&src.segments, src),
        dest: to_word(&dest.segments, dest),
        key: to_word(&key.segments, key),
        excludes,
        override_with,
        pos: node.line.pos,
    })
}

const EXCLUDE_FORM: &str = "EXCLUDE <pattern>... (or --binary)";

/// `EXCLUDE <pattern>...`: each token is a gitignore pattern, except a fully
/// bare `--binary` (the marker) and any other fully bare `--` token (an
/// unknown flag; quote it to mean a pattern). A trailing colon is literal.
fn exclude(line: &LogicalLine) -> Result<Vec<ExcludeAst>> {
    let tokens = line.words()?;
    if tokens.is_empty() {
        return Err(syntax(line.pos, "EXCLUDE", EXCLUDE_FORM));
    }
    let mut out = Vec::with_capacity(tokens.len());
    for token in &tokens {
        match token.bare_text() {
            Some("--binary") => out.push(ExcludeAst::Binary),
            Some(flag) if flag.starts_with("--") => {
                return Err(parse_err(
                    token.pos,
                    ParseErrorKind::UnknownFlag {
                        instr: "EXCLUDE",
                        flag: flag.to_string(),
                    },
                ));
            }
            _ => out.push(ExcludeAst::Pattern(to_word(&token.segments, token))),
        }
    }
    Ok(out)
}

const CHAIN_FORM: &str = "[transform --flag value, ...]";
const FLAG_FORM: &str = "--flag";
const NAME_FORM: &str = "a transform name matching [A-Za-z0-9][A-Za-z0-9_.-]*";
const ONLY_FORM: &str = "--only <glob>...";
const RUN_FORM: &str = "RUN <transform>[@N] [--flag value ...] [--only <glob>...]";
const INVOKE_FORM: &str = "INVOKE <recipe>[@N] [name=value ...]";
const ASSIGN_FORM: &str = "name=value";
const RECIPE_NAME_FORM: &str = "a recipe name matching [A-Za-z0-9][A-Za-z0-9_.-]*";

fn syntax(pos: Position, instr: &'static str, expected: &'static str) -> Error {
    parse_err(pos, ParseErrorKind::Syntax { instr, expected })
}

/// Parse the rest of a line as a transform chain:
/// `[]` or `[name flag*, name flag*, ...]`.
fn chain(line: &LogicalLine, instr: &'static str) -> Result<Vec<ChainElem>> {
    let tokens = line.chain()?;
    let mut rest = tokens.iter().peekable();
    match rest.next() {
        Some(ChainToken::Open(_)) => {}
        Some(other) => return Err(syntax(other.pos(), instr, CHAIN_FORM)),
        None => return Err(syntax(line.pos, instr, CHAIN_FORM)),
    }

    let mut elems = Vec::new();
    if matches!(rest.peek(), Some(ChainToken::Close(_))) {
        rest.next();
    } else {
        loop {
            let name_token = match rest.next() {
                Some(ChainToken::Word(token)) => token,
                Some(other) => return Err(syntax(other.pos(), instr, CHAIN_FORM)),
                None => return Err(syntax(line.pos, instr, CHAIN_FORM)),
            };
            let (name, pin) = split_pin(name_token, instr, NAME_FORM)?;
            if pin.is_some() {
                return Err(parse_err(name_token.pos, ParseErrorKind::PinNotAllowed));
            }

            let mut words = Vec::new();
            while let Some(ChainToken::Word(token)) = rest.peek() {
                words.push(token.clone());
                rest.next();
            }
            let (flags, _) = parse_flags(&words, instr, false)?;
            elems.push(ChainElem {
                name,
                flags,
                pos: name_token.pos,
            });

            match rest.next() {
                Some(ChainToken::Comma(_)) => {}
                Some(ChainToken::Close(_)) => break,
                Some(other) => return Err(syntax(other.pos(), instr, CHAIN_FORM)),
                None => return Err(syntax(line.pos, instr, CHAIN_FORM)),
            }
        }
    }

    if let Some(extra) = rest.next() {
        return Err(syntax(extra.pos(), instr, CHAIN_FORM));
    }
    Ok(elems)
}

/// `RUN <transform>[@N] [flags] [--only <glob> ...]`.
fn run(line: &LogicalLine) -> Result<Item> {
    let words = line.words()?;
    let Some((name_token, rest)) = words.split_first() else {
        return Err(syntax(line.pos, "RUN", RUN_FORM));
    };
    let (name, pin) = split_pin(name_token, "RUN", NAME_FORM)?;
    let pin = pin_number(pin, name_token)?;
    let (flags, only) = parse_flags(rest, "RUN", true)?;
    Ok(Item::Run {
        name,
        pin,
        flags,
        only,
        pos: line.pos,
    })
}

/// `INVOKE <recipe>[@N] [name=value ...]`.
fn invoke(line: &LogicalLine) -> Result<Item> {
    let words = line.words()?;
    let Some((name_token, rest)) = words.split_first() else {
        return Err(syntax(line.pos, "INVOKE", INVOKE_FORM));
    };
    let (name, pin) = split_pin(name_token, "INVOKE", RECIPE_NAME_FORM)?;
    let pin = pin_number(pin, name_token)?;

    let mut assigns = Vec::with_capacity(rest.len());
    let mut seen = HashSet::new();
    for token in rest {
        let Some((name_part, value_part)) = split_assign(token) else {
            return Err(syntax(token.pos, "INVOKE", ASSIGN_FORM));
        };
        let arg = match name_part.as_slice() {
            [RawSegment::Bare(text)] if is_arg_name(text) => text.clone(),
            _ => {
                let shown = Token {
                    segments: name_part,
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
        if !seen.insert(arg.clone()) {
            return Err(parse_err(
                token.pos,
                ParseErrorKind::Duplicate {
                    what: format!("INVOKE argument {arg}"),
                },
            ));
        }
        assigns.push((arg, to_word(&value_part, token), token.pos));
    }
    Ok(Item::Invoke {
        name,
        pin,
        assigns,
        pos: line.pos,
    })
}

/// Split a bare `name` or `name@N` token. Returns the name and the pin text.
/// `name_form` is the expected text for a malformed name.
fn split_pin(
    token: &Token,
    instr: &'static str,
    name_form: &'static str,
) -> Result<(String, Option<String>)> {
    let Some(text) = token.bare_text() else {
        return Err(syntax(token.pos, instr, name_form));
    };
    let (name, pin) = match text.split_once('@') {
        Some((name, pin)) => (name, Some(pin.to_string())),
        None => (text, None),
    };
    if !is_name(name) {
        return Err(syntax(token.pos, instr, name_form));
    }
    Ok((name.to_string(), pin))
}

/// Turn pin text into a version number (decimal, >= 1), or InvalidNumber at
/// the name token.
fn pin_number(pin: Option<String>, token: &Token) -> Result<Option<u32>> {
    let Some(digits) = pin else {
        return Ok(None);
    };
    parse_pin(&digits).map(Some).ok_or_else(|| {
        parse_err(
            token.pos,
            ParseErrorKind::InvalidNumber {
                what: "version pin",
            },
        )
    })
}

/// A version pin: decimal, >= 1, fits u32.
fn parse_pin(digits: &str) -> Option<u32> {
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok().filter(|n| *n >= 1)
}

/// A token that starts with a bare `--`: its name and inline `=value`.
/// Returns None for a non-flag token.
fn flag_parts(token: &Token) -> Option<(String, Option<Word>)> {
    let Some(RawSegment::Bare(first)) = token.segments.first() else {
        return None;
    };
    let rest_of_first = first.strip_prefix("--")?;
    let mut segments = token.segments.clone();
    if rest_of_first.is_empty() {
        segments.remove(0);
    } else {
        segments[0] = RawSegment::Bare(rest_of_first.to_string());
    }
    let stripped = Token {
        segments,
        pos: token.pos,
    };
    let (name_part, value) = match split_assign(&stripped) {
        Some((name_part, value_part)) => (name_part, Some(to_word(&value_part, token))),
        None => (stripped.segments.clone(), None),
    };
    let name = match name_part.as_slice() {
        [] => String::new(),
        [RawSegment::Bare(text)] => text.clone(),
        _ => Token {
            segments: name_part,
            pos: token.pos,
        }
        .display(),
    };
    Some((name, value))
}

/// Parse a run of flag tokens. With `allow_only`, `--only` collects globs
/// (every following non-flag token) and may repeat; otherwise it is an
/// ordinary flag. Returns the flags and the `--only` globs.
fn parse_flags(
    tokens: &[Token],
    instr: &'static str,
    allow_only: bool,
) -> Result<(Vec<Flag>, Vec<Word>)> {
    let mut out = Vec::new();
    let mut only = Vec::new();
    let mut seen = HashSet::new();
    let mut i = 0;
    while let Some(token) = tokens.get(i) {
        let Some((name, inline)) = flag_parts(token) else {
            return Err(syntax(token.pos, instr, FLAG_FORM));
        };
        if !is_flag_name(&name) {
            return Err(syntax(token.pos, instr, FLAG_FORM));
        }
        i += 1;

        if allow_only && name == "only" {
            let before = only.len();
            // An empty inline value (`--only=`) is no glob, so it errors
            // like a bare `--only` with nothing after it.
            only.extend(inline.filter(|w| !w.segments.is_empty()));
            while let Some(glob) = tokens.get(i).filter(|t| flag_parts(t).is_none()) {
                only.push(to_word(&glob.segments, glob));
                i += 1;
            }
            if only.len() == before {
                return Err(syntax(token.pos, instr, ONLY_FORM));
            }
            continue;
        }

        let value = match inline {
            Some(value) => value,
            None => match tokens.get(i).filter(|t| flag_parts(t).is_none()) {
                Some(value) => {
                    i += 1;
                    to_word(&value.segments, value)
                }
                None => Word {
                    segments: vec![Segment::Lit("true".to_string())],
                    pos: token.pos,
                },
            },
        };
        if !seen.insert(name.clone()) {
            return Err(parse_err(
                token.pos,
                ParseErrorKind::Duplicate {
                    what: format!("flag --{name}"),
                },
            ));
        }
        out.push(Flag { name, value });
    }
    Ok((out, only))
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
    use crate::recipe::SHIPPED_DEFAULT_RECIPE;
    use crate::recipe::error::{Error, ParseErrorKind};
    use crate::recipe::parse::ast::{Ast, ChainElem, ExcludeAst, Flag, Item, Segment};
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

    use super::{
        ASSIGN_FORM, CHAIN_FORM, EXCLUDE_FORM, FLAG_FORM, INVOKE_FORM, NAME_FORM, ONLY_FORM,
        RECIPE_NAME_FORM, RUN_FORM,
    };

    const COPY_FORM: &str = "COPY <src> <dest> AS <key>";

    /// Flags as (name, value-as-written) pairs.
    fn flag_pairs(flags: &[Flag]) -> Vec<(String, String)> {
        flags
            .iter()
            .map(|f| (f.name.clone(), f.value.display()))
            .collect()
    }

    /// The chain of a one-line COPY_DEFAULT_WITH recipe as (name, flags).
    fn chain_of(source: &str) -> Vec<(String, Vec<(String, String)>)> {
        match ast(source).items.as_slice() {
            [Item::CopyDefaultWith { chain, .. }] => chain
                .iter()
                .map(|ChainElem { name, flags, .. }| (name.clone(), flag_pairs(flags)))
                .collect(),
            other => panic!("expected one COPY_DEFAULT_WITH, got {other:?}"),
        }
    }

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

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

    /// Test 19 (C1, C2, and C3 rows): known instructions in the wrong block.
    #[test]
    fn misplaced_instructions_error_with_expected_parent() {
        let not_in = |instr, parent| ParseErrorKind::NotAllowedIn { instr, parent };
        let outside = |instr| ParseErrorKind::OutsideBlock {
            instr,
            expected: "COPY",
        };
        let cases = [
            ("SOURCE r:\n  ARG a", not_in("ARG", "SOURCE"), 2, 3),
            (
                "SOURCE r:\n  COPY . x/ AS k:\n    ARG a",
                not_in("ARG", "COPY"),
                3,
                5,
            ),
            (
                "SOURCE r:\n  COPY . x/ AS k:\n    SOURCE s:",
                not_in("SOURCE", "COPY"),
                3,
                5,
            ),
            ("SOURCE r:\n  RUN flatten", not_in("RUN", "SOURCE"), 2, 3),
            ("EXCLUDE *.log", outside("EXCLUDE"), 1, 1),
            ("OVERRIDE_WITH []", outside("OVERRIDE_WITH"), 1, 1),
            (
                "SOURCE r:\n  EXCLUDE *.log",
                not_in("EXCLUDE", "SOURCE"),
                2,
                3,
            ),
        ];
        for (source, kind, line, col) in cases {
            assert_eq!(
                parse_error(source),
                (kind, line, col),
                "misplaced instruction in {source:?}"
            );
        }
    }

    /// The members of the only COPY in a one-SOURCE recipe.
    fn copy_members(source: &str) -> (Vec<ExcludeAst>, Option<Vec<ChainElem>>) {
        match ast(source).items.as_slice() {
            [Item::Source { copies, .. }] => match copies.as_slice() {
                [copy] => (copy.excludes.clone(), copy.override_with.clone()),
                other => panic!("expected one COPY, got {other:?}"),
            },
            other => panic!("expected one SOURCE, got {other:?}"),
        }
    }

    fn lit(text: &str) -> Vec<Segment> {
        vec![Segment::Lit(text.to_string())]
    }

    fn pattern_texts(excludes: &[ExcludeAst]) -> Vec<String> {
        excludes
            .iter()
            .map(|e| match e {
                ExcludeAst::Pattern(word) => word.display(),
                ExcludeAst::Binary => "<binary>".to_string(),
            })
            .collect()
    }

    /// Test 23 (F-17): a trailing colon on a leaf is literal; a quoted colon opens nothing.
    #[test]
    fn trailing_colon_on_leaf_and_quoted_colon_are_literal() {
        let (excludes, _) = copy_members("SOURCE r:\n  COPY . x/ AS k:\n    EXCLUDE foo: bar");
        assert_eq!(
            pattern_texts(&excludes),
            vec!["foo:", "bar"],
            "EXCLUDE foo: excludes the pattern foo:"
        );
        match ast("SOURCE r:\n  COPY . x/ AS \"k:\"").items.as_slice() {
            [Item::Source { copies, .. }] => assert_eq!(
                copies[0].key.segments,
                lit("k:"),
                "a quoted trailing colon is part of the key, not a block colon"
            ),
            other => panic!("expected one SOURCE, got {other:?}"),
        }
    }

    /// Test 28 (C3 row; C5 adds the WATCH rows): duplicate block members.
    #[test]
    fn duplicate_block_members_error() {
        let cases = [(
            "SOURCE r:\n  COPY . x/ AS k:\n    OVERRIDE_WITH []\n    OVERRIDE_WITH [enrichment-trim]",
            "OVERRIDE_WITH",
            4,
            5,
        )];
        for (source, what, line, col) in cases {
            assert_eq!(
                parse_error(source),
                (ParseErrorKind::Duplicate { what: what.into() }, line, col),
                "duplicate member in {source:?}"
            );
        }
    }

    /// Test 35: `--binary` is the marker; it mixes with patterns; quoted `--x` is a pattern.
    #[test]
    fn exclude_binary_marker_mixes_with_patterns() {
        let (excludes, override_with) = copy_members(
            "SOURCE r:\n  COPY . x/ AS k:\n    EXCLUDE *.log --binary \"--x\"\n    EXCLUDE node_modules/",
        );
        assert_eq!(
            pattern_texts(&excludes),
            vec!["*.log", "<binary>", "--x", "node_modules/"],
            "patterns and the marker in file order, across two EXCLUDE lines"
        );
        assert!(override_with.is_none(), "no OVERRIDE_WITH declared");

        let (only_binary, _) = copy_members("SOURCE r:\n  COPY . x/ AS k:\n    EXCLUDE --binary");
        assert_eq!(only_binary, vec![ExcludeAst::Binary], "the marker alone");

        let (_, chain) = copy_members(
            "SOURCE r:\n  COPY . x/ AS k:\n    OVERRIDE_WITH [enrichment-trim]\n    EXCLUDE a",
        );
        let names: Vec<String> = chain
            .expect("OVERRIDE_WITH is recorded")
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["enrichment-trim"], "members in any order");
    }

    /// Test 36: an unknown bare `--` flag and an empty EXCLUDE are errors.
    #[test]
    fn exclude_unknown_flag_and_empty_error() {
        let cases = [
            (
                "SOURCE r:\n  COPY . x/ AS k:\n    EXCLUDE a --binaries",
                ParseErrorKind::UnknownFlag {
                    instr: "EXCLUDE",
                    flag: "--binaries".into(),
                },
                3,
                15,
            ),
            (
                "SOURCE r:\n  COPY . x/ AS k:\n    EXCLUDE",
                syntax("EXCLUDE", EXCLUDE_FORM),
                3,
                5,
            ),
        ];
        for (source, kind, line, col) in cases {
            assert_eq!(
                parse_error(source),
                (kind, line, col),
                "EXCLUDE error in {source:?}"
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
        assert_eq!(
            parse_error("SOURCE \"r:\"\n  COPY . x/ AS k"),
            (ParseErrorKind::MissingColon { instr: "SOURCE" }, 1, 1),
            "a quoted colon is not a block colon"
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

    /// Test 22 (F-17): the colon is optional on a childless block; the shipped text parses.
    #[test]
    fn childless_block_colon_optional() {
        assert!(
            SHIPPED_DEFAULT_RECIPE.starts_with("ARG repo\n"),
            "the file's directory comment is stripped from the embedded text"
        );
        assert_eq!(
            crate::recipe::strip_prefix_const("abc", "x"),
            "abc",
            "no prefix match leaves the text unchanged"
        );
        assert_eq!(
            crate::recipe::strip_prefix_const("ab", "abc"),
            "ab",
            "a prefix longer than the text leaves it unchanged"
        );
        let items = ast(SHIPPED_DEFAULT_RECIPE).items;
        assert_eq!(items.len(), 5, "ARG, COPY_DEFAULT_WITH, SOURCE, RUN, RUN");
        match &items[2] {
            Item::Source { copies, .. } => {
                assert_eq!(copies.len(), 1, "one COPY without a colon")
            }
            other => panic!("expected SOURCE, got {other:?}"),
        }
        let with_colon = ast("SOURCE r:\n  COPY . x/ AS k:\nRUN flatten").items;
        assert_eq!(with_colon.len(), 2, "a childless COPY may keep its colon");
    }

    /// Test 30: chain forms, including quoted values and `--only` as an ordinary flag.
    #[test]
    fn chain_forms_parse() {
        let cdw = |chain: &str| chain_of(&format!("COPY_DEFAULT_WITH {chain}"));
        assert_eq!(cdw("[]"), vec![], "empty chain");
        assert_eq!(cdw("[a]"), vec![("a".into(), vec![])], "one element");
        assert_eq!(
            cdw("[ a --k v , b --x=y --flag ]"),
            vec![
                ("a".into(), pairs(&[("k", "v")])),
                ("b".into(), pairs(&[("x", "y"), ("flag", "true")])),
            ],
            "spaced punctuation, inline value, bare flag"
        );
        assert_eq!(
            cdw("[a --k \"q ]\"]"),
            vec![("a".into(), pairs(&[("k", "q ]")]))],
            "a quoted bracket is literal"
        );
        assert_eq!(
            cdw("[a --only x]"),
            vec![("a".into(), pairs(&[("only", "x")]))],
            "--only is an ordinary flag in a chain"
        );
        assert_eq!(
            cdw("[a --k ${v}, b --empty=]"),
            vec![
                ("a".into(), pairs(&[("k", "${v}")])),
                ("b".into(), pairs(&[("empty", "")])),
            ],
            "variable value and empty inline value"
        );
    }

    /// Test 31: malformed chains.
    #[test]
    fn chain_malformed_errors() {
        let cdw = "COPY_DEFAULT_WITH";
        let cases = [
            ("COPY_DEFAULT_WITH", syntax(cdw, CHAIN_FORM), 1),
            ("COPY_DEFAULT_WITH a", syntax(cdw, CHAIN_FORM), 19),
            ("COPY_DEFAULT_WITH [a", syntax(cdw, CHAIN_FORM), 1),
            ("COPY_DEFAULT_WITH [a,]", syntax(cdw, CHAIN_FORM), 22),
            ("COPY_DEFAULT_WITH [a,,b]", syntax(cdw, CHAIN_FORM), 22),
            ("COPY_DEFAULT_WITH [a] x", syntax(cdw, CHAIN_FORM), 23),
            ("COPY_DEFAULT_WITH [a b]", syntax(cdw, FLAG_FORM), 22),
            ("COPY_DEFAULT_WITH [a --Bad]", syntax(cdw, FLAG_FORM), 22),
            ("COPY_DEFAULT_WITH [\"a\"]", syntax(cdw, NAME_FORM), 20),
            ("COPY_DEFAULT_WITH [a!b]", syntax(cdw, NAME_FORM), 20),
            ("COPY_DEFAULT_WITH [a@2]", ParseErrorKind::PinNotAllowed, 20),
            (
                "COPY_DEFAULT_WITH [a --k 1 --k 2]",
                ParseErrorKind::Duplicate {
                    what: "flag --k".into(),
                },
                28,
            ),
        ];
        for (source, kind, col) in cases {
            assert_eq!(
                parse_error(source),
                (kind, 1, col),
                "malformed chain in {source:?}"
            );
        }
    }

    /// Test 32: RUN with a pin, flags, and repeated `--only` globs.
    #[test]
    fn run_parses_pin_flags_and_only() {
        let source = "RUN pack@2 --format xml --only a/** b/** --file-limit=1 --dry --only c/**";
        match ast(source).items.as_slice() {
            [
                Item::Run {
                    name,
                    pin,
                    flags,
                    only,
                    pos,
                },
            ] => {
                assert_eq!(name, "pack", "transform name");
                assert_eq!(*pin, Some(2), "version pin");
                assert_eq!(
                    flag_pairs(flags),
                    pairs(&[("format", "xml"), ("file-limit", "1"), ("dry", "true")]),
                    "flags in order; --only is not a flag"
                );
                let globs: Vec<String> = only.iter().map(|w| w.display()).collect();
                assert_eq!(globs, vec!["a/**", "b/**", "c/**"], "globs accumulate");
                assert_eq!(*pos, Position::new(1, 1), "RUN position");
            }
            other => panic!("expected one RUN, got {other:?}"),
        }

        let quoted = "RUN pack --format \"--x\" --only \"--y\" z";
        match ast(quoted).items.as_slice() {
            [Item::Run { flags, only, .. }] => {
                assert_eq!(
                    flag_pairs(flags),
                    pairs(&[("format", "--x")]),
                    "a quoted --x is a flag value, never a flag"
                );
                let globs: Vec<String> = only.iter().map(|w| w.display()).collect();
                assert_eq!(globs, vec!["--y", "z"], "a quoted --y is an --only glob");
            }
            other => panic!("expected one RUN, got {other:?}"),
        }
    }

    /// Test 33 (RUN and INVOKE rows): malformed RUN and INVOKE lines.
    #[test]
    fn run_and_invoke_malformed_errors() {
        let bad_pin = ParseErrorKind::InvalidNumber {
            what: "version pin",
        };
        let cases = [
            ("RUN", syntax("RUN", RUN_FORM), 1),
            ("RUN pack --only", syntax("RUN", ONLY_FORM), 10),
            ("RUN pack --only a --only", syntax("RUN", ONLY_FORM), 19),
            ("RUN pack --only=", syntax("RUN", ONLY_FORM), 10),
            ("RUN flatten:", syntax("RUN", NAME_FORM), 5),
            ("RUN pack extra", syntax("RUN", FLAG_FORM), 10),
            ("RUN \"pack\"", syntax("RUN", NAME_FORM), 5),
            ("RUN pack@0", bad_pin.clone(), 5),
            ("RUN pack@x", bad_pin.clone(), 5),
            ("RUN pack@", bad_pin, 5),
            (
                "RUN pack --format a --format b",
                ParseErrorKind::Duplicate {
                    what: "flag --format".into(),
                },
                21,
            ),
            ("INVOKE", syntax("INVOKE", INVOKE_FORM), 1),
            ("INVOKE base repo", syntax("INVOKE", ASSIGN_FORM), 13),
            (
                "INVOKE base a=1 a=2",
                ParseErrorKind::Duplicate {
                    what: "INVOKE argument a".into(),
                },
                17,
            ),
            (
                "INVOKE base@0",
                ParseErrorKind::InvalidNumber {
                    what: "version pin",
                },
                8,
            ),
            ("INVOKE \"base\"", syntax("INVOKE", RECIPE_NAME_FORM), 8),
            (
                "INVOKE base my-arg=1",
                ParseErrorKind::InvalidArgName {
                    name: "my-arg".into(),
                },
                13,
            ),
        ];
        for (source, kind, col) in cases {
            assert_eq!(
                parse_error(source),
                (kind, 1, col),
                "malformed RUN or INVOKE {source:?}"
            );
        }
    }

    /// Test 34: INVOKE with a pin and assignments (quoted and variable values).
    #[test]
    fn invoke_parses_pin_and_assignments() {
        match ast("INVOKE base@3 repo=a msg=\"x y\" out=${o}")
            .items
            .as_slice()
        {
            [
                Item::Invoke {
                    name,
                    pin,
                    assigns,
                    pos,
                },
            ] => {
                assert_eq!(name, "base", "recipe name");
                assert_eq!(*pin, Some(3), "version pin");
                let shown: Vec<(String, String, Position)> = assigns
                    .iter()
                    .map(|(arg, word, at)| (arg.clone(), word.display(), *at))
                    .collect();
                assert_eq!(
                    shown,
                    vec![
                        ("repo".into(), "a".into(), Position::new(1, 15)),
                        ("msg".into(), "x y".into(), Position::new(1, 22)),
                        ("out".into(), "${o}".into(), Position::new(1, 32)),
                    ],
                    "assignments with values and positions"
                );
                assert_eq!(*pos, Position::new(1, 1), "INVOKE position");
            }
            other => panic!("expected one INVOKE, got {other:?}"),
        }
        match ast("INVOKE base").items.as_slice() {
            [Item::Invoke { pin, assigns, .. }] => {
                assert_eq!(*pin, None, "no pin");
                assert!(assigns.is_empty(), "no assignments");
            }
            other => panic!("expected one INVOKE, got {other:?}"),
        }
    }
}
