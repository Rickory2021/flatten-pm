# scripts/flatten-sync/lib/watch_profile.py
"""Watch profile YAML parsing and resolution."""

from pathlib import Path

import yaml


def load_watch_profile(path: Path) -> dict:
    """Load and validate a watch profile YAML file.

    Returns the raw config dict with sections: source, target, cleanup, polling.
    All sections are optional except target.repo when used at runtime.
    """
    with open(path) as f:
        data = yaml.safe_load(f) or {}

    cleanup = data.get("cleanup", {})
    if cleanup.get("mode") == "move" and "move_to" not in cleanup:
        raise ValueError(f"Profile {path.name}: cleanup mode 'move' requires 'move_to'")

    return data


def resolve_watch_profile_path(name: str, profiles_dir: Path) -> Path:
    """Resolve a watch profile name to its YAML file path.

    Resolution order (mirrors flatten profile convention):
    1. <name>.watch.config.yaml       (user's personal config, gitignored)
    2. <name>.watch.config.example.yaml  (committed reference)

    Args:
        name: profile name (e.g. "default", "work-laptop")
        profiles_dir: directory containing profile files

    Raises:
        FileNotFoundError: if neither file exists.
    """
    config_path = profiles_dir / f"{name}.watch.config.yaml"
    if config_path.exists():
        return config_path

    example_path = profiles_dir / f"{name}.watch.config.example.yaml"
    if example_path.exists():
        return example_path

    raise FileNotFoundError(
        f"Watch profile '{name}' not found. Looked for:\n"
        f"  {config_path}\n  {example_path}"
    )
