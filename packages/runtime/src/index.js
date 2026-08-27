let currentObserver = null;

let pendingEffects = new Set();
let flushScheduled = false;

function flush() {
  while (pendingEffects.size > 0) {
    const batch = [...pendingEffects];
    pendingEffects.clear();
    for (const effect of batch) {
      effect.execute();
    }
  }
  flushScheduled = false;
}

function schedule(observer) {
  pendingEffects.add(observer);
  if (!flushScheduled) {
    flushScheduled = true;
    queueMicrotask(flush);
  }
}

export function signal(initialValue) {
  let value = initialValue;
  const subscribers = new Set();

  const self = {
    _subscribers: subscribers,
    get value() {
      if (currentObserver) {
        subscribers.add(currentObserver);
        currentObserver._sources.add(self);
      }
      return value;
    },
    set(newValue) {
      if (Object.is(value, newValue)) return;
      value = newValue;
      for (const sub of [...subscribers]) {
        sub.notify();
      }
    },
  };
  return self;
}

export function computed(fn) {
  const subscribers = new Set();
  const sources = new Set();
  let dirty = true;
  let cachedValue;

  const observer = {
    _subscribers: subscribers,
    _sources: sources,
    notify() {
      if (!dirty) {
        dirty = true;
        for (const sub of [...subscribers]) {
          sub.notify();
        }
      }
    },
  };

  const self = {
    _subscribers: subscribers,
    get value() {
      if (currentObserver) {
        subscribers.add(currentObserver);
        currentObserver._sources.add(self);
      }
      if (dirty) {
        for (const src of [...sources]) {
          src._subscribers.delete(observer);
        }
        sources.clear();

        const prev = currentObserver;
        currentObserver = observer;
        try {
          cachedValue = fn();
        } finally {
          currentObserver = prev;
        }
        dirty = false;
      }
      return cachedValue;
    },
  };
  return self;
}

export function bind(fn) {
  const observer = {
    _sources: new Set(),
    notify() {
      schedule(this);
    },
    execute() {
      for (const src of [...this._sources]) {
        src._subscribers.delete(this);
      }
      this._sources.clear();

      const prev = currentObserver;
      currentObserver = this;
      try {
        fn();
      } finally {
        currentObserver = prev;
      }
    },
  };

  observer.execute();
}

// §7f: effects share bind()'s exact tracking engine (same observer shape,
// same scheduler); they add (a) optional cleanup returned by fn — invoked
// before each re-run and on unmount — and (b) the effect(fn, []) run-once
// form, which executes fn a single time with NO observer installed, so
// signal reads inside never subscribe. The only sanctioned second argument
// is [] — a dependency-array-of-values mechanism is deliberately out of v1
// scope, permanently.
//
// TIMING (load-bearing): the first run is NOT synchronous at the effect()
// call site — it is scheduled through the same microtask flush as re-runs.
// Generated code places effect statements after the render tree is built,
// but the island root is only CONNECTED to the document when mount() (or
// hydration adoption) finishes, which is still inside the same synchronous
// turn. Deferring the first run to the microtask boundary means effects
// always execute against a connected tree where every ref.current points at
// its live node — useEffect-like commit timing.
let pendingEffectHandles = [];

export function effect(fn, once) {
  // Run-once form: execute without installing currentObserver — reads are
  // inert, nothing subscribes, nothing ever re-runs.
  if (Array.isArray(once)) {
    let ran = false;
    let cleanup = null;
    const runnable = {
      _sources: new Set(),
      _disposed: false,
      notify() { schedule(this); },
      execute() {
        if (ran || this._disposed) return;
        ran = true;
        const result = fn();
        cleanup = typeof result === 'function' ? result : null;
      },
    };
    const handle = {
      _runnable: runnable,
      _observer: null,
      get _cleanup() { return cleanup; },
      _disposed: false,
      dispose() { disposeHandle(this); },
    };
    runnable._handle = handle;

    runnable.notify(); // schedule through the standard flush
    pendingEffectHandles.push(handle);
    return;
  }

  let cleanup = null;
  const observer = {
    _sources: new Set(),
    _disposed: false,
    notify() {
      schedule(this);
    },
    execute() {
      if (this._disposed) return;
      if (typeof cleanup === 'function') cleanup();
      for (const src of [...this._sources]) {
        src._subscribers.delete(this);
      }
      this._sources.clear();

      const prev = currentObserver;
      currentObserver = this;
      try {
        const result = fn();
        cleanup = typeof result === 'function' ? result : null;
        // mirrored onto the observer for disposeHandle, which has no access
        // to this closure
        this._cleanup = cleanup;
      } finally {
        currentObserver = prev;
      }
    },
  };

  const handle = {
    _observer: observer,
    _disposed: false,
    dispose() { disposeHandle(this); },
  };
  observer._handle = handle;

  observer.notify(); // schedule first run through the standard flush
  pendingEffectHandles.push(handle);
}

function disposeHandle(handle) {
  if (handle._disposed) return;
  handle._disposed = true;
  let cleanup = null;
  if (handle._observer) {
    const obs = handle._observer;
    obs._disposed = true;
    for (const src of [...obs._sources]) {
      src._subscribers.delete(obs);
    }
    obs._sources.clear();
    cleanup = obs._cleanup;
  } else if (handle._runnable) {
    handle._runnable._disposed = true;
    cleanup = handle._cleanup;
  }
  if (typeof cleanup === 'function') cleanup();
}

// §7f unmount modeling: every client component body opens with _markEffects()
// and closes with _attachEffects(rootEl, mark) (emitted by codegen). Handles
// created between the two calls belong to that component instance and are
// attached to its root element as el._effects. Nested component calls
// interleave safely: the inner attach takes only handles created after the
// inner mark, so the outer attach collects exactly the outer component's own.
export function _markEffects() {
  return pendingEffectHandles.length;
}

export function _attachEffects(rootEl, mark) {
  if (!rootEl || typeof rootEl !== 'object') return;
  const owned = pendingEffectHandles.splice(mark);
  if (owned.length > 0) {
    rootEl._effects = (rootEl._effects || []).concat(owned);
  }
}

// Runs every effect-handle cleanup registered on el's subtree — called by
// generated <For> reconciliation immediately BEFORE a removed item's node is
// detached. Walks descendants too: components nested inside the item attached
// their own registries to their own roots.
export function _disposeTree(el) {
  if (!el || typeof el.querySelectorAll !== 'function') return;
  const roots = [el, ...el.querySelectorAll('*')];
  for (const node of roots) {
    if (node._effects) {
      for (const h of node._effects) h.dispose();
      node._effects = null;
    }
  }
}

// §7f: a plain, non-reactive mutable box. Deliberately inert — .current
// reads/writes never subscribe to anything and never trigger re-runs. The
// compiler assigns the real DOM node to .current at element creation via the
// ref={} JSX attribute.
export function ref() {
  return { current: null };
}

export function mount(rootElement, componentFn) {
  const result = componentFn(rootElement);
  if (result instanceof Node) {
    rootElement.appendChild(result);
  } else if (result != null) {
    rootElement.appendChild(document.createTextNode(String(result)));
  }
}

// §7e: coerces a JSX child expression to a DOM node. DOM nodes (component
// children — an already-built subtree handed to the wrapping component)
// pass through unchanged; every other value (string, number, null,
// undefined) becomes a text node. Null/undefined become empty text — never
// the string "null"/"undefined" that String() would produce.
export function childNode(value) {
  if (value !== null && value !== undefined && typeof value === 'object' && value.nodeType !== undefined) {
    return value;
  }
  return document.createTextNode(value == null ? '' : String(value));
}

export function data(fetcher) {
  if (typeof fetcher !== 'function') {
    throw new Error('data() requires a function: data(async () => { ... })');
  }
  return fetcher();
}

// Properties that are unitless by definition — React's well-established
// isUnitlessNumber list (react-dom CSSPropertyOperations), keyed by the
// camelCase names used in JSX style objects. Numeric values on ANY OTHER
// property get an automatic px unit; bare numbers on dimensional properties
// ("width: 100;") are invalid CSS and silently ignored by browsers.
const UNITLESS_PROPERTIES = new Set([
  'animationIterationCount', 'aspectRatio', 'borderImageOutset', 'borderImageSlice',
  'borderImageWidth', 'boxFlex', 'boxFlexGroup', 'boxOrdinalGroup', 'columnCount',
  'columns', 'flex', 'flexGrow', 'flexPositive', 'flexShrink', 'flexNegative',
  'flexOrder', 'gridArea', 'gridRow', 'gridRowEnd', 'gridRowSpan', 'gridRowStart',
  'gridColumn', 'gridColumnEnd', 'gridColumnSpan', 'gridColumnStart', 'fontWeight',
  'lineClamp', 'lineHeight', 'opacity', 'order', 'orphans', 'tabSize', 'widows',
  'zIndex', 'zoom', 'fillOpacity', 'floodOpacity', 'stopOpacity',
  'strokeDasharray', 'strokeDashoffset', 'strokeMiterlimit', 'strokeOpacity',
  'strokeWidth',
]);

// Serializes a JSX style object to a CSS string: camelCase keys become
// kebab-case properties (backgroundColor → background-color), values joined
// as "property: value;". Strings pass through unchanged so `style="a:1"` and
// `style={cond ? 'a:1' : 'b:2'}` keep working. Nullish/non-objects → ''.
// null/undefined property VALUES are omitted entirely (never "color: null;").
export function styleString(value) {
  if (typeof value === 'string') return value;
  if (value == null || typeof value !== 'object') return '';
  const parts = [];
  for (const key of Object.keys(value)) {
    const v = value[key];
    if (v == null) continue; // no value → omit the property
    const prop = key.replace(/[A-Z]/g, (m) => '-' + m.toLowerCase());
    if (typeof v === 'number' && v !== 0 && !UNITLESS_PROPERTIES.has(key)) {
      parts.push(prop + ': ' + v + 'px;');
    } else {
      parts.push(prop + ': ' + v + ';');
    }
  }
  return parts.join(' ');
}
