<!-- scripts/flatten-sync/README.md -->
# Flatten-Sync

Bidirectional sync between a monorepo and AI project UIs:

- **Flatten** exports repo files into a flat directory for upload to
  Claude Projects, Gemini, or ChatGPT.
- **Watch** monitors a source directory (e.g. Downloads) for project
  files and auto-places them back into the repo.

## Quick Start

From repo root:

```bash
# Flatten (export)
make project-export                      # export with default profile
make project-export PROFILE=bench-code   # export with named profile
make project-export-dry-run              # preview without copying
make project-test                        # run unit tests (pytest)

# Watch (import)
make project-watch                       # watch Downloads, auto-place files
make project-bash-test                   # run watcher integration test (bash)
```

## Flatten

Encodes directory paths into filenames using `--` as the separator:

```
extension/components/App.tsx  ->  extension--components--App.tsx
```

Output goes to `project-export-files/<profile>/` at the repo root.

### Configuration

Two config files, fully decoupled:

**modifiers.yaml**: named patterns that define WHAT to match.
Profiles decide which modifiers to activate. `silent: true` suppresses
excluded entries from the _CONTEXT.yaml manifest. Committed and shared.

**profiles/\<name\>.flatten.config.yaml**: nested tree that decides
WHERE and WHICH DIRECTION to apply modifiers. Gitignored. Example
files (`.example.yaml`) are committed as references.

### Profile Config

Profiles are nested trees using `__` dunder prefix for metadata:

```yaml
root:
  __path: .
  __modifier:
    exclude: [tests]
  __note: "Full repo, no tests"
  extension:
    __path: extension
    __modifier:
      exclude: [assets]
```

Resolution at each node: inherit parent -> apply include -> apply exclude.

### CLI Reference

```bash
flatten_for_project.py run [--profile NAME] [--root PATH] [--dry-run] [--clean] [--preserve PATTERN...]
flatten_for_project.py discover [--profile NAME]   # TODO: Phase 3
```

| Flag | Default | Purpose |
|------|---------|---------|
| `--profile` | `default` | Profile name |
| `--root` | `.` | Repo root directory |
| `--dry-run` | off | Preview without copying |
| `--clean` | off | Force re-copy (ignore mtime cache) |
| `--preserve` | none | Glob patterns for manual files to keep |

## Watch

Monitors a source directory for new files and auto-places them into
the repo using a three-stage detection cascade (first match wins):

1. **Directory comment** in the first 5 lines (`# path`, `// path`,
   `<!-- path -->`, `-- path`, `/* path */`): handles shebangs on L1.
2. **JSON `"path"` field**: top-level `"path"` key in `.json` files.
3. **Embedded path substring**: known repo paths found as substrings in
   the first 5 lines (catches decorated comments like Makefile banners).

All detection tiers validate paths through `_valid_target` which
rejects absolute paths, traversal, backslash separators, and paths
whose parent directory does not exist.

### Input hardening

`_read_head` handles several edge cases from browser downloads and
AI chat output:

- **UTF-8 BOM**: stripped transparently via `utf-8-sig` codec
- **Truncated multi-byte chars**: `HEAD_BYTES` truncation at 2048
  bytes can split an em dash or similar; fallback `errors='ignore'`
  handles this since only the first 5 lines matter
- **Binary files**: rejected early via null-byte check
- **Invisible Unicode**: NBSP mapped to regular space, ZWSP and soft
  hyphen stripped before pattern matching

### Configuration

**profiles/\<name\>.watch.config.yaml**: source directory, target repo,
cleanup mode, polling intervals. Gitignored. Example file committed.

```yaml
source:
  windows_user: auto            # "auto" or explicit username
  # directory: /custom/path     # override for non-Downloads source

target:
  repo: ~/DeeVec

cleanup:
  mode: move                    # "delete" or "move"
  move_to: processed            # relative to script dir (gitignored)

polling:
  interval: 1.5
  stable_wait: 0.5
  stable_retries: 6
  repo_map_refresh: 30          # full re-index every 30s for external changes
```

The repo file map is rebuilt every `repo_map_refresh` seconds to pick up
external changes (mkdir, git checkout, etc.). Newly placed files are also
added to the map immediately after each successful placement.

### Cleanup

When `mode: move`, processed files land in day subfolders with
T-prefixed HHMMSS timestamps:

```
processed/
  2026-06-06/
    T134201_chunker.py
    T134522_chunker.py
    T140105_App.tsx
```

Same file downloaded multiple times -> distinct entries preserved.
Clean up a day with `rm -rf processed/2026-06-06/`.

### CLI Reference

```bash
watch.py --repo ~/DeeVec                     # no profile, defaults
watch.py --profile default                  # repo from profile
watch.py --profile default --repo ~/other   # CLI overrides profile
watch.py --profile default -v               # show skipped files
```

| Flag | Default | Purpose |
|------|---------|---------|
| `--repo` | from profile | Repo root path |
| `--profile` | tries `default` | Watch profile name |
| `--user` | auto-detect | Windows username (WSL) |
| `-v` | off | Show skipped non-project files |

## Standalone Usage

This directory is portable. To use in another repo:

1. Copy the `flatten-sync/` directory anywhere in your repo
2. Edit `modifiers.yaml` for your project's patterns
3. Create profiles in `profiles/`
4. Run directly:

```bash
cd path/to/flatten-sync
make                          # uses git rev-parse to find repo root
make PROFILE=my-profile
make watch
make test
```

Or delegate from your root Makefile:

```makefile
project-export:
	$(MAKE) -C path/to/flatten-sync run PROFILE=$(PROFILE) PROJECT_ROOT=$(CURDIR)

project-watch:
	$(MAKE) -C path/to/flatten-sync watch PROFILE=$(PROFILE)
```

## File Layout

```
flatten-sync/
  lib/                          # shared modules (profiles, cascade, walk, etc.)
  profiles/                     # *.flatten.config.* and *.watch.config.* files
  tests/                        # pytest unit tests + bash integration test
  processed/                    # moved files from watcher (gitignored)

  flatten_for_project.py        # flatten entry point
  watch.py                      # watcher entry point
  modifiers.yaml                # modifier patterns (committed, shared)
  Makefile                      # portable make targets
```
