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
marisjs dev ./src
```

Starts a local dev server, builds your project, and rebuilds on save.

```
marisjs build ./src --out dist
```

Produces a static, deployable output directory.

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
[`framework-grammar-spec.md`](docs/framework-grammar-spec.md).

## File structure & routing

Server pages must live under `src/pages/`. Client components can live anywhere else (e.g.,
`src/components/`). The directory tree maps to URL routes:

| File | Route |
|------|-------|
| `src/pages/Index.tsx` | `/` |
| `src/pages/About.tsx` | `/about` |
| `src/pages/blog/Post.tsx` | `/blog/post` |

A server page is a file under `pages/` with `// @runsOn server`. It imports client
components to create interactive islands:

```tsx
// src/pages/Index.tsx — server page (the route)
// @runsOn server
import { data } from '@marisjs/runtime';
import { Counter } from '../components/Counter.tsx';

type Props = {};

export function Index(props: Props) {
  const greeting = data(async () => 'Hello from the server');
  return (
    <div>
      <h1>{greeting.value}</h1>
      <Counter client:hydrate />
    </div>
  );
}
```

```tsx
// src/components/Counter.tsx — client island
// @runsOn client
import { signal } from '@marisjs/runtime';

type Props = {};

export function Counter(props: Props) {
  const count = signal(0);
  return (
    <div>
      <span>Count: {count.value}</span>
      <button onClick={() => count.value++}>+</button>
    </div>
  );
}
```

On first build the server page is pre-rendered to static HTML. Client islands are hydrated
on page load.

## CLI commands

| Command | Description |
|---------|-------------|
| `marisjs dev ./src --out dist` | Dev server with hot reload on file change |
| `marisjs build ./src --out dist` | Compile source directory to static output |
| `marisjs validate ./src/App.tsx` | Check a single file for errors |

## Language rules

The full grammar spec is at [`docs/framework-grammar-spec.md`](docs/framework-grammar-spec.md).
Key constraints:

- One component per file. Filename must match the exported component name.
- Every file begins with `// @runsOn client` or `// @runsOn server`.
- Reactive state via `signal(initial)` and `computed(() => expr)` from `@marisjs/runtime`.
- Lists use `<For each={array} key={fn}>{(item) => <li>...</li>}</For>` — no `.map()` in JSX.
- Props are a single typed parameter (`props: MyType`), never destructured.
- Named handlers in the component body (`function handleClick() { ... }`), referenced as
  `onClick={handleClick}`.

## MCP server — AI agent integration

marisjs ships an [MCP](https://modelcontextprotocol.io/) server so AI coding agents can
call the validator directly. Register it with your agent:

**opencode** (`opencode.json`):
```json
{
  "mcp": {
    "marisjs": {
      "type": "local",
      "command": ["marisjs-mcp"],
      "enabled": true
    }
  }
}
```

**Claude Code** (`.mcp.json`):
```json
{
  "mcpServers": {
    "marisjs": {
      "command": "marisjs-mcp",
      "args": []
    }
  }
}
```

See [`docs/mcp-server.md`](docs/mcp-server.md) for build instructions and development setup.

## Deployment adapters

| Adapter | Package | Description |
|---------|---------|-------------|
| Node.js server | `@marisjs/adapter-node` | Zero-dependency HTTP server. Re-executes server routes per request, serves static routes from disk. |
| Static output | `@marisjs/adapter-static` | Produces a directory of HTML/CSS/JS for any static host (S3, GitHub Pages, Cloudflare Pages). Fails with a clear error if any route requires server execution. |

See [`docs/adapter-interface.md`](docs/adapter-interface.md) for the adapter contract, and
[`docs/writing-an-adapter.md`](docs/writing-an-adapter.md) for a walkthrough on writing your own.

## Example apps

See the `examples/` directory:

| App | Demonstrates |
|-----|-------------|
| `examples/todo-app/` | Signals, bindings, client-side reactivity |
| `examples/dashboard-app/` | Computed chains, style attributes |
| `examples/settings-app/` | Named handlers, boolean attrs, validation |
| `examples/blog-app/` | `data()` API, nested server components |
| `examples/islands-app/` | Multiple island types on one page |

## Size

A full `npm install marisjs` on Linux x64 is **4.9 MB** (16 KB wrapper + 4.8 MB native
binary). No runtime dependencies beyond Node.js >= 18. The reactive runtime is 2,812 bytes
of zero-dependency JavaScript, embedded in the CLI binary at compile time.

## Cross-platform

| Platform | Architecture | Package |
|----------|-------------|---------|
| Linux | x64 | `marisjs-linux-x64` |
| Linux | arm64 | `marisjs-linux-arm64` |
| macOS | x64 (Intel) | `marisjs-darwin-x64` |
| macOS | arm64 (Apple Silicon) | `marisjs-darwin-arm64` |
| Windows | x64 | `marisjs-win32-x64` |

npm installs only the matching platform package automatically. The wrapper locates the
native binary at runtime.

## Requirements

- Node.js >= 18
- A project with `.tsx` component files

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
- Two reference deployment adapters (Node.js server and static output).

## What marisjs does not yet do

- **No CSS scoping.** Styles are global — two components using the same class name will
  silently collide. See the spec's Section 2a for the recommended naming convention.
- **A meaningful subset of everyday JavaScript is unsupported and will fail validation
  loudly** rather than compiling into broken output: loops, `switch`/`try`/`catch`, class
  expressions, tagged templates, and a few other constructs. This is deliberate scope, not
  an oversight — see the spec for the full list.
- **A few known compiler limitations are tracked and documented in the spec** — see
  Section 8 for the current list.

## Philosophy

marisjs is a **strict subset of real TSX** — every valid marisjs file is also valid
TypeScript. The compiler adds a validation pass that rejects patterns outside the allowed
subset. Every rule is machine-checkable, surfacing a specific error code and fix hint. The
goal is to catch as many bugs as possible at validation time, before code ever reaches the
browser.

## License

MIT
