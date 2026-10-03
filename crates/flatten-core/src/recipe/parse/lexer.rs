// crates/flatten-core/src/recipe/parse/lexer.rs
//
// Recipe lexer: physical lines -> logical lines -> tokens.
//
// `logical_lines` applies the physical-line rules: leading BOM, CRLF, tab
// indentation, `#` comments, blank lines, and `\` continuation. It returns
// one LogicalLine per instruction, with the first token already lexed and
// the rest kept as characters that remember their physical positions.
// `LogicalLine::words` tokenizes that rest in word mode; `LogicalLine::chain`
// tokenizes it in chain mode, where bare `[`, `]`, and `,` are punctuation.
//
// A token is a run of segments with no whitespace between them:
//   bare text, "quoted text" (escapes \" \\ \n \t), and ${NAME} variables.
// See the Lexical rules section of the EX-001 spec and docs/RECIPE.md.

use super::path::is_arg_name;
use crate::recipe::error::{ParseErrorKind, Result, parse_err};
use crate::recipe::types::Position;

/// Characters of a logical line, each with its physical position.
type Chars = Vec<(char, Position)>;

/// One instruction line after comment stripping and continuation joining.
#[derive(Debug)]
pub(crate) struct LogicalLine {
    /// Leading spaces on the first physical line.
    pub indent: u32,
    /// Position of the first token.
    pub pos: Position,
    /// The first token, lexed in word mode (the keyword).
    pub first: Token,
    /// Everything after `first`; each char keeps its physical position.
    rest: Chars,
}

impl LogicalLine {
    /// Tokenize everything after the first token in word mode: tokens split
    /// on whitespace only, so `[`, `]`, and `,` are ordinary characters.
    pub fn words(&self) -> Result<Vec<Token>> {
        let mut out = Vec::new();
        let mut at = 0;
        while let Some((token, next)) = next_token(&self.rest, at, Mode::Word)? {
            out.push(token);
            at = next;
        }
        Ok(out)
    }

    /// Tokenize everything after the first token in chain mode: bare `[`,
    /// `]`, and `,` are punctuation and end the token before them. Quoted
    /// text is still literal, so `"]"` is a word.
    pub fn chain(&self) -> Result<Vec<ChainToken>> {
        let mut out = Vec::new();
        let mut at = 0;
        loop {
            while self.rest.get(at).is_some_and(|&(c, _)| is_ws(c)) {
                at += 1;
            }
            let Some(&(c, pos)) = self.rest.get(at) else {
                return Ok(out);
            };
            let punct = match c {
                '[' => Some(ChainToken::Open(pos)),
                ']' => Some(ChainToken::Close(pos)),
                ',' => Some(ChainToken::Comma(pos)),
                _ => None,
            };
            if let Some(punct) = punct {
                out.push(punct);
                at += 1;
                continue;
            }
            let Some((token, next)) = next_token(&self.rest, at, Mode::Chain)? else {
                return Ok(out);
            };
            out.push(ChainToken::Word(token));
            at = next;
        }
    }
}

/// A chain-mode token.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ChainToken {
    /// `[`
    Open(Position),
    /// `]`
    Close(Position),
    /// `,`
    Comma(Position),
    /// Anything else.
    Word(Token),
}

impl ChainToken {
    /// Where the token starts.
    pub fn pos(&self) -> Position {
        match self {
            ChainToken::Open(pos) | ChainToken::Close(pos) | ChainToken::Comma(pos) => *pos,
            ChainToken::Word(token) => token.pos,
        }
    }
}

/// Which characters end a bare run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Whitespace only.
    Word,
    /// Whitespace, `[`, `]`, and `,`.
    Chain,
}

impl Mode {
    fn ends_bare(self, c: char) -> bool {
        is_ws(c) || (self == Mode::Chain && matches!(c, '[' | ']' | ','))
    }
}

/// A whitespace-delimited token.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Token {
    /// The segments in source order; adjacent bare text is one segment.
    pub segments: Vec<RawSegment>,
    /// Position of the token's first character.
    pub pos: Position,
}

/// One piece of a token. Bare vs quoted matters to the grammar (keywords,
/// the block colon, and `=` splitting act on bare text only).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RawSegment {
    /// Unquoted text.
    Bare(String),
    /// Quoted text with escapes decoded.
    Quoted(String),
    /// A `${NAME}` reference.
    Var {
        /// The variable name.
        name: String,
        /// Position of the `$`.
        pos: Position,
    },
}

impl Token {
    /// The token's text when it is exactly one bare segment.
    pub fn bare_text(&self) -> Option<&str> {
        match self.segments.as_slice() {
            [RawSegment::Bare(text)] => Some(text),
            _ => None,
        }
    }

    /// Source-like rendering for error messages.
    pub fn display(&self) -> String {
        let mut out = String::new();
        for segment in &self.segments {
            match segment {
                RawSegment::Bare(text) => out.push_str(text),
                RawSegment::Quoted(text) => {
                    out.push('"');
                    out.push_str(text);
                    out.push('"');
                }
                RawSegment::Var { name, .. } => {
                    out.push_str("${");
                    out.push_str(name);
                    out.push('}');
                }
            }
        }
        out
    }
}

/// Split recipe text into logical lines.
pub(crate) fn logical_lines(source: &str) -> Result<Vec<LogicalLine>> {
    let source = source.strip_prefix('\u{FEFF}').unwrap_or(source);
    let mut out = Vec::new();
    let mut pending: Option<Pending> = None;

    for (index, raw) in source.split('\n').enumerate() {
        let line_no = to_u32(index + 1);
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let physical = scan_physical(raw, line_no)?;

        match (pending.take(), physical) {
            (None, Physical::Blank) => {}
            (Some(acc), Physical::Blank) => {
                return Err(parse_err(
                    acc.backslash,
                    ParseErrorKind::DanglingContinuation,
                ));
            }
            (
                None,
                Physical::Content {
                    indent,
                    chars,
                    backslash,
                },
            ) => match backslash {
                Some(backslash) => {
                    pending = Some(Pending {
                        indent,
                        chars,
                        backslash,
                    })
                }
                None => out.extend(finish(indent, chars)?),
            },
            (
                Some(mut acc),
                Physical::Content {
                    chars, backslash, ..
                },
            ) => {
                acc.chars.push((' ', acc.backslash));
                acc.chars.extend(chars);
                match backslash {
                    Some(backslash) => {
                        acc.backslash = backslash;
                        pending = Some(acc);
                    }
                    None => out.extend(finish(acc.indent, acc.chars)?),
                }
            }
        }
    }

    if let Some(acc) = pending {
        return Err(parse_err(
            acc.backslash,
            ParseErrorKind::DanglingContinuation,
        ));
    }
    Ok(out)
}

/// A logical line still being joined across continuations.
struct Pending {
    indent: u32,
    chars: Chars,
    /// Position of the most recent trailing `\`.
    backslash: Position,
}

/// One physical line after comment stripping.
enum Physical {
    /// Blank or comment-only.
    Blank,
    /// Content with leading whitespace and the comment removed.
    Content {
        indent: u32,
        chars: Chars,
        /// Position of a trailing continuation `\`, already removed from `chars`.
        backslash: Option<Position>,
    },
}

/// Apply the physical-line rules to one line (without its `\n` or `\r`).
fn scan_physical(raw: &str, line_no: u32) -> Result<Physical> {
    let chars: Vec<char> = raw.chars().collect();
    let pos = |index: usize| Position::new(line_no, to_u32(index + 1));

    // Leading whitespace. A tab is only an error if the line has content.
    let mut start = 0;
    let mut first_tab = None;
    while let Some(&c) = chars.get(start) {
        if !is_ws(c) {
            break;
        }
        if c == '\t' && first_tab.is_none() {
            first_tab = Some(start);
        }
        start += 1;
    }

    // Find the comment start, tracking quotes so `"a # b"` is not a comment.
    let mut end = chars.len();
    let mut open_quote: Option<usize> = None;
    let mut i = start;
    while i < chars.len() {
        let c = chars[i];
        match open_quote {
            Some(_) if c == '\\' => i += 1,
            Some(_) if c == '"' => open_quote = None,
            Some(_) => {}
            None if c == '"' => open_quote = Some(i),
            None if c == '#' && (i == start || is_ws(chars[i - 1])) => {
                end = i;
                break;
            }
            None => {}
        }
        i += 1;
    }
    if let Some(quote) = open_quote {
        return Err(parse_err(pos(quote), ParseErrorKind::UnterminatedQuote));
    }

    // Trim trailing whitespace; an empty result is a blank line.
    while end > start && is_ws(chars[end - 1]) {
        end -= 1;
    }
    if end == start {
        return Ok(Physical::Blank);
    }
    if let Some(tab) = first_tab {
        return Err(parse_err(pos(tab), ParseErrorKind::TabIndent));
    }

    let mut backslash = None;
    if chars[end - 1] == '\\' {
        backslash = Some(pos(end - 1));
        end -= 1;
    }

    Ok(Physical::Content {
        indent: to_u32(start),
        chars: (start..end)
            .map(|index| (chars[index], pos(index)))
            .collect(),
        backslash,
    })
}

/// Turn joined content into a LogicalLine by lexing its first token.
fn finish(indent: u32, chars: Chars) -> Result<Option<LogicalLine>> {
    let Some((first, next)) = next_token(&chars, 0, Mode::Word)? else {
        return Ok(None);
    };
    Ok(Some(LogicalLine {
        indent,
        pos: first.pos,
        first,
        rest: chars[next..].to_vec(),
    }))
}

/// Lex the next token at or after `at`. Returns the token and the index
/// just past it, or None at end of input. `mode` decides which characters
/// end a bare run.
fn next_token(chars: &[(char, Position)], at: usize, mode: Mode) -> Result<Option<(Token, usize)>> {
    let mut i = at;
    while chars.get(i).is_some_and(|&(c, _)| is_ws(c)) {
        i += 1;
    }
    let Some(&(_, pos)) = chars.get(i) else {
        return Ok(None);
    };

    let start = i;
    let mut segments = Vec::new();
    let mut bare = String::new();
    while let Some(&(c, here)) = chars.get(i) {
        if mode.ends_bare(c) {
            break;
        }
        if c == '"' {
            flush_bare(&mut bare, &mut segments);
            i = lex_quoted(chars, i, &mut segments)?;
        } else if c == '$' && next_is_brace(chars, i) {
            flush_bare(&mut bare, &mut segments);
            let (name, next) = lex_var(chars, i)?;
            segments.push(RawSegment::Var { name, pos: here });
            i = next;
        } else {
            bare.push(c);
            i += 1;
        }
    }
    flush_bare(&mut bare, &mut segments);
    // A character that ends a bare run before anything was read (chain
    // punctuation) starts no token. Returning None keeps a caller's loop
    // from spinning on a zero-width token.
    if i == start {
        return Ok(None);
    }
    Ok(Some((Token { segments, pos }, i)))
}

/// Lex a quoted run starting at the opening quote. Pushes Quoted and Var
/// segments and returns the index just past the closing quote.
fn lex_quoted(
    chars: &[(char, Position)],
    open: usize,
    segments: &mut Vec<RawSegment>,
) -> Result<usize> {
    let open_pos = chars[open].1;
    let mut text = String::new();
    let mut pushed = false;
    let mut i = open + 1;
    loop {
        let Some(&(c, here)) = chars.get(i) else {
            return Err(parse_err(open_pos, ParseErrorKind::UnterminatedQuote));
        };
        match c {
            '"' => {
                if !text.is_empty() || !pushed {
                    segments.push(RawSegment::Quoted(text));
                }
                return Ok(i + 1);
            }
            '\\' => {
                let Some(&(escaped, _)) = chars.get(i + 1) else {
                    return Err(parse_err(open_pos, ParseErrorKind::UnterminatedQuote));
                };
                text.push(match escaped {
                    '"' => '"',
                    '\\' => '\\',
                    'n' => '\n',
                    't' => '\t',
                    other => return Err(parse_err(here, ParseErrorKind::InvalidEscape(other))),
                });
                i += 2;
            }
            '$' if next_is_brace(chars, i) => {
                if !text.is_empty() {
                    segments.push(RawSegment::Quoted(std::mem::take(&mut text)));
                }
                let (name, next) = lex_var(chars, i)?;
                segments.push(RawSegment::Var { name, pos: here });
                pushed = true;
                i = next;
            }
            _ => {
                text.push(c);
                i += 1;
            }
        }
    }
}

/// Lex `${NAME}` starting at the `$`. Returns the name and the index just
/// past the closing brace.
fn lex_var(chars: &[(char, Position)], dollar: usize) -> Result<(String, usize)> {
    let pos = chars[dollar].1;
    let mut name = String::new();
    let mut i = dollar + 2;
    while let Some(&(c, _)) = chars.get(i) {
        if c == '}' || c == '"' || is_ws(c) {
            break;
        }
        name.push(c);
        i += 1;
    }
    if chars.get(i).map(|&(c, _)| c) != Some('}') {
        let found = format!("${{{name}");
        return Err(parse_err(pos, ParseErrorKind::BadVariable { found }));
    }
    if !is_arg_name(&name) {
        let found = format!("${{{name}}}");
        return Err(parse_err(pos, ParseErrorKind::BadVariable { found }));
    }
    Ok((name, i + 1))
}

fn next_is_brace(chars: &[(char, Position)], i: usize) -> bool {
    chars.get(i + 1).is_some_and(|&(c, _)| c == '{')
}

fn flush_bare(bare: &mut String, segments: &mut Vec<RawSegment>) {
    if !bare.is_empty() {
        segments.push(RawSegment::Bare(std::mem::take(bare)));
    }
}

fn is_ws(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// Line and column counts never approach u32::MAX for real recipes;
/// saturate instead of panicking.
fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::error::Error;

    fn lines(source: &str) -> Vec<LogicalLine> {
        logical_lines(source).expect("source should lex")
    }

    fn words(line: &LogicalLine) -> Vec<Token> {
        line.words().expect("rest of line should lex")
    }

    fn texts(tokens: &[Token]) -> Vec<String> {
        tokens.iter().map(Token::display).collect()
    }

    /// Lex and return (kind, line, col) of the expected error.
    fn lex_error(source: &str) -> (ParseErrorKind, u32, u32) {
        let result = logical_lines(source).and_then(|ls| {
            ls.iter().try_for_each(|l| l.words().map(|_| ()))?;
            Ok(ls)
        });
        match result {
            Err(Error::Parse { location, kind }) => (kind, location.line, location.col),
            other => panic!("expected a lex error for {source:?}, got {other:?}"),
        }
    }

    /// Test 1: blank and comment-only lines produce no logical lines.
    #[test]
    fn blank_and_comment_lines_produce_nothing() {
        let ls = lines(
            "\n   \n# a comment\n    # indented comment\n\t# tab-indented comment\nARG a\n\n",
        );
        assert_eq!(ls.len(), 1, "only the ARG line should remain");
        assert_eq!(
            ls[0].first.display(),
            "ARG",
            "first token of the surviving line"
        );
        assert_eq!(
            ls[0].pos,
            Position::new(6, 1),
            "positions count skipped lines"
        );
    }

    /// Test 2: `#` is literal inside a token and inside quotes.
    #[test]
    fn hash_is_literal_inside_token_and_quotes() {
        let ls = lines("EXCLUDE foo#bar \"a # b\" # trailing comment");
        let w = words(&ls[0]);
        assert_eq!(
            texts(&w),
            vec!["foo#bar".to_string(), "\"a # b\"".to_string()],
            "the trailing comment is dropped, the others stay"
        );
    }

    /// Test 3: continuation joins lines and tokens keep their physical positions.
    #[test]
    fn continuation_joins_lines_keeps_physical_positions() {
        let ls = lines("COPY a \\\n    b AS k\n");
        assert_eq!(ls.len(), 1, "two physical lines form one logical line");
        let w = words(&ls[0]);
        assert_eq!(texts(&w), vec!["a", "b", "AS", "k"], "joined tokens");
        let positions: Vec<Position> = w.iter().map(|t| t.pos).collect();
        assert_eq!(
            positions,
            vec![
                Position::new(1, 6),
                Position::new(2, 5),
                Position::new(2, 7),
                Position::new(2, 10)
            ],
            "tokens on the continued line report line 2"
        );
    }

    /// Test 4: the comment is stripped before the trailing `\` is checked.
    #[test]
    fn comment_then_backslash_continues() {
        let ls = lines("SOURCE r \\ # note\n  :\n");
        assert_eq!(
            ls.len(),
            1,
            "the backslash before the comment continues the line"
        );
        assert_eq!(texts(&words(&ls[0])), vec!["r", ":"], "continued content");

        let ls = lines("# note \\\nARG a\n");
        assert_eq!(
            ls.len(),
            1,
            "a backslash inside a comment does not continue"
        );
        assert_eq!(ls[0].pos, Position::new(2, 1), "the next line stands alone");
        assert_eq!(texts(&words(&ls[0])), vec!["a"], "its own content only");
    }

    /// Test 5: a trailing `\` needs a following content line.
    #[test]
    fn dangling_continuation_errors() {
        for source in [
            "ARG a \\",
            "ARG a \\\n",
            "ARG a \\\n\nARG b",
            "ARG a \\\n# only a comment\nARG b",
        ] {
            assert_eq!(
                lex_error(source),
                (ParseErrorKind::DanglingContinuation, 1, 7),
                "dangling continuation in {source:?}"
            );
        }
    }

    /// Test 6: a tab in leading whitespace is an error at the tab.
    #[test]
    fn tab_in_indent_errors_with_line_col() {
        assert_eq!(
            lex_error("SOURCE r:\n\tCOPY . x/ AS k"),
            (ParseErrorKind::TabIndent, 2, 1),
            "tab at the start of line 2"
        );
        assert_eq!(
            lex_error("  \tARG a"),
            (ParseErrorKind::TabIndent, 1, 3),
            "tab after two spaces"
        );
    }

    /// Test 7: tabs inside quotes are literal; tabs between tokens are whitespace.
    #[test]
    fn tab_inside_quotes_is_literal() {
        let ls = lines("ARG\ta=\"x\ty\"");
        let w = words(&ls[0]);
        assert_eq!(w.len(), 1, "the tab after ARG separates tokens");
        assert_eq!(
            w[0].segments,
            vec![
                RawSegment::Bare("a=".into()),
                RawSegment::Quoted("x\ty".into())
            ],
            "the quoted tab is kept"
        );
    }

    /// Test 8: the four escapes decode inside quotes.
    #[test]
    fn quoted_escapes_decode() {
        let ls = lines(r#"ARG a="q\"b\\c\nd\te""#);
        let w = words(&ls[0]);
        assert_eq!(
            w[0].segments,
            vec![
                RawSegment::Bare("a=".into()),
                RawSegment::Quoted("q\"b\\c\nd\te".into())
            ],
            "decoded escapes"
        );
    }

    /// Test 9: any other escape is an error at the backslash.
    #[test]
    fn unknown_escape_errors() {
        assert_eq!(
            lex_error(r#"ARG a="\q""#),
            (ParseErrorKind::InvalidEscape('q'), 1, 8),
            "invalid escape position is the backslash"
        );
    }

    /// Test 10: an unclosed quote is an error at the opening quote, even before `\`.
    #[test]
    fn unterminated_quote_errors() {
        assert_eq!(
            lex_error("ARG a=\"abc"),
            (ParseErrorKind::UnterminatedQuote, 1, 7),
            "unterminated at end of line"
        );
        assert_eq!(
            lex_error("ARG a=\"abc \\\nx\""),
            (ParseErrorKind::UnterminatedQuote, 1, 7),
            "a quote cannot span lines through a continuation"
        );
    }

    /// Test 11: bare, quoted, and variable segments concatenate into one token.
    #[test]
    fn segments_concatenate_with_variables() {
        let ls = lines("ARG msg=\"a b\" x${A}y \"p ${B} q\" \"\" a$b");
        let w = words(&ls[0]);
        assert_eq!(w.len(), 5, "five tokens");
        assert_eq!(
            w[0].segments,
            vec![
                RawSegment::Bare("msg=".into()),
                RawSegment::Quoted("a b".into())
            ],
            "bare + quoted"
        );
        assert_eq!(
            w[1].segments,
            vec![
                RawSegment::Bare("x".into()),
                RawSegment::Var {
                    name: "A".into(),
                    pos: Position::new(1, 16)
                },
                RawSegment::Bare("y".into())
            ],
            "variable inside a bare token"
        );
        assert_eq!(
            w[2].segments,
            vec![
                RawSegment::Quoted("p ".into()),
                RawSegment::Var {
                    name: "B".into(),
                    pos: Position::new(1, 25)
                },
                RawSegment::Quoted(" q".into())
            ],
            "variable inside quotes"
        );
        assert_eq!(
            w[3].segments,
            vec![RawSegment::Quoted(String::new())],
            "empty quotes"
        );
        assert_eq!(
            w[4].segments,
            vec![RawSegment::Bare("a$b".into())],
            "a lone $ is literal"
        );
    }

    /// Test 12: malformed `${...}` references error at the `$`.
    #[test]
    fn malformed_variables_error() {
        let cases = [
            ("ARG a=${", "${", 7),
            ("ARG a=${1x}", "${1x}", 7),
            ("ARG a=${a", "${a", 7),
            ("ARG a=\"x ${a b}\"", "${a", 10),
        ];
        for (source, found, col) in cases {
            assert_eq!(
                lex_error(source),
                (
                    ParseErrorKind::BadVariable {
                        found: found.into()
                    },
                    1,
                    col
                ),
                "malformed variable in {source:?}"
            );
        }
    }

    /// Test 13: CRLF line endings and a leading BOM are accepted.
    #[test]
    fn crlf_and_leading_bom_accepted() {
        let ls = lines("\u{FEFF}ARG a\r\nARG b\r\n");
        assert_eq!(ls.len(), 2, "two lines");
        assert_eq!(
            ls[0].pos,
            Position::new(1, 1),
            "the BOM does not count as a column"
        );
        assert_eq!(texts(&words(&ls[0])), vec!["a"], "no stray \\r on line 1");
        assert_eq!(texts(&words(&ls[1])), vec!["b"], "no stray \\r on line 2");
    }

    /// Test 14: chain mode splits bare brackets and commas; quoted text stays literal.
    #[test]
    fn chain_mode_splits_brackets_and_commas() {
        let ls = lines("COPY_DEFAULT_WITH [a,b] \"]\" [x --k=v]");
        let tokens = ls[0].chain().expect("chain should lex");
        let shown: Vec<String> = tokens
            .iter()
            .map(|t| match t {
                ChainToken::Open(_) => "[".to_string(),
                ChainToken::Close(_) => "]".to_string(),
                ChainToken::Comma(_) => ",".to_string(),
                ChainToken::Word(w) => w.display(),
            })
            .collect();
        assert_eq!(
            shown,
            vec!["[", "a", ",", "b", "]", "\"]\"", "[", "x", "--k=v", "]"],
            "chain tokens"
        );
        assert_eq!(tokens[2].pos(), Position::new(1, 21), "comma position");
        assert_eq!(
            tokens[5].pos(),
            Position::new(1, 25),
            "quoted bracket position"
        );

        let at_bracket = ls[0]
            .rest
            .iter()
            .position(|&(c, _)| c == ']')
            .expect("the line has a ]");
        assert!(
            next_token(&ls[0].rest, at_bracket, Mode::Chain)
                .expect("lexing at ] cannot fail")
                .is_none(),
            "chain punctuation starts no token, so next_token never returns a zero-width one"
        );
    }

    /// Test 15: word mode keeps brackets and commas inside tokens.
    #[test]
    fn word_mode_keeps_brackets() {
        let ls = lines("EXCLUDE *.[oa] [x],y");
        assert_eq!(
            texts(&words(&ls[0])),
            vec!["*.[oa]", "[x],y"],
            "brackets are ordinary"
        );
    }

    /// Test 16: columns count characters, not bytes.
    #[test]
    fn columns_count_chars_not_bytes() {
        let ls = lines("SOURCE \"\u{e9}\" r");
        let w = words(&ls[0]);
        assert_eq!(
            w[0].pos,
            Position::new(1, 8),
            "quoted token starts at column 8"
        );
        assert_eq!(
            w[1].pos,
            Position::new(1, 12),
            "the two-byte character counts once"
        );
    }
}
