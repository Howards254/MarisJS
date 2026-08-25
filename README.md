<p align="center">
  <img src="assets/logo.svg" alt="marisjs" width="220">
</p>

# marisjs

A strict, signals-based, AI-agent-oriented full-stack framework for the web.

marisjs is a small subset of TSX, compiled by a Rust toolchain straight to plain, vanilla
JavaScript — no virtual DOM, no hydration overhead, no framework runtime beyond a ~5.4KB
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

Islands may also receive **JSX children** from the server page. The children are rendered on
the server *inside* the hydrate placeholder — so the content exists in the prerendered HTML
(crawlable, visible before JS loads) — and the runtime adopts that exact DOM at mount time
instead of re-rendering it:

```tsx
<Tabs tabs={['Design', 'Build']} client:hydrate>
  <div class="panels">…server-rendered panels…</div>
</Tabs>
```

## Page metadata, sitemap & robots

A server page declares `<head>` content by assigning a raw HTML string to a `const head`
(injected verbatim into the built page's head), or by using the `meta()` helper, which emits
a fixed, escaped tag set (`title`, `description`, Open Graph fields, `twitterCard`,
`noindex`) and composes with raw HTML (e.g. a JSON-LD `<script>` block):

```tsx
const head = meta({
  title: 'MarisJS',
  description: 'A tiny, opinionated web framework.',
  ogImage: 'https://marisjs.example/og.png',
});
```

`marisjs build` also writes `sitemap.xml` at the output root (every page route included;
API routes excluded; `meta({ noindex: true })` excludes a page; requires `SITE_URL` in the
environment — skipped with a warning if unset) and a permissive default `robots.txt` unless
the project provides its own at the source root. See the grammar spec's §2b for the exact
contracts.

## API routes, env, sessions & middleware

Alongside `pages/`, a top-level `api/` directory holds HTTP endpoints mapped to `/api/*`
URLs (`api/checkout.ts` → `/api/checkout`). Every file declares `// @runsOn api` and
exports one function per HTTP method — the export list *is* the supported-methods list.
Handlers receive and return standard Web `Request`/`Response` objects; async handlers are
the norm:

```tsx
// src/api/checkout.ts
// @runsOn api

export async function POST(req: Request): Promise<Response> {
  const key = env("STRIPE_SECRET_KEY");   // build-time secret, never shipped to clients
  const body = await req.json();
  // ...call an external service...
  return Response.json({ received: true });
}
```

Three more server-side primitives, all validator-enforced (calling any of them from a
`@runsOn client` file is a hard error — none need an import; the compiler injects them):

- **`env(key)`** — reads a value from `.env`/process environment at build time and bakes it
  into compiled server/api modules only. A missing key yields `undefined`, so
  `env("PORT") ?? "3000"` works.
- **`session()` / `setSession(data, response)`** — stateless HMAC-signed cookie sessions
  (`HttpOnly`, `SameSite=Lax`, `Secure` when built with `NODE_ENV=production`,
  constant-time verification, every failure mode degrades safely to `null`). Modules using
  sessions fail the build unless a strong `SESSION_SECRET` (16+ chars) is present.
- **`middleware(req)`** — one optional `middleware.ts` at the project root gates matching
  requests *before* any dispatch. It returns exactly one of `next()`, `redirect(url)`, or
  `respond(response)`, scoped by a static `matcher` array (`*` wildcard supported).

```ts
// middleware.ts — project root
export function middleware(req: Request) {
  const s = session();
  if (!s) return redirect('/login');
  return next();
}

export const matcher: string[] = ['/admin/*'];
```

Dispatch precedence: middleware → API routes → pages/SSR → static files.

## CLI commands

| Command | Description |
|---------|-------------|
| `marisjs dev` | Dev server with hot reload on file change (defaults: `src/` → `dist/`; serves pages, API routes, and middleware) |
| `marisjs build` | Compile source directory to static output (defaults: `src/` → `dist/`) |
| `marisjs init` | Scaffold a starter `package.json` with `dev`/`build` scripts, plus a `.gitignore` excluding `.env` and a `.env.example` |
| `marisjs validate ./src/App.tsx` | Check a single file for errors |

## Language rules

The full grammar spec is at [`docs/framework-grammar-spec.md`](docs/framework-grammar-spec.md).
Key constraints:

- One component per file. Filename must match the exported component name.
- Every file begins with `// @runsOn client`, `// @runsOn server`, or `// @runsOn api`.
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
| Node.js server | `@marisjs/adapter-node` | Zero-dependency HTTP server. Runs middleware, dispatches API routes, re-executes server routes per request, serves static routes from disk. `npx @marisjs/adapter-node ./dist` |
| Static output | `@marisjs/adapter-static` | Produces a directory of HTML/CSS/JS for any static host (S3, GitHub Pages, Cloudflare Pages). Fails with a clear error listing every route that requires server execution — server-mode pages, API routes, and middleware. `npx @marisjs/adapter-static ./dist ./out` |

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
binary). No runtime dependencies beyond Node.js >= 18. The reactive runtime is 5,450 bytes
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
- File-based routing (`pages/` directory → URL paths) and file-based API routes (`api/`
  directory → `/api/*`, one exported handler per HTTP method over standard Web
  `Request`/`Response`).
- Build-time environment secrets via `env()` (`.env` + process env snapshot baked into
  server/api modules only; `CLIENT_ENV_ACCESS` is a hard validator error).
- Stateless signed-cookie sessions: `session()` / `setSession()` with HMAC-SHA256,
  constant-time verification, `HttpOnly`/`SameSite=Lax`/`Secure`-in-production cookies,
  and a build-time `SESSION_SECRET` strength gate.
- A site-wide middleware gate: one optional root `middleware.ts` with a static `matcher`,
  returning exactly `next()`, `redirect()`, or `respond()` — run before any dispatch by the
  dev server and adapter-node.
- Islands that accept server-composed JSX children — content is prerendered inside the
  hydrate placeholder (crawlable) and adopted client-side at mount (§6a/§7e of the spec).
- Page metadata and SEO: raw `const head` HTML, the `meta()` helper (exact escaped tag
  contract), generated `sitemap.xml` (`SITE_URL`-driven, `noindex`-aware), default
  `robots.txt`, verbatim JSON-LD passthrough.
- Plain, co-located CSS files, globally scoped, copied verbatim — with a build-time
  `CSS_CLASS_COLLISION` warning calibrated against intentional overlap patterns.
- A local dev server with rebuild-on-save serving pages, API routes, and middleware.
- A structured, agent-callable validator, available as both a CLI command and an MCP tool.
- Two reference deployment adapters (Node.js server and static output).

## What marisjs does not yet do

- **No CSS scoping.** Styles are global — two components using the same class name will
  collide, but the build now warns (`CSS_CLASS_COLLISION`, naming both files and the
  class) when the same class is defined in two stylesheets loaded into one page — unless
  the overlap is the established intentional pattern (see the spec's Section 2a).
- **A meaningful subset of everyday JavaScript is unsupported and will fail validation
  loudly** rather than compiling into broken output: loops, `switch`/`try`/`catch`, class
  expressions, tagged templates, and a few other constructs. This is deliberate scope, not
  an oversight — see the spec for the full list.
- **A few known compiler limitations are tracked and documented in the spec** — see
  Section 9 for the current list.

## Philosophy

marisjs is a **strict subset of real TSX** — every valid marisjs file is also valid
TypeScript. The compiler adds a validation pass that rejects patterns outside the allowed
subset. Every rule is machine-checkable, surfacing a specific error code and fix hint. The
goal is to catch as many bugs as possible at validation time, before code ever reaches the
browser.

## License

MIT
