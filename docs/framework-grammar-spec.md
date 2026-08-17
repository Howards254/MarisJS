# Framework Grammar Spec — v0.1
### Layer 1: Language Rules (design doc, not code)

**Purpose of this document:** this is the single source of truth for what counts as valid code
in this framework. Every other layer — the compiler's validator, the agent-callable validator tool,
and any documentation fed to an AI agent — must be generated from (or checked against) this file.
If a rule isn't written here, it isn't a rule yet.

**Foundational decision:** the framework uses a **strict subset of real TSX**, parsed by a
standard TS/TSX parser (e.g. SWC). Nothing here is custom syntax. Every valid file in this
framework is also valid TSX. The compiler adds a validation pass on top that rejects patterns
outside the allowed subset — it does not change what the parser accepts as syntax, only what
the framework considers *legal* semantically.

---

## 1. Design Principles (why every rule below exists)

Every restriction in this spec must trace back to one of these. If a proposed rule doesn't map
to one of these, it doesn't belong in the spec.

1. **One canonical way to do each thing.** No competing valid patterns for the same job.
2. **No hidden execution order.** Nothing in the language depends on knowing *when* something
   runs unless it's written down explicitly at that exact location.
3. **No action at a distance.** Everything a file touches must be visible in that file:
   imports, boundary declarations, data dependencies.
4. **Fail at validation time, not runtime.** Anything checkable statically must be rejected
   before it ever reaches the compiler's code-generation stage.
5. **Every rule must be machine-checkable.** If a rule can't be turned into a concrete,
   automatable check with a specific error message, it isn't a rule — it's a guideline, and
   guidelines don't belong in a strict framework.

---

## 2. File-Level Rules

- One component per file (exception: `@runsOn api` files — see §7b — which export one
  function per HTTP method, not a component).
- Filename must match the exported component name exactly (`Cart.tsx` exports `Cart`).
- Every component file **must** begin with a directive comment declaring where it runs:

```tsx
// @runsOn client
```
or
```tsx
// @runsOn server
```
or
```tsx
// @runsOn api
```

  This is a real TSX comment — parses fine in any TS/TSX toolchain — but the framework's
  validator treats it as a mandatory, machine-read directive. No file may omit it. No file may
  have more than one `@runsOn` directive. There is no inference from filename, folder location,
  or import site. This is the framework's single mechanism for the server/client/api boundary:
  `client` files run in the browser, `server` files render pages at prerender/SSR time,
  `api` files are HTTP route handlers (see §7b).

- Allowed top-level exports per file: exactly one `export function ComponentName(...)`. No
  default exports. No secondary named exports of helper functions from a component file —
  helpers live in a separate, non-component `.ts` file and are imported.

### 2a. CSS Imports

A component may import a co-located `.css` file via a bare side-effect import at the top
of the file:

```tsx
// @runsOn client
import "./Cart.css";
import type { CartProps } from "./types";
```

**Rules (validator-enforced):**

- A CSS import must use the **bare side-effect form** and exactly that form:
  `import "./X.css"` (single or double quotes). No default import, no named import, no
  namespace import, no `import type` from a `.css` file. Any other import shape from a
  `.css` file — `import styles from "./X.css"`, `import { foo } from "./X.css"`, etc. —
  is a validation error (`INVALID_CSS_IMPORT`).
- The imported path must resolve to a real `.css` file relative to the component, using the
  same path-resolution logic already implemented for component-to-component imports
  (the `collect_component_imports` mechanism from the codegen layer). A `.css` import whose
  path does not resolve to an existing file is a validation error.
- CSS imports are only valid in `@runsOn client` files. In a `@runsOn server` file, a `.css`
  import has no effect (the server renders static HTML, not a DOM with stylesheets) and is a
  validation error: server components style their output via inline attributes on the JSX
  elements themselves, per the existing HTML generation rules.
- A component may have zero or more `.css` imports. Each is a separate `import` statement.
  No bundling or concatenation of multiple `.css` imports into a single statement is permitted.

**Runtime behavior:**

- CSS is **globally scoped** — meaning its cascade semantics: the stylesheet is loaded
  into the document's global style scope with no class-name rewriting, no CSS
  Modules-style collision protection, no Shadow DOM encapsulation. This means selectors
  like `.cart` or `button` affect every matching element in the document, not just those
  rendered by the importing component. (**This is a different meaning of "global" than
  "a site-wide shared stylesheet"** — "globally scoped" here refers to CSS cascade
  behavior (no per-component scope isolation), NOT to whether a `.css` file is included
  on every page. The pattern for a single CSS file loaded on all pages is the site-wide
  stylesheet convention described immediately below.)
  This is a known, deliberate v1 limitation: it keeps the runtime simple (the compiler
  copies the `.css` file verbatim to the output directory with no transformation), and —
  decisively — **automatic scoping is not planned in any future phase**. Class-name
  rewriting (CSS Modules-style or otherwise) would break compatibility with external CSS
  frameworks (Tailwind, Bootstrap, etc.) that depend on exact, predictable global class
  names. Instead of scoping, collision RISK is made visible: at build time, when the same
  class name is defined in two different `.css` files both transitively imported into the
  same page, the build prints a `CSS_CLASS_COLLISION` warning naming both files and the
  colliding class (a warning, never a build error — colliding class names across two
  libraries is sometimes intentional and harmless). The check is calibrated not to fire on
  the established intentional-overlap patterns: (1) the cascade-order override pattern — a
  descendant component's stylesheet redefining a class from an ancestor component's
  stylesheet (the ancestor's file loads first in the per-page `<link>` order, so the later
  stylesheet deliberately wins); (2) the site-wide stylesheet convention below — a
  stylesheet imported by a component rendered on more than one page is the base layer that
  page-specific stylesheets legitimately refine. The recommended convention remains to
  prefix class names with the component name (e.g. `.Cart-header` rather than `.header`),
  but this is not enforced by any validator rule — two components using the same class name
  will silently collide at runtime (visible only via the build warning above).

**Site-wide stylesheet convention:**

The CSS pipeline collects stylesheets per-page by walking the component dependency tree
(§2a, build-time step). For a single `.css` file shared across every page — resets,
typography, layout utilities — the pattern is a `Layout` component that every page
renders:

```tsx
// src/components/Layout.tsx
// @runsOn client
import "./styles.css";   // shared site-wide CSS

type LayoutProps = {};

export function Layout(props: LayoutProps) {
  return <nav>/* shared header, footer, etc. */</nav>;
}
```

```tsx
// src/pages/Index.tsx
// @runsOn server
import { Layout } from '../components/Layout';

type IndexProps = {};

export function Index(props: IndexProps) {
  return (
    <div>
      <Layout client:hydrate />
      <h1>Home</h1>
    </div>
  );
}
```

Because `Index`'s render tree contains `<Layout>`, the compiler walks into `Layout.tsx`,
finds `import "./styles.css"`, and includes `<link rel="stylesheet" href=".../styles.css">`
in the generated HTML. Every page that renders `<Layout>` gets the same CSS file in its
output — there is nothing else to configure.

**Rules and caveats:**

- The Layout must be `@runsOn client` because CSS imports are a validation error in
  `@runsOn server` files (§2a).
- In page files, the Layout must be marked `client:hydrate` so the SSR prerender step
  skips it (client-side DOM code like `document.createElement` would crash Node.js during
  prerender). The CSS import metadata is still collected — the `collect_component_imports`
  and `collect_css_recursive` pipeline does not execute the component; it only reads the
  import declarations.
- There is no framework-level `children` slot or `<slot>` mechanism. The Layout is
  rendered as a sibling to page content in the server page's JSX. Shared page chrome
  (nav, footer) lives inside the Layout component's own JSX. A page that needs to pass
  content through a Layout should do so via typed props — e.g., `Layout({ content: ... })`.
- The dependency direction is one-way: server files may import client files (islands),
  but a `@runsOn client` file importing a `@runsOn server` component or an api module
  is a **hard build error** (`CLIENT_IMPORTS_SERVER`) — server modules are emitted
  under `dist/_server/` with no public URL, so the client bundle would reference an
  undefined import at runtime. Server data must flow into islands via props, not
  imports.
- This is a **convention**, not a framework feature. There is no `<Layout>` built-in
  component, no special `<slot>` syntax, and no automatic global-stylesheet injection.
  The pattern works because the CSS collection walk is transitive and name-based — any
  component tag in the render tree triggers an import look-up and CSS collection.


### 2b. File-Based Routing

Server pages must be placed under a `pages/` directory inside the source root. The
directory tree maps directly to URL routes:

| File | Route |
|------|-------|
| `pages/Index.tsx` | `/` |
| `pages/About.tsx` | `/about` |
| `pages/blog/Post.tsx` | `/blog/post` |

**Rules:**

- A page file must have `// @runsOn server`. A file under `pages/` that declares
  `// @runsOn client` is a client component that happens to live in that directory —
  it is not routed as a page.
- The route for a page file is derived from its path relative to `pages/`: lowercase the
  filename, strip the extension, and strip a trailing `/index` if present.
- Client components (non-page files) can live anywhere outside `pages/`, or inside
  `pages/` as helper components — their location does not affect routing.
- **Canonical route → file mapping.** The route string is lowercased (URL convention),
  but the compiled output path keeps the source's on-disk casing. `pages/Docs/Api/Signals.tsx`
  maps to route `/docs/api/signals`, prerendered HTML at `docs/api/signals.html`, compiled
  module at `pages/Docs/Api/Signals.mjs`. The `routes.json` manifest records the real path
  (`mjs` field) for every route — the compiler, dev server, and adapter-node all read this
  mapping instead of reconstructing file paths from route strings (reconstructing with
  first-segment capitalization breaks for nested routes).
- **Depth-aware relative references.** Every root-relative reference inside a page's HTML
  (import map runtime, CSS `<link>` hrefs, client module imports) is prefixed with `../`
  once per route segment so it resolves from the page's actual output location: a page at
  `/docs/api/signals` (3 segments) uses `../../../` prefixes.
- Pages are pre-rendered to static HTML during `marisjs build` by invoking the server
  component via Node.js. The generated HTML includes an import map so browser-side code
  resolves `@marisjs/runtime` to `./runtime.mjs` without any `node_modules` dependency.
- **Folder-URL convention (static output).** `@marisjs/adapter-static` writes each route to
  a folder with an `index.html` (`/docs` → `docs/index.html`, `/docs/api/signals` →
  `docs/api/signals/index.html`) and rewrites `routes.json` to match. URL depth is
  preserved, so the compiler's depth-aware references resolve unchanged.
- **API routes are a separate convention.** Pages routing is exactly as above; HTTP
  API routes live under a top-level `api/` directory and map to `/api/*` URLs — see
  §7b for the full rules.

**Page metadata (`<head>` content):**

A server page declares page-level metadata by assigning a raw HTML string to a `const head`
at the top of the component body (in the derived-const section, per Section 3 statement
ordering rules). The string is injected verbatim into the built page's `<head>`, alongside
the import map and any CSS `<link>` tags the page's component tree requires:

```tsx
// @runsOn server
type IndexProps = {};

export function Index(props: IndexProps) {
  const head = '<title>My Page</title><meta name="description" content="About marisjs"><meta name="viewport" content="width=device-width, initial-scale=1">';
  return (
    <div>
      <h1>Hello</h1>
    </div>
  );
}
```

**Rules:**

- The name is fixed: `head`. A server page declaring `const head` gets `head` in its
  compiled return value (`{ html, head, clientBundles }`), and the prerender step injects
  it between the existing `<head>` tags of the generated HTML.
- `head` is a **raw HTML string** — the framework does not parse, validate, escape, or
  transform it. It is inserted verbatim. Use it for `<title>`, `<meta>` tags, `<link
  rel="icon">`, Open Graph tags, structured data, or any other `<head>` content.
- It must be a string literal (or a const that evaluates to a string). It must appear in
  the derived-const section: before any event handlers, per the standard ordering rules.
- This is the sanctioned pattern for page metadata in v1. `<Title>`/`<Meta>`/`<Head>`
  convenience components are deliberately not provided — raw strings keep the language
  subset small and machine-checkable.
- If the page declares no `head` const, the `<head>` contains only the import map and the
  page's CSS links (as before). A `head` const on a `@runsOn client` file has no special
  meaning — it is an ordinary derived const.

---

## 3. Component Definition

A component is a single exported function, always named in PascalCase, always taking exactly
one parameter named `props`, always typed with an inline or named `type`/`interface`.

```tsx
// @runsOn client
import type { CartItem } from "./types";

type CartProps = {
  items: CartItem[];
  onCheckout: (total: number) => void;
};

export function Cart(props: CartProps) {
  // body
  return ( /* JSX */ );
}
```

**Rules:**
- Props type must be explicit — `props: any` or an untyped `props` parameter is a validation error.
- Props type must be a `type` alias or `interface` declared in the same file, immediately above
  the component, OR imported from a dedicated `*.types.ts` file. No inline object-literal types
  in the function signature (keeps the shape scannable in one place).
- No destructuring props in the function signature (`function Cart({ items }: CartProps)`).
  Always access via `props.items`. This is a deliberate rigidity: destructuring at the signature
  obscures which fields are actually used vs. merely typed, and produces inconsistent access
  patterns across a codebase when partially destructured. One canonical access pattern:
  `props.<field>`, always.
- A component function's only allowed top-level statements are, in this order:
  1. `signal(...)` / `computed(...)` declarations
  2. Plain local `const` derived from the above (no `let`, no reassignable local state —
     mutation only happens through signals)
  3. Event handler function declarations
  4. A single `return (<JSX>)`

---

## 4. State — Signals (the only state primitive)

There is exactly one way to hold reactive state: `signal()`. There is exactly one way to
derive state from other state: `computed()`. Neither is a hook. Neither has a dependency array.
Neither has lifecycle timing rules — a `computed` simply re-evaluates whenever any signal it
reads changes; there is no "when does this run" question to get wrong.

```tsx
const discount = signal(0);
// read: discount.value, write: discount.set(0.15)
const total = computed(() => props.items.reduce((sum, i) => sum + i.price, 0) - discount.value);
```

**Rules:**
- Reading a signal's current value is always `.value` — a getter on the signal object.
  The expression must explicitly access `.value` in both JSX expressions and regular
  code; there is no implicit unwrapping or auto-subscription in JSX interpolation.
- Writing a signal is always `signal.set(newValue)` — the single canonical write method.
  Direct property assignment (`signal.value = x`) is not provided by the runtime and
  will produce a runtime error. The `.set()` form was chosen because assignment-via-property
  can be visually missed in a diff/review pass, while an explicit `.set()` call is searchable
  and unambiguous.
- `useState`, `useEffect`, `useReducer`, `useMemo`, `useCallback`, `useContext`, or any
  hook-shaped API from any other framework: **forbidden**. The validator rejects any import
  from `react`, `preact/hooks`, etc. outright — not just discourages, rejects.
- No global mutable variables outside a `signal()`. Any top-level `let` or exported mutable
  binding used as shared state is a validation error: "Global mutable state detected — use a
  signal and pass it explicitly, or lift it to a parent component's props."
- No Context API, no Provider/Consumer pattern, no dependency-injection container. State flows
  only through props, top-down.

### 4a. Cross-Cutting Concerns (theme, locale, auth session, etc.)

There is no special mechanism for cross-cutting concerns. The answer is consistent with
everything this framework already provides: a signal, created once at the application root
(or the nearest common parent), passed down through props to every component that needs it.

```tsx
// Root — e.g., pages/Index.tsx
const theme = signal({ color: 'blue' });
const locale = signal('en');

return (
  <div>
    <Header theme={theme} locale={locale} />
    <MainContent theme={theme} locale={locale} />
  </div>
);
```

The signal is passed by reference at the first level (see Section 4 rules: bare signal
identifiers are passed without `.value` unwrapping). At every deeper level, it travels as
a regular prop — the signal object itself — until a leaf component reads its `.value` and
subscribes for reactivity. Changing the signal at the root updates every subscribed leaf,
regardless of how many intermediate pass-through components exist between root and leaf.

**Known ergonomic limitation (honest assessment):** prop-drilling a theme or locale through
three, five, or ten intermediate components that never read the value is boilerplate —
each layer must declare the prop in its type and forward it in JSX. This is a deliberate
v1 tradeoff: it imposes zero new constructs, zero new codegen work, and zero new validation
rules. A future phase may evaluate whether the pain justifies a dedicated mechanism
(e.g., compiler-level prop forwarding, a `static`-like annotation, or a SignalRegistry
pattern), but that evaluation waits for real usage data — how deep chains actually get,
how many components actually need the same cross-cutting values — rather than speculative
design from first principles.

### 4b. Signals Inside `<For>` Item Templates

The same signal-by-reference rule applies to expressions inside `<For>` item templates:
to get reactive updates, read `.value` directly in the item template rather than
pre-computing derived values through a `computed()` that produces plain (non-signal)
objects.

**Does NOT update reactively (pre-computed, loses signal identity):**

```tsx
const maxVal = computed(() => Math.max(...items.value.map(i => i.val)));
const barData = computed(() => items.value.map(i => ({
  product: i.product,
  percentage: Math.round((i.val / maxVal.value) * 100),
})));
// ...
<For each={barData.value} key={(x) => x.product}>
  {(item) => <div style={'width:' + item.percentage + '%'}>{item.product}</div>}
</For>
```

Here `item.percentage` is a plain number — no `.value` access, no signal subscription.
When `maxVal` changes, `barData` recomputes and the `<For>` reconciliation re-runs,
but existing item DOM nodes keep their old widths because the compiler sees no
signal-dependent expression inside the item template.

**DOES update reactively (reads `.value` directly in the template):**

```tsx
const maxVal = computed(() => Math.max(...items.value.map(i => i.val)));
// ...
<For each={items.value} key={(x) => x.id}>
  {(item) => <div style={'width:' + Math.round((item.val / maxVal.value) * 100) + 'px'}>{item.val}</div>}
</For>
```

Here `maxVal.value` appears directly in the item template expression. The compiler
detects the `.value` access and emits a per-item `bind()` wrapper. When `maxVal`
changes, every item's style attribute updates automatically — no reconciliation
restructure needed, no special `<For>` mechanism beyond what already exists.

**Validator note:** the existing `PROP_UNWRAPPED_SIGNAL` lint (Section 4) performs
best-effort detection of signal references passed to component props, but it does
not extend to expressions inside `<For>` item templates. The compiler cannot
equivalently detect the `<For>`-item case because a `computed().map()` produces
plain objects whose properties carry no signal-identity lineage back to the original
`.value` read. The caveat already present in Section 4 applies equally here: this
pattern gap will not always be caught at validation time. Authors and agents should
treat direct `.value` reads in item templates as the canonical reactive pattern,
matching the same rule already established for component props.

---

## 5. Rendering / JSX Rules

- Exactly one root element per `return`. No fragments-as-workaround for multiple roots — if you
  need multiple top-level siblings, the parent must wrap them.
- Conditional rendering: **one canonical form**, ternary only, no `&&` short-circuit rendering.

```tsx
{ isLoggedIn ? <Dashboard /> : <LoginPrompt /> }
```

  Rationale: `condition && <Component />` silently renders `0`, `""`, or `NaN` onto the page
  when `condition` is falsy-but-not-boolean — a well-known real bug class. Ternary with an
  explicit `null` on the empty branch removes that failure mode entirely by construction.

- List rendering: **one canonical form**, a dedicated `<For>` construct, not `.map()` inline.

```tsx
<For each={props.items} key={(item) => item.id}>
  {(item) => <ItemRow item={item} />}
</For>
```

  Rationale: inline `.map()` inside JSX is where most missing-`key`-prop bugs originate, and it's
  a second valid pattern competing with a dedicated construct. Forcing a single `<For>` construct
  makes `key` a required parameter of the construct itself — impossible to forget it because
  there's no code path that skips it.

- Event handlers: always inline arrow functions or a named handler function declared inside the
  component body (per Section 3, rule 3). No handler passed by string name, no event delegation
  configuration.

---

## 6. Server/Client Data Rules

- A `// @runsOn server` component may declare data fetches using the `data()` primitive,
  imported from `@marisjs/runtime`. The `data()` function takes an async fetcher callback
  and must be `await`-ed:

```tsx
// @runsOn server
const products = await data(async () => {
  const res = await fetch("https://api.example.com/products");
  return res.json();
});
```

  **Rules:**
  - `data(fetcher: () => Promise<T>): Promise<T>` — calls `fetcher()` and returns the promise.
    No caching, no revalidation, no stale-while-revalidate in v1. Those are deferred to Layer 2+.
  - Must be used with `await`: `const x = await data(async () => ...)`. The `await` is
    required because the server component function is marked `async`; writing `const x = data(...)`
    without `await` assigns a Promise, not the resolved value, which is a bug.
  - Called only at the top level of a `@runsOn server` component body, inside a `const`
    declaration. The declaration must appear in the signal/derived-const section (per Section 3
    statement ordering rules).
  - If the fetcher throws or returns a rejected promise, the error propagates:
    at build time the prerender step fails with the error message, halting the build;
    at request time (future, deferred to Layer 2+) it produces a 500-equivalent error.
    There is no silent fallback, no empty state, no `undefined` output —
    a failed `data()` call means a failed build.

- A `// @runsOn client` component may **not** call `data()` — this is a validation error
  (`CLIENT_DATA_CALL`), not a runtime failure. Data only flows into client components via
  props from a server parent.
- Crossing the boundary (a server component rendering a client component) requires an explicit
  marker at the JSX call site — not a file-level setting, a per-usage marker, since one server
  page may mix static and interactive children:

```tsx
<Cart items={products} client:hydrate />
```

---

## 7. Environment Variables, API Routes & Sessions

### 7a. Environment Variables and Secrets

The only sanctioned way to read an environment variable is the `env()` primitive:

```tsx
// @runsOn server
const apiKey = env("STRIPE_SECRET_KEY");  // string | undefined
```

**Rules (validator-enforced):**

- `env(key: string): string | undefined` — reads a value loaded from a `.env` file at
  build/dev time (standard dotenv convention: `KEY=value` lines, `#` comments, quoted
  values). The values are baked into the compiled server/api modules at build time —
  `env()` reads the build-time snapshot; it does not touch `process.env` at runtime.
  A missing key yields `undefined`, so the standard fallback idiom works:
  `env("PORT") ?? "3000"`.
- The `.env` file is loaded from the project root (the directory where `marisjs` is
  invoked), falling back to the source directory. A key already present in the real
  process environment takes precedence over the `.env` file (standard dotenv
  non-override convention) — this is what lets CI set real secrets without editing
  `.env`.
- `env()` is only callable from `@runsOn server` or `@runsOn api` files. Calling it
  from a `@runsOn client` file is a **hard validator error** (`CLIENT_ENV_ACCESS`),
  the same enforcement tier as `CLIENT_DATA_CALL` — environment values are
  build-time server secrets, and a client bundle is publicly downloadable.
- **Best-effort lint `ENV_LEAK_TO_CLIENT_PROP`:** AST-based detection — any
  `env()` call appearing anywhere within a `client:hydrate` component's prop
  expression is flagged: the direct pattern (`<Widget apiKey={env("STRIPE_KEY")}
  client:hydrate />`), chained method calls (`apiKey={env("K").trim()}`), and
  template-literal interpolation (`auth={`Bearer ${env("API_KEY")}`}`). The
  parser records the flag from the syntax tree itself, not from textual
  matching of the expression. It is a warning, never a build failure. The
  caveat is stated explicitly: this is a best-effort check, not a complete
  guarantee — an `env()` result first stored in an intermediate object or
  variable (e.g. `const cfg = { key: env("K") }` then `<Widget cfg={cfg}
  client:hydrate />`) is not caught, because the passed expression no longer
  contains the call. The hard `CLIENT_ENV_ACCESS` rejection is the actual
  guarantee; this lint is a bonus signal.
- `marisjs init` generates a `.gitignore` excluding `.env` (appending to an existing
  `.gitignore` if present) and a `.env.example` with no real values, as the
  convention for what keys a project expects. The `.gitignore` is the **primary
  defense** against accidental secret commits; the validator rules are a **secondary
  layer** for a different failure mode — leakage of secret values into client code —
  not for git history.

### 7b. API Routes

**File-based routing.** Alongside `pages/` and `components/`, a top-level `api/`
directory holds API route files. `api/checkout.ts` maps to the route `/api/checkout`;
directory nesting maps the same way as pages (`api/billing/charge.ts` →
`/api/billing/charge`; `api/Index.ts` → `/api`).

**Rules (validator-enforced):**

- Every file under `api/` must begin with `// @runsOn api` — the same mandatory
  directive rule as pages/components, with `api` as the third valid value.
- **Handler export convention:** one exported function per HTTP method. The router
  determines which methods a route supports by which functions are exported:
  `export function GET(req)`, `export function POST(req)`, and so on for `PUT`,
  `PATCH`, `DELETE`. There is no internal method-branching pattern — the
  supported-methods list IS the export list. A default export or a non-method export
  name is a validation error.
- Handlers receive the **standard Web `Request` object** and return a standard Web
  `Response` (or a `Promise<Response>` — async handlers are the norm, since a real
  API route calling an `env()`-configured external service is almost always async).
  No custom request/response shape exists; the same types already used by `fetch()`.
- `env()` is callable from `@runsOn api` files — same tier as `@runsOn server`.
- `data()` is **not** available in `@runsOn api` files. `data()`'s contract is
  page-render-time fetching; an API route handler is not rendering a page. A `data()`
  call in an api file is a hard validation error (`API_DATA_CALL`).
- API files are not components: the component rules (props parameter, statement
  ordering, signal/computed declarations, JSX render tree, hydrate markers) do NOT
  apply to them. An api file's handler bodies are ordinary TypeScript.
- `marisjs dev` serves API routes; `@marisjs/adapter-node` serves them in production.
  `@marisjs/adapter-static` refuses a build containing API routes (fail-loud: it
  cannot execute server code, exactly as it already refuses server-mode pages).

### 7c. Sessions

Deliberately minimal session primitives for `@runsOn api` and `@runsOn server` files:
an HMAC-signed session cookie whose contents the server can read and write. No
server-side session store, no user accounts, no auth framework — just two functions.

```tsx
// @runsOn api
export function POST(req) {
  const s = session();
  return setSession(
    { visits: (s ? s.visits : 0) + 1 },
    new Response('ok'),
  );
}
export function GET(req) {
  const s = session();
  return new Response(JSON.stringify({ visits: s ? s.visits : null }));
}
```

**Rules (validator-enforced):**

- `session(): Record<string, any> | null` — reads and HMAC-verifies the incoming
  session cookie. Returns the decoded data, or `null` when the cookie is absent,
  malformed, or fails verification (tampered, truncated, wrong signature). Signature
  verification runs on every read, and all three failure modes fail safe to `null` —
  never a throw, never partial or unverified data. The incoming request is the one
  the framework dispatched to the handler; no argument is passed because the handler
  does not choose which request to read.
- `setSession(data: Record<string, any>, response: Response): Response` — signs
  `data` with HMAC and attaches it to `response` as a `Set-Cookie` header, returning
  the modified `Response`. Callers chain it on the response they return from the
  handler.
- The signing secret comes from `env('SESSION_SECRET')` — the E1 primitive, reusing
  its infrastructure; there is no second secrets mechanism. A module that calls
  `session()`/`setSession()` **must** build with a strong `SESSION_SECRET` present (in
  `.env` or the real environment) — `marisjs build`/`marisjs dev` fail loudly at build
  time with a clear error if it is missing, empty, whitespace-only, or shorter than
  16 characters. It never fails silently with a weak, default, or empty secret.
  Production guidance: a long random value (e.g. `openssl rand -base64 32`). The
  strength gate is a floor, not a substitute for good secret hygiene.
- Cookie defaults: `HttpOnly` (never readable from client JS), `SameSite=Lax` (the v1
  CSRF baseline, see below), and `Secure` in production. Whether `Secure` is set is
  determined at build time from the environment (`NODE_ENV=production`, per the
  standard convention; an adapter-provided context may override in future versions).
  Consequence: a site built without `NODE_ENV=production` serves non-`Secure` cookies
  even on an HTTPS deployment — set it at build time.
- `session()` and `setSession()` are callable only from `@runsOn api` or `@runsOn
  server` files. Calling either from a `@runsOn client` file is a **hard validator
  error** (`CLIENT_SESSION_ACCESS`), the same enforcement tier and mechanism as
  `CLIENT_DATA_CALL`/`CLIENT_ENV_ACCESS` — a session cookie is a credential-bearing
  server secret, and a client bundle is publicly downloadable. Detection is
  AST-based and covers the direct call, optional-call (`session?.()`),
  parenthesized (`(session)()`), and comma-sequence (`(0, session)()`) shapes;
  indirect wrappers (e.g. `const f = session; f()`) are a documented detection gap
  — the same limitation the `env()` leak-warning has for intermediate variables.
- The cookie value is `base64url(payload) + "." + hex(hmac-sha256(payload))` — the
  signature covers the encoded payload, so any structural change (added/removed
  fields, re-encoding) breaks verification. Verification is constant-time
  (`timingSafeEqual` over the full-length digest), and every failure mode — absent,
  truncated, length-mismatched, garbage-signature, or non-object payload — fails
  safe to `null`. The payload is `JSON.parse`d object data only; if application code
  merges session data with `Object.assign`, treat session data as untrusted input
  (a `__proto__` key can pollute prototypes at the merge site).
- The emitted runtime reserves module-scope names in files that use sessions:
  `session`, `setSession`, and `env`. A user top-level binding or import with one of
  those names is a **hard validator error** (`RUNTIME_NAME_COLLISION`) — the
  generated module would otherwise be a SyntaxError at deploy time. Duplicate
  handler exports (`export function GET` twice) are likewise rejected
  (`API_DUPLICATE_HANDLER`).

**Documented v1 limitation (deliberate, honest tradeoff):** sessions are **stateless**
— a signed cookie with no server-side store. The server cannot forcibly revoke a
session before it expires; the only revocation mechanism is rotating `SESSION_SECRET`,
which invalidates every session for all users at once. This is a design decision
stated plainly, in the same spirit as the `data()` no-caching decision: the primitive
is deliberately simple, and applications that need per-session revocation, expiry, or
server-side blacklisting must build it themselves (or treat sessions as short-lived).
The framework will not quietly grow a store in a later version — if this changes, it
is a new section, not a silent amendment.

**Documented non-goals:** marisjs does not provide password hashing or any
authentication-provider/OAuth system. Password hashing is ordinary JavaScript:
install `bcrypt` or `argon2` from npm and call it from a `@runsOn api` handler — no
special integration is needed, it is just code in a handler. Session primitives are
deliberately minimal; anything beyond signed-cookie sessions (server-side stores,
revocation, expiry, OAuth, and so on) is application code.

**CSRF baseline:** `SameSite=Lax` is the v1 baseline defense. It stops cross-site
requests from carrying the cookie in the common case, but it is not a complete CSRF
posture — by design, Lax still sends the cookie on top-level navigation GETs (a
cross-site top-level GET that triggers state changes is the residual exposure), on
same-site subdomain relationships, and during the short "Lax allow-unsafe" grace
window after a cookie is set. State-changing API routes (POST/PUT/PATCH/DELETE)
should verify the request origin explicitly (e.g. compare the `Origin` header
against the site's own origin). The full CSRF posture is subject to the mandatory
security review (E4); this section is revisited if that review finds gaps.

**Secret storage:** server-side modules (api handlers, `@runsOn server` pages and
components) are compiled under the private `dist/_server/` tree; the dev server and
the node adapter never serve `_server/` or `api/` paths statically, so the baked env
snapshot (`SESSION_SECRET`, API keys) cannot be downloaded. Client modules carry no
secrets by construction — the validator blocks `env()`/`session()` in `@runsOn
client` files — and remain publicly served for hydration.

**Two documented limits of the static-serving guarantee:**

- A `@runsOn server` page **without `data()`** is prerendered at build time into a
  static `dist/*.html` served to everyone. If its module code reads `env()`, the
  value is evaluated during that prerender and can end up inside the public HTML.
  The validator emits a `SERVER_PAGE_ENV_PRERENDER` warning for such pages — never
  read secrets from a static page; use an api route or a `data()` (dynamic) page.
  `session()` cannot leak this way — it degrades to `null` during prerender.
- The dev server and adapter-node send `Access-Control-Allow-Origin: *` on
  dispatched responses. Cookies are still `HttpOnly` (never readable by script),
  but an arbitrary website can *cause* a credentialed request to a deployed
  marisjs api route from the visitor's browser (a CSRF-style origin); this is why
  the CSRF baseline above requires explicit origin verification on
  state-changing routes, and it is an additional reason never to put secrets in
  cookies beyond the session payload.

**Directive strictness (E4-11):** `@runsOn` values must match exactly —
`client`, `server`, or `api`. A misspelled value (`@runsOn apiserver`) is treated
as a missing directive (`MISSING_RUNSON`) and fails the build; it is never
silently reclassified.

### 7d. Middleware

A single request gate for the whole site. Middleware runs on the server for every
request whose path matches a declared pattern — **before** any page/API route
dispatch and before any static-file serving. It can allow the request through,
redirect it, or short-circuit with its own response. It is the v1 answer to
auth-gating (e.g. "protect `/admin/*` behind a session check"), not a general
request pipeline.

```ts
// middleware.ts — project root, sibling to pages/ and api/
export function middleware(req: Request): MiddlewareResult {
  const s = session();
  if (!s) return redirect('/login');
  return next();
}

export const matcher: string[] = ['/admin/*'];
```

**Conventions (validator-enforced):**

- **One file, one function.** The file is exactly `middleware.ts` at the project
  root. It exports exactly one function, named `middleware`, and one array,
  named `matcher`. There is exactly one middleware for the whole site — no
  per-route middleware files, no chaining, no arrays of middlewares. This is
  the v1 scope, stated plainly.
- **Signature.** `export function middleware(req: Request): MiddlewareResult`.
  The function receives the same standard Web `Request` the matched route would
  receive.
- **Exactly three sanctioned result shapes, no others.** The function must
  `return` one of:
  - `next()` — allow the request through unchanged, proceeding to normal
    page/API routing.
  - `redirect(url: string, status?: number)` — send a real HTTP redirect
    (`status` defaults to `302`). The browser follows it.
  - `respond(response: Response)` — short-circuit: send the given standard
    `Response` and **stop** — the matched route's handler is never invoked.
  Any other return — a bare `return;`, `return undefined`, a return of an
  arbitrary value, a conditional expression that is not one of the three — is
  a **hard validator error** (`MIDDLEWARE_RESULT`). These helpers are
  middleware-scoped runtime functions; a call to `next()`/`redirect()`/
  `respond()` that is **not** the direct return value of the function is also
  a hard error (`MIDDLEWARE_HELPER_NOT_RETURNED`) — a discarded gate result is
  a silent pass-through waiting to happen.
- **Matcher semantics — exact and unambiguous (this is a security property, not
  a convenience):**
  - `matcher` must be present and an array of strings (`MATCHER_REQUIRED` /
    `MATCHER_NOT_ARRAY` / `MATCHER_NOT_STRING`).
  - Matching is performed against the request **pathname** exactly as received
    from the request line (percent-encoding preserved, **not** URL-decoded,
    matching how routing itself matches). Routing and middleware agree by
    construction, so an encoded variant cannot slip past the gate to a route
    that only the raw pathname reaches — both see the same string.
  - **Case-sensitive.** `/Admin/*` and `/admin/*` are different patterns.
  - **Trailing slashes are significant.** `/admin` matches only `/admin`;
    `/admin/` matches only `/admin/`; `/admin/*` matches `/admin/`,
    `/admin/anything`, and `/admin/a/b` — but not `/admin` (the `*` may match
    an empty run, but the literal `/` after `admin` must be present). To match
    a directory with or without its trailing slash, write `/admin*` (a bare
    `*` matches any run of characters including `/` and the empty string).
  - `*` is the only wildcard: it matches any run of characters (including `/`
    and the empty string). Everything else is literal. There is no `?` and no
    character class. `*` alone matches every path.
  - An empty `matcher` array (`[]`) matches nothing — middleware never runs.
  - A request matches if **any** pattern matches (the array is an OR).
- **`session()` and `env()` are callable inside `middleware.ts`** — the same
  enforcement tier as `@runsOn api`. The session secret gate applies: a
  middleware that calls `session()`/`setSession()` must build with a strong
  `SESSION_SECRET`. `data()` is **not** callable (`MIDDLEWARE_DATA_CALL`) — it
  is page-render-time fetching, and middleware is a request gate, not a renderer.
  A `@runsOn` directive is not allowed in `middleware.ts` (`MIDDLEWARE_NO_RUNSON`);
  middleware is implicitly server-side for the whole site. The reserved runtime
  names in `middleware.ts` are `middleware`, `matcher`, `next`, `redirect`,
  `respond`, `session`, `setSession`, `env`, and `__matchPath` — a user binding
  with any of them is a hard error (`RUNTIME_NAME_COLLISION`), same mechanism
  as session files.
- Middleware is compiled into the private `dist/_server/` tree (E4-01): it is
  only reachable through the server dispatchers, never served statically.

**Documented v1 limitations (deliberate, honest tradeoffs — the same tone as
every other v1 limitation in this spec):**

- **No response rewriting or header injection on `next()`-ed requests.** If
  middleware returns `next()`, the request proceeds untouched — middleware
  cannot add headers to, rewrite the body of, or otherwise mutate the eventual
  response. If you need to stamp a header on every response, that is per-route
  handler code, not middleware.
- **No multi-file middleware chaining.** Exactly one middleware for the site.
  Composability is application code: call one function from another inside
  `middleware.ts`.
- **Middleware must not consume the request body unless it `respond()`s.** The
  matched route receives the request fresh; if middleware has already read the
  body stream (adapter-node passes a streaming body), the handler's body is
  consumed. Gates that need the body (e.g. validating a signature) should
  `respond()` or `redirect()` after reading it, never `next()`. The dev server
  buffers bodies in memory, so this asymmetry is dev-only-invisible — the
  adapter is the source of truth.

**Routing precedence:** middleware evaluation runs first (for matched paths);
then API routes (which still dispatch before page routes — a matched `/api/*`
request that middleware `next()`s falls through to the API dispatcher); then
page routes / SSR; then static files. `redirect()` and `respond()` return
without touching any of them.

---

## 8. Forbidden Patterns (validator hard-rejects, full list — extend as discovered)

| Pattern | Why forbidden |
|---|---|
| Any hook from React/Preact/etc. | Competing state pattern; implicit timing |
| `useEffect`, or any bare side-effect-on-render construct | Hidden execution order |
| Global `let`/mutable exported bindings | Action at a distance |
| Context/Provider/DI container | Untraceable data flow |
| `&&` conditional rendering | Silent falsy-value rendering bug |
| Inline `.map()` in JSX | Missing-key bug class |
| Destructured props in function signature | Obscures actual field usage |
| Default exports | Ambiguous import naming across files |
| Multiple components per file | Breaks 1:1 filename-to-component traceability |
| Missing or duplicate `@runsOn` directive | Undefined/ambiguous execution target |
| `any`-typed or untyped props | Silent runtime type errors |
| `data()` call inside a `@runsOn client` file | Server-only capability leaking to client bundle |
| `env()` call inside a `@runsOn client` file | Build-time secret leaking to a publicly downloadable bundle |
| `session()`/`setSession()` call inside a `@runsOn client` file | Credential-bearing session cookie handling leaking to a publicly downloadable bundle |
| `data()` call inside a `@runsOn api` file | `data()` is page-render-time fetching; an api handler renders no page |

---

## 9. Open Items for Layer 2 (compiler) to resolve

These are flagged, not decided, because they're implementation questions rather than language
rules — but each needs an answer before the compiler is built:

- How `<For>` handles nested reactivity (does the callback re-run per-item on unrelated signal
  changes, or only when the underlying array changes?).

### D2 Benchmark — deferred gaps (found 2026-07-28)

These were discovered during the benchmark comparison (Phase D2) and are tracked here
rather than being silently worked around:

**#2 (resolved 2026-08-13) — Module-level consts emitted in module scope**

`const` declarations at module level (outside the component function) were not captured by
the parser and not emitted by codegen, so referencing one in JSX (e.g. `<For each={products}>`)
produced a `ReferenceError` at runtime. Fixed by capturing module-level consts in the parser's
`ComponentFile` (the same `strip_var_ts` mechanism used for in-component derived consts,
stripping TS annotations) and emitting them at module scope of the generated output, above
the component function, on BOTH the client and server codegen paths (each verified
independently — this project's history of client/server parity gaps says don't assume one
path implies the other).

Regression coverage: `client_component_references_module_level_const` asserts the const is
emitted ABOVE the component function (with TS annotations stripped) and then EXECUTES the
generated module in jsdom, asserting the rendered DOM; `server_page_references_module_level_const`
builds through the real CLI and asserts the prerendered html contains the evaluated const
values. The pre-fix state was confirmed failing on both paths (missing emit + `ReferenceError:
sections is not defined` during prerender).

**#6 (resolved 2026-08-13) — style={{...}} objects serialize to CSS strings**

JSX `style={{ background: 'red', padding: '1rem' }}` previously compiled to
`setAttribute('style', { ... })` which stringified to `[object Object]`. Fixed by adding a
runtime `styleString` serializer (`@marisjs/runtime`) that converts objects to CSS strings —
camelCase keys → kebab-case properties (`backgroundColor` → `background-color`), values
joined as `property: value;` — and wiring it into BOTH codegen paths:

- Client: any DOM element `style` expression attribute emits `setAttribute('style', styleString(expr))`.
  Object literals, computed signals (`style={boxStyle.value}`), and string values (passthrough,
  so `style={cond ? 'a:1' : 'b:2'}` keeps working) are all handled. When the expression reads
  a signal (`.value`), the call is wrapped in the existing `bind()` mechanism, so the style
  updates LIVE on signal change. `styleString` is imported only when the tree contains a style
  expression (no dead imports).
- Server: the prerendered html embeds `styleString(expr)` at render time, evaluated in the
  same expression pipeline as every other attribute expression.

Regression coverage: `client_static_style_object_serializes_to_css_string` (static object —
asserts the exact style attribute string AND `getComputedStyle`), `client_reactive_style_object_updates_computed_style`
(one value read from a signal — computed width + backgroundColor verified after signal
changes), `client_computed_style_object_updates_computed_style` (computed() form — same live
checks), and `server_style_object_serializes_in_prerendered_html` (CLI build — html contains
the serialized CSS string, no `[object Object]`). The pre-fix state was confirmed failing on
all four paths with the exact `[object Object]` symptom. Note: jsdom's `getComputedStyle` does
not refresh values for elements in detached trees (probe-verified), so the reactive tests
attach their tree to `document.body` like real usage.

**#10 (resolved 2026-08-13) — CSS collision visibility (replaces "CSS scoping" from the benchmark fix list)**

The original finding suggested CSS scoping as a gap. Deliberate decision (architect +
owner): **no automatic class-name rewriting, under any framing** — it would break
compatibility with external CSS frameworks (Tailwind, Bootstrap, etc.) that depend on
exact, predictable global class names. Instead, collision risk is made visible rather than
silent: a build-time check detects when the same class name is defined in two different
`.css` files both transitively imported into the same page, and emits a
`CSS_CLASS_COLLISION` warning (never a hard error) naming both source files and the
colliding class.

Implementation: a new `validator::css_collision` module — `extract_class_names` (a
comment/string/number-aware `.class` selector extractor, so `1.5em`, `url(...)`, and
`content: ".x"` never count as definitions) plus `find_css_class_collisions` — wired into
the CLI build. For each page, the transitive closure walk (now `collect_page_css_closure`)
records each stylesheet's import site, every component in the closure, and the
child→parent map; the check runs after all pages are walked. Calibration against the two
established intentional-overlap patterns (see §2a): a pair is exempt when the importing
components stand in a strict ancestor/descendant relation (the cascade-order override
pattern — the ancestor's file loads first in the `<link>` order, the descendant's
redefinition is deliberate), or when either importing component renders on more than one
page (the Layout site-wide stylesheet convention — the shared stylesheet is the base layer
others refine).

Regression coverage: validator unit tests (extraction edge cases: compound/list/
descendant selectors, comments, strings, numeric `1.5em`/`.5em` and `url()` dots, keyframes,
attribute-selector values, CSS escapes; finder: sibling collision fires, ancestor override
silent, site-wide silent, mixed page finds only the genuine collision) and three full-CLI
integration tests — `css_class_collision_warns_for_sibling_components_with_same_class`
(two sibling components both defining `.header` → warning names both files and the class,
build still succeeds), `css_class_collision_silent_for_ancestor_override_pattern` (the
B2/B2.3 Base.css/Override.css `.box` shape → no warning), and
`css_class_collision_silent_for_site_wide_layout_stylesheet` (Layout's `styles.css` and a
Button's `Button.css` both defining `.btn`, Layout rendered on two pages → no warning).
All three scenarios were verified manually against the real CLI binary before being locked
in as tests; the pre-existing `transitive_css_collected_and_linked_in_page_html` fixture
keeps passing unchanged (it IS the override pattern).

**#11 (resolved 2026-08-14) — Follow-up hardening round (independent verification pass)**

An independent debugging-agent pass over Fixes 1–3 found one blocking issue and three
real-but-lower-severity issues. All fixed in this round:

**Prop-drill depth (blocking).** `chain_reads_signal` only recognized a DIRECT `props`
base, so `props.count.value` (1 level) was reactive but `props.nested.count.value` (2),
`props.deeply.nested.count.value` (3), and deeper chains silently rendered once and never
updated again — the base was a `MemberExpr`, not the bare `props` identifier, so the
check fell through to the signal-name test and missed the props root. Fixed by replacing
the fixed-shape match with a general iterative walk of the entire member chain to its
root: reactive iff the chain ultimately originates from the `props` identifier at ANY
depth, or ends in a known signal/computed name (historical behavior for
`store.list.count.value` preserved; a non-string index like `props[idx].value` no longer
defeats the props-root check). No fixed ceiling remains — depth is irrelevant by
construction. Regression coverage: `props_drilled_signal_value_reactive_at_any_depth`
asserts 1, 2, 3, AND 4 levels (`props.one.value` … `props.four.a.b.c.d.value`) all emit
`bind()` (pre-fix: exactly 1 of 4) and all update live in jsdom when their parent
signals change; the pre-existing `plain_object_value_field_is_not_reactive` still passes
(no bind for non-signal chains).

**Style serializer edge cases.** Two silent-invalid-CSS bugs in `styleString()`:
null/undefined property values were emitted literally (`"color: null;"` — invalid CSS
that silently does nothing) and bare numbers on dimensional properties (`{ width: 100 }`
→ `"width: 100;"`) were missing their unit. Fixed: null/undefined values omit the
property entirely (React's no-value convention), and numeric values get an automatic
`px` unless the property is on React's well-established unitless-exempt list
(`opacity`, `zIndex`, `lineHeight`, `flexGrow`, the full `isUnitlessNumber` set —
reused, not invented). Both codegen paths were verified independently (parity
discipline): `client_style_null_values_omitted_and_numeric_dimensional_gets_px`
(asserts the exact style attribute AND that computed styles resolve — bare "width: 100;"
is ignored by browsers, so computed width === '100px' proves the px is real) and
`server_style_numeric_px_and_null_omitted_in_prerendered_html` (prerendered HTML has
`width: 120px`, no `null`/`undefined`). The pre-existing static style test's expectation
was updated (`font-size: 14` → `font-size: 14px`); all other pre-existing style tests
use string values and were unaffected.

**Server-const test independence.** `server_page_references_module_level_const` now
asserts the server-emitted module directly (`pages/Menu.mjs` carries the consts at
module scope with TS annotations stripped) in addition to the prerendered HTML — the
fixture is a pure server component with no imports and no islands, so the client
codegen path provably never runs for it and cannot "cover for" a server-only
regression.

**MCP error message quality.** Ambiguous `validate_component` input surfaced serde's
internal message ("data did not match any variant of untagged enum ValidateInput") —
a Rust type name with no guidance. `ValidateInput` now has a manual `Deserialize` that
returns an actionable error: "Provide exactly one of `path` or `source` — both or
neither were given." The strict oneOf schema is unchanged (still custom `schema_with`),
unknown fields still rejected. `test_ambiguous_input_is_rejected` asserts the message
contains the guidance and leaks no internal type; the stdio smoke test confirms the
friendly message over the real MCP transport.

**#7 (resolved 2026-08-10) — Island props now serialized through SSR**

`client:hydrate` islands previously received an empty props object `{}` at the mount call
site: `<Widget label="Go" client:hydrate />` compiled to `mount(..., () => Widget({}))`,
silently dropping every prop. Fixed by serializing the REAL props at SSR render time:

- Server codegen emits the hydrate placeholder as `<div data-hydrate="Widget" data-props='...'>`
  where the attribute is `JSON.stringify(<props object>)` computed when the server page
  executes — so dynamic values (from `data()`, signals, expressions) work, not just literals.
- The mount call reads them back: `mount(el, () => Widget(el.dataset.props ? JSON.parse(el.dataset.props) : {}))`.
  The `{}` fallback keeps hand-written HTML shells (and HTML from other compilers) working.
- The runtime `mount(rootElement, componentFn)` now passes `rootElement` to the component
  factory so the props reader can use it.
- The compiler prerender path and both adapters (node SSR shell + static export, which
  copies the prerendered HTML unchanged) emit/consume the same `data-props` contract.

Regression coverage: `nested_route_two_levels_resolves_files_css_and_imports` asserts the
serialized attribute; `adapter_node_ssr_serves_nested_route_with_depth_aware_paths` asserts
both the attribute and the props-reading mount call; `adapter_static_uses_folder_url_convention_for_nested_route`
asserts props survive the folder-URL move. Live browser verification (fresh npm install of
the published binary + fixed local build, real Chromium) confirms a static-prop island
(`label="Go"`) and a dynamic-prop island (`label={items.length === 2 ? 'TwoItems' : 'Other'}`
derived from `data()`) both hydrate with correct text after SSR.

**#1 (resolved 2026-07-28) — Block-bodied For arrows now fully supported**
Parser fix: `extract_arrow_body_jsx` walks block statements to find the return JSX
and captures preceding function/const declarations via `span_to_snippet`. Codegen fix:
`gen_for_each` emits captured declarations inside `_rX` before the DOM-construction code,
with TS annotations stripped. Each per-item `_rX` invocation creates fresh closures.
Regression test `for_block_body_handler_is_per_item_scoped` confirms delete-on-second-item
only removes the second item (not the first).

**#8 (resolved 2026-08-10) — Duplicate island imports crashed pages**

Using the SAME `client:hydrate` island twice on one page emitted two `import { Widget } from '...'`
statements for one identifier — a SyntaxError that aborted the page's module script, so the
second (and all subsequent) islands never mounted. Found via live production testing.
Fix: `collect_hydrate_roots` dedupes by component name (one import per island component),
and the mount script iterates ALL instances —

```js
for (const el of document.querySelectorAll('[data-hydrate="Widget"]')) {
  mount(el, () => Widget(el.dataset.props ? JSON.parse(el.dataset.props) : {}));
}
```

— so N instances (including islands inside `<For>` loops) each mount with their own
SSR-serialized `data-props`. Both adapters emit the same loop; `clientModules` in routes.json
is deduped. Regression tests: `same_island_twice_emits_single_import_and_two_props` (static
assertions: exactly one import, two distinct prop payloads) and
`duplicate_island_mounts_both_instances_in_browser` (real Chromium: page loads with zero
script errors, both instances hydrate with their own labels).

**#9 (resolved 2026-08-10) — Server attribute expressions were stringified**

`class={expr}`, `href={expr}`, and any other JSX attribute expression in a `@runsOn server`
component was emitted as the LITERAL source text (`class="{expr}"`) into the SSR html instead
of being evaluated — while client codegen evaluates them via `setAttribute`. Found via live
production testing. Fix: server codegen now builds the opening tag from interleaved static
string parts and evaluated expression parts (`'<div class="' + (expr) + '">'`), with boolean
attributes using presence semantics (`cond ? ' disabled=""' : ''`) mirroring the client's
setAttribute/removeAttribute pair. Regression test:
`server_expression_attributes_evaluate_in_ssr_html` covers class/href expressions, per-item
expressions inside `<For>`, truthy/falsy boolean attributes, fragments, and asserts no literal
expression source leaks into the html.

**Parity audit (2026-08-10) — client/server codegen feature matrix**

Following four client/server parity gaps surfacing one at a time (data() `.value`, DerivedConst
reactivity, the async-await fix, and #8/#9 above), every codegen feature with both a client and
a server path was audited against tests:

| Feature | Client | Server | Notes |
|---|---|---|---|
| Text nodes | ✓ tested | ✓ tested | |
| Static attrs | ✓ tested | ✓ tested | |
| Expression attrs | ✓ tested | ✓ tested (was #9) | |
| Style objects | ✓ tested (new) | ✓ tested (new) | CSS string serialization, reactive via bind (was #6) |
| Boolean attrs | ✓ tested (new) | ✓ tested | presence semantics both paths |
| Event handlers | ✓ tested | ✓ rejected (new) | server html has no JS — validator now rejects with SERVER_EVENT_HANDLER |
| Conditional/ternary | ✓ tested | ✓ tested | element-position added to #9 test |
| ForEach | ✓ tested | ✓ tested | |
| Fragments | ✓ tested (new) | ✓ tested (new) | client_fragment_renders_children_inline + #9 test |
| Components & props | ✓ tested | ✓ tested | props objects evaluated both paths |
| Hydrate islands | ✓ tested | ✓ tested | props serialization + instance dedup (#7, #8) |
| Signals/computed | ✓ tested | ✓ rejected (new) | validator now rejects with SERVER_SIGNAL — server codegen has no reactive runtime |
| data() | n/a (rejected CLIENT_DATA_CALL) | ✓ tested | |
| Derived consts | ✓ tested | ✓ tested | |
| head injection | n/a | ✓ tested | |
| CSS imports | ✓ tested | rejected (INVALID_CSS_IMPORT) | by design |

Beyond attribute expressions (#9), the audit found and fixed two more parity gaps: signals in
server files passed validation then crashed prerender with a ReferenceError (now rejected with
`SERVER_SIGNAL`), and `on*` handlers in server files were silently dropped from SSR html (now
rejected with `SERVER_EVENT_HANDLER`). No known divergence remains: `style` object attributes
previously rendered `[object Object]` on both paths — now serialized to CSS strings on both
(resolved as #6, 2026-08-13).

**#12 (resolved 2026-08-14) — Phase E1+E2: `env()` primitive & API routes**

Two genuinely new capabilities, specified in Section 7 above and implemented with the
full three-role process (workhorse → independent debugging-agent verification pass →
commit):

**E1 — `env()`.** Parser detects `env()` calls at call sites (same mechanism as
`data()`); validator enforces the §7a boundary: `CLIENT_ENV_ACCESS` (hard error,
`@runsOn client` file) and `ENV_LEAK_TO_CLIENT_PROP` (best-effort warning lint —
AST-based detection, so any `env()` call anywhere in a hydrate prop expression is
flagged: direct, chained-method, and template-literal shapes; an intermediate
variable/object wrap remains the documented limitation). The CLI loads `.env`
(project root, then source dir; real process env takes precedence — standard dotenv
semantics) at build/dev time and codegen bakes
a module-scope `env` helper carrying the value snapshot into every server/api module
that calls it. `marisjs init` now writes `.gitignore` (appending `.env` if it exists)
and a no-real-values `.env.example`. Regression coverage: server page reads a real
`.env` value end-to-end through the prerendered html; api handler reads one through a
live request; client file with `env()` hard-rejected; direct/chained/template-leak
shapes linted; indirect wrap NOT flagged — the test documents the accepted
limitation explicitly. (Follow-up hardening, 2026-08-14: the lint was upgraded from a
text-shape heuristic to AST-based detection per the independent debugging agent's
finding; `marisjs validate` and the MCP tool dispatch to `validate_api` for api/
files — previously they applied component rules to API files and emitted misleading
errors.)

**E2 — API routes.** New `@runsOn api` directive (third valid value), `api/` directory
routed file-based to `/api/*` (nested like pages), one exported function per HTTP
method (GET/POST/PUT/PATCH/DELETE) with standard Web `Request`/`Response` — async
handlers are the norm and are proven live. API files get their own, smaller validator
rule set rather than the component rules (props/ordering/signals/JSX do not apply —
handlers are ordinary TS; approach: `validate_api` runs runs-on, handler-name,
default-export, `API_DATA_CALL`, forbidden-import, CSS-import, and unsupported-
construct checks only). Codegen emits handlers verbatim (TS stripped) with relative
imports rewritten to `.mjs`; the same build-time `env` snapshot as server pages. CLI
build registers `apiRoutes` in routes.json; `marisjs dev` dispatches api requests
through Node (Request/Response constructed from the raw HTTP line); adapter-node
serves them live; adapter-static fails loud listing every api route. Regression
coverage: GET JSON e2e through `marisjs dev`; POST reading a JSON body with a computed
response; async handler with mocked `fetch()` + `env()` config (the webhook/payment
pattern) proven at module level; adapter-node live serve; adapter-static refusal;
`API_DATA_CALL` hard rejection; combined pages+api build proving pages routing is
undisturbed.

Parity: `env()` — server ✓ tested, api ✓ tested, client ✗ rejected (CLIENT_ENV_ACCESS),
exactly the `data()`-boundary shape. `data()` — server ✓, api ✗ rejected
(API_DATA_CALL), client ✗ rejected (CLIENT_DATA_CALL).

**#13 (resolved 2026-08-15) — Phase E3+E4: signed-cookie sessions**

Specified in §7c. `session()`/`setSession()` on `@runsOn api`/`@runsOn server` only
(hard `CLIENT_SESSION_ACCESS` boundary for `@runsOn client`), HMAC-SHA256-signed
`marisjs_session` cookie (`HttpOnly`, `SameSite=Lax`, `Secure` when built with
`NODE_ENV=production`), `timingSafeEqual` constant-time verification, every failure
mode fails safe to `null`. Key material comes from `env('SESSION_SECRET')`; a module
using sessions must build with a strong secret (missing/empty/whitespace-only/sub-16
characters = loud build failure). Detection is AST-based incl. `session?.()`,
`(session)()`, and `(0, session)()` shapes; wrapper emission covers sync and async
handlers; emitted module-scope names (`session`/`setSession`/`env`) are collision-
checked (`RUNTIME_NAME_COLLISION`) and duplicate handlers rejected
(`API_DUPLICATE_HANDLER`). E2 parity: server ✓, api ✓, client ✗ rejected.

**E4 — independent adversarial security review (mandatory gate, APPROVED).** An
independent review agent attacked the feature before release; every finding was
fixed and regression-tested, then re-verified by the same reviewer (252 tests,
0 failures). Fixes shipped: (E4-01 CRITICAL) server modules emitted under private
`dist/_server/`; dev server and adapter-node 404 `_server/`/`api/` paths statically;
island imports rewritten across the boundary. (E4-02) dev server path-traversal
(`/../.env`) → 404. (E4-04) 16+ char secret strength gate. (E4-05) optional-call and
comma-sequence detection shapes. (E4-06) multiple `Set-Cookie` headers forwarded as
pairs. (E4-07) 10 MiB body cap → 413. (E4-11) `@runsOn` values exact-match only.
(E4-13) static server pages reading `env()` warn `SERVER_PAGE_ENV_PRERENDER` — the
value can leak into prerendered public HTML (honesty, not prevention; documented).
(E4-14) client→server imports hard-rejected (`CLIENT_IMPORTS_SERVER`). Informational
findings (ACAO `*` CSRF-style origin caveat, `__proto__` merge note, Lax grace
window) documented in §7c. Residual non-blocking follow-ups tracked: JSX-tag-only
client references to server components (undefined import, no secret exposure) and
cosmetic `MISSING_RUNSON` wording for api files. Crypto core (HMAC construction,
constant-time compare) verified safe by review.

---

## 10. How this doc gets used

- The **compiler's validator** (Layer 2) implements every rule in Sections 2–8 as a discrete,
  independently-testable check, each returning the exact error message format shown or implied
  here.
- The **agent-facing validator tool** (Layer 3a) is the same checks, exposed with structured
  JSON output and a `fix_hint` per error (see earlier discussion) — it should not need its own
  separate rule definitions; it calls the same validator code as the compiler.
- Any prompt/doc handed to a coding agent generating framework code should be generated from
  this file directly (or include it verbatim), so the agent's instructions and the compiler's
  enforcement never drift out of sync with each other.