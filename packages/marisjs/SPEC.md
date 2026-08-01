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

- One component per file.
- Filename must match the exported component name exactly (`Cart.tsx` exports `Cart`).
- Every component file **must** begin with a directive comment declaring where it runs:

```tsx
// @runsOn client
```
or
```tsx
// @runsOn server
```

  This is a real TSX comment — parses fine in any TS/TSX toolchain — but the framework's
  validator treats it as a mandatory, machine-read directive. No file may omit it. No file may
  have more than one `@runsOn` directive. There is no inference from filename, folder location,
  or import site. This is the framework's single mechanism for the server/client boundary.

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
  copies the `.css` file verbatim to the output directory with no transformation). A future
  phase may add scoping if collisions become a real debugging burden in practice.
  Until then, the recommended convention is to prefix class names with the component name
  (e.g. `.Cart-header` rather than `.header`), but this is not enforced by any validator
  rule   — two components using the same class name will silently collide at runtime.

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
- Pages are pre-rendered to static HTML during `marisjs build` by invoking the server
  component via Node.js. The generated HTML includes an import map so browser-side code
  resolves `@marisjs/runtime` to `./runtime.mjs` without any `node_modules` dependency.

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

## 7. Forbidden Patterns (validator hard-rejects, full list — extend as discovered)

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

---

## 8. Open Items for Layer 2 (compiler) to resolve

These are flagged, not decided, because they're implementation questions rather than language
rules — but each needs an answer before the compiler is built:

- How `<For>` handles nested reactivity (does the callback re-run per-item on unrelated signal
  changes, or only when the underlying array changes?).

### D2 Benchmark — deferred gaps (found 2026-07-28)

These were discovered during the benchmark comparison (Phase D2) and are tracked here
rather than being silently worked around:

**#2 — Module-level consts not emitted in compiled output**
`const` declarations at module level (outside the component function) are not captured
by the parser and not emitted by codegen. If a component references a module-level const
in JSX (e.g. `<For each={products}>`), the generated JS references an undefined variable.
Existing example apps do not use this pattern (audited 20 files, zero occurrences), so
no apps are currently broken. Fix: add module-level const capture to the parser's
`ComponentFile` struct and emit them in codegen.

**#6 — Object-based style attributes compiled as `[object Object]`**
JSX `style={{ background: 'red', padding: '1rem' }}` is parsed as an expression containing
a JS object literal. Codegen emits `setAttribute('style', { ... })` which stringifies to
`[object Object]`. The codegen has no special handling for style objects — it treats them
as generic expression attributes. Fix: either (a) add codegen support to convert style
objects to CSS strings at compile time, or (b) add a validator check that rejects object
style syntax with a clear error directing users to use string styles
(`style="background:red;padding:1rem"`).

**#1 (resolved 2026-07-28) — Block-bodied For arrows now fully supported**
Parser fix: `extract_arrow_body_jsx` walks block statements to find the return JSX
and captures preceding function/const declarations via `span_to_snippet`. Codegen fix:
`gen_for_each` emits captured declarations inside `_rX` before the DOM-construction code,
with TS annotations stripped. Each per-item `_rX` invocation creates fresh closures.
Regression test `for_block_body_handler_is_per_item_scoped` confirms delete-on-second-item
only removes the second item (not the first).

---

## 9. How this doc gets used

- The **compiler's validator** (Layer 2) implements every rule in Sections 2–7 as a discrete,
  independently-testable check, each returning the exact error message format shown or implied
  here.
- The **agent-facing validator tool** (Layer 3a) is the same checks, exposed with structured
  JSON output and a `fix_hint` per error (see earlier discussion) — it should not need its own
  separate rule definitions; it calls the same validator code as the compiler.
- Any prompt/doc handed to a coding agent generating framework code should be generated from
  this file directly (or include it verbatim), so the agent's instructions and the compiler's
  enforcement never drift out of sync with each other.