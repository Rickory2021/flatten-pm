# scripts/flatten-sync/lib/modifiers.py
"""Modifier library loading and path matching."""

import re
from pathlib import Path

import yaml


def load_modifiers(path: Path) -> dict:
    """Load and parse modifiers.yaml.

    Args:
        path: Path to modifiers.yaml file.

    Returns:
        dict of modifier_name -> modifier definition dict.

    Raises:
        FileNotFoundError: if path does not exist.
    """
    if not path.exists():
        raise FileNotFoundError(f"Modifiers file not found: {path}")
    with open(path) as f:
        data = yaml.safe_load(f)
    return data.get("modifiers", {})


def matches_modifier(relative_path: str, modifier: dict) -> bool:
    """Check if a relative path matches a modifier's dirs/files/patterns.

    Matching rules:
    - dirs: directory name matches an exact path component (not filename)
    - files: filename matches the basename of the path
    - patterns: regex matches against the full relative path

    Any single match returns True.
    """
    parts = Path(relative_path).parts
    basename = parts[-1] if parts else ""
    # Directory components (everything except the filename)
    dir_parts = parts[:-1] if len(parts) > 1 else ()

    # Check dirs -- exact component match in parent directories
    for dir_name in modifier.get("dirs", []):
        if dir_name in dir_parts:
            return True

    # Check files -- exact basename match at any depth
    for file_name in modifier.get("files", []):
        if basename == file_name:
            return True

    # Check patterns -- regex against full relative path
    for pattern in modifier.get("patterns", []):
        if re.search(pattern, relative_path):
            return True

    return False
