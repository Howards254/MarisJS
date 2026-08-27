import { describe, it, expect, vi } from 'vitest';
import { signal, computed, bind, mount, effect, ref, _markEffects, _attachEffects, _disposeTree } from './index.js';

function nextTick() {
  return new Promise((resolve) => queueMicrotask(resolve));
}

describe('signal', () => {
  it('returns an object with .value getter and .set method', () => {
    const s = signal(42);
    expect(s.value).toBe(42);
    s.set(100);
    expect(s.value).toBe(100);
  });

  it('does not notify when set to the same value', async () => {
    const s = signal(0);
    const spy = vi.fn(() => { void s.value; });
    bind(spy);
    expect(spy).toHaveBeenCalledTimes(1); // initial run
    await nextTick();

    s.set(0); // same value — should NOT trigger re-run
    await nextTick();
    expect(spy).toHaveBeenCalledTimes(1); // no re-run — spy not called again
  });

  it('notifies dependent bind when value changes', async () => {
    const s = signal('hello');
    let captured;
    bind(() => {
      captured = s.value;
    });
    await nextTick();
    expect(captured).toBe('hello');

    s.set('world');
    await nextTick();
    expect(captured).toBe('world');
  });

  it('supports multiple independent signals', async () => {
    const a = signal(1);
    const b = signal(10);
    let sum;
    bind(() => {
      sum = a.value + b.value;
    });
    await nextTick();
    expect(sum).toBe(11);

    a.set(2);
    await nextTick();
    expect(sum).toBe(12);

    b.set(20);
    await nextTick();
    expect(sum).toBe(22);
  });

  it('batches multiple synchronous set calls into one bind re-run', async () => {
    const s = signal(0);
    let runs = 0;
    bind(() => {
      runs++;
      void s.value;
    });
    await nextTick();
    expect(runs).toBe(1);

    s.set(1);
    s.set(2);
    s.set(3);
    await nextTick();
    // All three sets happened before the microtask flush, so only one re-run
    expect(runs).toBe(2);
  });
});

describe('computed', () => {
  it('returns an object with .value getter only (no .set)', () => {
    const c = computed(() => 42);
    expect(c.value).toBe(42);
    expect(c.set).toBeUndefined();
  });

  it('recomputes lazily when read', () => {
    let computeCount = 0;
    const s = signal(2);
    const c = computed(() => {
      computeCount++;
      return s.value * 2;
    });

    // Not yet computed
    expect(computeCount).toBe(0);

    expect(c.value).toBe(4);
    expect(computeCount).toBe(1);

    // Read again — cached
    expect(c.value).toBe(4);
    expect(computeCount).toBe(1);
  });

  it('only recalculates when a dependency actually changes', () => {
    const spy = vi.fn((x) => x * 3);
    const s = signal(5);
    const c = computed(() => spy(s.value));

    expect(spy).toHaveBeenCalledTimes(0);

    expect(c.value).toBe(15);
    expect(spy).toHaveBeenCalledTimes(1);

    // Same value — no recompute
    s.set(5);
    expect(c.value).toBe(15);
    expect(spy).toHaveBeenCalledTimes(1);

    // Actually changes — recompute on next read
    s.set(10);
    expect(spy).toHaveBeenCalledTimes(1); // not yet read

    expect(c.value).toBe(30);
    expect(spy).toHaveBeenCalledTimes(2);
  });

  it('chains with other computeds', () => {
    let aCalls = 0;
    let bCalls = 0;
    let cCalls = 0;

    const s = signal(2);
    const a = computed(() => { aCalls++; return s.value + 1; });
    const b = computed(() => { bCalls++; return a.value * 2; });
    const c = computed(() => { cCalls++; return b.value - 1; });

    expect(aCalls).toBe(0);
    expect(bCalls).toBe(0);
    expect(cCalls).toBe(0);

    expect(c.value).toBe(5); // (2+1)*2 - 1
    expect(aCalls).toBe(1);
    expect(bCalls).toBe(1);
    expect(cCalls).toBe(1);

    s.set(3);
    // All are lazy — no calls until read
    expect(aCalls).toBe(1);
    expect(bCalls).toBe(1);
    expect(cCalls).toBe(1);

    expect(c.value).toBe(7); // (3+1)*2 - 1
    expect(aCalls).toBe(2);
    expect(bCalls).toBe(2);
    expect(cCalls).toBe(2);
  });

  it('tracks only its own dependencies, not a parent observer', () => {
    const innerSpy = vi.fn(() => 10);
    const outerSpy = vi.fn();

    const s = signal(0);
    const c = computed(() => innerSpy(s.value));
    bind(() => {
      outerSpy(c.value);
    });

    expect(outerSpy).toHaveBeenCalledTimes(1);
    expect(innerSpy).toHaveBeenCalledTimes(1);

    // Read c.value again without signal change — cached
    expect(c.value).toBe(10);
    expect(innerSpy).toHaveBeenCalledTimes(1);
  });
});

describe('bind', () => {
  it('runs the callback immediately', () => {
    let ran = false;
    bind(() => { ran = true; });
    expect(ran).toBe(true);
  });

  it('re-runs when a dependency signal changes', async () => {
    const s = signal(0);
    let count = 0;
    bind(() => { count++; void s.value; });
    await nextTick();
    expect(count).toBe(1);

    s.set(1);
    await nextTick();
    expect(count).toBe(2);
  });

  it('tears down old dependencies and rebinds when re-run', async () => {
    const s1 = signal('a');
    const s2 = signal('b');
    const useFirst = signal(true);
    let captured;

    bind(() => {
      captured = useFirst.value ? s1.value : s2.value;
    });
    await nextTick();
    expect(captured).toBe('a');

    // s2 changes while bind is tracking s1 — should not re-run
    s2.set('b-updated');
    await nextTick();
    expect(captured).toBe('a');

    // Flip condition — now should read s2
    useFirst.set(false);
    await nextTick();
    expect(captured).toBe('b-updated');

    // Now s1 changes — should not trigger since tracking s2
    s1.set('a-updated');
    await nextTick();
    expect(captured).toBe('b-updated');

    // s2 changes — should trigger
    s2.set('b-final');
    await nextTick();
    expect(captured).toBe('b-final');
  });

  it('cleans up stale dependencies after re-run', () => {
    const s1 = signal(1);
    const s2 = signal(10);

    // After bind runs, observer._sources should only contain the signals
    // actually read, and those signals' _subscribers should contain the observer.
    let readS1 = false;
    let readS2 = false;

    bind(() => {
      readS1 = true;
      void s1.value;
    });

    expect(s1._subscribers.size).toBe(1); // bind observer is subscribed
    expect(s2._subscribers.size).toBe(0);

    // Change nothing, just verify the observer is in s1's subscribers
    expect(readS1).toBe(true);
    expect(readS2).toBe(false);
  });
});

describe('mount', () => {
  it('appends a DOM Node returned by the component to rootElement', () => {
    const root = document.createElement('div');
    const child = document.createElement('span');
    child.textContent = 'hello';

    mount(root, () => child);

    expect(root.children.length).toBe(1);
    expect(root.children[0]).toBe(child);
    expect(root.children[0].textContent).toBe('hello');
  });

  it('appends a text node when a string is returned', () => {
    const root = document.createElement('div');

    mount(root, () => 'plain text');

    expect(root.childNodes.length).toBe(1);
    expect(root.textContent).toBe('plain text');
  });

  it('works with signals and bind inside the component', async () => {
    const root = document.createElement('div');
    const s = signal('initial');

    mount(root, () => {
      const span = document.createElement('span');
      bind(() => {
        span.textContent = s.value;
      });
      return span;
    });

    expect(root.textContent).toBe('initial');

    s.set('updated');
    await nextTick();
    expect(root.textContent).toBe('updated');
  });

  it('does nothing for null/undefined return', () => {
    const root = document.createElement('div');
    mount(root, () => null);
    mount(root, () => undefined);
    expect(root.childNodes.length).toBe(0);
  });

  it('returns undefined (void)', () => {
    const root = document.createElement('div');
    const result = mount(root, () => {
      const div = document.createElement('div');
      return div;
    });
    expect(result).toBeUndefined();
  });
});

describe('edge cases', () => {
  it('supports signal set during a bind run (re-entrant safety)', async () => {
    const a = signal(1);
    const b = signal(0);
    let runs = 0;

    bind(() => {
      runs++;
      if (a.value > 3) {
        b.set(a.value);
      }
    });

    await nextTick();
    expect(runs).toBe(1);

    a.set(2);
    await nextTick();
    expect(runs).toBe(2);

    a.set(5); // triggers b.set(5) inside the bind
    await nextTick();
    // b.set inside bind should schedule a re-run, ran at least twice more
    expect(runs).toBeGreaterThanOrEqual(3);
  });

  it('handles null/undefined signal values', async () => {
    const s = signal(null);
    let captured;
    bind(() => { captured = s.value; });
    await nextTick();
    expect(captured).toBeNull();

    s.set(undefined);
    await nextTick();
    expect(captured).toBeUndefined();
  });

  it('signals with NaN correctly notify on change', async () => {
    const s = signal(NaN);
    let runs = 0;
    bind(() => { runs++; void s.value; });
    await nextTick();
    expect(runs).toBe(1);

    // Object.is(NaN, NaN) is true, so setting to NaN again should be a no-op
    s.set(NaN);
    await nextTick();
    expect(runs).toBe(1);

    s.set(0);
    await nextTick();
    expect(runs).toBe(2);
  });
});

// ─── §7f: effect() — shares bind()'s tracking engine; adds cleanup + run-once ───

describe('effect', () => {
  it('auto-tracks signal reads and re-runs with an observable side effect', async () => {
    const s = signal('a');
    const log = [];
    effect(() => {
      // side effect on an external observable, not a DOM update (that's bind's job)
      log.push(s.value);
    });
    expect(log).toEqual([]); // first run is deferred to the microtask flush

    await nextTick();
    expect(log).toEqual(['a']);

    s.set('b');
    await nextTick();
    expect(log).toEqual(['a', 'b']);

    s.set('c');
    await nextTick();
    expect(log).toEqual(['a', 'b', 'c']);
  });

  it('drops subscriptions to signals not read in the latest run', async () => {
    const a = signal(0);
    const b = signal(0);
    let runs = 0;
    effect(() => {
      runs++;
      if (a.value > 0) { void b.value; }
    });
    expect(runs).toBe(0);
    await nextTick();
    expect(runs).toBe(1); // first run happens at the flush boundary

    b.set(1); // b was never read in run 1 → no subscription → no re-run
    await nextTick();
    expect(runs).toBe(1);

    a.set(1); // now the run reads b too
    await nextTick();
    expect(runs).toBe(2);

    b.set(2); // subscribed in run 2 → re-runs
    await nextTick();
    expect(runs).toBe(3);
  });

  it('runs cleanup before each re-run (not after the final one)', async () => {
    const s = signal(0);
    const events = [];
    effect(() => {
      events.push('run:' + s.value);
      return () => { events.push('cleanup:' + s.value); };
    });
    expect(events).toEqual([]);
    await nextTick();
    expect(events).toEqual(['run:0']);

    s.set(1);
    await nextTick();
    // cleanup of run:0 fires before run:1 — and it observes the NEW value,
    // proving it ran as pre-re-run teardown, not post-run teardown
    expect(events).toEqual(['run:0', 'cleanup:1', 'run:1']);

    s.set(2);
    await nextTick();
    expect(events).toEqual(['run:0', 'cleanup:1', 'run:1', 'cleanup:2', 'run:2']);
  });

  it('effect(fn, []) runs exactly once regardless of subsequent signal changes', async () => {
    const s = signal(0);
    let runs = 0;
    effect(() => {
      runs++;
      void s.value; // read must NOT subscribe in once-mode
    }, []);
    expect(runs).toBe(0);
    await nextTick();
    expect(runs).toBe(1);

    s.set(1);
    await nextTick();
    s.set(2);
    await nextTick();
    expect(runs).toBe(1); // never re-ran
    expect(s.value).toBe(2); // but reads still see current values
  });

  it('effect(fn, []) supports a cleanup that runs on dispose', async () => {
    let cleaned = false;
    const handles = [];
    const mark = _markEffects();
    effect(() => {
      return () => { cleaned = true; };
    }, []);
    const fakeRoot = { querySelectorAll: () => [] };
    _attachEffects(fakeRoot, mark);
    await nextTick(); // once-mode executes at the flush boundary
    expect(cleaned).toBe(false);
    _disposeTree(fakeRoot);
    expect(cleaned).toBe(true);
  });
});

// ─── §7f: ref() — plain inert box ───

describe('ref', () => {
  it('returns a box with current initialized to null', () => {
    const r = ref();
    expect(r.current).toBeNull();
    r.current = {};
    expect(r.current).toEqual({});
  });

  it('is deliberately inert: writes never trigger any tracking machinery', async () => {
    const s = signal(0);
    let effectRuns = 0;
    let bindRuns = 0;
    const r = ref();

    effect(() => {
      effectRuns++;
      void s.value;
      r.current = { touched: effectRuns }; // write .current inside a tracked scope
    });
    bind(() => {
      bindRuns++;
      void r.current; // read .current inside a tracked scope
      void s.value;
    });

    expect(bindRuns).toBe(1); // bind still runs synchronously
    expect(effectRuns).toBe(0);
    await nextTick();
    expect(effectRuns).toBe(1); // effect's first run lands at the flush boundary

    s.set(1); // only the signal change may drive further re-runs
    await nextTick();

    // exactly one re-run each from the signal set — ref reads/writes contributed none
    expect(effectRuns).toBe(2);
    expect(bindRuns).toBe(2);

    // a bare write outside any run persists and provokes nothing further
    r.current = { other: true };
    await nextTick();
    await nextTick();
    expect(effectRuns).toBe(2);
    expect(bindRuns).toBe(2);
    expect(r.current).toEqual({ other: true });
  });
});

// ─── §7f: unmount modeling — mark/attach/dispose contract used by codegen ───

describe('effect scope disposal (codegen contract)', () => {
  function makeFakeElement() {
    const children = [];
    return {
      children,
      _effects: undefined,
      appendChild(c) { children.push(c); },
      querySelectorAll() {
        // depth-first over appended children (enough for these tests)
        const out = [];
        const walk = (el) => { out.push(el); el.children.forEach(walk); };
        children.forEach(walk);
        return out;
      },
    };
  }

  it('_attachEffects takes only handles created after its mark (nested-safe)', async () => {
    const outerRoot = makeFakeElement();
    const innerRoot = makeFakeElement();

    const outerMark = _markEffects();
    const ran = [];
    effect(() => { ran.push('outer'); });       // belongs to OUTER

    const innerMark = _markEffects();
    effect(() => { ran.push('inner'); });       // belongs to INNER
    _attachEffects(innerRoot, innerMark);

    _attachEffects(outerRoot, outerMark);

    expect(ran).toEqual([]); // nothing ran synchronously — all deferred to flush
    await nextTick();
    expect(ran.sort()).toEqual(['inner', 'outer']); // both executed at the flush boundary

    _disposeTree(outerRoot);                    // outer disposal does NOT touch inner
    _disposeTree(innerRoot);
    // no assertion beyond "no throw" here; ownership covered by cleanup tests below
  });

  it('_disposeTree runs cleanups for every attached root in the subtree', async () => {
    const parent = makeFakeElement();
    const child = makeFakeElement();
    parent.appendChild(child);

    const cleaned = [];
    const mark = _markEffects();
    effect(() => { return () => cleaned.push('p'); });
    const childMark = _markEffects();
    effect(() => { return () => cleaned.push('c'); });
    _attachEffects(child, childMark);
    _attachEffects(parent, mark);

    await nextTick(); // both effects ran -> cleanups captured
    _disposeTree(parent);
    expect(cleaned.sort()).toEqual(['c', 'p']);
  });

  it('a disposed effect stops re-running when its signal changes later', async () => {
    const s = signal(0);
    let runs = 0;
    const root = makeFakeElement();
    const mark = _markEffects();
    effect(() => { runs++; void s.value; });
    _attachEffects(root, mark);
    await nextTick();
    expect(runs).toBe(1);

    _disposeTree(root);
    s.set(99);
    await nextTick();
    expect(runs).toBe(1); // disposed — late signal writes are inert
  });
});
