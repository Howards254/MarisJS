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
  const result = componentFn();
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
