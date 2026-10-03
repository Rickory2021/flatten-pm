// crates/flatten-core/src/recipe/lint.rs
//
// Recipe warnings: codes, the warning type, their order, and the lint pass.
//
// Warnings never stop a save. The resolver emits two (L006, L007) while it
// expands INVOKE; `lint` adds L001 to L005 and L008 over the resolved
// recipe. `analyze` returns them merged and sorted: the root recipe first,
// then invoked recipes in `invoked_versions` order, then by position, then
// by code. Display goes through the same `fmt_origin` as error locations.
//
// Lint compares chains with what watch will actually run on return, by
// transform name only (docs/design/5_WATCH.md, Return Step): Step 2 trims
// the enrichment unless the injection is `--reversible=false`; Step 3 runs
// each other forward transform's reverser in reverse order. A WATCH
// OVERRIDE replaces both steps for its key.

use std::fmt;

use serde::Serialize;

use super::catalog::Catalog;
use super::error::{Result, fmt_origin};
use super::types::{CopyBlock, Instruction, Position, Recipe, RunInstruction, TransformRef};

/// A warning code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum LintCode {
    /// A forward transform has no reverse transform.
    L001,
    /// A WATCH OVERRIDE drops the enrichment trim the injection needs.
    L002,
    /// A WATCH OVERRIDE differs from the default return path.
    L003,
    /// A RUN (directory) transform declares a reverse.
    L004,
    /// A COPY chain has no enrichment-injection, so its files cannot
    /// round-trip.
    L005,
    /// An invoked recipe's DEPTH_TOLERANCE was ignored (ADR-041).
    L006,
    /// A caller's WATCH OVERRIDE replaced an invoked recipe's entry for the
    /// same key.
    L007,
    /// A forward transform has several reverse transforms.
    L008,
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
/// order, then by line, column, and code. Identical warnings collapse to one:
/// a recipe version invoked twice (with different ARGs) expands its WATCH
/// twice and would otherwise repeat the same L006 word for word.
pub(crate) fn sort_warnings(warnings: &mut Vec<LintWarning>, recipe: &Recipe) {
    let origin = |w: &LintWarning| match w.position.recipe_version_id {
        None => 0,
        Some(id) => recipe
            .invoked_versions
            .iter()
            .position(|v| v.version_id == id)
            .map_or(usize::MAX, |i| i + 1),
    };
    warnings.sort_by_key(|w| (origin(w), w.position.line, w.position.col, w.code));
    warnings.dedup();
}

// ---------------------------------------------------------------------------
// The lint pass
// ---------------------------------------------------------------------------

/// The injection and trim builtins the return path special-cases
/// (docs/design/5_WATCH.md, Return Step).
const INJECTION: &str = "enrichment-injection";
const TRIM: &str = "enrichment-trim";

/// What watch's default return path does for one forward transform, by name
/// only (EX-005 owns the recorded reverse chain's full shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReturnEntry {
    /// Return Step 2: strip the enrichment injection added.
    Trim,
    /// Return Step 3: the one transform that reverses a forward transform.
    Reverser(String),
    /// A forward transform nothing reverses; the return path cannot undo it.
    Skipped(String),
    /// A forward transform with several reversers; the choice is ambiguous.
    Ambiguous {
        /// The forward transform.
        forward: String,
        /// Every candidate reverser, by id.
        candidates: Vec<String>,
    },
}

/// Whether the forward chain has an injection that watch trims on return
/// (present, and not marked `--reversible=false`).
fn trimmed_injection(forward: &[TransformRef]) -> bool {
    forward.iter().any(|t| {
        t.name == INJECTION && t.args.get("reversible").map(String::as_str) != Some("false")
    })
}

/// The default return path for a COPY block with no WATCH OVERRIDE: Return
/// Step 2 (trim), then Step 3 (each other forward transform's reverser, in
/// reverse order).
pub(crate) fn default_return_path(
    forward: &[TransformRef],
    catalog: &dyn Catalog,
) -> Result<Vec<ReturnEntry>> {
    let mut path = Vec::new();
    if trimmed_injection(forward) {
        path.push(ReturnEntry::Trim);
    }
    for t in forward.iter().rev().filter(|t| t.name != INJECTION) {
        let reversers = catalog.reversers_of(&t.name)?;
        path.push(match reversers.as_slice() {
            [] => ReturnEntry::Skipped(t.name.clone()),
            [one] => ReturnEntry::Reverser(one.name.clone()),
            many => ReturnEntry::Ambiguous {
                forward: t.name.clone(),
                candidates: many.iter().map(|r| r.name.clone()).collect(),
            },
        });
    }
    Ok(path)
}

/// Lint a resolved recipe: L001 to L005 and L008. Resolver warnings (L006,
/// L007) come from resolve; `analyze` merges and sorts both.
pub(crate) fn lint(recipe: &Recipe, catalog: &dyn Catalog) -> Result<Vec<LintWarning>> {
    let mut linter = Linter {
        recipe,
        catalog,
        out: Vec::new(),
    };
    for instruction in &recipe.instructions {
        match instruction {
            Instruction::Run(run) => linter.run(run)?,
            Instruction::Source(source) => {
                for copy in &source.copies {
                    linter.copy(copy)?;
                }
            }
        }
    }
    Ok(linter.out)
}

/// Lint state: the recipe, the catalog, and the warnings so far.
struct Linter<'a> {
    recipe: &'a Recipe,
    catalog: &'a dyn Catalog,
    out: Vec<LintWarning>,
}

impl Linter<'_> {
    /// Record a warning; `recipe` is filled from the position's version id.
    fn warn(&mut self, code: LintCode, position: Position, message: String) {
        let recipe = position.recipe_version_id.and_then(|id| {
            self.recipe
                .invoked_versions
                .iter()
                .find(|v| v.version_id == id)
                .map(|v| format!("{}@{}", v.name, v.version))
        });
        self.out.push(LintWarning {
            code,
            message,
            position,
            recipe,
        });
    }

    /// L004: a RUN (directory) transform that declares a reverse.
    fn run(&mut self, run: &RunInstruction) -> Result<()> {
        let transform = &run.transform;
        let Some(info) = self
            .catalog
            .transform(&transform.name, Some(transform.version))?
        else {
            return Ok(());
        };
        if let Some(reverses) = &info.reverses {
            self.warn(
                LintCode::L004,
                run.position,
                format!(
                    "directory transform {} declares that it reverses {reverses}; \
                     directory transforms are one-way and never run in reverse",
                    info.name
                ),
            );
        }
        Ok(())
    }

    /// The COPY-level checks: L001, L002, L003, L005, L008.
    fn copy(&mut self, copy: &CopyBlock) -> Result<()> {
        let forward = &copy.forward_chain;
        let key = &copy.key;
        if !forward.iter().any(|t| t.name == INJECTION) {
            self.warn(
                LintCode::L005,
                copy.position,
                format!(
                    "COPY {key} has no enrichment-injection, so its files cannot be matched \
                     and will not round-trip through watch"
                ),
            );
        }

        let default_path = default_return_path(forward, self.catalog)?;
        let mut ambiguous = false;
        for entry in &default_path {
            match entry {
                ReturnEntry::Skipped(name) => self.warn(
                    LintCode::L001,
                    copy.position,
                    format!(
                        "transform {name} in COPY {key} has no reverse transform, so watch \
                         cannot undo it when a file returns"
                    ),
                ),
                ReturnEntry::Ambiguous {
                    forward,
                    candidates,
                } => {
                    ambiguous = true;
                    self.warn(
                        LintCode::L008,
                        copy.position,
                        format!(
                            "transform {forward} in COPY {key} has several reverse transforms \
                             ({}), so its return path is ambiguous",
                            candidates.join(", ")
                        ),
                    );
                }
                ReturnEntry::Trim | ReturnEntry::Reverser(_) => {}
            }
        }

        let Some(override_chain) = self.recipe.watch_config.overrides.get(key) else {
            return Ok(());
        };
        let override_names: Vec<&str> = override_chain.iter().map(|t| t.name.as_str()).collect();
        if trimmed_injection(forward) && !override_names.contains(&TRIM) {
            self.warn(
                LintCode::L002,
                copy.position,
                format!(
                    "WATCH OVERRIDE for {key} has no enrichment-trim; an override replaces \
                     the default return steps, so returned files keep their enrichment"
                ),
            );
        }
        if !ambiguous {
            let default_names: Vec<&str> = default_path
                .iter()
                .filter_map(|e| match e {
                    ReturnEntry::Trim => Some(TRIM),
                    ReturnEntry::Reverser(name) => Some(name.as_str()),
                    _ => None,
                })
                .collect();
            if override_names != default_names {
                self.warn(
                    LintCode::L003,
                    copy.position,
                    format!(
                        "WATCH OVERRIDE for {key} is [{}], but the default return path is [{}]",
                        override_names.join(", "),
                        default_names.join(", ")
                    ),
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::LintCode;
    use crate::recipe::catalog::{MemCatalog, TransformScope};
    use crate::recipe::{ArgInput, LintWarning, RootRef, SHIPPED_DEFAULT_RECIPE, analyze};

    const INPUT: RootRef = RootRef::Pending { name: None };

    /// Lint `source` (open mode) and return every warning.
    fn warnings(source: &str, catalog: &MemCatalog) -> Vec<LintWarning> {
        analyze(source, &ArgInput::Open, catalog, &INPUT)
            .unwrap_or_else(|e| panic!("{source:?} should resolve, got {e}"))
            .warnings
    }

    /// The warnings with one code, as (line, col, message).
    fn with_code(all: &[LintWarning], code: LintCode) -> Vec<(u32, u32, String)> {
        all.iter()
            .filter(|w| w.code == code)
            .map(|w| (w.position.line, w.position.col, w.message.clone()))
            .collect()
    }

    /// Builtins plus file transforms `upper` and `strip`, with `lower`
    /// reversing `upper`.
    fn catalog() -> MemCatalog {
        MemCatalog::builtins()
            .with_transform("upper", TransformScope::File, None)
            .with_transform("lower", TransformScope::File, Some("upper"))
            .with_transform("strip", TransformScope::File, None)
    }

    /// Test 73: the shipped recipe lints clean.
    #[test]
    fn shipped_default_is_clean() {
        assert_eq!(
            warnings(SHIPPED_DEFAULT_RECIPE, &MemCatalog::builtins()),
            vec![],
            "the shipped recipe has no warnings against the seeded builtins"
        );
    }

    /// Test 74: L001, a forward transform nothing reverses.
    #[test]
    fn non_reversible_transform_warns() {
        let all = warnings(
            "COPY_DEFAULT_WITH [strip, enrichment-injection]\nSOURCE r:\n  COPY . x/ AS k",
            &catalog(),
        );
        let l001 = with_code(&all, LintCode::L001);
        assert_eq!(l001.len(), 1, "one L001, for strip: {all:?}");
        assert_eq!((l001[0].0, l001[0].1), (3, 3), "at the COPY");
        assert!(
            l001[0].2.contains("strip"),
            "names the transform: {}",
            l001[0].2
        );
        assert!(
            with_code(&all, LintCode::L005).is_empty(),
            "the chain has an injection, so no L005"
        );
        assert!(
            with_code(
                &warnings(
                    "COPY_DEFAULT_WITH [upper, enrichment-injection]\nSOURCE r:\n  COPY . x/ AS k",
                    &catalog()
                ),
                LintCode::L001
            )
            .is_empty(),
            "a transform with a reverser is not L001"
        );
    }

    /// Test 75: L002, an override without the trim the injection needs;
    /// `--reversible=false` and the absence of an override suppress it.
    #[test]
    fn override_without_trim_warns_unless_irreversible() {
        let with_override = |chain: &str, override_chain: &str| {
            format!(
                "COPY_DEFAULT_WITH [{chain}]\nSOURCE r:\n  COPY . x/ AS k\nWATCH:\n  OVERRIDE:\n    k [{override_chain}]"
            )
        };
        let l002 = |source: &str| with_code(&warnings(source, &catalog()), LintCode::L002);

        let fired = l002(&with_override("enrichment-injection", ""));
        assert_eq!(fired.len(), 1, "an override with no trim is L002");
        assert_eq!((fired[0].0, fired[0].1), (3, 3), "at the COPY");
        assert!(
            l002(&with_override("enrichment-injection", "enrichment-trim")).is_empty(),
            "an override that keeps the trim is fine"
        );
        assert!(
            l002(&with_override(
                "enrichment-injection --reversible=false",
                ""
            ))
            .is_empty(),
            "an irreversible injection needs no trim"
        );
        assert!(
            l002("COPY_DEFAULT_WITH [enrichment-injection]\nSOURCE r:\n  COPY . x/ AS k")
                .is_empty(),
            "without an override, return step 2 trims"
        );

        let invoked = catalog().with_recipe(
            "base",
            1,
            1,
            &[(
                1,
                11,
                "COPY_DEFAULT_WITH [enrichment-injection]\nSOURCE r:\n  COPY . b/ AS bk",
            )],
        );
        let rooted: Vec<(u32, u32, Option<String>)> =
            warnings("INVOKE base\nWATCH:\n  OVERRIDE:\n    bk []", &invoked)
                .into_iter()
                .filter(|w| w.code == LintCode::L002)
                .map(|w| (w.position.line, w.position.col, w.recipe))
                .collect();
        assert_eq!(
            rooted,
            vec![(3, 3, Some("base@1".into()))],
            "a root override of an invoked COPY points at that COPY, labeled with its recipe"
        );
    }

    /// Test 76: L003, an override that differs from the default return path
    /// (compared in return-step order: trim first).
    #[test]
    fn override_differs_from_default_return_path_warns() {
        let source = |override_chain: &str| {
            format!(
                "COPY_DEFAULT_WITH [upper, enrichment-injection]\nSOURCE r:\n  COPY . x/ AS k\nWATCH:\n  OVERRIDE:\n    k [{override_chain}]"
            )
        };
        let differs = with_code(
            &warnings(&source("lower, enrichment-trim"), &catalog()),
            LintCode::L003,
        );
        assert_eq!(differs.len(), 1, "a reordered override is L003");
        assert_eq!(
            differs[0].2,
            "WATCH OVERRIDE for k is [lower, enrichment-trim], but the default return path is [enrichment-trim, lower]",
            "the message shows both sequences"
        );
        assert!(
            with_code(
                &warnings(&source("enrichment-trim, lower"), &catalog()),
                LintCode::L003
            )
            .is_empty(),
            "an override equal to the default path is not L003"
        );

        let skipped = warnings(
            "COPY_DEFAULT_WITH [strip, enrichment-injection]\nSOURCE r:\n  COPY . x/ AS k\nWATCH:\n  OVERRIDE:\n    k [enrichment-trim]",
            &catalog(),
        );
        assert!(
            with_code(&skipped, LintCode::L003).is_empty(),
            "Skipped entries are omitted from the comparison: [enrichment-trim] matches"
        );
        assert_eq!(
            with_code(&skipped, LintCode::L001).len(),
            1,
            "strip is still reported as L001"
        );
    }

    /// Test 77: L004, a directory transform that declares a reverse.
    #[test]
    fn directory_transform_with_reverses_warns() {
        let catalog = catalog().with_transform("unpack", TransformScope::Directory, Some("pack"));
        let all = warnings("RUN flatten\nRUN unpack", &catalog);
        assert_eq!(
            with_code(&all, LintCode::L004)
                .iter()
                .map(|(line, col, _)| (*line, *col))
                .collect::<Vec<_>>(),
            vec![(2, 1)],
            "only the RUN of a directory transform with a reverse"
        );
    }

    /// Test 78: L005, a COPY chain without an injection, including an
    /// explicit empty OVERRIDE_WITH and inside an invoked recipe.
    #[test]
    fn chain_without_injection_warns() {
        let catalog =
            catalog().with_recipe("base", 1, 1, &[(1, 11, "SOURCE r:\n  COPY . b/ AS b")]);
        let all = warnings(
            "COPY_DEFAULT_WITH [enrichment-injection]\nSOURCE r:\n  COPY . x/ AS k:\n    OVERRIDE_WITH []\n  COPY . y/ AS j\nINVOKE base",
            &catalog,
        );
        let l005: Vec<(u32, u32, Option<&str>)> = all
            .iter()
            .filter(|w| w.code == LintCode::L005)
            .map(|w| (w.position.line, w.position.col, w.recipe.as_deref()))
            .collect();
        assert_eq!(
            l005,
            vec![(3, 3, None), (2, 3, Some("base@1"))],
            "the emptied COPY in the root, then base's COPY (root first, then invoked)"
        );
    }

    /// Test 79: L008, several reversers; L003 is skipped for that key.
    #[test]
    fn ambiguous_reverse_warns_and_skips_l003() {
        let catalog = catalog().with_transform("lower2", TransformScope::File, Some("upper"));
        let all = warnings(
            "COPY_DEFAULT_WITH [upper, enrichment-injection]\nSOURCE r:\n  COPY . x/ AS k\nWATCH:\n  OVERRIDE:\n    k [enrichment-trim]",
            &catalog,
        );
        let l008 = with_code(&all, LintCode::L008);
        assert_eq!(l008.len(), 1, "one L008: {all:?}");
        assert!(
            l008[0].2.contains("lower, lower2"),
            "names every candidate: {}",
            l008[0].2
        );
        assert!(
            with_code(&all, LintCode::L003).is_empty(),
            "no L003 against a path that is ambiguous"
        );
    }
}
