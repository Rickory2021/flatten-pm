# scripts/flatten-sync/tests/test_context.py
"""Tests for _CONTEXT.yaml generation."""

import yaml

from lib.context import generate_manifest


class TestGenerateManifest:
    """Generate _CONTEXT.yaml with AI context preamble and exclusion data."""

    def test_context_preamble_present(self):
        """Manifest includes the AI-facing context section."""
        content = generate_manifest(
            profile_name="default",
            profile_path="scripts/flatten-sync/profiles/default.profile.config.example.yaml",
            included_files=[("Makefile", "Makefile")],
            exclusion_log={},
            modifiers={},
        )
        data = yaml.safe_load(content)
        assert "context" in data
        assert "--" in data["context"]
        assert "path separator" in data["context"]

    def test_separator_documented_in_context(self):
        """Context preamble explains the -- separator convention."""
        content = generate_manifest(
            profile_name="default",
            profile_path="profiles/default.profile.config.example.yaml",
            included_files=[],
            exclusion_log={},
            modifiers={},
        )
        data = yaml.safe_load(content)
        assert "extension--components--App.tsx" in data["context"]

    def test_profile_name_recorded(self):
        """Manifest records the profile name."""
        content = generate_manifest(
            profile_name="bench-code",
            profile_path="profiles/bench-code.profile.config.example.yaml",
            included_files=[],
            exclusion_log={},
            modifiers={},
        )
        data = yaml.safe_load(content)
        assert data["profile"] == "bench-code"

    def test_timestamp_present(self):
        """Manifest includes an ISO timestamp."""
        content = generate_manifest(
            profile_name="default",
            profile_path="profiles/default.profile.config.example.yaml",
            included_files=[],
            exclusion_log={},
            modifiers={},
        )
        data = yaml.safe_load(content)
        assert "exported_at" in data
        assert "T" in data["exported_at"]
        assert data["exported_at"].endswith("Z")

    def test_file_count_correct(self):
        """File count matches number of included files."""
        content = generate_manifest(
            profile_name="default",
            profile_path="profiles/default.profile.config.example.yaml",
            included_files=[
                ("Makefile", "Makefile"),
                ("extension/App.tsx", "extension--App.tsx"),
                ("lambdas/handler.py", "lambdas--handler.py"),
            ],
            exclusion_log={},
            modifiers={},
        )
        data = yaml.safe_load(content)
        assert data["file_count"] == 3

    def test_excluded_count_correct(self):
        """Excluded count sums all files across all modifiers."""
        content = generate_manifest(
            profile_name="default",
            profile_path="profiles/default.profile.config.example.yaml",
            included_files=[("Makefile", "Makefile")],
            exclusion_log={
                "tests": ["lambdas/tests/test_handler.py", "lambdas/tests/test_db.py"],
                "corpus": ["benchmarks/extraction/corpus/entry1.yaml"],
            },
            modifiers={
                "tests": {"dirs": ["tests"], "note": "Test suites."},
                "corpus": {"dirs": ["corpus"], "note": "YAML entries."},
            },
        )
        data = yaml.safe_load(content)
        assert data["excluded_count"] == 3

    def test_modifiers_applied_lists_non_silent(self):
        """Non-silent modifiers appear in modifiers_applied."""
        content = generate_manifest(
            profile_name="default",
            profile_path="profiles/default.profile.config.example.yaml",
            included_files=[],
            exclusion_log={
                "tests": ["lambdas/tests/test_handler.py"],
            },
            modifiers={
                "tests": {"dirs": ["tests"], "note": "Test suites."},
            },
        )
        data = yaml.safe_load(content)
        assert "tests" in data["modifiers_applied"]
        assert data["modifiers_applied"]["tests"]["files"] == 1
        assert data["modifiers_applied"]["tests"]["note"] == "Test suites."

    def test_modifiers_applied_includes_excluded_files(self):
        """Each modifier lists its excluded file paths."""
        content = generate_manifest(
            profile_name="default",
            profile_path="profiles/default.profile.config.example.yaml",
            included_files=[],
            exclusion_log={
                "tests": [
                    "lambdas/tests/test_db.py",
                    "lambdas/tests/test_handler.py",
                ],
            },
            modifiers={
                "tests": {"dirs": ["tests"], "note": "Test suites."},
            },
        )
        data = yaml.safe_load(content)
        excluded = data["modifiers_applied"]["tests"]["excluded"]
        assert "lambdas/tests/test_db.py" in excluded
        assert "lambdas/tests/test_handler.py" in excluded

    def test_silent_modifiers_not_in_manifest(self):
        """Silent modifiers never appear in exclusion_log or manifest."""
        # exclusion_log should never contain silent modifiers
        # (walk_node filters them out). Verify manifest handles
        # an empty log correctly.
        content = generate_manifest(
            profile_name="default",
            profile_path="profiles/default.profile.config.example.yaml",
            included_files=[("handler.py", "handler.py")],
            exclusion_log={},
            modifiers={},
        )
        data = yaml.safe_load(content)
        assert "modifiers_applied" not in data

    def test_excluded_files_sorted(self):
        """Excluded file lists are sorted."""
        content = generate_manifest(
            profile_name="default",
            profile_path="profiles/default.profile.config.example.yaml",
            included_files=[],
            exclusion_log={
                "tests": [
                    "lambdas/tests/test_handler.py",
                    "lambdas/tests/test_db.py",
                    "extension/tests/App.test.tsx",
                ],
            },
            modifiers={"tests": {"dirs": ["tests"], "note": "Test suites."}},
        )
        data = yaml.safe_load(content)
        excluded = data["modifiers_applied"]["tests"]["excluded"]
        assert excluded == sorted(excluded)

    def test_valid_yaml_output(self):
        """Output is valid YAML that round-trips."""
        content = generate_manifest(
            profile_name="default",
            profile_path="profiles/default.profile.config.example.yaml",
            included_files=[("Makefile", "Makefile")],
            exclusion_log={"tests": ["tests/test.py"]},
            modifiers={"tests": {"dirs": ["tests"], "note": "Test suites."}},
        )
        data = yaml.safe_load(content)
        assert isinstance(data, dict)
        assert isinstance(data["modifiers_applied"], dict)
