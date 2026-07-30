#!/usr/bin/env bash
# ── Size-budget check ─────────────────────────────────────────────────
# Protects the project's core efficiency claim with automated CI gates:
#
#   Runtime hard ceiling
#     @maris/runtime source must stay under 10KB unminified (currently
#     ~2.8KB). This is an absolute cap — no growth-percentage tolerance.
#
#   Binary growth limit
#     Each platform binary is compared against its last-measured
#     baseline in scripts/size-baseline.json. CI fails if any binary
#     grows more than 25% beyond its baseline in a single change.
#     Intentional growth requires updating the baseline file.
#
# Usage:
#   scripts/check-sizes.sh              check both runtime and binaries
#   scripts/check-sizes.sh --runtime    check runtime only
#   scripts/check-sizes.sh --binary <path> <target>   check one binary
# ──────────────────────────────────────────────────────────────────────

set -euo pipefail
cd "$(dirname "$0")/.."

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

BASELINE_FILE="scripts/size-baseline.json"
RUNTIME_FILE="packages/runtime/src/index.js"

EXIT=0
CHECK_RUNTIME=true
CHECK_BINARIES=true
BINARY_PATH=""
BINARY_TARGET=""

# ── Parse args ───────────────────────────────────────────────────────

while [ $# -gt 0 ]; do
  case "$1" in
    --runtime) CHECK_BINARIES=false; shift ;;
    --binary)
      CHECK_RUNTIME=false
      BINARY_PATH="$2"
      BINARY_TARGET="$3"
      shift 3
      ;;
    *) echo "Usage: $0 [--runtime] [--binary <path> <target>]"; exit 1 ;;
  esac
done

# ── Helpers ──────────────────────────────────────────────────────────

pass() { echo -e "  ${GREEN}PASS${NC}  $*"; }
fail() { echo -e "  ${RED}FAIL${NC}  $*"; EXIT=1; }

read_json_field() {
  # Reads a dotted path like "binaries.x86_64-unknown-linux-gnu" from baseline.
  # Uses python3 (available on all CI runners) to parse JSON without deps.
  local path="$1"
  python3 -c "
import json, sys
try:
    data = json.load(open('$BASELINE_FILE'))
    parts = '$path'.split('.')
    val = data
    for p in parts:
        val = val.get(p) if isinstance(val, dict) else None
    sys.stdout.write(str(val) if val is not None else '')
except:
    pass
" 2>/dev/null
}

pct_change() {
  local old="$1" new="$2"
  if [ "$old" -eq 0 ] 2>/dev/null; then echo "N/A"; return; fi
  awk "BEGIN { printf \"%.0f\", (($new - $old) / $old) * 100 }"
}

check_one_binary() {
  local bin="$1" target="$2"
  local bytes
  bytes=$(stat -c%s "$bin" 2>/dev/null || stat -f%z "$bin" 2>/dev/null)

  local baseline
  baseline=$(read_json_field "binaries.$target")

  if [ -z "$baseline" ] || [ "$baseline" = "None" ] || [ "$baseline" = "null" ]; then
    echo -e "  ${YELLOW}NEW${NC}   $target: ${bytes}B"
    echo "         baseline entry missing — add this to scripts/size-baseline.json:"
    echo "         \"$target\": $bytes"
    return
  fi

  local max_pct max_bytes pct
  max_pct=$(read_json_field "binaries.growth_max_percent")
  max_pct=${max_pct:-25}
  max_bytes=$(awk "BEGIN { printf \"%.0f\", $baseline * (1 + $max_pct / 100) }")
  pct=$(pct_change "$baseline" "$bytes")

  if [ "$bytes" -gt "$max_bytes" ]; then
    fail "$target: ${bytes}B (+${pct}%) exceeds max ${max_bytes}B (baseline ${baseline}B +${max_pct}%)"
  else
    pass "$target: ${bytes}B (+${pct}% of ${baseline}B baseline, limit +${max_pct}%)"
  fi
}

# ── Basline validation ──────────────────────────────────────────────

if [ ! -f "$BASELINE_FILE" ]; then
  echo "ERROR: baseline file not found: $BASELINE_FILE"
  exit 1
fi

# ── Check 1: Runtime hard ceiling ───────────────────────────────────

if $CHECK_RUNTIME; then
  echo ""
  echo "── @maris/runtime size check ────────────────────────────────────"

  if [ ! -f "$RUNTIME_FILE" ]; then
    fail "runtime source not found: $RUNTIME_FILE"
  else
    RUNTIME_BYTES=$(wc -c < "$RUNTIME_FILE")
    RUNTIME_MAX=$(read_json_field "runtime.max_bytes")

    if [ -z "$RUNTIME_MAX" ]; then
      fail "baseline missing runtime.max_bytes"
    elif [ "$RUNTIME_BYTES" -gt "$RUNTIME_MAX" ]; then
      fail "runtime source is ${RUNTIME_BYTES}B (ceiling: ${RUNTIME_MAX}B = ${RUNTIME_MAX%% *} bytes)"
    else
      pass "runtime source is ${RUNTIME_BYTES}B (ceiling: ${RUNTIME_MAX}B)"
    fi
  fi
fi

# ── Check 2: Binary growth limit ────────────────────────────────────

if $CHECK_BINARIES; then
  echo ""
  echo "── CLI binary size check ────────────────────────────────────────"

  # If specific binary provided, check just that one
  if [ -n "$BINARY_PATH" ]; then
    if [ ! -f "$BINARY_PATH" ]; then
      fail "binary not found: $BINARY_PATH"
    else
      check_one_binary "$BINARY_PATH" "$BINARY_TARGET"
    fi
  else
    # Auto-detect: check the currently built native binary
    NATIVE_BIN="target/release/marisjs"
    NATIVE_TARGET=$(rustc -vV | grep ^host: | awk '{print $2}')
    if [ -f "$NATIVE_BIN" ]; then
      check_one_binary "$NATIVE_BIN" "$NATIVE_TARGET"
    else
      echo "  (no native binary found at $NATIVE_BIN — build with 'cargo build --release' first)"
    fi
  fi
fi

# ── Final verdict ───────────────────────────────────────────────────

echo ""
if [ "$EXIT" -eq 0 ]; then
  echo -e "${GREEN}Size budget: all checks passed${NC}"
else
  echo -e "${RED}Size budget: BUDGET EXCEEDED${NC}"
  echo ""
  echo "If the size growth is intentional, update $BASELINE_FILE and re-run."
fi
exit $EXIT
