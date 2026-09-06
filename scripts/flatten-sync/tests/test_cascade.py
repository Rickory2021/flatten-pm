# scripts/flatten-sync/tests/test_cascade.py
"""Tests for modifier cascade resolution.

Pure function tests -- no filesystem, no YAML loading.
Validates the inherit -> include -> exclude resolution logic.
"""

from lib.cascade import flatten_list, resolve_cascade


# -- flatten_list --------------------------------------------------------------


class TestFlattenList:
    """Flatten nested YAML lists into a single list of modifier names."""

    def test_flat_input_unchanged(self):
        assert list(flatten_list(["a", "b", "c"])) == ["a", "b", "c"]

    def test_nested_lists_flattened(self):
        assert list(flatten_list([["a", "b"], "c"])) == ["a", "b", "c"]

    def test_multiple_nested_lists(self):
        assert list(flatten_list([["a", "b"], ["c", "d"]])) == ["a", "b", "c", "d"]

    def test_mixed_nested_and_flat(self):
        assert list(flatten_list(["x", ["a", "b"], "y"])) == ["x", "a", "b", "y"]

    def test_empty_list(self):
        assert list(flatten_list([])) == []

    def test_single_item(self):
        assert list(flatten_list(["a"])) == ["a"]

    def test_single_nested_list(self):
        assert list(flatten_list([["a", "b"]])) == ["a", "b"]

    def test_empty_nested_list(self):
        assert list(flatten_list([[], "a"])) == ["a"]


# -- resolve_cascade -----------------------------------------------------------

# Helper to build node dicts matching the expected parse_profile output.
# Each node has:
#   path: str (resolved absolute-ish path, not used by cascade)
#   modifier: dict with optional 'exclude' and 'include' lists
#   children: dict of child_key -> node


def _node(path=".", modifier=None, children=None):
    return {
        "path": path,
        "modifier": modifier or {},
        "children": children or {},
    }


class TestResolveCascade:
    """Resolve modifier cascade: inherit -> include -> exclude through tree."""

    def test_no_modifiers_empty_set(self):
        """Single node with no modifiers produces empty exclude set."""
        tree = {"root": _node(".")}
        resolved = resolve_cascade(tree)
        assert resolved["root"] == set()

    def test_parent_exclude_inherited_by_child(self):
        """Parent excludes [tests], child inherits it."""
        tree = {
            "root": _node(
                "benchmarks",
                modifier={"exclude": ["tests"]},
                children={
                    "extraction": _node("benchmarks/extraction"),
                },
            ),
        }
        resolved = resolve_cascade(tree)
        assert resolved["root"] == {"tests"}
        assert resolved["root.extraction"] == {"tests"}

    def test_child_include_overrides_parent_exclude(self):
        """Parent excludes [tests], child includes [tests] -> child has empty set."""
        tree = {
            "root": _node(
                "benchmarks",
                modifier={"exclude": ["tests"]},
                children={
                    "extraction": _node(
                        "benchmarks/extraction",
                        modifier={"include": ["tests"]},
                    ),
                },
            ),
        }
        resolved = resolve_cascade(tree)
        assert resolved["root"] == {"tests"}
        assert resolved["root.extraction"] == set()

    def test_child_adds_own_excludes(self):
        """Parent excludes [tests], child excludes [corpus] -> child has both."""
        tree = {
            "root": _node(
                "benchmarks",
                modifier={"exclude": ["tests"]},
                children={
                    "extraction": _node(
                        "benchmarks/extraction",
                        modifier={"exclude": ["corpus"]},
                    ),
                },
            ),
        }
        resolved = resolve_cascade(tree)
        assert resolved["root"] == {"tests"}
        assert resolved["root.extraction"] == {"tests", "corpus"}

    def test_three_level_nesting(self):
        """Cascade through 3 levels: grandchild inherits from root and parent."""
        tree = {
            "root": _node(
                ".",
                modifier={"exclude": ["tests"]},
                children={
                    "benchmarks": _node(
                        "benchmarks",
                        modifier={"exclude": ["corpus"]},
                        children={
                            "extraction": _node(
                                "benchmarks/extraction",
                                modifier={"exclude": ["docs"]},
                            ),
                        },
                    ),
                },
            ),
        }
        resolved = resolve_cascade(tree)
        assert resolved["root"] == {"tests"}
        assert resolved["root.benchmarks"] == {"tests", "corpus"}
        assert resolved["root.benchmarks.extraction"] == {"tests", "corpus", "docs"}

    def test_nested_list_flattening_in_exclude(self):
        """Nested lists in exclude are flattened before processing."""
        tree = {
            "root": _node(
                ".",
                modifier={"exclude": [["a", "b"], "c"]},
            ),
        }
        resolved = resolve_cascade(tree)
        assert resolved["root"] == {"a", "b", "c"}

    def test_nested_list_flattening_in_include(self):
        """Nested lists in include are flattened before processing."""
        tree = {
            "root": _node(
                ".",
                modifier={"exclude": ["a", "b", "c"]},
                children={
                    "child": _node(
                        "child",
                        modifier={"include": [["a", "b"]]},
                    ),
                },
            ),
        }
        resolved = resolve_cascade(tree)
        assert resolved["root.child"] == {"c"}

    def test_null_modifier_no_crash(self):
        """Null modifier (all items commented out in YAML) treated as empty."""
        tree = {
            "root": _node(
                ".",
                modifier={"exclude": None, "include": None},
            ),
        }
        resolved = resolve_cascade(tree)
        assert resolved["root"] == set()

    def test_empty_modifier_inherits_parent(self):
        """Empty modifier block inherits parent unchanged."""
        tree = {
            "root": _node(
                ".",
                modifier={"exclude": ["tests"]},
                children={
                    "child": _node("child", modifier={}),
                },
            ),
        }
        resolved = resolve_cascade(tree)
        assert resolved["root.child"] == {"tests"}

    def test_include_then_exclude_same_name(self):
        """Include runs first, exclude runs second. Same name -> net excluded."""
        tree = {
            "root": _node(
                ".",
                modifier={"exclude": ["tests"]},
                children={
                    "child": _node(
                        "child",
                        modifier={
                            "include": ["tests"],
                            "exclude": ["tests"],
                        },
                    ),
                },
            ),
        }
        resolved = resolve_cascade(tree)
        # include removes tests, then exclude adds it back -> excluded
        assert resolved["root.child"] == {"tests"}

    def test_multiple_top_level_nodes(self):
        """Multiple root-level nodes resolve independently."""
        tree = {
            "ext": _node("extension", modifier={"exclude": ["assets"]}),
            "lambdas": _node("lambdas", modifier={"exclude": ["tests"]}),
        }
        resolved = resolve_cascade(tree)
        assert resolved["ext"] == {"assets"}
        assert resolved["lambdas"] == {"tests"}

    def test_child_include_only(self):
        """Child with include-only removes from inherited set."""
        tree = {
            "root": _node(
                "benchmarks",
                modifier={"exclude": ["tests", "corpus"]},
                children={
                    "extraction": _node(
                        "benchmarks/extraction",
                        modifier={"include": ["tests"]},
                    ),
                },
            ),
        }
        resolved = resolve_cascade(tree)
        assert resolved["root.extraction"] == {"corpus"}

    def test_no_modifier_key_at_all(self):
        """Node with no modifier key at all inherits parent."""
        tree = {
            "root": _node(
                ".",
                modifier={"exclude": ["tests"]},
                children={
                    "child": _node("child"),  # no modifier
                },
            ),
        }
        resolved = resolve_cascade(tree)
        assert resolved["root.child"] == {"tests"}
