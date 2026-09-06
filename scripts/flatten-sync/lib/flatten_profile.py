# scripts/flatten-sync/lib/flatten_profile.py
"""Flatten profile YAML parsing."""

from pathlib import Path

import yaml


def parse_profile(path: Path) -> dict:
    """Parse a profile YAML file into a tree of nodes.

    Profile YAML uses __ dunder prefix for metadata keys:
    - __path: relative path from parent (required)
    - __modifier: dict with exclude/include lists (optional)
    - __note: description string (optional)

    All other keys at the same level are child folder nodes.

    Args:
        path: Path to the profile YAML file.

    Returns:
        dict of top_level_key -> node, where each node has:
            path: str (resolved path, joined with parent)
            modifier: dict
            note: str or None
            children: dict of child_key -> node

    Raises:
        ValueError: if any node is missing __path.
    """
    with open(path) as f:
        data = yaml.safe_load(f)

    tree = {}
    for key, value in data.items():
        tree[key] = _parse_node(key, value, parent_path=None)
    return tree


def _parse_node(key, raw, parent_path):
    """Parse a single node from raw YAML dict.

    Args:
        key: the node's key name (for error messages)
        raw: the raw YAML dict for this node
        parent_path: resolved path of the parent, or None for top-level
    """
    if "__path" not in raw:
        raise ValueError(f"Node '{key}' is missing required '__path' field")

    local_path = raw["__path"]
    if parent_path is not None and parent_path != ".":
        resolved_path = f"{parent_path}/{local_path}"
    else:
        resolved_path = local_path

    modifier = raw.get("__modifier", {}) or {}
    note = raw.get("__note")

    # Everything without __ prefix is a child node
    children = {}
    for child_key, child_value in raw.items():
        if child_key.startswith("__"):
            continue
        if isinstance(child_value, dict):
            children[child_key] = _parse_node(
                child_key, child_value, parent_path=resolved_path
            )

    return {
        "path": resolved_path,
        "modifier": modifier,
        "note": note,
        "children": children,
    }


def resolve_profile_path(name: str, profiles_dir: Path) -> Path:
    """Resolve a profile name to its YAML file path.

    Resolution order (mirrors watch profile convention):
    1. <name>.flatten.config.yaml       (user's personal config, gitignored)
    2. <name>.flatten.config.example.yaml  (committed reference)

    Args:
        name: profile name (e.g. "default", "bench-code")
        profiles_dir: directory containing profile files

    Raises:
        FileNotFoundError: if neither file exists.
    """
    config_path = profiles_dir / f"{name}.flatten.config.yaml"
    if config_path.exists():
        return config_path

    example_path = profiles_dir / f"{name}.flatten.config.example.yaml"
    if example_path.exists():
        return example_path

    raise FileNotFoundError(
        f"Profile '{name}' not found. Looked for:\n  {config_path}\n  {example_path}"
    )
