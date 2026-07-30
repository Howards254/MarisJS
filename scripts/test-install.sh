#!/usr/bin/env bash
# ── marisjs install test ──────────────────────────────────────────────
# Simulates a fresh `npm install marisjs` on the current platform:
#  1. Builds the Rust binary (if not already built)
#  2. Copies it into the matching platform package
#  3. npm packs both the main package and platform package
#  4. Installs both tarballs into a clean temp project (single command)
#  5. Runs `npx marisjs --version` and asserts the output
#
# Both tarballs are installed in one `npm install` invocation because
# npm's Arborist deduplication can choke when an optionalDependency
# placeholder node (created by installing the main tarball first) exists
# alongside a subsequent tarball install of the same package — a
# legitimate npm edge case that doesn't apply in real registry installs
# (where the platform package IS on the registry).
#
# This catches the #1 failure mode for the esbuild/SWC pattern:
# a wrapper script that resolves the binary path at the wrong level
# (e.g. using __dirname relative to the wrong node_modules layout).
# A unit test cannot surface this — only a real npm install can.
# ──────────────────────────────────────────────────────────────────────

set -euo pipefail
cd "$(dirname "$0")/.."

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

say()  { echo -e "$*"; }
pass() { echo -e "${GREEN}PASS${NC} $*"; }
fail() { echo -e "${RED}FAIL${NC} $*"; exit 1; }

# ── 1. Detect current platform ───────────────────────────────────────

OS=$(node -e "console.log(process.platform)" 2>/dev/null) || fail "node not found"
ARCH=$(node -e "console.log(process.arch)" 2>/dev/null)

case "$OS-$ARCH" in
  linux-x64)    PLATFORM_PKG="marisjs-linux-x64" ;;
  linux-arm64)  PLATFORM_PKG="marisjs-linux-arm64" ;;
  darwin-x64)   PLATFORM_PKG="marisjs-darwin-x64" ;;
  darwin-arm64) PLATFORM_PKG="marisjs-darwin-arm64" ;;
  win32-x64)    PLATFORM_PKG="marisjs-win32-x64" ;;
  *)            fail "unsupported platform: $OS $ARCH" ;;
esac

say "platform: $OS / $ARCH → $PLATFORM_PKG"

# ── 2. Build and place binary ────────────────────────────────────────

BINARY="bin/marisjs"
[ "$OS" = "win32" ] && BINARY="bin/marisjs.exe"

if [ ! -f "packages/$PLATFORM_PKG/$BINARY" ]; then
  say "building Rust binary…"
  cargo build --release -p cli
  mkdir -p "packages/$PLATFORM_PKG/bin"
  cp "target/release/marisjs" "packages/$PLATFORM_PKG/$BINARY"
  chmod +x "packages/$PLATFORM_PKG/$BINARY"
fi

say "binary size: $(wc -c < "packages/$PLATFORM_PKG/$BINARY") bytes"

# ── 3. npm pack both packages ────────────────────────────────────────

TEST_DIR=$(mktemp -d)
trap "rm -rf '$TEST_DIR'" EXIT

say "packing main package…"
MAIN_TGZ=$(cd packages/marisjs && npm pack --pack-destination "$TEST_DIR" 2>/dev/null | tail -1)
say "packing platform package…"
PLAT_TGZ=$(cd "packages/$PLATFORM_PKG" && npm pack --pack-destination "$TEST_DIR" 2>/dev/null | tail -1)

say "main tarball:    $MAIN_TGZ"
say "platform tarball: $PLAT_TGZ"

# ── 4. Install into a clean temp project (single command) ────────────

INSTALL_DIR="$TEST_DIR/project"
mkdir -p "$INSTALL_DIR"

(cd "$INSTALL_DIR" && npm init -y >/dev/null 2>&1)

# Install both tarballs at once to avoid Arborist deduplication
# issues with the optionalDependency placeholder from the main package.
OUTPUT=$(cd "$INSTALL_DIR" && npm install "../$MAIN_TGZ" "../$PLAT_TGZ" 2>&1)
if [ $? -ne 0 ]; then
  fail "npm install failed: $OUTPUT"
fi

say "installed: $(echo "$OUTPUT" | grep 'added')"

# ── 5. Run marisjs --version ─────────────────────────────────────────

say "running marisjs --version from installed location…"

OUTPUT=$(cd "$INSTALL_DIR" && npx --no-install marisjs --version 2>&1)
EXIT_CODE=$?

if [ $EXIT_CODE -ne 0 ]; then
  fail "npx marisjs --version exited with code $EXIT_CODE: $OUTPUT"
fi

EXPECTED="marisjs 0.1.0"
if [ "$OUTPUT" = "$EXPECTED" ]; then
  pass "version output matches: '$OUTPUT'"
else
  fail "expected '$EXPECTED', got '$OUTPUT'"
fi

say ""
pass "All checks passed — marisjs installs and runs correctly on $OS $ARCH"
