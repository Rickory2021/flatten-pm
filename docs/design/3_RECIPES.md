<!-- docs/design/3_RECIPES.md -->

# Scripting Recipes

There is one recipe type. A pipeline binding activates a recipe. Export is the
run the user triggers. Watch is the standing inverse of every active binding's
last export. A recipe describes both directions: `COPY` blocks define the forward
path (export) and watch derives the reverse path from them automatically.

## Recipe

### Instructions

| Level | Instruction | What it does |
|---|---|---|
| Recipe | `ARG name[=default]` | Declares a variable, substituted as `${name}`. |
| Recipe | `COPY_DEFAULT_WITH [transforms]` | Sets the default per-file transform chain. Sequential: if absent, the default is empty. A second declaration overrides the first. Scoped to the recipe it's declared in; an invoked recipe's default does not inherit from the caller. |
| Recipe | `SOURCE <repo>:` | Block. Ingests the repo; contains the `COPY` blocks that read from it. |
| Recipe | `RUN <transform> [--only <glob>...]` | Applies a structural (directory-level) transform to the run folder. |
| Recipe | `INVOKE <recipe>[@N] [name=value ...]` | Expands another recipe's instructions in place. Self-contained: brings its own SOURCEs and `COPY_DEFAULT_WITH`. `COPY` keys must be unique across the expanded recipe (collision at expansion time is a parse error). Depth limit 100; cycle detection. |
| Recipe | `WATCH:` | Optional block. Watch configuration for this recipe: depth tolerance and reverse chain overrides. |
| Inside `WATCH` | `DEPTH_TOLERANCE <n>` | Max missing levels (file + dirs) for auto-placement. Default 2. |
| Inside `WATCH` | `OVERRIDE:` | Optional sub-block. Per-`COPY`-key reverse chain overrides. |
| Inside `OVERRIDE` | `<key> [transforms]` | Replaces the derived reverse for this `COPY` key. |
| Inside `SOURCE` | `COPY <src> <dest> AS <key>:` | Block. Copies files from this `SOURCE`, applies per-file transforms, records excludes. |
| Inside `COPY` | `EXCLUDE <pattern>...` | Deletes matching paths from this `COPY`'s contribution. |
| Inside `COPY` | `OVERRIDE_WITH [transforms]` | Replaces `COPY_DEFAULT_WITH` for this block. |

A `COPY` outside a `SOURCE` block is a parse error. No other ordering constraints:
recipe-level instructions (ARG, `COPY_DEFAULT_WITH`, `SOURCE`, `RUN`, `INVOKE`,
WATCH) execute in the order they appear. `INVOKE` expands at its
position; invoked recipes bring their own SOURCEs and `COPY_DEFAULT_WITH`. `COPY`
is the mapping instruction; nothing after `COPY` changes `dest_path`. `RUN`
reshapes the folder (move, merge, create); it does not edit sourced file
content.

### SOURCE block

```
SOURCE <repo>:
  COPY <src> <dest> AS <key>:
    EXCLUDE <pattern>...
    OVERRIDE_WITH [per-file transforms]
```

`SOURCE` ingests the repo and scopes the `COPY` blocks inside it to that repo.
Multi-repo recipes have multiple `SOURCE` blocks. The nesting makes scope
explicit: there is no implicit "current repo" state.

### COPY block

```
COPY <src> <dest> AS <key>:
  EXCLUDE <pattern>...
  OVERRIDE_WITH [per-file transforms]
```

- `AS <key>` is mandatory. Gives the block a stable name for the export state
  record and for WATCH `OVERRIDE` references. Keys must be unique within a
  recipe. Explicit keys stay stable across recipe edits; auto-generated keys
  would break WATCH `OVERRIDE` references and export state matching.
- `EXCLUDE` lines delete matching paths from this `COPY`'s contribution to the
  run folder. Gitignore-style globs. `EXCLUDE --binary` deletes by extension
  list.
- `OVERRIDE_WITH` replaces `COPY_DEFAULT_WITH` for this block. `OVERRIDE_WITH
  []` means no per-file transforms; files from this `COPY` block carry no
  enrichment and will not round-trip through watch (skipped as "not our file"
  on return). Lint warns on any resolved chain missing `enrichment-injection`.
- Without `OVERRIDE_WITH`, the `COPY_DEFAULT_WITH` chain applies.
- Every transform in a `COPY` chain must be a file transform. `reverses` is
  optional: a transform without a reverse is skipped on return (the file keeps
  that transformation). Lint warns on non-reversible transforms in a chain.
  Lint errors if `enrichment-injection` is present without a reachable
  `enrichment-trim` reverse.

### Two transform types

| Type | Scope | API | Used by | Reversible |
|---|---|---|---|---|
| File | One file | read, write, rename | `COPY` chains | Optional. Declares `reverses` in metadata when it has a paired inverse. |
| Directory | Whole run folder | list, read, write, move, remove | `RUN` | Ignored. A directory transform declaring `reverses` is a lint warning; watch never runs it. |

File transforms edit content per file. Directory transforms reshape the folder.
The cut: sourced file content is only modified by the per-file chain attached
to its `COPY`. A directory transform may create new files (emit) and
move/remove existing files, but does not edit sourced content.

`reverses` is a file-transform concept only. Watch reverses at the per-file
level; the structural inverse of directory transforms is already handled by
the mapping rules and the enrichment. A directory transform that declares
`reverses` in its module metadata is ignored by watch and flagged by lint.

### Watch configuration

The optional `WATCH` block configures how this recipe behaves on the return
path: depth tolerance for auto-placement and reverse chain overrides.

```
WATCH:
  DEPTH_TOLERANCE 3
  OVERRIDE:
    vendor-files [custom-vendor-restore, enrichment-trim --template-set vendor]
```

**`DEPTH_TOLERANCE`** sets the max missing path levels (file + parent directories)
for auto-placement. Default 2 if absent. Depth 0 means the file must already
exist in the trie; depth 1 allows a new file if the parent directory exists;
depth 2 allows a new file with one new parent directory.

**`OVERRIDE`** replaces the derived reverse chain for named `COPY` keys. Watch
derives the reverse chain from each `COPY` block's recorded forward chain: for
each transform in reverse order, if it declares a `reverses`, run that reverse;
if it doesn't, skip it (the file keeps that transformation). An `OVERRIDE`
replaces this entire derived chain for the named key.

If `WATCH` is absent, default depth tolerance applies and every `COPY` block
reverses using its declared reverses. If `OVERRIDE` is absent, all keys use
the derived chain. A WATCH `OVERRIDE` that differs from the declared reverses is
flagged by lint (informational, not blocking).

### Example recipe

```
ARG repo

COPY_DEFAULT_WITH [enrichment-injection --template-set default]

SOURCE ${repo}:
  COPY . ${repo}/ AS all-files:
    EXCLUDE *.log
    EXCLUDE node_modules/
    EXCLUDE --binary

  COPY vendor/ ${repo}/vendor/ AS vendor-files:
    OVERRIDE_WITH [strip-vendor-headers, enrichment-injection --template-set vendor]

  COPY raw/ ${repo}/raw/ AS raw-files:
    OVERRIDE_WITH []

RUN flatten
RUN context-manifest

WATCH:
  DEPTH_TOLERANCE 2
  OVERRIDE:
    vendor-files [custom-vendor-restore, enrichment-trim --template-set vendor]
```

Export records each `COPY` block as a self-contained unit: key, repo, src/dest
prefixes, excludes, and the resolved forward chain with versions and args.
The export state also records which template sets were used. Watch reads one
`COPY` record per block to know what was placed, what was excluded, and what
to reverse.

### Shipped generic recipe

```
ARG repo
COPY_DEFAULT_WITH [enrichment-injection --template-set default]
SOURCE ${repo}:
  COPY . ${repo}/ AS all-files
RUN flatten
RUN context-manifest
```

Seeded at migration. The binding supplies `repo`. Makes every output path start
with the repo name, which keeps mappings disjoint across repos and lets the AI
see which project a file belongs to.

### Parser and storage

The parser is a hand-rolled line parser in `flatten-core` that produces a
`Recipe` struct (ARGs and an ordered instruction list with INVOKEs expanded).
The recipe text is canonical; the parsed structure is a derived view. Edits
happen on text, then reparse. See the Recipe grammar contract for the formal
grammar.

Recipe text is stored in `build_recipe_versions.source` and versioned via
pointer-based versioning (edit = insert new version + move pointer, rollback =
move pointer to old row). See the Versioning contract.
### Recipe Grammar


A build recipe is a block-structured text file. Its text is the only stored
form. A hand-rolled line parser in `flatten-core` produces the parse output.

**Syntax:**

| Feature | Rule |
|---|---|
| Comments | `#` to end of line |
| Continuation | `\` at end of line joins the next line |
| Quoting | Double-quoted args with escapes (`\"`, `\\`, `\n`, `\t`) |
| Substitution | `${NAME}` in any argument, resolved before validation |
| Indentation | Significant for nesting (`SOURCE` > `COPY` > `EXCLUDE`/`OVERRIDE_WITH`, WATCH > `OVERRIDE`) |
| Blocks | Opened by a trailing `:` on the parent line |

**Recipe-level instructions (execute in file order, no ordering constraints):**

| Instruction | Syntax | Semantics |
|---|---|---|
| ARG | `ARG name[=default]` | Declares a variable. Defaults may reference earlier ARGs. Without a default, the binding must supply it. |
| `COPY_DEFAULT_WITH` | `COPY_DEFAULT_WITH [t1, t2 ...]` | Sets the default per-file transform chain. Sequential: absent = empty, second declaration overrides. Scoped to the recipe (invoked recipes don't inherit). |
| `SOURCE` | `SOURCE <repo>:` | Block. Ingests the repo. Contains the `COPY` blocks that read from it. |
| `RUN` | `RUN <transform> [--only <glob>...]` | Applies a directory transform. View depends on what has executed so far. |
| `INVOKE` | `INVOKE <recipe>[@N] [name=value ...]` | Recursive: expands into current execution. Self-contained (own SOURCEs, own `COPY_DEFAULT_WITH`). Depth limit 100; cycle detection via visited set keyed by `(build_recipe_id, resolved_version)`. Same recipe at different versions is not a cycle. |
| WATCH | `WATCH:` | Optional block. Contains `DEPTH_TOLERANCE` and `OVERRIDE`. |

**Inside `SOURCE`:**

| Instruction | Syntax | Semantics |
|---|---|---|
| `COPY` | `COPY <src> <dest> AS <key>:` | Block. Copies files from this `SOURCE`. `AS <key>` mandatory, unique across the expanded recipe. |

**Inside `COPY`:**

| Instruction | Syntax | Semantics |
|---|---|---|
| `EXCLUDE` | `EXCLUDE <pattern> [<pattern>...]` | Gitignore-style globs. `EXCLUDE --binary` deletes by extension list from settings. |
| `OVERRIDE_WITH` | `OVERRIDE_WITH [t1, t2 ...]` | Replaces `COPY_DEFAULT_WITH` for this block. `[]` = no per-file transforms. |

**Inside `WATCH`:**

| Instruction | Syntax | Semantics |
|---|---|---|
| `DEPTH_TOLERANCE` | `DEPTH_TOLERANCE <n>` | Max missing path levels (file + dirs). Default 2. |
| `OVERRIDE` | `OVERRIDE:` | Sub-block. Per-`COPY`-key reverse chain overrides. |

**Inside `OVERRIDE`:**

| Syntax | Semantics |
|---|---|
| `<key> [t1, t2 ...]` | Replaces the derived reverse for this `COPY` key. |

**`COPY_DEFAULT_WITH` and `OVERRIDE_WITH` constraints:**

- Every transform must be a file transform.
- `reverses` is optional: a transform without a reverse is skipped on return.
- Lint warns on non-reversible transforms in a chain.
- Lint errors if `enrichment-injection` is present without a reachable
  `enrichment-trim` reverse.

**ARG resolution order:** recipe default → binding `arg_values` → `--arg
NAME=value` on the command line.

**`INVOKE` binding order:** explicit `name=value` on the `INVOKE` line → caller's
ARG of the same name → invoked recipe's default. A required ARG still unbound
is an error naming both recipes. Invoked ARGs are local.

**Version pinning:** `@N` on `INVOKE` and `RUN` pins a specific version number.
Absent = follow `current_version_id`.

**Lint (warnings only, never errors):**

| Lint | Reason |
|---|---|
| Non-reversible transform in a `COPY` chain | Returned files will keep that transformation |
| `enrichment-injection` without `enrichment-trim` as its reverse | Watch can't strip the enrichment |
| WATCH `OVERRIDE` differs from derived reverses | Informational: custom return path |
| Directory transform declares `reverses` | Ignored by watch; `reverses` is file-only |

**Shipped generic recipe:**

```
ARG repo
COPY_DEFAULT_WITH [enrichment-injection --template-set default]
SOURCE ${repo}:
  COPY . ${repo}/ AS all-files
RUN flatten
RUN context-manifest
```

Seeded at migration. The binding supplies `repo`.

### Parse Output


The in-memory structure the parser produces from recipe text. A derived view;
the text is canonical.

```
Recipe {
    args: Vec<Arg>,             // name, default (Option), position
    instructions: Vec<Instruction>,  // flattened, INVOKEs expanded
    watch_config: WatchConfig,  // depth_tolerance, overrides
    invoked_versions: Vec<VersionId>, // every INVOKE's resolved version
}

Arg {
    name: String,
    default: Option<String>,    // may reference earlier ARGs
    position: Position,
}

Instruction = SOURCE | COPY | RUN

SourceInstruction {
    repo_name: String,          // post-substitution
    copies: Vec<CopyBlock>,
    position: Position,
}

CopyBlock {
    src: String,                // post-substitution
    dest: String,               // post-substitution, normalized
    key: String,                // AS <key>
    excludes: Vec<Pattern>,
    forward_chain: Vec<TransformRef>,  // resolved from COPY_DEFAULT_WITH or OVERRIDE_WITH
    position: Position,
}

TransformRef {
    name: String,
    version: ResolvedVersion,   // pinned or current
    args: Map<String, String>,  // defaults applied
}

RunInstruction {
    transform: TransformRef,
    scope: Vec<Glob>,           // --only, empty = everything
    position: Position,
}

WatchConfig {
    depth_tolerance: u32,       // default 2
    overrides: Map<String, Vec<TransformRef>>,  // key → override chain
}
```

Every parse error carries line and column. The parse output retains positions
for every instruction so the export state record can reference them.

## Components: Transform

### Transform Contract


A JavaScript module that operates on files. Two types with distinct APIs and
roles.

**Two types:**

| Type | Signature | API | Used by | reverses |
|---|---|---|---|---|
| File | `apply(file, args, ctx)` | `read()`, `write(content)`, `rename(name)` | `COPY` chains (`COPY_DEFAULT_WITH`, `OVERRIDE_WITH`) | Optional. Declares in metadata. Skipped on return if absent. |
| Directory | `apply(folder, args, ctx)` | `list()`, `read(path)`, `write(path, content)`, `move(from, to)`, `remove(path)` | `RUN` | Ignored. Lint warns if declared. |

File transforms edit content per file. Directory transforms reshape the folder
(move, merge, create, remove). The cut: sourced file content is only modified
by the per-file chain attached to its `COPY` block.

**Module metadata (cached on row at save):**

| Field | Type | Meaning |
|---|---|---|
| `name` | String | Transform name. Resolution: `RUN <name>` or in a `COPY` chain. |
| `scope` | `file` \| `directory` | Determines the API surface. |
| `curation` | `builtin` \| `custom` | Builtin rows are protected from deletion. |
| `reverses` | Option\<String\> | File scope only. Names the transform this one reverses on return. Multiple transforms may declare the same `reverses` target; the reverse used at runtime is determined by the `COPY` block's forward chain, not by global declarations. Lint does not warn on duplicate `reverses` values. |

**Runtime (QuickJS-NG via rquickjs):**

| Rule | Detail |
|---|---|
| Execution | JS module loaded from `transform_versions.source`. One fresh QuickJS context per invocation. |
| Sandbox | No filesystem outside the run folder, no network, no process, no timers, no imports. |
| Determinism | `Date.now` and `Math.random` replaced with deterministic implementations seeded from input hash and instruction position. |
| Limits | Per-call timeout (`transform_timeout_ms`, default 10s) and memory limit (`transform_memory_mb`, default 256 MB). |
| Failure | A throw or limit breach fails the export; the run folder is deleted. During watch, a failure fails the individual placement; the source file stays. See Return step contract. |
| File API note | `read()` returns a JS string (UTF-16). A file near `copy_size_limit_mb` is near the memory limit. |

**Directory transform constraints:**

- `list()` sorted by `/`-joined relative path in byte order, filtered by
  `--only` scope.
- Every path resolves inside the run folder; `../`, absolute, and backslash
  paths rejected.
- `write` to a new path creates an emitted file. `write` to an existing sourced
  file is a lint warning (directory transforms should not edit sourced content).
- `move` changes where a file lands in the export folder, never its `dest_path`.

**Transform imports (s1):**

One transform may call another. The import mechanism (module-level `import` or
a host API function) is a contract-level decision to settle during
implementation. Reversed from the old ADR that rejected this.

**ctx:**

| Field | What |
|---|---|
| `args` | Resolved args (defaults applied) |
| `only_globs` | Resolved `--only` globs (directory transforms only) |
| `repos` | Map of repo id to name |
| `log(msg)` | Logging |
| `context.note(key, value)` | Appends to the run-scoped accumulator (ordered by instruction position and path) |
| `render(template[@N], params)` | Renders a template through MiniJinja. Used by enrichment-injection and context-manifest. |

**Builtins:**

| Name | Type | reverses | What it does |
|---|---|---|---|
| `enrichment-injection` | file | — | Injects enrichment (see Enrichment contract) |
| `enrichment-trim` | file | `enrichment-injection` | Strips enrichment by matched pattern |
| `flatten` | directory | — | Moves files to flat encoded names |
| `pack` | directory | — | Merges files into fewer output files |
| `context-manifest` | directory | — | Renders ctx accumulator to `_CONTEXT.yaml` |

Builtins ship as embedded JS in the binary. Seeded into SQLite on first
launch or if missing at startup. Marked `curation=builtin`: inspectable,
editable, versioned like any other, but protected from deletion. Restore from
shipped reads the embedded content and inserts it as a new version.

### Builtin Transforms


All builtins follow the same contract as custom transforms (same runtime,
same API per scope). The distinction is `curation=builtin`: protected from
deletion, upgradable on app update, restore-from-shipped available.

**`RUN` flatten (directory):**

| Rule | Detail |
|---|---|
| Encoding | Moves each file to a flat name encoding its `dest_path` via `folder.move`. |
| Delimiter | Configurable, default `--`. `src/app/main.rs` → `src--app--main.rs`. |
| Percent-encoding | For injectivity: `%` first, then delimiter literal inside a segment, then leading `_`, then characters illegal on any target OS. |
| Dotfile mapping | After encoding: leading `.` on any segment → `_`. `.env` and `_env` cannot collide (real `_` already encoded). Upload platforms rename dotfiles; the app owns the rename. |
| NAME_MAX | Names over 255 UTF-8 bytes truncated with BLAKE3 hash suffix. Warned. |
| Content | Never edited. The enrichment already inside the file preserves `dest_path` after the move. |
| Windows | Reserved names (CON, PRN, etc.) and trailing dot/space warned. |
| ctx | Records delimiter and encoding convention. |

**`RUN` pack (directory):**

| Rule | Detail |
|---|---|
| Merge | Files merged into fewer output files. |
| Limits | `--file-limit N` (hard cap, N=1 for single file), `--byte-limit N` (best-effort). |
| Fill order | Sequential in alphabetical `dest_path` order (directory affinity). `file-limit` wins when both set; last file absorbs overflow with warning. |
| Format | `--format xml` (Repomix convention) or `markdown`. |
| XML | Content wrapped in CDATA with `]]>` split. Control characters stripped and warned. |
| Markdown | Fence length is one longer than the longest backtick run in content. |
| Binary | Skipped with warning. |
| Emitted | Pack files are emitted (ledger: remove-then-write). Packed sources' `dest_path`s still resolve through rules because enrichment was injected before packing. |
| ctx | Records pack boundaries and format. |

**`RUN` context-manifest (directory):**

| Rule | Detail |
|---|---|
| Renders | Accumulated ctx to `_CONTEXT.yaml` through the `context-manifest` template (kind=file). |
| Default content | Encoding convention, dotfile mapping, enrichment convention with exact form, exclusion log, instruction to preserve enrichment, repo-name statement. |
| Emitted | Written as an emitted file. |
| Opt-in | Only runs if the recipe includes `RUN context-manifest`. |
| Template | `--template <name[@N]>` selects. Default is `context-manifest`. |

## Components: Template

### Template Contract


Versioned text a transform renders through `ctx.render`. Two kinds with
different validation.

**Kinds:**

| Kind | What it is | Validation | Used by |
|---|---|---|---|
| `enrichment` (template set) | A collection of entries defining how to enrich files by type | Linearity check, render-extract round-trip (see Enrichment contract) | `enrichment-injection` via `--template-set <name>` |
| `file` | A single MiniJinja text | Free MiniJinja (`{% if %}`, `{% for %}` allowed) | `context-manifest` and custom transforms via `ctx.render(name, params)` |

**Storage:** `template_versions.source` is the text. Versioned via pointer
(see Versioning). `curation` = `builtin` or `custom`. Builtin templates are
seeded and protected from deletion.

**Template set entries (kind=enrichment):**

Each entry matches a set of files and defines how their enrichment looks.

| Entry field | What |
|---|---|
| Filename regex | Anchored at right end (`.rs`, `.d.ts`, `Makefile`). Longest match wins when several match. |
| Open/close markers | Wrapping the body (e.g., `//` or `<!-- -->`). |
| Reserved-prefix rule | What must stay on line 1: shebang, `"use strict"`, `<?xml`, YAML `---`/`%YAML`, Markdown front matter, Python encoding cookie, `/// <reference>`, `# syntax=`. Injection lands after it. |
| Syntax-safe flag | True for a real comment, false for a JSON field or YAML key. |
| Body | Must be linear: literals and `{{ }}` only, `{{path}}` exactly once. |
| Default body | Per-set default; entries may override. |

**Shipped template sets:**

`default` (kind=enrichment) with entries:

| Marker | File types |
|---|---|
| `#` | py, yaml, yml, toml, sh, Makefile, Dockerfile, .gitignore, LICENSE, NOTICE, AUTHORS, COPYING |
| `//` | ts, tsx, js, jsx, rs, go, c, h, cpp, java |
| `<!-- -->` | md, html, xml |
| `--` | sql |
| `/* */` | css |
| JSON path field | json, jsonc (markers are field syntax; syntax-safe = false) |

Shipped body carries a distinguishing literal (e.g., `flatten: {{path}}`) so
a bare first-line comment already in a repo never matches an extraction pattern.

`context-manifest` (kind=file): MiniJinja template rendering the ctx
accumulator.

**Storage format:** the template set is stored as a single YAML document in
`template_versions.source`. Each entry is a mapping. The exact schema is an
implementation decision.

**Save-time validation (enrichment kind):**

Render sample paths per entry (including nested, dotfile, unicode paths) and
extract each through the derived pattern. A miss rejects the save.

### Enrichment


The signal watch extracts to find where a file belongs. Injected by
`enrichment-injection` (file transform), stripped by `enrichment-trim` (file
transform).

**Injection (`enrichment-injection`):**

| Step | What happens |
|---|---|
| Match entry | File's name matched against the template set's entries (longest match). No match: file skipped, reported as will-not-round-trip. |
| Render body | `{{path}}` replaced with the file's `dest_path` (`/` delimiter). |
| Find insertion point | After the file's reserved prefix (per the entry's rule). |
| Idempotent replace | If the file's head already matches an extraction pattern, replace it. Never stack. |
| Line ending | Detected from file's first newline. BOM preserved before the enrichment. |
| JSON | Top-level `path` field inserted textually after opening brace with detected indent. Arrays and unparseable JSON skipped and logged. |
| `--reversible=false` | Tells watch to leave the enrichment in place on return. Allowed only for syntax-safe entries. |
| `--template-set` | Selects which template set. Default is `default`. |

**Trim (`enrichment-trim`, reverses `enrichment-injection`):**

Removes whichever extraction pattern the returned file matched. Needs no
knowledge of which injection version, args, or template set ran. Pattern-driven.

A file matching no pattern is left unchanged.

**Extraction patterns:**

One per `(template_version, entry)`. Derived from every version of every
enrichment template set, current or not (files exported under an older version
must still match on return).

| Component | How derived |
|---|---|
| Pattern | Escaped markers wrapping the body. Every line. Literals escaped, other variables as non-greedy wildcards, `{{path}}` as the capture group. |
| Read window | Rendered pattern length + buffer for reserved prefix and path length. |

**Committed comment preservation:**

If the file already on disk at the target begins with a line matching an
extraction pattern (a committed directory comment the repo had), that line is
preserved after the reverse chain. A round trip never deletes repo content.

## Binding


The activation record for a recipe. One binding = one recipe + the ARG values
that complete it.

**Fields:**

| Field | Type | What |
|---|---|---|
| `id` | int | PK |
| `build_recipe_id` | int FK | The recipe this binding activates. |
| `version_pin` | int, nullable | Null = follow `current_version_id`. `@N` syntax. |
| `active` | bool | Whether watch serves this binding. Default true. |
| `arg_values` | JSON | Required ARGs and default overrides. |
| `created_at` | datetime | |
| `last_used_at` | datetime, nullable | Last export completion. |

**Export folder:** derived at runtime as `{app_data}/export/binding-{binding_id}/{export_uuid}/`. Each export creates a new subdirectory. The path is stored in `export_state.output_dir`. Not stored on the binding.

**Lifecycle:**

| Operation | What happens |
|---|---|
| Create | Insert row. A missing required ARG is an error. |
| Activate/deactivate | Toggle `active`. Triggers watch state reload. Does not touch existing flags or candidates (frozen). |
| Edit | Change `version_pin` or `arg_values`. A changed arg invalidates the short-circuit on next export. |
| Delete | Cascade: delete `export_state` row (cascades `safety_findings`), remove binding's export directory. `watch_match_candidates` rows kept with `pipeline_binding_id` set null (frozen). Triggers watch state reload. |

**Soft-delete guard on parent entities:** soft-deleting a repo or recipe that
has active bindings is a domain error naming the bindings. Deactivate or
delete the bindings first. This prevents orphaned active bindings from
producing export or watch failures.

**Repos** are not stored on the binding. Active repos are discovered from the
recipe's `SOURCE` names after ARG substitution.

**Independence:** two bindings on one recipe with different `arg_values` are
fully independent: separate runs, folders, rules, and candidates.

**Discovered repos:** `SOURCE` names from the parsed recipe after substitution,
resolved to `repos` rows. Unknown names shown as errors.

