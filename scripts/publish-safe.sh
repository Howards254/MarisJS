#!/usr/bin/env bash
set -euo pipefail
# ── marisjs safe publish helper ────────────────────────────────────────
# Wraps `npm publish` with idempotency: if the exact version already
# exists on the registry, logs a warning and succeeds (skip).  Any other
# publish failure is fatal.  Called from CI; not intended for direct use.
# ───────────────────────────────────────────────────────────────────────

PKG_DIR="$1"
DRY_FLAG="${2:-}"

cd "$PKG_DIR"

NAME=$(node -e "console.log(require('./package.json').name)")
VERSION=$(node -e "console.log(require('./package.json').version)")

echo "=== $NAME@$VERSION ==="

OUTPUT=$(npm publish --access public $DRY_FLAG 2>&1) && EXIT=0 || EXIT=$?

if [ $EXIT -eq 0 ]; then
  echo "  ✓ published"
  exit 0
fi

# Only skip-if-already-exists for real publishes (dry-run doesn't touch registry)
if [ -z "$DRY_FLAG" ]; then
  if echo "$OUTPUT" | grep -qiE "previously published|cannot publish over|already exists|code E403|code EPUBLISHCONFLICT"; then
    echo "  ⚠ $NAME@$VERSION already exists — skipping"
    exit 0
  fi
fi

echo "  ✗ PUBLISH FAILED"
echo "$OUTPUT" >&2
exit 1
