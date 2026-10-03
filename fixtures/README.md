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

### `recipes/`

Recipe files for EX-001's CLI verification (`docs/BACKLOG.yaml`, EX-001
`verification`). Each starts with a `# fixtures/recipes/<name>.recipe`
comment, so error positions in the `bad-*` files start at line 2. Every
fixture uses only the five seeded builtin transforms. Add `base` before
`invoke`.

| File | What it exercises |
|---|---|
| `basic.recipe` | Every instruction, comments, a continuation, a quoted arg; `show --json` and `show --raw` byte identity |
| `base.recipe` | A recipe meant to be invoked; its COPY key substitutes `${repo}` |
| `invoke.recipe` | `INVOKE base repo=repo-a`; `show --json` lists `base` in `invoked_versions` |
| `misorder.recipe` | Valid, with `OVERRIDE_WITH []`; `lint` reports L005 |
| `bad-unknown-instruction.recipe` | `COPIE`: unknown instruction at 2:1 |
| `bad-dotdot-dest.recipe` | `..` in a COPY dest: invalid path |
| `bad-copy-outside-source.recipe` | top-level COPY |
| `bad-invoke-cycle.recipe` | `INVOKE cyc` saved as `cyc`: `cyc@pending -> cyc@pending` |
| `bad-invoke-unbound.recipe` | `INVOKE shipped-default` with no `ARG repo`: names both recipes |
| `bad-duplicate-key.recipe` | two COPY blocks `AS dup`: both locations |

### `recipes/`

Recipe files for EX-001's CLI verification (`docs/BACKLOG.yaml`, EX-001
`verification`). Each starts with a `# fixtures/recipes/<name>.recipe`
comment, so error positions in the `bad-*` files start at line 2. Every
fixture uses only the five seeded builtin transforms. Add `base` before
`invoke`.

| File | What it exercises |
|---|---|
| `basic.recipe` | Every instruction, comments, a continuation, a quoted arg; `show --json` and `show --raw` byte identity |
| `base.recipe` | A recipe meant to be invoked; its COPY key substitutes `${repo}` |
| `invoke.recipe` | `INVOKE base repo=repo-a`; `show --json` lists `base` in `invoked_versions` |
| `misorder.recipe` | Valid, with `OVERRIDE_WITH []`; `lint` reports L005 |
| `bad-unknown-instruction.recipe` | `COPIE`: unknown instruction at 2:1 |
| `bad-dotdot-dest.recipe` | `..` in a COPY dest: invalid path |
| `bad-copy-outside-source.recipe` | top-level COPY |
| `bad-invoke-cycle.recipe` | `INVOKE cyc` saved as `cyc`: `cyc@pending -> cyc@pending` |
| `bad-invoke-unbound.recipe` | `INVOKE shipped-default` with no `ARG repo`: names both recipes |
| `bad-duplicate-key.recipe` | two COPY blocks `AS dup`: both locations |
