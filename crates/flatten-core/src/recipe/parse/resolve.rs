// crates/flatten-core/src/recipe/parse/resolve.rs
//
// Resolve: Ast + ARG input + Catalog -> Recipe.
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
// EXCLUDE patterns are validated with the gitignore matcher once they are
// final. In open mode a pattern that still holds `${` is skipped (bound mode
// re-checks it): literal `${` is inexpressible in recipe text, so `${` in
// open-mode output always means a variable is still symbolic. A pattern that
// matches nothing (blank, or a `#` comment to gitignore) is rejected.
//
// INVOKE expands another recipe version in place, in its own frame: its ARGs
// bind from the INVOKE line, then the caller's same-named ARG, then its own
// default; its ARGs and COPY_DEFAULT_WITH never leak either way. Expansion
// walks a path of (recipe, version) keys with the root first: revisiting a
// key on the path is a cycle, the path is capped at 100 INVOKE levels, and
// the whole expansion at 10,000 SOURCE, COPY, and RUN instructions. Errors
// point at the innermost recipe, with the INVOKE chain that led there.
//
// WATCH: the root's DEPTH_TOLERANCE sets the recipe's (default 2); an
// invoked recipe's is ignored with L006 (ADR-041). OVERRIDE entries from
// every recipe merge into one map after expansion: the entry closest to the
// root wins a key (ties: first expanded), and each replaced invoked entry is
// an L007 warning. Every winning key must name a COPY block; open mode skips
// that check while the key, or any COPY key, is still symbolic.
//
// Every instruction is implemented (plan chunk C5).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use ignore::gitignore::GitignoreBuilder;

use super::ast::{Ast, ChainElem, CopyAst, ExcludeAst, Flag, Item, OverrideAst, Segment, Word};
use super::path::{canonical_copy_shape, check_key, normalize_rel_path};
use crate::recipe::catalog::{Catalog, RecipeSource, TransformInfo, TransformScope};
use crate::recipe::error::{Error, InvokeSite, Location, ParseErrorKind, Result, SourceRef};
use crate::recipe::lint::{LintCode, LintWarning};
use crate::recipe::types::{
    Arg, CopyBlock, Exclude, Instruction, InvokedVersion, Position, Recipe, RunInstruction,
    SourceInstruction, TransformRef,
};

/// INVOKE nesting limit (levels below the root).
const MAX_INVOKE_DEPTH: usize = 100;
/// Expanded SOURCE + COPY + RUN instruction limit.
const MAX_INSTRUCTIONS: usize = 10_000;

/// ARG values for resolution.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgInput {
    /// Save-time, show, and lint: required ARGs without a value stay symbolic.
    Open,
    /// Export: the caller's merged values (recipe default, then binding
    /// `arg_values`, then `--arg`); resolve sees one map.
    Bound(BTreeMap<String, String>),
}

/// Identity of the recipe being analyzed. It is the first entry on the
/// expansion path, so an INVOKE back to it is a cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootRef {
    /// Text not yet stored (add, edit, lint of a file). With a name,
    /// `analyze` answers unpinned lookups of that name with this text.
    Pending {
        /// The recipe name the text will be saved under, if any.
        name: Option<String>,
    },
    /// A stored version (show, rollback check, export).
    Stored {
        /// `build_recipes.id`.
        recipe_id: i64,
        /// `build_recipe_versions.id`.
        version_id: i64,
        /// The recipe name.
        name: String,
        /// The version number.
        version: u32,
    },
}

/// The result of `analyze`.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolution {
    /// The resolved recipe.
    pub recipe: Recipe,
    /// Warnings, sorted: root first, then invoked recipes in
    /// `invoked_versions` order, then by position and code.
    pub warnings: Vec<LintWarning>,
}

/// A WATCH OVERRIDE entry waiting for the post-expansion merge.
struct PendingOverride {
    key: String,
    chain: Vec<TransformRef>,
    /// For UnknownOverrideKey.
    location: Location,
    /// For L007.
    position: Position,
    /// `name@version` for invoked recipes; `None` for the root.
    recipe: Option<String>,
    /// Expansion path length when the entry was read: 1 for the root.
    depth: usize,
}

/// A (recipe, version) on the expansion path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ExpansionKey {
    /// Unsaved text (the pending root, or the overlay's answer for its name).
    Pending,
    Stored {
        recipe_id: i64,
        version_id: i64,
    },
}

/// How an invoked frame's ARGs get their values (binding steps 1 and 2).
struct Binding {
    /// INVOKE `name=value` assignments, substituted in the caller's frame.
    explicit: HashMap<String, String>,
    /// The caller's ARG values at the INVOKE line.
    caller_env: HashMap<String, String>,
    /// The caller, for messages.
    caller: String,
    /// The invoked recipe, `name@version`, for messages.
    invoked: String,
    /// The INVOKE line, in the caller's frame.
    invoke_loc: Location,
}

/// One recipe's resolution state: ARG values, the default chain, and where
/// its positions and errors point.
// SPEC-DEVIATION(EX-001): the spec's frame also holds a `symbolic` flag per
// value and a `declared` list. Neither is needed. Open-mode symbolic
// detection scans for `${` (see the module header; decided in C3). INVOKE
// binding step 2 reads the caller's `env`, which holds exactly the ARGs
// declared by that point, and step 3 walks the invoked recipe's own ARG items
// in file order (decided in C4). `recipe.args` carries the root's ARG order.
struct Frame {
    env: HashMap<String, String>,
    /// The COPY_DEFAULT_WITH in effect: empty until declared; a later
    /// declaration replaces it for the COPY blocks after it.
    default_chain: Vec<TransformRef>,
    /// Which recipe text this frame's positions refer to.
    source: SourceRef,
    /// INVOKE sites from the root down to this frame.
    via: Vec<InvokeSite>,
    /// Stamped on output positions; `None` for the root.
    version_id: Option<i64>,
    /// This recipe in messages and in INVOKE sites below it: the root's
    /// name (or `<input>`), or `name` of an invoked recipe.
    name: String,
    /// The version shown in INVOKE sites below it; `None` for the root.
    version: Option<u32>,
    /// `None` for the root, whose ARGs come from `ArgInput`.
    binding: Option<Binding>,
}

impl Frame {
    fn root(name: String) -> Self {
        Frame {
            env: HashMap::new(),
            default_chain: Vec::new(),
            source: SourceRef::Root,
            via: Vec::new(),
            version_id: None,
            name,
            version: None,
            binding: None,
        }
    }

    /// `name@version` for invoked frames; the plain name for the root.
    fn label(&self) -> String {
        match self.version {
            Some(version) => format!("{}@{version}", self.name),
            None => self.name.clone(),
        }
    }

    /// A location in this frame's recipe text.
    fn loc(&self, pos: Position) -> Location {
        Location {
            line: pos.line,
            col: pos.col,
            source: self.source.clone(),
            via: self.via.clone(),
        }
    }

    /// A parse error at a position in this frame's recipe text.
    fn err(&self, pos: Position, kind: ParseErrorKind) -> Error {
        Error::Parse {
            location: Box::new(self.loc(pos)),
            kind,
        }
    }

    /// An output position, tagged with this frame's recipe version.
    fn stamp(&self, pos: Position) -> Position {
        Position {
            recipe_version_id: self.version_id,
            ..pos
        }
    }
}

/// Resolve a parsed recipe.
pub(crate) fn resolve(
    ast: &Ast,
    input: &ArgInput,
    catalog: &dyn Catalog,
    root: &RootRef,
) -> Result<(Recipe, Vec<LintWarning>)> {
    let (root_key, root_label, root_name) = match root {
        RootRef::Pending { name: Some(name) } => (
            ExpansionKey::Pending,
            format!("{name}@pending"),
            name.clone(),
        ),
        RootRef::Pending { name: None } => (
            ExpansionKey::Pending,
            "<input>".to_string(),
            "<input>".to_string(),
        ),
        RootRef::Stored {
            recipe_id,
            version_id,
            name,
            version,
        } => (
            ExpansionKey::Stored {
                recipe_id: *recipe_id,
                version_id: *version_id,
            },
            format!("{name}@{version}"),
            name.clone(),
        ),
    };

    let mut expander = Expander {
        input,
        catalog,
        recipe: Recipe {
            args: Vec::new(),
            instructions: Vec::new(),
            watch_config: Default::default(),
            invoked_versions: Vec::new(),
            unbound: Vec::new(),
        },
        keys: HashMap::new(),
        path: vec![(root_key, root_label)],
        cache: HashMap::new(),
        emitted: 0,
        overrides: Vec::new(),
        warnings: Vec::new(),
    };
    let mut frame = Frame::root(root_name);
    expander.expand(ast, &mut frame)?;

    if let ArgInput::Bound(values) = input
        && let Some(name) = values.keys().find(|name| !frame.env.contains_key(*name))
    {
        return Err(Error::UnknownArg { name: name.clone() });
    }
    expander.merge_overrides()?;
    Ok((expander.recipe, expander.warnings))
}

/// Whole-expansion state shared by every frame.
struct Expander<'a> {
    input: &'a ArgInput,
    catalog: &'a dyn Catalog,
    recipe: Recipe,
    /// COPY key -> where it was first defined, across every frame.
    keys: HashMap<String, Location>,
    /// The expansion path: keys with their labels for cycle messages.
    path: Vec<(ExpansionKey, String)>,
    /// Parsed invoked versions, so each is parsed once.
    cache: HashMap<ExpansionKey, Rc<Ast>>,
    /// SOURCE + COPY + RUN instructions emitted so far.
    emitted: usize,
    /// WATCH OVERRIDE entries from every frame, in expansion order.
    overrides: Vec<PendingOverride>,
    /// Resolver warnings (L006, L007).
    warnings: Vec<LintWarning>,
}

impl Expander<'_> {
    /// Run one recipe's items in file order.
    fn expand(&mut self, ast: &Ast, frame: &mut Frame) -> Result<()> {
        for item in &ast.items {
            match item {
                Item::Arg { name, default, pos } => {
                    self.arg(name, default.as_ref(), *pos, frame)?
                }
                Item::CopyDefaultWith { chain, .. } => {
                    frame.default_chain = self.resolve_chain(chain, frame)?;
                }
                Item::Run {
                    name,
                    pin,
                    flags,
                    only,
                    pos,
                } => {
                    self.count(1, *pos, frame)?;
                    let info = self.lookup(name, *pin, *pos, frame)?;
                    require_scope(&info, TransformScope::Directory, *pos, frame)?;
                    let mut scope = Vec::with_capacity(only.len());
                    for glob in only {
                        scope.push(substitute(glob, frame)?);
                    }
                    let transform = transform_ref(info, flags, frame)?;
                    self.recipe
                        .instructions
                        .push(Instruction::Run(RunInstruction {
                            transform,
                            scope,
                            position: frame.stamp(*pos),
                        }));
                }
                Item::Source { repo, copies, pos } => {
                    self.count(1 + copies.len(), *pos, frame)?;
                    let repo_name = substitute(repo, frame)?;
                    if repo_name.is_empty() {
                        return Err(frame.err(repo.pos, ParseErrorKind::InvalidRepoName));
                    }
                    let mut blocks = Vec::with_capacity(copies.len());
                    for copy in copies {
                        blocks.push(self.copy_block(copy, frame)?);
                    }
                    self.recipe
                        .instructions
                        .push(Instruction::Source(SourceInstruction {
                            repo_name,
                            copies: blocks,
                            position: frame.stamp(*pos),
                        }));
                }
                Item::Invoke {
                    name,
                    pin,
                    assigns,
                    pos,
                } => self.invoke(name, *pin, assigns, *pos, frame)?,
                Item::Watch {
                    depth, overrides, ..
                } => self.watch(*depth, overrides, frame)?,
            }
        }
        Ok(())
    }

    /// `WATCH:`: apply the root's DEPTH_TOLERANCE (an invoked one is L006)
    /// and collect OVERRIDE entries for the merge after expansion.
    fn watch(
        &mut self,
        depth: Option<(u32, Position)>,
        overrides: &[OverrideAst],
        frame: &Frame,
    ) -> Result<()> {
        let invoked = frame.binding.is_some().then(|| frame.label());
        if let Some((value, pos)) = depth {
            match &invoked {
                None => self.recipe.watch_config.depth_tolerance = value,
                Some(label) => self.warnings.push(LintWarning {
                    code: LintCode::L006,
                    message: format!(
                        "DEPTH_TOLERANCE {value} in invoked recipe {label} is ignored; \
                         only the root recipe's WATCH sets it"
                    ),
                    position: frame.stamp(pos),
                    recipe: Some(label.clone()),
                }),
            }
        }

        let mut seen = HashSet::new();
        for entry in overrides {
            let key = substitute(&entry.key, frame)?;
            if !seen.insert(key.clone()) {
                return Err(frame.err(
                    entry.pos,
                    ParseErrorKind::Duplicate {
                        what: format!("OVERRIDE key {key}"),
                    },
                ));
            }
            let chain = self.resolve_chain(&entry.chain, frame)?;
            self.overrides.push(PendingOverride {
                key,
                chain,
                location: frame.loc(entry.pos),
                position: frame.stamp(entry.pos),
                recipe: invoked.clone(),
                depth: self.path.len(),
            });
        }
        Ok(())
    }

    /// Merge OVERRIDE entries into the WATCH config after expansion. The
    /// entry closest to the root wins a key (ties: first expanded); each
    /// replaced entry is L007. Every winning key must name a COPY block,
    /// except in open mode while the key or any COPY key is symbolic.
    fn merge_overrides(&mut self) -> Result<()> {
        let mut entries = std::mem::take(&mut self.overrides);
        entries.sort_by_key(|e| e.depth);
        let open = matches!(self.input, ArgInput::Open);
        let symbolic_copy_key = open && self.keys.keys().any(|k| k.contains("${"));

        for entry in entries {
            if self.recipe.watch_config.overrides.contains_key(&entry.key) {
                self.warnings.push(LintWarning {
                    code: LintCode::L007,
                    message: format!(
                        "WATCH OVERRIDE for {} is replaced by an entry closer to the root recipe",
                        entry.key
                    ),
                    position: entry.position,
                    recipe: entry.recipe,
                });
                continue;
            }
            let symbolic = symbolic_copy_key || (open && entry.key.contains("${"));
            if !symbolic && !self.keys.contains_key(&entry.key) {
                return Err(Error::Parse {
                    location: Box::new(entry.location),
                    kind: ParseErrorKind::UnknownOverrideKey { key: entry.key },
                });
            }
            self.recipe
                .watch_config
                .overrides
                .insert(entry.key, entry.chain);
        }
        Ok(())
    }

    /// `ARG name[=default]`: bind from `ArgInput` (root) or the INVOKE
    /// binding (invoked frames).
    fn arg(
        &mut self,
        name: &str,
        default: Option<&Word>,
        pos: Position,
        frame: &mut Frame,
    ) -> Result<()> {
        if frame.env.contains_key(name) {
            return Err(frame.err(
                pos,
                ParseErrorKind::DuplicateArg {
                    name: name.to_string(),
                },
            ));
        }
        // SPEC-DEVIATION(EX-001): the default is substituted even when a
        // bound value overrides it, so an undeclared reference in a default
        // fails in both modes. That keeps the open-mode invariant: open mode
        // never rejects what bound mode accepts. The spec wording follows at
        // reconcile.
        let default_value = default.map(|word| substitute(word, frame)).transpose()?;

        let value = match &frame.binding {
            Some(binding) => binding
                .explicit
                .get(name)
                .or_else(|| binding.caller_env.get(name))
                .cloned()
                .or(default_value)
                .ok_or_else(|| Error::Parse {
                    location: Box::new(binding.invoke_loc.clone()),
                    kind: ParseErrorKind::UnboundInvokedArg {
                        arg: name.to_string(),
                        invoked: binding.invoked.clone(),
                        caller: binding.caller.clone(),
                        arg_pos: frame.stamp(pos),
                    },
                })?,
            None => match (self.input, default_value) {
                (ArgInput::Bound(values), default_value) => match (values.get(name), default_value)
                {
                    (Some(value), _) => value.clone(),
                    (None, Some(default_value)) => default_value,
                    (None, None) => {
                        return Err(frame.err(
                            pos,
                            ParseErrorKind::MissingRequiredArg {
                                name: name.to_string(),
                            },
                        ));
                    }
                },
                (ArgInput::Open, Some(default_value)) => default_value,
                (ArgInput::Open, None) => {
                    self.recipe.unbound.push(name.to_string());
                    format!("${{{name}}}")
                }
            },
        };
        frame.env.insert(name.to_string(), value);

        if frame.binding.is_none() {
            self.recipe.args.push(Arg {
                name: name.to_string(),
                default: default.map(Word::display),
                required: default.is_none(),
                position: pos,
            });
        }
        Ok(())
    }

    /// Count expanded instructions against the budget.
    fn count(&mut self, n: usize, pos: Position, frame: &Frame) -> Result<()> {
        self.emitted += n;
        if self.emitted > MAX_INSTRUCTIONS {
            return Err(frame.err(pos, ParseErrorKind::ExpansionTooLarge));
        }
        Ok(())
    }

    /// `INVOKE <recipe>[@N] [name=value ...]`: expand the target in place.
    fn invoke(
        &mut self,
        name: &str,
        pin: Option<u32>,
        assigns: &[(String, Word, Position)],
        pos: Position,
        frame: &Frame,
    ) -> Result<()> {
        let target = self.lookup_recipe(name, pin, pos, frame)?;
        let (key, label) = match (target.recipe_id, target.version_id, target.version) {
            (Some(recipe_id), Some(version_id), Some(version)) => (
                ExpansionKey::Stored {
                    recipe_id,
                    version_id,
                },
                format!("{}@{version}", target.name),
            ),
            _ => (ExpansionKey::Pending, format!("{}@pending", target.name)),
        };
        if self.path.iter().any(|(k, _)| *k == key) {
            let mut chain: Vec<String> = self.path.iter().map(|(_, l)| l.clone()).collect();
            chain.push(label);
            return Err(frame.err(pos, ParseErrorKind::InvokeCycle { chain }));
        }
        // Only the pending root (or the overlay's answer for its name) lacks a
        // version, and that key is always on the path, so it is caught above.
        let (Some(version_id), Some(version)) = (target.version_id, target.version) else {
            let mut chain: Vec<String> = self.path.iter().map(|(_, l)| l.clone()).collect();
            chain.push(label);
            return Err(frame.err(pos, ParseErrorKind::InvokeCycle { chain }));
        };
        if self.path.len() > MAX_INVOKE_DEPTH {
            return Err(frame.err(pos, ParseErrorKind::InvokeDepthExceeded));
        }

        let source = SourceRef::Invoked {
            name: target.name.clone(),
            version,
        };
        let mut via = frame.via.clone();
        via.push(InvokeSite {
            recipe: frame.name.clone(),
            version: frame.version,
            line: pos.line,
            col: pos.col,
        });
        let ast = self.parsed(&key, &target, &source, &via)?;

        let declared: HashSet<&str> = ast
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Arg { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        let mut explicit = HashMap::with_capacity(assigns.len());
        for (arg, word, arg_pos) in assigns {
            if !declared.contains(arg.as_str()) {
                return Err(frame.err(
                    *arg_pos,
                    ParseErrorKind::UnknownInvokeArg {
                        arg: arg.clone(),
                        invoked: label,
                    },
                ));
            }
            explicit.insert(arg.clone(), substitute(word, frame)?);
        }

        if !self
            .recipe
            .invoked_versions
            .iter()
            .any(|v| v.version_id == version_id)
        {
            self.recipe.invoked_versions.push(InvokedVersion {
                version_id,
                name: target.name.clone(),
                version,
            });
        }

        let mut child = Frame {
            env: HashMap::new(),
            default_chain: Vec::new(),
            source,
            via,
            version_id: Some(version_id),
            name: target.name.clone(),
            version: Some(version),
            binding: Some(Binding {
                explicit,
                caller_env: frame.env.clone(),
                caller: frame.label(),
                invoked: label.clone(),
                invoke_loc: frame.loc(pos),
            }),
        };
        self.path.push((key, label));
        let result = self.expand(&ast, &mut child);
        self.path.pop();
        result
    }

    /// Parse an invoked version once. Its parse errors point into its own
    /// text, with the INVOKE chain that reached it.
    fn parsed(
        &mut self,
        key: &ExpansionKey,
        target: &RecipeSource,
        source: &SourceRef,
        via: &[InvokeSite],
    ) -> Result<Rc<Ast>> {
        if let Some(ast) = self.cache.get(key) {
            return Ok(Rc::clone(ast));
        }
        let ast = super::parse(&target.source).map_err(|e| match e {
            Error::Parse { location, kind } if location.source == SourceRef::Root => Error::Parse {
                location: Box::new(Location {
                    source: source.clone(),
                    via: via.to_vec(),
                    ..*location
                }),
                kind,
            },
            other => other,
        })?;
        let ast = Rc::new(ast);
        self.cache.insert(key.clone(), Rc::clone(&ast));
        Ok(ast)
    }

    /// Look up an INVOKE target, current or pinned; the requested form is
    /// tried first, then a miss is classified (an unknown name wins).
    fn lookup_recipe(
        &self,
        name: &str,
        pin: Option<u32>,
        pos: Position,
        frame: &Frame,
    ) -> Result<RecipeSource> {
        if let Some(source) = self.catalog.recipe(name, pin)? {
            return Ok(source);
        }
        let kind = match pin {
            Some(version) if self.catalog.recipe(name, None)?.is_some() => {
                ParseErrorKind::RecipeVersionNotFound {
                    name: name.to_string(),
                    version,
                }
            }
            _ => ParseErrorKind::UnknownRecipe {
                name: name.to_string(),
            },
        };
        Err(frame.err(pos, kind))
    }

    /// Resolve one COPY block: substitute, normalize, canonicalize, check the key.
    fn copy_block(&mut self, copy: &CopyAst, frame: &Frame) -> Result<CopyBlock> {
        let src = substituted_path(&copy.src, frame)?;
        let dest = substituted_path(&copy.dest, frame)?;
        let (src, dest) = canonical_copy_shape(src, dest);

        let key = substitute(&copy.key, frame)?;
        // SPEC-DEVIATION(EX-001): the spec says open mode checks only the
        // literal parts of a symbolic key. This checks the full substituted
        // text, which is equivalent (symbolic `${name}` text is never empty
        // and has no control characters) and also accepts a fully symbolic
        // key such as `AS ${k}`, which a literal-parts-only check would
        // wrongly reject as empty.
        if let Err(reason) = check_key(&key) {
            return Err(frame.err(copy.key.pos, ParseErrorKind::InvalidKey { key, reason }));
        }
        if let Some(first) = self.keys.get(&key) {
            return Err(frame.err(
                copy.key.pos,
                ParseErrorKind::DuplicateKey {
                    key,
                    first: first.clone(),
                },
            ));
        }
        self.keys.insert(key.clone(), frame.loc(copy.key.pos));

        let mut excludes = Vec::with_capacity(copy.excludes.len());
        for entry in &copy.excludes {
            excludes.push(match entry {
                ExcludeAst::Binary => Exclude::Binary,
                ExcludeAst::Pattern(word) => {
                    Exclude::Pattern(exclude_pattern(word, frame, self.input)?)
                }
            });
        }

        let forward_chain = match &copy.override_with {
            Some(chain) => self.resolve_chain(chain, frame)?,
            None => frame.default_chain.clone(),
        };

        Ok(CopyBlock {
            src,
            dest,
            key,
            excludes,
            forward_chain,
            position: frame.stamp(copy.pos),
        })
    }

    /// Resolve a COPY chain: every element must be a file transform.
    fn resolve_chain(&self, chain: &[ChainElem], frame: &Frame) -> Result<Vec<TransformRef>> {
        let mut out = Vec::with_capacity(chain.len());
        for elem in chain {
            let info = self.lookup(&elem.name, None, elem.pos, frame)?;
            require_scope(&info, TransformScope::File, elem.pos, frame)?;
            out.push(transform_ref(info, &elem.flags, frame)?);
        }
        Ok(out)
    }

    /// Look up a transform, current or pinned. The requested form is tried
    /// first, so a pinned lookup succeeds even when the transform's current
    /// version is unavailable. A miss is then classified: a pinned miss on a
    /// name the catalog knows is TransformVersionNotFound; anything else is
    /// UnknownTransform (an unknown name wins over a missing version).
    fn lookup(
        &self,
        name: &str,
        pin: Option<u32>,
        pos: Position,
        frame: &Frame,
    ) -> Result<TransformInfo> {
        if let Some(info) = self.catalog.transform(name, pin)? {
            return Ok(info);
        }
        let kind = match pin {
            Some(version) if self.catalog.transform(name, None)?.is_some() => {
                ParseErrorKind::TransformVersionNotFound {
                    name: name.to_string(),
                    version,
                }
            }
            _ => ParseErrorKind::UnknownTransform {
                name: name.to_string(),
            },
        };
        Err(frame.err(pos, kind))
    }
}

/// Substitute an EXCLUDE pattern and validate it: it must match something,
/// and, unless open mode leaves it symbolic, the gitignore matcher must
/// accept it.
fn exclude_pattern(word: &Word, frame: &Frame, input: &ArgInput) -> Result<String> {
    let pattern = substitute(word, frame)?;
    if pattern.trim().is_empty() || pattern.starts_with('#') {
        return Err(frame.err(
            word.pos,
            ParseErrorKind::InvalidPattern {
                pattern,
                reason: "matches nothing: gitignore reads it as a blank line or a comment \
                         (write \\#name for a literal #)"
                    .to_string(),
            },
        ));
    }
    let symbolic = matches!(input, ArgInput::Open) && pattern.contains("${");
    if !symbolic && let Err(e) = GitignoreBuilder::new("").add_line(None, &pattern) {
        return Err(frame.err(
            word.pos,
            ParseErrorKind::InvalidPattern {
                pattern,
                reason: e.to_string(),
            },
        ));
    }
    Ok(pattern)
}

fn require_scope(
    info: &TransformInfo,
    expected: TransformScope,
    pos: Position,
    frame: &Frame,
) -> Result<()> {
    if info.scope != expected {
        return Err(frame.err(
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
        .map_err(|issue| frame.err(word.pos, ParseErrorKind::InvalidPath { path, issue }))
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
                    return Err(
                        frame.err(*pos, ParseErrorKind::UndeclaredArg { name: name.clone() })
                    );
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
    use crate::recipe::error::{Error, InvokeSite, Location, ParseErrorKind, PathIssue, SourceRef};
    use crate::recipe::lint::{LintCode, LintWarning};
    use crate::recipe::types::{
        CopyBlock, Exclude, Instruction, InvokedVersion, Position, Recipe, RunInstruction,
        SourceInstruction, TransformRef,
    };
    use crate::recipe::{ArgInput, Resolution, RootRef, analyze};

    /// Unsaved text with no recipe name (lint of a file).
    const INPUT: RootRef = RootRef::Pending { name: None };

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
        run_root(source, input, catalog, &INPUT)
    }

    fn run_root(source: &str, input: &ArgInput, catalog: &MemCatalog, root: &RootRef) -> Recipe {
        analyze(source, input, catalog, root)
            .unwrap_or_else(|e| panic!("{source:?} should resolve, got {e}"))
            .recipe
    }

    fn open_ok(source: &str) -> bool {
        analyze(source, &ArgInput::Open, &MemCatalog::builtins(), &INPUT).is_ok()
    }

    /// Expect a parse error; return its kind and full location.
    fn located(
        result: crate::recipe::Result<Resolution>,
        what: &str,
    ) -> (ParseErrorKind, Location) {
        match result {
            Err(Error::Parse { location, kind }) => (kind, *location),
            Err(other) => panic!("{what}: expected a parse error, got {other:?}"),
            Ok(r) => panic!("{what}: should fail to resolve, got {r:?}"),
        }
    }

    fn invoked(name: &str, version: u32) -> SourceRef {
        SourceRef::Invoked {
            name: name.into(),
            version,
        }
    }

    fn site(recipe: &str, version: Option<u32>, line: u32, col: u32) -> InvokeSite {
        InvokeSite {
            recipe: recipe.into(),
            version,
            line,
            col,
        }
    }

    fn at(line: u32, col: u32, source: SourceRef, via: Vec<InvokeSite>) -> Location {
        Location {
            line,
            col,
            source,
            via,
        }
    }

    fn all_copies(recipe: &Recipe) -> Vec<&CopyBlock> {
        recipe
            .instructions
            .iter()
            .filter_map(|i| match i {
                Instruction::Source(s) => Some(s.copies.iter()),
                Instruction::Run(_) => None,
            })
            .flatten()
            .collect()
    }

    fn run_err(source: &str, input: &ArgInput) -> Error {
        run_err_with(source, input, &MemCatalog::builtins())
    }

    fn run_err_with(source: &str, input: &ArgInput, catalog: &MemCatalog) -> Error {
        match analyze(source, input, catalog, &INPUT) {
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

    /// Test 39 (C1 to C5 rows): every substitutable argument substitutes.
    #[test]
    fn substitutes_in_every_target() {
        let source = "ARG repo\nARG sub\nARG set\nARG fmt\n\
                      COPY_DEFAULT_WITH [enrichment-injection --template-set ${set}]\n\
                      SOURCE ${repo}:\n  COPY ${sub}/ ${repo}/${sub}/ AS ${repo}-${sub}:\n\
                      \x20   EXCLUDE ${sub}/*.tmp\n\
                      RUN pack --format ${fmt} --only ${repo}/** ${sub}/*.rs\n\
                      WATCH:\n  OVERRIDE:\n    ${repo}-${sub} [enrichment-trim]";
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
            copy.excludes,
            vec![Exclude::Pattern("lib/*.tmp".into())],
            "EXCLUDE pattern substituted"
        );
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
        assert!(
            recipe.watch_config.overrides.contains_key("repo-a-lib"),
            "OVERRIDE key substituted, got {:?}",
            recipe.watch_config.overrides.keys().collect::<Vec<_>>()
        );

        let catalog = MemCatalog::builtins().with_recipe(
            "base",
            1,
            1,
            &[(1, 11, "ARG v\nSOURCE r:\n  COPY . ${v}/ AS inv")],
        );
        let invoked = run_with(
            "ARG sub\nINVOKE base v=${sub}",
            &bound(&[("sub", "lib")]),
            &catalog,
        );
        assert_eq!(
            first_source(&invoked).copies[0].dest,
            "lib/",
            "INVOKE value substituted in the caller's frame"
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

    /// Test 42 (C1 and C3 rows): open mode keeps required ARGs symbolic and checks defaults as final.
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

        // `{${p}` is not a valid glob as literal text (an unclosed `{`), so
        // passing here shows open mode skips validating a symbolic pattern.
        let skipped = run(
            "ARG p\nSOURCE r:\n  COPY . x/ AS k:\n    EXCLUDE {${p}",
            &ArgInput::Open,
        );
        assert_eq!(
            first_source(&skipped).copies[0].excludes,
            vec![Exclude::Pattern("{${p}".into())],
            "a symbolic EXCLUDE pattern is recorded unvalidated in open mode"
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

    /// Test 46 (C1 and C3 rows): values that open mode could not see are checked once bound.
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

        // EXCLUDE: the matcher's reason text is not asserted (it belongs to
        // the ignore crate); the variant, pattern, and position are.
        let excludes = [
            (
                "ARG v\nSOURCE r:\n  COPY . x/ AS k:\n    EXCLUDE ${v}",
                "[z-a]",
                "[z-a]",
            ),
            (
                "ARG v\nSOURCE r:\n  COPY . x/ AS k:\n    EXCLUDE {${v}",
                "a",
                "{a",
            ),
            (
                "ARG v\nSOURCE r:\n  COPY . x/ AS k:\n    EXCLUDE ${v}",
                "",
                "",
            ),
        ];
        for (source, value, pattern) in excludes {
            assert!(
                open_ok(source),
                "{source:?} is valid in open mode while v is symbolic"
            );
            match run_err(source, &bound(&[("v", value)])) {
                Error::Parse { location, kind } => {
                    assert!(
                        matches!(&kind, ParseErrorKind::InvalidPattern { pattern: p, .. } if p == pattern),
                        "{source:?} with v = {value:?}: expected InvalidPattern for {pattern:?}, got {kind:?}"
                    );
                    assert_eq!(
                        (location.line, location.col),
                        (4, 13),
                        "the error points at the pattern word"
                    );
                }
                other => panic!("expected a parse error, got {other:?}"),
            }
        }
        let valid_once_bound = run(
            "ARG v\nSOURCE r:\n  COPY . x/ AS k:\n    EXCLUDE {${v}",
            &bound(&[("v", "a,b}")]),
        );
        assert_eq!(
            first_source(&valid_once_bound).copies[0].excludes,
            vec![Exclude::Pattern("{a,b}".into())],
            "the same symbolic pattern passes validation once its value completes it"
        );
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

    /// Test 54: literal EXCLUDE patterns the matcher rejects, or that match nothing.
    #[test]
    fn invalid_exclude_pattern_errors() {
        let cases = [
            ("EXCLUDE [z-a]", "[z-a]", 13),
            ("EXCLUDE ok/ {a,b", "{a,b", 17),
            ("EXCLUDE \"a\\\\\"", "a\\", 13),
            ("EXCLUDE \"\"", "", 13),
            ("EXCLUDE \"#c\"", "#c", 13),
            ("EXCLUDE \"   \"", "   ", 13),
        ];
        for (member, pattern, col) in cases {
            let source = format!("SOURCE r:\n  COPY . x/ AS k:\n    {member}");
            for input in [ArgInput::Open, bound(&[])] {
                match run_err(&source, &input) {
                    Error::Parse { location, kind } => {
                        assert!(
                            matches!(&kind, ParseErrorKind::InvalidPattern { pattern: p, reason } if p == pattern && !reason.is_empty()),
                            "{member:?}: expected InvalidPattern for {pattern:?} with a reason, got {kind:?}"
                        );
                        assert_eq!(
                            (location.line, location.col),
                            (3, col),
                            "{member:?}: the error points at the pattern word"
                        );
                    }
                    other => panic!("{member:?}: expected a parse error, got {other:?}"),
                }
            }
        }

        let escaped = run(
            "SOURCE r:\n  COPY . x/ AS k:\n    EXCLUDE \\#c",
            &ArgInput::Open,
        );
        assert_eq!(
            first_source(&escaped).copies[0].excludes,
            vec![Exclude::Pattern("\\#c".into())],
            "an escaped # names a file and is accepted"
        );
    }

    /// Test 47 (C2 and C3 rows): COPY_DEFAULT_WITH is sequential; OVERRIDE_WITH replaces it per COPY.
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

        let overrides = run(
            "COPY_DEFAULT_WITH [enrichment-injection --template-set default]\n\
             SOURCE r:\n\
             \x20 COPY a/ a/ AS a:\n    OVERRIDE_WITH [enrichment-trim]\n\
             \x20 COPY b/ b/ AS b:\n    OVERRIDE_WITH []\n\
             \x20 COPY c/ c/ AS c",
            &ArgInput::Open,
        );
        let chains: Vec<(String, Vec<&str>)> = first_source(&overrides)
            .copies
            .iter()
            .map(|c| (c.key.clone(), chain_names(&c.forward_chain)))
            .collect();
        assert_eq!(
            chains,
            vec![
                ("a".to_string(), vec!["enrichment-trim"]),
                ("b".to_string(), vec![]),
                ("c".to_string(), vec!["enrichment-injection"]),
            ],
            "OVERRIDE_WITH replaces the default for its COPY only; [] means no transforms"
        );
    }

    /// Test 48 (C2, C3, and C5 rows): COPY and OVERRIDE chains need file transforms; RUN needs a directory transform.
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
            (
                "SOURCE r:\n  COPY . x/ AS k:\n    OVERRIDE_WITH [pack]",
                wrong("pack", TransformScope::File, TransformScope::Directory),
                3,
                20,
            ),
            (
                "SOURCE r:\n  COPY . x/ AS k\nWATCH:\n  OVERRIDE:\n    k [pack]",
                wrong("pack", TransformScope::File, TransformScope::Directory),
                5,
                8,
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

        // The current version is unavailable (3 is not among the versions):
        // a pinned lookup still resolves; an unpinned one finds nothing.
        let no_current = MemCatalog::builtins().with_versions(
            "pack",
            TransformScope::Directory,
            3,
            &[(1, 20), (2, 21)],
        );
        let pinned = run_with("RUN pack@1", &ArgInput::Open, &no_current);
        assert_eq!(
            runs(&pinned)[0].transform.version,
            1,
            "a pin survives an unavailable current version"
        );
        match run_err_with("RUN pack", &ArgInput::Open, &no_current) {
            Error::Parse { kind, .. } => assert_eq!(
                kind,
                ParseErrorKind::UnknownTransform {
                    name: "pack".into()
                },
                "no resolvable current version reads as unknown when unpinned"
            ),
            other => panic!("expected a parse error, got {other:?}"),
        }
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

    /// The keys of every COPY, in expansion order.
    fn keys_of(recipe: &Recipe) -> Vec<String> {
        all_copies(recipe).iter().map(|c| c.key.clone()).collect()
    }

    /// Test 51: duplicate keys report both locations, across recipes too.
    #[test]
    fn duplicate_key_errors_with_both_locations() {
        let catalog = MemCatalog::builtins().with_recipe(
            "base",
            1,
            1,
            &[(1, 11, "SOURCE r:\n  COPY . b/ AS k")],
        );
        let dup = |first: Location| ParseErrorKind::DuplicateKey {
            key: "k".into(),
            first,
        };
        let root_2_16 = at(2, 16, SourceRef::Root, vec![]);
        let base_2_16 = |invoke_line| {
            at(
                2,
                16,
                invoked("base", 1),
                vec![site("<input>", None, invoke_line, 1)],
            )
        };
        let cases = [
            (
                "SOURCE r:\n  COPY . a/ AS k\n  COPY . c/ AS k",
                dup(root_2_16.clone()),
                at(3, 16, SourceRef::Root, vec![]),
            ),
            (
                "SOURCE r:\n  COPY . a/ AS k\nINVOKE base",
                dup(root_2_16.clone()),
                base_2_16(3),
            ),
            (
                "INVOKE base\nSOURCE r:\n  COPY . a/ AS k",
                dup(base_2_16(1)),
                at(3, 16, SourceRef::Root, vec![]),
            ),
        ];
        for (source, kind, location) in cases {
            assert_eq!(
                located(analyze(source, &ArgInput::Open, &catalog, &INPUT), source),
                (kind, location),
                "duplicate key in {source:?}"
            );
        }
    }

    /// Test 55: INVOKE binds explicit values, then the caller's ARG, then the default.
    #[test]
    fn invoke_binding_order() {
        let catalog = MemCatalog::builtins().with_recipe(
            "base",
            1,
            1,
            &[(
                1,
                11,
                "ARG a=da\nARG b=db\nARG c=dc\nSOURCE r:\n  COPY . ${a}-${b}-${c}/ AS k",
            )],
        );
        let cases = [
            ("ARG b=rb\nINVOKE base a=ea", "ea-rb-dc/"),
            ("ARG a=ra\nINVOKE base a=ea", "ea-db-dc/"),
            ("ARG a=ra\nINVOKE base", "ra-db-dc/"),
            ("ARG x=X\nINVOKE base a=${x}", "X-db-dc/"),
            ("INVOKE base", "da-db-dc/"),
            ("INVOKE base\nARG a=ra", "da-db-dc/"),
        ];
        for (source, dest) in cases {
            let recipe = run_with(source, &ArgInput::Open, &catalog);
            assert_eq!(
                first_source(&recipe).copies[0].dest,
                dest,
                "binding order in {source:?}"
            );
        }
        let symbolic = run_with("ARG b\nINVOKE base", &ArgInput::Open, &catalog);
        assert_eq!(
            first_source(&symbolic).copies[0].dest,
            "da-${b}-dc/",
            "a symbolic caller value propagates into the invoked recipe"
        );
    }

    /// Test 56: INVOKE target and argument errors.
    #[test]
    fn invoke_resolution_errors() {
        let catalog = MemCatalog::builtins()
            .with_recipe(
                "base",
                1,
                1,
                &[(1, 11, "ARG a=1\nSOURCE r:\n  COPY . x/ AS k")],
            )
            .with_recipe(
                "req",
                2,
                1,
                &[(
                    1,
                    21,
                    "ARG repo\nSOURCE ${repo}:\n  COPY . ${repo}/ AS ${repo}-k",
                )],
            );
        let root = |line, col| at(line, col, SourceRef::Root, vec![]);
        let unbound = |caller: &str| ParseErrorKind::UnboundInvokedArg {
            arg: "repo".into(),
            invoked: "req@1".into(),
            caller: caller.into(),
            arg_pos: Position {
                line: 1,
                col: 1,
                recipe_version_id: Some(21),
            },
        };
        let named_top = RootRef::Pending {
            name: Some("top".into()),
        };
        let cases = [
            (
                "INVOKE nope",
                &INPUT,
                ParseErrorKind::UnknownRecipe {
                    name: "nope".into(),
                },
                root(1, 1),
            ),
            (
                "INVOKE nope@1",
                &INPUT,
                ParseErrorKind::UnknownRecipe {
                    name: "nope".into(),
                },
                root(1, 1),
            ),
            (
                "INVOKE base@9",
                &INPUT,
                ParseErrorKind::RecipeVersionNotFound {
                    name: "base".into(),
                    version: 9,
                },
                root(1, 1),
            ),
            (
                "SOURCE r:\n  COPY . y/ AS j\nINVOKE req",
                &INPUT,
                unbound("<input>"),
                root(3, 1),
            ),
            ("INVOKE req", &named_top, unbound("top"), root(1, 1)),
            (
                "INVOKE base zz=1",
                &INPUT,
                ParseErrorKind::UnknownInvokeArg {
                    arg: "zz".into(),
                    invoked: "base@1".into(),
                },
                root(1, 13),
            ),
        ];
        for (source, root_ref, kind, location) in cases {
            assert_eq!(
                located(analyze(source, &ArgInput::Open, &catalog, root_ref), source),
                (kind, location),
                "INVOKE error in {source:?}"
            );
        }
    }

    /// Test 57: an invoked recipe's ARGs are not visible to the caller.
    #[test]
    fn invoke_args_are_local() {
        let catalog = MemCatalog::builtins().with_recipe("base", 1, 1, &[(1, 11, "ARG inner=x")]);
        let source = "INVOKE base\nSOURCE r:\n  COPY . ${inner}/ AS k";
        assert_eq!(
            located(analyze(source, &ArgInput::Open, &catalog, &INPUT), source),
            (
                ParseErrorKind::UndeclaredArg {
                    name: "inner".into()
                },
                at(3, 10, SourceRef::Root, vec![])
            ),
            "the caller cannot read an invoked ARG"
        );
    }

    /// Test 58: COPY_DEFAULT_WITH does not flow into or out of an invoked recipe.
    #[test]
    fn invoke_default_chain_not_inherited_either_way() {
        let catalog = MemCatalog::builtins().with_recipe(
            "base",
            1,
            1,
            &[(
                1,
                11,
                "SOURCE r:\n  COPY . b1/ AS b1\nCOPY_DEFAULT_WITH [enrichment-trim]\nSOURCE r:\n  COPY . b2/ AS b2",
            )],
        );
        let recipe = run_with(
            "COPY_DEFAULT_WITH [enrichment-injection --template-set default]\nINVOKE base\nSOURCE r:\n  COPY . r/ AS r",
            &ArgInput::Open,
            &catalog,
        );
        let chains: Vec<(String, Vec<&str>)> = all_copies(&recipe)
            .iter()
            .map(|c| (c.key.clone(), chain_names(&c.forward_chain)))
            .collect();
        assert_eq!(
            chains,
            vec![
                ("b1".to_string(), vec![]),
                ("b2".to_string(), vec!["enrichment-trim"]),
                ("r".to_string(), vec!["enrichment-injection"]),
            ],
            "each recipe keeps its own default chain"
        );
    }

    /// Test 59: invoked versions are recorded once each, nested ones included.
    #[test]
    fn invoke_records_invoked_versions_including_nested() {
        let catalog = MemCatalog::builtins()
            .with_recipe("a", 1, 1, &[(1, 11, "INVOKE b")])
            .with_recipe("b", 2, 3, &[(3, 23, "")]);
        let recipe = run_with("INVOKE a\nINVOKE a", &ArgInput::Open, &catalog);
        assert_eq!(
            recipe.invoked_versions,
            vec![
                InvokedVersion {
                    version_id: 11,
                    name: "a".into(),
                    version: 1
                },
                InvokedVersion {
                    version_id: 23,
                    name: "b".into(),
                    version: 3
                },
            ],
            "first-seen order, no duplicates"
        );
    }

    /// Test 60: INVOKE of the recipe being saved is a cycle (pending overlay).
    #[test]
    fn invoke_self_via_pending_overlay_is_cycle() {
        let cyc = RootRef::Pending {
            name: Some("cyc".into()),
        };
        let cycle = ParseErrorKind::InvokeCycle {
            chain: vec!["cyc@pending".into(), "cyc@pending".into()],
        };
        let new_recipe = MemCatalog::builtins();
        let existing = MemCatalog::builtins().with_recipe("cyc", 1, 1, &[(1, 11, "")]);
        for (what, catalog) in [("add", &new_recipe), ("edit", &existing)] {
            assert_eq!(
                located(analyze("INVOKE cyc", &ArgInput::Open, catalog, &cyc), what),
                (cycle.clone(), at(1, 1, SourceRef::Root, vec![])),
                "{what}: a self-INVOKE is the cycle export would see"
            );
        }
        let pinned = run_root("INVOKE cyc@1", &ArgInput::Open, &existing, &cyc);
        assert_eq!(
            pinned.invoked_versions.len(),
            1,
            "a pinned self-INVOKE expands the stored version, not the pending text"
        );
    }

    /// Test 61: a mutual cycle names the path, root first.
    #[test]
    fn invoke_mutual_cycle_errors_naming_chain() {
        let catalog = MemCatalog::builtins()
            .with_recipe("a", 1, 2, &[(2, 12, "INVOKE b")])
            .with_recipe("b", 2, 1, &[(1, 21, "INVOKE a")]);
        let root = RootRef::Stored {
            recipe_id: 1,
            version_id: 12,
            name: "a".into(),
            version: 2,
        };
        assert_eq!(
            located(
                analyze("INVOKE b", &ArgInput::Open, &catalog, &root),
                "a <-> b"
            ),
            (
                ParseErrorKind::InvokeCycle {
                    chain: vec!["a@2".into(), "b@1".into(), "a@2".into()],
                },
                at(1, 1, invoked("b", 1), vec![site("a", None, 1, 1)])
            ),
            "the cycle is reported at the INVOKE that closes it"
        );
    }

    /// Test 62: other versions of a recipe, and repeated INVOKEs, are not cycles.
    #[test]
    fn non_cycles_are_legal() {
        let versions =
            MemCatalog::builtins().with_recipe("a", 1, 2, &[(1, 11, ""), (2, 12, "INVOKE a@1")]);
        let root = RootRef::Stored {
            recipe_id: 1,
            version_id: 12,
            name: "a".into(),
            version: 2,
        };
        let recipe = run_root("INVOKE a@1", &ArgInput::Open, &versions, &root);
        assert_eq!(
            recipe.invoked_versions[0].version_id, 11,
            "a@2 may invoke a@1"
        );

        let base = MemCatalog::builtins().with_recipe(
            "base",
            1,
            1,
            &[(
                1,
                11,
                "ARG repo\nSOURCE ${repo}:\n  COPY . ${repo}/ AS ${repo}-files",
            )],
        );
        let twice = run_with(
            "INVOKE base repo=x\nINVOKE base repo=y",
            &ArgInput::Open,
            &base,
        );
        assert_eq!(
            keys_of(&twice),
            vec!["x-files".to_string(), "y-files".to_string()],
            "the same recipe twice in sequence, with substituted keys"
        );
    }

    /// Test 63: INVOKE depth is capped at 100 levels; expansion at 10,000 instructions.
    #[test]
    fn expansion_limits_error() {
        let chain = |levels: usize| {
            let mut catalog = MemCatalog::builtins();
            for i in 1..=levels {
                let source = if i < levels {
                    format!("INVOKE r{}", i + 1)
                } else {
                    String::new()
                };
                catalog = catalog.with_recipe(
                    &format!("r{i}"),
                    i as i64,
                    1,
                    &[(1, 1000 + i as i64, &source)],
                );
            }
            catalog
        };
        assert!(
            analyze("INVOKE r1", &ArgInput::Open, &chain(100), &INPUT).is_ok(),
            "100 nested INVOKE levels are allowed"
        );
        let (kind, location) = located(
            analyze("INVOKE r1", &ArgInput::Open, &chain(101), &INPUT),
            "depth 101",
        );
        assert_eq!(
            kind,
            ParseErrorKind::InvokeDepthExceeded,
            "the 101st level is rejected"
        );
        assert_eq!(
            location.source,
            invoked("r100", 1),
            "reported at the INVOKE in r100"
        );

        // Fan-out: each level invokes the one below twice, so l14 expands to
        // 2^14 RUN lines. The per-resolve parse cache keeps this fast.
        let mut fan = MemCatalog::builtins().with_recipe("l0", 100, 1, &[(1, 200, "RUN flatten")]);
        for k in 1..=14 {
            let below = format!("INVOKE l{}\nINVOKE l{}", k - 1, k - 1);
            fan = fan.with_recipe(&format!("l{k}"), 100 + k, 1, &[(1, 200 + k, &below)]);
        }
        let (kind, location) = located(
            analyze("INVOKE l14", &ArgInput::Open, &fan, &INPUT),
            "fan-out",
        );
        assert_eq!(
            kind,
            ParseErrorKind::ExpansionTooLarge,
            "fan-out past 10,000 instructions is rejected"
        );
        assert_eq!(
            (location.source, location.via.len()),
            (invoked("l0", 1), 15),
            "reported at the RUN that crossed the limit, inside l0, via the root and l14 to l1"
        );

        // Exact boundary: ten recipes of 1,000 RUN lines are 10,000
        // instructions and pass; one more RUN in the root fails there.
        let thousand = vec!["RUN flatten"; 1000].join("\n");
        let mut exact = MemCatalog::builtins();
        for b in 1..=10 {
            exact = exact.with_recipe(&format!("b{b}"), 500 + b, 1, &[(1, 600 + b, &thousand)]);
        }
        let ten: String = (1..=10).map(|b| format!("INVOKE b{b}\n")).collect();
        assert!(
            analyze(&ten, &ArgInput::Open, &exact, &INPUT).is_ok(),
            "exactly 10,000 instructions are allowed"
        );
        let (kind, location) = located(
            analyze(
                &format!("{ten}RUN flatten"),
                &ArgInput::Open,
                &exact,
                &INPUT,
            ),
            "10,001",
        );
        assert_eq!(
            (kind, location),
            (
                ParseErrorKind::ExpansionTooLarge,
                at(11, 1, SourceRef::Root, vec![])
            ),
            "the 10,001st instruction is rejected where it is emitted"
        );
    }

    /// Test 64: errors inside invoked recipes point there, with the INVOKE chain.
    #[test]
    fn error_inside_invoked_reports_invoked_location_and_via() {
        let catalog = MemCatalog::builtins()
            .with_recipe("mid", 1, 1, &[(1, 11, "ARG x=1\nINVOKE leaf")])
            .with_recipe("leaf", 2, 1, &[(1, 21, "COPIE . x/ AS k")])
            .with_recipe("mid2", 3, 1, &[(1, 31, "INVOKE leaf2")])
            .with_recipe(
                "leaf2",
                4,
                2,
                &[(2, 42, "SOURCE ${nope}:\n  COPY . x/ AS k")],
            );
        let top = RootRef::Pending {
            name: Some("top".into()),
        };
        assert_eq!(
            located(
                analyze("ARG a\n\nINVOKE mid", &ArgInput::Open, &catalog, &top),
                "parse error"
            ),
            (
                ParseErrorKind::UnknownInstruction("COPIE".into()),
                at(
                    1,
                    1,
                    invoked("leaf", 1),
                    vec![site("top", None, 3, 1), site("mid", Some(1), 2, 1)]
                )
            ),
            "a parse error in an invoked recipe points into its own text"
        );
        assert_eq!(
            located(
                analyze("INVOKE mid2", &ArgInput::Open, &catalog, &top),
                "resolve error"
            ),
            (
                ParseErrorKind::UndeclaredArg {
                    name: "nope".into()
                },
                at(
                    1,
                    8,
                    invoked("leaf2", 2),
                    vec![site("top", None, 1, 1), site("mid2", Some(1), 1, 1)]
                )
            ),
            "a resolve error in an invoked recipe points there too"
        );
    }

    /// Test 67: positions from an invoked recipe carry its version id.
    #[test]
    fn positions_carry_origin_version_id() {
        let catalog = MemCatalog::builtins().with_recipe(
            "base",
            1,
            1,
            &[(1, 11, "SOURCE r:\n  COPY . b/ AS b\nRUN flatten")],
        );
        let recipe = run_with(
            "SOURCE r:\n  COPY . a/ AS a\nINVOKE base",
            &ArgInput::Open,
            &catalog,
        );
        let origins: Vec<Option<i64>> = recipe
            .instructions
            .iter()
            .flat_map(|i| match i {
                Instruction::Source(s) => {
                    let mut v = vec![s.position.recipe_version_id];
                    v.extend(s.copies.iter().map(|c| c.position.recipe_version_id));
                    v
                }
                Instruction::Run(r) => vec![r.position.recipe_version_id],
            })
            .collect();
        assert_eq!(
            origins,
            vec![None, None, Some(11), Some(11), Some(11)],
            "root positions have no version id; invoked ones carry base's"
        );
    }

    /// Test 65: WATCH OVERRIDE keys must name a COPY block, checked after
    /// expansion; open mode skips the check while a key is symbolic.
    #[test]
    fn watch_override_unknown_key_rules() {
        let unknown = |key: &str| ParseErrorKind::UnknownOverrideKey { key: key.into() };

        let literal = "SOURCE r:\n  COPY . x/ AS a\nWATCH:\n  OVERRIDE:\n    b [enrichment-trim]";
        for input in [ArgInput::Open, bound(&[])] {
            assert_eq!(
                parse_error(literal, &input),
                (unknown("b"), 5, 5),
                "a literal unknown key errors in {input:?}"
            );
        }

        let watch_first = run(
            "WATCH:\n  OVERRIDE:\n    a [enrichment-trim]\nSOURCE r:\n  COPY . x/ AS a",
            &ArgInput::Open,
        );
        assert_eq!(
            watch_first
                .watch_config
                .overrides
                .get("a")
                .map(|c| chain_names(c)),
            Some(vec!["enrichment-trim"]),
            "WATCH may precede the COPY it names"
        );
        assert_eq!(
            watch_first.watch_config.depth_tolerance, 2,
            "DEPTH_TOLERANCE defaults to 2"
        );

        let symbolic_copy =
            "ARG k\nSOURCE r:\n  COPY . x/ AS ${k}\nWATCH:\n  OVERRIDE:\n    lit [enrichment-trim]";
        assert!(
            open_ok(symbolic_copy),
            "open mode skips the check while a COPY key is symbolic"
        );
        run(symbolic_copy, &bound(&[("k", "lit")]));
        assert_eq!(
            parse_error(symbolic_copy, &bound(&[("k", "other")])),
            (unknown("lit"), 6, 5),
            "bound mode enforces the check"
        );

        let symbolic_key =
            "ARG k\nSOURCE r:\n  COPY . x/ AS a\nWATCH:\n  OVERRIDE:\n    ${k} [enrichment-trim]";
        assert!(
            open_ok(symbolic_key),
            "open mode skips the check while the OVERRIDE key is symbolic"
        );
        assert!(
            run(symbolic_key, &bound(&[("k", "a")]))
                .watch_config
                .overrides
                .contains_key("a"),
            "a substituted OVERRIDE key resolves"
        );
        assert_eq!(
            parse_error(symbolic_key, &bound(&[("k", "z")])),
            (unknown("z"), 6, 5),
            "a substituted unknown key errors once bound"
        );

        assert_eq!(
            run("WATCH:\n  DEPTH_TOLERANCE 0", &ArgInput::Open)
                .watch_config
                .depth_tolerance,
            0,
            "the root's DEPTH_TOLERANCE sets the value"
        );

        assert_eq!(
            parse_error(
                "ARG a=x\nARG b=x\nSOURCE r:\n  COPY . x/ AS x\nWATCH:\n  OVERRIDE:\n    ${a} []\n    ${b} []",
                &ArgInput::Open
            ),
            (
                ParseErrorKind::Duplicate {
                    what: "OVERRIDE key x".into()
                },
                8,
                5
            ),
            "keys that collide after substitution are duplicates"
        );
    }

    /// Test 66: an invoked recipe's DEPTH_TOLERANCE is ignored (L006); its
    /// OVERRIDE entries merge, and the entry closest to the root wins (L007).
    #[test]
    fn invoked_watch_depth_ignored_and_overrides_merged() {
        let catalog = MemCatalog::builtins()
            .with_recipe(
                "base",
                1,
                1,
                &[(
                    1,
                    11,
                    "SOURCE r:\n  COPY . b/ AS bk\n  COPY . c/ AS ck\nWATCH:\n  DEPTH_TOLERANCE 5\n  OVERRIDE:\n    bk [enrichment-trim]\n    ck [enrichment-trim]",
                )],
            )
            .with_recipe(
                "inner",
                2,
                1,
                &[(
                    1,
                    21,
                    "SOURCE r:\n  COPY . i/ AS ik\nWATCH:\n  OVERRIDE:\n    ik [enrichment-trim]",
                )],
            )
            .with_recipe(
                "outer",
                3,
                1,
                &[(1, 31, "INVOKE inner\nWATCH:\n  OVERRIDE:\n    ik []")],
            );
        let stamped = |line, col, id| Position {
            line,
            col,
            recipe_version_id: Some(id),
        };

        let resolution = analyze(
            "INVOKE base\nWATCH:\n  DEPTH_TOLERANCE 3\n  OVERRIDE:\n    ck []",
            &ArgInput::Open,
            &catalog,
            &INPUT,
        )
        .expect("resolves");
        let watch = &resolution.recipe.watch_config;
        assert_eq!(watch.depth_tolerance, 3, "the root's DEPTH_TOLERANCE wins");
        let merged: Vec<(&str, Vec<&str>)> = watch
            .overrides
            .iter()
            .map(|(k, c)| (k.as_str(), chain_names(c)))
            .collect();
        assert_eq!(
            merged,
            vec![("bk", vec!["enrichment-trim"]), ("ck", vec![])],
            "invoked entries merge; the root's entry wins its key"
        );
        // Only the resolver's codes: the lint pass adds L005 here (these
        // COPY blocks have no injection), which lint's own tests cover.
        let resolver_codes = |w: &&LintWarning| matches!(w.code, LintCode::L006 | LintCode::L007);
        let codes: Vec<(LintCode, Position, Option<&str>)> = resolution
            .warnings
            .iter()
            .filter(resolver_codes)
            .map(|w| (w.code, w.position, w.recipe.as_deref()))
            .collect();
        assert_eq!(
            codes,
            vec![
                (LintCode::L006, stamped(5, 3, 11), Some("base@1")),
                (LintCode::L007, stamped(8, 5, 11), Some("base@1")),
            ],
            "L006 at the ignored DEPTH_TOLERANCE, L007 at the replaced entry, sorted"
        );

        let nested = analyze("INVOKE outer", &ArgInput::Open, &catalog, &INPUT).expect("resolves");
        assert_eq!(
            nested
                .recipe
                .watch_config
                .overrides
                .get("ik")
                .map(|c| chain_names(c)),
            Some(vec![]),
            "the caller's entry wins even when its WATCH comes after the INVOKE"
        );
        let nested_resolver: Vec<LintWarning> = nested
            .warnings
            .iter()
            .filter(resolver_codes)
            .cloned()
            .collect();
        assert_eq!(
            nested_resolver,
            vec![LintWarning {
                code: LintCode::L007,
                message: "WATCH OVERRIDE for ik is replaced by an entry closer to the root recipe"
                    .into(),
                position: stamped(5, 5, 21),
                recipe: Some("inner@1".into()),
            }],
            "the replaced nested entry is L007"
        );
    }
}
