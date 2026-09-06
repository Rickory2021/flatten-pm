# scripts/flatten-sync/flatten_for_project.py
# /// script
# requires-python = ">=3.12"
# dependencies = ["pyyaml"]
# ///
"""Flatten DeeVec monorepo files into a flat directory for AI project upload.

Directory paths are encoded into filenames using -- as separator:
  extension/components/App.tsx -> extension--components--App.tsx

Configuration:
  modifiers.yaml        Named patterns (committed, shared)
  profiles/<name>.yaml  Nested tree with cascading modifiers

Usage:
  flatten_for_project.py run                          # default profile
  flatten_for_project.py run --profile bench-code     # named profile
  flatten_for_project.py run --dry-run                # preview only
  flatten_for_project.py discover --profile default   # show tree (TODO)
"""

import argparse
import fnmatch
import shutil
import sys
from pathlib import Path

# Bootstrap: add this script's directory to sys.path so lib/ imports
# resolve regardless of where the flatten-sync/ directory lives.
sys.path.insert(0, str(Path(__file__).resolve().parent))

from lib.cascade import resolve_cascade
from lib.flatten import flatten_name
from lib.context import generate_manifest
from lib.modifiers import load_modifiers
from lib.flatten_profile import parse_profile, resolve_profile_path
from lib.walk import walk_node

# ---------------------------------------------------------------------------
# Config resolution
# ---------------------------------------------------------------------------

SCRIPT_DIR = Path(__file__).parent
MODIFIERS_PATH = SCRIPT_DIR / "modifiers.yaml"
PROFILES_DIR = SCRIPT_DIR / "profiles"


# ---------------------------------------------------------------------------
# Tree flattening helpers
# ---------------------------------------------------------------------------


def _build_walk_nodes(tree, resolved, root, prefix=""):
    """Convert parsed tree + resolved cascade into walk-ready node list.

    Returns list of dicts with:
        key, path (absolute), resolved_excludes, child_paths
    Ordered parent-first so children are available for parent skip lists.
    """
    nodes = []
    for key, node in tree.items():
        dotted = f"{prefix}{key}" if not prefix else f"{prefix}.{key}"
        if not prefix:
            dotted = key

        abs_path = (root / node["path"]).resolve()
        child_abs_paths = []
        child_nodes = []

        for child_key, child_node in node.get("children", {}).items():
            child_abs = (root / child_node["path"]).resolve()
            child_abs_paths.append(child_abs)
            child_nodes.extend(
                _build_walk_nodes(
                    {child_key: child_node},
                    resolved,
                    root,
                    prefix=dotted,
                )
            )

        nodes.append(
            {
                "key": dotted,
                "path": abs_path,
                "resolved_excludes": resolved.get(dotted, set()),
                "child_paths": child_abs_paths,
            }
        )
        nodes.extend(child_nodes)

    return nodes


# ---------------------------------------------------------------------------
# Commands
# ---------------------------------------------------------------------------


def cmd_run(args):
    """Run the export."""
    root = Path(args.root).resolve()

    # Load config
    modifiers = load_modifiers(MODIFIERS_PATH)
    profile_path = resolve_profile_path(args.profile, PROFILES_DIR)
    tree = parse_profile(profile_path)
    resolved = resolve_cascade(tree)

    # Build walk nodes
    nodes = _build_walk_nodes(tree, resolved, root)

    # Walk and collect files (with exclusion tracking for manifest)
    exclusion_log = {}
    all_files = []
    for node in nodes:
        files = walk_node(
            root=root,
            node_path=node["path"],
            resolved_excludes=node["resolved_excludes"],
            modifiers=modifiers,
            child_paths=node["child_paths"],
            exclusion_log=exclusion_log,
        )
        all_files.extend(files)

    if not all_files:
        print("  No files matched. Check your profile config.")
        sys.exit(0)

    # Build flat name mapping
    output = root / "project-export-files" / args.profile
    expected = {}
    for abs_path in all_files:
        rel = str(abs_path.relative_to(root))
        flat = flatten_name(rel)
        expected[flat] = (abs_path, rel)

    # Parse preserve patterns
    preserve_patterns = []
    for pat in args.preserve:
        preserve_patterns.append(pat)

    def is_preserved(filename):
        return any(fnmatch.fnmatch(filename, p) for p in preserve_patterns)

    # Remove stale files from output
    removed = 0
    preserved = 0
    if output.exists():
        for existing in sorted(output.iterdir()):
            if not existing.is_file():
                continue
            if existing.name in expected:
                continue
            if existing.name == "_CONTEXT.yaml":
                continue
            if is_preserved(existing.name):
                preserved += 1
                continue
            if args.dry_run:
                print(f"  [REMOVE] {existing.name}  (stale)")
            else:
                existing.unlink()
            removed += 1

    # Print header
    print(f"  Profile: {args.profile}")
    print(f"  Config:  {profile_path.relative_to(root)}")
    print(f"  Root:    {root}")
    print(f"  Output:  {output}")
    print(f"  Files:   {len(expected)}")
    print()

    if not args.dry_run:
        output.mkdir(parents=True, exist_ok=True)

    # Copy files
    copied = 0
    unchanged = 0

    for flat, (abs_path, rel) in sorted(expected.items()):
        dest = output / flat

        if dest.exists() and not args.clean:
            src_stat = abs_path.stat()
            dst_stat = dest.stat()
            if (
                src_stat.st_size == dst_stat.st_size
                and src_stat.st_mtime <= dst_stat.st_mtime
            ):
                unchanged += 1
                continue

        if args.dry_run:
            print(f"  {rel}  ->  {flat}")
        else:
            shutil.copy2(abs_path, dest)
            copied += 1

    # Summary
    print()
    if args.dry_run:
        to_copy = len(expected) - unchanged
        excluded_count = sum(len(f) for f in exclusion_log.values())
        print(
            f"  [DRY RUN] Would copy {to_copy}, "
            f"skip {unchanged} unchanged, "
            f"remove {removed} stale, "
            f"{excluded_count} excluded by modifiers"
        )
    else:
        # Write manifest
        included_files = [(rel, flat) for flat, (_, rel) in sorted(expected.items())]
        profile_rel = str(profile_path.relative_to(root))
        manifest_content = generate_manifest(
            profile_name=args.profile,
            profile_path=profile_rel,
            included_files=included_files,
            exclusion_log=exclusion_log,
            modifiers=modifiers,
        )
        context_path = output / "_CONTEXT.yaml"
        context_path.write_text(manifest_content)

        parts = [f"Copied {copied}"]
        if unchanged:
            parts.append(f"unchanged {unchanged}")
        if removed:
            parts.append(f"removed {removed} stale")
        if preserved:
            parts.append(f"preserved {preserved} manual")
        print(f"  {', '.join(parts)} -> {output}")
        print(f"  Context: {context_path.relative_to(root)}")

    if not args.dry_run and copied > 0:
        print()
        print("  Upload the contents of this folder to your AI project:")
        print(f"    {output}/")


def cmd_discover(args):
    """Show profile tree with file counts. TODO: Phase 3."""
    print("  discover is not yet implemented (Phase 3)")
    print(f"  Profile: {args.profile}")
    sys.exit(0)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def main():
    parser = argparse.ArgumentParser(
        description="Flatten monorepo for AI project upload."
    )
    subparsers = parser.add_subparsers(dest="command")

    # -- run ---
    run_parser = subparsers.add_parser("run", help="Export files")
    run_parser.add_argument(
        "--profile",
        default="default",
        help="Profile name (default: default)",
    )
    run_parser.add_argument(
        "--root",
        default=".",
        help="Repo root directory (default: .)",
    )
    run_parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Preview without copying",
    )
    run_parser.add_argument(
        "--clean",
        action="store_true",
        help="Force re-copy all files (ignore mtime/size cache)",
    )
    run_parser.add_argument(
        "--preserve",
        nargs="+",
        default=[],
        help="Glob patterns for manually-added files to keep",
    )

    # -- discover ---
    disc_parser = subparsers.add_parser("discover", help="Show profile tree (TODO)")
    disc_parser.add_argument(
        "--profile",
        default="default",
        help="Profile name (default: default)",
    )

    args = parser.parse_args()

    if args.command == "discover":
        cmd_discover(args)
    else:
        # Default to run (handles both explicit 'run' and no subcommand)
        if args.command is None:
            args = run_parser.parse_args()
        cmd_run(args)


if __name__ == "__main__":
    main()
