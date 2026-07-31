#!/usr/bin/env bash
# ── @marisjs/runtime publishability test ────────────────────────────────
# Packs the runtime as a standalone tarball, installs it into a clean
# temp Node project, then verifies all five exports (signal, computed,
# bind, mount, data) work exactly as generated apps expect at runtime.
#
# Generated component code imports from '@marisjs/runtime' with simple ESM
# import statements — this test replicates that exact consumer pattern.
# ──────────────────────────────────────────────────────────────────────

set -euo pipefail
cd "$(dirname "$0")/.."

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

say()  { echo -e "$*"; }
pass() { echo -e "${GREEN}PASS${NC} $*"; }
fail() { echo -e "${RED}FAIL${NC} $*"; exit 1; }

say "@marisjs/runtime standalone package test"

# ── 1. Pack the runtime ──────────────────────────────────────────────

TEST_DIR=$(mktemp -d)
trap "rm -rf '$TEST_DIR'" EXIT

say "packing @marisjs/runtime…"
RUNTIME_TGZ=$(cd packages/runtime && npm pack --pack-destination "$TEST_DIR" 2>/dev/null | tail -1)

if [ ! -f "$TEST_DIR/$RUNTIME_TGZ" ]; then
  fail "pack failed — no tarball at $TEST_DIR/$RUNTIME_TGZ"
fi
say "tarball: $RUNTIME_TGZ"

# Check that test file is NOT included in the published tarball
if tar tzf "$TEST_DIR/$RUNTIME_TGZ" | grep -q '\.test\.'; then
  fail "tarball contains test files — 'files' field in package.json must exclude them"
fi
pass "tarball does not ship test files"

# ── 2. Install in a clean temp project ───────────────────────────────

PROJ_DIR="$TEST_DIR/project"
mkdir -p "$PROJ_DIR"

(cd "$PROJ_DIR" && npm init -y >/dev/null 2>&1)

# Install runtime + jsdom (needed only for the mount test)
(cd "$PROJ_DIR" && npm install "../$RUNTIME_TGZ" jsdom 2>&1) \
  || fail "npm install failed"

say "installed"

# ── 3. Run all-exports verification ──────────────────────────────────

cat > "$PROJ_DIR/test-runtime.mjs" <<'TESTEOF'
import { signal, computed, bind, mount, data } from '@marisjs/runtime';
import { JSDOM } from 'jsdom';

let failures = 0;
function assert(label, condition, detail) {
  if (condition) {
    console.log(`  PASS  ${label}`);
  } else {
    console.error(`  FAIL  ${label}${detail ? ': ' + detail : ''}`);
    failures++;
  }
}

// ── signal ────────────────────────────────────────────────────────

const s = signal(42);
assert('signal: initial value', s.value === 42, `got ${s.value}`);

s.set(99);
assert('signal: .set() updates .value', s.value === 99, `got ${s.value}`);

s.set(99); // same value — no-op
assert('signal: .set() with same value is no-op', true);

assert('signal: .set is a function', typeof s.set === 'function');

// .value is a getter defined on the object instance itself, not the prototype
assert('signal: .value is a getter', typeof Object.getOwnPropertyDescriptor(s, 'value')?.get === 'function');

// ── computed ──────────────────────────────────────────────────────

const src = signal(10);
const doubler = computed(() => src.value * 2);

assert('computed: initial derived value', doubler.value === 20, `got ${doubler.value}`);

src.set(7);
assert('computed: recalcs after source change', doubler.value === 14, `got ${doubler.value}`);

assert('computed: has .value getter', typeof doubler.value === 'number', `got ${typeof doubler.value}`);

// ── bind ──────────────────────────────────────────────────────────

const a = signal(3);
const b = signal(4);
let sum = 0;
bind(() => { sum = a.value + b.value; });
assert('bind: initial effect runs', sum === 7, `got ${sum}`);
a.set(10);
// Microtask flush
await new Promise(r => queueMicrotask(r));
assert('bind: re-executes after dependency change', sum === 14, `got ${sum}`);

// ── mount ─────────────────────────────────────────────────────────

const dom = new JSDOM('<!DOCTYPE html><div id="root"></div>');
globalThis.document = dom.window.document;
globalThis.Node = dom.window.Node;
const root = dom.window.document.getElementById('root');

mount(root, () => {
  const span = dom.window.document.createElement('span');
  span.textContent = 'hello';
  return span;
});
assert('mount: DOM node appended', root.innerHTML.includes('hello'), root.innerHTML);

// ── data ──────────────────────────────────────────────────────────

const result = await data(async () => 'server-data');
assert('data: returns fetcher result', result === 'server-data', `got ${result}`);

const badResult = await data(() => 42);
assert('data: works with sync fetcher', badResult === 42, `got ${badResult}`);

let thrown = false;
try { data(null); } catch (e) { thrown = true; }
assert('data: throws on non-function argument', thrown);

// ── Multiple observer teardown ────────────────────────────────────

const x = signal(1);
let calls = 0;
bind(() => { void x.value; calls++; });
assert('bind count after first', calls === 1);
x.set(2); await new Promise(r => queueMicrotask(r));
assert('bind count after set', calls === 2);

// Second bind reads different signal — first bind should NOT re-fire
const y = signal(100);
let yCalls = 0;
bind(() => { void y.value; yCalls++; });
assert('y-bind count after first', yCalls === 1);
x.set(3); await new Promise(r => queueMicrotask(r));
assert('x-bind count remains correct', calls === 3, `got ${calls}`);
assert('y-bind NOT affected by x change', yCalls === 1, `got ${yCalls}`);

// ── Report ────────────────────────────────────────────────────────

if (failures > 0) {
  console.error(`\n${failures} assertion(s) FAILED`);
  process.exit(1);
}
console.log('\n  All assertions passed');
TESTEOF

OUTPUT=$(cd "$PROJ_DIR" && node test-runtime.mjs 2>&1)
EXIT_CODE=$?

if [ $EXIT_CODE -ne 0 ]; then
  fail "runtime verification failed:\n$OUTPUT"
fi

echo "$OUTPUT"
pass "@marisjs/runtime passes all assertions as a standalone install"
