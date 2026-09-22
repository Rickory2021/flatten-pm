// crates/flatten-core/src/recipe/parse/resolve.rs
//
// Resolve: Ast + ARG input -> Recipe.
//
// Runs the items in file order with an ARG environment. Every substitutable
// argument is substituted before it is validated, so the Recipe holds final
// strings (rules are recorded post-substitution).
//
// Two modes:
//   Open   save time, show, lint. A required ARG with no value stays
//          symbolic: its value is the text `${name}`. ARG defaults are
//          treated as final ("defaults must be valid on their own").
//   Bound  export. Every required ARG must have a value; provided values
//          override defaults; undeclared provided names are an error.
// Open mode never rejects a recipe that a binding supplying only its
// required ARGs would accept: every check here runs on literal text or on
// symbolic `${name}` text, which contains no `/`, `..`, or control
// characters and is never empty.
//
// Transform names resolve through the Catalog at save time (D3): an unknown
// name is an error in both modes. COPY chains need file transforms; RUN
// needs a directory transform. `--only` globs go to RunInstruction.scope,
// never into the transform's args.
//
// Implemented so far (plan chunk C2): ARG, COPY_DEFAULT_WITH, SOURCE, COPY,
// RUN.

use std::collections::{BTreeMap, HashMap};

use super::ast::{Ast, ChainElem, CopyAst, Flag, Item, Segment, Word};
use super::path::{canonical_copy_shape, check_key, normalize_rel_path};
use crate::recipe::catalog::{Catalog, TransformInfo, TransformScope};
use crate::recipe::error::{Error, Location, ParseErrorKind, Result, parse_err};
use crate::recipe::types::{
    Arg, CopyBlock, Instruction, Position, Recipe, RunInstruction, SourceInstruction, TransformRef,
};

/// ARG values for resolution.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgInput {
    /// Save-time, show, and lint: required ARGs without a value stay symbolic.
    Open,
    /// Export: the caller's merged values (recipe default, then binding
    /// `arg_values`, then `--arg`); resolve sees one map.
    Bound(BTreeMap<String, String>),
}

/// The result of `analyze`.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolution {
    /// The resolved recipe.
    pub recipe: Recipe,
}

/// The ARG environment and default chain of one recipe.
// SPEC-DEVIATION(EX-001): the spec's frame also holds a `symbolic` flag per
// value and a `declared` list. Both land with their first readers (the
// symbolic flag in C3 for EXCLUDE validation, declaration order in C4 for
// INVOKE binding step 2), per plan rule 2. Until then `recipe.args` carries
// ARG order and `env` answers membership.
#[derive(Default)]
struct Frame {
    env: HashMap<String, String>,
    /// The COPY_DEFAULT_WITH in effect: empty until declared; a later
    /// declaration replaces it for the COPY blocks after it.
    default_chain: Vec<TransformRef>,
}

/// Resolve a parsed recipe.
pub(crate) fn resolve(ast: &Ast, input: &ArgInput, catalog: &dyn Catalog) -> Result<Recipe> {
    let mut frame = Frame::default();
    let mut keys: HashMap<String, Location> = HashMap::new();
    let mut recipe = Recipe {
        args: Vec::new(),
        instructions: Vec::new(),
        unbound: Vec::new(),
    };

    for item in &ast.items {
        match item {
            Item::Arg { name, default, pos } => {
                if frame.env.contains_key(name) {
                    return Err(parse_err(
                        *pos,
                        ParseErrorKind::DuplicateArg { name: name.clone() },
                    ));
                }
                // SPEC-DEVIATION(EX-001): the default is substituted even when
                // a bound value overrides it, so an undeclared reference in a
                // default fails in both modes. That keeps the open-mode
                // invariant: open mode never rejects what bound mode accepts.
                // The spec wording follows at reconcile.
                let default_value = default
                    .as_ref()
                    .map(|word| substitute(word, &frame))
                    .transpose()?;
                let value = match (input, default_value) {
                    (ArgInput::Bound(values), default_value) => {
                        match (values.get(name), default_value) {
                            (Some(value), _) => value.clone(),
                            (None, Some(default_value)) => default_value,
                            (None, None) => {
                                return Err(parse_err(
                                    *pos,
                                    ParseErrorKind::MissingRequiredArg { name: name.clone() },
                                ));
                            }
                        }
                    }
                    (ArgInput::Open, Some(default_value)) => default_value,
                    (ArgInput::Open, None) => {
                        recipe.unbound.push(name.clone());
                        format!("${{{name}}}")
                    }
                };
                frame.env.insert(name.clone(), value);
                recipe.args.push(Arg {
                    name: name.clone(),
                    default: default.as_ref().map(Word::display),
                    required: default.is_none(),
                    position: *pos,
                });
            }
            Item::CopyDefaultWith { chain, .. } => {
                frame.default_chain = resolve_chain(chain, &frame, catalog)?;
            }
            Item::Run {
                name,
                pin,
                flags,
                only,
                pos,
            } => {
                let info = lookup(catalog, name, *pin, *pos)?;
                require_scope(&info, TransformScope::Directory, *pos)?;
                let mut scope = Vec::with_capacity(only.len());
                for glob in only {
                    scope.push(substitute(glob, &frame)?);
                }
                recipe.instructions.push(Instruction::Run(RunInstruction {
                    transform: transform_ref(info, flags, &frame)?,
                    scope,
                    position: *pos,
                }));
            }
            Item::Source { repo, copies, pos } => {
                let repo_name = substitute(repo, &frame)?;
                if repo_name.is_empty() {
                    return Err(parse_err(repo.pos, ParseErrorKind::InvalidRepoName));
                }
                let mut blocks = Vec::with_capacity(copies.len());
                for copy in copies {
                    blocks.push(copy_block(copy, &frame, &mut keys)?);
                }
                recipe
                    .instructions
                    .push(Instruction::Source(SourceInstruction {
                        repo_name,
                        copies: blocks,
                        position: *pos,
                    }));
            }
        }
    }

    if let ArgInput::Bound(values) = input
        && let Some(name) = values.keys().find(|name| !frame.env.contains_key(*name))
    {
        return Err(Error::UnknownArg { name: name.clone() });
    }
    Ok(recipe)
}

/// Resolve one COPY block: substitute, normalize, canonicalize, check the key.
fn copy_block(
    copy: &CopyAst,
    frame: &Frame,
    keys: &mut HashMap<String, Location>,
) -> Result<CopyBlock> {
    let src = substituted_path(&copy.src, frame)?;
    let dest = substituted_path(&copy.dest, frame)?;
    let (src, dest) = canonical_copy_shape(src, dest);

    let key = substitute(&copy.key, frame)?;
    // SPEC-DEVIATION(EX-001): the spec says open mode checks only the literal
    // parts of a symbolic key. This checks the full substituted text, which
    // is equivalent (symbolic `${name}` text is never empty and has no control
    // characters) and also accepts a fully symbolic key such as `AS ${k}`,
    // which a literal-parts-only check would wrongly reject as empty.
    if let Err(reason) = check_key(&key) {
        return Err(parse_err(
            copy.key.pos,
            ParseErrorKind::InvalidKey { key, reason },
        ));
    }
    if let Some(first) = keys.get(&key) {
        return Err(parse_err(
            copy.key.pos,
            ParseErrorKind::DuplicateKey {
                key,
                first: first.clone(),
            },
        ));
    }
    keys.insert(key.clone(), Location::root(copy.key.pos));

    Ok(CopyBlock {
        src,
        dest,
        key,
        forward_chain: frame.default_chain.clone(),
        position: copy.pos,
    })
}

/// Resolve a COPY chain: every element must be a file transform.
fn resolve_chain(
    chain: &[ChainElem],
    frame: &Frame,
    catalog: &dyn Catalog,
) -> Result<Vec<TransformRef>> {
    let mut out = Vec::with_capacity(chain.len());
    for elem in chain {
        let info = lookup(catalog, &elem.name, None, elem.pos)?;
        require_scope(&info, TransformScope::File, elem.pos)?;
        out.push(transform_ref(info, &elem.flags, frame)?);
    }
    Ok(out)
}

/// Look up a transform, current or pinned. An unknown name is
/// UnknownTransform even when pinned; a known name with a missing version is
/// TransformVersionNotFound.
fn lookup(
    catalog: &dyn Catalog,
    name: &str,
    pin: Option<u32>,
    pos: Position,
) -> Result<TransformInfo> {
    let Some(current) = catalog.transform(name, None)? else {
        return Err(parse_err(
            pos,
            ParseErrorKind::UnknownTransform {
                name: name.to_string(),
            },
        ));
    };
    let Some(version) = pin else {
        return Ok(current);
    };
    catalog.transform(name, Some(version))?.ok_or_else(|| {
        parse_err(
            pos,
            ParseErrorKind::TransformVersionNotFound {
                name: name.to_string(),
                version,
            },
        )
    })
}

fn require_scope(info: &TransformInfo, expected: TransformScope, pos: Position) -> Result<()> {
    if info.scope != expected {
        return Err(parse_err(
            pos,
            ParseErrorKind::WrongScope {
                name: info.name.clone(),
                expected,
                found: info.scope,
            },
        ));
    }
    Ok(())
}

/// Build a TransformRef with substituted flag values.
fn transform_ref(info: TransformInfo, flags: &[Flag], frame: &Frame) -> Result<TransformRef> {
    let mut args = BTreeMap::new();
    for flag in flags {
        args.insert(flag.name.clone(), substitute(&flag.value, frame)?);
    }
    Ok(TransformRef {
        name: info.name,
        transform_id: info.transform_id,
        version_id: info.version_id,
        version: info.version,
        args,
    })
}

/// Substitute a path word, then normalize it.
fn substituted_path(word: &Word, frame: &Frame) -> Result<String> {
    let path = substitute(word, frame)?;
    normalize_rel_path(&path)
        .map_err(|issue| parse_err(word.pos, ParseErrorKind::InvalidPath { path, issue }))
}

/// Replace every `${NAME}` in a word with its value. Values are never
/// rescanned, so there is no recursive expansion.
fn substitute(word: &Word, frame: &Frame) -> Result<String> {
    let mut out = String::new();
    for segment in &word.segments {
        match segment {
            Segment::Lit(text) => out.push_str(text),
            Segment::Var { name, pos } => match frame.env.get(name) {
                Some(value) => out.push_str(value),
                None => {
                    return Err(parse_err(
                        *pos,
                        ParseErrorKind::UndeclaredArg { name: name.clone() },
                    ));
                }
            },
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::recipe::catalog::{MemCatalog, TransformScope};
    use crate::recipe::error::{Error, Location, ParseErrorKind, PathIssue};
    use crate::recipe::types::{
        Instruction, Position, Recipe, RunInstruction, SourceInstruction, TransformRef,
    };
    use crate::recipe::{ArgInput, analyze};

    fn bound(pairs: &[(&str, &str)]) -> ArgInput {
        ArgInput::Bound(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    fn run(source: &str, input: &ArgInput) -> Recipe {
        run_with(source, input, &MemCatalog::builtins())
    }

    fn run_with(source: &str, input: &ArgInput, catalog: &MemCatalog) -> Recipe {
        analyze(source, input, catalog)
            .unwrap_or_else(|e| panic!("{source:?} should resolve, got {e}"))
            .recipe
    }

    fn open_ok(source: &str) -> bool {
        analyze(source, &ArgInput::Open, &MemCatalog::builtins()).is_ok()
    }

    fn run_err(source: &str, input: &ArgInput) -> Error {
        run_err_with(source, input, &MemCatalog::builtins())
    }

    fn run_err_with(source: &str, input: &ArgInput, catalog: &MemCatalog) -> Error {
        match analyze(source, input, catalog) {
            Err(e) => e,
            Ok(r) => panic!("{source:?} should fail to resolve, got {r:?}"),
        }
    }

    /// (kind, line, col) of an expected parse error.
    fn parse_error(source: &str, input: &ArgInput) -> (ParseErrorKind, u32, u32) {
        match run_err(source, input) {
            Error::Parse { location, kind } => (kind, location.line, location.col),
            other => panic!("expected a parse error for {source:?}, got {other:?}"),
        }
    }

    fn first_source(recipe: &Recipe) -> &SourceInstruction {
        recipe
            .instructions
            .iter()
            .find_map(|i| match i {
                Instruction::Source(source) => Some(source),
                Instruction::Run(_) => None,
            })
            .unwrap_or_else(|| panic!("expected a SOURCE instruction in {recipe:?}"))
    }

    fn runs(recipe: &Recipe) -> Vec<&RunInstruction> {
        recipe
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Run(run) => Some(run),
                Instruction::Source(_) => None,
            })
            .collect()
    }

    fn chain_names(chain: &[TransformRef]) -> Vec<&str> {
        chain.iter().map(|t| t.name.as_str()).collect()
    }

    /// Test 39 (C1 and C2 rows): every substitutable argument substitutes.
    #[test]
    fn substitutes_in_every_target() {
        let source = "ARG repo\nARG sub\nARG set\nARG fmt\n\
                      COPY_DEFAULT_WITH [enrichment-injection --template-set ${set}]\n\
                      SOURCE ${repo}:\n  COPY ${sub}/ ${repo}/${sub}/ AS ${repo}-${sub}\n\
                      RUN pack --format ${fmt} --only ${repo}/** ${sub}/*.rs";
        let recipe = run(
            source,
            &bound(&[
                ("repo", "repo-a"),
                ("sub", "lib"),
                ("set", "vendor"),
                ("fmt", "xml"),
            ]),
        );
        let src = first_source(&recipe);
        assert_eq!(src.repo_name, "repo-a", "SOURCE repo substituted");
        let copy = &src.copies[0];
        assert_eq!(copy.src, "lib/", "COPY src substituted");
        assert_eq!(copy.dest, "repo-a/lib/", "COPY dest substituted");
        assert_eq!(copy.key, "repo-a-lib", "COPY key substituted");
        assert_eq!(
            copy.forward_chain[0]
                .args
                .get("template-set")
                .map(String::as_str),
            Some("vendor"),
            "chain flag value substituted"
        );
        let run = runs(&recipe)[0];
        assert_eq!(
            run.transform.args.get("format").map(String::as_str),
            Some("xml"),
            "RUN flag value substituted"
        );
        assert_eq!(
            run.scope,
            vec!["repo-a/**".to_string(), "lib/*.rs".to_string()],
            "--only globs substituted"
        );
        assert!(
            !run.transform.args.contains_key("only"),
            "--only never appears in the transform's args"
        );
    }

    /// Test 40: ARG defaults may reference earlier ARGs, and are substituted eagerly.
    #[test]
    fn arg_default_references_earlier_arg() {
        let source = "ARG repo=r\nARG out=${repo}-out\nSOURCE ${repo}:\n  COPY . ${out}/ AS k";
        let recipe = run(source, &ArgInput::Open);
        assert_eq!(
            first_source(&recipe).copies[0].dest,
            "r-out/",
            "default chain resolves"
        );
        assert_eq!(
            recipe.args[1].default.as_deref(),
            Some("${repo}-out"),
            "Arg.default shows the default as written"
        );
        assert!(
            !recipe.args[1].required,
            "an ARG with a default is optional"
        );
        assert!(recipe.unbound.is_empty(), "nothing is left symbolic");
    }

    /// Test 41: use before declaration, self-reference, and duplicates.
    #[test]
    fn arg_declaration_errors() {
        let undeclared = |name: &str| ParseErrorKind::UndeclaredArg { name: name.into() };
        let cases = [
            ("ARG b=${a}\nARG a", undeclared("a"), 1, 7),
            ("ARG a=${a}", undeclared("a"), 1, 7),
            ("SOURCE ${r}:\n  COPY . x/ AS k", undeclared("r"), 1, 8),
            (
                "ARG a\nARG a",
                ParseErrorKind::DuplicateArg { name: "a".into() },
                2,
                1,
            ),
        ];
        for (source, kind, line, col) in cases {
            for input in [ArgInput::Open, bound(&[("a", "1")])] {
                assert_eq!(
                    parse_error(source, &input),
                    (kind.clone(), line, col),
                    "declaration error in {source:?} with {input:?}"
                );
            }
        }
    }

    /// Test 42 (C1 rows): open mode keeps required ARGs symbolic and checks defaults as final.
    #[test]
    fn open_mode_keeps_unbound_symbolic_and_checks_defaults() {
        let recipe = run(
            "ARG repo\nSOURCE ${repo}:\n  COPY . ${repo}/ AS all-files",
            &ArgInput::Open,
        );
        let src = first_source(&recipe);
        assert_eq!(src.repo_name, "${repo}", "symbolic repo name");
        assert_eq!(src.copies[0].src, "", "root src is the empty prefix");
        assert_eq!(
            src.copies[0].dest, "${repo}/",
            "symbolic dest keeps its slash"
        );
        assert_eq!(src.copies[0].key, "all-files", "literal key");
        assert_eq!(recipe.unbound, vec!["repo".to_string()], "repo is unbound");
        assert!(recipe.args[0].required, "repo has no default");

        assert_eq!(
            parse_error(
                "ARG src=\nSOURCE r:\n  COPY ${src} x/ AS k",
                &ArgInput::Open
            ),
            (
                ParseErrorKind::InvalidPath {
                    path: String::new(),
                    issue: PathIssue::Empty
                },
                3,
                8
            ),
            "an invalid default is rejected at save time"
        );
        assert_eq!(
            parse_error(
                "ARG repo\nSOURCE r:\n  COPY . ${repo}/../x AS k",
                &ArgInput::Open
            ),
            (
                ParseErrorKind::InvalidPath {
                    path: "${repo}/../x".into(),
                    issue: PathIssue::ParentSegment
                },
                3,
                10
            ),
            "a literal .. inside a symbolic dest still errors"
        );
    }

    /// Test 43: bound mode requires every ARG without a default.
    #[test]
    fn bound_mode_missing_required_errors() {
        assert_eq!(
            parse_error("ARG repo\nARG x=1", &bound(&[])),
            (
                ParseErrorKind::MissingRequiredArg {
                    name: "repo".into()
                },
                1,
                1
            ),
            "required ARG with no value"
        );
    }

    /// Test 44: a bound value overrides the default.
    #[test]
    fn bound_mode_value_overrides_default() {
        let recipe = run(
            "ARG out=a\nSOURCE r:\n  COPY . ${out}/ AS k",
            &bound(&[("out", "b")]),
        );
        assert_eq!(
            first_source(&recipe).copies[0].dest,
            "b/",
            "provided value wins"
        );
        assert!(
            recipe.unbound.is_empty(),
            "bound mode leaves nothing symbolic"
        );
    }

    /// Test 45: bound mode rejects values for ARGs the recipe never declares.
    #[test]
    fn bound_mode_unknown_arg_errors() {
        match run_err("ARG a", &bound(&[("a", "1"), ("zz", "2")])) {
            Error::UnknownArg { name } => assert_eq!(name, "zz", "the undeclared name"),
            other => panic!("expected UnknownArg, got {other:?}"),
        }
    }

    /// Test 46 (C1 rows): values that open mode could not see are checked once bound.
    #[test]
    fn bound_mode_revalidates_substituted_values() {
        let cases = [
            (
                "ARG v\nSOURCE ${v}:\n  COPY . x/ AS k",
                "",
                ParseErrorKind::InvalidRepoName,
                2,
                8,
            ),
            (
                "ARG v\nSOURCE r:\n  COPY . ${v} AS k",
                "../x",
                ParseErrorKind::InvalidPath {
                    path: "../x".into(),
                    issue: PathIssue::ParentSegment,
                },
                3,
                10,
            ),
            (
                "ARG v\nSOURCE r:\n  COPY . ${v} AS k",
                "a\nb",
                ParseErrorKind::InvalidPath {
                    path: "a\nb".into(),
                    issue: PathIssue::ControlChar,
                },
                3,
                10,
            ),
            (
                "ARG v\nSOURCE r:\n  COPY . x/ AS ${v}",
                "",
                ParseErrorKind::InvalidKey {
                    key: String::new(),
                    reason: "key is empty",
                },
                3,
                16,
            ),
        ];
        for (source, value, kind, line, col) in cases {
            assert!(
                open_ok(source),
                "{source:?} is valid in open mode while v is symbolic"
            );
            assert_eq!(
                parse_error(source, &bound(&[("v", value)])),
                (kind, line, col),
                "{source:?} with v = {value:?}"
            );
        }
    }

    /// Test 52: keys collide after substitution, in both modes.
    #[test]
    fn substituted_keys_collide_after_substitution() {
        let same_var = "ARG repo\nSOURCE ${repo}:\n  COPY a/ ${repo}/a/ AS ${repo}-f\n  COPY b/ ${repo}/b/ AS ${repo}-f";
        for (input, key) in [
            (ArgInput::Open, "${repo}-f"),
            (bound(&[("repo", "x")]), "x-f"),
        ] {
            assert_eq!(
                parse_error(same_var, &input),
                (
                    ParseErrorKind::DuplicateKey {
                        key: key.into(),
                        first: Location::root(Position::new(3, 25)),
                    },
                    4,
                    25
                ),
                "same symbolic key twice collides with {input:?}"
            );
        }

        let two_vars = "ARG a\nARG b\nSOURCE r:\n  COPY x/ x/ AS ${a}\n  COPY y/ y/ AS ${b}";
        assert!(
            open_ok(two_vars),
            "different symbolic keys do not collide in open mode"
        );
        assert_eq!(
            parse_error(two_vars, &bound(&[("a", "k"), ("b", "k")])),
            (
                ParseErrorKind::DuplicateKey {
                    key: "k".into(),
                    first: Location::root(Position::new(4, 17)),
                },
                5,
                17
            ),
            "bound mode catches collisions open mode cannot see"
        );
    }

    /// Test 53: literal invalid paths are rejected at the offending word.
    #[test]
    fn dest_invalid_paths_error() {
        let cases = [
            (
                "SOURCE r:\n  COPY . ../out/ AS k",
                "../out/",
                PathIssue::ParentSegment,
                10,
            ),
            (
                "SOURCE r:\n  COPY . /abs AS k",
                "/abs",
                PathIssue::Absolute,
                10,
            ),
            (
                "SOURCE r:\n  COPY . a\\b AS k",
                "a\\b",
                PathIssue::Backslash,
                10,
            ),
            (
                "SOURCE r:\n  COPY C:x out/ AS k",
                "C:x",
                PathIssue::Absolute,
                8,
            ),
            ("SOURCE r:\n  COPY \"\" out/ AS k", "", PathIssue::Empty, 8),
        ];
        for (source, path, issue, col) in cases {
            assert_eq!(
                parse_error(source, &ArgInput::Open),
                (
                    ParseErrorKind::InvalidPath {
                        path: path.into(),
                        issue
                    },
                    2,
                    col
                ),
                "invalid path in {source:?}"
            );
        }
    }

    /// Test 47 (C2 rows): COPY_DEFAULT_WITH is sequential; absent means empty.
    #[test]
    fn chain_selection_default_override_and_empty() {
        let source = "SOURCE r:\n  COPY a/ a/ AS a\n\
                      COPY_DEFAULT_WITH [enrichment-injection --template-set default]\n\
                      SOURCE r:\n  COPY b/ b/ AS b\n\
                      COPY_DEFAULT_WITH [enrichment-trim]\n\
                      SOURCE r:\n  COPY c/ c/ AS c\n\
                      COPY_DEFAULT_WITH []\n\
                      SOURCE r:\n  COPY d/ d/ AS d";
        let recipe = run(source, &ArgInput::Open);
        let chains: Vec<(String, Vec<&str>)> = recipe
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Source(s) => Some(s),
                Instruction::Run(_) => None,
            })
            .flat_map(|s| s.copies.iter())
            .map(|c| (c.key.clone(), chain_names(&c.forward_chain)))
            .collect();
        assert_eq!(
            chains,
            vec![
                ("a".to_string(), vec![]),
                ("b".to_string(), vec!["enrichment-injection"]),
                ("c".to_string(), vec!["enrichment-trim"]),
                ("d".to_string(), vec![]),
            ],
            "each COPY takes the default in effect at its position"
        );
        let single = run(
            "COPY_DEFAULT_WITH [enrichment-injection --template-set default]\nSOURCE r:\n  COPY . x/ AS k",
            &ArgInput::Open,
        );
        let first = &first_source(&single).copies[0].forward_chain[0];
        assert_eq!(
            (first.transform_id, first.version_id, first.version),
            (3, 3, 1),
            "the chain element resolves to the catalog row"
        );
    }

    /// Test 48 (C2 rows): COPY chains need file transforms; RUN needs a directory transform.
    #[test]
    fn scope_mismatch_errors() {
        let wrong = |name: &str, expected, found| ParseErrorKind::WrongScope {
            name: name.into(),
            expected,
            found,
        };
        let cases = [
            (
                "COPY_DEFAULT_WITH [flatten]",
                wrong("flatten", TransformScope::File, TransformScope::Directory),
                1,
                20,
            ),
            (
                "RUN enrichment-injection",
                wrong(
                    "enrichment-injection",
                    TransformScope::Directory,
                    TransformScope::File,
                ),
                1,
                1,
            ),
        ];
        for (source, kind, line, col) in cases {
            assert_eq!(
                parse_error(source, &ArgInput::Open),
                (kind, line, col),
                "scope mismatch in {source:?}"
            );
        }
    }

    /// Test 49: an unknown transform is an error in a chain and on RUN, pinned or not.
    #[test]
    fn unknown_transform_errors() {
        let unknown = ParseErrorKind::UnknownTransform {
            name: "nope".into(),
        };
        for (source, col) in [
            ("COPY_DEFAULT_WITH [nope]", 20),
            ("RUN nope", 1),
            ("RUN nope@1", 1),
        ] {
            assert_eq!(
                parse_error(source, &ArgInput::Open),
                (unknown.clone(), 1, col),
                "unknown transform in {source:?}"
            );
        }
    }

    /// Test 50: RUN @N pins a version; unpinned follows current; a missing version errors.
    #[test]
    fn transform_pin_resolves_and_missing_pin_errors() {
        let catalog = MemCatalog::builtins().with_versions(
            "pack",
            TransformScope::Directory,
            2,
            &[(1, 20), (2, 21)],
        );
        let version_of = |source: &str| {
            let recipe = run_with(source, &ArgInput::Open, &catalog);
            let t = &runs(&recipe)[0].transform;
            (t.version, t.version_id)
        };
        assert_eq!(version_of("RUN pack"), (2, 21), "unpinned follows current");
        assert_eq!(version_of("RUN pack@1"), (1, 20), "pinned to version 1");
        match run_err_with("RUN pack@9", &ArgInput::Open, &catalog) {
            Error::Parse { location, kind } => assert_eq!(
                (kind, location.line, location.col),
                (
                    ParseErrorKind::TransformVersionNotFound {
                        name: "pack".into(),
                        version: 9
                    },
                    1,
                    1
                ),
                "missing pinned version"
            ),
            other => panic!("expected a parse error, got {other:?}"),
        }
    }
}
