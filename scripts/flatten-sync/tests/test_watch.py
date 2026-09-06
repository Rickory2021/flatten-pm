# scripts/flatten-sync/tests/test_watch.py
"""Tests for the download watcher detection cascade.

Covers each detection tier in isolation and the full cascade ordering.
Uses tmp_path for filesystem fixtures - no real repo needed.
"""

import json


from watch import (
    _detect_comment,
    _detect_embedded,
    _detect_json_path,
    _read_head,
    _valid_target,
    build_repo_map,
    detect,
)

# UTF-8 BOM as raw bytes (EF BB BF). Browser downloads sometimes prepend this.
_BOM_BYTES = b"\xef\xbb\xbf"


# -- _read_head ----------------------------------------------------------------


class TestReadHead:
    """Read first N lines from a file, skipping binary."""

    def test_utf8_file(self, tmp_path):
        """Returns stripped lines from a UTF-8 file."""
        f = tmp_path / "test.py"
        f.write_text("# line one\nline two\nline three\n")
        lines = _read_head(f)
        assert lines is not None
        assert lines[0] == "# line one"
        assert lines[1] == "line two"

    def test_binary_file_returns_none(self, tmp_path):
        """Binary content with null bytes returns None."""
        f = tmp_path / "image.jpg"
        f.write_bytes(b"\xff\xd8\xff\xe0\x00\x10JFIF")
        assert _read_head(f) is None

    def test_caps_at_head_lines(self, tmp_path):
        """Returns at most HEAD_LINES lines."""
        f = tmp_path / "long.py"
        f.write_text("\n".join(f"line {i}" for i in range(20)))
        lines = _read_head(f)
        assert len(lines) <= 5

    def test_empty_file(self, tmp_path):
        """Empty file returns a list with one empty string."""
        f = tmp_path / "empty.py"
        f.write_text("")
        lines = _read_head(f)
        assert lines is not None
        assert lines[0] == ""

    def test_missing_file_returns_none(self, tmp_path):
        """Non-existent file returns None."""
        assert _read_head(tmp_path / "nope.py") is None

    def test_strips_carriage_return(self, tmp_path):
        """Windows line endings are stripped."""
        f = tmp_path / "win.py"
        f.write_bytes(b"# path/to/file.py\r\nline two\r\n")
        lines = _read_head(f)
        assert lines[0] == "# path/to/file.py"

    def test_strips_utf8_bom(self, tmp_path):
        """UTF-8 BOM from browser downloads is transparently removed."""
        f = tmp_path / "bom.css"
        f.write_bytes(_BOM_BYTES + b"/* extension/styles/theme.css */\n")
        lines = _read_head(f)
        assert lines is not None
        assert lines[0] == "/* extension/styles/theme.css */"

    def test_strips_bom_with_hash_comment(self, tmp_path):
        """BOM before a Python-style path comment is stripped."""
        f = tmp_path / "bom.py"
        f.write_bytes(_BOM_BYTES + b"# lambdas/handler.py\n")
        lines = _read_head(f)
        assert lines[0] == "# lambdas/handler.py"

    def test_strips_bom_with_double_slash(self, tmp_path):
        """BOM before a TypeScript-style path comment is stripped."""
        f = tmp_path / "bom.tsx"
        f.write_bytes(_BOM_BYTES + b"// extension/components/App.tsx\n")
        lines = _read_head(f)
        assert lines[0] == "// extension/components/App.tsx"

    def test_truncated_multibyte_at_boundary(self, tmp_path):
        """Multi-byte UTF-8 char split by HEAD_BYTES does not kill detection."""
        comment = b"// extension/components/search/DocumentGroup.tsx\n"
        # Pad to exactly HEAD_BYTES - 1, then add a 3-byte char (em dash)
        # so the 2048-byte slice cuts it mid-character.
        pad = b"x" * (2048 - len(comment) - 1)
        em_dash = "\u2014".encode("utf-8")  # 3 bytes: e2 80 94
        content = comment + pad + em_dash
        f = tmp_path / "trunc.tsx"
        f.write_bytes(content)
        lines = _read_head(f)
        assert lines is not None
        assert lines[0] == "// extension/components/search/DocumentGroup.tsx"

    def test_clustered_multibyte_at_boundary(self, tmp_path):
        """Multiple multi-byte chars near HEAD_BYTES boundary all handled."""
        comment = b"// extension/components/shared/Pills.tsx\n"
        em_dash = "\u2014".encode("utf-8")  # 3 bytes each
        # Place two em dashes right before the 2048 boundary so trimming
        # 3 bytes would just hit the next one (the Pills.tsx failure mode).
        pad_len = 2048 - len(comment) - 4  # land mid-second em dash
        content = comment + b"x" * pad_len + em_dash + em_dash + b"\nmore\n"
        f = tmp_path / "cluster.tsx"
        f.write_bytes(content)
        lines = _read_head(f)
        assert lines is not None
        assert lines[0] == "// extension/components/shared/Pills.tsx"

    def test_binary_file_still_returns_none(self, tmp_path):
        """Binary with null bytes is rejected before decode is attempted."""
        f = tmp_path / "image.jpg"
        f.write_bytes(b"\xff\xd8\xff\xe0\x00\x10JFIF")
        assert _read_head(f) is None

    def test_strips_nbsp(self, tmp_path):
        """NBSP (U+00A0) between // and path is stripped."""
        f = tmp_path / "nbsp.tsx"
        f.write_text("//\u00a0extension/App.tsx\n")
        lines = _read_head(f)
        assert lines[0] == "// extension/App.tsx"

    def test_strips_zero_width_space(self, tmp_path):
        """Zero-width space (U+200B) before comment is stripped."""
        f = tmp_path / "zwsp.tsx"
        f.write_text("\u200b// extension/App.tsx\n")
        lines = _read_head(f)
        assert lines[0] == "// extension/App.tsx"

    def test_strips_soft_hyphen(self, tmp_path):
        """Soft hyphen (U+00AD) in path is stripped."""
        f = tmp_path / "shy.py"
        f.write_text("# lambdas/\u00adhandler.py\n")
        lines = _read_head(f)
        assert lines[0] == "# lambdas/handler.py"


# -- _detect_comment -----------------------------------------------------------


class TestDetectComment:
    """Match directory comment patterns in the first N lines."""

    def test_python_hash(self):
        assert _detect_comment(["# lambdas/chunker.py"]) == (
            "lambdas/chunker.py",
            1,
        )

    def test_typescript_double_slash(self):
        assert _detect_comment(["// extension/App.tsx"]) == (
            "extension/App.tsx",
            1,
        )

    def test_markdown_html_comment(self):
        assert _detect_comment(["<!-- database/README.md -->"]) == (
            "database/README.md",
            1,
        )

    def test_sql_double_dash(self):
        assert _detect_comment(["-- database/schema.sql"]) == (
            "database/schema.sql",
            1,
        )

    def test_css_block_comment(self):
        assert _detect_comment(["/* extension/App.css */"]) == (
            "extension/App.css",
            1,
        )

    def test_css_nested_path(self):
        """CSS block comment with multi-level directory path."""
        assert _detect_comment(["/* extension/styles/theme.css */"]) == (
            "extension/styles/theme.css",
            1,
        )

    def test_shebang_pushes_to_line_2(self):
        lines = ["#!/usr/bin/env python3", "# lambdas/dev_server.py"]
        result = _detect_comment(lines)
        assert result == ("lambdas/dev_server.py", 2)

    def test_ts_directive_pushes_to_line_2(self):
        lines = ['/// <reference lib="dom" />', "// lib/dashboard/client.ts"]
        result = _detect_comment(lines)
        assert result == ("lib/dashboard/client.ts", 2)

    def test_no_match_returns_none(self):
        assert _detect_comment(["just some text", "no comments here"]) is None

    def test_empty_lines_returns_none(self):
        assert _detect_comment(["", "", ""]) is None

    def test_hash_with_spaces_no_match(self):
        """# followed by space-containing text is not a path."""
        assert _detect_comment(["# This is a regular comment"]) is None

    def test_hash_heading_no_match(self):
        """Markdown heading is not a directory comment."""
        assert _detect_comment(["## What"]) is None


# -- _detect_json_path ---------------------------------------------------------


class TestDetectJsonPath:
    """Extract repo path from JSON files with a top-level "path" key."""

    def test_json_with_path_field(self, tmp_path):
        f = tmp_path / "meta.json"
        f.write_text(json.dumps({"path": "benchmarks/snapshots/meta.json"}))
        assert _detect_json_path(f) == "benchmarks/snapshots/meta.json"

    def test_json_without_path_field(self, tmp_path):
        f = tmp_path / "data.json"
        f.write_text(json.dumps({"name": "test", "version": "1.0"}))
        assert _detect_json_path(f) is None

    def test_non_json_extension_skipped(self, tmp_path):
        """Files without .json suffix are not parsed."""
        f = tmp_path / "data.yaml"
        f.write_text('{"path": "should/be/ignored.yaml"}')
        assert _detect_json_path(f) is None

    def test_invalid_json_returns_none(self, tmp_path):
        f = tmp_path / "broken.json"
        f.write_text("{not valid json")
        assert _detect_json_path(f) is None

    def test_json_array_no_path(self, tmp_path):
        """Top-level array has no "path" key."""
        f = tmp_path / "list.json"
        f.write_text('[{"path": "nested/path.json"}]')
        assert _detect_json_path(f) is None

    def test_path_must_be_string(self, tmp_path):
        """Non-string "path" values are ignored."""
        f = tmp_path / "num.json"
        f.write_text(json.dumps({"path": 42}))
        assert _detect_json_path(f) is None


# -- _detect_embedded ----------------------------------------------------------


class TestDetectEmbedded:
    """Find known repo paths as substrings in the first lines."""

    def test_finds_path_in_decorated_comment(self):
        repo_map = {"benchmarks/Makefile", "extension/App.tsx"}
        lines = [
            "# -- benchmarks/Makefile -------------------------------------------------------"
        ]
        assert _detect_embedded(lines, repo_map) == "benchmarks/Makefile"

    def test_prefers_longest_match(self):
        """When multiple paths match, the longest (most specific) wins."""
        repo_map = {"src/lib/utils.py", "lib/utils.py"}
        lines = ["# something about src/lib/utils.py here"]
        assert _detect_embedded(lines, repo_map) == "src/lib/utils.py"

    def test_ignores_bare_filenames(self):
        """Paths without / are excluded to avoid false positives."""
        repo_map = {"Makefile", "README.md"}
        lines = ["This mentions Makefile and README.md"]
        assert _detect_embedded(lines, repo_map) is None

    def test_no_match(self):
        repo_map = {"extension/App.tsx"}
        lines = ["nothing relevant here"]
        assert _detect_embedded(lines, repo_map) is None

    def test_matches_on_later_line(self):
        repo_map = {"lambdas/handler.py"}
        lines = ["#!/usr/bin/env python3", "# lambdas/handler.py but decorated"]
        assert _detect_embedded(lines, repo_map) == "lambdas/handler.py"


# -- _valid_target -------------------------------------------------------------


class TestValidTarget:
    """Validate extracted paths against repo structure."""

    def test_normal_path(self, tmp_path):
        (tmp_path / "extension").mkdir()
        assert _valid_target("extension/App.tsx", tmp_path) is True

    def test_absolute_path_rejected(self, tmp_path):
        assert _valid_target("/etc/passwd", tmp_path) is False

    def test_traversal_rejected(self, tmp_path):
        assert _valid_target("extension/../../etc/passwd", tmp_path) is False

    def test_backslash_rejected(self, tmp_path):
        """Windows-style backslash paths are rejected."""
        (tmp_path / "lambdas").mkdir()
        assert _valid_target("lambdas\\handler.py", tmp_path) is False

    def test_no_extension_new_file_rejected(self, tmp_path):
        """New files (not yet in repo) without an extension are rejected."""
        (tmp_path / "src").mkdir()
        assert _valid_target("src/SomeNewComponent", tmp_path) is False

    def test_extensionless_existing_file_accepted(self, tmp_path):
        """Existing extensionless files like Makefile are accepted."""
        (tmp_path / "Makefile").touch()
        assert _valid_target("Makefile", tmp_path) is True

    def test_extensionless_existing_nested(self, tmp_path):
        """Existing extensionless file in subdirectory."""
        (tmp_path / "benchmarks").mkdir()
        (tmp_path / "benchmarks" / "Makefile").touch()
        assert _valid_target("benchmarks/Makefile", tmp_path) is True

    def test_parent_missing_rejected(self, tmp_path):
        """Path whose parent directory doesn't exist is rejected."""
        assert _valid_target("nonexistent/dir/file.py", tmp_path) is False


# -- build_repo_map -----------------------------------------------------------


class TestBuildRepoMap:
    """Walk repo tree and collect relative file paths."""

    def test_collects_files(self, tmp_path):
        (tmp_path / "src").mkdir()
        (tmp_path / "src" / "app.py").touch()
        (tmp_path / "README.md").touch()
        repo_map = build_repo_map(tmp_path)
        assert "src/app.py" in repo_map
        assert "README.md" in repo_map

    def test_skips_git_dir(self, tmp_path):
        (tmp_path / ".git").mkdir()
        (tmp_path / ".git" / "config").touch()
        (tmp_path / "app.py").touch()
        repo_map = build_repo_map(tmp_path)
        assert "app.py" in repo_map
        assert ".git/config" not in repo_map

    def test_skips_node_modules(self, tmp_path):
        (tmp_path / "node_modules").mkdir()
        (tmp_path / "node_modules" / "pkg.js").touch()
        repo_map = build_repo_map(tmp_path)
        assert "node_modules/pkg.js" not in repo_map

    def test_includes_dotfiles(self, tmp_path):
        """Dotfiles not in SKIP_DIRS are included."""
        (tmp_path / ".github").mkdir()
        (tmp_path / ".github" / "ci.yml").touch()
        repo_map = build_repo_map(tmp_path)
        assert ".github/ci.yml" in repo_map


# -- detect (full cascade) ----------------------------------------------------


class TestDetect:
    """Full detection cascade: comment > json > embedded."""

    def _make_repo(self, tmp_path):
        """Create a minimal repo structure and return (repo_root, repo_map)."""
        root = tmp_path / "repo"
        (root / "extension").mkdir(parents=True)
        (root / "extension" / "App.tsx").touch()
        (root / "extension" / "components" / "search").mkdir(parents=True)
        (root / "extension" / "styles").mkdir()
        (root / "extension" / "styles" / "theme.css").touch()
        (root / "lambdas").mkdir()
        (root / "lambdas" / "handler.py").touch()
        (root / "benchmarks").mkdir()
        (root / "benchmarks" / "Makefile").touch()
        repo_map = build_repo_map(root)
        return root, repo_map

    def test_tier1_comment_wins(self, tmp_path):
        root, repo_map = self._make_repo(tmp_path)
        f = tmp_path / "download.tsx"
        f.write_text("// extension/App.tsx\nimport React from 'react';\n")
        path, method = detect(f, repo_map, root)
        assert path == "extension/App.tsx"
        assert "comment" in method

    def test_tier2_json_fallback(self, tmp_path):
        root, repo_map = self._make_repo(tmp_path)
        f = tmp_path / "meta.json"
        f.write_text(json.dumps({"path": "extension/App.tsx", "id": "test"}))
        path, method = detect(f, repo_map, root)
        assert path == "extension/App.tsx"
        assert method == "json"

    def test_tier3_embedded_fallback(self, tmp_path):
        root, repo_map = self._make_repo(tmp_path)
        f = tmp_path / "random-name.txt"
        f.write_text("# -- benchmarks/Makefile ---------\n#\n")
        path, method = detect(f, repo_map, root)
        assert path == "benchmarks/Makefile"
        assert method == "embedded"

    def test_no_match_returns_none(self, tmp_path):
        root, repo_map = self._make_repo(tmp_path)
        f = tmp_path / "random.txt"
        f.write_text("nothing useful\n")
        path, method = detect(f, repo_map, root)
        assert path is None
        assert method is None

    def test_binary_returns_none(self, tmp_path):
        root, repo_map = self._make_repo(tmp_path)
        f = tmp_path / "image.png"
        f.write_bytes(b"\x89PNG\r\n\x1a\n\x00\x00\x00")
        path, method = detect(f, repo_map, root)
        assert path is None

    def test_invalid_comment_path_falls_to_embedded(self, tmp_path):
        """Comment with non-existent parent falls through to embedded tier."""
        root, repo_map = self._make_repo(tmp_path)
        f = tmp_path / "some-file.txt"
        f.write_text("# nonexistent/dir/file.py\n# See extension/App.tsx for details\n")
        path, method = detect(f, repo_map, root)
        assert path == "extension/App.tsx"
        assert method == "embedded"

    def test_bom_css_block_comment(self, tmp_path):
        """UTF-8 BOM does not prevent CSS block comment detection."""
        root, repo_map = self._make_repo(tmp_path)
        f = tmp_path / "theme.css"
        f.write_bytes(_BOM_BYTES + b"/* extension/styles/theme.css */\n")
        path, method = detect(f, repo_map, root)
        assert path == "extension/styles/theme.css"
        assert "comment" in method

    def test_bom_double_slash_comment(self, tmp_path):
        """UTF-8 BOM does not prevent // comment detection."""
        root, repo_map = self._make_repo(tmp_path)
        f = tmp_path / "download.tsx"
        f.write_bytes(_BOM_BYTES + b"// extension/App.tsx\n")
        path, method = detect(f, repo_map, root)
        assert path == "extension/App.tsx"
        assert "comment" in method

    def test_bom_hash_comment(self, tmp_path):
        """UTF-8 BOM does not prevent # comment detection."""
        root, repo_map = self._make_repo(tmp_path)
        f = tmp_path / "download.py"
        f.write_bytes(_BOM_BYTES + b"# lambdas/handler.py\n")
        path, method = detect(f, repo_map, root)
        assert path == "lambdas/handler.py"
        assert "comment" in method

    def test_nbsp_in_comment(self, tmp_path):
        """NBSP between // and path does not prevent detection."""
        root, repo_map = self._make_repo(tmp_path)
        f = tmp_path / "download.tsx"
        f.write_text("//\u00a0extension/App.tsx\n")
        path, method = detect(f, repo_map, root)
        assert path == "extension/App.tsx"
        assert "comment" in method

    def test_backslash_comment_rejected(self, tmp_path):
        """Backslash path in comment is detected but rejected by valid_target."""
        root, repo_map = self._make_repo(tmp_path)
        f = tmp_path / "download.py"
        f.write_text("# lambdas\\handler.py\n")
        path, method = detect(f, repo_map, root)
        assert path is None

    def test_new_file_in_existing_dir(self, tmp_path):
        """New file placed in existing directory via comment detection."""
        root, repo_map = self._make_repo(tmp_path)
        f = tmp_path / "NewComponent.tsx"
        f.write_text("// extension/components/search/NewComponent.tsx\n")
        path, method = detect(f, repo_map, root)
        assert path == "extension/components/search/NewComponent.tsx"
        assert "comment" in method

    def test_repo_map_add_after_placement(self, tmp_path):
        """repo_map.add makes newly placed files visible to embedded detection."""
        root, repo_map = self._make_repo(tmp_path)
        new_path = "extension/components/search/NewComponent.tsx"
        assert new_path not in repo_map
        repo_map.add(new_path)
        assert new_path in repo_map
