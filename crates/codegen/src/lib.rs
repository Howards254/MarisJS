use parser::{ComponentFile, JsxAttrValue, JsxNode, RunsOn, SignalKind};

pub fn generate(component: &ComponentFile) -> Result<String, String> {
    if component.runs_on == Some(RunsOn::Server) {
        generate_server(component)
    } else {
        generate_client(component)
    }
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

    let has_signal = component.signals.iter().any(|s| s.kind == SignalKind::Signal);
    let has_computed = component.signals.iter().any(|s| s.kind == SignalKind::Computed);
    let needs_bind = tree_reads_signal(render_tree, &signal_names)
        || has_for_each_or_conditional(render_tree);

    let mut imports = Vec::new();
    if has_signal { imports.push("signal"); }
    if has_computed { imports.push("computed"); }
    if needs_bind { imports.push("bind"); }

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

    let root_var = gen_node(render_tree, &mut output, &mut counter, 1, &signal_names)?;

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

fn generate_server(component: &ComponentFile) -> Result<String, String> {
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

    let mut output = String::new();

    let comp_imports = collect_component_imports(render_tree, &component.imports);
    for (name, source) in &comp_imports {
        output.push_str(&format!("import {{ {} }} from '{}';\n", name, source));
    }
    if !comp_imports.is_empty() {
        output.push('\n');
    }

    if has_data {
        output.push_str("import { data } from '@marisjs/runtime';\n\n");
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

    if hydrates.is_empty() {
        output.push_str("  return ");
        gen_html_node(render_tree, &mut output, has_data)?;
        output.push_str(";\n");
    } else {
        output.push_str("  const _html = ");
        gen_html_node(render_tree, &mut output, has_data)?;
        output.push_str(";\n");
        output.push_str("  return { html: _html, clientBundles: [");
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
            if *is_hydrate_root { names.push(tag.clone()); }
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
            output.push_str(expr);
        }
        JsxNode::Element { tag, attrs, children, is_hydrate_root, is_component } => {
            if *is_hydrate_root {
                output.push_str(&format!("'<div data-hydrate=\"{}\"></div>'", tag));
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
            let attr_str = build_attr_string(attrs);
            let open = if attr_str.is_empty() {
                format!("<{}>", tag)
            } else {
                format!("<{} {}>", tag, attr_str)
            };
            let close = format!("</{}>", tag);
            output.push_str(&format!("('{}'", html_escape(&open)));
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

fn build_attr_string(attrs: &[parser::JsxAttr]) -> String {
    let mut s = String::new();
    for attr in attrs {
        if attr.name.starts_with("on") && attr.name.chars().nth(2).map_or(false, |c| c.is_uppercase()) {
            continue;
        }
        match &attr.value {
            JsxAttrValue::String(value) => {
                s.push_str(&format!(" {}=\"{}\"", attr.name, html_attr_escape(value)));
            }
            JsxAttrValue::Expr(expr) => {
                s.push_str(&format!(" {}=\"${{{}}}\"", attr.name, expr));
            }
        }
    }
    s
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

fn tree_reads_signal(node: &JsxNode, signal_names: &[String]) -> bool {
    match node {
        JsxNode::Expr(text) => is_reactive_expr(text, signal_names),
        JsxNode::Conditional { test, .. } => is_reactive_expr(test, signal_names),
        JsxNode::ForEach { each, .. } => is_reactive_expr(each, signal_names),
        JsxNode::Element { children, attrs, .. } => {
            children.iter().any(|c| tree_reads_signal(c, signal_names))
                || attrs.iter().any(|a| match &a.value {
                    JsxAttrValue::Expr(text) => is_reactive_expr(text, signal_names),
                    _ => false,
                })
        }
        _ => false,
    }
}

fn reads_signal(expr: &str, signal_names: &[String]) -> bool {
    for name in signal_names {
        if expr.contains(&format!("{}.value", name)) { return true; }
    }
    false
}

fn is_reactive_expr(expr: &str, signal_names: &[String]) -> bool {
    reads_signal(expr, signal_names) || expr.contains(".value")
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
) -> Result<String, String> {
    match node {
        JsxNode::Conditional { test, cons, alt } =>
            gen_conditional(test, cons, alt, output, counter, indent, signal_names),
        JsxNode::ForEach { each, key_fn, item_param, body, for_body_decls } =>
            gen_for_each(each, key_fn, item_param, body, for_body_decls, output, counter, indent, signal_names),
        JsxNode::Element { tag, attrs, children, is_hydrate_root, is_component } =>
            gen_element(tag, attrs, children, *is_hydrate_root, *is_component, output, counter, indent, signal_names),
        JsxNode::Text(text) => Ok(gen_text(text, output, counter, indent)),
        JsxNode::Expr(expr) => Ok(gen_expr(expr, output, counter, indent, signal_names)),
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
) -> Result<String, String> {
    if is_component {
        return gen_component_call(tag, attrs, output, counter, indent, signal_names);
    }
    let var = format!("el{}", counter.next());

    if tag.is_empty() {
        if children.len() == 1 {
            return gen_node(&children[0], output, counter, indent, signal_names);
        }
        writeln(output, indent, &format!("const {} = document.createDocumentFragment();", var));
        for child in children {
            let cv = gen_node(child, output, counter, indent, signal_names)?;
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
                    if is_reactive_expr(expr, signal_names) {
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
                    let dom_op = if needs_property_assignment(&attr.name) {
                        format!("{}.{} = {}", var, attr.name, expr)
                    } else {
                        format!("{}.setAttribute('{}', {})", var, attr.name, expr)
                    };
                    if is_reactive_expr(expr, signal_names) {
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
                if is_reactive_expr(expr, signal_names) {
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
                    let cv = gen_node(child, output, counter, indent, signal_names)?;
                    writeln(output, indent, &format!("{}.appendChild({});", var, cv));
                }
                JsxNode::Text(text) => {
                    let tv = gen_text(text, output, counter, indent);
                    writeln(output, indent, &format!("{}.appendChild({});", var, tv));
                }
                JsxNode::Expr(expr) => {
                    let tv = gen_expr_text_node(expr, output, counter, indent, signal_names);
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

fn gen_expr(expr: &str, output: &mut String, counter: &mut AtomicCounter, indent: usize, signal_names: &[String]) -> String {
    let var = format!("txt{}", counter.next());
    writeln(output, indent, &format!("const {} = document.createTextNode({});", var, expr));
    if is_reactive_expr(expr, signal_names) {
        writeln(output, indent, &format!("bind(() => {{ {}.nodeValue = {}; }});", var, expr));
    }
    var
}

fn gen_expr_text_node(expr: &str, output: &mut String, counter: &mut AtomicCounter, indent: usize, signal_names: &[String]) -> String {
    gen_expr(expr, output, counter, indent, signal_names)
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
) -> Result<String, String> {
    let anchor_var = format!("_an{}", counter.next());
    let frag_var = format!("_fr{}", counter.next());
    let curr_var = format!("_cu{}", counter.next());

    let true_var = gen_node(cons, output, counter, indent + 1, signal_names)?;
    let false_var = gen_node(alt, output, counter, indent + 1, signal_names)?;

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
    let body_var = gen_node(body, output, counter, render_indent, signal_names)?;
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
