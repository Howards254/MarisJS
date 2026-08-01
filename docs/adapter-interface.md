# Adapter Interface — v1

**Purpose:** this document is the single source of truth for the contract between a
`marisjs build` output directory and any deployment adapter. An adapter author
should be able to write a correct adapter having read only this document — never
the compiler's source, never the framework's grammar spec, never the language rules.

**Audience:** anyone writing a platform adapter (Vercel, Netlify, Cloudflare, AWS
Lambda, a generic Node.js server, a custom hosting environment), and anyone
maintaining the compiler who needs to know what promises the build output makes to
adapters.

**For a worked example:** see [Writing a marisjs Adapter](writing-an-adapter.md) —
a concrete walkthrough of building an adapter for Cloudflare Workers, step by step,
referencing only this interface document.

**Ground rule:** the interface is defined from what the compiler already produces,
not from what any one platform expects. Platform-specific adapters are optional,
bounded pieces of work written against this interface — they do not define it.

---

## 1. Build output layout

After `marisjs build`, the output directory contains:

```
dist/
├── routes.json                     # Route manifest (the primary contract)
├── runtime.mjs                     # Reactive runtime (signal, computed, mount, data)
├── __build_timestamp.txt           # Unix millis timestamp of last build
├── index.html                      # Prerendered HTML for "/" (if root page exists)
├── pages/
│   └── Index.mjs                   # Server-side page module
├── components/
│   ├── Counter.mjs                 # Client-side component module
│   ├── Header.mjs
│   └── Header.css                  # Transitive CSS, copied verbatim
└── node_modules/
    └── @marisjs/
        └── runtime/
            └── package.json        # Node.js resolution shim for SSR
```

**Invariants an adapter can rely on:**

- Every file under `pages/` is a server page module (exported function that returns
  `{ html }` when called with an empty props object).
- Every file under `components/` is a client component module (exports a function
  that produces DOM nodes when called with props).
- CSS files are copied verbatim to the output directory, preserving their relative
  path from the source tree. They are globally scoped (no class-name rewriting).
- `runtime.mjs` always exists and exports `signal`, `computed`, `bind`, `mount.`
- `node_modules/@marisjs/runtime/package.json` exists inside `dist/` solely to let
  Node.js resolve `import { ... } from '@marisjs/runtime'` during SSR. Browsers use
  the import map injected into HTML — they never touch this directory.

**What an adapter should NOT assume:**

- The number of files, their exact names, or the depth of the directory tree.
  Everything is discovered through the manifest.
- That `pages/` or `components/` are the only directories. Additional asset
  directories may exist alongside them.
- That any route uses any particular client component. Client modules are listed
  per-route in the manifest.

---

## 2. The routes manifest (`routes.json`)

This is the primary contract. Every adapter starts by reading this file.

### 2.1 Schema

```typescript
// routes.json — produced by marisjs build
interface RoutesManifest {
  version: number;           // Manifest format version (currently 1)
  runtime: string;           // Path to the runtime script, relative to dist root
  routes: RouteEntry[];      // One entry per discovered page route
}

interface RouteEntry {
  path: string;              // URL path, e.g. "/", "/about", "/blog/post"
  file: string;              // Relative path to the output file for this route
  mode: "static" | "server"; // Rendering mode
  css: string[];             // Transitive CSS files for this route (relative paths)
  clientModules?: ClientModuleEntry[];  // Client modules needed for hydration
}

interface ClientModuleEntry {
  name: string;              // Exported component name, e.g. "Counter"
  path: string;              // Relative import path to the .mjs file, e.g. "./components/Counter.mjs"
}
```

### 2.2 Field reference

| Field | Type | Always present? | Purpose |
|-------|------|----------------|---------|
| `version` | integer | Yes | Manifest format version. An adapter should reject unknown versions. |
| `runtime` | string | Yes | Path to `runtime.mjs` for SSR execution. |
| `routes` | array | Yes (may be empty) | Every discovered page route, in no guaranteed order. |
| `path` | string | Yes | URL path. Always starts with `/`. |
| `file` | string | Yes | The file to serve or execute for this route. |
| `mode` | `"static"` or `"server"` | Yes | How to produce the response for this route. |
| `css` | string[] | Yes (may be empty) | CSS files to link in the HTML `<head>` for this route. Paths relative to dist root. |
| `clientModules` | array | No (absent when empty) | Client components to hydrate on page load. Each entry has `name` and `path`. |

### 2.3 Route modes

**`"static"`** — The route has no `data()` calls anywhere in its render tree. Its HTML
file is fully self-contained: same content for every request, no server execution needed.
An adapter serves this file directly from disk.

**`"server"`** — The route contains one or more `data()` calls and requires
server-side execution at request time. `file` points to an `.html` file
that was prerendered at build time — it exists as a fallback cache but the
adapter should always prefer importing the page `.mjs` module and calling
the component function to produce fresh HTML. The `data()` fetcher runs on
every request when the module is re-executed. If SSR fails (e.g., the module
throws), the adapter may fall back to serving the prerendered `.html` file.

### 2.4 Example

```json
{
  "version": 1,
  "runtime": "./runtime.mjs",
  "routes": [
    {
      "path": "/",
      "file": "index.html",
      "mode": "server",
      "css": ["components/Header.css"],
      "clientModules": [
        { "name": "Header", "path": "./pages/../components/Header.mjs" }
      ]
    },
    {
      "path": "/about",
      "file": "about.html",
      "mode": "static",
      "css": []
    }
  ]
}
```

### 2.5 Backward compatibility

A legacy format existed in pre-v1 builds where `routes.json` was a flat map of
`{ "/": "index.html" }`. Adapters handling pre-v1 output should treat any route
in that map as `mode: "static"`, with `css: []` and no `clientModules`. An adapter
targeting v1 only does not need to support the legacy format.

---

## 3. What an adapter is required to do

At minimum, an adapter must:

### 3.1 Read the manifest

Parse `routes.json` from the build output directory at startup (or deployment-
config generation time, depending on the platform). Reject unknown `version`
values — do not guess at interpretation.

### 3.2 Serve static routes

For every route with `mode: "static"`, serve `file` as a static file. A standard
static file server reading from the dist directory is sufficient.

The HTML file already contains:
- A `<script type="importmap">` that maps `@marisjs/runtime` to `./runtime.mjs`.
- CSS `<link>` tags for every file listed in `css`.
- A `<script type="module">` block that imports `mount` from the runtime and
  calls it for each entry in `clientModules` (if any).
- Server-rendered HTML inside `<div id="root">`.

The adapter does NOT need to generate any of these — they are already embedded
in the HTML by the compiler.

### 3.3 Provide SSR for server-rendered routes

For routes with `mode: "server"`, the adapter must execute the page module and
produce HTML. The canonical execution method:

```js
import { PageComponent } from './dist/pages/Page.mjs';
const result = await PageComponent({});
// result is one of:
//   { html: "<div>...</div>" }
//   "<div>...</div>"
```

The adapter wraps the result in the standard HTML shell (see Section 4). How the
adapter invokes Node.js — whether as a child process, an embedded worker, a Lambda
handler, an edge function — is entirely the adapter's business. The interface does
not prescribe an execution mechanism.

**Current state:** the v1 compiler produces `mode: "static"` for pages without `data()`
and `mode: "server"` for pages that contain `data()` calls. Both modes also produce a
prerendered `.html` file at build time; for `mode: "server"` routes the adapter must
re-execute the page module (import and call the component function) to get fresh data —
the prerendered file is a fallback, not the primary response.

### 3.4 Handle missing routes

If a request arrives for a path not listed in the manifest, the adapter returns a
404 response. It may serve a `404.html` from the output root if one exists, but the
compiler does not generate one by default.

### 3.5 Set correct Content-Type headers

- `.html` → `text/html`
- `.mjs`, `.js` → `application/javascript` or `text/javascript`
- `.css` → `text/css`
- `.json` → `application/json`
- Everything else → `application/octet-stream`

---

## 4. HTML shell for server-rendered routes

When an adapter executes a server page module for a `mode: "server"` route, it
receives raw HTML (or `{ html: "...", ... }` response). The adapter must wrap
this in the standard HTML shell — the compiler does not generate a shell for
`mode: "server"` routes since the HTML is produced at request time, not build time.

The standard shell:

```html
<!DOCTYPE html>
<html>
<head>
  <script type="importmap">
  {
    "imports": {
      "@marisjs/runtime": "./runtime.mjs"
    }
  }
  </script>
  <!-- CSS links from routes[].css -->
</head>
<body>
  <div id="root">
  <!-- SSR HTML from PageComponent({}) inserted here -->
  </div>
  <script type="module">
    import { mount } from '@marisjs/runtime';
    // import each client module from routes[].clientModules
    // mount(root, () => ComponentName({}))
  </script>
</body>
</html>
```

An adapter may generate this shell once per route and cache it, or generate it
per-request. The `<script type="importmap">` is identical to what `mode: "static"`
HTML files contain.

**Decision:** the compiler writes the complete HTML shell for static routes. The
adapter writes it for server routes. This keeps the interface simple: "static"
means "serve the file as-is," "server" means "execute the module and insert the
result into the standard shell."

---

## 5. What an adapter is explicitly NOT responsible for

An adapter author needs zero knowledge of:

- **marisjs language rules.** The adapter never sees `.tsx` source files, never
  validates component structure, never parses `@runsOn` directives, and never
  looks at `import` statements in source code. It only sees compiled `.mjs`, `.css`,
  `.html`, and `.json` files.

- **Component structure.** The adapter does not need to know which components exist,
  how they are composed, or what their render trees look like. The manifest lists
  client modules by name and path — the adapter includes them in the HTML shell
  without understanding them.

- **Signal reactivity.** The runtime handles all reactivity. The adapter never
  interacts with signals, computed values, or effects.

- **Data fetching.** `data()` calls are resolved either at build time (for static
  routes) or by executing the server module (for server routes). The adapter does
  not implement `data()` — it just executes the module.

- **CSS processing.** CSS files listed in the manifest are linked in the HTML head
  as `<link>` tags. The adapter does not bundle, minify, or scope CSS.

- **The import map.** The import map is either embedded in static HTML or generated
  by the adapter for server routes. In both cases its content is fixed:
  `@marisjs/runtime` → `./runtime.mjs`. No discovery or compilation needed.

---

## 6. Runtime resolution contract

The reactive runtime (`runtime.mjs`) is a self-contained ES module with no
dependencies. It exports:

| Export | Signature | Purpose |
|--------|-----------|---------|
| `signal` | `(initial: T) => Signal<T>` | Creates a reactive value |
| `computed` | `(fn: () => T) => Signal<T>` | Creates a derived reactive value |
| `bind` | `(el, signal, attr?) => void` | Two-way binding for form elements |
| `mount` | `(root, fn: () => Node) => void` | Attaches a client component to a DOM root |
| `data` | `(fn: () => Promise<T>) => T` | Server-only data fetcher |

The runtime is resolved at two different points:

1. **Browser:** via the import map `<script type="importmap">` in the HTML head,
   which maps `@marisjs/runtime` to `./runtime.mjs`. The `.mjs` file is served as
   a static file from the output directory.

2. **Node.js (SSR):** via the `node_modules/@marisjs/runtime/package.json` shim
   automatically generated inside the output directory. Node's standard module
   resolver walks the directory tree, finds this shim, and resolves to
   `runtime.mjs`. The adapter does not need to create this shim — it is part of
   every build output.

---

## 7. Adapter host responsibilities

Beyond the routes, the output directory may contain files that are not listed in
the manifest — e.g., client component `.mjs` files imported by `<script>` tags in
static HTML. An adapter hosting the output directory must:

- Treat the entire output directory as a web root. Any file in it may be requested
  by a browser (e.g., `./components/Counter.mjs` imported by a `<script
  type="module">`).
- Not expose `node_modules/` to browsers. The `node_modules/@marisjs/runtime/`
  shim is only for Node.js SSR — it serves no purpose for browser requests and
  can be excluded from static serving.
- Not assume any particular hosting mechanism. The adapter may copy files to a
  CDN, serve them from a local process, upload them to a serverless platform, or
  generate configuration for a reverse proxy. The interface is the files and the
  manifest — the hosting mechanism is the adapter's domain.

---

## 8. Reference: minimal Node.js adapter pseudocode

This is not a specification — it's an illustration of what a complete adapter
looks like against this interface. It handles both static and server routes.

```js
import { readFileSync, existsSync } from 'node:fs';
import { join, extname } from 'node:path';
import { createServer } from 'node:http';

const DIST = './dist';
const manifest = JSON.parse(readFileSync(join(DIST, 'routes.json'), 'utf-8'));
const mimeTypes = { '.html': 'text/html', '.mjs': 'application/javascript', '.js': 'application/javascript', '.css': 'text/css', '.json': 'application/json' };

const routeMap = new Map(manifest.routes.map(r => [r.path, r]));

function htmlShell(serverHtml, route) {
  const cssLinks = (route.css || []).map(f => `  <link rel="stylesheet" href="${f}">`).join('\n');
  const clientImports = (route.clientModules || []).map(m =>
    `    import { ${m.name} } from '${m.path}';`
  ).join('\n');
  const clientMounts = (route.clientModules || []).map(m =>
    `    mount(root, () => ${m.name}({}));`
  ).join('\n');

  return `<!DOCTYPE html>
<html>
<head>
  <script type="importmap">
  { "imports": { "@marisjs/runtime": "./runtime.mjs" } }
  </script>
${cssLinks}
</head>
<body>
  <div id="root">
  ${serverHtml}
  </div>
  <script type="module">
    import { mount } from '@marisjs/runtime';
${clientImports}
    const root = document.getElementById('root');
${clientMounts}
  </script>
</body>
</html>`;
}

createServer(async (req, res) => {
  const url = new URL(req.url, `http://${req.headers.host}`);
  const route = routeMap.get(url.pathname);

  if (route) {
    if (route.mode === 'server') {
      // Re-execute the page module for fresh data
      const pageModule = join(DIST, route.file.replace('.html', '.mjs'));
      if (existsSync(pageModule)) {
        try {
          const { [url.pathname === '/' ? 'Index' : url.pathname.slice(1).replace(/\//g, '')] : Page } = await import(pageModule);
          const result = await Page({});
          const html = typeof result === 'string' ? result : result.html;
          res.writeHead(200, { 'Content-Type': 'text/html' });
          res.end(htmlShell(html, route));
          return;
        } catch {}
      }
      // Fall through to serving prerendered HTML if SSR fails
    }

    // Static: serve prerendered HTML directly
    const filePath = join(DIST, route.file);
    if (existsSync(filePath)) {
      const ext = extname(filePath);
      res.writeHead(200, { 'Content-Type': mimeTypes[ext] || 'application/octet-stream' });
      res.end(readFileSync(filePath));
      return;
    }
  }

  // Serve arbitrary files from dist/ for script/CSS imports
  const filePath = join(DIST, url.pathname);
  if (existsSync(filePath) && !filePath.includes('node_modules')) {
    const ext = extname(filePath);
    res.writeHead(200, { 'Content-Type': mimeTypes[ext] || 'application/octet-stream' });
    res.end(readFileSync(filePath));
    return;
  }

  res.writeHead(404);
  res.end('Not found');
}).listen(process.env.PORT || 3000);
```

---

## 9. Version history

| Version | Date | Changes |
|---------|------|---------|
| 1 | 2026-07 | Initial interface. Manifest schema: version, runtime, routes[] with path/file/mode/css/clientModules. Mode determined by presence of `data()` calls: "server" if any `data()` exists in the render tree, "static" otherwise. Both modes produce a prerendered `.html` file. |
