# scripts/flatten-sync/lib/walk.py
"""Filesystem walking with modifier application."""

import os
import warnings
from pathlib import Path

from .modifiers import matches_modifier


def walk_node(
    root: Path,
    node_path: Path,
    resolved_excludes: set,
    modifiers: dict,
    child_paths: list[Path],
    exclusion_log: dict | None = None,
) -> list[Path]:
    """Walk a single node's path, applying modifiers and skipping children.

    Filtering is controlled entirely by resolved_excludes (from the profile
    cascade). The silent flag on a modifier only controls whether its
    exclusions appear in the manifest -- it does not auto-activate.

    Directories claimed by child nodes are skipped entirely -- they
    will be walked separately with their own modifier stack.

    Args:
        root: repo root (for relative path computation)
        node_path: absolute path to walk
        resolved_excludes: set of modifier names to exclude at this node
        modifiers: full modifier library dict
        child_paths: absolute paths claimed by child nodes (skipped)
        exclusion_log: if provided, populated with {mod_name: [rel_paths]}
            for non-silent modifiers only. Silent exclusions are not tracked.

    Returns:
        list of absolute file paths that pass all filters.
    """
    if not node_path.exists():
        warnings.warn(f"Path does not exist, skipping: {node_path}")
        return []

    # Build list of active modifiers with metadata:
    #   (mod_name, mod_def, is_silent)
    # Only profile-resolved excludes activate filtering. The silent flag
    # controls manifest suppression only, not auto-activation.
    active_mods = []
    for mod_name in resolved_excludes:
        if mod_name in modifiers:
            mod_def = modifiers[mod_name]
            is_silent = mod_def.get("silent", False)
            active_mods.append((mod_name, mod_def, is_silent))

    # Map dir names to their first matching modifier
    # Used for fast dir-level pruning
    excluded_dirs = {}  # dir_name -> (mod_name, is_silent)
    for mod_name, mod_def, is_silent in active_mods:
        for d in mod_def.get("dirs", []):
            if d not in excluded_dirs:
                excluded_dirs[d] = (mod_name, is_silent)

    # Build silent checkers from the full modifier library.
    # Files inside a non-silent excluded dir that match a silent modifier
    # are noise and should not appear in the manifest. This is the only
    # effect of the silent flag -- it suppresses manifest entries, it does
    # not auto-activate filtering.
    silent_dir_names = set()
    silent_mods = []
    for mod_def in modifiers.values():
        if mod_def.get("silent", False):
            for d in mod_def.get("dirs", []):
                silent_dir_names.add(d)
            silent_mods.append(mod_def)

    def _silently_excluded(file_path, rel_from_node):
        """Check if a file would be caught by a silent modifier."""
        # Check if any parent dir matches a silent dir
        for part in file_path.relative_to(node_path).parts[:-1]:
            if part in silent_dir_names:
                return True
        # Check file-level silent patterns
        for mod_def in silent_mods:
            if matches_modifier(rel_from_node, mod_def):
                return True
        return False

    # Normalize child paths for fast lookup
    child_path_set = {p.resolve() for p in child_paths}

    results = []
    for dirpath_str, dirnames, filenames in os.walk(node_path):
        dirpath = Path(dirpath_str)

        # Prune directories in-place (topdown=True default)
        kept_dirs = []
        for d in sorted(dirnames):
            full = dirpath / d
            if full.resolve() in child_path_set:
                continue
            if d in excluded_dirs:
                mod_name, is_silent = excluded_dirs[d]
                if exclusion_log is not None and not is_silent:
                    # Walk the pruned dir, skip files caught by silent modifiers
                    for excluded_file in sorted(full.rglob("*")):
                        if not excluded_file.is_file():
                            continue
                        rel_node = str(excluded_file.relative_to(node_path))
                        if _silently_excluded(excluded_file, rel_node):
                            continue
                        rel = str(excluded_file.relative_to(root))
                        exclusion_log.setdefault(mod_name, []).append(rel)
                continue
            kept_dirs.append(d)
        dirnames[:] = kept_dirs

        # Check each file against active modifiers
        for fname in sorted(filenames):
            fpath = dirpath / fname
            rel_from_node = str(fpath.relative_to(node_path))
            rel_from_root = str(fpath.relative_to(root))

            excluded = False
            for mod_name, mod_def, is_silent in active_mods:
                if matches_modifier(rel_from_node, mod_def):
                    if exclusion_log is not None and not is_silent:
                        exclusion_log.setdefault(mod_name, []).append(rel_from_root)
                    excluded = True
                    break
            if not excluded:
                results.append(fpath)

    return results


def collect_files(
    nodes: list[dict],
    root: Path,
    modifiers: dict,
    exclusion_log: dict | None = None,
) -> list[Path]:
    """Orchestrate walk across all nodes.

    Args:
        nodes: list of dicts, each with:
            key: dotted key name
            path: absolute Path to walk
            resolved_excludes: set of modifier names
            child_paths: list of absolute Paths claimed by children
        root: repo root
        modifiers: full modifier library dict
        exclusion_log: if provided, populated with {mod_name: [rel_paths]}

    Returns:
        combined list of absolute file paths from all nodes.
    """
    results = []
    for node in nodes:
        files = walk_node(
            root=root,
            node_path=node["path"],
            resolved_excludes=node["resolved_excludes"],
            modifiers=modifiers,
            child_paths=node["child_paths"],
            exclusion_log=exclusion_log,
        )
        results.extend(files)
    return results
