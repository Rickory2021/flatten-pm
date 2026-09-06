# scripts/flatten-sync/lib/flatten.py
"""Filename flattening with -- separator."""


def flatten_name(relative_path: str) -> str:
    """Convert a/b/c.py to a--b--c.py using -- separator."""
    return relative_path.replace("/", "--")
