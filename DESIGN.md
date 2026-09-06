<!-- DESIGN.md -->
# Design

Architecture, settled decisions, and roadmap for Flatten PM. See `PROJECT_INSTRUCTIONS.md` for what the project is and how the assistant should behave.

## Target user

Developers using web-based AI project UIs who need to keep project context current with their codebase. Agent-based tools (Claude Code, Cursor, Aider, Cline, Codex, Windsurf) cover the full workflow natively; Flatten PM fills no gap for agent users. The differentiator is safe placement back (Step C) and the audit trail (Step E), which are universally unserved on chat-UI platforms.

RISK: non-agent population is shrinking. Track agent adoption. If daily agent usage crosses 30-40%, pivot toward agent-complementary features.

## Architecture

The app is a Tauri desktop application with a Rust backend and a React frontend (via Vite). The core logic lives in a standalone library crate (`flatten-core`) with no Tauri or GUI dependencies, consumable by the Tauri UI, the in-process MCP server, CLI entrypoints, and tests independently.

```
flatten-pm/
  src/                    # React frontend (TypeScript, Vite)
  src-tauri/              # Tauri app crate (depends on flatten-core)
  crates/
    flatten-core/         # Library crate (no Tauri deps)
    flatten-cli/          # CLI binary (depends on flatten-core)
```

## Export pipeline

Walks a monorepo, applies include/exclude modifier cascades, flattens directory paths into filenames using `--` as separator.

Injects directory comments as the first line of each file (correct syntax per file type: `#` for Python/YAML/shell, `//` for TypeScript/JavaScript, `<!-- -->` for Markdown/HTML, `--` for SQL, `/* */` for CSS). Ships with sensible defaults but the comment syntax mapping is configurable in the GUI: users can add, override, or remove extension-to-syntax mappings for file types the defaults don't cover. Handles shebang and directive lines by placing the comment on line 2 when line 1 is reserved.

Runs a content safety scan on each file before writing to output: secret detection using gitleaks TOML rules with keyword pre-filter and Shannon entropy fallback, plus regex PII detection (email, credit card, SSN, phone, IPv4). Two tiers: HIGH (named secrets, block pending review) and LOW (entropy, PII, flag only). Configurable per profile: enable/disable, tier thresholds, allowlist/baseline paths. Baseline/allowlist for known-intentional findings.

Generates a `_CONTEXT.yaml` manifest explaining the export format to AI assistants, including the encoding convention, modifier decisions, and the directory comment convention with an explicit instruction to preserve it.

Generates a content-hash SHA256 manifest (line ending normalization configurable per profile, default LF) for incremental update detection and watcher reconciliation.

Supports two output modes: individual flat files (for platforms that accept individual file uploads) and consolidated single-file output (packed format, for platforms that consolidate uploads or for pasting into chat). Profile controls which mode.

Export-side watch mode re-runs the export on file change (debounced), keeping the output directory current without manual re-export.

Profiles are stored in a central app data directory rather than embedded per-repo.

## Watch pipeline

Monitors a source directory for new files using native filesystem events (`notify` 8.x crate with `notify-debouncer-full`) plus application-level size/mtime settle checks for write completion.

Events are treated as hints, not truth. A periodic reconciliation rescan (timer-driven directory walk, diffed against the manifest, newest files processed first) catches files the event system missed due to OS-level buffer overflow, burst event loss, or network filesystem limitations.

On the notify "rescan required" signal (queue overflow), an immediate full directory scan triggers instead of waiting for the timer.

### Placement rule

The watcher only auto-places a file if it has a valid directory comment (first match in the detection cascade). If no directory comment is found, the file is quarantined and flagged with diagnostic context: content-hash comparison against the manifest to identify likely matches, and a summary of what changed. The user reviews quarantined files and decides placement. The watcher never guesses.

This keeps placement deterministic and auditable. Comment stripping by AI platforms is surfaced as a visible problem, not silently absorbed.

Detection cascade (comment-gated, first match wins):

1. Directory comment in the first 5 lines (regex patterns for all supported comment syntaxes, with invisible Unicode char stripping: NBSP, ZWSP, soft hyphen, BOM).
2. JSON "path" field (top-level key in `.json` files).
3. Embedded path substring (known repo paths found as substrings in the first 5 lines, longest match preferred).

If none match: quarantine with diagnostics.

## Manifest dual purpose

The SHA256 manifest serves both export (incremental update detection: hash only changed files on re-export) and watch (reconciliation baseline: what files were in the last export, what should be in the watched directory). This single artifact makes both the watcher's event unreliability and the AI's comment stripping tolerable.

## Watch history

Every watch placement and quarantine event is recorded as a timestamped entry in a local store (SQLite or JSON log per repo). Each entry captures: the incoming file, the target repo path, the diff against the previous version (if the file existed), the detection method used, and the placement outcome (placed, quarantined, user-resolved).

When the same file arrives at different times with different changes, the newest version is the placed version. Older versions are logged for tracing but the repo always reflects the most recent.

The UI exposes this as a browsable diff thread per file: the user can click through and see how a file evolved across multiple AI-generated iterations.

History entries are append-only. The watcher never overwrites history.

This serves two purposes: audit trail (what changed and when) and development feedback (review the AI's changes before committing).

## MCP backend (v2)

Flatten PM's MCP tools run inside the Tauri app as a Streamable HTTP endpoint on localhost, using the `rmcp` crate with `axum`.

Targets the 2026-07-28 stateless MCP spec (rmcp 3.x): every request is self-contained, reconnection is just retry.

GET `/health` endpoint (unauthenticated) for probes and GUI liveness checks. Auth via static bearer token with Origin/Host validation. Bound to 127.0.0.1 only.

Tools: `read_file`, `list_files`, `search_across_projects`. Design borrowed from rust-mcp-filesystem (read-only, scoped to registered repos). Intentionally small surface (3-5 tools).

Graceful shutdown wired to Tauri's exit handler.

ARCHITECTURAL RULE: MCP tool logic lives behind the HTTP endpoint, never in Tauri-internal command state. This ensures the forge daemon can proxy to it without refactoring.

## Forge daemon (v2)

A persistent FastMCP (Python) daemon that starts at user login and aggregates MCP tools from all forge ecosystem projects behind a single Streamable HTTP endpoint.

Each project mounts its tools under a namespace prefix (`flatten.read_file`, `convention.query`, `devforge.traverse`), eliminating namespace collisions and staying under client tool caps (Cursor hard 40, Windsurf 100).

When a project backend is not running, the daemon returns structured errors instead of connection refused. When a backend comes online, the daemon emits `notifications/tools/list_changed`.

Registers as a per-user login agent (launchd LaunchAgent, systemd user service, Windows startup). Auto-start is an explicit user-visible toggle.

A `.mcpb` bundle packages a stdio shim for Claude Desktop that proxies to the daemon's localhost endpoint.

## Tech stack

**Languages:** Rust, TypeScript, Python (flatten-sync scripts, forge daemon v2)
**Backend:** Rust (flatten-core library crate, standalone from Tauri)
**Frontend:** React (via Vite)
**Desktop:** Tauri 2.x (desktop shell, renders React UI, calls Rust backend)
**Framework:** FastMCP (MCP server, v2)
**Protocols:** MCP
**Libraries:** rmcp (Rust MCP SDK, v2), axum (HTTP server for MCP endpoint, v2), notify 8.x + notify-debouncer-full (file watching), gitleaks TOML rules (content safety scanning via regex crate)
**Build:** Cargo workspace, Vite, npm

## Architecture decision records

### Directory comment is the sole placement gate

**Context:** AI platforms (Claude, ChatGPT) strip, rewrite, and truncate comments during file regeneration. No platform guarantees comment preservation. The watcher's primary detection method depends on directory comments. When the AI strips them, the question is whether to fall back to fuzzy matching or refuse to place.

**Choice:** Quarantine and flag with hash/diff diagnostics when comment is missing.

**Rationale:** Auto-placement via fuzzy matching risks silent misplacement when files are similar or when the AI made large changes. The directory comment is the contract between the export and watch pipelines. If the AI breaks the contract, that should be visible to the user, not silently absorbed. Hash/diff diagnostics give the user enough context to fix the problem quickly.

### Watcher reconciliation rescan prioritized by recency

**Context:** The notify crate has confirmed, unfixed event loss under burst load (issue #412: 246 of 1500 files lost, wontfix/upstream). OS-level buffer overflow on Windows and Linux silently discards filesystem events.

**Choice:** Events as hints + periodic reconciliation rescan against the manifest, newest files first.

**Rationale:** Events provide fast response (sub-second detection). The reconciliation rescan provides correctness (nothing gets lost). Processing newest files first ensures the most recent download gets attention before older ones. The SHA256 manifest is the reconciliation baseline. On the notify "rescan required" signal (queue overflow), an immediate full scan triggers.

### MCP backend hosted inside Tauri via Streamable HTTP

**Context:** Streamable HTTP on localhost works natively for 8 of 9 major MCP clients. Only Claude Desktop rejects localhost HTTP. The rmcp crate targets the 2026-07-28 stateless spec.

**Choice:** MCP inside Tauri via Streamable HTTP on localhost.

**Rationale:** One process owns all state. MCP tools are function calls into the same code the GUI uses. Stateless spec means reconnection is free. The forge daemon (FastMCP) proxies to this endpoint, so Claude Desktop's stdio requirement is handled by the daemon's .mcpb shim.

### Forge daemon as FastMCP aggregator

**Context:** MCP clients don't reliably reconnect when servers restart. Claude Code permanently drops tools unavailable at startup. Cursor enforces a hard cap of 40 tools. The MCP spec has a flat tool namespace with 775 documented collisions across 1,470 servers.

**Choice:** Shared daemon in FastMCP/Python.

**Rationale:** The daemon is a thin proxy, not performance-critical. FastMCP's mount system provides prefixing, routing, and composition out of the box. One daemon endpoint means one config entry for the user. Prefixed tool names eliminate namespace collisions by construction. Python distribution via uv is straightforward.

### MCP auth via static bearer token

**Context:** The MCP spec mandates Origin-header validation (CVE-2025-9611, CVSS 8.8). CLI clients don't send an Origin header. All target clients support Authorization: Bearer.

**Choice:** Static bearer token + Origin/Host validation.

**Rationale:** Token generated on first launch, written to port/lock file. Origin validated when present, Host validated when absent. Bound to 127.0.0.1. OAuth 2.1 reserved for beyond-localhost.

### Core logic in a standalone library crate

**Context:** The Tauri app, CLI entrypoint, MCP server, and tests all need the same core logic.

**Choice:** Standalone library crate from day one.

**Rationale:** The cost of a library boundary is one extra crate. The cost of NOT having it shows up in three places: CLI duplicates logic, MCP server needs Tauri wrappers, tests need a Tauri window.

### Line ending normalization configurable per profile

**Context:** Same file with LF vs CRLF produces different SHA256 hashes, causing phantom diffs.

**Choice:** Configurable per profile, default LF normalization for text files.

**Rationale:** Default LF prevents phantom diffs for the common case. Binary detection via null byte check.

### Watch history as a browsable diff thread

**Context:** AI-generated file iterations produce changes that disappear after placement.

**Choice:** Append-only log with diffs, browsable in the UI.

**Rationale:** The user needs to see what the AI changed before committing. Rollback is not needed because git provides that. The history is for review, not version control.

### Content safety scanning before export

**Context:** No mature Rust secret/PII scanner exists. The gitleaks rule set (TOML-defined RE2 regex) is portable to Rust.

**Choice:** Build a safety-scan crate using gitleaks TOML rules + regex PII, skip NER for v1.

**Rationale:** Parse pinned gitleaks.toml, compile into RegexSet with aho-corasick keyword pre-filter. Two tiers: HIGH blocks, LOW flags. Baseline/allowlist for known-intentional findings. Skip NER for v1. Performance is a non-issue: pure-Rust regex scanning of 500 files is sub-second.

### No code compression, comment removal, or file processors

**Choice:** Full-content export, filtered at the profile level.

**Rationale:** Comments and implementation bodies carry intent, context, and design rationale the AI uses. Over-broad exports are a profile problem, not a compression problem.

### No token counting in v1

**Choice:** Defer to future.

**Rationale:** Token counting requires a tokenizer library. The user can see file counts and sizes in the manifest.

### No git-aware sorting or git metadata in export

**Choice:** Keep export git-agnostic for metadata.

**Rationale:** No git commit logs, change-frequency sorting, or diffs in the export. Git is available directly. The modifier cascade can honor .gitignore patterns for file filtering, but that is about which files to include, not about building git features.

### No remote repo packing

**Choice:** Local repos only.

**Rationale:** Flatten PM manages repos the developer actively works in, not one-off remote analysis.

### Repomix as reference, not dependency

**Context:** Repomix is TypeScript/Node.js. Flatten PM is Rust/Tauri.

**Choice:** Use as constant reference, build natively in Rust.

**Rationale:** Language, format, and philosophy mismatch. Borrow ideas (secret scanning patterns, watch debounce, consolidated output), none require Repomix as a dependency. The export logic is not the hard part. Placement, reconciliation, history, and MCP are.

### Desktop GUI over CLI-only

**Choice:** Library crate + CLI + GUI. The GUI is what Flatten PM adds over the existing CLI.

**Rationale:** The flatten-core library enables both consumers. The GUI solves multi-project profile management friction the CLI handles poorly. Validated by competitive research: no competitor has unified multi-repo visual profile management.

### Competitive positioning validated as chat-UI bridge

**Choice:** Position as a chat-UI bridge for developers using web-based AI project UIs.

**Rationale:** Agent users don't need Flatten PM. Export (Steps A/B) is commodity. The differentiator is safe placement back and the audit trail. OpenAI deprecated their GitHub synced connector (Sep 2026), reversing the strongest erosion signal.

## Roadmap

### v1

Bidirectional sync: flatten export with modifier cascades and comment-gated watch-and-place. Directory comment injection on export. Content safety scanning. Comment-gated placement with quarantine diagnostics. Reconciliation rescan. _CONTEXT.yaml manifest. SHA256 content-hash manifest. Consolidated output format. Export-side watch mode. Watch history. Visual profile manager. System tray mode. CLI entrypoint. Core logic in flatten-core. Daemon-ready architecture.

### v2

MCP backend (rmcp, axum, Streamable HTTP, static bearer auth). MCP tools: flatten.read_file, flatten.list_files, flatten.search_across_projects. Forge daemon (FastMCP, prefixed tool mounts, .mcpb bundle). Platform compatibility testing. Lightweight cross-project import index.

### Future

Export diffing (compare current export to previous manifest). Integration with Convention Graph Maker.
