<!-- docs/VOCABULARY.md -->
# Vocabulary

Terms used in Flatten PM's design. Each entry: one-sentence definition, then
the contract or ADR that governs it.

---

**ambiguous (flag type)** — A watch match flag where multiple claims disagree on the target; candidate rows carry each binding's opinion. See Watch flow (flag types) in `5_WATCH.md`.

**arena** — The in-memory trie representation: a `Vec<Node>` with index-based references, cache-friendly and serializable to MessagePack. See Trie contract in `2_INGEST.md`.

**arg_values** — JSON column on `pipeline_bindings` holding the required ARG values and default overrides that complete a recipe for a specific binding. See Binding contract in `3_RECIPES.md`.

**basename normalization** — Stripping browser dedupe suffixes (` (1)`, ` (2)`) and appended `.txt` from a downloaded filename before comparing it against the export folder index. See Reconciliation contract in `5_WATCH.md`.

**binding (pipeline binding)** — The activation record for a recipe: one recipe plus ARG values, the "on switch" that produces independent exports and watch behavior. See Binding contract in `3_RECIPES.md`.

**build recipe** — A block-structured text file describing how to export a set of repos; the text in `build_recipe_versions.source` is the only stored form. See Recipe grammar contract in `3_RECIPES.md`.

**builtin protection** — The rule that `curation=builtin` entities cannot be soft-deleted or hard-deleted; edits insert new versions, and restore-from-shipped reads the embedded binary content. See Versioning contract in `6_VERSIONING.md`.

**candidate** — A target `(repo, source_path, binding, COPY key)` proposed by one binding's rule match during watch resolution; candidates agreeing on `(repo_id, source_path)` merge into a claim. See Resolution contract in `5_WATCH.md`.

**chain (file_history)** — A sequence of snapshot and diff entries for one file in one pipeline, keyed by `(placed_by, repo_id, file_path)`, with a snapshot every 5th entry. See File history contract in `6_VERSIONING.md`.

**claim** — The result of merging candidates that agree on `(repo_id, source_path)`; one claim places the file, multiple claims flag it as ambiguous. See Resolution contract in `5_WATCH.md`.

**committed comment** — A line at the start of an existing repo file that matches an extraction pattern (a directory comment the repo already had), preserved after the reverse chain so a round trip never deletes repo content. See Enrichment contract in `3_RECIPES.md`.

**config ejection** — The pattern of seeding configuration from `.gitignore` at registration, then letting the user own the stored flat pattern list from that point forward. See Ingest rules contract in `2_INGEST.md`. ADR-029.

**context accumulator** — A run-scoped key-value log built during export via `ctx.context.note(key, value)`, ordered by `(instruction_position, path)`, rendered to `_CONTEXT.yaml` by the context-manifest transform. See Context accumulator contract in `4_EXPORT.md`.

**context-manifest (builtin)** — A builtin directory transform that renders the context accumulator to `_CONTEXT.yaml` through a `file`-kind template. See Builtin transforms contract in `3_RECIPES.md`.

**COPY block** — A block inside a `SOURCE` that maps files from one source prefix to a destination prefix, with per-file excludes and an optional transform chain override. See Recipe grammar contract in `3_RECIPES.md`.

**COPY key (AS)** — The mandatory stable name on a `COPY` block (`AS <key>`), used in export state records, `WATCH` `OVERRIDE` references, and watch match candidate rows. See Recipe grammar contract in `3_RECIPES.md`.

**COPY_DEFAULT_WITH** — A recipe-level instruction setting the default per-file transform chain for all `COPY` blocks; sequential (second declaration overrides), scoped to the declaring recipe. See Recipe grammar contract in `3_RECIPES.md`.

**copy_key** — The `COPY` block key stored on a `watch_match_candidates` row, identifying which `COPY` block's rules produced the candidate. See Watch match candidates contract in `5_WATCH.md`.

**crash recovery** — On startup, deleting any directory under `binding-{binding_id}/` that does not match the current `export_state.output_dir`, removing orphans from incomplete exports. See Export folder contract in `4_EXPORT.md`.

**ctx.context.note** — The transform API method that appends a key-value entry to the run-scoped context accumulator, ordered by instruction position and path. See Transform contract in `3_RECIPES.md`.

**ctx.render** — The transform API method that renders a template through MiniJinja, used by enrichment-injection and context-manifest. See Transform contract in `3_RECIPES.md`.

**curation** — A field on `transforms` and `templates` distinguishing `builtin` (protected from deletion, upgradable) from `custom` (user-created, deletable). See Transform contract in `3_RECIPES.md`. ADR-036.

**DEPTH_TOLERANCE** — The maximum number of missing path levels (file + parent directories) for watch auto-placement, set per-recipe in the `WATCH` block, default 2. See Recipe grammar contract in `3_RECIPES.md`. ADR-034.

**dest_path** — A file's path in the run folder at the moment enrichment is injected; the enrichment carries it, and watch extracts it to find where the file belongs. See Enrichment contract in `3_RECIPES.md`.

**directory transform** — A transform with `scope=directory` that reshapes the run folder (list, read, write, move, remove) without editing sourced file content; `reverses` is ignored on directory transforms. See Transform contract in `3_RECIPES.md`.

**directory transform API** — The QuickJS API surface for directory transforms: `list()`, `read(path)`, `write(path, content)`, `move(from, to)`, `remove(path)`. See Transform contract in `3_RECIPES.md`.

**emitted file** — A file created by a directory transform (pack, context-manifest, or custom) during export; written to the run folder but not tracked in `resolution_rules` because it is a transient artifact of the run. See Export state contract in `4_EXPORT.md`.

**enrichment** — Metadata injected into exported files by `enrichment-injection` so they carry their `dest_path` and can be recognized and placed back by watch. See Enrichment contract in `3_RECIPES.md`. ADR-009.

**enrichment-injection (builtin)** — A builtin file transform that injects enrichment from a template set into each file's head, rendering the body with the file's `dest_path`. See Builtin transforms contract in `3_RECIPES.md`.

**enrichment-trim (builtin)** — A builtin file transform that strips enrichment by matching against extraction patterns; declares `reverses enrichment-injection`. See Builtin transforms contract in `3_RECIPES.md`.

**entry_type** — A column on `file_history` distinguishing `snapshot` (full content) from `diff` (delta against the previous entry). See File history contract in `6_VERSIONING.md`.

**error model** — The `flatten-core` error enum with five categories (domain, usage, io, transform, database), each mapping to a CLI exit code and JSON serialization. See Error model contract in `1_INFRASTRUCTURE.md`.

**exit codes** — CLI exit conventions: 0 for success, 1 for domain or IO errors, 2 for usage errors. See CLI contract in `1_INFRASTRUCTURE.md`.

**export** — The "build" stage: running a recipe's instructions in order on a fresh directory, producing the export folder and replacing the export state row. See Export flow contract in `4_EXPORT.md`.

**export directory** — The fresh UUID directory (`binding-{id}/{uuid}/`) that one export builds into; on success it becomes the export folder, on failure it is deleted. See Export folder contract in `4_EXPORT.md`.

**export folder** — The app-owned directory at `{app_data}/export/binding-{binding_id}/{export_uuid}/` holding what was last uploaded; written only by export, read by watch for reconciliation. See Export folder contract in `4_EXPORT.md`.

**export folder index** — The basename set built at watch session start from each active binding's export folder, re-indexed on pipeline config changes, used for reconciliation. See Reconciliation contract in `5_WATCH.md`.

**export state** — The record of a binding's last successful export, stored as one row per binding in `export_state`, replaced on each successful export; read by short-circuit and watch. See Export state contract in `4_EXPORT.md`. ADR-037.

**extraction pattern** — A regex derived from a `(template_version, entry)` pair, matching the enrichment markers and body with `{{path}}` as the capture group; patterns from all versions are active so files exported under older versions still match. See Enrichment contract in `3_RECIPES.md`.

**file_history (table)** — The s2 append-only audit trail of placed files, storing post-transform content as snapshots and diffs per file. See File history contract in `6_VERSIONING.md`. ADR-020, ADR-040.

**file transform** — A transform with `scope=file` that operates on one file at a time (read, write, rename), used in `COPY` chains; may optionally declare `reverses`. See Transform contract in `3_RECIPES.md`.

**file transform API** — The QuickJS API surface for file transforms: `read()`, `write(content)`, `rename(name)`. See Transform contract in `3_RECIPES.md`.

**fixtures convention** — The CLI testing layout: `fixtures/<name>/` for repos, `fixtures/recipes/*.recipe` for recipe files, `fixtures/transforms/*.js` for transforms, `fixtures/downloads/` for watch inputs. See CLI contract in `1_INFRASTRUCTURE.md`.

**flag** — A file with enrichment that could not be auto-placed; frozen at detection, resolved by approve, dismiss, or re-drop. See Watch match flags contract in `5_WATCH.md`. ADR-006.

**flatten (builtin)** — A builtin directory transform that moves files to flat encoded names using a configurable delimiter and percent-encoding for injectivity. See Builtin transforms contract in `3_RECIPES.md`.

**flatten CLI** — The `flatten` binary in `src-cli/`, a thin subcommand dispatch over `flatten-core` functions serving as the development entry point. See CLI contract in `1_INFRASTRUCTURE.md`.

**flatten encoding** — The percent-encoding scheme the flatten transform uses for injectivity: encoding `%`, delimiter literals, leading `_`, and OS-illegal characters, with dotfile mapping (leading `.` to `_`). See Builtin transforms contract in `3_RECIPES.md`.

**flatten-core** — The standalone Rust library crate (`crates/flatten-core/`) containing all pipeline logic, storage, and the transform runtime, with no Tauri or GUI dependencies. See Components section in `0_SUMMARY.md`. ADR-014.

**force (export)** — The user override that bypasses a short-circuit unchanged result to run a full export anyway. See Short-circuit contract in `4_EXPORT.md`.

**forward chain** — The per-file transform list in a `COPY` block (from `COPY_DEFAULT_WITH` or `OVERRIDE_WITH`), run during export; watch derives the reverse chain from it. See Recipe grammar contract in `3_RECIPES.md`.

**ingest** — The "read" stage: walking a registered source directory under its pattern list, building a trie of paths and BLAKE3 hashes. See Ingest flow contract in `2_INGEST.md`.

**ingest_patterns** — The per-repo JSON column holding a gitignore-style pattern list that controls what enters the trie, seeded from `.gitignore` or defaulting to `.git/` only. See Ingest rules contract in `2_INGEST.md`.

**instruction** — A recipe-level or `COPY`-level directive (ARG, `COPY_DEFAULT_WITH`, `SOURCE`, `COPY`, `RUN`, `INVOKE`, `WATCH`, `EXCLUDE`, `OVERRIDE_WITH`). See Recipe grammar contract in `3_RECIPES.md`.

**--json** — CLI flag that emits machine-readable JSON on stdout, wrapping errors with a `kind` discriminator and structured detail. See CLI contract in `1_INFRASTRUCTURE.md`.

**last match wins** — The gitignore-style evaluation rule for watch resolution: all recorded `COPY`/`EXCLUDE` rules are evaluated in instruction order, and the last matching rule decides. See Resolution contract in `5_WATCH.md`.

**leaf (trie)** — A trie node representing a file: `(path, content_hash)` where the hash is BLAKE3 over raw file bytes with no normalization. See Trie contract in `2_INGEST.md`.

**line-ending policy** — A per-repo setting (`repos.line_ending_policy`): `preserve` (match existing target file's ending, LF for new files) or `lf` (always LF), applied by watch at placement. See Ingest rules contract in `2_INGEST.md`. ADR-035.

**map_generation** — A monotonic counter in the watch session, incremented on every resolution state reload, exposed in the session start report and `watch status` output. See Watch resolution state contract in `5_WATCH.md`.

**mapping instruction** — `COPY` is the only instruction that determines a file's `dest_path`; nothing after `COPY` changes `dest_path`. See Recipe grammar contract in `3_RECIPES.md`.

**Merkle hash** — A directory node's hash computed over sorted `(child_name, child_hash)` pairs using BLAKE3; the root hash identifies the whole repo state for the short-circuit. See Trie contract in `2_INGEST.md`.

**new_directory (flag type)** — A watch match flag where the target path exceeds the recipe's depth tolerance. See Watch flow (flag types) in `5_WATCH.md`.

**OVERRIDE_WITH** — A `COPY`-level instruction that replaces `COPY_DEFAULT_WITH` for that block; `OVERRIDE_WITH []` means no per-file transforms. See Recipe grammar contract in `3_RECIPES.md`.

**pack (builtin)** — A builtin directory transform that merges files into fewer output files, supporting XML (Repomix convention) or Markdown format with file-limit and byte-limit options. See Builtin transforms contract in `3_RECIPES.md`.

**pack formats** — The two output formats for the pack transform: XML (CDATA wrapping, control character stripping) or Markdown (fence length one longer than the longest backtick run). See Builtin transforms contract in `3_RECIPES.md`.

**partial-download suffix** — File extensions (`.crdownload`, `.part`, `.download`, `.partial`, `.tmp`) that cause watch to skip a file as still downloading. See Download detection contract in `5_WATCH.md`.

**pipeline_bindings (table)** — The SQLite table holding binding records: recipe FK, version pin, active flag, and `arg_values`. See Binding contract in `3_RECIPES.md`.

**placed_by** — A column on `file_history` recording which writer placed the file: `export`, `watch_auto`, or `watch_manual`. See File history contract in `6_VERSIONING.md`. ADR-040.

**pointer-based versioning** — The versioning model where a parent entity holds a `current_version_id` FK; edit inserts a new version row and moves the pointer, rollback moves the pointer to an older row. See Versioning contract in `6_VERSIONING.md`. ADR-016.

**PRAGMA user_version** — The SQLite pragma used for schema versioning; forward migrations run on startup inside `BEGIN IMMEDIATE` to prevent CLI/app races. See Database contract in `1_INFRASTRUCTURE.md`.

**read window** — The byte range read from a file's head during enrichment detection, derived from the extraction pattern length plus a buffer for reserved prefix and path length. See Enrichment contract in `3_RECIPES.md`.

**recipe type** — There is one recipe type; a binding activates a recipe, export is the run, and watch is the standing inverse of every active binding's last export. ADR-001.

**reconciliation** — The unchanged-re-download check during the periodic sweep: a download whose normalized basename and hash match an exported file is skipped. See Reconciliation contract in `5_WATCH.md`. ADR-010.

**re-drop** — When watch receives a download event with changed size or mtime for a file that already has a flag, the old flag is replaced with a fresh detection. See Watch flow in `5_WATCH.md`.

**re-ingest** — An idempotent trie rebuild for an existing repo, called by export (every `SOURCE` repo before a run) and watch (every active repo at session start, then on a timer). See Trie contract in `2_INGEST.md`.

**repo_root_hashes** — A JSON field on `export_state` mapping each repo_id to its trie Merkle root hash at run start, used by the short-circuit to detect repo changes. See Export state contract in `4_EXPORT.md`.

**repo_versions (table)** — The append-only config history table for repos, snapshotting `ingest_patterns`, `line_ending_policy`, and `safety_allowlist` per edit; the exception to pointer-based versioning. See Versioning contract in `6_VERSIONING.md`. ADR-016.

**repos (table)** — The SQLite table for registered source directories: path, name, `ingest_patterns`, `line_ending_policy`, `trie_updated_at`, `deleted_at`. See Repos contract in `2_INGEST.md`.

**reserved prefix** — Content that must stay on line 1 of a file (shebang, `"use strict"`, `<?xml`, YAML `---`, etc.); enrichment injection lands after it. See Template contract in `3_RECIPES.md`.

**resolution** — The process of matching an incoming file's `dest_path` against every active binding's recorded `COPY` rules, producing candidates, merging claims, and deciding place/flag. See Resolution contract in `5_WATCH.md`.

**resolution_rules** — A JSON field on `export_state` recording per `COPY` key the repo, source/dest prefixes, excludes, and forward chain with versions; read by watch for resolution and reverse. See Export state contract in `4_EXPORT.md`.

**return step** — The processing between watch approval and file placement: enrichment-trim, custom file reverses (or `WATCH` `OVERRIDE`), committed comment preservation, and line-ending policy. See Return step contract in `5_WATCH.md`.

**reverse chain (derived)** — The chain watch runs on a returning file, derived from the forward chain's declared `reverses` in reverse order, with non-reversible transforms skipped. See Watch configuration section in `3_RECIPES.md`.

**run folder** — The fresh directory one export builds into (`binding-{id}/{uuid}/`), which becomes the export folder on success or is deleted on failure. See Export folder contract in `4_EXPORT.md`.

**run report** — The structured output of an export: counts of written, unchanged, deleted files, plus warnings, errors, and per-phase durations. See Run report contract in `4_EXPORT.md`.

**runtime_inputs** — A JSON field on `export_state` recording the resolved ARG values plus settings (`copy_size_limit_mb`, `binary_extensions`) that shaped the run, used by the short-circuit. See Export state contract in `4_EXPORT.md`.

**runtime_versions** — A JSON field on `export_state` recording the version snapshot of all entities (recipes, directory transforms, template sets), used by the short-circuit and watch. See Export state contract in `4_EXPORT.md`.

**safety_findings (table)** — The s2 table storing export-time content safety scan results (secrets, PII), linked to `export_state` with CASCADE on delete. See Export flow (content safety) in `4_EXPORT.md`. ADR-030.

**sandbox** — The QuickJS-NG runtime restrictions: no filesystem outside the run folder, no network, no process, no timers, no imports; one fresh context per invocation with deterministic `Date.now` and `Math.random`. See Transform contract in `3_RECIPES.md`.

**scope (transform field)** — A field on `transforms` distinguishing `file` (per-file API: read, write, rename) from `directory` (folder API: list, read, write, move, remove). See Transform contract in `3_RECIPES.md`. ADR-036.

**seen-set** — Watch's in-memory set of examined files, keyed by `(source_path, size, mtime)`; a hit skips detection, a miss examines the file, and a re-download (same path, changed size or mtime) triggers re-examination. See Download detection contract in `5_WATCH.md`.

**session scope** — The rule that watch evaluates only files whose mtime is after the session start; files present before the session started are never examined. See Download detection contract in `5_WATCH.md`.

**settings (table)** — Global key-value configuration in SQLite: 9 keys with defaults controlling watch behavior, transform limits, and export size limits. See Settings contract in `1_INFRASTRUCTURE.md`.

**settle check** — The download-readiness test: size and mtime unchanged across consecutive checks, and size non-zero; a file still being written is skipped. See Download detection contract in `5_WATCH.md`.

**shipped defaults** — Content embedded in the binary (5 transforms, 2 templates, 1 recipe, settings defaults), seeded into SQLite on first launch or if a builtin entity is missing at startup. See Shipped defaults contract in `6_VERSIONING.md`.

**short-circuit** — The export's only cache: compares `runtime_inputs`, `repo_root_hashes`, `resolution_rules`, and `runtime_versions` against the previous run; all equal means interactive accept/force, any differ means full export. See Short-circuit contract in `4_EXPORT.md`.

**soft delete** — Setting a `deleted_at` timestamp on `repos`, `build_recipes`, `transforms`, or `templates`; queries filter `deleted_at IS NULL`, and builtin rows cannot be soft-deleted. See Database contract in `1_INFRASTRUCTURE.md`.

**source (column)** — The authoritative text stored in a version row (`build_recipe_versions.source`, `transform_versions.source`, `template_versions.source`); the word "snapshot" is reserved for `file_history`. See Versioning contract in `6_VERSIONING.md`.

**SOURCE block** — A recipe-level block that ingests a named repo and scopes the `COPY` blocks inside it to that repo. See Recipe grammar contract in `3_RECIPES.md`.

**stage (s1/s2/s3)** — The development stages: s1 is the core engine plus CLI, s2 is the desktop UI plus product features, s3 is v2+ (MCP, forge daemon); v1 = s1 + s2. See Stages section in `0_SUMMARY.md`.

**state reload** — Watch's mechanism for picking up changes: reloads resolution rules, template sets, basename index, trie, or WATCH config on specific triggers (export success, binding changes, pointer moves, repo changes, new directory placement). See Watch resolution state contract in `5_WATCH.md`.

**syntax-safe flag** — A template set entry property: true for a real comment marker, false for a JSON field or YAML key; controls eligibility for `--reversible=false` on enrichment-injection. See Template contract in `3_RECIPES.md`.

**template (file kind)** — A versioned MiniJinja text template (`kind=file`) rendered through `ctx.render`, used by context-manifest and custom transforms. See Template contract in `3_RECIPES.md`.

**template set** — A versioned collection of enrichment entries (`kind=enrichment`) stored as YAML in `template_versions.source`, defining how files of different types receive enrichment. See Template contract in `3_RECIPES.md`. ADR-005.

**three stores** — The three persistence layers: trie cache (paths and hashes), export folder (files on disk), and export state (SQLite row recording how the folder was made). See Three stores section in `0_SUMMARY.md`.

**transform** — A JavaScript module executed in QuickJS-NG that operates on files; two scopes (file and directory) with distinct APIs and roles. See Transform contract in `3_RECIPES.md`. ADR-007.

**transform imports** — The s1 capability for one transform to call another; the import mechanism is a contract-level decision to settle during implementation. See Transform contract in `3_RECIPES.md`. ADR-033.

**trie** — A per-segment path trie with BLAKE3 content hashes at leaves and Merkle hashes at directories, representing everything a registered repo contains now. See Trie contract in `2_INGEST.md`.

**trie cache** — The on-disk trie file at `{app_data}/tries/{repo_id}.trie`, serialized as MessagePack, written atomically via tempfile+rename with debounced persistence. See Trie contract in `2_INGEST.md`.

**two exclusion layers** — Ingest exclude (per-repo pattern list keeping the trie clean) and `COPY` `EXCLUDE` (per-block pattern keeping the export clean); two layers, two concerns. See Ingest rules contract in `2_INGEST.md` and Recipe grammar contract in `3_RECIPES.md`.

**unroutable (flag type)** — A watch match flag where the file has enrichment but no active binding claimed it. See Watch flow (flag types) in `5_WATCH.md`.

**upgrade** — On startup, if the embedded shipped content for a builtin entity differs from its current version, a new version is inserted and the pointer moves; automatic. See Versioning contract in `6_VERSIONING.md`.

**version_pin** — A nullable int on `pipeline_bindings` and on `INVOKE`/`RUN` instructions; null means follow `current_version_id`, a value pins a specific version number (`@N` syntax). See Binding contract in `3_RECIPES.md`.

**WAL mode** — SQLite's write-ahead log mode, enabling concurrent readers with one writer; periodic checkpoints flush the WAL when no export is active. See Database contract in `1_INFRASTRUCTURE.md`.

**watch** — The "write-back" stage: monitoring a source directory, detecting enrichment in incoming files, resolving against recorded `COPY` rules, and placing files back in repos. See Watch flow contract in `5_WATCH.md`.

**WATCH block** — An optional recipe-level block configuring watch behavior: `DEPTH_TOLERANCE` and per-`COPY`-key reverse chain `OVERRIDE`s. See Recipe grammar contract in `3_RECIPES.md`.

**watch_match_candidates (table)** — Per-flag proposed targets in SQLite, recording each binding's opinion on where a flagged file should go; frozen at detection, with SET NULL on binding or version delete. See Watch match candidates contract in `5_WATCH.md`.

**watch_match_flags (table)** — SQLite table for files with enrichment that could not be auto-placed: one row per detection, carrying the full file content, flag type, source path, and extracted path. See Watch match flags contract in `5_WATCH.md`. ADR-006.

**WATCH OVERRIDE** — A sub-block of `WATCH` that replaces the derived reverse chain for a named `COPY` key, running a custom chain instead of steps 1-2 of the return step. See Recipe grammar contract in `3_RECIPES.md`.

**watch report** — The session start report (per-binding status, per-repo status, pattern count, generation) plus running counters (placed, skipped, flagged, errors). See Watch report contract in `5_WATCH.md`.

**watch resolution state** — The data watch loads and resolves against: resolution rules per binding, template sets, repo tries, export folder basename index, and WATCH config; reloaded on specific triggers. See Watch resolution state contract in `5_WATCH.md`.

**writer thread** — The single dedicated thread owning the SQLite write connection, fed by a channel whose messages are whole transactions, serializing all database writes. See Database contract in `1_INFRASTRUCTURE.md`. ADR-038.
