<!-- docs/ADR.md -->
# Architecture Decision Records

Decisions that shaped Flatten PM's design. Each ADR: context, choice, rationale,
rejected alternatives. Contracts referenced by name; see `docs/design/`.

Records are numbered for reference. New decisions append; numbers are stable.


## Carried forward (from original design, updated for new structure)

### ADR-001: One recipe type; watch is the inverse of export

**Context:** the original design had export recipes and watch recipes as
separate types, with watch redeclaring repos and inverting the layout by
re-reading the current recipe. Rerooted COPY, version skew, and edits between
export and return all diverge "what the recipe says now" from "what produced
these files."

**Choice:** one recipe type. A binding activates a recipe. Export is the run.
Watch is the standing inverse of every active binding's last export. The export
run records what ran; watch resolves against that record.

**Rationale:** the run record is what produced the files; the recipe is what
would produce them now. Only the record cannot diverge. One entity means no link
table, no repo restatement, one definition of "active repo."

See: Recipe grammar contract, Export state contract, Resolution contract.

**Rejected:** watch recipe with `FROM <export-recipe>` (a second document that
can disagree). Re-reading the current recipe at watch time (wrong after any
edit or upgrade). Per-file map entries (the rules expanded; the export folder
on disk is the per-file record).


### ADR-002: Recipes are block-structured; INVOKE is a function call

**Context:** the original flat-sequential design had SOURCE as state-setting,
EXCLUDE as a top-level instruction, and no scoping of COPY transforms. With the
introduction of per-file transforms in COPY blocks and the WATCH configuration
block, the recipe needed explicit nesting.

**Choice:** block-structured recipe. SOURCE is a block containing COPY blocks.
COPY is a block containing EXCLUDE and OVERRIDE_WITH. WATCH is a block
containing DEPTH_TOLERANCE and OVERRIDE. INVOKE expands another recipe's
instructions at its position (function call with argument binding).

**Rationale:** nesting makes scope explicit. A COPY outside a SOURCE is a parse
error. EXCLUDE is per-COPY, not ambient. The WATCH block configures the return
path in the same file as the forward path. INVOKE is self-contained (own
SOURCEs, own COPY_DEFAULT_WITH).

See: Recipe grammar contract, Parse output contract.

**Rejected:** flat-sequential with implicit state (the original model; EXCLUDE
was ambient, SOURCE was state-setting). Dockerfile-style layers (no per-layer
output to stack, no instruction cache).


### ADR-003: Export builds in a fresh directory

**Context:** the earlier design ran instructions as trie-to-trie in memory. The
memory limit was per instruction, not per run; a large repo did not fit.

**Choice:** each export builds in a fresh directory on disk. `COPY` copies bytes.
`EXCLUDE` deletes. `RUN` transforms in place. On success the export state row is
replaced and the previous directory is deleted.

**Rationale:** memory bounded per file. Fresh directory per export means no
partial writes. The mapping is immutable by construction.

See: Export folder contract, Export state contract.

**Rejected:** in-memory tree with spill-to-disk (two code paths). Running
transforms in the export folder directly (no atomic swap).


### ADR-004: Export is on demand; the export folder is what was last uploaded

**Context:** with the trie updated on every placement, the question was whether
placement should also propagate into export folders.

**Choice:** no. Export runs only when the user runs it. The export folder means
"what was last uploaded." A placement updates the trie only.

**Rationale:** the export folder is derived data. Patching one file means
re-running enrichment with current versions, leaves output_hash stale, and adds
a second writer. The short-circuit makes recomputing cheap.

See: Export folder contract, Short-circuit contract.

**Rejected:** placement bus into export folders. Auto-export on trie change.


### ADR-005: Enrichment templates hold the entry table

**Context:** the enrichment body lived in a template while the entry table
(which files, which markers) lived in the injection transform's args_schema.

**Choice:** templates have a `kind`. An `enrichment` template (template set) is
a collection of entries. A `file` template is a single MiniJinja text.
Extraction patterns are derived from every version of every enrichment template
set.

**Rationale:** one entity, one version, one edit surface, one source of
patterns. Per-platform variation is two template set rows.

See: Template contract, Enrichment contract.

**Rejected:** a collection entity above templates (a table for a list of one).
Parse-and-reserialize for JSON.


### ADR-006: Flags are frozen file state; candidates are per-binding opinions

**Context:** with multiple active bindings, one file can be seen by several.
Re-detecting on every config change routed deactivated bindings' files elsewhere.

**Choice:** a flag is one row per detection (each detection creates a new
independent flag row). Each binding's resolution result is a candidate row,
written for every active binding (not just viable candidates). Detection runs
once and freezes the candidate set including the reverse chain snapshot.
Resolution is approve, dismiss, or expire. There is no replacement logic;
users dismiss stale flags manually.

**Rationale:** one problem, one row. Freezing prevents misfires from config
changes. Source filename is not part of the key (browsers rename). Writing a
candidate row for every binding (with status `ok`, `exceeds_depth`,
`excluded`, or `no_match`) provides complete diagnostic data for `flag
explain` without re-running resolution. Snapshotting the reverse chain on
the candidate row means approve uses the chain that was active at detection
time, not the current export_state (which may have changed between detection
and approval).

See: Watch match flags contract and Watch match candidates contract in `5_WATCH.md`.
See also: ADR-044 (independent flag detection).

**Rejected:** re-detection on every config change. Deactivation as resolution.
A dismissed status (duplicates file_history). Re-drop replacement (no
`content_hash` needed; same filename with different size/mtime creates a new
independent flag).


### ADR-007: Two transform scopes; every transform is JS in QuickJS-NG

**Context:** the original design had one transform type. Builtins were stored
as source with no execution path. Per-file transforms in COPY chains need a
restricted API (no list, no access to other files).

**Choice:** two scopes: file (read, write, rename) and directory (list, read,
write, move, remove). Every transform is JS executed by QuickJS-NG via rquickjs.
Builtins ship as JS and are seeded as protected rows.

**Rationale:** one runtime for builtin and custom. File scope is safe for COPY
chain fusion by construction (can't see other files). JS matches the frontend
and needs nothing installed.

See: Transform contract.

**Rejected:** native builtins (two execution paths). `inverse()` method.
Runtime rustc. WASM. Embedded Python.


### ADR-008: Recipe text is the source of truth; no instruction cache

**Context:** the earlier design stored recipes twice (text + rows) plus a
per-instruction cache.

**Choice:** `build_recipe_versions.source` is the only stored form. Parsing
yields the in-memory recipe. No instruction cache. A whole-run short-circuit
is the only shortcut.

**Rationale:** parsing happens anyway. A full run at 500-2000 files is seconds.
The short-circuit makes the common case free.

See: Recipe grammar contract, Short-circuit contract.

**Rejected:** rows as SSOT. Per-instruction output storage.


### ADR-009: Enrichment is the sole placement gate; no enrichment = skip

**Context:** AI platforms strip and rewrite comments. Downloads folders hold
files from all sources. Any heuristic matching risks silent misplacement.

**Choice:** detection uses only the enrichment. No enrichment: skip (not our
file). Enrichment present but unresolvable: flag. No fuzzy or similarity
matching.

**Rationale:** the enrichment is the contract between export and watch. If the
AI breaks it, that is visible as a flag or a skip, not silently absorbed.

See: Enrichment contract, Resolution contract.

**Rejected:** similarity flags for stripped files. Embedded path substring
detection. Fuzzy matching.

**Note:** the original design flagged no-enrichment files as `no_match`. Changed
to skip: non-enriched files are not ours (random downloads during dev work).


### ADR-010: Reconciliation rescan prioritized by recency

**Context:** the notify crate has confirmed unfixed event loss under burst load.

**Choice:** events as hints + periodic reconciliation rescan against the export
folder, newest files first.

**Rationale:** events provide fast response. The sweep provides correctness.
Newest files first ensures the most recent download gets attention first.

See: Download detection contract, Reconciliation contract.

**Rejected:** events only (loses files under load).


### ADR-011: MCP backend hosted inside Tauri via Streamable HTTP

**Context:** Streamable HTTP on localhost works for 8/9 major MCP clients.

**Choice:** MCP inside Tauri via Streamable HTTP on localhost. Targets the
stateless spec.

**Rationale:** one process owns all state. MCP tools are function calls into
the same code the GUI uses. The forge daemon proxies to this endpoint.

See: Architecture section (v2).


### ADR-012: Forge daemon as FastMCP aggregator

**Choice:** shared daemon in FastMCP/Python. Prefixed tool mounts. One endpoint
for the user.

**Rationale:** thin proxy, not performance-critical. FastMCP's mount system
provides prefixing out of the box.


### ADR-013: MCP auth via static bearer token

**Choice:** static bearer token + Origin/Host validation. Bound to 127.0.0.1.

**Rationale:** token generated on first launch. Origin validated when present,
Host when absent. OAuth reserved for beyond-localhost.


### ADR-014: Core logic in a standalone library crate

**Choice:** `flatten-core` library crate from day one. No Tauri dependencies.

**Rationale:** the CLI, Tauri app, MCP server, and tests all consume it. The
cost is one extra crate. The cost of NOT having it is duplicated logic
everywhere.

See: Architecture section (Components).


### ADR-015: Persistent storage: hybrid SQLite + in-memory

**Choice:** SQLite (rusqlite, bundled, WAL mode) for queryable data. Trie cache
on filesystem as MessagePack. Export folders on filesystem.

**Rationale:** SQLite in WAL gives concurrent readers + one writer. Tries are
too hot for disk queries but too important to lose. Three stores, three roles.

See: Database contract, Trie contract, Export folder contract.


### ADR-016: Pointer-based versioning over append-only history

**Choice:** parent entity holds `current_version_id` FK. Edit = insert + move
pointer. Rollback = move pointer. Version rows immutable.

**Rationale:** single-hop runtime access. Diffs computed on the fly.
repo_versions is the exception (append-only).

See: Versioning contract.


### ADR-017: Pipeline bindings are recipe-centric, not repo-centric

**Choice:** one binding = one active recipe + ARG values. No repo_id on
bindings. Repos discovered from SOURCE instructions.

**Rationale:** recipes are the execution unit. A recipe's SOURCE instructions
declare which repos it reads. Multiple bindings on one recipe with different
args are independent.

See: Binding contract.


### ADR-018: Embedded path detection dropped

**Choice:** drop the prototype's tier-3 embedded-path-substring detection.
Detection uses only the enrichment.

**Rationale:** false positives from paths in prose or code examples. The cost of
silent misplacement outweighs the benefit.

See: ADR-009.


### ADR-019: No normalization at ingest or trie level

**Choice:** trie hashes raw bytes on disk. Consumers normalize at point of use.

**Rationale:** if bytes didn't change, the hash shouldn't change. Line-ending
changes between runs are real changes.

See: Trie contract, Ingest rules contract.


### ADR-020: Watch history as a browsable diff thread (s2)

**Choice:** append-only log with mixed snapshot+diff model. Rollback
reconstructs content and appends a new entry.

**Rationale:** the user needs to see what the AI changed before committing.
Rollback is append, not destructive.

See: File history contract.


### ADR-021: Export preserves file content

**Choice:** full-content export. Transforms reshape, not alter source content.

**Rationale:** comments carry intent and context the AI uses. Over-broad exports
are a recipe problem. A custom transform that alters content is the user's
explicit choice.


### ADR-022: No token counting in v1

**Choice:** defer. The user can see file counts and sizes in the run report.


### ADR-023: No git-aware sorting or git metadata in export

**Choice:** export is git-agnostic for metadata. Ingest rules can use gitignore
patterns for filtering, but no git commit logs or change-frequency sorting.


### ADR-024: No remote repo packing

**Choice:** local repos only. Flatten PM manages repos the developer actively
works in.


### ADR-025: Repomix as reference, not dependency

**Choice:** constant reference, build natively in Rust. Repomix is
TypeScript/Node.js.


## New decisions (from design rewrite)

### ADR-026: Full re-export; no incremental export

**Context:** the prototype does incremental export (compares mtime/size per
file, copies only changed files). The new design runs every file through a
`COPY` block with per-file transforms, then directory transforms, in a fresh
export directory.

**Choice:** full re-export on every non-short-circuited run. The whole-run
short-circuit handles "nothing changed." An interactive force handles
"something is wrong." No per-file or per-COPY-block cache.

**Rationale:** the export target is an AI context window (500-2,000 files).
Full re-export at that scale is 2-5 seconds. Incrementality fails because:
directory transforms are lossy/order-dependent, a COPY-block cache saves
only transform time (bytes still needed in the run folder), and atomicity
requires a complete folder.

See: Short-circuit contract, Export folder contract.

**Rejected:** per-file mtime/size comparison (loses atomicity). Per-COPY-block
cache (saves transform time only). Incremental directory transforms
(structurally impossible for pack).


### ADR-027: COPY block model; EXCLUDE absorbed into COPY

**Context:** the original flat model had EXCLUDE as a top-level instruction
operating on the entire run folder. With per-file transforms in COPY chains,
EXCLUDE needed to be scoped to a specific COPY's contribution.

**Choice:** COPY is a block with EXCLUDE and OVERRIDE_WITH inside it. SOURCE is
a block containing COPY blocks. EXCLUDE is per-COPY, not ambient.

**Rationale:** the export state records each COPY block as a self-contained unit.
Watch reads one record per block. No ambiguity about which COPY an EXCLUDE
applies to.

See: Recipe grammar contract.


### ADR-028: reverses scoped to file transforms only

**Context:** the original design allowed any transform to declare `reverses`.
With two transform scopes (file and directory), the question was whether
directory transforms need reversal.

**Choice:** `reverses` is optional on file transforms, ignored on directory
transforms. Watch reverses at the per-file level only. The structural inverse
of directory transforms is handled by the mapping rules and the enrichment.

**Rationale:** flatten and pack are structural (move/merge). Their inverse is
the enrichment (which carries `dest_path`) and the COPY rules (which map
`dest_path` to `source_path`). Running a directory transform in reverse would
require reconstructing folder structure, which the enrichment already solves.

See: Transform contract, Return step contract.


### ADR-029: Config ejection for ingest; no live gitignore dependency

**Context:** the original design read `.gitignore` files at every ingest.
Precedence rules (gitignore then source-level, with negation) were confusing.
Exceptions required `!` syntax.

**Choice:** gitignore is a one-time import at registration (config ejection).
The stored flat pattern list is the source of truth. No live dependency.

**Rationale:** what you see is what you get. Exceptions: just delete the
pattern. No negation syntax needed. s2 adds diff-against-current and re-import.

See: Ingest rules contract.

**Rejected:** live `.gitignore` reading (precedence confusion, implicit config).


### ADR-030: Content safety scanning at export, not ingest

**Context:** the original design scanned at ingest. But ingest builds a trie
for internal use; the concern is files reaching an AI platform, which happens
at export.

**Choice:** scan during the COPY pass at export. Piggybacks on reading file
bytes. Flag-only, never blocks or mutates.

**Rationale:** scans exactly the files about to leave the machine, after EXCLUDE
has already filtered. Export-scoped flags are directly actionable.

See: Export flow diagram (content safety scan node).

**Rejected:** scanning at ingest (scans files that may never be exported).


### ADR-031: No enrichment = skip, not flag

**Context:** the original design flagged `no_match` for files without
enrichment. But downloads folders contain random files (PDFs, screenshots)
that would spam the flag queue.

**Choice:** no enrichment = skip (not our file). Flags exist only for files
that have enrichment but can't be unambiguously placed.

**Rationale:** the flag queue is a work queue for real decisions, not a log
of irrelevant downloads. If the AI strips enrichment, the user notices when
the file doesn't land.

See: Resolution contract, Watch flow diagram.

**Rejected:** flagging `no_match` (noisy; indistinguishable from random
downloads).


### ADR-032: Hand-rolled DSL for recipes; no piggybacking

**Context:** the recipe format evolved into a block-structured DSL. A deep
research investigation compared 10 existing embeddable languages (Starlark,
HCL, Nickel, CUE, Dhall, KCL, Lua, Rhai, JavaScript, custom DSL).

**Choice:** custom DSL with a hand-rolled line parser. Estimated 1,000-1,800
LOC in Rust. The recipe text is canonical; the parsed structure is a derived
view.

**Rationale:** the grammar is small (~10 keywords) and stable. Piggybacking
adds an adapter layer, unused language features, and a dependency. The DSL
maps exactly to the domain. winnow is the fallback if tokenizing gets fiddly.
tree-sitter is s2/s3 for editor support.

See: Recipe grammar contract, Parse output contract.

**Rejected:** Starlark (adapter layer, Python association), HCL (no maintained
Rust parser for embedding), Nickel (too heavy), CUE (no Rust binding), Lua
(second runtime alongside QuickJS), Rhai (niche), JavaScript (too permissive
for a declarative format).


### ADR-033: Transform imports; reversal of original rejection

**Context:** the original design rejected transform imports ("No transform
calls another; reuse is at the recipe level"). With the extension surface
maturing, imports should be available from the start.

**Choice:** s1. One transform may call another. The import mechanism is a
contract-level decision to settle during implementation.

**Rationale:** without imports, complex per-file transforms must be monolithic
or duplicate logic. The extension surface should be complete from s1 so users
don't hit walls.

See: Transform contract.


### ADR-034: WATCH block replaces global depth setting

**Context:** `watch_new_directory_depth` was a global setting. Different recipes
need different tolerances (stable codebase: depth 0; new project: depth 2+).

**Choice:** DEPTH_TOLERANCE lives in the recipe's WATCH block. Per-recipe, not
global. Default 2.

**Rationale:** the tolerance is a property of the recipe's layout, not a
system-wide preference. A recipe with `COPY . repo/` has different needs than
one with `COPY src/ app/src/`.

See: Recipe grammar contract, Watch resolution state contract, Resolution contract.


### ADR-035: Line-ending policy as per-repo setting; no .gitattributes scanning

**Context:** the original design had a three-tier policy (target file, then
.gitattributes, then LF). Scanning .gitattributes is a live git dependency.

**Choice:** per-repo setting (`repos.line_ending_policy`): `preserve` (default;
match existing target, LF for new) or `lf` (always LF). No .gitattributes
scanning.

**Rationale:** two rules, no scanning. Covers the common case. If .gitattributes
support is needed, it follows the config ejection pattern in s2.

See: Ingest rules contract, Return step contract.

**Rejected:** .gitattributes scanning (live git dependency, complexity for
narrow edge case). Global LF only (loses existing-file convention).


### ADR-036: Scope and curation replace overloaded type field

**Context:** the `transforms` table had a `type` field meaning both
"builtin vs custom" and "file vs directory," which are independent concerns.

**Choice:** two fields: `scope` (file/directory, determines API surface) and
`curation` (builtin/custom, determines deletion protection). Same split on
`templates` (curation + kind).

**Rationale:** independent concerns get independent fields. No ambiguity for
implementers.

See: Transform contract, Database contract.

### ADR-037: Export state as single-row replacement, not append-only history

**Context:** the `export_state` row stores the configuration of a binding's
last export. The question was whether to keep every row (append-only) or
replace on each success.

**Choice:** one row per binding, replaced on each successful export. No
export history table.

**Rationale:** the export state exists for two consumers: the short-circuit
(needs only the latest inputs) and watch (needs only the latest rules). No
consumer needs previous rows. file_history (s2) is the audit trail for file
content. Keeping old rows would require pruning logic, add storage, and serve
no consumer.

See: Export state contract.

**Rejected:** append-only export history (no consumer, adds pruning).


### ADR-038: Single writer thread for SQLite

**Context:** SQLite in WAL mode supports concurrent readers but serializes
writes. The question was how to manage write access from multiple pipeline
stages (export, watch, ingest, settings).

**Choice:** one dedicated thread owns the write connection, fed by a channel.
Each message is a complete transaction. No connection shared across threads or
held across an await.

**Rationale:** serialized writes match SQLite's model. Channel-fed
transactions eliminate lock contention and deadlocks by construction. The
write thread is the only code that calls `BEGIN`; callers compose messages and
receive results, never hold connections.

See: Database contract.

**Rejected:** connection pool (SQLite doesn't benefit from one in WAL mode).
Async rusqlite (adds runtime complexity for no concurrency gain on the write
path). Direct connection sharing with mutex (risks held-across-await).


### ADR-039: Serial file processing in watch

**Context:** watch handles files arriving in a source directory. The question
was whether to process files concurrently (parallel detection and placement)
or serially.

**Choice:** serial. One file at a time on a single thread.

**Rationale:** files arrive at human download speed. The settle check alone
throttles throughput. Serial processing eliminates trie races, flag write
conflicts, and seen-set synchronization. Concurrent processing is a measured
optimization; the serial-to-concurrent upgrade path is clean because the trie
already uses Arc swap.

See: Download detection contract, Watch resolution state contract.

**Rejected:** concurrent with per-file locking (complexity without measured
need).

### ADR-040: File history records placed files only; placed_by replaces pipeline + detection_method

**Context:** file_history is the s2 audit trail. The question was whether to
log every operational event (skips, flags, errors, placements) or only files
that were actually written. Separately, the original design tracked the source
pipeline (`export` or `watch`) and the detection method (`auto` or `manual`)
as two columns.

**Choice:** file_history records only placed files: files written to a repo by
watch, or written to the export directory by export. Skips, flags, and errors
are operational events handled by watch counters and the flag tables. A single
`placed_by` column (`export`, `watch_auto`, `watch_manual`) replaces the two
separate columns.

**Rationale:** file_history is content history, not an event log. A user
rolling back a file needs the content chain, not the skip log. Skips and
flags already have their own representations (watch report counters,
`watch_match_flags`). Three `placed_by` values cover every writer without
ambiguity: export always writes, watch auto-places on single claim, watch
manual places on user approval. Two columns encoded the same information
with more combinations than actual states.

See: File history contract in `6_VERSIONING.md`.

**Rejected:** logging all events (bloats the table, mixes content history with
operational telemetry). Separate `pipeline` + `detection_method` columns
(four combinations, only three are real states).

### ADR-041: Watch config recorded in export_state; watch never reads live recipes at runtime

**Context:** watch originally read `DEPTH_TOLERANCE` and `OVERRIDE` chains
from the live recipe's WATCH block, and resolved reverse transform versions
from live transform rows. This meant a recipe edit or app upgrade between
export and return could change the reverse path, violating ADR-001 ("watch
resolves against the record, not the current recipe").

**Choice:** export records all watch-relevant config in the export_state row.
Per-key: `derived_reverse_chain` (forward chain reversed, each reversible
transform mapped to its reverse at the recorded version and args) and
`override_chain` (from the recipe's WATCH OVERRIDE block, resolved with
versions and args; null if no override). Recipe-level: `depth_tolerance`
(from the caller recipe's WATCH block; default 2; invoked recipes'
DEPTH_TOLERANCE is ignored with a lint warning). Watch reads these fields
from the export_state record. Watch never reads a recipe's WATCH block or
a transform row by name at runtime.

**Rationale:** strengthens ADR-001. The export_state row now fully describes
the return path: rules, reverse chains, override chains, depth tolerance,
template set versions. A recipe edit, transform upgrade, or rollback between
export and return cannot change what watch does. Pointer moves still reload
template sets (for extraction patterns) but no longer reload WATCH config.

See: Export state contract in `4_EXPORT.md`, Watch resolution state contract
in `5_WATCH.md`, Return step contract in `5_WATCH.md`.

**Rejected:** continuing to read live WATCH config at runtime (breaks
ADR-001 on any edit). Recording only forward chains and deriving reverses at
watch time (requires watch to resolve transform `reverses` declarations,
which can change between versions).


### ADR-042: Cross-process coordination via state_generation

**Context:** the Tauri desktop app and the CLI are separate processes sharing
one SQLite database. When the CLI runs an export or changes a binding, a
watch session in the app process does not see the change until its next
timer-driven reload. In-process triggers (channel messages on export success,
binding change, pointer move) only work when app and watch share a process.

**Choice:** a `state_generation` integer counter in a dedicated single-row
table. Every export success, binding activate/deactivate/delete, and entity
pointer move bumps it inside the same transaction as the change. Watch polls
`state_generation` on its existing 30s trie-refresh timer. If the value has
changed since the last check, watch performs the same reload as the
in-process triggers. In-process triggers remain as a latency optimization.

**Rationale:** polling is simple and bounded (one integer read every 30s).
The counter is updated transactionally with the change that matters, so
watch never sees a bumped counter without the data it references being
committed. No file watches, no IPC, no pub/sub. The 30s worst-case latency
matches the trie refresh window and is acceptable for CLI-driven changes.
All SQLite connections set `busy_timeout` (e.g. 5000ms) to handle
cross-process write contention.

See: Database contract in `1_INFRASTRUCTURE.md`, Watch resolution state
contract in `5_WATCH.md`.

**Rejected:** filesystem watches on the database file (WAL makes this
unreliable). Unix domain socket or named pipe (platform-specific, adds IPC
code). Shared memory (complexity for a 30s poll). Immediate notification
(unnecessary; 30s is fine for cross-process).


### ADR-043: Reject concurrent export of same binding

**Context:** the original design allowed two concurrent exports of the same
binding, each writing to a different UUID directory, with last successful row
replacement winning. This created a race: start-of-run cleanup could delete
a concurrent run's directory, and the "last wins" row replacement was
non-deterministic.

**Choice:** per-binding lock file at
`{app_data}/export/binding-{binding_id}/.lock`. Export acquires at start,
releases at end (success or failure). A second export of the same binding
while the lock is held returns a domain error. Different bindings export
concurrently.

**Rationale:** eliminates the race by construction. Start-of-run cleanup
(delete orphan directories) is now safe because no concurrent export can be
writing to a sibling directory. The lock is a file (not a database row) so
it works across processes (app + CLI) without SQLite contention. A crash
leaves a stale lock file; the existing startup cleanup (delete directories
not matching export_state.output_dir) can also clean stale locks.

See: Export flow diagram and Export folder contract in `4_EXPORT.md`,
Filesystem table in `1_INFRASTRUCTURE.md`.

**Rejected:** database-level advisory lock (adds write contention for a
filesystem concern). Allowing concurrent exports with conflict detection
(complexity without benefit; the user never needs two exports of one binding
simultaneously). Queue-based serialization (over-engineered for the
frequency of concurrent attempts).


### ADR-044: Independent flag detection; no re-drop replacement

**Context:** the original design used `content_hash` to identify re-drops
(a new download event with changed size or mtime for content that already had
a flag). A re-drop replaced the existing flag with a fresh detection. This
required computing a content hash for every incoming file and matching it
against existing flags.

**Choice:** each detection creates a new independent flag row. No replacement
logic. No `content_hash` needed. Same filename + same size/mtime = seen-set
skip (no re-detection). Same filename + different size/mtime = new flag.
Different filename = new flag. Users dismiss stale flags manually.

**Rationale:** re-drop replacement added complexity (content hashing,
flag lookup, transactional replace) for a narrow edge case (user
re-downloads the same file while a flag is pending). The independent model
is simpler: detection is append-only, the seen-set handles the common case
(identical re-download), and stale flags are cheap (dismiss is one click).
Removing `content_hash` also eliminates hashing every incoming file during
detection, which is wasted work for the vast majority of files that are
auto-placed.

See: Watch match flags contract in `5_WATCH.md`, ADR-006 (updated).

**Rejected:** re-drop replacement via content_hash (complexity for a narrow
case). Automatic expiry of stale flags (time-based expiry could delete a
flag the user hasn't reviewed yet). Re-detection on flag dismiss (would
require re-running detection, which may produce different results if
pipeline config changed).
