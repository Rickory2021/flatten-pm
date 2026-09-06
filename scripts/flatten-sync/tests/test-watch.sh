# scripts/flatten-sync/tests/test-watch.sh
#
# Integration test for watch.py detection cascade.
# Run from repo root: bash scripts/flatten-sync/tests/test-watch.sh
#
# Creates a fake repo tree and source directory in /tmp. The watcher runs
# against the fake repo so nothing in the real repo is touched. Uses
# cleanup mode "move" so processed files can be inspected afterward.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROFILES_DIR="$SCRIPT_DIR/profiles"
TEST_ROOT="/tmp/watch-test-$$"
FAKE_REPO="$TEST_ROOT/repo"
SOURCE="$TEST_ROOT/source"
PROCESSED="$TEST_ROOT/processed"
WATCHER_PID=""

# ── Cleanup on exit ──────────────────────────────────────────────────────────
cleanup() {
  if [ -n "$WATCHER_PID" ] && kill -0 "$WATCHER_PID" 2>/dev/null; then
    kill "$WATCHER_PID" 2>/dev/null
    wait "$WATCHER_PID" 2>/dev/null || true
  fi
  rm -f "$PROFILES_DIR/_test.watch.config.yaml"
  echo ""
  echo "Test artifacts at: $TEST_ROOT"
  echo "  Fake repo:   $FAKE_REPO"
  echo "  Processed:   $PROCESSED"
  echo "  Source:      $SOURCE"
  echo "  Clean up with: rm -rf $TEST_ROOT"
}
trap cleanup EXIT

# ── Build fake repo tree ─────────────────────────────────────────────────────
mkdir -p "$SOURCE" "$PROCESSED"

# Directories the watcher needs for path validation (parent must exist)
mkdir -p "$FAKE_REPO/lambdas/src/shared_layer/shared"
mkdir -p "$FAKE_REPO/extension/components/search"
mkdir -p "$FAKE_REPO/extension/styles"
mkdir -p "$FAKE_REPO/extension"
mkdir -p "$FAKE_REPO/benchmarks"

# Existing files - watcher overwrites these with test content
echo "original" > "$FAKE_REPO/lambdas/src/shared_layer/shared/chunker.py"
echo "original" > "$FAKE_REPO/lambdas/dev_server.py"
echo "original" > "$FAKE_REPO/extension/package.json"
echo "original" > "$FAKE_REPO/benchmarks/Makefile"
echo "original" > "$FAKE_REPO/extension/styles/theme.css"
echo "original" > "$FAKE_REPO/extension/styles/tokens.css"

# ── Write test profile ───────────────────────────────────────────────────────
cat > "$PROFILES_DIR/_test.watch.config.yaml" << EOF
source:
  directory: $SOURCE
target:
  repo: $FAKE_REPO
cleanup:
  mode: move
  move_to: $PROCESSED
polling:
  interval: 0.5
  stable_wait: 0.2
  stable_retries: 4
  repo_map_refresh: 30
EOF

echo "Fake repo:  $FAKE_REPO"
echo "Source:     $SOURCE"
echo "Processed:  $PROCESSED"
echo ""

# ── Start watcher in background ──────────────────────────────────────────────
uv run "$SCRIPT_DIR/watch.py" --profile _test -v &
WATCHER_PID=$!
sleep 3  # let watcher start, index fake repo, take initial snapshot

echo ""
echo "══════════════════════════════════════════════════════"
echo "  Dropping test files..."
echo "══════════════════════════════════════════════════════"
echo ""

# ── Tier 1: Comment on line 1 ────────────────────────────────────────────────
printf '# lambdas/src/shared_layer/shared/chunker.py\n"""test content tier1-L1"""\n' \
  > "$SOURCE/chunker.py"
sleep 1

# ── Tier 1: Comment on line 2 (shebang pushes to L2) ────────────────────────
printf '#!/usr/bin/env python3\n# lambdas/dev_server.py\n"""test tier1-L2"""\n' \
  > "$SOURCE/dev_server.py"
sleep 1

# ── Tier 1: CSS block comment ───────────────────────────────────────────────
printf '/* extension/styles/theme.css */\n/* Blue Steel theme */\n:root { color: red; }\n' \
  > "$SOURCE/theme.css"
sleep 1

# ── Tier 1: New file in existing dir (not overwriting) ──────────────────────
# extension/components/search/ exists but DocNew.tsx does not.
# Validates _valid_target's parent.is_dir() path for new files.
printf '// extension/components/search/DocNew.tsx\nimport React from "react";\n' \
  > "$SOURCE/DocNew.tsx"
sleep 1

# ── Tier 1: Multi-byte content past HEAD_BYTES boundary ─────────────────────
# The real bug: em dashes in JSDoc push past 2048 bytes. HEAD_BYTES
# truncation splits a multi-byte char, UnicodeDecodeError kills _read_head.
# python3 generates a file where byte 2047 starts a 3-byte em dash.
python3 -c "
import sys
comment = b'// extension/components/search/DocMultibyte.tsx\n'
jsdoc = b'/**\n * Document group component.\n */\n'
header = comment + jsdoc
# Pad with newline-separated lines to reach byte 2047
pad_target = 2047 - len(header)
lines = []
while sum(len(l) for l in lines) < pad_target - 50:
    lines.append(b'const x%d = true;\n' % len(lines))
remaining = pad_target - sum(len(l) for l in lines)
lines.append(b'z' * remaining)
# em dash at byte 2047: \\xe2\\x80\\x94 straddles the 2048 boundary
content = header + b''.join(lines) + b'\xe2\x80\x94 rest of file\n'
assert len(header + b''.join(lines)) == 2047, f'pad missed: {len(header + b\"\".join(lines))}'
sys.stdout.buffer.write(content)
" > "$SOURCE/DocMultibyte.tsx"
sleep 1

# ── Tier 1: BOM-prefixed file ──────────────────────────────────────────────
# Some editors / browser downloads prepend a UTF-8 BOM (EF BB BF).
printf '\xef\xbb\xbf/* extension/styles/tokens.css */\n/* BOM test */\n:root { color: blue; }\n' \
  > "$SOURCE/tokens-bom.css"
sleep 1

# ── Tier 2: JSON path field ─────────────────────────────────────────────────
printf '{"path": "extension/package.json", "name": "test-tier2"}\n' \
  > "$SOURCE/some-meta.json"
sleep 1

# ── Tier 3: Embedded path in decorated comment ──────────────────────────────
printf '# -- benchmarks/Makefile -------------------------------------------------------\n#\n' \
  > "$SOURCE/some-makefile"
sleep 1

# ── Should skip: binary ─────────────────────────────────────────────────────
dd if=/dev/urandom of="$SOURCE/photo.jpg" bs=256 count=1 2>/dev/null
sleep 1

# ── Should skip: no match ───────────────────────────────────────────────────
printf 'just some random text\n' > "$SOURCE/notes.txt"
sleep 1

# ── Should skip: crdownload ─────────────────────────────────────────────────
printf 'partial chrome download\n' > "$SOURCE/thing.crdownload"
sleep 1

# ── Wait for processing ─────────────────────────────────────────────────────
sleep 2

# ── Stop watcher before checking ─────────────────────────────────────────────
kill "$WATCHER_PID" 2>/dev/null
wait "$WATCHER_PID" 2>/dev/null || true
WATCHER_PID=""

echo ""
echo "══════════════════════════════════════════════════════"
echo "  Results"
echo "══════════════════════════════════════════════════════"
echo ""

PASS=0
FAIL=0

check() {
  local label="$1" file="$2" expected="$3"
  if [ -f "$file" ] && grep -q "$expected" "$file" 2>/dev/null; then
    echo "  ✓  $label"
    PASS=$((PASS + 1))
  else
    echo "  ✗  $label"
    [ ! -f "$file" ] && echo "     file missing: $file"
    [ -f "$file" ] && echo "     content: $(head -1 "$file" 2>/dev/null)"
    FAIL=$((FAIL + 1))
  fi
}

check_source_remains() {
  local label="$1" file="$2"
  if [ -f "$file" ]; then
    echo "  ✓  $label"
    PASS=$((PASS + 1))
  else
    echo "  ✗  $label (file was consumed but should have been ignored)"
    FAIL=$((FAIL + 1))
  fi
}

# Placed files should have test content in the fake repo
check "Tier 1 comment L1" \
  "$FAKE_REPO/lambdas/src/shared_layer/shared/chunker.py" "test content tier1-L1"

check "Tier 1 comment L2 (shebang)" \
  "$FAKE_REPO/lambdas/dev_server.py" "test tier1-L2"

check "Tier 1 CSS block comment" \
  "$FAKE_REPO/extension/styles/theme.css" "Blue Steel theme"

check "Tier 1 new file in existing dir" \
  "$FAKE_REPO/extension/components/search/DocNew.tsx" "import React"

check "Tier 1 multi-byte truncation" \
  "$FAKE_REPO/extension/components/search/DocMultibyte.tsx" "Document group"

check "Tier 1 BOM-prefixed file" \
  "$FAKE_REPO/extension/styles/tokens.css" "BOM test"

check "Tier 2 JSON path" \
  "$FAKE_REPO/extension/package.json" "test-tier2"

check "Tier 3 embedded path" \
  "$FAKE_REPO/benchmarks/Makefile" "benchmarks/Makefile"

# Consumed files should be in processed dir (day subfolder, timestamp prefix)
echo ""
echo "  Processed dir (moved from source after placement):"
if [ -d "$PROCESSED" ] && [ "$(find "$PROCESSED" -type f 2>/dev/null)" ]; then
  find "$PROCESSED" -type f | sort | while read -r f; do
    echo "    ${f#$PROCESSED/}"
  done
else
  echo "    (empty - no files were moved)"
fi

# Skipped files should remain in source untouched
echo ""
check_source_remains "Skip: binary (photo.jpg stays in source)" "$SOURCE/photo.jpg"
check_source_remains "Skip: no match (notes.txt stays in source)" "$SOURCE/notes.txt"
check_source_remains "Skip: crdownload stays in source" "$SOURCE/thing.crdownload"

echo ""
echo "══════════════════════════════════════════════════════"
echo "  $PASS passed, $FAIL failed"
echo "══════════════════════════════════════════════════════"

exit "$FAIL"
