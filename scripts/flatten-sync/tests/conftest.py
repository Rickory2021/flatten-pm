# scripts/flatten-sync/tests/conftest.py
"""Add the flatten directory to sys.path so tests can import lib modules
regardless of where the flatten/ directory lives in the repo."""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
