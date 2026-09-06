# scripts/flatten-sync/lib/cascade.py
"""Cascade resolution for modifier inheritance."""


def flatten_list(items):
    """Flatten nested lists: [[a, b], c] -> [a, b, c]."""
    for item in items:
        if isinstance(item, list):
            yield from item
        else:
            yield item


def resolve_cascade(tree):
    """Walk tree and compute resolved exclude sets per node.

    Args:
        tree: dict of top_level_key -> node, where each node has:
            path: str
            modifier: dict with optional 'exclude' and 'include' lists
            children: dict of child_key -> node

    Returns:
        dict of dotted_key -> set of excluded modifier names.
        Keys use dot notation to track tree position:
          "root" for top-level, "root.extraction" for children, etc.
    """
    result = {}

    def _resolve_node(key, node, inherited):
        modifier = node.get("modifier", {})

        # Start with inherited set
        resolved = set(inherited)

        # Step 1: include removes from inherited
        includes = modifier.get("include") or []
        includes = set(flatten_list(includes))
        resolved -= includes

        # Step 2: exclude adds to set
        excludes = modifier.get("exclude") or []
        excludes = set(flatten_list(excludes))
        resolved |= excludes

        result[key] = resolved

        # Recurse into children
        for child_key, child_node in node.get("children", {}).items():
            _resolve_node(f"{key}.{child_key}", child_node, resolved)

    for top_key, top_node in tree.items():
        _resolve_node(top_key, top_node, set())

    return result
