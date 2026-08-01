# marisjs

A strict, signals-based, AI-agent-oriented full-stack framework for the web.

marisjs is a small subset of TSX, compiled by a Rust toolchain straight to plain, vanilla
JavaScript — no virtual DOM, no hydration overhead, no framework runtime beyond a ~2.8KB
signals library. It's designed from the ground up to be easy for a coding agent to generate
correctly: one canonical way to do each thing, explicit rules a fast validator can check
mid-generation, and no hidden execution order to get wrong.

---

## Why marisjs

Most frameworks are designed for human ergonomics — flexible APIs, multiple valid patterns,
implicit conventions a developer learns over time. marisjs is designed for a different
consumer: an AI coding agent that benefits from **rigidity, not flexibility** — a small,
unambiguous rule set it can check its own work against before a build even runs.

Concretely, this means:
- **No hooks, no implicit lifecycle timing** — state is `signal()`/`computed()`, with no
  dependency arrays and no "when does this run" question to get wrong.
- **One way to hold state, one way to render a list, one way to declare a component** — no
  competing valid patterns for an agent to inconsistently choose between.
- **Explicit server/client boundaries** — every file declares where it runs; nothing is
  inferred from a filename or import location.
- **A real-time validator tool** an agent can call mid-generation, returning structured,
  machine-actionable errors — not a wall of prose it has to parse and guess at.
- **A small, honest runtime.** The compiled output ships almost no framework code to the
  browser — correctness and performance come from the compiler, not a large runtime library.

## Install

```
npm install marisjs
npm install @marisjs/runtime
```

`marisjs` is the CLI/compiler. `@marisjs/runtime` is the tiny signals library your compiled
components import at runtime.

## Quick start

```
marisjs dev
```

Starts a local dev server, builds your project, and rebuilds on save. `src/` and `dist/`
are the defaults — pass a source path and/or `--out <dir>` for a non-standard layout.

```
marisjs build
```

Produces a static, deployable output directory in `dist/`.

```
npx @marisjs/adapter-node ./dist
```

Serves the finished output with clean URLs at http://localhost:3000 — for previewing
locally or running anywhere Node.js exists. (`@marisjs/adapter-static` produces a plain
static directory instead, for CDNs and static hosts.)

```
marisjs validate ./src/MyComponent.tsx
```

Checks a single file against the language rules and prints structured JSON diagnostics —
this is the same check an AI agent can call as a tool while it's writing code.

## A minimal component

```tsx
// @runsOn client
type CounterProps = {
  label: string;
};

export function Counter(props: CounterProps) {
  const count = signal(0);
  const doubled = computed(() => count.value * 2);

  function increment() {
    count.set(count.value + 1);
  }

  return (
    <div>
      <p>{props.label}: {count.value}</p>
      <p>Doubled: {doubled.value}</p>
      <button onClick={increment}>+1</button>
    </div>
  );
}
```

Every rule this example follows — the `@runsOn` directive, `signal`/`computed` instead of
hooks, no destructured props — is documented in full in
[`docs/framework-grammar-spec.md`](./docs/framework-grammar-spec.md). Read that file for the
complete language reference.

## What marisjs does today

- Client-side reactive components: signals, computed values, event handlers, conditional
  rendering, keyed list rendering (`<For>`), component composition with preserved internal
  state across parent re-renders.
- Server-side rendering with `@runsOn server` components, including server-fetched data via
  `data()`.
- File-based routing (`pages/` directory → URL paths).
- Plain, co-located CSS files, globally scoped.
- A local dev server with rebuild-on-save.
- A structured, agent-callable validator, available as both a CLI command and an MCP tool.

## What marisjs does not yet do

Being upfront about this matters more than sounding finished:

- **No platform-specific deployment adapters yet.** `@marisjs/adapter-node` serves the
  build output anywhere Node.js runs, and `@marisjs/adapter-static` produces a plain static
  directory for any static host; Vercel/Netlify-specific adapters are future work.
- **No CSS scoping.** Styles are global — two components using the same class name will
  silently collide. See the spec's Section 2a for the recommended (unenforced) naming
  convention.
- **A meaningful subset of everyday JavaScript is unsupported and will fail validation
  loudly** rather than compiling into broken output: loops, `switch`/`try`/`catch`, class
  expressions, tagged templates, and a few other constructs. This is deliberate scope, not
  an oversight — see the spec for the full list.
- **A few known compiler limitations are tracked and documented, not silently ignored** — see
  Section 8 of the spec for the current list.
- **No published npm package yet.** Install locally from source for now
  (`npm install /path/to/marisjs/packages/marisjs`).



## License

MIT