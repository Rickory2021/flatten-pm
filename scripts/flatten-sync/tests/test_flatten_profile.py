# scripts/flatten-sync/tests/test_flatten_profile.py
"""Tests for flatten profile YAML parsing.

Validates that profile YAML files are correctly parsed into a tree
structure, with __ dunder-prefixed keys as metadata and everything
else as child folder nodes.
"""

import pytest

from lib.flatten_profile import parse_profile, resolve_profile_path


# -- parse_profile -------------------------------------------------------------


class TestParseProfile:
    """Parse profile YAML into tree nodes with dunder metadata extraction."""

    def _write_profile(self, tmp_path, content):
        """Write YAML content to a profile file and return path."""
        p = tmp_path / "test.flatten.config.yaml"
        p.write_text(content)
        return p

    def test_simple_root_node(self, tmp_path):
        """Single root node with __path extracts correctly."""
        path = self._write_profile(
            tmp_path,
            """\
root:
  __path: .
  __note: "Full repo"
""",
        )
        tree = parse_profile(path)
        assert "root" in tree
        assert tree["root"]["path"] == "."
        assert tree["root"]["children"] == {}

    def test_dunder_keys_extracted_as_metadata(self, tmp_path):
        """Keys starting with __ are metadata, not children."""
        path = self._write_profile(
            tmp_path,
            """\
root:
  __path: .
  __modifier:
    exclude: [tests]
  __note: "Full repo"
""",
        )
        tree = parse_profile(path)
        node = tree["root"]
        assert node["path"] == "."
        assert node["modifier"] == {"exclude": ["tests"]}
        assert node["note"] == "Full repo"
        assert node["children"] == {}

    def test_non_dunder_keys_are_children(self, tmp_path):
        """Keys without __ prefix are child folder nodes."""
        path = self._write_profile(
            tmp_path,
            """\
root:
  __path: .
  extension:
    __path: extension
  lambdas:
    __path: lambdas
""",
        )
        tree = parse_profile(path)
        children = tree["root"]["children"]
        assert "extension" in children
        assert "lambdas" in children
        assert children["extension"]["path"] == "extension"
        assert children["lambdas"]["path"] == "lambdas"

    def test_path_required_missing_raises(self, tmp_path):
        """Node without __path raises an error."""
        path = self._write_profile(
            tmp_path,
            """\
root:
  __note: "No path"
""",
        )
        with pytest.raises(ValueError, match="__path"):
            parse_profile(path)

    def test_child_path_resolution(self, tmp_path):
        """Child __path is relative to parent __path."""
        path = self._write_profile(
            tmp_path,
            """\
root:
  __path: benchmarks
  extraction:
    __path: extraction
""",
        )
        tree = parse_profile(path)
        child = tree["root"]["children"]["extraction"]
        assert child["path"] == "benchmarks/extraction"

    def test_multilevel_path_resolution(self, tmp_path):
        """Multi-level child path joins correctly."""
        path = self._write_profile(
            tmp_path,
            """\
root:
  __path: benchmarks
  extraction:
    __path: extraction
    src:
      __path: src
""",
        )
        tree = parse_profile(path)
        grandchild = tree["root"]["children"]["extraction"]["children"]["src"]
        assert grandchild["path"] == "benchmarks/extraction/src"

    def test_modifier_optional(self, tmp_path):
        """Node without __modifier gets empty dict."""
        path = self._write_profile(
            tmp_path,
            """\
root:
  __path: .
""",
        )
        tree = parse_profile(path)
        assert tree["root"]["modifier"] == {}

    def test_note_optional(self, tmp_path):
        """Node without __note gets None or empty string."""
        path = self._write_profile(
            tmp_path,
            """\
root:
  __path: .
""",
        )
        tree = parse_profile(path)
        assert tree["root"].get("note") in (None, "")

    def test_leaf_node_no_children(self, tmp_path):
        """Node with no non-dunder keys is a leaf."""
        path = self._write_profile(
            tmp_path,
            """\
root:
  __path: .
  __modifier:
    exclude: [tests]
  __note: "Leaf"
""",
        )
        tree = parse_profile(path)
        assert tree["root"]["children"] == {}

    def test_multiple_top_level_nodes(self, tmp_path):
        """Profile can have multiple top-level nodes."""
        path = self._write_profile(
            tmp_path,
            """\
extension:
  __path: extension
lambdas:
  __path: lambdas
""",
        )
        tree = parse_profile(path)
        assert "extension" in tree
        assert "lambdas" in tree
        assert tree["extension"]["path"] == "extension"
        assert tree["lambdas"]["path"] == "lambdas"

    def test_top_level_path_not_joined(self, tmp_path):
        """Top-level nodes have no parent, so __path is used as-is."""
        path = self._write_profile(
            tmp_path,
            """\
root:
  __path: .
  extension:
    __path: extension
""",
        )
        tree = parse_profile(path)
        # Top-level path used directly
        assert tree["root"]["path"] == "."
        # Child joined with parent
        assert tree["root"]["children"]["extension"]["path"] == "extension"

    def test_nested_exclude_list_preserved(self, tmp_path):
        """Nested list syntax in modifier is preserved for cascade to flatten."""
        path = self._write_profile(
            tmp_path,
            """\
root:
  __path: .
  __modifier:
    exclude:
      - [dev-env, build-output, generated-files]
      - tests
""",
        )
        tree = parse_profile(path)
        excludes = tree["root"]["modifier"]["exclude"]
        # YAML parses this as [["dev-env", ...], "tests"]
        assert isinstance(excludes[0], list)
        assert excludes[1] == "tests"

    def test_default_profile_parses(self):
        """The frozen default profile example parses without error."""
        from pathlib import Path

        profile_path = (
            Path(__file__).parent.parent
            / "profiles"
            / "default.flatten.config.example.yaml"
        )
        if not profile_path.exists():
            pytest.skip("default profile not available")
        tree = parse_profile(profile_path)
        assert "root" in tree
        assert tree["root"]["path"] == "."


# -- resolve_profile_path -----------------------------------------------------


class TestResolveProfilePath:
    """Resolve profile name to YAML file with .config.yaml / .example.yaml fallback."""

    def test_finds_config_yaml(self, tmp_path):
        """Finds <name>.flatten.config.yaml first."""
        profiles_dir = tmp_path / "profiles"
        profiles_dir.mkdir()
        (profiles_dir / "myprofile.flatten.config.yaml").write_text(
            "root:\n  __path: .\n"
        )
        (profiles_dir / "myprofile.flatten.config.example.yaml").write_text(
            "root:\n  __path: .\n"
        )

        result = resolve_profile_path("myprofile", profiles_dir)
        assert result.name == "myprofile.flatten.config.yaml"

    def test_falls_back_to_example(self, tmp_path):
        """Falls back to <name>.flatten.config.example.yaml."""
        profiles_dir = tmp_path / "profiles"
        profiles_dir.mkdir()
        (profiles_dir / "myprofile.flatten.config.example.yaml").write_text(
            "root:\n  __path: .\n"
        )

        result = resolve_profile_path("myprofile", profiles_dir)
        assert result.name == "myprofile.flatten.config.example.yaml"

    def test_not_found_raises(self, tmp_path):
        """Neither file exists -> FileNotFoundError."""
        profiles_dir = tmp_path / "profiles"
        profiles_dir.mkdir()

        with pytest.raises(FileNotFoundError):
            resolve_profile_path("nonexistent", profiles_dir)
