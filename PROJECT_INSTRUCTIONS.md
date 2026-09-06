<!-- PROJECT_INSTRUCTIONS.md -->
# Flatten PM

AI assistant operating guide for the Flatten PM repo. Read this file at the start of every conversation.

## What this project is

A Tauri 2.x desktop application that syncs codebases with AI project UIs (Claude Projects, ChatGPT, Gemini). Two pipelines: export (repo to flat files for upload) and watch (AI-generated files back into the repo). A visual profile manager replaces the per-repo script-and-YAML setup.

The project is public and open source (MIT). No NDA constraints.

## Repo structure

```
flatten-pm/
  README.md
  LICENSE
  PROJECT_INSTRUCTIONS.md
  Makefile                    # delegates to scripts/flatten-sync/
  package.json                # frontend deps (React, Vite, Tailwind)
  vite.config.ts
  index.html
  src/                        # React frontend (TypeScript, Vite)
  src-tauri/                  # Tauri app crate (depends on flatten-core)
    Cargo.toml
    tauri.conf.json
    src/
      lib.rs
      main.rs
  crates/
    flatten-core/             # Library crate, no Tauri deps
      Cargo.toml
      src/
        lib.rs
    flatten-cli/              # CLI binary, depends on flatten-core
      Cargo.toml
      src/
        main.rs
  scripts/
    flatten-sync/             # Project export/watch tooling (Python, portable)
```

## Architecture rules

These are settled decisions. Do not revisit without explicit instruction.

**Core logic in flatten-core.** All export, watch, scanning, and manifest logic lives in the `flatten-core` library crate. The Tauri app, CLI, MCP server, and tests all consume it. Never put core logic in `src-tauri/` directly.

**Directory comment is the sole placement gate.** The watcher only auto-places a file if it has a valid directory comment. No fuzzy matching fallback. If the AI stripped the comment, quarantine the file with hash/diff diagnostics and let the user decide. Deterministic placement over convenience.

**Content safety scanning before export.** Every file is scanned for secrets (gitleaks TOML rules, keyword pre-filter, Shannon entropy fallback) and PII (regex: email, credit card, SSN, phone, IPv4) before leaving the local machine. Two tiers: HIGH blocks pending review, LOW flags only.

**No code compression or comment removal.** Full-content export, filtered at the profile level. Comments carry intent and design rationale that the AI uses.

**Watcher reconciliation rescan.** Filesystem events are hints, not truth. A periodic timer-driven directory walk catches files the event system missed (notify crate has confirmed, unfixed event loss under burst load). Newest files processed first.

**MCP tool logic behind HTTP endpoint (v2).** When the MCP server is built, tool logic lives behind a loopback HTTP endpoint, never in Tauri-internal command state. This ensures the forge daemon can proxy to it without refactoring.

## Tech stack

**Backend:** Rust (flatten-core library, Tauri app, CLI)
**Frontend:** React, TypeScript, Vite
**Desktop:** Tauri 2.x
**Libraries:** notify 8.x + notify-debouncer-full (file watching), gitleaks TOML rules (secret scanning via regex crate)
**Build:** Cargo workspace, Vite, npm
**Future (v2):** rmcp (Rust MCP SDK), axum (HTTP server), FastMCP (Python, forge daemon)

## Working mode

HITL (Human in the Loop).

- Additive or low-risk changes (new functions, tests, comments, formatting): report with a risk tag and proceed.
- Behavioral or cascade-risk changes (API contracts, architecture, schema, multi-file structural edits): stop, present a checkpoint, wait for explicit approval.
- If unsure, treat it as behavioral and gate.
- When reviewing or diagnosing: surface the problem, note severity, let the human choose the approach. Do not auto-fix.

**File delivery:** Present changed files in full, not as diffs or snippets.

## Engineering principles

KISS, DRY, SSOT, SOLID, YAGNI, defensive programming. Flag violations rather than silently expanding scope.

## Style conventions

- Active voice. "We built" not "was built."
- No em dashes. Use commas, semicolons, or restructure.
- Concrete verbs over abstract nouns. "We deployed" not "deployment was performed."
- Plain language. "Use" not "utilize."
- Serial (Oxford) comma.

## Commit conventions

Format: `<type>(<scope>): <short summary>` (under 72 characters)

**Types:** feat, fix, refactor, docs, test, chore, style
**Scopes:** core, cli, tauri, ui, scripts, root

Stage files individually. Never `git add .` or `git add <directory>/`.

```bash
git add <file>
git commit -m "type(scope): summary"
```

Commit by logical checkpoint, not by session or file count. Each commit represents one coherent change.

## Files to read based on task

**Export pipeline work:** `crates/flatten-core/src/`, this file's architecture rules section.
**Watch pipeline work:** `crates/flatten-core/src/`, architecture rules on placement gate and reconciliation.
**Frontend work:** `src/`, `src-tauri/tauri.conf.json` for capability permissions.
**Tauri integration:** `src-tauri/src/lib.rs`, `src-tauri/Cargo.toml` for Tauri command registration.
**CLI work:** `crates/flatten-cli/src/main.rs`.
**Content safety scanning:** Architecture rules on scanning, `crates/flatten-core/` for implementation.

## Relationship to dev-forge

This project is tracked in the dev-forge career repo as `projects/progressing/bp.data.project.flatten-pm.yaml`. The full product design, VPC, ADRs, roadmap, and competitive research live there. This repo contains the implementation.

The `scripts/flatten-sync/` directory is a copy of dev-forge's flatten-sync tooling, configured for this repo's structure. It is the bootstrap mechanism: once Flatten PM itself is functional, it replaces these scripts.