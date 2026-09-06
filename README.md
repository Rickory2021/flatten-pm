<!-- README.md -->
# Flatten PM

Desktop app that syncs your codebase with AI project UIs (Claude Projects, ChatGPT, Gemini). Replaces the copy-paste-and-reconfigure workflow with a configurable tool that handles export, watch, and cross-project discovery.

**Status:** Early development. Not yet usable.

## What it does

**Export:** Walks a monorepo, applies include/exclude modifier cascades, flattens directory paths into filenames (`src/components/App.tsx` becomes `src--components--App.tsx`), injects directory comments, runs a content safety scan for secrets and PII, and outputs flat files for upload to AI project knowledge bases.

**Watch:** Monitors a directory for AI-generated files, detects their origin repo path via directory comments, and auto-places them back. Files with missing or mangled comments are quarantined with diagnostic context instead of silently skipped.

**Profiles:** One installed app manages per-repo export and watch settings through a visual profile editor. No more copying scripts into each repo and editing YAML by hand.

**History:** Every watch placement is recorded as a timestamped entry with diffs, browsable in the UI. Review what the AI changed before committing.

## Architecture

Tauri 2.x desktop app with a Rust backend and React frontend (Vite).

Core logic lives in `flatten-core`, a standalone library crate with no Tauri or GUI dependencies. The Tauri app, CLI, and tests all consume it independently.

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
