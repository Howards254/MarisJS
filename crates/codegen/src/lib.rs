use parser::{ComponentFile, JsxAttrValue, JsxNode, RunsOn, SignalKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Build-time snapshot of the project's environment (loaded from `.env` +
/// real process env by the CLI). Baked into server/api modules as a
/// module-scope `env` helper; never touches `process.env` at runtime.
pub type EnvMap = HashMap<String, String>;

pub fn generate(component: &ComponentFile, env: &EnvMap) -> Result<String, String> {
    generate_with_server_files(component, env, Path::new(""), &std::collections::HashSet::new())
}

/// E4-01: server-side modules are emitted under `_server/<rel>` while client
/// modules stay at the public `<rel>` — so a server module importing a CLIENT
/// module (a hydrate island) must rewrite the relative specifier to walk back
/// up to the public tree. `rel` is the importing file's root-relative source
/// path; `server_files` is the set of root-relative source paths that are
/// server-side (emitted under `_server/`). Server→server relative imports are
/// untouched (both modules move together, so relative resolution still works).
pub fn generate_with_server_files(
    component: &ComponentFile,
    env: &EnvMap,
    rel: &Path,
    server_files: &std::collections::HashSet<std::path::PathBuf>,
) -> Result<String, String> {
    if component.runs_on == Some(RunsOn::Server) {
        generate_server(component, env, rel, server_files)
    } else {
        generate_client(component)
    }
}

/// §7b: API route codegen. Emits a plain ESM module: non-CSS imports,
/// (the build-time env snapshot when the file calls env()), module-level
/// consts, then every exported method handler VERBATIM (TS annotations
/// stripped by the parser). There is no signal/JSX machinery — handlers are
/// ordinary TypeScript compiled to ordinary JavaScript.
pub fn generate_api(component: &ComponentFile, env: &EnvMap) -> Result<String, String> {
    let mut output = String::new();

    for imp in &component.imports {
        if imp.is_css {
            continue; // rejected by the validator anyway — never emitted
        }
        // Rewrite RELATIVE imports to the compiled .mjs sibling (same rule as
        // component imports); bare/node_modules specifiers pass through.
        let source = if imp.source.starts_with("./") || imp.source.starts_with("../") {
            format!("{}.mjs", imp.source.trim_end_matches(".tsx").trim_end_matches(".ts"))
        } else {
            imp.source.clone()
        };
        let names = imp
            .imported_names
            .iter()
            .map(|n| n.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        output.push_str(&format!("import {{ {} }} from '{}';\n", names, source));
    }
    if !component.imports.is_empty() {
        output.push('\n');
    }

    let mut referenced_keys = component.env_call_keys.clone();
    if component.has_session_call {
        // The session block reads env('SESSION_SECRET') and
        // env('NODE_ENV'); make sure both are baked even when the file
        // never calls env() directly.
        for key in ["SESSION_SECRET", "NODE_ENV"] {
            if !referenced_keys.iter().any(|k| k == key) {
                referenced_keys.push(key.to_string());
            }
        }
    }
    if component.has_env_call || component.has_session_call {
        output.push_str(&emit_env_helper(env, &referenced_keys));
        output.push('\n');
    }

    if component.has_session_call {
        output.push_str(&emit_session_block());
        output.push('\n');
    }

    // ALL top-level statements in source order — api files are ordinary
    // TypeScript modules, so setup code, helper functions, and consts are
    // all emitted verbatim (TS stripped). Handlers reference these bindings
    // at call time; the env helper above is defined first so module-level
    // env() uses (e.g. a const built from env at eval time) resolve too.
    for stmt in &component.module_statements {
        output.push_str(stmt);
        output.push('\n');
    }

    for (name, source) in &component.exported_fn_sources {
        // The captured snippet spans the full `export function GET(...) {...}`
        // declaration (TS annotations already stripped by the parser), so it
        // is emitted verbatim. Every captured source is a named function
        // export — validated to be a sanctioned method name.
        //
        // When the module uses sessions, each handler is wrapped so the
        // request it was dispatched with lands in the AsyncLocalStorage
        // context (module-scoped mutation would race across concurrent
        // requests — the wrapper is the request-context seam, so neither
        // the dev server nor the adapter needs session knowledge).
        let emitted = if component.has_session_call {
            wrap_handler_with_session_context(name, source)
        } else {
            source.clone()
        };
        output.push_str(&emitted);
        if !emitted.trim_end().ends_with('}') {
            output.push('\n');
        }
        output.push('\n');
    }

    if output.trim().is_empty() {
        return Err("no handlers emitted".to_string());
    }
    Ok(output)
}

/// Emits the module-scope env helper carrying the build-time snapshot:
/// `const env = (key) => ({...values})[key];`. Keys/values are JSON-escaped,
/// so any value (quotes, newlines, non-ASCII) survives the round trip.
///
/// Only the keys the file actually references (parser's env_call_keys) are
/// baked — never the whole process environment. An unreferenced key cannot
/// leak into the compiled output; a referenced-but-absent key is simply not
/// in the map and yields undefined at runtime (so `?? fallback` works).
fn emit_env_helper(env: &EnvMap, referenced_keys: &[String]) -> String {
    let entries: Vec<String> = referenced_keys
        .iter()
        .filter_map(|k| {
            env.get(k).map(|v| {
                format!(
                    "{}: {}",
                    serde_json::to_string(k).unwrap(),
                    serde_json::to_string(v).unwrap()
                )
            })
        })
        .collect();
    format!(
        "const env = (key) => ({{ {} }})[key];\n",
        entries.join(", ")
    )
}

/// §7c canonical session runtime, emitted inline into every server/api
/// module that calls session()/setSession() (server modules are
/// self-contained — same rule as the env helper). HMAC signing/verification
/// uses Node's built-in crypto only: `createHmac` + `timingSafeEqual`
/// (constant-time comparison — a naive `===` on strings would leak timing).
/// The incoming request lives in an AsyncLocalStorage context set by the
/// generated handler wrapper — never a module-scoped mutable, which would
/// race across concurrent requests in the adapter.
///
/// Failure modes are all fail-safe-to-null: no cookie, unparseable cookie,
/// wrong-length or mismatched signature, malformed payload — `session()`
/// returns null, never throws, never returns unverified data.
fn emit_session_block() -> String {
    r#"import { createHmac, timingSafeEqual } from 'node:crypto';
import { AsyncLocalStorage } from 'node:async_hooks';

const __sessionAls = new AsyncLocalStorage();
const __sessionCookie = 'marisjs_session';
const __sessionSecure = (env('NODE_ENV') || '') === 'production';

const __sessionSign = (payload) => createHmac('sha256', env('SESSION_SECRET')).update(payload).digest('hex');

const __sessionExtract = (header) => {
  for (const part of header.split(';')) {
    const eq = part.indexOf('=');
    if (eq <= 0) continue;
    if (part.slice(0, eq).trim() === __sessionCookie) {
      return part.slice(eq + 1).trim();
    }
  }
  return null;
};

const session = () => {
  const secret = env('SESSION_SECRET');
  if (!secret) return null;
  const req = __sessionAls.getStore();
  if (!req) return null;
  const header = req.headers && req.headers.get('cookie');
  if (!header) return null;
  const raw = __sessionExtract(header);
  if (!raw) return null;
  const dot = raw.lastIndexOf('.');
  if (dot <= 0 || dot === raw.length - 1) return null;
  const payload = raw.slice(0, dot);
  const expected = __sessionSign(payload);
  const given = Buffer.from(raw.slice(dot + 1), 'hex');
  const want = Buffer.from(expected, 'hex');
  if (given.length !== want.length || !timingSafeEqual(given, want)) return null;
  try {
    const decoded = JSON.parse(Buffer.from(payload, 'base64url').toString('utf8'));
    return decoded && typeof decoded === 'object' && !Array.isArray(decoded) ? decoded : null;
  } catch {
    return null;
  }
};

const setSession = (data, response) => {
  const secret = env('SESSION_SECRET');
  if (!secret || !response) return response;
  const payload = Buffer.from(JSON.stringify(data), 'utf8').toString('base64url');
  const cookie = __sessionCookie + '=' + payload + '.' + __sessionSign(payload) + '; Path=/; HttpOnly; SameSite=Lax' + (__sessionSecure ? '; Secure' : '');
  response.headers.append('Set-Cookie', cookie);
  return response;
};
"#
    .to_string()
}

/// §7c: `export function GET(req) { ... }` → `function __raw_GET(req) { ... }`
/// plus an exported wrapper that runs the real body inside the session
/// request context (args[0] is the dispatched Request). The wrapper is the
/// single seam that makes the no-argument `session()` work — neither the
/// dev server nor the adapter-node router needs any session knowledge.
/// Async handlers are handled too (`export async function GET`): the wrapped
/// body stays async and AsyncLocalStorage preserves the request context
/// across `await`, so `session()` keeps working.
/// The body is emitted verbatim; only the declaration header changes.
fn wrap_handler_with_session_context(name: &str, source: &str) -> String {
    for (prefix, raw_decl) in [
        (format!("export async function {name}("), "async function"),
        (format!("export function {name}("), "function"),
    ] {
        if let Some(rest) = source.strip_prefix(&prefix) {
            return format!(
                "{raw_decl} __raw_{name}({rest}\n\nexport function {name}(...args) {{\n  return __sessionAls.run(args[0], () => __raw_{name}(...args));\n}}\n"
            );
        }
    }
    source.to_string()
}

fn generate_client(component: &ComponentFile) -> Result<String, String> {
    let component_name = component
        .exports
        .first()
        .map(|e| e.name.as_str())
        .ok_or_else(|| "No exported component found".to_string())?;

    let render_tree = component
        .render_tree
        .as_ref()
        .ok_or_else(|| "No render tree found in component".to_string())?;

    let mut signal_names: Vec<String> = component.signals.iter().map(|s| s.name.clone()).collect();

    let mut output = String::new();
    let mut counter = AtomicCounter::new();

    let props_param = component.props.as_ref().map(|p| p.name.as_str()).unwrap_or("props");

    let has_signal = component.signals.iter().any(|s| s.kind == SignalKind::Signal);
    let has_computed = component.signals.iter().any(|s| s.kind == SignalKind::Computed);
    let needs_bind = tree_reads_signal(render_tree, &signal_names, props_param)
        || has_for_each_or_conditional(render_tree);
    let needs_style = tree_has_style_expr(render_tree);

    let mut imports = Vec::new();
    if has_signal { imports.push("signal"); }
    if has_computed { imports.push("computed"); }
    if needs_bind { imports.push("bind"); }
    if needs_style { imports.push("styleString"); }

    let comp_imports = collect_component_imports(render_tree, &component.imports);
    for (name, source) in &comp_imports {
        output.push_str(&format!("import {{ {} }} from '{}';\n", name, source));
    }
    if !comp_imports.is_empty() {
        output.push('\n');
    }

    if !imports.is_empty() {
        output.push_str(&format!(
            "import {{ {} }} from '@marisjs/runtime';\n\n",
            imports.join(", ")
        ));
    }

    for mc in &component.module_consts {
        for line in mc.lines() {
            output.push_str(line);
            output.push('\n');
        }
    }
    if !component.module_consts.is_empty() {
        output.push('\n');
    }

    output.push_str(&format!("export function {}(props) {{\n", component_name));

    for sig in &component.signals {
        let fn_name = match sig.kind {
            SignalKind::Signal => "signal",
            SignalKind::Computed => "computed",
        };
        output.push_str(&format!(
            "  const {} = {}({});\n",
            sig.name, fn_name, sig.initial_value
        ));
    }
    if !component.signals.is_empty() {
        output.push('\n');
    }

    for dc in &component.derived_consts {
        let dc_trimmed = dc.trim();
        if let Some(var_name) = is_reactive_derived_const(dc_trimmed, &signal_names) {
            let init_expr = extract_const_init(dc_trimmed);
            output.push_str(&format!("  const {var_name} = computed(() => ({init_expr}));\n"));
            signal_names.push(var_name);
        } else {
            for line in dc.lines() {
                output.push_str(&format!("  {}\n", line));
            }
        }
    }
    if !component.derived_consts.is_empty() {
        output.push('\n');
    }

    for handler in &component.handler_decls {
        // Indent the handler by 2 spaces to match component body
        for line in handler.lines() {
            output.push_str(&format!("  {}\n", line));
        }
    }
    if !component.handler_decls.is_empty() {
        output.push('\n');
    }

    let root_var = gen_node(render_tree, &mut output, &mut counter, 1, &signal_names, props_param)?;

    if !component.signals.is_empty() {
        let entries: Vec<String> = component.signals.iter().map(|s| s.name.clone()).collect();
        output.push_str(&format!(
            "  {}._signals = {{ {} }};\n",
            root_var,
            entries.join(", ")
        ));
    }

    output.push_str(&format!("  return {};\n", root_var));
    output.push_str("}\n");
    Ok(output)
}

// ---------------------------------------------------------------------------
// server codegen
// ---------------------------------------------------------------------------

fn generate_server(
    component: &ComponentFile,
    env: &EnvMap,
    rel: &Path,
    server_files: &std::collections::HashSet<std::path::PathBuf>,
) -> Result<String, String> {
    let component_name = component
        .exports
        .first()
        .map(|e| e.name.as_str())
        .ok_or_else(|| "No exported component found".to_string())?;

    let render_tree = component
        .render_tree
        .as_ref()
        .ok_or_else(|| "No render tree found in component".to_string())?;

    let has_data = component.has_data_call;
    let hydrates = collect_hydrate_roots(render_tree);

    // A server page may declare `const head = '...'` (raw HTML injected into the
    // built page's <head>). Codegen includes it in the return object as `head`.
    let has_head = component.derived_consts.iter().any(|line| {
        let t = line.trim_start();
        t.strip_prefix("const")
            .map(|rest| rest.trim_start().split([' ', '=']).next().unwrap_or("") == "head")
            .unwrap_or(false)
    });

    let mut output = String::new();

    let comp_imports = collect_component_imports(render_tree, &component.imports);
    for (name, source) in &comp_imports {
        output.push_str(&format!(
            "import {{ {} }} from '{}';\n",
            name,
            rewrite_server_import(rel, source, server_files)
        ));
    }
    if !comp_imports.is_empty() {
        output.push('\n');
    }

    if has_data {
        output.push_str("import { data } from '@marisjs/runtime';\n\n");
    }

    let mut referenced_keys = component.env_call_keys.clone();
    if component.has_session_call {
        // The session block reads env('SESSION_SECRET') and env('NODE_ENV');
        // make sure both are baked even when the file never calls env()
        // directly.
        for key in ["SESSION_SECRET", "NODE_ENV"] {
            if !referenced_keys.iter().any(|k| k == key) {
                referenced_keys.push(key.to_string());
            }
        }
    }
    if component.has_env_call || component.has_session_call {
        output.push_str(&emit_env_helper(env, &referenced_keys));
        output.push('\n');
    }

    if component.has_session_call {
        // Server pages prerender at build time — there is no request, so
        // session() degrades to null and setSession() is a no-op unless a
        // Response is passed in. The block is emitted so the file compiles
        // and the boundary rule (CLIENT_SESSION_ACCESS) stays symmetric.
        output.push_str(&emit_session_block());
        output.push('\n');
    }

    if tree_has_style_expr(render_tree) {
        output.push_str("import { styleString } from '@marisjs/runtime';\n\n");
    }

    for mc in &component.module_consts {
        for line in mc.lines() {
            output.push_str(line);
            output.push('\n');
        }
    }
    if !component.module_consts.is_empty() {
        output.push('\n');
    }

    let fn_prefix = if has_data { "async " } else { "" };
    output.push_str(&format!(
        "export {}function {}(props) {{\n",
        fn_prefix, component_name
    ));

    for sig in &component.signals {
        let fn_name = match sig.kind {
            SignalKind::Signal => "signal",
            SignalKind::Computed => "computed",
        };
        output.push_str(&format!(
            "  const {} = {}({});\n",
            sig.name, fn_name, sig.initial_value
        ));
    }

    for dc in &component.derived_consts {
        for line in dc.lines() {
            output.push_str(&format!("  {}\n", line));
        }
    }

    if has_data {
        output.push_str("  // data() resolution (v1: direct await)\n");
    }

    if hydrates.is_empty() && !has_head {
        output.push_str("  return ");
        gen_html_node(render_tree, &mut output, has_data)?;
        output.push_str(";\n");
    } else {
        output.push_str("  const _html = ");
        gen_html_node(render_tree, &mut output, has_data)?;
        output.push_str(";\n");
        output.push_str("  return { html: _html");
        if has_head {
            output.push_str(", head: head");
        }
        output.push_str(", clientBundles: [");
        let names: Vec<String> = hydrates.iter().map(|h| format!("'./{}.js'", h)).collect();
        output.push_str(&names.join(", "));
        output.push_str("] };\n");
    }

    output.push_str("}\n");
    Ok(output)
}

pub fn collect_hydrate_roots(node: &JsxNode) -> Vec<String> {
    let mut names = Vec::new();
    collect_hydrate(node, &mut names);
    names
}

pub fn collect_child_component_tags(node: &JsxNode) -> Vec<String> {
    let mut names = Vec::new();
    collect_tags(node, &mut names);
    names
}

fn collect_hydrate(node: &JsxNode, names: &mut Vec<String>) {
    match node {
        JsxNode::Element { tag, is_hydrate_root, children, .. } => {
            // Dedupe by component name: a page using the same island twice
            // must import it ONCE (a duplicate import is a SyntaxError) and
            // mount every instance via querySelectorAll.
            if *is_hydrate_root && !names.contains(tag) {
                names.push(tag.clone());
            }
            for child in children { collect_hydrate(child, names); }
        }
        JsxNode::Conditional { cons, alt, .. } => {
            collect_hydrate(cons, names);
            collect_hydrate(alt, names);
        }
        JsxNode::ForEach { body, .. } => { collect_hydrate(body, names); }
        _ => {}
    }
}

fn gen_html_node(node: &JsxNode, output: &mut String, parent_is_async: bool) -> Result<(), String> {
    match node {
        JsxNode::Text(text) => {
            output.push_str(&format!("'{}'", html_escape(text)));
        }
        JsxNode::Expr(expr) => {
            // Parens are REQUIRED: the expression lands in the middle of a
            // string-concat chain, and a binary expression (env('K') ??
            // 'fallback', a || b, …) would otherwise bind looser than + and
            // silently change meaning (`'x' + a ?? b + 'y'` ≠ `'x' + (a ??
            // b) + 'y'`). Genuinely additive exprs are unaffected by the wrap.
            output.push_str(&format!("({})", expr));
        }
        JsxNode::Element { tag, attrs, children, is_hydrate_root, is_component } => {
            if *is_hydrate_root {
                // Emit the REAL props object (computed at SSR render time, so
                // dynamic values work) into the placeholder. The client-side
                // mount call reads them back via dataset.props.
                let props_arg = build_props_object(attrs)?;
                output.push_str(&format!(
                    "'<div data-hydrate=\"{}\" data-props=\\'' + JSON.stringify({}) + '\\'></div>'",
                    tag, props_arg
                ));
                return Ok(());
            }
            if *is_component {
                let props_arg = build_props_object(attrs)?;
                let await_kw = if parent_is_async { "await " } else { "" };
                output.push_str(&format!("({}{}({}))", await_kw, tag, props_arg));
                return Ok(());
            }
            if tag.is_empty() {
                output.push('(');
                let mut first = true;
                for child in children {
                    if !first { output.push_str(" + "); }
                    first = false;
                    gen_html_node(child, output, parent_is_async)?;
                }
                output.push(')');
                return Ok(());
            }
            let open_js = gen_open_tag_js(tag, attrs);
            let close = format!("</{}>", tag);
            output.push_str(&format!("({}", open_js));
            for child in children {
                output.push_str(" + ");
                gen_html_node(child, output, parent_is_async)?;
            }
            output.push_str(&format!(" + '{}')", html_escape(&close)));
        }
        JsxNode::Conditional { test, cons, alt } => {
            output.push_str(&format!("({} ? ", test));
            gen_html_node(cons, output, parent_is_async)?;
            output.push_str(" : ");
            gen_html_node(alt, output, parent_is_async)?;
            output.push(')');
        }
        JsxNode::ForEach { each, key_fn: _, item_param, body, .. } => {
            output.push_str(&format!("({}.map(({}) => ", each, item_param));
            gen_html_node(body, output, parent_is_async)?;
            output.push_str(").join(''))");
        }
    }
    Ok(())
}

/// Builds a JS expression that produces the opening tag string for a server
/// (html-string) render. Static attribute values are embedded (entity-escaped
/// at compile time); EXPRESSION values are EVALUATED at render time, matching
/// the client codegen's behavior (which calls setAttribute with the value).
/// Boolean attributes use presence semantics, mirroring the client's
/// setAttribute/removeAttribute pair.
fn gen_open_tag_js(tag: &str, attrs: &[parser::JsxAttr]) -> String {
    let mut toks: Vec<String> = Vec::new();
    let mut static_acc = format!("<{}", tag);
    for attr in attrs {
        if is_event_attr(&attr.name).is_some() {
            continue;
        }
        match &attr.value {
            JsxAttrValue::String(value) => {
                static_acc.push_str(&format!(" {}=\"{}\"", attr.name, html_attr_escape(value)));
            }
            JsxAttrValue::Expr(expr) => {
                if !static_acc.is_empty() {
                    toks.push(format!("'{}'", html_escape(&static_acc)));
                    static_acc.clear();
                }
                if is_boolean_attr(&attr.name) {
                    toks.push(format!("({} ? \" {}=\\\"\\\"\" : \"\")", expr, attr.name));
                } else if attr.name == "style" {
                    // style={expr} must evaluate to a CSS string — serialize
                    // objects at render time (same styleString as the client).
                    toks.push(format!("\" {}=\\\"\" + styleString({}) + \"\\\"\"", attr.name, expr));
                } else {
                    toks.push(format!("\" {}=\\\"\" + ({}) + \"\\\"\"", attr.name, expr));
                }
            }
        }
    }
    static_acc.push('>');
    if !static_acc.is_empty() {
        toks.push(format!("'{}'", html_escape(&static_acc)));
    }
    if toks.is_empty() {
        "'<>'".to_string()
    } else {
        format!("({})", toks.join(" + "))
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
        .replace('\'', "&#39;").replace('"', "&quot;")
}

fn html_attr_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;")
        .replace('\'', "&#39;").replace('<', "&lt;").replace('>', "&gt;")
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

struct AtomicCounter { n: u32 }
impl AtomicCounter {
    fn new() -> Self { Self { n: 0 } }
    fn next(&mut self) -> u32 { let v = self.n; self.n += 1; v }
}

fn has_for_each_or_conditional(node: &JsxNode) -> bool {
    match node {
        JsxNode::Conditional { .. } | JsxNode::ForEach { .. } => true,
        JsxNode::Element { children, .. } => children.iter().any(has_for_each_or_conditional),
        _ => false,
    }
}

/// True when the tree contains a DOM element with a style EXPRESSION attribute
/// (`style={...}`), i.e. one that needs the runtime styleString serializer.
/// String style attrs and component style PROPS are untouched by this.
fn tree_has_style_expr(node: &JsxNode) -> bool {
    match node {
        JsxNode::Element { attrs, children, is_component, is_hydrate_root, .. } => {
            if !is_component
                && !is_hydrate_root
                && attrs.iter().any(|a| {
                    a.name == "style" && matches!(a.value, JsxAttrValue::Expr(_))
                })
            {
                return true;
            }
            children.iter().any(tree_has_style_expr)
        }
        JsxNode::Conditional { cons, alt, .. } => {
            tree_has_style_expr(cons) || tree_has_style_expr(alt)
        }
        JsxNode::ForEach { body, .. } => tree_has_style_expr(body),
        _ => false,
    }
}

fn tree_reads_signal(node: &JsxNode, signal_names: &[String], props_param: &str) -> bool {
    match node {
        JsxNode::Expr(text) => is_reactive_expr(text, signal_names, props_param),
        JsxNode::Conditional { test, .. } => is_reactive_expr(test, signal_names, props_param),
        JsxNode::ForEach { each, .. } => is_reactive_expr(each, signal_names, props_param),
        JsxNode::Element { children, attrs, .. } => {
            children.iter().any(|c| tree_reads_signal(c, signal_names, props_param))
                || attrs.iter().any(|a| match &a.value {
                    JsxAttrValue::Expr(text) => is_reactive_expr(text, signal_names, props_param),
                    _ => false,
                })
        }
        _ => false,
    }
}

/// AST-based reactivity check: reads of known signal/computed `.value`
/// properties (or props-drilled signals). See parser::expr_reads_signal_value
/// for the exact semantics — no more text substring matching, so a plain
/// object with an unrelated `.value` field is not reactive.
fn is_reactive_expr(expr: &str, signal_names: &[String], props_param: &str) -> bool {
    parser::expr_reads_signal_value(expr, signal_names, props_param)
}

fn is_signal_name(expr: &str, signal_names: &[String]) -> bool {
    signal_names.iter().any(|n| n == expr)
}

fn is_unwrapped_signal(expr: &str, signal_names: &[String]) -> bool {
    for name in signal_names {
        if expr == format!("{}.value", name) { return true; }
    }
    false
}

fn is_event_attr(name: &str) -> Option<String> {
    if let Some(rest) = name.strip_prefix("on") {
        if rest.starts_with(|c: char| c.is_uppercase()) {
            return Some(rest.to_lowercase());
        }
    }
    None
}

fn is_boolean_attr(name: &str) -> bool {
    matches!(name, "disabled" | "checked" | "selected" | "readonly" | "required"
        | "hidden" | "multiple" | "autofocus" | "autoplay" | "controls"
        | "loop" | "muted" | "novalidate" | "formnovalidate" | "async"
        | "defer" | "ismap" | "nomodule" | "open" | "reversed" | "default"
        | "itemscope" | "typemustmatch")
}

fn needs_property_assignment(name: &str) -> bool {
    matches!(name, "value")
}

fn is_reactive_derived_const(dc: &str, signal_names: &[String]) -> Option<String> {
    if !dc.starts_with("const ") { return None; }
    let rest = dc.strip_prefix("const ")?;
    let eq_pos = rest.find('=')?;
    let var_name = rest[..eq_pos].trim();
    if var_name.is_empty() || var_name.contains(' ') { return None; }
    let init = &rest[eq_pos + 1..];
    for sig in signal_names {
        if init.contains(&format!("{}.value", sig)) {
            return Some(var_name.to_string());
        }
    }
    None
}

fn extract_const_init(dc: &str) -> String {
    if let Some(rest) = dc.strip_prefix("const ") {
        if let Some(eq) = rest.find('=') {
            return rest[eq + 1..].trim().trim_end_matches(';').to_string();
        }
    }
    dc.to_string()
}

fn writeln(output: &mut String, indent: usize, s: &str) {
    let pad = "  ".repeat(indent);
    output.push_str(&pad);
    output.push_str(s);
    output.push('\n');
}

fn collect_component_imports(
    node: &JsxNode,
    file_imports: &[parser::ImportInfo],
) -> Vec<(String, String)> {
    let names = collect_component_tags(node);
    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in names {
        if seen.contains(&name) { continue; }
        seen.insert(name.clone());
        if let Some(source) = resolve_import_source(&name, file_imports) {
            result.push((name, source));
        }
    }
    result
}

fn collect_component_tags(node: &JsxNode) -> Vec<String> {
    let mut names = Vec::new();
    collect_tags(node, &mut names);
    names
}

fn collect_tags(node: &JsxNode, names: &mut Vec<String>) {
    match node {
        JsxNode::Element { tag, is_component, children, .. } => {
            if *is_component { names.push(tag.clone()); }
            for child in children { collect_tags(child, names); }
        }
        JsxNode::Conditional { cons, alt, .. } => {
            collect_tags(cons, names);
            collect_tags(alt, names);
        }
        JsxNode::ForEach { body, .. } => { collect_tags(body, names); }
        _ => {}
    }
}

fn resolve_import_source(name: &str, file_imports: &[parser::ImportInfo]) -> Option<String> {
    for imp in file_imports {
        if imp.imported_names.contains(&name.to_string()) {
            let s = imp.source.trim_end_matches(".tsx").trim_end_matches(".ts");
            return Some(format!("{}.mjs", s));
        }
    }
    None
}

/// E4-01: server modules live under `_server/<rel>`; client modules stay at
/// `<rel>`. A server module importing another server module keeps its
/// relative specifier (both moved, resolution unchanged). A server module
/// importing a CLIENT module (a hydrate island) must walk back up to the
/// public tree, because the old relative path from `_server/…` would land
/// inside `_server/`. Bare specifiers (@marisjs/runtime, packages) are
/// untouched — they resolve via node_modules from anywhere.
fn rewrite_server_import(
    rel: &Path,
    source: &str,
    server_files: &std::collections::HashSet<std::path::PathBuf>,
) -> String {
    if !source.starts_with("./") && !source.starts_with("../") {
        return source.to_string();
    }
    if rel.as_os_str().is_empty() {
        return source.to_string();
    }

    let imported_rel = normalize_join(rel.parent().unwrap_or(Path::new("")), source);
    let imported_source = imported_rel.with_extension("tsx");
    if server_files.contains(&imported_source) {
        return source.to_string();
    }

    // Client module — resolve a relative path from the server module's
    // directory (inside _server/) up to the root, then down to the public
    // module path. The _server/ prefix adds one extra level.
    let depth = rel
        .parent()
        .map(|p| p.components().count() + 1)
        .unwrap_or(1);
    let up = "../".repeat(depth);
    let target = imported_rel.to_string_lossy().trim_end_matches(".mjs").to_string();
    format!("{}{}.mjs", up, target)
}

/// Lexically join `dir` and `specifier` (a relative `./`/`../` path), resolving
/// `.` and `..` without touching the filesystem.
fn normalize_join(dir: &Path, specifier: &str) -> PathBuf {
    let mut parts: Vec<std::ffi::OsString> = dir
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(p) => Some(p.to_os_string()),
            _ => None,
        })
        .collect();
    for part in specifier.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(std::ffi::OsString::from(p)),
        }
    }
    parts.into_iter().collect()
}

// ---------------------------------------------------------------------------
// props object builder (shared by client and server)
// ---------------------------------------------------------------------------

fn build_props_object(attrs: &[parser::JsxAttr]) -> Result<String, String> {
    let pairs: Vec<String> = attrs.iter().map(|a| {
        match &a.value {
            JsxAttrValue::Expr(expr) => Ok(format!("{}: {}", a.name, expr)),
            JsxAttrValue::String(s) => {
                let esc = s.replace('\\', "\\\\").replace('\'', "\\'");
                Ok(format!("{}: '{}'", a.name, esc))
            }
        }
    }).collect::<Result<Vec<String>, String>>()?;

    Ok(if pairs.is_empty() {
        "{}".to_string()
    } else {
        format!("{{ {} }}", pairs.join(", "))
    })
}

// ---------------------------------------------------------------------------
// client node generation — all functions return Result
// ---------------------------------------------------------------------------

fn gen_node(
    node: &JsxNode,
    output: &mut String,
    counter: &mut AtomicCounter,
    indent: usize,
    signal_names: &[String],
    props_param: &str,
) -> Result<String, String> {
    match node {
        JsxNode::Conditional { test, cons, alt } =>
            gen_conditional(test, cons, alt, output, counter, indent, signal_names, props_param),
        JsxNode::ForEach { each, key_fn, item_param, body, for_body_decls } =>
            gen_for_each(each, key_fn, item_param, body, for_body_decls, output, counter, indent, signal_names, props_param),
        JsxNode::Element { tag, attrs, children, is_hydrate_root, is_component } =>
            gen_element(tag, attrs, children, *is_hydrate_root, *is_component, output, counter, indent, signal_names, props_param),
        JsxNode::Text(text) => Ok(gen_text(text, output, counter, indent)),
        JsxNode::Expr(expr) => Ok(gen_expr(expr, output, counter, indent, signal_names, props_param)),
    }
}

fn gen_element(
    tag: &str,
    attrs: &[parser::JsxAttr],
    children: &[JsxNode],
    _is_hydrate_root: bool,
    is_component: bool,
    output: &mut String,
    counter: &mut AtomicCounter,
    indent: usize,
    signal_names: &[String],
    props_param: &str,
) -> Result<String, String> {
    if is_component {
        return gen_component_call(tag, attrs, output, counter, indent, signal_names);
    }
    let var = format!("el{}", counter.next());

    if tag.is_empty() {
        if children.len() == 1 {
            return gen_node(&children[0], output, counter, indent, signal_names, props_param);
        }
        writeln(output, indent, &format!("const {} = document.createDocumentFragment();", var));
        for child in children {
            let cv = gen_node(child, output, counter, indent, signal_names, props_param)?;
            writeln(output, indent, &format!("{}.appendChild({});", var, cv));
        }
        return Ok(var);
    }

    writeln(output, indent, &format!("const {} = document.createElement('{}');", var, tag));

    for attr in attrs {
        if let Some(event_name) = is_event_attr(&attr.name) {
            if let JsxAttrValue::Expr(handler) = &attr.value {
                writeln(output, indent, &format!("{}.addEventListener('{}', {});", var, event_name, handler));
            }
            continue;
        }
        match &attr.value {
            JsxAttrValue::String(value) => {
                let escaped = value.replace('\\', "\\\\").replace('\'', "\\'");
                writeln(output, indent, &format!("{}.setAttribute('{}', '{}');", var, attr.name, escaped));
            }
            JsxAttrValue::Expr(expr) => {
                if is_boolean_attr(&attr.name) {
                    let set_op = format!("{}.setAttribute('{}', '')", var, attr.name);
                    let remove_op = format!("{}.removeAttribute('{}')", var, attr.name);
                    if is_reactive_expr(expr, signal_names, props_param) {
                        writeln(output, indent, &format!(
                            "bind(() => {{ if ({}) {{ {}; }} else {{ {}; }} }});",
                            expr, set_op, remove_op
                        ));
                    } else {
                        writeln(output, indent, &format!(
                            "if ({}) {{ {}; }} else {{ {}; }}",
                            expr, set_op, remove_op
                        ));
                    }
                } else {
                    // style={{...}} objects and style={expr} must serialize to a
                    // CSS string at runtime — an object coerced by setAttribute
                    // renders "[object Object]". styleString handles object
                    // literals, computed() results, and string values (passthrough).
                    let dom_op = if attr.name == "style" {
                        let op_value = format!("styleString({})", expr);
                        if needs_property_assignment(&attr.name) {
                            format!("{}.{} = {}", var, attr.name, op_value)
                        } else {
                            format!("{}.setAttribute('{}', {})", var, attr.name, op_value)
                        }
                    } else if needs_property_assignment(&attr.name) {
                        format!("{}.{} = {}", var, attr.name, expr)
                    } else {
                        format!("{}.setAttribute('{}', {})", var, attr.name, expr)
                    };
                    if is_reactive_expr(expr, signal_names, props_param) {
                        writeln(output, indent, &format!("bind(() => {{ {}; }});", dom_op));
                    } else {
                        writeln(output, indent, &format!("{};", dom_op));
                    }
                }
            }
        }
    }

    let has_element = children.iter().any(|c| matches!(
        c, JsxNode::Element { .. } | JsxNode::Conditional { .. } | JsxNode::ForEach { .. }
    ));

    if children.len() == 1 && !has_element {
        match &children[0] {
            JsxNode::Text(text) => {
                let escaped = text.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
                writeln(output, indent, &format!("{}.textContent = '{}';", var, escaped));
            }
            JsxNode::Expr(expr) => {
                let dom_op = format!("{}.textContent = {}", var, expr);
                if is_reactive_expr(expr, signal_names, props_param) {
                    writeln(output, indent, &format!("bind(() => {{ {}; }});", dom_op));
                } else {
                    writeln(output, indent, &format!("{};", dom_op));
                }
            }
            _ => {}
        }
    } else if !children.is_empty() {
        for child in children {
            match child {
                JsxNode::Element { .. } | JsxNode::Conditional { .. } | JsxNode::ForEach { .. } => {
                    let cv = gen_node(child, output, counter, indent, signal_names, props_param)?;
                    writeln(output, indent, &format!("{}.appendChild({});", var, cv));
                }
                JsxNode::Text(text) => {
                    let tv = gen_text(text, output, counter, indent);
                    writeln(output, indent, &format!("{}.appendChild({});", var, tv));
                }
                JsxNode::Expr(expr) => {
                    let tv = gen_expr_text_node(expr, output, counter, indent, signal_names, props_param);
                    writeln(output, indent, &format!("{}.appendChild({});", var, tv));
                }
            }
        }
    }

    Ok(var)
}

fn gen_text(text: &str, output: &mut String, counter: &mut AtomicCounter, indent: usize) -> String {
    let var = format!("txt{}", counter.next());
    let escaped = text.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n");
    writeln(output, indent, &format!("const {} = document.createTextNode('{}');", var, escaped));
    var
}

fn gen_expr(expr: &str, output: &mut String, counter: &mut AtomicCounter, indent: usize, signal_names: &[String], props_param: &str) -> String {
    let var = format!("txt{}", counter.next());
    writeln(output, indent, &format!("const {} = document.createTextNode({});", var, expr));
    if is_reactive_expr(expr, signal_names, props_param) {
        writeln(output, indent, &format!("bind(() => {{ {}.nodeValue = {}; }});", var, expr));
    }
    var
}

fn gen_expr_text_node(expr: &str, output: &mut String, counter: &mut AtomicCounter, indent: usize, signal_names: &[String], props_param: &str) -> String {
    gen_expr(expr, output, counter, indent, signal_names, props_param)
}

// ---------------------------------------------------------------------------
// component call — one-shot, no bind/replaceWith. Signal references pass
// through; .value unwrapping at the call site is a hard error.
// ---------------------------------------------------------------------------

fn gen_component_call(
    tag: &str,
    attrs: &[parser::JsxAttr],
    output: &mut String,
    counter: &mut AtomicCounter,
    indent: usize,
    signal_names: &[String],
) -> Result<String, String> {
    let pairs: Vec<String> = attrs.iter().map(|a| {
        match &a.value {
            JsxAttrValue::Expr(expr) => {
                if is_unwrapped_signal(expr, signal_names) {
                    // e.g. label={label.value} — forbidden per spec §6a
                    let sig_name = expr.trim_end_matches(".value");
                    Err(format!(
                        "PROP_UNWRAPPED_SIGNAL: attribute '{}={}' on <{}> unwraps signal '{}' at the call site. Pass the signal by reference: <{} {}={{{}}}> and read .value inside the child component's bind().",
                        a.name, expr, tag, sig_name, tag, a.name, sig_name
                    ))
                } else if is_signal_name(expr, signal_names) {
                    // e.g. label={label} — pass the signal object by reference
                    Ok(format!("{}: {}", a.name, expr))
                } else {
                    Ok(format!("{}: {}", a.name, expr))
                }
            }
            JsxAttrValue::String(s) => {
                let esc = s.replace('\\', "\\\\").replace('\'', "\\'");
                Ok(format!("{}: '{}'", a.name, esc))
            }
        }
    }).collect::<Result<Vec<String>, String>>()?;

    let props_arg = if pairs.is_empty() {
        "{}".to_string()
    } else {
        format!("{{ {} }}", pairs.join(", "))
    };

    let var = format!("el{}", counter.next());
    writeln(output, indent, &format!("const {} = {}({});", var, tag, props_arg));
    Ok(var)
}

// ---------------------------------------------------------------------------
// conditional
// ---------------------------------------------------------------------------

fn gen_conditional(
    test: &str,
    cons: &JsxNode,
    alt: &JsxNode,
    output: &mut String,
    counter: &mut AtomicCounter,
    indent: usize,
    signal_names: &[String],
    props_param: &str,
) -> Result<String, String> {
    let anchor_var = format!("_an{}", counter.next());
    let frag_var = format!("_fr{}", counter.next());
    let curr_var = format!("_cu{}", counter.next());

    let true_var = gen_node(cons, output, counter, indent + 1, signal_names, props_param)?;
    let false_var = gen_node(alt, output, counter, indent + 1, signal_names, props_param)?;

    writeln(output, indent, &format!("const {} = document.createComment('');", anchor_var));
    writeln(output, indent, &format!("const {} = document.createDocumentFragment();", frag_var));
    writeln(output, indent, &format!("let {} = {} ? {} : {};", curr_var, test, true_var, false_var));
    writeln(output, indent, &format!("{}.appendChild({});", frag_var, curr_var));
    writeln(output, indent, &format!("{}.appendChild({});", frag_var, anchor_var));
    writeln(output, indent, &format!(
        "bind(() => {{ const _nx = {} ? {} : {}; if (_nx !== {}) {{ {}.remove(); {}.parentNode.insertBefore(_nx, {}); {} = _nx; }} }});",
        test, true_var, false_var, curr_var, curr_var, anchor_var, anchor_var, curr_var
    ));

    Ok(frag_var)
}

// ---------------------------------------------------------------------------
// for-each
// ---------------------------------------------------------------------------

fn gen_for_each(
    each: &str,
    key_fn: &str,
    item_param: &str,
    body: &JsxNode,
    for_body_decls: &[String],
    output: &mut String,
    counter: &mut AtomicCounter,
    indent: usize,
    signal_names: &[String],
    props_param: &str,
) -> Result<String, String> {
    let anchor_var = format!("_fa{}", counter.next());
    let frag_var = format!("_ff{}", counter.next());
    let map_var = format!("_fm{}", counter.next());
    let render_fn = format!("_r{}", counter.next());

    writeln(output, indent, &format!("const {} = document.createComment('');", anchor_var));
    writeln(output, indent, &format!("const {} = document.createDocumentFragment();", frag_var));
    writeln(output, indent, &format!("{}.appendChild({});", frag_var, anchor_var));
    writeln(output, indent, &format!("const {} = {{}};", map_var));

    writeln(output, indent, &format!("function {}({}) {{", render_fn, item_param));
    let render_indent = indent + 1;
    for decl in for_body_decls {
        for line in decl.lines() {
            writeln(output, render_indent, line);
        }
    }
    let body_var = gen_node(body, output, counter, render_indent, signal_names, props_param)?;
    writeln(output, render_indent, &format!("return {};", body_var));
    writeln(output, indent, "}");

    writeln(output, indent, "bind(() => {");
    let bi = indent + 1;
    writeln(output, bi, &format!("const _list = {};", each));
    writeln(output, bi, "const _seen = {};");
    writeln(output, bi, "const _order = [];");
    writeln(output, bi, "for (let _i = 0; _i < _list.length; _i++) {");
    writeln(output, bi + 1, &format!("const _key = ({}) (_list[_i]);", key_fn));
    writeln(output, bi + 1, "_seen[_key] = _list[_i];");
    writeln(output, bi + 1, "_order.push(_key);");
    writeln(output, bi, "}");
    writeln(output, bi, &format!("for (const _k in {}) {{", map_var));
    writeln(output, bi + 1, "if (!(_k in _seen)) {");
    writeln(output, bi + 2, &format!("{}[_k].remove();", map_var));
    writeln(output, bi + 2, &format!("delete {}[_k];", map_var));
    writeln(output, bi + 1, "}");
    writeln(output, bi, "}");
    writeln(output, bi, &format!("const _parent = {}.parentNode || {};", anchor_var, frag_var));
    writeln(output, bi, &format!("let _ref = {};", anchor_var));
    writeln(output, bi, "for (let _i = _order.length - 1; _i >= 0; _i--) {");
    writeln(output, bi + 1, "const _key = _order[_i];");
    writeln(output, bi + 1, &format!("if (!{}[_key]) {{", map_var));
    writeln(output, bi + 2, &format!("{}[_key] = {}(_seen[_key]);", map_var, render_fn));
    writeln(output, bi + 1, "}");
    writeln(output, bi + 1, &format!("const _n = {}[_key];", map_var));
    writeln(output, bi + 1, "if (_n.nextSibling !== _ref) {");
    writeln(output, bi + 2, "_parent.insertBefore(_n, _ref);");
    writeln(output, bi + 1, "}");
    writeln(output, bi + 1, "_ref = _n;");
    writeln(output, bi, "}");
    writeln(output, indent, "});");

    Ok(frag_var)
}
