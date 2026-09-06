# scripts/flatten-sync/tests/test_walk.py
"""Tests for filesystem walking with modifier application.

Integration tests that create real temporary directory structures
and verify the walk logic correctly applies modifiers, skips
children, and handles unclaimed directories.
"""

import pytest

from lib.walk import walk_node, collect_files
from lib.flatten import flatten_name


# -- Helpers -------------------------------------------------------------------


def _create_tree(base, structure):
    """Create a directory tree from a nested dict.

    Keys ending with / are directories. Values are either:
    - str: file content
    - dict: subdirectory contents
    """
    for name, content in structure.items():
        if isinstance(content, dict):
            d = base / name
            d.mkdir(parents=True, exist_ok=True)
            _create_tree(d, content)
        else:
            f = base / name
            f.parent.mkdir(parents=True, exist_ok=True)
            f.write_text(content)


# Modifier definitions matching the library format
MODIFIERS = {
    "dev-env": {
        "silent": True,
        "dirs": [".git", "node_modules", "__pycache__"],
    },
    "build-output": {
        "silent": True,
        "dirs": [".aws-sam", "dist", "build"],
    },
    "generated-files": {
        "silent": True,
        "files": [".DS_Store"],
        "patterns": ["\\.pyc$", "package-lock\\.json$"],
    },
    "secrets": {
        "silent": True,
        "patterns": ["\\.env$"],
    },
    "active-configs": {
        "silent": True,
        "patterns": ["\\.config\\.yaml$"],
    },
    "tests": {"dirs": ["tests"]},
    "corpus": {"dirs": ["corpus"]},
    "models": {"dirs": ["models"]},
    "assets": {"dirs": ["public"]},
}


# -- walk_node -----------------------------------------------------------------


class TestWalkNode:
    """Test walking a single node's filesystem path."""

    def test_walks_all_files(self, tmp_path):
        """Walks all files under a path with no modifiers."""
        _create_tree(
            tmp_path,
            {
                "src": {
                    "handler.py": "code",
                    "utils.py": "code",
                },
                "README.md": "readme",
            },
        )
        files = walk_node(
            root=tmp_path,
            node_path=tmp_path,
            resolved_excludes=set(),
            modifiers=MODIFIERS,
            child_paths=[],
        )
        rel_paths = {str(f.relative_to(tmp_path)) for f in files}
        assert "src/handler.py" in rel_paths
        assert "src/utils.py" in rel_paths
        assert "README.md" in rel_paths

    def test_silent_modifier_excludes_dir(self, tmp_path):
        """Silent modifier activated via profile excludes matching dirs."""
        _create_tree(
            tmp_path,
            {
                "src": {"handler.py": "code"},
                "node_modules": {"react": {"index.js": "react"}},
                "__pycache__": {"handler.cpython-314.pyc": "bytecode"},
            },
        )
        files = walk_node(
            root=tmp_path,
            node_path=tmp_path,
            resolved_excludes={"dev-env"},
            modifiers=MODIFIERS,
            child_paths=[],
        )
        rel_paths = {str(f.relative_to(tmp_path)) for f in files}
        assert "src/handler.py" in rel_paths
        assert not any("node_modules" in p for p in rel_paths)
        assert not any("__pycache__" in p for p in rel_paths)

    def test_silent_modifier_excludes_pattern(self, tmp_path):
        """Silent modifier activated via profile excludes matching patterns."""
        _create_tree(
            tmp_path,
            {
                "handler.py": "code",
                ".env": "SECRET=x",
                "run.config.yaml": "config: true",
            },
        )
        files = walk_node(
            root=tmp_path,
            node_path=tmp_path,
            resolved_excludes={"secrets", "active-configs"},
            modifiers=MODIFIERS,
            child_paths=[],
        )
        rel_paths = {str(f.relative_to(tmp_path)) for f in files}
        assert "handler.py" in rel_paths
        assert ".env" not in rel_paths
        assert "run.config.yaml" not in rel_paths

    def test_silent_modifier_excludes_file(self, tmp_path):
        """Silent modifier activated via profile excludes specific filenames."""
        _create_tree(
            tmp_path,
            {
                "handler.py": "code",
                ".DS_Store": "junk",
            },
        )
        files = walk_node(
            root=tmp_path,
            node_path=tmp_path,
            resolved_excludes={"generated-files"},
            modifiers=MODIFIERS,
            child_paths=[],
        )
        rel_paths = {str(f.relative_to(tmp_path)) for f in files}
        assert "handler.py" in rel_paths
        assert ".DS_Store" not in rel_paths

    def test_silent_modifier_inactive_without_profile(self, tmp_path):
        """Silent flag does not auto-activate; profile controls activation."""
        _create_tree(
            tmp_path,
            {
                "handler.py": "code",
                "node_modules": {"react": {"index.js": "react"}},
                ".env": "SECRET=x",
            },
        )
        files = walk_node(
            root=tmp_path,
            node_path=tmp_path,
            resolved_excludes=set(),
            modifiers=MODIFIERS,
            child_paths=[],
        )
        rel_paths = {str(f.relative_to(tmp_path)) for f in files}
        assert "handler.py" in rel_paths
        assert "node_modules/react/index.js" in rel_paths
        assert ".env" in rel_paths

    def test_toggled_modifier_excludes_dir(self, tmp_path):
        """Resolved exclude 'tests' excludes tests/ directory."""
        _create_tree(
            tmp_path,
            {
                "src": {"handler.py": "code"},
                "tests": {"test_handler.py": "test"},
            },
        )
        files = walk_node(
            root=tmp_path,
            node_path=tmp_path,
            resolved_excludes={"tests"},
            modifiers=MODIFIERS,
            child_paths=[],
        )
        rel_paths = {str(f.relative_to(tmp_path)) for f in files}
        assert "src/handler.py" in rel_paths
        assert not any("tests" in p for p in rel_paths)

    def test_child_path_skipped(self, tmp_path):
        """Directories claimed as children are skipped from parent walk."""
        _create_tree(
            tmp_path,
            {
                "extension": {"App.tsx": "react"},
                "lambdas": {"handler.py": "python"},
                "README.md": "readme",
            },
        )
        files = walk_node(
            root=tmp_path,
            node_path=tmp_path,
            resolved_excludes=set(),
            modifiers=MODIFIERS,
            child_paths=[tmp_path / "extension", tmp_path / "lambdas"],
        )
        rel_paths = {str(f.relative_to(tmp_path)) for f in files}
        # Only root-level files, not children
        assert "README.md" in rel_paths
        assert "extension/App.tsx" not in rel_paths
        assert "lambdas/handler.py" not in rel_paths

    def test_unclaimed_dirs_walked_with_parent_modifiers(self, tmp_path):
        """Dirs not claimed as children get parent modifiers."""
        _create_tree(
            tmp_path,
            {
                "scripts": {"build.sh": "#!/bin/bash"},
                "docs": {"guide.md": "guide"},
            },
        )
        # No child_paths, so scripts/ and docs/ are unclaimed
        files = walk_node(
            root=tmp_path,
            node_path=tmp_path,
            resolved_excludes=set(),
            modifiers=MODIFIERS,
            child_paths=[],
        )
        rel_paths = {str(f.relative_to(tmp_path)) for f in files}
        assert "scripts/build.sh" in rel_paths
        assert "docs/guide.md" in rel_paths

    def test_nonexistent_path_returns_empty(self, tmp_path):
        """Walking a nonexistent path returns empty list (with warning)."""
        with pytest.warns(UserWarning, match="does not exist"):
            files = walk_node(
                root=tmp_path,
                node_path=tmp_path / "nonexistent",
                resolved_excludes=set(),
                modifiers=MODIFIERS,
                child_paths=[],
            )
        assert files == []

    def test_nested_excluded_dir(self, tmp_path):
        """Excluded dir matched at nested depth."""
        _create_tree(
            tmp_path,
            {
                "lambdas": {
                    "src": {"handler.py": "code"},
                    "tests": {"test_handler.py": "test"},
                },
            },
        )
        files = walk_node(
            root=tmp_path,
            node_path=tmp_path,
            resolved_excludes={"tests"},
            modifiers=MODIFIERS,
            child_paths=[],
        )
        rel_paths = {str(f.relative_to(tmp_path)) for f in files}
        assert "lambdas/src/handler.py" in rel_paths
        assert "lambdas/tests/test_handler.py" not in rel_paths


# -- exclusion_log -------------------------------------------------------------


class TestExclusionLog:
    """Track which non-silent modifiers excluded which files."""

    def test_toggled_modifier_logged(self, tmp_path):
        """Non-silent excluded modifier records its excluded files."""
        _create_tree(
            tmp_path,
            {
                "src": {"handler.py": "code"},
                "tests": {"test_handler.py": "test", "test_db.py": "test"},
            },
        )
        log = {}
        walk_node(
            root=tmp_path,
            node_path=tmp_path,
            resolved_excludes={"tests"},
            modifiers=MODIFIERS,
            child_paths=[],
            exclusion_log=log,
        )
        assert "tests" in log
        assert len(log["tests"]) == 2

    def test_silent_modifier_not_logged(self, tmp_path):
        """Silent modifiers activated via profile are excluded but never logged."""
        _create_tree(
            tmp_path,
            {
                "handler.py": "code",
                ".env": "SECRET=x",
                "node_modules": {"react": {"index.js": "react"}},
            },
        )
        log = {}
        walk_node(
            root=tmp_path,
            node_path=tmp_path,
            resolved_excludes={"secrets", "dev-env"},
            modifiers=MODIFIERS,
            child_paths=[],
            exclusion_log=log,
        )
        # .env matched by secrets (silent), node_modules by dev-env (silent)
        # Both are active via profile but silent suppresses logging
        assert log == {}

    def test_no_log_when_none(self, tmp_path):
        """Passing None for exclusion_log does not crash."""
        _create_tree(
            tmp_path,
            {
                "tests": {"test_handler.py": "test"},
            },
        )
        files = walk_node(
            root=tmp_path,
            node_path=tmp_path,
            resolved_excludes={"tests"},
            modifiers=MODIFIERS,
            child_paths=[],
            exclusion_log=None,
        )
        assert files == []

    def test_multiple_modifiers_logged_separately(self, tmp_path):
        """Different modifiers track their exclusions independently."""
        _create_tree(
            tmp_path,
            {
                "src": {"handler.py": "code"},
                "tests": {"test_handler.py": "test"},
                "corpus": {"entry.yaml": "entry"},
            },
        )
        log = {}
        walk_node(
            root=tmp_path,
            node_path=tmp_path,
            resolved_excludes={"tests", "corpus"},
            modifiers=MODIFIERS,
            child_paths=[],
            exclusion_log=log,
        )
        assert "tests" in log
        assert "corpus" in log
        assert len(log["tests"]) == 1
        assert len(log["corpus"]) == 1

    def test_exclusion_paths_relative_to_root(self, tmp_path):
        """Excluded file paths are relative to root, not node_path."""
        _create_tree(
            tmp_path,
            {
                "lambdas": {
                    "tests": {"test_handler.py": "test"},
                },
            },
        )
        log = {}
        walk_node(
            root=tmp_path,
            node_path=tmp_path / "lambdas",
            resolved_excludes={"tests"},
            modifiers=MODIFIERS,
            child_paths=[],
            exclusion_log=log,
        )
        assert "tests" in log
        assert log["tests"][0] == "lambdas/tests/test_handler.py"

    def test_silent_files_inside_excluded_dir_not_logged(self, tmp_path):
        """Files matching silent modifiers inside a non-silent excluded dir
        are filtered out of the exclusion log."""
        _create_tree(
            tmp_path,
            {
                "corpus": {
                    "entry.yaml": "entry",
                    "another.yaml": "entry",
                    "__pycache__": {"chunker.cpython-310.pyc": "bytecode"},
                    ".env": "SECRET=x",
                },
            },
        )
        log = {}
        walk_node(
            root=tmp_path,
            node_path=tmp_path,
            resolved_excludes={"corpus"},
            modifiers=MODIFIERS,
            child_paths=[],
            exclusion_log=log,
        )
        assert "corpus" in log
        # Only the .yaml files, not __pycache__/*.pyc or .env
        assert len(log["corpus"]) == 2
        assert all(f.endswith(".yaml") for f in log["corpus"])


# -- collect_files -------------------------------------------------------------


class TestCollectFiles:
    """Test orchestrated collection across multiple nodes."""

    def test_collects_from_multiple_nodes(self, tmp_path):
        """Collects files from root + children with different modifiers."""
        _create_tree(
            tmp_path,
            {
                "Makefile": "all:",
                "extension": {
                    "App.tsx": "react",
                    "public": {"icon.png": "png"},
                },
                "lambdas": {
                    "handler.py": "python",
                    "tests": {"test_handler.py": "test"},
                },
            },
        )

        # Simulated parsed tree with resolved excludes
        nodes = [
            {
                "key": "root",
                "path": tmp_path,
                "resolved_excludes": set(),
                "child_paths": [tmp_path / "extension", tmp_path / "lambdas"],
            },
            {
                "key": "root.extension",
                "path": tmp_path / "extension",
                "resolved_excludes": {"assets"},
                "child_paths": [],
            },
            {
                "key": "root.lambdas",
                "path": tmp_path / "lambdas",
                "resolved_excludes": {"tests"},
                "child_paths": [],
            },
        ]

        all_files = collect_files(nodes, tmp_path, MODIFIERS)
        rel_paths = {str(f.relative_to(tmp_path)) for f in all_files}

        # Root gets Makefile
        assert "Makefile" in rel_paths
        # Extension gets App.tsx but not public/ (assets excluded)
        assert "extension/App.tsx" in rel_paths
        assert "extension/public/icon.png" not in rel_paths
        # Lambdas gets handler.py but not tests/
        assert "lambdas/handler.py" in rel_paths
        assert "lambdas/tests/test_handler.py" not in rel_paths


# -- flatten_name --------------------------------------------------------------


class TestFlattenName:
    """Convert relative paths to flat filenames using -- separator."""

    def test_simple_path(self):
        assert flatten_name("extension/App.tsx") == "extension--App.tsx"

    def test_nested_path(self):
        assert flatten_name("lambdas/src/shared_layer/shared/db.py") == (
            "lambdas--src--shared_layer--shared--db.py"
        )

    def test_root_file(self):
        assert flatten_name("Makefile") == "Makefile"

    def test_init_py(self):
        """__init__.py stays clean with -- separator."""
        assert flatten_name("shared/__init__.py") == "shared--__init__.py"

    def test_dunder_in_path(self):
        """Double underscores in dir names are preserved."""
        assert flatten_name("__pycache__/handler.pyc") == ("__pycache__--handler.pyc")

    def test_dotfile(self):
        assert flatten_name(".github/workflows/ci.yml") == (
            ".github--workflows--ci.yml"
        )

    def test_single_dash_in_name(self):
        """Single dashes in filenames are visually distinct from -- separator."""
        assert flatten_name("embedding-speed/runner.py") == (
            "embedding-speed--runner.py"
        )

    def test_env_example(self):
        assert flatten_name("database/.env.example") == ("database--.env.example")
