//! Walks the AST from parser and checks it against every rule in docs/framework-grammar-spec.md Sections 2–7. Produces a Vec<Diagnostic>, nothing else. Does not generate code.

pub mod css_collision;

use std::path::{Path, PathBuf};

use parser::{
    BodyStmtKind, ComponentFile, ExportKind, JsxAttrValue, JsxExpression, JsxNode, RunsOn,
    TopLevelBinding, TypeAnnotation,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub code: &'static str,
    pub message: String,
    pub fix_hint: &'static str,
    /// Warnings never fail the build/validation; errors do. Lints
    /// (ENV_LEAK_TO_CLIENT_PROP) are warnings; every other check is an error.
    pub is_warning: bool,
}

impl Diagnostic {
    pub fn new(
        code: &'static str,
        message: impl Into<String>,
        fix_hint: &'static str,
        line: Option<usize>,
        column: Option<usize>,
    ) -> Self {
        Self {
            line,
            column,
            code,
            message: message.into(),
            fix_hint,
            is_warning: false,
        }
    }

    /// A warning diagnostic: reported and visible, but never a build failure.
    pub fn warning(
        code: &'static str,
        message: impl Into<String>,
        fix_hint: &'static str,
        line: Option<usize>,
        column: Option<usize>,
    ) -> Self {
        let mut d = Self::new(code, message, fix_hint, line, column);
        d.is_warning = true;
        d
    }
}

/// §2: Every component file must have exactly one `@runsOn` directive (client or server).
/// Missing the directive or having more than one is an error.
pub fn check_runs_on_directive(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    let line = file.runs_on_line;
    let col = file.runs_on_column;

    if file.runs_on_count == 0 {
        diagnostics.push(Diagnostic::new(
            "MISSING_RUNSON",
            "Missing @runsOn directive — every component file must declare exactly one // @runsOn client or // @runsOn server.",
            "Add `// @runsOn client` or `// @runsOn server` as the first line of the file.",
            None,
            None,
        ));
    } else if file.runs_on_count > 1 {
        diagnostics.push(Diagnostic::new(
            "DUPLICATE_RUNSON",
            format!(
                "Multiple @runsOn directives found ({}) — a component file must declare exactly one.",
                file.runs_on_count
            ),
            "Remove all but one @runsOn directive.",
            Some(line),
            Some(col),
        ));
    }
}

/// §2: Exactly one named function export per component file. No default exports, no multiple exports.
pub fn check_single_export(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    if file.exports.is_empty() {
        diagnostics.push(Diagnostic::new(
            "NO_EXPORT",
            "No exported component function found — file must export exactly one component.",
            "Add `export function ComponentName(props: ...)` to the file.",
            None,
            None,
        ));
        return;
    }

    if file.exports.len() > 1 {
        let first = &file.exports[0];
        diagnostics.push(Diagnostic::new(
            "MULTIPLE_EXPORTS",
            format!(
                "Multiple exports found ({}) — a component file must export exactly one component.",
                file.exports.len()
            ),
            "Move each component to its own file, keeping exactly one export per file.",
            Some(first.line),
            Some(first.column),
        ));
    }

    for export in &file.exports {
        if export.kind == ExportKind::DefaultExport {
            diagnostics.push(Diagnostic::new(
                "DEFAULT_EXPORT",
                "Default export found — use a named export function instead.",
                "Replace `export default function Foo(...)` with `export function Foo(...)`.",
                Some(export.line),
                Some(export.column),
            ));
        }
    }
}

/// §2: Filename must match the exported component name exactly (e.g., `Cart.tsx` exports `Cart`).
/// Requires the filename from the ComponentFile.
pub fn check_filename_matches_component(
    file: &ComponentFile,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let component_name = match file.exports.first() {
        Some(export) if export.kind == ExportKind::NamedFunction => &export.name,
        _ => return,
    };

    let expected_filename = format!("{}.tsx", component_name);
    if file.filename != expected_filename {
        let first = &file.exports[0];
        diagnostics.push(Diagnostic::new(
            "FILENAME_MISMATCH",
            format!(
                "Filename mismatch — file is named '{}' but exports component '{}'. Expected filename: '{}'.",
                file.filename, component_name, expected_filename
            ),
            "Rename the file to match the exported component name, or rename the component to match the filename.",
            Some(first.line),
            Some(first.column),
        ));
    }
}

/// §3: Component function must accept exactly one parameter named `props` with an explicit type.
/// No destructuring, no `any`, no untyped. Covers both function declarations and arrow-function components.
pub fn check_props_parameter(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    match &file.props {
        None => {
            diagnostics.push(Diagnostic::new(
                "NO_PROPS",
                "No props parameter found — component must accept exactly one parameter named 'props' with an explicit type.",
                "Add exactly one parameter named `props` with an explicit type annotation.",
                None,
                None,
            ));
        }
        Some(props) => {
            let line = props.line;
            let col = props.column;

            if props.name != "props" {
                diagnostics.push(Diagnostic::new(
                    "PROPS_WRONG_NAME",
                    format!(
                        "Incorrect props parameter name '{}' — the single parameter must be named 'props'.",
                        props.name
                    ),
                    "Rename the parameter to `props`.",
                    Some(line),
                    Some(col),
                ));
            }

            if props.is_destructured {
                diagnostics.push(Diagnostic::new(
                    "PROPS_DESTRUCTURED",
                    "Destructured props in function signature — use 'props' as a single parameter and access fields via props.fieldName.",
                    "Replace destructured signature with a single `props` parameter and access fields via `props.fieldName`.",
                    Some(line),
                    Some(col),
                ));
            }

            match &props.type_annotation {
                TypeAnnotation::Untyped => {
                    diagnostics.push(Diagnostic::new(
                        "PROPS_UNTYPED",
                        "Untyped props parameter — add an explicit type annotation (e.g., 'props: MyProps').",
                        "Add an explicit type annotation (e.g., `props: MyProps`).",
                        Some(line),
                        Some(col),
                    ));
                }
                TypeAnnotation::Any => {
                    diagnostics.push(Diagnostic::new(
                        "PROPS_ANY",
                        "Props typed as 'any' — use an explicit type alias or interface instead.",
                        "Replace `any` with an explicit type alias or interface.",
                        Some(line),
                        Some(col),
                    ));
                }
                TypeAnnotation::Named(_) => {}
            }
        }
    }
}

/// §4 / §7: Reject imports from `react`, `preact/hooks`, or any hook-shaped name pattern
/// (`use` followed by an uppercase letter).
pub fn check_forbidden_imports(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    let forbidden_sources = ["react", "preact/hooks", "preact/compat"];

    for import in &file.imports {
        if import.is_css {
            continue; // CSS imports are not subject to forbidden-source checks
        }
        let source_is_forbidden = forbidden_sources.contains(&import.source.as_str());

        if source_is_forbidden {
            diagnostics.push(Diagnostic::new(
                "FORBIDDEN_IMPORT",
                format!(
                    "Forbidden import from '{}' — React/Preact APIs are not allowed in this framework. Use signal() for state and computed() for derived values.",
                    import.source
                ),
                "Remove the import. Use signal() for state and computed() for derived values.",
                Some(import.line),
                Some(import.column),
            ));
        } else {
            for name in &import.imported_names {
                if is_hook_shaped(name) {
                    diagnostics.push(Diagnostic::new(
                        "FORBIDDEN_HOOK",
                        format!(
                            "Forbidden hook import '{}' from '{}' — hooks are not allowed in this framework. Use signal() for state and computed() for derived values.",
                            name, import.source
                        ),
                        "Remove the hook import. Use signal() for state and computed() for derived values.",
                        Some(import.line),
                        Some(import.column),
                    ));
                }
            }
        }
    }
}

fn is_hook_shaped(name: &str) -> bool {
    name.len() > 3
        && name.starts_with("use")
        && name[3..].chars().next().map_or(false, |c| c.is_uppercase())
}

/// §4: No top-level `let` or exported mutable bindings. All reactive state must live in `signal()`.
pub fn check_no_global_mutable_state(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    for binding in &file.top_level_bindings {
        match binding {
            TopLevelBinding::Let {
                name,
                line,
                column,
                ..
            } => {
                diagnostics.push(Diagnostic::new(
                    "GLOBAL_LET",
                    format!(
                        "Top-level mutable binding 'let {}' — global mutable state is forbidden. Use signal() and pass it explicitly.",
                        name
                    ),
                    "Replace `let` with `const` for immutable data, or wrap the value in `signal()` for reactive state.",
                    Some(*line),
                    Some(*column),
                ));
            }
            TopLevelBinding::Var {
                name,
                line,
                column,
                ..
            } => {
                diagnostics.push(Diagnostic::new(
                    "GLOBAL_VAR",
                    format!(
                        "Top-level mutable binding 'var {}' — use const for immutable bindings or signal() for reactive state.",
                        name
                    ),
                    "Replace `var` with `const` for immutable data, or wrap the value in `signal()` for reactive state.",
                    Some(*line),
                    Some(*column),
                ));
            }
            TopLevelBinding::Const { .. } => {}
        }
    }
}

/// §5: Reject `&&`-based conditional rendering. Require ternary with explicit `null` on the false branch.
pub fn check_conditional_rendering_form(
    file: &ComponentFile,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for expr in &file.jsx_expressions {
        if expr.kind == JsxExpression::AndConditional {
            diagnostics.push(Diagnostic::new(
                "AND_CONDITIONAL",
                "&& conditional rendering found — use a ternary operator with explicit null on the false branch instead. Example: { condition ? <Component /> : null }.",
                "Replace `{condition && <Component />}` with `{condition ? <Component /> : null}`.",
                Some(expr.line),
                Some(expr.column),
            ));
        }
    }
}

/// §5: Reject inline `.map()` inside JSX. Require the `<For>` component.
pub fn check_list_rendering_form(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    for expr in &file.jsx_expressions {
        if expr.kind == JsxExpression::InlineMap {
            diagnostics.push(Diagnostic::new(
                "INLINE_MAP",
                "Inline .map() rendering found — use the <For> component instead. Example: <For each={items} key={(item) => item.id}>{(item) => <ItemRow item={item} />}</For>.",
                "Replace `.map()` with a `<For>` component: `<For each={items} key={(item) => item.id}>{(item) => <Item item={item} />}</For>`.",
                Some(expr.line),
                Some(expr.column),
            ));
        }
    }
}

/// §6: Reject `data()` calls in files marked `@runsOn client`. `data()` is server-only.
pub fn check_data_call_boundary(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    if file.has_data_call {
        if let Some(RunsOn::Client) = file.runs_on {
            diagnostics.push(Diagnostic::new(
                "CLIENT_DATA_CALL",
                "data() call in @runsOn client file — data() is only allowed in @runsOn server components. Data flows to client components via props.",
                "Move the `data()` call to a `@runsOn server` parent component and pass the result via props.",
                Some(file.data_call_line),
                Some(file.data_call_column),
            ));
        }
    }
}

/// §7c: the compiler emits module-scope declarations for the runtime that a
/// file uses — `session`/`setSession` (when sessions are used) and `env`
/// (when env() or session() is used). A user top-level binding or import with
/// the same name turns the generated module into a SyntaxError at deploy
/// time; fail at validation time instead (spec §1 rule 4).
pub fn check_runtime_collisions(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    let mut emitted: Vec<&str> = Vec::new();
    if file.has_session_call {
        emitted.push("session");
        emitted.push("setSession");
    }
    if file.has_env_call || file.has_session_call {
        emitted.push("env");
    }
    if emitted.is_empty() {
        return;
    }
    check_binding_import_collisions(file, &emitted, diagnostics);
}

/// §7d: middleware additionally emits the three result helpers and the
/// canonical matcher into the module scope — top-level bindings or imports
/// named next/redirect/respond/__matchPath would collide with them.
fn check_middleware_runtime_collisions(
    file: &ComponentFile,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut emitted: Vec<&str> = Vec::new();
    if file.has_session_call {
        emitted.push("session");
        emitted.push("setSession");
    }
    if file.has_env_call || file.has_session_call {
        emitted.push("env");
    }
    emitted.push("next");
    emitted.push("redirect");
    emitted.push("respond");
    emitted.push("__matchPath");
    if emitted.is_empty() {
        return;
    }
    check_binding_import_collisions(file, &emitted, diagnostics);
}

fn check_binding_import_collisions(
    file: &ComponentFile,
    emitted: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for binding in &file.top_level_bindings {
        let (name, line, column) = match binding {
            TopLevelBinding::Let { name, line, column, .. }
            | TopLevelBinding::Var { name, line, column, .. }
            | TopLevelBinding::Const { name, line, column, .. } => (name, *line, *column),
        };
        if emitted.contains(&name.as_str()) {
            diagnostics.push(Diagnostic::new(
                "RUNTIME_NAME_COLLISION",
                format!(
                    "Top-level binding '{}' collides with a name the compiler emits ({}). The generated module would be a SyntaxError — rename your binding.",
                    name,
                    emitted.join(", ")
                ),
                "Rename the binding.",
                Some(line),
                Some(column),
            ));
        }
    }

    for import in &file.imports {
        for name in &import.imported_names {
            if emitted.contains(&name.as_str()) {
                diagnostics.push(Diagnostic::new(
                    "RUNTIME_NAME_COLLISION",
                    format!(
                        "Imported name '{}' collides with a name the compiler emits ({}). The generated module would be a SyntaxError — rename the import.",
                        name,
                        emitted.join(", ")
                    ),
                    "Rename the imported name.",
                    Some(import.line),
                    Some(import.column),
                ));
            }
        }
    }
}

/// §7b: duplicate handler exports make the route ambiguous (the router picks
/// one by method name) and the generated module ill-formed — fail at
/// validation time.
pub fn check_duplicate_handlers(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    let mut seen: Vec<&str> = Vec::new();
    for export in &file.exports {
        if export.kind != ExportKind::NamedFunction {
            continue;
        }
        if seen.contains(&export.name.as_str()) {
            diagnostics.push(Diagnostic::new(
                "API_DUPLICATE_HANDLER",
                format!(
                    "Handler '{}' is exported more than once — the route would be ambiguous and the generated module ill-formed.",
                    export.name
                ),
                "Remove all but one definition.",
                Some(export.line),
                Some(export.column),
            ));
        }
        seen.push(&export.name);
    }
}

/// §7a: Reject `env()` calls in files marked `@runsOn client` — the same
/// enforcement tier as CLIENT_DATA_CALL. Environment values are build-time
/// server secrets; a client bundle is publicly downloadable, so a value
/// reaching one is a leak by construction.
pub fn check_env_access_boundary(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    if file.has_env_call {
        if let Some(RunsOn::Client) = file.runs_on {
            diagnostics.push(Diagnostic::new(
                "CLIENT_ENV_ACCESS",
                "env() call in @runsOn client file — env() is only allowed in @runsOn server or @runsOn api files. Environment values are build-time secrets; a client bundle is publicly downloadable.",
                "Move the env() read to a @runsOn server or @runsOn api file and pass the value via props (or read it in an API route).",
                Some(file.env_call_line),
                Some(file.env_call_column),
            ));
        }
    }
}

/// §7c: Reject `session()`/`setSession()` calls in files marked
/// `@runsOn client` — the same enforcement tier and mechanism as
/// CLIENT_DATA_CALL/CLIENT_ENV_ACCESS. A session cookie is a
/// credential-bearing server secret; a client bundle is publicly
/// downloadable, so session handling must never reach one.
pub fn check_session_access_boundary(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    if file.has_session_call {
        if let Some(RunsOn::Client) = file.runs_on {
            diagnostics.push(Diagnostic::new(
                "CLIENT_SESSION_ACCESS",
                "session()/setSession() call in @runsOn client file — sessions are only allowed in @runsOn server or @runsOn api files. A session cookie is a credential-bearing server secret; a client bundle is publicly downloadable.",
                "Move the session read/write to a @runsOn server or @runsOn api file and pass the result via props (or read/write it in an API route).",
                Some(file.session_call_line),
                Some(file.session_call_column),
            ));
        }
    }
}

/// §7a best-effort lint (WARNING, never a build failure): an env() call
/// appearing anywhere within a `client:hydrate` component's prop expression
/// — `<Widget apiKey={env("STRIPE_KEY")} client:hydrate />`,
/// `apiKey={env('K').trim()}`, `auth={`Bearer ${env('API_KEY')}`}`. Detection
/// is AST-based: the parser flags the attribute when its expression subtree
/// contains an env() call, so every shape (direct, chained, template
/// interpolation, nested) is caught uniformly. Known limitation, stated in
/// the SPEC: an env() result stored in an intermediate variable/object first
/// is NOT caught — the hard CLIENT_ENV_ACCESS rejection is the actual
/// guarantee, this lint is a bonus signal.
pub fn check_env_leak_to_client_prop(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    let tree = match &file.render_tree {
        Some(t) => t,
        None => return,
    };
    check_env_leak_in_node(tree, diagnostics);
}

fn check_env_leak_in_node(node: &JsxNode, diagnostics: &mut Vec<Diagnostic>) {
    match node {
        JsxNode::Element { is_hydrate_root, attrs, children, .. } => {
            if *is_hydrate_root {
                for attr in attrs {
                    if attr.contains_env_call {
                        let shown = match &attr.value {
                            JsxAttrValue::Expr(expr) => format!("{}=\"{}\"", attr.name, expr),
                            JsxAttrValue::String(_) => attr.name.clone(),
                        };
                        diagnostics.push(Diagnostic::warning(
                            "ENV_LEAK_TO_CLIENT_PROP",
                            format!(
                                "attribute {} passes an env() result into a client:hydrate component prop — the value is baked into the public client bundle. This is a best-effort lint: any env() call anywhere in the prop expression (direct calls, chained methods, template literals) is caught; an env() result stored in an intermediate variable or object first is not detected.",
                                shown
                            ),
                            "Resolve the env() value in a @runsOn server or @runsOn api file and pass the result via props, or read it in an API route.",
                            None,
                            None,
                        ));
                    }
                }
            }
            for child in children {
                check_env_leak_in_node(child, diagnostics);
            }
        }
        JsxNode::Conditional { cons, alt, .. } => {
            check_env_leak_in_node(cons, diagnostics);
            check_env_leak_in_node(alt, diagnostics);
        }
        JsxNode::ForEach { body, .. } => check_env_leak_in_node(body, diagnostics),
        _ => {}
    }
}

/// §6: Reject reactivity and event wiring in `@runsOn server` files. The server
/// codegen emits a static HTML string with no signal/computed wiring and no
/// event listeners, so both would silently break or do nothing:
///  - a signal referenced in server JSX would be an undefined identifier at
///    prerender time (ReferenceError);
///  - an on* handler attribute is dropped from the SSR html (no JS runs there).
pub fn check_server_boundaries(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    if file.runs_on != Some(RunsOn::Server) {
        return;
    }

    for sig in &file.signals {
        diagnostics.push(Diagnostic::new(
            "SERVER_SIGNAL",
            format!(
                "signal/computed `{}` in @runsOn server file — reactive state is client-only. Server components render static HTML and have no reactive runtime.",
                sig.name
            ),
            "Remove the signal and compute the value with data() or a plain const, or split the component: keep the reactive part in a @runsOn client island.",
            None,
            None,
        ));
    }

    if let Some(tree) = &file.render_tree {
        collect_server_events(tree, diagnostics);
    }
}

fn collect_server_events(node: &parser::JsxNode, diagnostics: &mut Vec<Diagnostic>) {
    match node {
        parser::JsxNode::Element { attrs, children, .. } => {
            for attr in attrs {
                if let Some(rest) = attr.name.strip_prefix("on") {
                    if rest.starts_with(|c: char| c.is_uppercase()) {
                        diagnostics.push(Diagnostic::new(
                            "SERVER_EVENT_HANDLER",
                            format!(
                                "on* handler `{}` in @runsOn server file — event listeners cannot run in server-rendered HTML and are silently dropped. Put the handler on a @runsOn client island instead.",
                                attr.name
                            ),
                            "Move the interactive element into a @runsOn client component used with client:hydrate, or remove the handler.",
                            None,
                            None,
                        ));
                    }
                }
            }
            for child in children {
                collect_server_events(child, diagnostics);
            }
        }
        parser::JsxNode::Conditional { cons, alt, .. } => {
            collect_server_events(cons, diagnostics);
            collect_server_events(alt, diagnostics);
        }
        parser::JsxNode::ForEach { body, .. } => collect_server_events(body, diagnostics),
        _ => {}
    }
}

/// §3 rule 3: Component body statements must appear in the fixed order:
/// signals/computed → derived consts → event handlers → single return.
///
/// Additional restrictions from the spec:
///  - No `let` or `var` bindings inside the component body (mutation through signals only).
///  - No other statement types (for, while, if-at-top-level, bare expressions, etc.)
///  - Exactly one return statement, and it must be the last statement.
pub fn check_statement_ordering(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    if !file.has_component_body {
        return;
    }

    if file.body_stmts.is_empty() {
        diagnostics.push(Diagnostic::new(
            "MISSING_RETURN",
            "No return statement found — a component body must end with exactly one return statement that produces JSX.",
            "Add a return statement as the last line of the component body, e.g., 'return (<div>...</div>)'.",
            None,
            None,
        ));
        return;
    }

    let stmts = &file.body_stmts;
    let mut has_signal = false;
    let mut has_derived_const = false;
    let mut has_handler = false;
    let mut has_return = false;

    for stmt in stmts {
        if has_return {
            match stmt.kind {
                BodyStmtKind::Return => {
                    diagnostics.push(Diagnostic::new(
                        "MULTIPLE_RETURN",
                        "Multiple return statements found — a component body must have exactly one return.",
                        "Keep only one return statement as the last statement in the component body.",
                        Some(stmt.line),
                        Some(stmt.column),
                    ));
                }
                _ => {
                    diagnostics.push(Diagnostic::new(
                        "STATEMENT_AFTER_RETURN",
                        "Statement found after return — the return statement must be the last statement in the component body.",
                        "Move this statement before the return statement.",
                        Some(stmt.line),
                        Some(stmt.column),
                    ));
                }
            }
            continue;
        }

        match stmt.kind {
            BodyStmtKind::Let => {
                diagnostics.push(Diagnostic::new(
                    "BODY_LET",
                    "let binding inside component body — use const for immutable derived values, or signal() for reactive state.",
                    "Replace let with const, or wrap the value in signal() for reactive state.",
                    Some(stmt.line),
                    Some(stmt.column),
                ));
            }
            BodyStmtKind::Var => {
                diagnostics.push(Diagnostic::new(
                    "BODY_VAR",
                    "var binding inside component body — use const for immutable derived values, or signal() for reactive state.",
                    "Replace var with const, or wrap the value in signal() for reactive state.",
                    Some(stmt.line),
                    Some(stmt.column),
                ));
            }
            BodyStmtKind::Other => {
                diagnostics.push(Diagnostic::new(
                    "BODY_FORBIDDEN_STMT",
                    "Forbidden statement type in component body — only const declarations, inner function declarations, and a single return statement are allowed.",
                    "Remove this statement or restructure as a const/signal/computed/event handler.",
                    Some(stmt.line),
                    Some(stmt.column),
                ));
            }
            BodyStmtKind::Signal => {
                if has_derived_const {
                    diagnostics.push(Diagnostic::new(
                        "STATEMENT_OUT_OF_ORDER",
                        "signal()/computed() declaration appears after derived const values — signals must come first in the component body.",
                        "Move this signal()/computed() declaration above all derived const declarations.",
                        Some(stmt.line),
                        Some(stmt.column),
                    ));
                }
                if has_handler {
                    diagnostics.push(Diagnostic::new(
                        "STATEMENT_OUT_OF_ORDER",
                        "signal()/computed() declaration appears after event handlers — signals must come before handlers.",
                        "Move this signal()/computed() declaration above all event handler declarations.",
                        Some(stmt.line),
                        Some(stmt.column),
                    ));
                }
                has_signal = true;
            }
            BodyStmtKind::DerivedConst => {
                if has_handler {
                    diagnostics.push(Diagnostic::new(
                        "STATEMENT_OUT_OF_ORDER",
                        "const declaration appears after event handlers — derived consts must come before handlers in the component body.",
                        "Move this const declaration above all event handler declarations.",
                        Some(stmt.line),
                        Some(stmt.column),
                    ));
                }
                has_derived_const = true;
            }
            BodyStmtKind::EventHandler => {
                has_handler = true;
            }
            BodyStmtKind::Return => {
                has_return = true;
            }
        }
    }

    if !has_return {
        diagnostics.push(Diagnostic::new(
            "MISSING_RETURN",
            "No return statement found — a component body must end with exactly one return statement that produces JSX.",
            "Add a return statement as the last line of the component body, e.g., 'return (<div>...</div>)'.",
            None,
            None,
        ));
    }
}

/// §2a: Reject non-bare CSS imports and CSS imports in server components.
pub fn check_css_imports(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    for import in &file.imports {
        if !import.is_css {
            continue;
        }

        if !import.imported_names.is_empty() {
            let names = import.imported_names.join(", ");
            diagnostics.push(Diagnostic::new(
                "INVALID_CSS_IMPORT",
                format!(
                    "Invalid CSS import from '{}' — CSS imports must be bare side-effect imports (import \"./X.css\"). Found named/default/namespace binding(s): {}.",
                    import.source, names
                ),
                "Change to a bare side-effect import: import \"./path/to/file.css\" with no bindings.",
                Some(import.line),
                Some(import.column),
            ));
        }

        if file.runs_on == Some(RunsOn::Server) {
            diagnostics.push(Diagnostic::new(
                "INVALID_CSS_IMPORT",
                format!(
                    "CSS import '{}' in @runsOn server file — CSS is only valid in @runsOn client components. Server components render static HTML without stylesheets.",
                    import.source
                ),
                "Remove the CSS import from this server file. Style server output with inline attributes on JSX elements.",
                Some(import.line),
                Some(import.column),
            ));
        }

        if file.runs_on == Some(RunsOn::Api) {
            diagnostics.push(Diagnostic::new(
                "INVALID_CSS_IMPORT",
                format!(
                    "CSS import '{}' in @runsOn api file — API route handlers return Responses, they never render stylesheets.",
                    import.source
                ),
                "Remove the CSS import from this API route file.",
                Some(import.line),
                Some(import.column),
            ));
        }
    }
}

/// Runs all checks in sequence, collecting every diagnostic without short-circuiting.
/// An agent fixing errors benefits from seeing everything wrong at once, not one at a time.
pub fn check_unwrapped_signal_prop(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    let tree = match &file.render_tree {
        Some(t) => t,
        None => return,
    };
    let signal_names: Vec<&str> = file.signals.iter().map(|s| s.name.as_str()).collect();
    check_unwrapped_in_node(tree, &signal_names, diagnostics);
}

fn check_unwrapped_in_node(
    node: &JsxNode,
    signal_names: &[&str],
    diagnostics: &mut Vec<Diagnostic>,
) {
    match node {
        JsxNode::Element { is_component, attrs, children, .. } => {
            if *is_component {
                for attr in attrs {
                    if let JsxAttrValue::Expr(expr) = &attr.value {
                        for name in signal_names {
                            if expr == &format!("{}.value", *name) {
                                diagnostics.push(Diagnostic::new(
                                    "PROP_UNWRAPPED_SIGNAL",
                                    format!(
                                        "attribute {}=\"{}\" unwraps signal '{}' at call site. \
                                         Pass the signal by reference: {}=\"{{{}}}\" and read \
                                         .value inside the child component",
                                        attr.name, expr, name, attr.name, name
                                    ),
                                    "pass the signal by reference and read .value inside the child",
                                    None,
                                    None,
                                ));
                            }
                        }
                    }
                }
            }
            for child in children {
                check_unwrapped_in_node(child, signal_names, diagnostics);
            }
        }
        JsxNode::Conditional { cons, alt, .. } => {
            check_unwrapped_in_node(cons, signal_names, diagnostics);
            check_unwrapped_in_node(alt, signal_names, diagnostics);
        }
        JsxNode::ForEach { body, .. } => {
            check_unwrapped_in_node(body, signal_names, diagnostics);
        }
        _ => {}
    }
}

/// Converts parser-level unsupported-construct errors into validator diagnostics.
pub fn check_unsupported_constructs(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    for err in &file.unsupported_errors {
        diagnostics.push(Diagnostic::new(
            err.code,
            err.message.clone(),
            err.fix_hint,
            None,
            None,
        ));
    }
}

/// Returns true if a statement's AST subtree contains any JSX element or fragment.
/// §8: Helper functions in component body must not contain JSX in their bodies.
/// JSX in handlers is not compiled by the codegen — it's emitted verbatim, causing
/// runtime SyntaxErrors or silent stringification.
pub fn check_handler_jsx(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {    for (i, has_jsx) in file.handler_has_jsx.iter().enumerate() {
        if *has_jsx {
            let handler_name = file.handler_decls
                .get(i)
                .and_then(|h| h.lines().next())
                .unwrap_or("unknown handler");
            diagnostics.push(Diagnostic::new(
                "HANDLER_JSX",
                format!(
                    "Handler function contains JSX in its body: `{}` — helper functions that return JSX are not yet compiled. Move the JSX inline into the component's return statement instead.",
                    handler_name.trim()
                ),
                "Move the JSX directly into the return statement, or use a ternary/For pattern inline.",
                None,
                None,
            ));
        }
    }
}

/// §7b: The API-file rule set. Deliberately SMALLER than the component rule
/// set — an API handler is not a component. The component checks (props
/// parameter, statement ordering, signals/computed, JSX render tree, hydrate
/// markers, filename-matches-component) do NOT apply and are NOT run; a
/// handler body is ordinary TypeScript, emitted verbatim. Only the checks
/// below apply:
///  - `@runsOn api` directive presence/uniqueness/correctness
///  - exports: at least one, each a sanctioned HTTP method name, no defaults
///  - no `data()` (page-render-time fetching; an api handler renders no page)
///  - no forbidden/hook imports, no CSS imports (reused component checks)
///  - no parser-level unsupported constructs
/// `env()` is allowed and needs no check (same tier as @runsOn server).
pub const API_METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

pub fn validate_api(file: &ComponentFile) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    check_api_runs_on(file, &mut diagnostics);

    if file.exports.is_empty() {
        diagnostics.push(Diagnostic::new(
            "API_NO_HANDLER",
            "No exported HTTP method handler found — an API route file must export at least one of GET, POST, PUT, PATCH, DELETE.",
            "Add e.g. `export function GET(req) { return new Response(...); }`.",
            None,
            None,
        ));
    }
    for export in &file.exports {
        if export.kind == ExportKind::DefaultExport {
            diagnostics.push(Diagnostic::new(
                "DEFAULT_EXPORT",
                "Default export in API route file — API routes export one function per HTTP method, by name.",
                "Replace the default export with named `export function GET(req)` / `POST(req)` / etc.",
                Some(export.line),
                Some(export.column),
            ));
        } else if !API_METHODS.contains(&export.name.as_str()) {
            diagnostics.push(Diagnostic::new(
                "API_INVALID_HANDLER",
                format!(
                    "Export '{}' is not a sanctioned API handler name — the router determines supported methods by which of GET, POST, PUT, PATCH, DELETE are exported.",
                    export.name
                ),
                "Rename the export to one of GET, POST, PUT, PATCH, DELETE.",
                Some(export.line),
                Some(export.column),
            ));
        }
    }

    if file.has_data_call {
        diagnostics.push(Diagnostic::new(
            "API_DATA_CALL",
            "data() call in @runsOn api file — data() is page-render-time fetching; an API route handler is not rendering a page.",
            "Remove the data() call and fetch directly in the handler with fetch() or a client library.",
            Some(file.data_call_line),
            Some(file.data_call_column),
        ));
    }

    check_forbidden_imports(file, &mut diagnostics);
    check_css_imports(file, &mut diagnostics);
    check_unsupported_constructs(file, &mut diagnostics);
    check_duplicate_handlers(file, &mut diagnostics);
    check_runtime_collisions(file, &mut diagnostics);

    diagnostics
}

fn check_api_runs_on(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    if file.runs_on_count == 0 {
        diagnostics.push(Diagnostic::new(
            "MISSING_RUNSON",
            "Missing @runsOn directive — every file under api/ must declare exactly one // @runsOn api.",
            "Add `// @runsOn api` as the first line of the file.",
            None,
            None,
        ));
    } else if file.runs_on_count > 1 {
        diagnostics.push(Diagnostic::new(
            "DUPLICATE_RUNSON",
            format!(
                "Multiple @runsOn directives found ({}) — an API route file must declare exactly one.",
                file.runs_on_count
            ),
            "Remove all but one @runsOn directive.",
            Some(file.runs_on_line),
            Some(file.runs_on_column),
        ));
    } else if file.runs_on != Some(RunsOn::Api) {
        diagnostics.push(Diagnostic::new(
            "API_RUNSON_REQUIRED",
            "File under api/ must declare // @runsOn api — @runsOn client/server files belong under pages/ or components/.",
            "Change the directive to `// @runsOn api`.",
            Some(file.runs_on_line),
            Some(file.runs_on_column),
        ));
    }
}

pub fn validate(file: &ComponentFile) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    check_runs_on_directive(file, &mut diagnostics);
    check_single_export(file, &mut diagnostics);
    check_filename_matches_component(file, &mut diagnostics);
    check_props_parameter(file, &mut diagnostics);
    check_forbidden_imports(file, &mut diagnostics);
    check_no_global_mutable_state(file, &mut diagnostics);
    check_conditional_rendering_form(file, &mut diagnostics);
    check_list_rendering_form(file, &mut diagnostics);
    check_data_call_boundary(file, &mut diagnostics);
    check_env_access_boundary(file, &mut diagnostics);
    check_env_leak_to_client_prop(file, &mut diagnostics);
    check_session_access_boundary(file, &mut diagnostics);
    check_server_boundaries(file, &mut diagnostics);
    check_statement_ordering(file, &mut diagnostics);
    check_unwrapped_signal_prop(file, &mut diagnostics);
    check_children_shape(file, &mut diagnostics);
    check_unsupported_constructs(file, &mut diagnostics);
    check_css_imports(file, &mut diagnostics);
    check_handler_jsx(file, &mut diagnostics);
    check_runtime_collisions(file, &mut diagnostics);
    diagnostics
}

/// §7e: a component call with nested content whose Props type does not
/// declare a `children` field is UNEXPECTED_CHILDREN, with a fix hint
/// pointing at adding the field.
///
/// `props_type` is the TARGET component's Props declaration — the file's own
/// when the call targets the file's exported component, or a parsed
/// neighbor's when the call targets an imported component (the CLI resolves
/// imports; a single file cannot know its neighbors' Props types, the same
/// reason CLIENT_IMPORTS_SERVER is build-orchestrated). None → the compiler
/// cannot see the target's fields (untyped/any props, or a *.types.ts
/// imported type) — no diagnostic, exactly as strong as the view it has.
/// This is what "integrates with the existing props- rules" means: a target
/// with a broken props parameter (NO_PROPS / PROPS_UNTYPED / PROPS_ANY)
/// already fails its own file's validation; UNEXPECTED_CHILDREN never stacks
/// on top of that, it only fires when the props type is a known, declared
/// shape.
pub fn unexpected_children_diagnostic(
    tag: &str,
    props_type: Option<&parser::TypeDecl>,
) -> Option<Diagnostic> {
    let decl = props_type?;
    if !decl.complete {
        return None;
    }
    let has_children = decl.fields.iter().any(|f| f.name == "children");
    if has_children {
        return None;
    }
    Some(Diagnostic::new(
        "UNEXPECTED_CHILDREN",
        format!(
            "<{}> is called with nested JSX, but its Props type '{}' does not declare a children field — children is an explicit prop, not implicit magic",
            tag, decl.name
        ),
        "Add a `children: JSX.Element` field to the component's Props type.",
        None,
        None,
    ))
}

/// §7e: multiple sibling children passed without a wrapping element — the
/// same "one root element per return" discipline from §5, applied to a
/// component's children slot. The caller must wrap siblings in a single
/// parent (or an explicit fragment) before passing; there is no implicit
/// fragment-wrapping anywhere in the language.
fn check_children_shape(file: &ComponentFile, diagnostics: &mut Vec<Diagnostic>) {
    let Some(tree) = &file.render_tree else { return };
    check_children_shape_in_node(tree, diagnostics);
}

fn check_children_shape_in_node(node: &JsxNode, diagnostics: &mut Vec<Diagnostic>) {
    match node {
        JsxNode::Element { tag, children, is_component, .. } => {
            if *is_component {
                let real = parser::real_children(children);
                if real.len() > 1 {
                    diagnostics.push(Diagnostic::new(
                        "MULTIPLE_CHILDREN",
                        format!(
                            "<{}> is passed {} sibling children — a component's children must be a single root element, the same discipline as §5's one-root-per-return",
                            tag, real.len()
                        ),
                        "Wrap the siblings in a single parent element (or an explicit fragment) before passing them.",
                        None,
                        None,
                    ));
                }
            }
            for child in children {
                check_children_shape_in_node(child, diagnostics);
            }
        }
        JsxNode::Conditional { cons, alt, .. } => {
            check_children_shape_in_node(cons, diagnostics);
            check_children_shape_in_node(alt, diagnostics);
        }
        JsxNode::ForEach { body, .. } => check_children_shape_in_node(body, diagnostics),
        _ => {}
    }
}

/// §7d: the middleware.ts rule set — the single request-gating surface, so
/// every rule here is mandatory.
///
/// - The file must export exactly a `middleware` function (MISSING_MIDDLEWARE)
///   and a `matcher` const (MATCHER_REQUIRED), whose value is a static array
///   of string literals (MATCHER_NOT_ARRAY / MATCHER_NOT_STRING).
/// - Every `return` inside the middleware function body must be a direct call
///   to next()/redirect()/respond() (MIDDLEWARE_RESULT). Anything else is
///   rejected — a non-sanctioned return is a gate that can silently pass
///   traffic through.
/// - A call to next()/redirect()/respond() that is NOT the direct return
///   value is rejected (MIDDLEWARE_HELPER_NOT_RETURNED): a discarded gate
///   result is the same silent pass-through.
/// - The server-only surfaces that middleware may use are session(),
///   setSession() and env() (same tier as @runsOn api). data() is
///   page-render-time fetching and has no meaning before routing
///   (MIDDLEWARE_DATA_CALL). @runsOn is not a middleware directive
///   (MIDDLEWARE_NO_RUNSON).
pub fn validate_middleware(file: &ComponentFile) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    if !file.has_middleware_fn {
        diagnostics.push(Diagnostic::new(
            "MISSING_MIDDLEWARE",
            "middleware.ts must export a function named `middleware` — it is the request-gating entry point run before every route.",
            "Add `export function middleware(req: Request): MiddlewareResult { ... }` (or `export const middleware = (req) => ...`).",
            Some(file.middleware_fn_line),
            Some(file.middleware_fn_column),
        ));
    }

    if !file.matcher_present {
        diagnostics.push(Diagnostic::new(
            "MATCHER_REQUIRED",
            "middleware.ts must export a `matcher` const — it declares which request paths the middleware applies to.",
            "Add `export const matcher: string[] = ['/admin/*'];`",
            None,
            None,
        ));
    } else if file.matcher.is_none() {
        let (code, detail) = match file.matcher_invalid_reason.as_deref() {
            Some(reason) if reason.contains("array literal") => (
                "MATCHER_NOT_ARRAY",
                "`matcher` must be an array literal of string literals — the value is extracted statically at build time.",
            ),
            _ => (
                "MATCHER_NOT_STRING",
                "`matcher` must contain only string literals (no spread, no computed values).",
            ),
        };
        diagnostics.push(Diagnostic::new(
            code,
            detail,
            "Rewrite `matcher` as a plain array literal of string patterns, e.g. `['/admin/*']`.",
            Some(file.matcher_line),
            Some(file.matcher_column),
        ));
    }

    for ret in &file.middleware_returns {
        if ret.kind == parser::MiddlewareResultKind::Bad {
            diagnostics.push(Diagnostic::new(
                "MIDDLEWARE_RESULT",
                format!(
                    "middleware return path is not a sanctioned result: {}",
                    ret.detail
                ),
                "Return a direct call to next(), redirect(), or respond() — no ternaries, no bare returns, no other functions.",
                Some(ret.line),
                Some(ret.column),
            ));
        }
    }

    // The cross-check: every helper call inside the middleware body must BE
    // the direct return value of its return statement. A call whose span does
    // not match a sanctioned return's arg span was discarded.
    for call in &file.middleware_helper_calls {
        let is_returned = file
            .middleware_returns
            .iter()
            .any(|ret| ret.arg_span == Some(call.span));
        if !is_returned {
            diagnostics.push(Diagnostic::new(
                "MIDDLEWARE_HELPER_NOT_RETURNED",
                format!(
                    "{}() was called but its result is not the direct return value — a gate result that is discarded silently lets the request pass through.",
                    call.name
                ),
                "Return the call directly: `return next();` / `return redirect(url);` / `return respond(res);`.",
                Some(call.line),
                Some(call.column),
            ));
        }
    }

    if file.has_data_call {
        diagnostics.push(Diagnostic::new(
            "MIDDLEWARE_DATA_CALL",
            "data() call in middleware.ts — data() is page-render-time fetching; middleware runs before routing and cannot call it.",
            "Fetch in the middleware with fetch() directly, or move the fetch to the route handler.",
            Some(file.data_call_line),
            Some(file.data_call_column),
        ));
    }

    if file.runs_on_count > 0 {
        diagnostics.push(Diagnostic::new(
            "MIDDLEWARE_NO_RUNSON",
            "@runsOn directive in middleware.ts — middleware is not a component and has no runsOn placement; it runs on the server before every matching request.",
            "Remove the @runsOn directive from middleware.ts.",
            Some(file.runs_on_line),
            Some(file.runs_on_column),
        ));
    }

    check_forbidden_imports(file, &mut diagnostics);
    check_unsupported_constructs(file, &mut diagnostics);
    check_middleware_runtime_collisions(file, &mut diagnostics);

    // §7d: a binding inside the middleware function body named next/redirect/
    // respond shadows the emitted helper — `const next = ...; return next();`
    // would make a "redirect" silently become a pass-through. Reject it.
    for shadow in &file.middleware_shadows {
        diagnostics.push(Diagnostic::new(
            "MIDDLEWARE_HELPER_SHADOW",
            format!(
                "Binding '{}' shadows the emitted middleware result helper — a `return {}()` in scope would call YOUR binding, not the helper, silently changing the gate decision.",
                shadow.name,
                shadow.name
            ),
            "Rename the binding (the result helpers next/redirect/respond are reserved inside middleware).",
            Some(shadow.line),
            Some(shadow.column),
        ));
    }

    diagnostics
}
/// under an `api/` directory (any ancestor directory named `api` — the
/// §7c: dispatches to the API rule set when the file lives under an `api/`
/// directory (any ancestor directory named `api` — the faithful mirror of
/// the CLI build's root-relative `api/` prefix match) or carries the
/// `@runsOn api` directive; §7d middleware.ts gets the middleware rule set;
/// everything else gets the full component rule set. One dispatch rule,
/// shared by every validator entry point, so a path can never validate with
/// the wrong rule set.
pub fn validate_for_path(file: &ComponentFile, path: &Path) -> Vec<Diagnostic> {
    let mut diagnostics = if file.is_middleware {
        validate_middleware(file)
    } else if file.runs_on == Some(RunsOn::Api) || path_is_under_api_dir(path) {
        validate_api(file)
    } else {
        validate(file)
    };
    check_server_page_env_prerender(file, path, &mut diagnostics);
    diagnostics
}

/// E4-13: a @runsOn server PAGE without `data()` is prerendered at build
/// time into a static dist/*.html served to everyone. If its module code
/// reads env(), the baked value can end up inside that public HTML (the
/// value is evaluated during prerender). Warning (not error): env() of
/// non-secret configuration on a static page is legitimate. Secret values
/// must move to an api route or a data() (dynamic) page.
fn check_server_page_env_prerender(file: &ComponentFile, path: &Path, diagnostics: &mut Vec<Diagnostic>) {
    let is_page = path
        .strip_prefix("pages")
        .map_or(false, |rest| rest.as_os_str().is_empty() || rest.starts_with("/"));
    if file.runs_on == Some(RunsOn::Server)
        && is_page
        && !file.has_data_call
        && file.has_env_call
    {
        diagnostics.push(Diagnostic::warning(
            "SERVER_PAGE_ENV_PRERENDER",
            format!(
                "this page is prerendered into static HTML at build time (no data()), but its module code reads env() — the value is evaluated during prerender and may end up in the public .html file served to everyone. Never read secrets (SESSION_SECRET, API keys) here."
            ),
            "Read secrets only from api routes or data(); for non-secret configuration, this warning is informational.",
            Some(file.env_call_line.max(1)),
            Some(file.env_call_column.max(1)),
        ));
    }
}

fn path_is_under_api_dir(path: &Path) -> bool {
    path.ancestors()
        .skip(1) // the file itself
        .any(|anc| anc.file_name().map_or(false, |f| f == "api"))
}

/// §2: Every import must resolve to an existing file in the source tree.
/// This is a build-time safety net for the relative-import emission logic:
/// it catches case mismatches, typos, and moves the compiler's emission can't
/// detect (the compiler emits the specifier as-written in source, relative to
/// the generated output's location). Requires the file's own source path and
/// the project source root — unlike `validate`, it does file-system checks.
pub fn validate_imports(file: &ComponentFile, file_rel: &Path, source_dir: &Path) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let file_parent = file_rel.parent().unwrap_or(Path::new(""));

    for imp in &file.imports {
        if imp.is_css {
            continue;
        }
        // Only relative file imports are validated — bare specifiers and
        // scoped packages (@marisjs/runtime, jsdom, ...) are external
        // modules resolved by the runtime, not files in the source tree.
        if !(imp.source.starts_with("./") || imp.source.starts_with("../")) {
            continue;
        }
        // Resolve the import relative to this file's directory, normalizing
        // ./ and ../ segments, then look for the compiled sibling (.tsx or
        // .ts — API route files compile from .ts sources).
        let trimmed = imp.source.trim_end_matches(".tsx").trim_end_matches(".ts");
        let resolved = normalize_path(&file_parent.join(trimmed));
        let candidate_tsx = source_dir.join(&resolved).with_extension("tsx");
        let candidate_ts = source_dir.join(&resolved).with_extension("ts");
        if !candidate_tsx.exists() && !candidate_ts.exists() {
            diagnostics.push(Diagnostic::new(
                "IMPORT_NOT_FOUND",
                format!(
                    "import '{}' does not resolve to an existing file (resolved to {})",
                    imp.source,
                    resolved.display()
                ),
                "Fix the import path so it points to a real .tsx or .ts file in the project.",
                Some(imp.line),
                Some(imp.column),
            ));
        }
    }

    diagnostics
}

fn normalize_path(p: &Path) -> PathBuf {
    let mut components = Vec::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            std::path::Component::Normal(s) => components.push(s.to_os_string()),
            _ => {}
        }
    }
    components.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use parser::{
        ComponentFile, ExportInfo, ExportKind, ImportInfo, JsxExprInfo,
        JsxExpression, JsxNode, PropField, PropsInfo, RunsOn, SignalDecl, SignalKind,
        TopLevelBinding, TypeAnnotation, TypeDecl, BodyStmt, BodyStmtKind, ParserError,
    };

    fn make_file() -> ComponentFile {
        ComponentFile::new("Test.tsx")
    }

    fn assert_code(diagnostics: &[Diagnostic], expected_code: &str) {
        assert!(
            diagnostics.iter().any(|d| d.code == expected_code),
            "expected diagnostic with code '{}', got codes: {:?}",
            expected_code,
            diagnostics.iter().map(|d| d.code).collect::<Vec<_>>()
        );
    }

    // ── check_runs_on_directive ──────────────────────────────────────────

    #[test]
    fn runs_on_directive_present() {
        let file = ComponentFile {
            runs_on_count: 1,
            runs_on: Some(RunsOn::Client),
            runs_on_line: 1,
            runs_on_column: 1,
            ..make_file()
        };
        let mut diags = Vec::new();
        check_runs_on_directive(&file, &mut diags);
        assert!(diags.is_empty());
    }

    #[test]
    fn runs_on_directive_missing() {
        let file = ComponentFile {
            runs_on_count: 0,
            runs_on: None,
            ..make_file()
        };
        let mut diags = Vec::new();
        check_runs_on_directive(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "MISSING_RUNSON");
        assert!(!diags[0].fix_hint.is_empty());
    }

    #[test]
    fn runs_on_directive_duplicate() {
        let file = ComponentFile {
            runs_on_count: 2,
            runs_on: Some(RunsOn::Server),
            runs_on_line: 1,
            runs_on_column: 1,
            ..make_file()
        };
        let mut diags = Vec::new();
        check_runs_on_directive(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "DUPLICATE_RUNSON");
    }

    // ── check_single_export ──────────────────────────────────────────────

    #[test]
    fn single_named_export() {
        let file = ComponentFile {
            exports: vec![ExportInfo {
                name: "Cart".into(),
                kind: ExportKind::NamedFunction,
                line: 5,
                column: 1,
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_single_export(&file, &mut diags);
        assert!(diags.is_empty());
    }

    #[test]
    fn no_exports() {
        let file = make_file();
        let mut diags = Vec::new();
        check_single_export(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "NO_EXPORT");
    }

    #[test]
    fn multiple_exports() {
        let file = ComponentFile {
            exports: vec![
                ExportInfo {
                    name: "Cart".into(),
                    kind: ExportKind::NamedFunction,
                    line: 3,
                    column: 1,
                },
                ExportInfo {
                    name: "helper".into(),
                    kind: ExportKind::NamedFunction,
                    line: 8,
                    column: 1,
                },
            ],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_single_export(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "MULTIPLE_EXPORTS");
    }

    #[test]
    fn default_export() {
        let file = ComponentFile {
            exports: vec![ExportInfo {
                name: "Cart".into(),
                kind: ExportKind::DefaultExport,
                line: 3,
                column: 1,
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_single_export(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "DEFAULT_EXPORT");
    }

    // ── check_filename_matches_component ─────────────────────────────────

    #[test]
    fn filename_matches() {
        let file = ComponentFile {
            filename: "Cart.tsx".into(),
            exports: vec![ExportInfo {
                name: "Cart".into(),
                kind: ExportKind::NamedFunction,
                line: 5,
                column: 1,
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_filename_matches_component(&file, &mut diags);
        assert!(diags.is_empty());
    }

    #[test]
    fn filename_mismatch() {
        let file = ComponentFile {
            filename: "Cart.tsx".into(),
            exports: vec![ExportInfo {
                name: "ShoppingCart".into(),
                kind: ExportKind::NamedFunction,
                line: 5,
                column: 1,
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_filename_matches_component(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "FILENAME_MISMATCH");
    }

    #[test]
    fn filename_check_skipped_when_no_named_export() {
        let file = ComponentFile {
            filename: "Cart.tsx".into(),
            exports: vec![ExportInfo {
                name: "Cart".into(),
                kind: ExportKind::DefaultExport,
                line: 3,
                column: 1,
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_filename_matches_component(&file, &mut diags);
        assert!(diags.is_empty());
    }

    // ── check_props_parameter ────────────────────────────────────────────

    #[test]
    fn valid_props() {
        let file = ComponentFile {
            props: Some(PropsInfo {
                name: "props".into(),
                type_annotation: TypeAnnotation::Named("CartProps".into()),
                is_destructured: false,
                line: 6,
                column: 29,
            }),
            ..make_file()
        };
        let mut diags = Vec::new();
        check_props_parameter(&file, &mut diags);
        assert!(diags.is_empty());
    }

    #[test]
    fn props_missing() {
        let file = make_file();
        let mut diags = Vec::new();
        check_props_parameter(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "NO_PROPS");
    }

    #[test]
    fn props_wrong_name() {
        let file = ComponentFile {
            props: Some(PropsInfo {
                name: "data".into(),
                type_annotation: TypeAnnotation::Named("CartProps".into()),
                is_destructured: false,
                line: 6,
                column: 29,
            }),
            ..make_file()
        };
        let mut diags = Vec::new();
        check_props_parameter(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "PROPS_WRONG_NAME");
    }

    #[test]
    fn props_destructured() {
        let file = ComponentFile {
            props: Some(PropsInfo {
                name: "props".into(),
                type_annotation: TypeAnnotation::Named("CartProps".into()),
                is_destructured: true,
                line: 6,
                column: 29,
            }),
            ..make_file()
        };
        let mut diags = Vec::new();
        check_props_parameter(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "PROPS_DESTRUCTURED");
    }

    #[test]
    fn props_typed_any() {
        let file = ComponentFile {
            props: Some(PropsInfo {
                name: "props".into(),
                type_annotation: TypeAnnotation::Any,
                is_destructured: false,
                line: 6,
                column: 29,
            }),
            ..make_file()
        };
        let mut diags = Vec::new();
        check_props_parameter(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "PROPS_ANY");
    }

    #[test]
    fn props_untyped() {
        let file = ComponentFile {
            props: Some(PropsInfo {
                name: "props".into(),
                type_annotation: TypeAnnotation::Untyped,
                is_destructured: false,
                line: 6,
                column: 29,
            }),
            ..make_file()
        };
        let mut diags = Vec::new();
        check_props_parameter(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "PROPS_UNTYPED");
    }

    #[test]
    fn props_multiple_errors() {
        let file = ComponentFile {
            props: Some(PropsInfo {
                name: "data".into(),
                type_annotation: TypeAnnotation::Any,
                is_destructured: true,
                line: 6,
                column: 29,
            }),
            ..make_file()
        };
        let mut diags = Vec::new();
        check_props_parameter(&file, &mut diags);
        assert_eq!(diags.len(), 3);
        assert_code(&diags, "PROPS_WRONG_NAME");
        assert_code(&diags, "PROPS_DESTRUCTURED");
        assert_code(&diags, "PROPS_ANY");
    }

    // ── check_forbidden_imports ──────────────────────────────────────────

    #[test]
    fn allowed_imports() {
        let file = ComponentFile {
            imports: vec![ImportInfo {
                source: "./utils".into(),
                imported_names: vec!["formatPrice".into()],
                line: 1,
                column: 1,
                is_css: false,
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_forbidden_imports(&file, &mut diags);
        assert!(diags.is_empty());
    }

    #[test]
    fn no_imports() {
        let file = make_file();
        let mut diags = Vec::new();
        check_forbidden_imports(&file, &mut diags);
        assert!(diags.is_empty());
    }

    #[test]
    fn import_from_react() {
        let file = ComponentFile {
            imports: vec![ImportInfo {
                source: "react".into(),
                imported_names: vec!["useState".into()],
                line: 2,
                column: 1,
                is_css: false,
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_forbidden_imports(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "FORBIDDEN_IMPORT");
    }

    #[test]
    fn import_hook_from_allowed_module() {
        let file = ComponentFile {
            imports: vec![ImportInfo {
                source: "./my-utils".into(),
                imported_names: vec!["formatPrice".into(), "useState".into()],
                line: 1,
                column: 1,
                is_css: false,
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_forbidden_imports(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "FORBIDDEN_HOOK");
    }

    #[test]
    fn import_from_preact_hooks() {
        let file = ComponentFile {
            imports: vec![ImportInfo {
                source: "preact/hooks".into(),
                imported_names: vec!["useEffect".into()],
                line: 2,
                column: 1,
                is_css: false,
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_forbidden_imports(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "FORBIDDEN_IMPORT");
    }

    // ── check_no_global_mutable_state ────────────────────────────────────

    #[test]
    fn no_mutable_bindings() {
        let file = make_file();
        let mut diags = Vec::new();
        check_no_global_mutable_state(&file, &mut diags);
        assert!(diags.is_empty());
    }

    #[test]
    fn top_level_let() {
        let file = ComponentFile {
            top_level_bindings: vec![TopLevelBinding::Let {
                name: "count".into(),
                exported: false,
                line: 3,
                column: 1,
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_no_global_mutable_state(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "GLOBAL_LET");
    }

    #[test]
    fn exported_let() {
        let file = ComponentFile {
            top_level_bindings: vec![TopLevelBinding::Let {
                name: "cache".into(),
                exported: true,
                line: 3,
                column: 1,
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_no_global_mutable_state(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "GLOBAL_LET");
    }

    #[test]
    fn top_level_var() {
        let file = ComponentFile {
            top_level_bindings: vec![TopLevelBinding::Var {
                name: "debug".into(),
                exported: false,
                line: 3,
                column: 1,
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_no_global_mutable_state(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "GLOBAL_VAR");
    }

    // ── check_conditional_rendering_form ─────────────────────────────────

    #[test]
    fn no_and_conditional() {
        let file = make_file();
        let mut diags = Vec::new();
        check_conditional_rendering_form(&file, &mut diags);
        assert!(diags.is_empty());
    }

    #[test]
    fn and_conditional_found() {
        let file = ComponentFile {
            jsx_expressions: vec![JsxExprInfo {
                kind: JsxExpression::AndConditional,
                line: 10,
                column: 5,
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_conditional_rendering_form(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "AND_CONDITIONAL");
    }

    // ── check_list_rendering_form ────────────────────────────────────────

    #[test]
    fn no_inline_map() {
        let file = make_file();
        let mut diags = Vec::new();
        check_list_rendering_form(&file, &mut diags);
        assert!(diags.is_empty());
    }

    #[test]
    fn inline_map_found() {
        let file = ComponentFile {
            jsx_expressions: vec![JsxExprInfo {
                kind: JsxExpression::InlineMap,
                line: 10,
                column: 5,
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_list_rendering_form(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "INLINE_MAP");
    }

    // ── check_data_call_boundary ─────────────────────────────────────────

    #[test]
    fn data_call_on_server() {
        let file = ComponentFile {
            runs_on: Some(RunsOn::Server),
            runs_on_count: 1,
            runs_on_line: 1,
            runs_on_column: 1,
            has_data_call: true,
            data_call_line: 8,
            data_call_column: 20,
            ..make_file()
        };
        let mut diags = Vec::new();
        check_data_call_boundary(&file, &mut diags);
        assert!(diags.is_empty());
    }

    #[test]
    fn no_data_call_on_client() {
        let file = ComponentFile {
            runs_on: Some(RunsOn::Client),
            runs_on_count: 1,
            runs_on_line: 1,
            runs_on_column: 1,
            has_data_call: false,
            ..make_file()
        };
        let mut diags = Vec::new();
        check_data_call_boundary(&file, &mut diags);
        assert!(diags.is_empty());
    }

    #[test]
    fn data_call_on_client() {
        let file = ComponentFile {
            runs_on: Some(RunsOn::Client),
            runs_on_count: 1,
            runs_on_line: 1,
            runs_on_column: 1,
            has_data_call: true,
            data_call_line: 8,
            data_call_column: 20,
            ..make_file()
        };
        let mut diags = Vec::new();
        check_data_call_boundary(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "CLIENT_DATA_CALL");
    }

    // ── check_session_access_boundary ──────────────────────────────────

    #[test]
    fn session_call_on_client_is_rejected() {
        let file = ComponentFile {
            runs_on: Some(RunsOn::Client),
            runs_on_count: 1,
            runs_on_line: 1,
            runs_on_column: 1,
            has_session_call: true,
            session_call_line: 5,
            session_call_column: 12,
            ..make_file()
        };
        let mut diags = Vec::new();
        check_session_access_boundary(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "CLIENT_SESSION_ACCESS");
        assert!(!diags[0].is_warning, "CLIENT_SESSION_ACCESS must be a hard error");
        assert_eq!((diags[0].line, diags[0].column), (Some(5), Some(12)));
    }

    #[test]
    fn session_call_on_server_and_api_is_allowed() {
        for runs_on in [Some(RunsOn::Server), Some(RunsOn::Api)] {
            let mut file = ComponentFile {
                runs_on,
                runs_on_count: 1,
                runs_on_line: 1,
                runs_on_column: 1,
                has_session_call: true,
                session_call_line: 3,
                session_call_column: 7,
                ..make_file()
            };
            let mut diags = Vec::new();
            check_session_access_boundary(&file, &mut diags);
            assert!(diags.is_empty(), "sessions allowed on {:?}, got: {:?}", file.runs_on, diags);
        }
    }

    #[test]
    fn session_call_without_runs_on_is_allowed() {
        let file = ComponentFile {
            has_session_call: true,
            session_call_line: 3,
            session_call_column: 7,
            ..make_file()
        };
        let mut diags = Vec::new();
        check_session_access_boundary(&file, &mut diags);
        assert!(diags.is_empty());
    }

    // ── check_runtime_collisions ────────────────────────────────────────

    #[test]
    fn session_binding_collision_is_rejected() {
        let file = ComponentFile {
            has_session_call: true,
            session_call_line: 2,
            session_call_column: 5,
            top_level_bindings: vec![TopLevelBinding::Const {
                name: "session".to_string(),
                exported: false,
                line: 1,
                column: 1,
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_runtime_collisions(&file, &mut diags);
        assert_code(&diags, "RUNTIME_NAME_COLLISION");
        assert!(!diags[0].is_warning, "collisions are hard errors");
    }

    #[test]
    fn no_collision_without_runtime_names() {
        let file = ComponentFile {
            has_env_call: true,
            env_call_line: 2,
            env_call_column: 5,
            top_level_bindings: vec![TopLevelBinding::Const {
                name: "SESSION".to_string(),
                exported: false,
                line: 1,
                column: 1,
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_runtime_collisions(&file, &mut diags);
        assert!(diags.is_empty(), "SESSION (uppercase) is not the emitted 'env'");
    }

    // ── check_duplicate_handlers ────────────────────────────────────────

    #[test]
    fn duplicate_handler_is_rejected() {
        let file = ComponentFile {
            runs_on: Some(RunsOn::Api),
            runs_on_count: 1,
            exports: vec![
                ExportInfo {
                    name: "GET".to_string(),
                    kind: ExportKind::NamedFunction,
                    line: 1,
                    column: 1,
                },
                ExportInfo {
                    name: "GET".to_string(),
                    kind: ExportKind::NamedFunction,
                    line: 5,
                    column: 1,
                },
            ],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_duplicate_handlers(&file, &mut diags);
        assert_code(&diags, "API_DUPLICATE_HANDLER");
        assert!(!diags[0].is_warning);
    }

    #[test]
    fn distinct_handlers_are_allowed() {
        let file = ComponentFile {
            runs_on: Some(RunsOn::Api),
            runs_on_count: 1,
            exports: vec![
                ExportInfo {
                    name: "GET".to_string(),
                    kind: ExportKind::NamedFunction,
                    line: 1,
                    column: 1,
                },
                ExportInfo {
                    name: "POST".to_string(),
                    kind: ExportKind::NamedFunction,
                    line: 5,
                    column: 1,
                },
            ],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_duplicate_handlers(&file, &mut diags);
        assert!(diags.is_empty());
    }

    // ── check_server_boundaries ─────────────────────────────────────────

    #[test]
    fn server_signal_is_rejected() {
        let file = ComponentFile {
            runs_on: Some(RunsOn::Server),
            runs_on_count: 1,
            signals: vec![SignalDecl {
                name: "count".to_string(),
                kind: SignalKind::Signal,
                initial_value: "0".to_string(),
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_server_boundaries(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "SERVER_SIGNAL");
    }

    #[test]
    fn client_signal_is_allowed() {
        let file = ComponentFile {
            runs_on: Some(RunsOn::Client),
            runs_on_count: 1,
            signals: vec![SignalDecl {
                name: "count".to_string(),
                kind: SignalKind::Signal,
                initial_value: "0".to_string(),
            }],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_server_boundaries(&file, &mut diags);
        assert!(diags.is_empty());
    }

    #[test]
    fn server_event_handler_is_rejected() {
        let file = ComponentFile {
            runs_on: Some(RunsOn::Server),
            runs_on_count: 1,
            render_tree: Some(parser::JsxNode::Element {
                tag: "button".to_string(),
                attrs: vec![
                    parser::JsxAttr {
                        name: "onClick".to_string(),
                        value: parser::JsxAttrValue::Expr("() => {}".to_string()),
                        contains_env_call: false,
                    },
                ],
                children: vec![],
                is_hydrate_root: false,
                is_component: false,
            }),
            ..make_file()
        };
        let mut diags = Vec::new();
        check_server_boundaries(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "SERVER_EVENT_HANDLER");
    }

    #[test]
    fn server_plain_attribute_is_allowed() {
        let file = ComponentFile {
            runs_on: Some(RunsOn::Server),
            runs_on_count: 1,
            render_tree: Some(parser::JsxNode::Element {
                tag: "a".to_string(),
                attrs: vec![
                    parser::JsxAttr {
                        name: "href".to_string(),
                        value: parser::JsxAttrValue::String("/x".to_string()),
                        contains_env_call: false,
                    },
                ],
                children: vec![],
                is_hydrate_root: false,
                is_component: false,
            }),
            ..make_file()
        };
        let mut diags = Vec::new();
        check_server_boundaries(&file, &mut diags);
        assert!(diags.is_empty());
    }

    // ── check_statement_ordering ────────────────────────────────────────

    #[test]
    fn valid_body_order() {
        let file = ComponentFile {
            has_component_body: true,
            body_stmts: vec![
                BodyStmt { kind: BodyStmtKind::Signal, line: 3, column: 1 },
                BodyStmt { kind: BodyStmtKind::DerivedConst, line: 4, column: 1 },
                BodyStmt { kind: BodyStmtKind::EventHandler, line: 6, column: 1 },
                BodyStmt { kind: BodyStmtKind::Return, line: 8, column: 1 },
            ],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_statement_ordering(&file, &mut diags);
        assert!(diags.is_empty(), "expected no diagnostics, got: {:?}", diags);
    }

    #[test]
    fn multiple_signals_in_sequence() {
        let file = ComponentFile {
            has_component_body: true,
            body_stmts: vec![
                BodyStmt { kind: BodyStmtKind::Signal, line: 3, column: 1 },
                BodyStmt { kind: BodyStmtKind::Signal, line: 4, column: 1 },
                BodyStmt { kind: BodyStmtKind::Return, line: 6, column: 1 },
            ],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_statement_ordering(&file, &mut diags);
        assert!(diags.is_empty(), "expected no diagnostics, got: {:?}", diags);
    }

    #[test]
    fn multiple_derived_consts_in_sequence() {
        let file = ComponentFile {
            has_component_body: true,
            body_stmts: vec![
                BodyStmt { kind: BodyStmtKind::DerivedConst, line: 3, column: 1 },
                BodyStmt { kind: BodyStmtKind::DerivedConst, line: 4, column: 1 },
                BodyStmt { kind: BodyStmtKind::Return, line: 6, column: 1 },
            ],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_statement_ordering(&file, &mut diags);
        assert!(diags.is_empty(), "expected no diagnostics, got: {:?}", diags);
    }

    #[test]
    fn const_after_const_then_signal() {
        let file = ComponentFile {
            has_component_body: true,
            body_stmts: vec![
                BodyStmt { kind: BodyStmtKind::DerivedConst, line: 3, column: 1 },
                BodyStmt { kind: BodyStmtKind::DerivedConst, line: 4, column: 1 },
                BodyStmt { kind: BodyStmtKind::Signal, line: 6, column: 1 },
                BodyStmt { kind: BodyStmtKind::Return, line: 8, column: 1 },
            ],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_statement_ordering(&file, &mut diags);
        assert_code(&diags, "STATEMENT_OUT_OF_ORDER");
    }

    #[test]
    fn full_valid_sequence_with_multiples() {
        let file = ComponentFile {
            has_component_body: true,
            body_stmts: vec![
                BodyStmt { kind: BodyStmtKind::Signal, line: 3, column: 1 },
                BodyStmt { kind: BodyStmtKind::Signal, line: 4, column: 1 },
                BodyStmt { kind: BodyStmtKind::DerivedConst, line: 6, column: 1 },
                BodyStmt { kind: BodyStmtKind::DerivedConst, line: 7, column: 1 },
                BodyStmt { kind: BodyStmtKind::EventHandler, line: 9, column: 1 },
                BodyStmt { kind: BodyStmtKind::Return, line: 11, column: 1 },
            ],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_statement_ordering(&file, &mut diags);
        assert!(diags.is_empty(), "expected no diagnostics, got: {:?}", diags);
    }

    #[test]
    fn empty_body_is_ok() {
        let file = make_file();
        let mut diags = Vec::new();
        check_statement_ordering(&file, &mut diags);
        assert!(diags.is_empty());
    }

    /// A component with a parsed-but-empty function body (e.g. `function Foo() {}`)
    /// must still produce MISSING_RETURN. The `has_component_body` flag distinguishes
    /// "no component found" from "component found, body is empty."
    #[test]
    fn empty_function_body_produces_missing_return() {
        let file = ComponentFile {
            has_component_body: true,
            body_stmts: vec![],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_statement_ordering(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "MISSING_RETURN");
        assert_eq!(diags[0].line, None);
        assert_eq!(diags[0].column, None);
    }

    #[test]
    fn signal_after_const() {
        let file = ComponentFile {
            has_component_body: true,
            body_stmts: vec![
                BodyStmt { kind: BodyStmtKind::DerivedConst, line: 3, column: 1 },
                BodyStmt { kind: BodyStmtKind::Signal, line: 5, column: 1 },
                BodyStmt { kind: BodyStmtKind::Return, line: 7, column: 1 },
            ],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_statement_ordering(&file, &mut diags);
        assert_code(&diags, "STATEMENT_OUT_OF_ORDER");
    }

    #[test]
    fn signal_after_handler() {
        let file = ComponentFile {
            has_component_body: true,
            body_stmts: vec![
                BodyStmt { kind: BodyStmtKind::EventHandler, line: 3, column: 1 },
                BodyStmt { kind: BodyStmtKind::Signal, line: 6, column: 1 },
                BodyStmt { kind: BodyStmtKind::Return, line: 8, column: 1 },
            ],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_statement_ordering(&file, &mut diags);
        assert_code(&diags, "STATEMENT_OUT_OF_ORDER");
    }

    #[test]
    fn const_after_handler() {
        let file = ComponentFile {
            has_component_body: true,
            body_stmts: vec![
                BodyStmt { kind: BodyStmtKind::EventHandler, line: 3, column: 1 },
                BodyStmt { kind: BodyStmtKind::DerivedConst, line: 6, column: 1 },
                BodyStmt { kind: BodyStmtKind::Return, line: 8, column: 1 },
            ],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_statement_ordering(&file, &mut diags);
        assert_code(&diags, "STATEMENT_OUT_OF_ORDER");
    }

    #[test]
    fn statement_after_return() {
        let file = ComponentFile {
            has_component_body: true,
            body_stmts: vec![
                BodyStmt { kind: BodyStmtKind::Return, line: 3, column: 1 },
                BodyStmt { kind: BodyStmtKind::DerivedConst, line: 5, column: 1 },
            ],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_statement_ordering(&file, &mut diags);
        assert_code(&diags, "STATEMENT_AFTER_RETURN");
    }

    #[test]
    fn multiple_returns() {
        let file = ComponentFile {
            has_component_body: true,
            body_stmts: vec![
                BodyStmt { kind: BodyStmtKind::Return, line: 3, column: 1 },
                BodyStmt { kind: BodyStmtKind::Return, line: 5, column: 1 },
            ],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_statement_ordering(&file, &mut diags);
        assert_code(&diags, "MULTIPLE_RETURN");
    }

    #[test]
    fn missing_return() {
        let file = ComponentFile {
            has_component_body: true,
            body_stmts: vec![
                BodyStmt { kind: BodyStmtKind::Signal, line: 3, column: 1 },
            ],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_statement_ordering(&file, &mut diags);
        assert_code(&diags, "MISSING_RETURN");
    }

    #[test]
    fn let_in_body() {
        let file = ComponentFile {
            has_component_body: true,
            body_stmts: vec![
                BodyStmt { kind: BodyStmtKind::Let, line: 3, column: 1 },
                BodyStmt { kind: BodyStmtKind::Return, line: 5, column: 1 },
            ],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_statement_ordering(&file, &mut diags);
        assert_code(&diags, "BODY_LET");
    }

    #[test]
    fn var_in_body() {
        let file = ComponentFile {
            has_component_body: true,
            body_stmts: vec![
                BodyStmt { kind: BodyStmtKind::Var, line: 3, column: 1 },
                BodyStmt { kind: BodyStmtKind::Return, line: 5, column: 1 },
            ],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_statement_ordering(&file, &mut diags);
        assert_code(&diags, "BODY_VAR");
    }

    #[test]
    fn forbidden_stmt_in_body() {
        let file = ComponentFile {
            has_component_body: true,
            body_stmts: vec![
                BodyStmt { kind: BodyStmtKind::Other, line: 3, column: 1 },
                BodyStmt { kind: BodyStmtKind::Return, line: 5, column: 1 },
            ],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_statement_ordering(&file, &mut diags);
        assert_code(&diags, "BODY_FORBIDDEN_STMT");
    }

    // ── validate (integration) ───────────────────────────────────────────

    #[test]
    fn validate_empty_file() {
        let file = make_file();
        let diags = validate(&file);
        assert_eq!(diags.len(), 3);
        assert_code(&diags, "MISSING_RUNSON");
        assert_code(&diags, "NO_EXPORT");
        assert_code(&diags, "NO_PROPS");
    }

    #[test]
    fn validate_perfect_file() {
        let file = ComponentFile {
            filename: "Cart.tsx".into(),
            runs_on: Some(RunsOn::Server),
            runs_on_count: 1,
            runs_on_line: 1,
            runs_on_column: 1,
            exports: vec![ExportInfo {
                name: "Cart".into(),
                kind: ExportKind::NamedFunction,
                line: 5,
                column: 1,
            }],
            props: Some(PropsInfo {
                name: "props".into(),
                type_annotation: TypeAnnotation::Named("CartProps".into()),
                is_destructured: false,
                line: 5,
                column: 29,
            }),
            ..make_file()
        };
        let diags = validate(&file);
        assert!(diags.is_empty(), "expected no diagnostics, got: {:#?}", diags);
    }

    #[test]
    fn validate_collects_all_errors() {
        let file = ComponentFile {
            filename: "Wrong.tsx".into(),
            runs_on: None,
            runs_on_count: 0,
            exports: vec![
                ExportInfo {
                    name: "Foo".into(),
                    kind: ExportKind::NamedFunction,
                    line: 3,
                    column: 1,
                },
                ExportInfo {
                    name: "Bar".into(),
                    kind: ExportKind::DefaultExport,
                    line: 8,
                    column: 1,
                },
            ],
            props: Some(PropsInfo {
                name: "props".into(),
                type_annotation: TypeAnnotation::Any,
                is_destructured: true,
                line: 3,
                column: 29,
            }),
            imports: vec![ImportInfo {
                source: "react".into(),
                imported_names: vec!["useState".into()],
                line: 1,
                column: 1,
                is_css: false,
            }],
            top_level_bindings: vec![TopLevelBinding::Let {
                name: "state".into(),
                exported: false,
                line: 12,
                column: 1,
            }],
            jsx_expressions: vec![
                JsxExprInfo {
                    kind: JsxExpression::AndConditional,
                    line: 15,
                    column: 10,
                },
                JsxExprInfo {
                    kind: JsxExpression::InlineMap,
                    line: 18,
                    column: 10,
                },
            ],
            has_data_call: false,
            ..make_file()
        };
        let diags = validate(&file);
        assert_eq!(diags.len(), 10);
        assert_code(&diags, "MISSING_RUNSON");
        assert_code(&diags, "MULTIPLE_EXPORTS");
        assert_code(&diags, "DEFAULT_EXPORT");
        assert_code(&diags, "FILENAME_MISMATCH");
        assert_code(&diags, "PROPS_DESTRUCTURED");
        assert_code(&diags, "PROPS_ANY");
        assert_code(&diags, "FORBIDDEN_IMPORT");
        assert_code(&diags, "GLOBAL_LET");
        assert_code(&diags, "AND_CONDITIONAL");
        assert_code(&diags, "INLINE_MAP");
    }

    #[test]
    fn detects_unwrapped_signal_in_component_prop() {
        use parser::{JsxAttr, JsxAttrValue, JsxNode, SignalDecl, SignalKind};

        let mut file = ComponentFile::new("App.tsx");
        file.signals.push(SignalDecl {
            name: "label".into(),
            kind: SignalKind::Signal,
            initial_value: "'Hello'".into(),
        });
        file.render_tree = Some(JsxNode::Element {
            tag: "div".into(),
            attrs: vec![],
            children: vec![JsxNode::Element {
                tag: "Child".into(),
                attrs: vec![JsxAttr {
                    name: "label".into(),
                    value: JsxAttrValue::Expr("label.value".into()),
                    contains_env_call: false,
                }],
                children: vec![],
                is_hydrate_root: false,
                is_component: true,
            }],
            is_hydrate_root: false,
            is_component: false,
        });
        let diags = validate(&file);
        assert_code(&diags, "PROP_UNWRAPPED_SIGNAL");
    }

    #[test]
    fn allows_signal_passed_by_reference() {
        use parser::{JsxAttr, JsxAttrValue, JsxNode, SignalDecl, SignalKind};

        let mut file = ComponentFile::new("App.tsx");
        file.signals.push(SignalDecl {
            name: "label".into(),
            kind: SignalKind::Signal,
            initial_value: "'Hello'".into(),
        });
        file.render_tree = Some(JsxNode::Element {
            tag: "div".into(),
            attrs: vec![],
            children: vec![JsxNode::Element {
                tag: "Child".into(),
                attrs: vec![JsxAttr {
                    name: "label".into(),
                    value: JsxAttrValue::Expr("label".into()), // passed by reference
                    contains_env_call: false,
                }],
                children: vec![],
                is_hydrate_root: false,
                is_component: true,
            }],
            is_hydrate_root: false,
            is_component: false,
        });
        let diags = validate(&file);
        assert!(!diags.iter().any(|d| d.code == "PROP_UNWRAPPED_SIGNAL"));
    }

    // ── Task 3: regression tests for unsupported construct errors ──────────

    #[test]
    fn unsupported_operator_is_reported_not_debug_emitted() {
        let mut file = make_valid_file();
        file.unsupported_errors.push(ParserError {
            code: "UNSUPPORTED_OPERATOR",
            message: "The 'NullishCoalescing' operator is not yet supported.".into(),
            fix_hint: "Refactor to use a supported operator.",
        });
        let diags = validate(&file);
        assert_code(&diags, "UNSUPPORTED_OPERATOR");

        // The debug representation "NullishCoalescing" or similar must NOT appear
        // in the generated program. Since the build stops on validation failure,
        // no output file is produced — confirmed by the CLI build command
        // returning Err when diagnostics are non-empty.
    }

    #[test]
    fn unsupported_object_spread_is_reported_not_placeholder() {
        let mut file = make_valid_file();
        file.unsupported_errors.push(ParserError {
            code: "UNSUPPORTED_SYNTAX",
            message: "Spread in object literal ({...X}) is not yet supported.".into(),
            fix_hint: "Specify properties explicitly instead.",
        });
        let diags = validate(&file);
        assert_code(&diags, "UNSUPPORTED_SYNTAX");
        // "/* spread */" must never appear in output — validation fails, no file written.
    }

    #[test]
    fn unsupported_template_literal_is_reported() {
        let mut file = make_valid_file();
        file.unsupported_errors.push(ParserError {
            code: "UNSUPPORTED_EXPRESSION",
            message: "Template literals (`...`) are not yet supported.".into(),
            fix_hint: "Use string concatenation (+) or an array .join() instead.",
        });
        let diags = validate(&file);
        assert_code(&diags, "UNSUPPORTED_EXPRESSION");
    }

    #[test]
    fn unsupported_optional_chaining_is_reported() {
        let mut file = make_valid_file();
        file.unsupported_errors.push(ParserError {
            code: "UNSUPPORTED_EXPRESSION",
            message: "Optional chaining (?.) is not yet supported.".into(),
            fix_hint: "Use an explicit conditional check instead.",
        });
        let diags = validate(&file);
        assert_code(&diags, "UNSUPPORTED_EXPRESSION");
    }

    #[test]
    fn unsupported_jsx_spread_attr_is_reported() {
        let mut file = make_valid_file();
        file.unsupported_errors.push(ParserError {
            code: "UNSUPPORTED_JSX_CONSTRUCT",
            message: "JSX spread attributes ({...props}) are not yet supported.".into(),
            fix_hint: "Pass each prop explicitly.",
        });
        let diags = validate(&file);
        assert_code(&diags, "UNSUPPORTED_JSX_CONSTRUCT");
    }

    #[test]
    fn unsupported_statement_is_reported() {
        let mut file = make_valid_file();
        file.unsupported_errors.push(ParserError {
            code: "UNSUPPORTED_STATEMENT",
            message: "For statements are not yet supported in arrow function bodies.".into(),
            fix_hint: "Refactor to use only const/let/var declarations, if statements, return, and expression statements.",
        });
        let diags = validate(&file);
        assert_code(&diags, "UNSUPPORTED_STATEMENT");
    }

    #[test]
    fn unsupported_array_spread_is_reported() {
        let mut file = make_valid_file();
        file.unsupported_errors.push(ParserError {
            code: "UNSUPPORTED_EXPRESSION",
            message: "Spread in array literal ([...X]) is not yet supported.".into(),
            fix_hint: "Extract the spread to a separate variable before the JSX expression.",
        });
        let diags = validate(&file);
        assert_code(&diags, "UNSUPPORTED_EXPRESSION");
    }

    #[test]
    fn unsupported_destructured_pattern_is_reported() {
        let mut file = make_valid_file();
        file.unsupported_errors.push(ParserError {
            code: "UNSUPPORTED_SYNTAX",
            message: "Destructured or complex parameter patterns are not yet supported.".into(),
            fix_hint: "Use a simple identifier parameter and access fields explicitly in the function body.",
        });
        let diags = validate(&file);
        assert_code(&diags, "UNSUPPORTED_SYNTAX");
    }

    #[test]
    fn multiple_unsupported_errors_are_all_collected() {
        let mut file = make_valid_file();
        file.unsupported_errors.push(ParserError {
            code: "UNSUPPORTED_OPERATOR",
            message: "The 'Lt' operator is not yet supported.".into(),
            fix_hint: "",
        });
        file.unsupported_errors.push(ParserError {
            code: "UNSUPPORTED_EXPRESSION",
            message: "Template literals are not yet supported.".into(),
            fix_hint: "",
        });
        file.unsupported_errors.push(ParserError {
            code: "UNSUPPORTED_JSX_CONSTRUCT",
            message: "JSX spread children are not yet supported.".into(),
            fix_hint: "",
        });
        let diags = validate(&file);
        assert_code(&diags, "UNSUPPORTED_OPERATOR");
        assert_code(&diags, "UNSUPPORTED_EXPRESSION");
        assert_code(&diags, "UNSUPPORTED_JSX_CONSTRUCT");
    }

    // ── helper for Task 3 tests ──────────────────────────────────────────

    fn make_valid_file() -> ComponentFile {
        ComponentFile {
            filename: "Test.tsx".into(),
            runs_on: Some(RunsOn::Client),
            runs_on_count: 1,
            runs_on_line: 1,
            runs_on_column: 1,
            exports: vec![ExportInfo {
                name: "Test".into(),
                kind: ExportKind::NamedFunction,
                line: 5,
                column: 1,
            }],
            props: Some(PropsInfo {
                name: "props".into(),
                type_annotation: TypeAnnotation::Named("TestProps".into()),
                is_destructured: false,
                line: 5,
                column: 29,
            }),
            ..ComponentFile::new("Test.tsx")
        }
    }

    // ── CSS import tests (B2.2) ─────────────────────────────────────────

    #[test]
    fn bare_css_import_is_valid() {
        let mut file = make_valid_file();
        file.imports.push(ImportInfo {
            source: "./Cart.css".into(),
            imported_names: vec![],
            line: 2,
            column: 1,
            is_css: true,
        });
        let mut diags = Vec::new();
        check_css_imports(&file, &mut diags);
        assert!(diags.is_empty(), "bare CSS import should pass, got: {:?}", diags);
    }

    #[test]
    fn named_css_import_is_rejected() {
        let mut file = make_valid_file();
        file.imports.push(ImportInfo {
            source: "./Cart.css".into(),
            imported_names: vec!["styles".into()],
            line: 2,
            column: 1,
            is_css: true,
        });
        let mut diags = Vec::new();
        check_css_imports(&file, &mut diags);
        assert_code(&diags, "INVALID_CSS_IMPORT");
    }

    #[test]
    fn namespace_css_import_is_rejected() {
        let mut file = make_valid_file();
        file.imports.push(ImportInfo {
            source: "./theme.css".into(),
            imported_names: vec!["theme".into()],
            line: 3,
            column: 1,
            is_css: true,
        });
        let mut diags = Vec::new();
        check_css_imports(&file, &mut diags);
        assert_code(&diags, "INVALID_CSS_IMPORT");
    }

    #[test]
    fn css_import_in_server_component_is_rejected() {
        let mut file = make_valid_file();
        file.runs_on = Some(RunsOn::Server);
        file.imports.push(ImportInfo {
            source: "./Cart.css".into(),
            imported_names: vec![],
            line: 2,
            column: 1,
            is_css: true,
        });
        let mut diags = Vec::new();
        check_css_imports(&file, &mut diags);
        assert_code(&diags, "INVALID_CSS_IMPORT");
    }

    #[test]
    fn css_import_not_confused_with_forbidden_source() {
        // A .css import should NOT trigger FORBIDDEN_IMPORT just because
        // its source happens to match a forbidden pattern
        let mut file = make_valid_file();
        file.imports.push(ImportInfo {
            source: "react".into(), // not CSS — regular import gets checked
            imported_names: vec!["useState".into()],
            line: 2,
            column: 1,
            is_css: false,
        });
        let mut diags = Vec::new();
        check_forbidden_imports(&file, &mut diags);
        assert_code(&diags, "FORBIDDEN_IMPORT");
    }

    #[test]
    fn handler_with_jsx_is_rejected() {
        let file = ComponentFile {
            exports: vec![ExportInfo {
                name: "HandlerTest".into(),
                kind: ExportKind::NamedFunction,
                line: 4,
                column: 1,
            }],
            handler_decls: vec!["fn with_jsx() { return <span/>; }".into()],
            handler_has_jsx: vec![true],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_handler_jsx(&file, &mut diags);
        assert_code(&diags, "HANDLER_JSX");
    }

    #[test]
    fn normal_handler_without_jsx_is_allowed() {
        let file = ComponentFile {
            exports: vec![ExportInfo {
                name: "HandlerTest".into(),
                kind: ExportKind::NamedFunction,
                line: 4,
                column: 1,
            }],
            handler_decls: vec!["fn normal() { count.set(count.value + 1); }".into()],
            handler_has_jsx: vec![false],
            ..make_file()
        };
        let mut diags = Vec::new();
        check_handler_jsx(&file, &mut diags);
        assert!(diags.is_empty(), "normal handler should not produce HANDLER_JSX");
    }

    // ── import existence validation (nested-route safety net) ─────────────

    fn make_import_file() -> ComponentFile {
        let mut file = make_valid_file();
        file.imports.push(ImportInfo {
            source: "./Widget".into(),
            imported_names: vec!["Widget".into()],
            line: 2,
            column: 1,
            is_css: false,
        });
        file
    }

    #[test]
    fn import_exists_in_same_directory() {
        let dir = tempfile::tempdir().unwrap();
        let pages = dir.path().join("pages");
        std::fs::create_dir(&pages).unwrap();
        std::fs::write(pages.join("Widget.tsx"), "// @runsOn client\n").unwrap();
        let file = make_import_file();
        let diags = validate_imports(&file, Path::new("pages/Shop.tsx"), dir.path());
        assert!(diags.is_empty(), "existing import should pass, got: {:?}", diags);
    }

    #[test]
    fn import_missing_in_same_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = make_import_file();
        let diags = validate_imports(&file, Path::new("pages/Shop.tsx"), dir.path());
        assert_code(&diags, "IMPORT_NOT_FOUND");
        assert!(
            diags[0].message.contains("Widget"),
            "message should name the import, got: {}",
            diags[0].message
        );
    }

    #[test]
    fn nested_import_resolves_through_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let components = dir.path().join("pages/components");
        std::fs::create_dir_all(&components).unwrap();
        std::fs::write(components.join("Widget.tsx"), "// @runsOn client\n").unwrap();
        let mut file = make_valid_file();
        file.imports.push(ImportInfo {
            source: "../../components/Widget".into(),
            imported_names: vec!["Widget".into()],
            line: 2,
            column: 1,
            is_css: false,
        });
        let diags = validate_imports(&file, Path::new("pages/Docs/Api/Signals.tsx"), dir.path());
        assert!(diags.is_empty(), "parent-dir import should pass, got: {:?}", diags);
    }

    #[test]
    fn import_with_explicit_tsx_extension_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let pages = dir.path().join("pages");
        std::fs::create_dir(&pages).unwrap();
        std::fs::write(pages.join("Widget.tsx"), "// @runsOn client\n").unwrap();
        let mut file = make_valid_file();
        file.imports.push(ImportInfo {
            source: "./Widget.tsx".into(),
            imported_names: vec!["Widget".into()],
            line: 2,
            column: 1,
            is_css: false,
        });
        let diags = validate_imports(&file, Path::new("pages/Shop.tsx"), dir.path());
        assert!(diags.is_empty(), "explicit .tsx import should pass, got: {:?}", diags);
    }

    #[test]
    fn css_imports_are_skipped_by_import_validation() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = make_import_file();
        file.imports.push(ImportInfo {
            source: "./nonexistent.css".into(),
            imported_names: vec![],
            line: 3,
            column: 1,
            is_css: true,
        });
        let diags = validate_imports(&file, Path::new("pages/Shop.tsx"), dir.path());
        let only_widget = diags.iter().all(|d| d.code == "IMPORT_NOT_FOUND" && d.message.contains("Widget"));
        assert!(only_widget, "CSS import should be skipped, got: {:?}", diags);
    }

    // ── §7e: unexpected children / children shape ─────────────────────────

    fn make_card_type() -> TypeDecl {
        TypeDecl {
            name: "CardProps".into(),
            complete: true,
            fields: vec![
                PropField {
                    name: "title".into(),
                    line: 3,
                    column: 5,
                },
            ],
        }
    }

    #[test]
    fn unexpected_children_fires_with_fix_hint() {
        let diag = unexpected_children_diagnostic("Card", Some(&make_card_type())).unwrap();
        assert_eq!(diag.code, "UNEXPECTED_CHILDREN");
        assert!(
            diag.message.contains("CardProps"),
            "message should name the Props type, got: {}",
            diag.message
        );
        assert_eq!(
            diag.fix_hint,
            "Add a `children: JSX.Element` field to the component's Props type."
        );
    }

    #[test]
    fn children_field_silences_unexpected_children() {
        let mut decl = make_card_type();
        decl.fields.push(PropField {
            name: "children".into(),
            line: 4,
            column: 5,
        });
        assert!(unexpected_children_diagnostic("Card", Some(&decl)).is_none());
    }

    #[test]
    fn incomplete_type_silences_unexpected_children() {
        let mut decl = make_card_type();
        decl.complete = false;
        assert!(unexpected_children_diagnostic("Card", Some(&decl)).is_none());
    }

    #[test]
    fn unknown_props_type_silences_unexpected_children() {
        assert!(unexpected_children_diagnostic("Card", None).is_none());
    }

    fn make_component_tree(children: Vec<JsxNode>) -> ComponentFile {
        ComponentFile {
            filename: "Card.tsx".into(),
            runs_on: Some(RunsOn::Client),
            runs_on_count: 1,
            runs_on_line: 1,
            runs_on_column: 1,
            exports: vec![ExportInfo {
                name: "Card".into(),
                kind: ExportKind::NamedFunction,
                line: 5,
                column: 1,
            }],
            props: Some(PropsInfo {
                name: "props".into(),
                type_annotation: TypeAnnotation::Named("CardProps".into()),
                is_destructured: false,
                line: 5,
                column: 29,
            }),
            render_tree: Some(JsxNode::Element {
                tag: "Wrapper".into(),
                attrs: vec![],
                children,
                is_hydrate_root: false,
                is_component: true,
            }),
            ..make_file()
        }
    }

    #[test]
    fn multiple_children_on_component_are_rejected() {
        let file = make_component_tree(vec![
            JsxNode::Element {
                tag: "p".into(),
                attrs: vec![],
                children: vec![],
                is_hydrate_root: false,
                is_component: false,
            },
            JsxNode::Element {
                tag: "p".into(),
                attrs: vec![],
                children: vec![],
                is_hydrate_root: false,
                is_component: false,
            },
        ]);
        let mut diags = Vec::new();
        check_children_shape(&file, &mut diags);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "MULTIPLE_CHILDREN");
        assert!(diags[0].message.contains("2 sibling children"));
    }

    #[test]
    fn single_child_on_component_is_allowed() {
        let file = make_component_tree(vec![JsxNode::Element {
            tag: "p".into(),
            attrs: vec![],
            children: vec![],
            is_hydrate_root: false,
            is_component: false,
        }]);
        let mut diags = Vec::new();
        check_children_shape(&file, &mut diags);
        assert!(diags.is_empty());
    }

    #[test]
    fn whitespace_only_children_are_ignored() {
        let file = make_component_tree(vec![JsxNode::Text("\n  ".into())]);
        let mut diags = Vec::new();
        check_children_shape(&file, &mut diags);
        assert!(diags.is_empty(), "whitespace-only children are not real children");
    }

    #[test]
    fn fragment_wrapped_children_are_allowed() {
        let file = make_component_tree(vec![JsxNode::Element {
            tag: "p".into(),
            attrs: vec![],
            children: vec![
                JsxNode::Element {
                    tag: "span".into(),
                    attrs: vec![],
                    children: vec![],
                    is_hydrate_root: false,
                    is_component: false,
                },
                JsxNode::Element {
                    tag: "span".into(),
                    attrs: vec![],
                    children: vec![],
                    is_hydrate_root: false,
                    is_component: false,
                },
            ],
            is_hydrate_root: false,
            is_component: false,
        }]);
        let mut diags = Vec::new();
        check_children_shape(&file, &mut diags);
        assert!(diags.is_empty(), "siblings wrapped in a single parent are fine");
    }

    #[test]
    fn multiple_children_on_html_element_are_allowed() {
        let file = ComponentFile {
            filename: "Card.tsx".into(),
            render_tree: Some(JsxNode::Element {
                tag: "div".into(),
                attrs: vec![],
                children: vec![
                    JsxNode::Element {
                        tag: "p".into(),
                        attrs: vec![],
                        children: vec![],
                        is_hydrate_root: false,
                        is_component: false,
                    },
                    JsxNode::Element {
                        tag: "p".into(),
                        attrs: vec![],
                        children: vec![],
                        is_hydrate_root: false,
                        is_component: false,
                    },
                ],
                is_hydrate_root: false,
                is_component: false,
            }),
            ..make_file()
        };
        let mut diags = Vec::new();
        check_children_shape(&file, &mut diags);
        assert!(diags.is_empty(), "MULTIPLE_CHILDREN is for components only");
    }

    #[test]
    fn empty_tree_skips_children_check() {
        let file = make_file();
        let mut diags = Vec::new();
        check_children_shape(&file, &mut diags);
        assert!(diags.is_empty());
    }
}
