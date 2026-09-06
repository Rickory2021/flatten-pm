# scripts/flatten-sync/tests/test_modifiers.py
"""Tests for modifier loading and path matching.

Tests that modifiers.yaml is parsed correctly and that the matching
logic correctly identifies files/dirs that match a modifier's
dirs, files, and patterns fields.
"""

import pytest

from lib.modifiers import load_modifiers, matches_modifier


# -- load_modifiers ------------------------------------------------------------


class TestLoadModifiers:
    """Load and parse modifiers.yaml into modifier definitions."""

    @pytest.fixture()
    def modifiers_yaml(self, tmp_path):
        """Write a minimal modifiers.yaml and return path."""
        content = """\
modifiers:
  dev-env:
    silent: true
    dirs: [.git, node_modules, __pycache__]
    note: "Dev environment."
  tests:
    dirs: [tests]
    note: "Test suites."
  corpus:
    dirs: [corpus]
    note: "YAML entries."
  docs:
    files: [STRATEGY.md, REFERENCE.md]
    note: "Strategy docs."
  secrets:
    silent: true
    patterns:
      - '\\.env$'
    note: "Credentials."
"""
        p = tmp_path / "modifiers.yaml"
        p.write_text(content)
        return p

    def test_loads_all_modifiers(self, modifiers_yaml):
        mods = load_modifiers(modifiers_yaml)
        assert "dev-env" in mods
        assert "tests" in mods
        assert "corpus" in mods
        assert "docs" in mods
        assert "secrets" in mods

    def test_silent_flag_parsed(self, modifiers_yaml):
        mods = load_modifiers(modifiers_yaml)
        assert mods["dev-env"]["silent"] is True
        assert mods["tests"].get("silent", False) is False

    def test_dirs_parsed(self, modifiers_yaml):
        mods = load_modifiers(modifiers_yaml)
        assert mods["dev-env"]["dirs"] == [".git", "node_modules", "__pycache__"]

    def test_files_parsed(self, modifiers_yaml):
        mods = load_modifiers(modifiers_yaml)
        assert mods["docs"]["files"] == ["STRATEGY.md", "REFERENCE.md"]

    def test_patterns_parsed(self, modifiers_yaml):
        mods = load_modifiers(modifiers_yaml)
        assert mods["secrets"]["patterns"] == ["\\.env$"]

    def test_note_parsed(self, modifiers_yaml):
        mods = load_modifiers(modifiers_yaml)
        assert mods["tests"]["note"] == "Test suites."

    def test_missing_file_raises(self, tmp_path):
        with pytest.raises(FileNotFoundError):
            load_modifiers(tmp_path / "nonexistent.yaml")

    def test_missing_optional_fields_default_empty(self, tmp_path):
        """Modifier with only dirs has no files or patterns."""
        content = """\
modifiers:
  simple:
    dirs: [foo]
"""
        p = tmp_path / "modifiers.yaml"
        p.write_text(content)
        mods = load_modifiers(p)
        assert mods["simple"].get("files", []) == []
        assert mods["simple"].get("patterns", []) == []


# -- matches_modifier ---------------------------------------------------------


class TestMatchesModifier:
    """Test whether a relative path matches a modifier definition."""

    def test_dir_match_top_level(self):
        """Dir name 'tests' matches path starting with tests/."""
        mod = {"dirs": ["tests"]}
        assert matches_modifier("tests/test_foo.py", mod) is True

    def test_dir_match_nested(self):
        """Dir name 'tests' matches at any depth."""
        mod = {"dirs": ["tests"]}
        assert matches_modifier("lambdas/tests/test_handler.py", mod) is True

    def test_dir_match_deeply_nested(self):
        """Dir name matches even deeply nested."""
        mod = {"dirs": ["tests"]}
        assert matches_modifier("a/b/c/tests/test.py", mod) is True

    def test_dir_no_match(self):
        """Dir name does not match unrelated path."""
        mod = {"dirs": ["tests"]}
        assert matches_modifier("lambdas/src/handler.py", mod) is False

    def test_dir_no_partial_match(self):
        """Dir name 'tests' does not match 'tests_old' or 'mytests'."""
        mod = {"dirs": ["tests"]}
        assert matches_modifier("tests_old/foo.py", mod) is False
        assert matches_modifier("mytests/foo.py", mod) is False

    def test_dir_match_is_component(self):
        """Dir name must be an exact path component, not a substring."""
        mod = {"dirs": ["public"]}
        assert matches_modifier("extension/public/icon/96.png", mod) is True
        assert matches_modifier("publication/article.md", mod) is False

    def test_file_match(self):
        """Filename match at any depth."""
        mod = {"files": ["STRATEGY.md"]}
        assert matches_modifier("benchmarks/extraction/STRATEGY.md", mod) is True

    def test_file_match_top_level(self):
        """Filename match at repo root."""
        mod = {"files": ["README.md"]}
        assert matches_modifier("README.md", mod) is True

    def test_file_no_match(self):
        """Filename does not match different file."""
        mod = {"files": ["STRATEGY.md"]}
        assert matches_modifier("benchmarks/extraction/score.ts", mod) is False

    def test_file_no_partial_match(self):
        """Filename must be exact, not substring."""
        mod = {"files": ["STRATEGY.md"]}
        assert matches_modifier("MY_STRATEGY.md", mod) is False

    def test_pattern_match(self):
        """Regex pattern matches against relative path."""
        mod = {"patterns": ["\\.env$"]}
        assert matches_modifier(".env", mod) is True
        assert matches_modifier("lambdas/.env", mod) is True

    def test_pattern_no_match(self):
        """Regex pattern does not match unrelated path."""
        mod = {"patterns": ["\\.env$"]}
        assert matches_modifier("environment.ts", mod) is False

    def test_pattern_pyc(self):
        """Pattern for .pyc files."""
        mod = {"patterns": ["\\.pyc$"]}
        assert matches_modifier("lambdas/__pycache__/handler.pyc", mod) is True
        assert matches_modifier("handler.py", mod) is False

    def test_pattern_config_yaml(self):
        """Pattern for *.config.yaml files."""
        mod = {"patterns": ["\\.config\\.yaml$"]}
        assert matches_modifier("run.config.yaml", mod) is True
        assert matches_modifier("modifiers.yaml", mod) is False

    def test_multiple_dirs(self):
        """Multiple dirs -- any match triggers."""
        mod = {"dirs": [".git", "node_modules", "__pycache__"]}
        assert matches_modifier(".git/config", mod) is True
        assert matches_modifier("node_modules/react/index.js", mod) is True
        assert matches_modifier("src/app.py", mod) is False

    def test_combined_dirs_files_patterns(self):
        """All fields checked -- any match triggers."""
        mod = {
            "dirs": ["tests"],
            "files": ["STRATEGY.md"],
            "patterns": ["\\.pyc$"],
        }
        assert matches_modifier("tests/test_foo.py", mod) is True
        assert matches_modifier("docs/STRATEGY.md", mod) is True
        assert matches_modifier("cache/handler.pyc", mod) is True
        assert matches_modifier("src/handler.py", mod) is False

    def test_empty_modifier_matches_nothing(self):
        """Modifier with no dirs/files/patterns matches nothing."""
        mod = {}
        assert matches_modifier("any/path/file.py", mod) is False

    def test_dir_match_directory_itself(self):
        """A file directly inside matched dir."""
        mod = {"dirs": ["corpus"]}
        assert matches_modifier("corpus/entry.yaml", mod) is True


class TestSilentModifiers:
    """Verify silent modifier identification."""

    def test_silent_true(self):
        mod = {"silent": True, "dirs": [".git"]}
        assert mod.get("silent", False) is True

    def test_not_silent_default(self):
        mod = {"dirs": ["tests"]}
        assert mod.get("silent", False) is False
