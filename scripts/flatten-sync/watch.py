# scripts/flatten-sync/watch.py
# /// script
# requires-python = ">=3.12"
# dependencies = ["pyyaml"]
# ///
"""
Download watcher for Claude Projects -> repo placement.

Monitors a source directory (default: Windows Downloads via WSL) for new
files, detects project files via cascading strategies, and places them at
the correct repo path.

Detection cascade (first match wins):
  1. Directory comment in first 5 lines  (# path, // path, <!-- path -->, etc.)
  2. JSON "path" field                   (snapshot metadata, etc.)
  3. Embedded path substring             (known repo path found in first 5 lines)

Usage:
    uv run watch.py --repo ~/DeeVec
    uv run watch.py --profile default
    uv run watch.py --profile default --repo ~/other/repo
    Ctrl+C to stop.
"""

from __future__ import annotations

import argparse
import json
import logging
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

# Bootstrap: add script directory to sys.path for lib/ imports.
sys.path.insert(0, str(Path(__file__).resolve().parent))

from lib.watch_profile import load_watch_profile, resolve_watch_profile_path

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

# Directory comment patterns (order per handoff spec 5.2).
COMMENT_PATTERNS: list[re.Pattern[str]] = [
    re.compile(r"^# (\S+)$"),  # Python, YAML, TOML, shell, Makefile
    re.compile(r"^// (\S+)$"),  # TypeScript, JavaScript
    re.compile(r"^<!-- (\S+) -->$"),  # Markdown, HTML
    re.compile(r"^-- (\S+)$"),  # SQL
    re.compile(r"^/\* (\S+) \*/$"),  # CSS
]

HEAD_BYTES = 2048  # enough for 5 lines of any reasonable file
HEAD_LINES = 5  # lines to scan for directory comments

# Invisible Unicode chars that editors/chat UIs sometimes inject.
# Applied to each line before pattern matching. NBSP becomes a regular
# space (it separates tokens); zero-width chars are deleted.
_INVISIBLE = str.maketrans(
    {
        0x00A0: " ",  # NBSP -> regular space
        0x200B: None,  # zero-width space -> delete
        0x00AD: None,  # soft hyphen -> delete
        0xFEFF: None,  # BOM -> delete (backup for mid-line)
    }
)

POLL_DEFAULTS = {
    "interval": 1.5,
    "stable_wait": 0.5,
    "stable_retries": 6,
    "repo_map_refresh": 30,
}

# Directories to skip when building the repo map.
SKIP_DIRS = {
    ".git",
    "node_modules",
    "__pycache__",
    ".output",
    ".wxt",
    "project-export-files",
    "dist",
    ".venv",
    "venv",
}

log = logging.getLogger("watch")

# ---------------------------------------------------------------------------
# ANSI helpers
# ---------------------------------------------------------------------------

_TTY = hasattr(sys.stdout, "isatty") and sys.stdout.isatty()
G, Y, R, D, B, X = (
    ("\033[32m", "\033[33m", "\033[31m", "\033[2m", "\033[1m", "\033[0m")
    if _TTY
    else ("", "", "", "", "", "")
)

# ---------------------------------------------------------------------------
# Repo map
# ---------------------------------------------------------------------------


def build_repo_map(repo_root: Path) -> set[str]:
    """Walk the repo tree and collect every relative file path."""
    paths: set[str] = set()
    for dirpath, dirs, files in os.walk(repo_root):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
        for f in files:
            rel = str((Path(dirpath) / f).relative_to(repo_root))
            paths.add(rel)
    return paths


# ---------------------------------------------------------------------------
# Detection strategies
# ---------------------------------------------------------------------------


def _read_head(filepath: Path) -> list[str] | None:
    """Read first HEAD_BYTES, decode UTF-8, return first HEAD_LINES stripped.
    Uses utf-8-sig to strip BOM from browser downloads. Null-byte check
    rejects binary files. On truncated multi-byte chars at the HEAD_BYTES
    boundary, falls back to errors='ignore' since only the first 5 lines
    matter and the damage is always at the tail of the 2048-byte window.
    Invisible Unicode chars (NBSP, ZWSP, soft hyphen) are stripped so they
    cannot defeat comment pattern anchors."""
    try:
        raw = filepath.read_bytes()[:HEAD_BYTES]
    except OSError:
        return None
    if b"\x00" in raw:
        return None
    try:
        text = raw.decode("utf-8-sig")
    except UnicodeDecodeError:
        text = raw.decode("utf-8-sig", errors="ignore")
    return [
        line.strip().translate(_INVISIBLE) for line in text.split("\n")[:HEAD_LINES]
    ]


def _detect_comment(lines: list[str]) -> tuple[str, int] | None:
    """Match directory comment patterns across first N lines.
    Returns (path, 1-indexed line number) or None."""
    for i, line in enumerate(lines):
        for pattern in COMMENT_PATTERNS:
            m = pattern.match(line)
            if m:
                return m.group(1), i + 1
    return None


def _detect_json_path(filepath: Path) -> str | None:
    """For JSON files, check if a top-level "path" key holds a repo path."""
    if filepath.suffix != ".json":
        return None
    try:
        data = json.loads(filepath.read_text("utf-8"))
        if isinstance(data, dict) and isinstance(data.get("path"), str):
            return data["path"]
    except (OSError, json.JSONDecodeError, UnicodeDecodeError):
        pass
    return None


def _detect_embedded(lines: list[str], repo_map: set[str]) -> str | None:
    """Last resort: find any known repo path as a substring in the first lines.
    Only considers paths with at least one / to avoid bare-filename false positives.
    Prefers the longest match (most specific path)."""
    candidates = sorted(
        (p for p in repo_map if "/" in p),
        key=len,
        reverse=True,
    )
    for line in lines:
        for repo_path in candidates:
            if repo_path in line:
                return repo_path
    return None


def _valid_target(repo_path: str, repo_root: Path) -> bool:
    """Validate: no absolute path, no traversal, no backslashes, parent dir exists.
    Existing repo files (even extensionless like Makefile) are always valid.
    New files must have an extension to avoid matching directory names."""
    if repo_path.startswith("/") or ".." in repo_path.split("/"):
        return False
    if "\\" in repo_path:
        return False
    dest = repo_root / repo_path
    if dest.is_file():
        return True
    if "." not in dest.name:
        return False
    return dest.parent.is_dir()


def detect(
    filepath: Path, repo_map: set[str], repo_root: Path
) -> tuple[str, str] | tuple[None, None]:
    """Run the detection cascade. Returns (repo_path, method_label) or (None, None)."""
    lines = _read_head(filepath)
    if lines is None:
        return None, None

    # 1. Directory comment (most reliable)
    result = _detect_comment(lines)
    if result:
        path, line_num = result
        if _valid_target(path, repo_root):
            return path, f"comment L{line_num}"

    # 2. JSON "path" field
    json_path = _detect_json_path(filepath)
    if json_path and _valid_target(json_path, repo_root):
        return json_path, "json"

    # 3. Embedded path substring
    emb_path = _detect_embedded(lines, repo_map)
    if emb_path and _valid_target(emb_path, repo_root):
        return emb_path, "embedded"

    return None, None


# ---------------------------------------------------------------------------
# Source directory
# ---------------------------------------------------------------------------


def _detect_windows_user() -> str:
    """Detect the current Windows username from WSL via cmd.exe."""
    try:
        result = subprocess.run(
            ["cmd.exe", "/c", "echo", "%USERNAME%"],
            capture_output=True,
            text=True,
            timeout=5,
        )
        username = result.stdout.strip()
        if username and username != "%USERNAME%":
            return username
    except (subprocess.TimeoutExpired, FileNotFoundError, OSError):
        pass
    raise RuntimeError(
        "Could not detect Windows username. "
        "Set source.windows_user in your watch profile or pass --user."
    )


def _resolve_source(config: dict) -> Path:
    """Resolve the source directory from config."""
    source = config.get("source", {})
    if "directory" in source:
        path = Path(source["directory"]).expanduser()
    else:
        username = source.get("windows_user", "auto")
        if username == "auto":
            username = _detect_windows_user()
        path = Path(f"/mnt/c/Users/{username}/Downloads")
    if not path.is_dir():
        raise RuntimeError(f"Source directory not found: {path}")
    return path


# ---------------------------------------------------------------------------
# File processing
# ---------------------------------------------------------------------------


def _wait_for_stable(filepath: Path, config: dict) -> bool:
    """Poll until the file's size stops changing."""
    polling = config.get("polling", {})
    wait = polling.get("stable_wait", POLL_DEFAULTS["stable_wait"])
    retries = polling.get("stable_retries", POLL_DEFAULTS["stable_retries"])
    for _ in range(retries):
        try:
            s1 = filepath.stat().st_size
        except OSError:
            return False
        time.sleep(wait)
        try:
            s2 = filepath.stat().st_size
        except OSError:
            return False
        if s1 == s2 and s1 > 0:
            return True
    return False


def _cleanup_source(filepath: Path, config: dict) -> None:
    """Delete or move the source file after successful placement."""
    cleanup = config.get("cleanup", {})
    mode = cleanup.get("mode", "delete")
    if mode == "move":
        move_to = Path(cleanup["move_to"])
        if not move_to.is_absolute() and not str(move_to).startswith("~"):
            # Relative paths resolve against the script directory
            move_to = Path(__file__).resolve().parent / move_to
        # Day subfolder + timestamp prefix for audit trail
        now = time.strftime("%Y-%m-%d", time.localtime())
        stamp = time.strftime("T%H%M%S", time.localtime())
        dest_dir = move_to.expanduser() / now
        dest_dir.mkdir(parents=True, exist_ok=True)
        dest_name = f"{stamp}_{filepath.name}"
        shutil.move(str(filepath), str(dest_dir / dest_name))
    else:
        filepath.unlink()


def _process_file(
    filepath: Path,
    repo_root: Path,
    repo_map: set[str],
    config: dict,
    stats: dict,
) -> None:
    """Inspect one new file: detect, validate, place, clean up."""
    name = filepath.name

    if name.endswith(".crdownload") or not filepath.exists():
        return

    if not _wait_for_stable(filepath, config):
        log.warning(f"  {Y}·{X}  {name}  {D}(size did not stabilize){X}")
        return

    repo_path, method = detect(filepath, repo_map, repo_root)

    if repo_path is None:
        stats["skipped"] += 1
        log.debug(f"  {D}·  {name}  (no match){X}")
        return

    dest = repo_root / repo_path

    try:
        shutil.copy2(filepath, dest)
    except OSError as exc:
        stats["errors"] += 1
        log.error(f"  {R}✗{X}  {name} → {repo_path}  {R}({exc}){X}")
        return

    stats["placed"] += 1
    repo_map.add(repo_path)
    log.info(f"  {G}✓{X}  {B}{name}{X} → {repo_path}  {D}[{method}]{X}")

    try:
        _cleanup_source(filepath, config)
    except OSError as exc:
        log.warning(f"     {Y}cleanup failed: {exc}{X}")


# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------


def watch(config: dict) -> None:
    """Poll the source directory. Only processes files arriving after startup."""
    repo_root = Path(config["target"]["repo"]).expanduser().resolve()
    if not repo_root.is_dir():
        log.error(f"Repo not found: {repo_root}")
        sys.exit(1)

    source = _resolve_source(config)
    polling = config.get("polling", {})
    interval = polling.get("interval", POLL_DEFAULTS["interval"])
    refresh_interval = polling.get(
        "repo_map_refresh", POLL_DEFAULTS["repo_map_refresh"]
    )

    repo_map = build_repo_map(repo_root)

    cleanup = config.get("cleanup", {})
    mode = cleanup.get("mode", "delete")
    mode_desc = f"move → {cleanup.get('move_to', '?')}" if mode == "move" else "delete"

    log.info(f"  {B}Watching{X}  {source}")
    log.info(f"  {B}Target{X}   {repo_root}")
    log.info(f"  {B}Indexed{X}  {len(repo_map)} files")
    log.info(f"  {B}Cleanup{X}  {mode_desc}")
    log.info(f"  {'─' * 52}")

    known: set[str] = {f.name for f in source.iterdir() if f.is_file()}
    stats = {"placed": 0, "skipped": 0, "errors": 0}
    last_refresh = time.monotonic()

    try:
        while True:
            time.sleep(interval)

            # Periodic full repo map rebuild to pick up external changes
            # (mkdir, git checkout, etc.). Per-placement .add() covers the
            # immediate case; this sweep catches everything else.
            now = time.monotonic()
            if now - last_refresh >= refresh_interval:
                repo_map = build_repo_map(repo_root)
                last_refresh = now

            current = {f.name for f in source.iterdir() if f.is_file()}
            new_names = current - known
            known = current
            for name in sorted(new_names):
                _process_file(source / name, repo_root, repo_map, config, stats)
    except KeyboardInterrupt:
        p, s, e = stats["placed"], stats["skipped"], stats["errors"]
        err_color = R if e else ""
        log.info(
            f"\n  {B}Done{X}  {G}{p} placed{X}, {s} skipped, {err_color}{e} errors{X}"
        )


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def main() -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Watch for downloaded project files and auto-place them in the repo."
        ),
    )
    parser.add_argument(
        "--repo",
        type=Path,
        help="Repo root path (overrides profile target.repo)",
    )
    parser.add_argument(
        "--profile",
        help="Watch profile name from profiles/ dir (default: tries 'default')",
    )
    parser.add_argument(
        "--user",
        help="Windows username (overrides profile source.windows_user)",
    )
    parser.add_argument(
        "-v",
        "--verbose",
        action="store_true",
        help="Show skipped non-project files",
    )
    args = parser.parse_args()

    logging.basicConfig(
        stream=sys.stdout,
        format="%(asctime)s %(message)s",
        datefmt="%H:%M:%S",
        level=logging.DEBUG if args.verbose else logging.INFO,
    )

    # --- Load profile ---
    script_dir = Path(__file__).resolve().parent
    profiles_dir = script_dir / "profiles"

    config: dict = {}
    if args.profile:
        path = resolve_watch_profile_path(args.profile, profiles_dir)
        config = load_watch_profile(path)
        log.info(f"  {D}Profile: {path.name}{X}")
    elif profiles_dir.is_dir():
        try:
            path = resolve_watch_profile_path("default", profiles_dir)
            config = load_watch_profile(path)
            log.info(f"  {D}Profile: {path.name}{X}")
        except FileNotFoundError:
            pass

    # --- CLI overrides ---
    if args.repo:
        config.setdefault("target", {})["repo"] = str(args.repo)
    if args.user:
        config.setdefault("source", {})["windows_user"] = args.user

    # --- Validate ---
    if "target" not in config or "repo" not in config.get("target", {}):
        parser.error(
            "No repo specified. Use --repo or set target.repo in a watch profile."
        )

    watch(config)


if __name__ == "__main__":
    main()
