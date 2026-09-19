<!-- fixtures/README.md -->
# Fixtures

Test data for `flatten-core` unit tests and `flatten` CLI verification commands.

## Convention

From CLI-001 and `docs/design/1_INFRASTRUCTURE.md`:

| Path | What |
|---|---|
| `fixtures/<name>/` | Source repos for ingest and export tests |
| `fixtures/recipes/*.recipe` | Recipe files for parser and export tests |
| `fixtures/transforms/*.js` | Transform source for runtime tests |
| `fixtures/downloads/` | Incoming files for watch detection tests |

## Current fixtures

### `repo-a/`

Minimal source repo for IN-001. Contains:
- `.gitignore` with `node_modules/` and `*.log`
- `src/main.rs`, `src/lib.rs`, `README.md` (included files)
- `debug.log` (excluded by `*.log`; committed with `git add -f`)

**Generated artifacts (not committed):**
- `node_modules/` with 1000 dummy files. Run `make fixtures` to generate.
  Used by CLI verification 3 to prove directory-cut performance.
- Symlinks created at test time by `#[cfg(unix)]` tests.
