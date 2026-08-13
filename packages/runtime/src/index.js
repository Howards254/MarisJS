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

export function mount(rootElement, componentFn) {
  const result = componentFn(rootElement);
  if (result instanceof Node) {
    rootElement.appendChild(result);
  } else if (result != null) {
    rootElement.appendChild(document.createTextNode(String(result)));
  }
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
