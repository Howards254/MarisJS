#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CANONICAL="$ROOT/docs/framework-grammar-spec.md"
COMMITTED="$ROOT/packages/marisjs/SPEC.md"

if ! diff -q "$CANONICAL" "$COMMITTED" > /dev/null 2>&1; then
  echo "ERROR: packages/marisjs/SPEC.md differs from docs/framework-grammar-spec.md"
  echo ""
  echo "packages/marisjs/SPEC.md is regenerated during prepack (copied from docs/framework-grammar-spec.md),"
  echo "but the committed copy is out of date. Run this to sync:"
  echo ""
  echo "  cp docs/framework-grammar-spec.md packages/marisjs/SPEC.md"
  echo ""
  echo "Diff:"
  diff "$CANONICAL" "$COMMITTED" || true
  exit 1
fi

echo "SPEC.md is in sync"
