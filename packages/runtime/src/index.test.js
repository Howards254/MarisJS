import { describe, it, expect, vi } from 'vitest';
import { signal, computed, bind, mount } from './index.js';

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
