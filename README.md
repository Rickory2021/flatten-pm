<!-- README.md -->
# Flatten PM

Desktop app that syncs your codebase with AI project UIs (Claude Projects, ChatGPT, Gemini). Replaces the copy-paste-and-reconfigure workflow with one app that exports your repos in the shape each platform wants and places the AI's files back where they came from.

**Status:** Early development. Not yet usable.

## What it does

**Export:** A build recipe (Dockerfile-like text: SOURCE, FROM, COPY, EXCLUDE, RUN, ARG) selects files from your registered repos, applies transforms (flatten paths into filenames such as `src--components--App.tsx`, inject a directory comment on line 1, pack into one file), and writes the result to an app-managed directory for upload. Every export records the rules and versions that produced it, so files can find their way back.

**Watch:** Monitors your downloads directory, reads each file's directory comment, resolves it through the rules your export recorded, undoes the export's content transforms, and places the file back in the repo. Ambiguous files are flagged with every candidate target; nothing is guessed. Files with stripped comments are skipped in v1 s1 and matched by content hash in s2.

**Recipes:** One installed app holds your recipes, versioned with rollback. Activate a recipe with a binding, export on demand, and the watch serves every active binding. Transforms are JavaScript you can read, edit, and extend; the builtins ship as editable examples. No more copying scripts into each repo and editing YAML by hand.

**History (s2):** Every placement and export is recorded with diffs, browsable per file. Review what the AI changed before committing.

## Architecture

Tauri 2.x desktop app with a Rust backend and React frontend (Vite). Design, decisions, and roadmap in `DESIGN.md`; vocabulary in `docs/VOCABULARY.md`; backlog in `docs/BACKLOG.yaml`.

Three stages: ingest (walk a repo into a hashed trie), export (recipe to output tree to files), watch (files back to repos through the export's recorded rules).

Core logic lives in `flatten-core`, a standalone library crate with no Tauri or GUI dependencies. The Tauri app, the `flatten` CLI, and tests all consume it independently; the CLI is also the test harness for every pipeline.

```
flatten-pm/
  src/                    # React frontend (Vite)
  src-tauri/              # Tauri app crate
  crates/
    flatten-core/         # Library crate (no Tauri deps)
    flatten-cli/          # CLI binary
```

## Building

Prerequisites: [Rust](https://rustup.rs/), [Node.js](https://nodejs.org/), and the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your platform.

```bash
npm install
cargo tauri dev
```

## License

MIT
