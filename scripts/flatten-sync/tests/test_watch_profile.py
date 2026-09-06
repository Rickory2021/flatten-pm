# scripts/flatten-sync/tests/test_watch_profile.py
"""Tests for watch profile YAML parsing and resolution.

Validates loading, validation, and name-to-path resolution for
watch profiles (*.watch.config.yaml).
"""

import pytest

from lib.watch_profile import load_watch_profile, resolve_watch_profile_path


# -- load_watch_profile --------------------------------------------------------


class TestLoadWatchProfile:
    """Load and validate watch profile YAML files."""

    def _write_profile(self, tmp_path, content):
        """Write YAML content to a watch profile file and return path."""
        p = tmp_path / "test.watch.config.yaml"
        p.write_text(content)
        return p

    def test_full_config(self, tmp_path):
        """All sections parse correctly."""
        path = self._write_profile(
            tmp_path,
            """\
source:
  windows_user: testuser
target:
  repo: ~/DeeVec
cleanup:
  mode: delete
polling:
  interval: 2.0
  stable_wait: 0.3
  stable_retries: 4
""",
        )
        config = load_watch_profile(path)
        assert config["source"]["windows_user"] == "testuser"
        assert config["target"]["repo"] == "~/DeeVec"
        assert config["cleanup"]["mode"] == "delete"
        assert config["polling"]["interval"] == 2.0

    def test_minimal_config(self, tmp_path):
        """Only target.repo is needed at runtime; other sections optional."""
        path = self._write_profile(
            tmp_path,
            """\
target:
  repo: ~/DeeVec
""",
        )
        config = load_watch_profile(path)
        assert config["target"]["repo"] == "~/DeeVec"
        assert "source" not in config
        assert "cleanup" not in config

    def test_empty_file_returns_empty_dict(self, tmp_path):
        """Empty YAML file returns empty dict (runtime validates later)."""
        path = self._write_profile(tmp_path, "")
        config = load_watch_profile(path)
        assert config == {}

    def test_move_mode_requires_move_to(self, tmp_path):
        """Cleanup mode 'move' without move_to raises ValueError."""
        path = self._write_profile(
            tmp_path,
            """\
target:
  repo: ~/DeeVec
cleanup:
  mode: move
""",
        )
        with pytest.raises(ValueError, match="move_to"):
            load_watch_profile(path)

    def test_move_mode_with_move_to_ok(self, tmp_path):
        """Cleanup mode 'move' with move_to is valid."""
        path = self._write_profile(
            tmp_path,
            """\
target:
  repo: ~/DeeVec
cleanup:
  mode: move
  move_to: ~/processed
""",
        )
        config = load_watch_profile(path)
        assert config["cleanup"]["mode"] == "move"
        assert config["cleanup"]["move_to"] == "~/processed"

    def test_delete_mode_no_move_to_ok(self, tmp_path):
        """Cleanup mode 'delete' does not require move_to."""
        path = self._write_profile(
            tmp_path,
            """\
target:
  repo: ~/DeeVec
cleanup:
  mode: delete
""",
        )
        config = load_watch_profile(path)
        assert config["cleanup"]["mode"] == "delete"

    def test_source_directory_override(self, tmp_path):
        """Custom source directory is preserved."""
        path = self._write_profile(
            tmp_path,
            """\
source:
  directory: /tmp/my-downloads
target:
  repo: ~/DeeVec
""",
        )
        config = load_watch_profile(path)
        assert config["source"]["directory"] == "/tmp/my-downloads"

    def test_source_auto_user(self, tmp_path):
        """windows_user: auto is a valid value."""
        path = self._write_profile(
            tmp_path,
            """\
source:
  windows_user: auto
target:
  repo: ~/DeeVec
""",
        )
        config = load_watch_profile(path)
        assert config["source"]["windows_user"] == "auto"


# -- resolve_watch_profile_path ------------------------------------------------


class TestResolveWatchProfilePath:
    """Resolve profile name to YAML file with user > example fallback."""

    def test_finds_user_config(self, tmp_path):
        """User config takes priority over example."""
        (tmp_path / "default.watch.config.yaml").write_text(
            "target:\n  repo: ~/DeeVec\n"
        )
        (tmp_path / "default.watch.config.example.yaml").write_text(
            "target:\n  repo: ~/DeeVec\n"
        )
        result = resolve_watch_profile_path("default", tmp_path)
        assert result.name == "default.watch.config.yaml"

    def test_falls_back_to_example(self, tmp_path):
        """Falls back to example when user config absent."""
        (tmp_path / "default.watch.config.example.yaml").write_text(
            "target:\n  repo: ~/DeeVec\n"
        )
        result = resolve_watch_profile_path("default", tmp_path)
        assert result.name == "default.watch.config.example.yaml"

    def test_not_found_raises(self, tmp_path):
        """Neither file exists raises FileNotFoundError."""
        with pytest.raises(FileNotFoundError):
            resolve_watch_profile_path("nonexistent", tmp_path)

    def test_named_profile(self, tmp_path):
        """Non-default profile name resolves correctly."""
        (tmp_path / "work-laptop.watch.config.yaml").write_text(
            "target:\n  repo: ~/work/DeeVec\n"
        )
        result = resolve_watch_profile_path("work-laptop", tmp_path)
        assert result.name == "work-laptop.watch.config.yaml"

    def test_does_not_find_flatten_profiles(self, tmp_path):
        """Watch resolution ignores flatten profile naming convention."""
        (tmp_path / "default.profile.config.yaml").write_text("root:\n  __path: .\n")
        with pytest.raises(FileNotFoundError):
            resolve_watch_profile_path("default", tmp_path)
