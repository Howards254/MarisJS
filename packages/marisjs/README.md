# marisjs

A strict-subset reactive component framework. Write TSX components in a small, machine-checkable language — the compiler validates every rule before generating code, so errors surface at build time, not runtime. Built-in reactivity via signals, computed values, and effects. Server-side data loading with `data()`. Client-side islands with `@runsOn client`.

```bash
npm install marisjs
```

## Quick start

```tsx
// src/Counter.tsx
// @runsOn client
import { signal } from '@marisjs/runtime';

type Props = {};

export function Counter(props: Props) {
  const count = signal(0);
  return (
    <div>
      <p>Count: {count.value}</p>
      <button onClick={() => count.set(count.value + 1)}>+</button>
    </div>
  );
}
```

```bash
marisjs dev ./src --out dist
# -> http://localhost:3000
```

## File structure & routing

Server pages must live under `src/pages/`. Client components can live anywhere else (e.g., `src/components/`). The directory tree maps to URL routes:

| File | Route |
|------|-------|
| `src/pages/Index.tsx` | `/` |
| `src/pages/About.tsx` | `/about` |
| `src/pages/blog/Post.tsx` | `/blog/post` |

A server page is a file under `pages/` with `// @runsOn server`. It imports client components to create interactive islands:

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

On first build the server page is pre-rendered to static HTML. Client islands are hydrated on page load.

## CLI commands

| Command | Description |
|---------|-------------|
| `marisjs dev ./src --out dist` | Dev server with hot reload on file change |
| `marisjs build ./src --out dist` | Compile source directory to static output |
| `marisjs validate ./src/App.tsx` | Check a single file for errors |

## Language rules

The full grammar spec is shipped with the package at `SPEC.md` — use it as a reference for what's valid. Key constraints:

- One component per file. Filename must match the exported component name.
- Every file begins with `// @runsOn client` or `// @runsOn server`.
- Reactive state via `signal(initial)` and `computed(() => expr)` from `@marisjs/runtime`.
- Lists use `<For each={array} key={fn}>{(item) => <li>...</li>}</For>` — no `.map()` in JSX.
- Props are a single typed parameter (`props: MyType`), never destructured.
- Named handlers in the component body (`function handleClick() { ... }`), referenced as `onClick={handleClick}`.

## MCP server — AI agent integration

marisjs ships an [MCP](https://modelcontextprotocol.io/) server so AI coding agents can call the validator directly. Register it with your agent:

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

See `docs/mcp-server.md` in the [repository](https://github.com/howards254/MarisJS) for build instructions and development setup.

## Example apps

See `examples/` in the [repository](https://github.com/howards254/MarisJS):

| App | Demonstrates |
|-----|-------------|
| `examples/todo-app/` | Signals, bindings, client-side reactivity |
| `examples/dashboard-app/` | Computed chains, style attributes |
| `examples/settings-app/` | Named handlers, boolean attrs, validation |
| `examples/blog-app/` | `data()` API, nested server components |
| `examples/islands-app/` | Multiple island types on one page |

## Size

A full `npm install marisjs` on Linux x64 is **4.9 MB** (16 KB wrapper + 4.8 MB native binary). No runtime dependencies beyond Node.js >= 18. The reactive runtime is 2,812 bytes of zero-dependency JavaScript, embedded in the CLI binary at compile time.

## Cross-platform

| Platform | Architecture | Package |
|----------|-------------|---------|
| Linux | x64 | `marisjs-linux-x64` |
| Linux | arm64 | `marisjs-linux-arm64` |
| macOS | x64 (Intel) | `marisjs-darwin-x64` |
| macOS | arm64 (Apple Silicon) | `marisjs-darwin-arm64` |
| Windows | x64 | `marisjs-win32-x64` |

npm installs only the matching platform package automatically. The wrapper locates the native binary at runtime.

## Requirements

- Node.js >= 18
- A project with `.tsx` component files

## Philosophy

marisjs is a **strict subset of real TSX** — every valid marisjs file is also valid TypeScript. The compiler adds a validation pass that rejects patterns outside the allowed subset. Every rule is machine-checkable, surfacing a specific error code and fix hint. The goal is to catch as many bugs as possible at validation time, before code ever reaches the browser.

## Links

- [Repository](https://github.com/howards254/MarisJS)
- [Grammar spec](SPEC.md) (shipped with this package)
- [MCP server docs](https://github.com/howards254/MarisJS/blob/main/docs/mcp-server.md)
- [Benchmark report](https://github.com/howards254/MarisJS/blob/main/docs/benchmark-report.md)
