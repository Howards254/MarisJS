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

# Pre-flight: check whether this exact version is already on the registry
# BEFORE publishing. npm's "already published" error (EPUBLISHCONFLICT) is
# unambiguous, but a bare `code E403` can also mean a genuine permission
# failure — never treat a generic E403 as "already exists, skip".
# (A real 403 was once silently swallowed this way, leaving a platform
# package unpublished.)
ALREADY=$(npm view "$NAME@$VERSION" version --silent 2>/dev/null || true)

if [ -n "$ALREADY" ]; then
  echo "  ⚠ $NAME@$VERSION already exists — skipping"
  exit 0
fi

OUTPUT=$(npm publish --access public $DRY_FLAG 2>&1) && EXIT=0 || EXIT=$?

if [ $EXIT -eq 0 ]; then
  echo "  ✓ published"
  exit 0
fi

# Only skip-if-already-exists for real publishes (dry-run doesn't touch registry).
# A publish can also race with a concurrent one — treat EPUBLISHCONFLICT as skip.
if [ -z "$DRY_FLAG" ]; then
  if echo "$OUTPUT" | grep -qiE "previously published|cannot publish over|already exists|code EPUBLISHCONFLICT"; then
    echo "  ⚠ $NAME@$VERSION already exists — skipping"
    exit 0
  fi
fi

echo "  ✗ PUBLISH FAILED"
echo "$OUTPUT" >&2
exit 1
