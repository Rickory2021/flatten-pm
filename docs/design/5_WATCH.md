<!-- docs/design/5_WATCH.md -->

# Watch

Watch monitors a configured source directory (typically Downloads) and serves
every active binding at once. For each incoming file:

1. **Detect enrichment.** Match the file's head against enrichment extraction
   patterns. Check the template sets recorded in active export states first (the
   likely match), then fall back to patterns from all template set versions
   (files exported under an older version must still match on return).
2. **Extract `dest_path`.** The enrichment carries the `dest_path` the export
   wrote. No enrichment: skip (not our file).
3. **Resolve candidates.** Run every active binding's recorded `COPY` blocks
   against `dest_path`. Each `COPY` block's excludes and prefix rules produce a
   candidate `(repo, source_path)` or exclude. The repo trie validates each
   candidate (path exists, missing directory depth within limit).
4. **Decide.** Candidates that agree on `(repo, source_path,
   resolved_reverse_chain)` merge into one claim. Same target but different
   reverse chain = ambiguous (flagged with one candidate per chain). One claim:
   approve. Multiple claims: flag. Zero claims: flag with the set-aside reason.
5. **Reverse.** On approval, check the `COPY` key's recorded `override_chain`
   (from export_state). If present, run it. Otherwise, run the
   `derived_reverse_chain`. Apply line-ending policy.
6. **Place.** Write the file to the repo atomically. Update the trie.

Nothing is guessed. Nothing ranks.


## Watch Flow


Referenced by: Resolution contract, Watch resolution state contract, Return step
contract, Watch match flags contract, Download detection contract.

One session per process. Serves every active binding. Handles only files that
appear after the session starts. Two listeners feed candidates into one
detection path.

```mermaid
flowchart TD
    subgraph "How new files are noticed"
        notify["OS file events<br/>(fast, sub-second,<br/>but can miss files under load)"]
        poll["Periodic folder scan<br/>(every 30s, newest first;<br/>catches what OS events missed)"]
        rescan["OS signals event overflow"]
        rescan --> poll
    end

    notify --> candidate
    poll --> candidate

    candidate{"Candidate file"}
    candidate --> seen{"Already seen?<br/>(filename, size, mtime)"}
    seen -->|yes| skip_seen["Skip"]
    seen -->|no| ready{"File ready?<br/>(no partial suffix,<br/>size + mtime stable, non-zero)"}
    ready -->|no| skip_ready["Skip"]
    ready -->|yes| detect

    detect["Read file head and extract enrichment<br/>(lossy UTF-8, strip BOM/invisible;<br/>binary rejected; malformed path rejected)"]
    detect --> matched{"Enrichment<br/>found?"}
    matched -->|no| skip_notenriched["Skip (not our file)"]

    matched -->|yes| resolve["Per-binding resolution"]
    resolve --> binding_loop

    subgraph "Per binding (each active binding with an export state row)"
        binding_loop{"Next binding"}
        binding_loop --> rules["Run recorded COPY/EXCLUDE rules<br/>in instruction order against dest_path"]
        rules --> last_match{"Last matching rule?"}
        last_match -->|EXCLUDE or no match| binding_done["No candidate (reason noted)"]
        last_match -->|COPY matches| target["Target: (repo, source_path)"]
        target --> depth{"Target within<br/>depth_tolerance?<br/>(from runtime_versions)"}
        depth -->|no| binding_done
        depth -->|yes| add_candidate["Add candidate:<br/>(repo, source_path, binding, COPY key)"]
        add_candidate --> more_bindings
        binding_done --> more_bindings
    end

    more_bindings{"More bindings?"}
    more_bindings -->|yes| binding_loop
    more_bindings -->|no| merge

    merge["Merge: candidates agreeing on<br/>(repo_id, source_path, resolved_reverse_chain)<br/>= one claim"]
    merge --> decide{"How many claims?"}

    decide -->|one| approve["Approved: identified COPY key,<br/>repo, source_path, matched pattern"]
    decide -->|multiple claims| flag_ambiguous["Flag: ambiguous<br/>(candidates disagree; detail in candidate rows)"]
    decide -->|zero, reasons exist| flag_aside["Flag: highest-precedence reason<br/>(new_directory)"]
    decide -->|zero, no reasons| flag_unroutable["Flag: unroutable<br/>(has enrichment, no active binding claimed it)"]

    approve --> reverse

    subgraph "Reverse step"
        reverse{"override_chain non-null<br/>in resolution_rules?"}
        reverse -->|yes| run_override["Run override chain<br/>(from export_state record)"]
        reverse -->|no| run_derived["Run derived_reverse_chain<br/>(from export_state record)"]
        run_override --> line_ending
        run_derived --> line_ending
        line_ending["Apply line-ending policy:<br/>(per-repo setting: preserve or lf)"]
    end

    line_ending --> place

    subgraph "Placement"
        place["Canonicalize target path's parent directory.<br/>If canonical path is outside repo root, refuse (error)."]
        place --> write["Atomic write:<br/>tempfile (.flatten-tmp-*) + rename"]
        write --> mode["Preserve existing target mode<br/>or umask default + shebang exec bit"]
        mode --> trie_update["Re-hash placed file,<br/>copy-on-write trie update,<br/>swap Arc"]
        trie_update --> dir_created{"Created a<br/>new directory?"}
        dir_created -->|yes| bump_gen["Bump map_generation"]
        dir_created -->|no| delete_source
        bump_gen --> delete_source
        delete_source["Delete source file from<br/>source directory"]
        write -.->|failure (disk, permissions, escaped repo)| place_fail["Log error, increment error count;<br/>source file stays"]
    end

    delete_source --> update_seen["Update seen-set, increment placed count"]
    update_seen --> done(["Done"])
    place_fail --> done

    flag_ambiguous --> update_flag["Write watch_match_flag + candidates to SQLite"]
    flag_aside --> update_flag
    flag_unroutable --> update_flag
    update_flag --> update_seen_flag["Update seen-set, increment flagged count"]
    update_seen_flag --> done

    skip_seen --> done_skip(["Skip"])
    skip_ready --> done_skip
    skip_notenriched --> done_skip

    style flag_ambiguous fill:#fdd
    style flag_aside fill:#fdd
    style flag_unroutable fill:#fdd
    style approve fill:#dfd
    style skip_seen fill:#eee
    style skip_ready fill:#eee
    style skip_notenriched fill:#eee
```

**Key properties:**
- Pass or fail. Nothing ranks. An exact match and a one-level-new match are
  peers; peers flag.
- Qualification is per binding. Merge collapses agreement. Disagreement flags.
- Flags are written in one transaction (flag row + all candidate rows).
- The seen-set prevents re-detection of unchanged files. Same filename + same
  size/mtime = seen-set skip. Same filename + different size/mtime = new flag
  (independent detection; no replacement of existing flags). Different filename
  = new flag. Users dismiss stale flags manually.
- Reconciliation (poll listener): a download whose normalized basename and hash
  match an exported file is an unchanged re-download and is skipped.

**Watch match flags.** A flag is a file with enrichment that can't be
unambiguously placed. No enrichment is a skip (not our file). Flag exists =
pending; resolve = delete. Candidates are frozen at detection; deactivating or
deleting a binding does not re-resolve.

| Flag type | Meaning |
|---|---|
| `ambiguous` | Multiple claims disagree. Candidate rows carry the detail. |
| `new_directory` | Target exceeds depth tolerance. |
| `unroutable` | Has enrichment, no active binding claimed it. |

Precedence (when multiple reasons apply): `ambiguous` > `new_directory`.

**Flag resolution (s2, GUI):**

| Action | Applies to | What it does |
|---|---|---|
| **Approve** | `ambiguous`, `new_directory` | User picks a candidate. Run reverse, place, delete flag. |
| **Approve (manual)** | `unroutable` | User specifies target (repo, path). Run reverse, place, delete flag. |
| **Dismiss** | Any watch flag | Delete flag. Source file stays. |

**`flag explain <file>`:** runs detection and per-binding resolution against the
specified file without placing. Prints every rule evaluated, every candidate
with its status, the matched extraction pattern, and the reverse chain that
would apply. Nearly free once resolution is a pure function of the loaded
watch state and the file content. Part of the `flag` CLI subcommand group.

**Flag behaviors:** each detection creates a new independent flag row. No
replacement logic. A flag whose source file is gone from the source directory
can be dismissed or left to expire.

Note on `content` size: watch stores the full file. Files near
`copy_size_limit_mb` produce large rows. Acceptable because flags are few
and short-lived; approve or dismiss clears them.

Content safety findings are separate: see `safety_findings` table in
`4_EXPORT.md`.

## Download Detection


Shared rules between the two watch listeners (OS file events and periodic
folder scan).

**Partial-download suffixes (shared list):**

`.crdownload`, `.part`, `.download`, `.partial`, `.tmp`

A file with any of these suffixes is skipped.

**Binary detection (shared):**

The head read decodes as lossy UTF-8. A file whose first 2048 bytes contain a
null byte (0x00) is treated as binary and skipped. No extension-based check on
the watch side; the `binary_extensions` list applies only to `COPY` `EXCLUDE`
`--binary` during export.

**Settle check (shared):**

Size and mtime unchanged across consecutive checks, and size non-zero. A file
still being written is skipped.

**Concurrency:** watch processes files serially on a single thread. Trie
updates, flag writes, and seen-set mutations require no synchronization.
Concurrent processing is a measured optimization for a future stage.

**Session scope:**

Only files whose mtime is after the session start are candidates. Files present
before the session started are never evaluated.

**Seen-set:**

| Property | Value |
|---|---|
| Key | `(source_path, size, mtime)` |
| Hit | Skip detection. |
| Miss | Examine the file. |
| Re-download | Same filename, new size or mtime → examined again. |

**OS file events (notify 8.x with notify-debouncer-full):**

Non-recursive on the source directory. Create, modify, and rename-to events on
a non-partial name produce a candidate. Remove events ignored. Runs on its own
thread; handoff to the session via a channel. On the "rescan required" signal
(event overflow), triggers an immediate full scan.

**Periodic folder scan:**

Timer-driven (default 30s). Newest mtime first. Checks session scope and
seen-set before detection. Catches anything OS events missed.

## Resolution


How an incoming file finds its target. One operation: file in, decision out.
See the Watch flow activity diagram for the visual.

**Preconditions (handled before resolution):**

| Check | Result if failed |
|---|---|
| Already seen (filename, size, mtime) | Skip |
| File ready (no partial suffix, size+mtime stable) | Skip |
| Enrichment found in file head | Skip (not our file) |

**Steps (after enrichment extracted):**

| Step | What happens |
|---|---|
| 1. Extract dest_path | Normalize: strip `./`, `\` to `/`, trim trailing whitespace. Absolute paths and `..` rejected. |
| 2. Per-binding resolution | For each active binding with an export state row, run its recorded `COPY`/`EXCLUDE` rules in instruction order against dest_path. Last matching rule decides. |
| 3. `COPY` match | Produces a target `(repo, source_path)`. Check against repo trie: count missing path levels (file + parent directories). A path component that exists in the trie as a leaf is not a directory; a target path whose ancestor is a leaf counts the ancestor as a missing directory for depth purposes (e.g., `config/secrets` is a leaf symlink, target `config/secrets/key.yaml` has 2 missing levels, not 1). Within `depth_tolerance` (from `export_state.runtime_versions.depth_tolerance`): candidate. Exceeds: no candidate (reason: `new_directory`). |
| 4. `EXCLUDE` or no match | No candidate (reason noted). |
| 5. Merge | Candidates agreeing on `(repo_id, source_path, resolved_reverse_chain)` from any binding = one claim. Same target but different reverse chain = ambiguous (one candidate per chain). |
| 6. Decide | One claim: approve (identified `COPY` key, repo, source_path, matched pattern). Multiple claims: flag `ambiguous`. Zero claims with reasons: flag highest-precedence reason. Zero claims with no reasons: flag `unroutable`. |

**`COPY` matching rules:**

The `src` argument in the recorded `COPY` block determines the matching
algorithm. No `kind` column is stored; the recorded `src_prefix` and
`dest_prefix` carry the information.

| Matching type | Determination | How it matches dest_path |
|---|---|---|
| Prefix match | `src_prefix` is empty (from `.`) or either prefix ends with `/` | Anchored, segment-aligned. `dest/` matches `dest/x` but not `dest2/x`. Empty prefix (`COPY . .`) matches everything. Yields `(repo, src_prefix + remainder)`. |
| Exact match | Neither prefix is empty and neither ends with `/` | `dest == dest_path` exactly. Yields `(repo, src)`. |

Last matching rule wins (gitignore-style evaluation: all rules evaluated in
order, last match decides).

**Flag type precedence:** `ambiguous` > `new_directory`.

**Properties:**

- Pass or fail. Nothing ranks anywhere.
- An exact match and a one-level-new match are peers; peers from different
  bindings with different targets flag.
- New and existing files go through the same path.
- The filename on disk is never used for placement.
- There is no fallback that places a file no `COPY` rule authorized.

## Watch Resolution State


The data watch loads and resolves against. Loaded at session start, reloaded
on specific triggers.

**Loaded data:**

| What | Source | Used by |
|---|---|---|
| Resolution rules per binding | `export_state.resolution_rules` for every active binding with an export state row. Per-key: rules, `derived_reverse_chain`, `override_chain`. | Resolution (rules), return step (reverse chains, override chains) |
| Depth tolerance | `export_state.runtime_versions.depth_tolerance` | Resolution (depth check) |
| Template sets and versions | `export_state.runtime_versions.template_sets` from active runs, plus all versions of all enrichment template sets | Enrichment detection (extraction patterns) |
| Repo tries | Trie per active repo (loaded into memory) | Resolution (path validation, depth check) |
| Export folder basename index | Basenames of files in each binding's export folder | Reconciliation (unchanged re-download detection) |

Bindings without an export state row are listed in the start report and contribute
nothing to the loaded state.

**Reload triggers:**

| Trigger | What reloads | Why |
|---|---|---|
| Session start | Everything | Full initialization |
| Trie refresh timer (default 30s) | Repo tries only | Detect repo changes on disk |
| `change_counter` change (polled on trie refresh timer) | Resolution rules, template sets, basename index | Cross-process reload: another process (CLI) changed pipeline config. In-process triggers remain as a latency optimization when app and watch share a process (Tauri desktop). |
| Export success | Resolution rules (including reverse chains, override chains, depth tolerance), template sets, basename index | New export state with potentially different rules |
| Binding activate/deactivate/delete | Resolution rules, template sets, basename index | Pipeline set changed |
| Recipe/transform/template pointer move (edit, rollback, reset, upgrade) | Template sets | Extraction patterns may have changed (depth tolerance and reverse chains are recorded in export_state, not read from live recipes) |
| Repo registration or soft delete | Repo tries | Active repo set changed |
| New directory created during placement | Repo tries only | Ensures subsequent resolutions see the new directory immediately |

Note: export success reloads resolution rules, template sets, and the basename
index, but NOT repo tries. Export's re-ingest updates the trie on disk; the
trie refresh timer picks it up on its next tick. The 30s window means watch
may briefly resolve against a pre-export trie state.

**Cross-process coordination:** the `change_counter` counter is polled on
the same 30s trie-refresh timer. If the value has changed since the last
check, watch performs the same reload as the in-process triggers. In-process
triggers (export success, binding change, pointer move) remain as a latency
optimization when app and watch share a process; they are redundant with the
cross-process poll but fire immediately.

**`watch stop`:** reads `{app_data}/watch.pid`, sends SIGTERM (Unix) or writes
a stop-request file (cross-platform). The watch session deletes its PID file
on clean stop.

**`watch status`:** reads the PID file (liveness) and the running counters
from the database.

**Properties:**

- A reload never re-detects pending files or flags.
- Not persisted. A restart reloads from export state rows.
- `map_generation` is a monotonic counter incremented on every reload.
  Exposed in the watch session start report and `watch status` output.

## Return Step


What happens between detection approval and file placement. Operates on a
one-file temp folder using the same transform runtime as export.

**Steps in order:**

| Step | What happens | Condition |
|---|---|---|
| 1. Check override | Read `override_chain` from `resolution_rules` for this `COPY` key. If present, run the override chain (steps 2-3 skipped). | Only if `override_chain` is non-null. |
| 2. enrichment-trim | Strip the extraction pattern that matched during detection. | Skipped if the matched binding's `COPY` block recorded `--reversible=false` on enrichment-injection. |
| 3. Derived reverse | Run `derived_reverse_chain` from `resolution_rules` for this `COPY` key: the forward chain reversed, each reversible transform run at its recorded version and args. Non-reversible transforms already excluded at export recording time. | Only if no override and the derived chain is non-empty beyond enrichment-trim. |
| 4. Committed comment | If the file already on disk at the target begins with a line matching an extraction pattern, re-add that line after the chain. | Always checked. |
| 5. Line-ending policy | Normalize line endings per the repo's setting: `preserve` (match existing target file, LF for new files) or `lf` (always LF). | Always. |

All reverse chains and override chains are read from the export state record
(`resolution_rules`), not from live recipe WATCH blocks or live transform
rows. This ensures the return step uses exactly the transforms that produced
the export (ADR-001).

**Output:** bytes handed to placement. The return step never writes to the
repo directly.

**Placement hardening:** before the atomic write, canonicalize the target
path's parent directory. If the canonical path is outside the repo root,
refuse placement with an error. This catches symlinks that escaped ingest
(created after the last ingest).

**Failure:** a failing reverse fails the placement. The source file stays in
the downloads folder. Logged and counted as error.

## Reconciliation


The unchanged-re-download check during the periodic sweep. Prevents redundant
detection when a user re-downloads a file they already uploaded.

**Rule:** a download whose normalized basename matches an exported file's
basename (anywhere in the binding's export folder) and whose BLAKE3 hash
equals that file's hash is an unchanged re-download. Skipped.

**Basename normalization:** strip browser dedupe suffixes (` (1)`, ` (2)`) and
an appended `.txt`.

**Export folder index:** built at session start from each active binding's
export folder on disk. Re-indexed on pipeline config changes (state reload).

## Watch Match Flags


A file with enrichment that could not be auto-placed. One row per detection.

| Field | Type | What |
|---|---|---|
| `id` | int | PK |
| `flag_type` | text | `ambiguous`, `new_directory`, or `unroutable`. |
| `content` | blob | Full file content at detection time. |
| `source_path` | text | Path relative to the watch source directory. |
| `extracted_path` | text | The `dest_path` extracted from enrichment. |
| `created_at` | datetime | |

Flag exists = pending; resolve (approve or dismiss) = delete the flag row and
its candidates. See the Watch flow section for flag type definitions,
precedence rules, and resolution actions.

## Watch Match Candidates


One binding's resolution result for a flagged file. Every active binding with
an export state row writes a candidate row during resolution, not just
bindings that produced viable targets.

| Field | Type | What |
|---|---|---|
| `id` | int | PK |
| `watch_match_flag_id` | int FK | The parent flag. CASCADE on delete. |
| `pipeline_binding_id` | int FK, nullable | The binding whose rules produced this candidate. SET NULL on binding delete (frozen). |
| `build_recipe_version_id` | int FK, nullable | The recipe version that ran. SET NULL on version delete. |
| `repo_id` | int FK | Target repo. |
| `target_path` | text, nullable | Repo-relative path where this candidate would place the file. Null when status is `excluded` or `no_match` (no target was computed). |
| `copy_key` | text, nullable | Which `COPY` block matched. Null when status is `excluded` or `no_match`. |
| `status` | text | `ok`, `exceeds_depth`, `excluded`, or `no_match`. |
| `resolved_reverse_chain` | JSON | The reverse chain with versions and args that was active at detection time. Copied from `resolution_rules[copy_key].override_chain` (if present) or `.derived_reverse_chain`. Approve uses this, not the current export_state. |
| `template_set_version` | int, nullable | The template set version whose extraction pattern matched during detection. |

Uniqueness: `(watch_match_flag_id, pipeline_binding_id, repo_id)`.

Candidates are written once at detection and never replaced. A deactivated or
deleted binding's candidate stays on the flag (frozen). Approve reads
`resolved_reverse_chain` from the candidate row, not from the current
export_state.

## Watch Report


**Session start report:**

| Field | What |
|---|---|
| Per binding | Loaded (with rule count) or no export state yet. |
| Per repo | Loaded, re-ingested, or path missing. |
| Pattern count | Total extraction patterns across all template set versions. |
| Generation | Current snapshot generation number. |

One failing repo does not abort the session.

**Running counts (queryable via `flatten watch status`):**

| Counter | What |
|---|---|
| `placed` | Files successfully placed in repos. |
| `skipped` | Files skipped (not our file, already seen, re-download, binary). |
| `flagged` | Files flagged (ambiguous, new_directory, unroutable). |
| `errors` | Placement failures. |


