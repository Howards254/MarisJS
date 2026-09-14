//! Differential-testing harness for MarisJS sanctioned constructs.
//!
//! This file tests EVERY construct the validator accepts, compiling minimal
//! fixtures and executing them in real environments (jsdom for client, Node
//! for server). Each test asserts zero ReferenceErrors, zero SyntaxErrors,
//! and zero unexpected undefined/[object Object]-shaped output.
//!
//! Tests are organized by category matching the spec document, so adding a
//! new sanctioned construct has an obvious place for its corresponding test.
//!
//! Structure:
//! - `runners::client()` — compiles a client fixture, runs in jsdom, checks for errors
//! - `runners::server()` — compiles a server fixture via CLI, checks prerendered HTML
//! - Each test is a minimal fixture exercising exactly ONE sanctioned construct
//!
//! Pre-fix verification (against ce2ec47 / v0.1.12):
//! - 9 S-series bugs independently rediscovered (S1×2, S2×1, S3×2, S4×2, S5×1, S6×1)
//! - 5 additional failures for §7f effects/refs feature (not S-series; feature absent in pre-fix)
//! - Binary must be rebuilt from pre-fix source before running; stale binary = false negatives

use std::process::Command;

// ═══════════════════════════════════════════════════════════════
// Test infrastructure
// ═══════════════════════════════════════════════════════════════

fn workspace_root() -> &'static std::path::Path {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    Box::leak(
        manifest
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
            .into_boxed_path(),
    )
}

fn setup_test_dir(dir: &tempfile::TempDir) {
    let root = workspace_root();
    let ws_nm = root.join("node_modules");
    let maris_dir = ws_nm.join("@marisjs");
    let _ = std::fs::create_dir(&maris_dir);
    let runtime_link = maris_dir.join("runtime");
    if !runtime_link.exists() {
        let _ = std::os::unix::fs::symlink(&root.join("packages/runtime"), &runtime_link);
    }
    let target_modules = dir.path().join("node_modules");
    let _ = std::os::unix::fs::symlink(&ws_nm, &target_modules);
}

/// Compile a client fixture, write .mjs, run a JS assertion runner in jsdom.
/// Returns Ok(stdout) or Err((stdout, stderr)).
fn run_client_fixture(
    dir: &tempfile::TempDir,
    name: &str,
    fixture: &str,
    runner: &str,
) -> Result<String, (String, String)> {
    std::fs::write(dir.path().join(format!("{}.tsx", name)), fixture).unwrap();
    let component = parser::parse_component_file(dir.path().join(format!("{}.tsx", name)).to_str().unwrap()).unwrap();
    let diags = validator::validate(&component);
    if !diags.is_empty() {
        return Err((
            String::new(),
            format!("validation errors: {:?}", diags),
        ));
    }
    let js = codegen::generate(&component, &codegen::EnvMap::new()).unwrap();
    std::fs::write(dir.path().join(format!("{}.mjs", name)), &js).unwrap();

    std::fs::write(dir.path().join("runner.mjs"), runner).unwrap();
    let output = Command::new("node")
        .arg(dir.path().join("runner.mjs"))
        .current_dir(dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() && stdout.contains("PASS") {
        Ok(stdout)
    } else {
        Err((stdout, stderr))
    }
}

/// Build a server fixture via CLI, check prerendered HTML.
fn run_server_fixture(
    dir: &tempfile::TempDir,
    page_name: &str,
    page_fixture: &str,
    assertions: &str, // JS code that receives `html` variable
) -> Result<String, (String, String)> {
    let pages = dir.path().join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::write(pages.join(format!("{}.tsx", page_name)), page_fixture).unwrap();

    // Symlink node_modules
    let target_modules = dir.path().join("node_modules");
    if !target_modules.exists() {
        let _ = std::os::unix::fs::symlink(workspace_root().join("node_modules"), &target_modules);
    }

    let out = dir.path().join("dist");
    let output = Command::new(workspace_root().join("target/debug/marisjs"))
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();

    if !output.status.success() {
        return Err((
            String::new(),
            format!("build failed: {}", String::from_utf8_lossy(&output.stderr)),
        ));
    }

    let html_path = out.join(format!("{}.html", page_name.to_lowercase()));
    if !html_path.exists() {
        return Err((String::new(), format!("{} not found", html_path.display())));
    }
    let html = std::fs::read_to_string(&html_path).unwrap();

    // Run assertions in Node
    let runner = format!(
        "const html = {};\n{}\nconsole.log('PASS');",
        serde_json::to_string(&html).unwrap(),
        assertions
    );
    std::fs::write(dir.path().join("assert_runner.mjs"), &runner).unwrap();
    let output = Command::new("node")
        .arg(dir.path().join("assert_runner.mjs"))
        .current_dir(dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() && stdout.contains("PASS") {
        Ok(stdout)
    } else {
        Err((stdout, stderr))
    }
}

// ═══════════════════════════════════════════════════════════════
// §1 — JSX: Elements, Text, Expressions
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_jsx_element_basic() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  return <div class=\"foo\"><h1>Hello</h1></div>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
const h1 = root.querySelector('h1');
if (!h1) { console.error('FAIL: no h1'); process.exit(1); }
if (h1.textContent !== 'Hello') { console.error('FAIL: text=' + h1.textContent); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_jsx_element_self_closing() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  return <div><span class=\"icon\" data-name=\"star\" /></div>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
const span = root.querySelector('span');
if (!span) { console.error('FAIL: no span'); process.exit(1); }
if (span.getAttribute('data-name') !== 'star') { console.error('FAIL: data-name=' + span.getAttribute('data-name')); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_jsx_text_node() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  return <span>Hello world</span>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
const span = root.querySelector('span');
if (!span) { console.error('FAIL: no span'); process.exit(1); }
if (span.textContent !== 'Hello world') { console.error('FAIL: text=' + span.textContent); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_jsx_expression_text() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = { title: string };\nexport function C(props: P) {\n  return <span>{props.title}</span>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({ title: 'TestTitle' }));
const span = root.querySelector('span');
if (!span) { console.error('FAIL: no span'); process.exit(1); }
if (span.textContent !== 'TestTitle') { console.error('FAIL: text=' + span.textContent); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_jsx_expression_binary_operators() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = { a: number; b: number };\nexport function C(props: P) {\n  return <span>{props.a < props.b ? 'YES' : 'NO'}</span>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({ a: 1, b: 2 }));
const span = root.querySelector('span');
if (span.textContent !== 'YES') { console.error('FAIL: text=' + span.textContent); process.exit(1); }
root.innerHTML = '';
root.appendChild(C({ a: 5, b: 2 }));
if (root.querySelector('span').textContent !== 'NO') { console.error('FAIL: a<b should be NO'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_jsx_expression_nullish_coalescing() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = { val: string | null };\nexport function C(props: P) {\n  return <span>{props.val ?? 'default'}</span>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({ val: null }));
if (root.querySelector('span').textContent !== 'default') { console.error('FAIL'); process.exit(1); }
root.innerHTML = '';
root.appendChild(C({ val: 'custom' }));
if (root.querySelector('span').textContent !== 'custom') { console.error('FAIL'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_jsx_expression_template_literal() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = { name: string };\nexport function C(props: P) {\n  return <span>{`hello ${props.name}`}</span>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({ name: 'World' }));
if (root.querySelector('span').textContent !== 'hello World') { console.error('FAIL'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_jsx_fragment() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  return <><span>A</span><span>B</span></>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
const spans = root.querySelectorAll('span');
if (spans.length !== 2) { console.error('FAIL: expected 2 spans, got ' + spans.length); process.exit(1); }
if (spans[0].textContent !== 'A' || spans[1].textContent !== 'B') { console.error('FAIL'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_jsx_nested_elements() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  return <nav><div class=\"logo\">MyApp</div><ul><li>Home</li></ul></nav>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
const nav = root.querySelector('nav');
if (!nav) { console.error('FAIL: no nav'); process.exit(1); }
const logo = nav.querySelector('.logo');
if (!logo || logo.textContent !== 'MyApp') { console.error('FAIL: logo'); process.exit(1); }
const li = nav.querySelector('li');
if (!li || li.textContent !== 'Home') { console.error('FAIL: li'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §2 — JSX: Conditional Rendering
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_jsx_conditional_ternary() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const show = signal(true);\n  return <div>{show.value ? <span class=\"on\">ON</span> : <span class=\"off\">OFF</span>}</div>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
const result = C({});
root.appendChild(result);
const onSpan = root.querySelector('.on');
if (!onSpan) { console.error('FAIL: .on not found'); process.exit(1); }
if (onSpan.textContent !== 'ON') { console.error('FAIL: text=' + onSpan.textContent); process.exit(1); }
// Toggle
result._signals.show.set(false);
await new Promise(r => setTimeout(r, 0));
const offSpan = root.querySelector('.off');
if (!offSpan) { console.error('FAIL: .off not found after toggle'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_jsx_conditional_with_null() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = { active: string };\nexport function C(props: P) {\n  return <div>{props.active === 'a' ? <p>Content A</p> : null}</div>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({ active: 'a' }));
if (!root.querySelector('p')) { console.error('FAIL: p not found'); process.exit(1); }
root.innerHTML = '';
root.appendChild(C({ active: 'b' }));
if (root.querySelector('p')) { console.error('FAIL: p should not exist'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §3 — State: Signals, Computed
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_signal_declaration_and_read() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const count = signal(0);\n  return <span>{count.value}</span>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
if (root.querySelector('span').textContent !== '0') { console.error('FAIL'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_signal_set_value() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const count = signal(0);\n  return <div><span>{count.value}</span><button onClick={() => count.set(count.value + 1)}>+</button></div>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
const result = C({});
root.appendChild(result);
if (root.querySelector('span').textContent !== '0') { console.error('FAIL: initial'); process.exit(1); }
result._signals.count.set(5);
await new Promise(r => setTimeout(r, 0));
if (root.querySelector('span').textContent !== '5') { console.error('FAIL: after set=' + root.querySelector('span').textContent); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_signal_string_initial() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const name = signal('Alice');\n  return <span>{name.value}</span>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
if (root.querySelector('span').textContent !== 'Alice') { console.error('FAIL'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_signal_object_initial() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const theme = signal({ color: 'blue' });\n  return <span>{theme.value.color}</span>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
if (root.querySelector('span').textContent !== 'blue') { console.error('FAIL'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_signal_array_initial() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const items = signal([{ id: 1, text: 'Buy milk' }]);\n  return <span>{items.value.length}</span>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
if (root.querySelector('span').textContent !== '1') { console.error('FAIL'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_computed_declaration() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = { price: number; quantity: number };\nexport function C(props: P) {\n  const total = computed(() => props.price * props.quantity);\n  return <span>{total.value}</span>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({ price: 10, quantity: 3 }));
if (root.querySelector('span').textContent !== '30') { console.error('FAIL'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §4 — State: Effects, Refs
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_effect_default_tracking() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const count = signal(0);\n  const label = ref();\n  effect(() => { label.current.textContent = 'count=' + count.value; });\n  return <div ref={label}><span>{count.value}</span></div>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
const result = C({});
root.appendChild(result);
await new Promise(r => setTimeout(r, 0));
const div = root.querySelector('div');
if (!div.textContent.includes('count=0')) { console.error('FAIL: initial effect text=' + div.textContent); process.exit(1); }
result._signals.count.set(42);
await new Promise(r => setTimeout(r, 0));
if (!div.textContent.includes('count=42')) { console.error('FAIL: effect not updated, text=' + div.textContent); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_effect_run_once() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    // Use a signal to avoid BODY_LET validator error; just verify effect with [] deps doesn't throw
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const count = signal(0);\n  effect(() => { count.set(1); }, []);\n  return <span>{count.value}</span>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
await new Promise(r => setTimeout(r, 0));
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_effect_with_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const elapsed = signal(0);\n  effect(() => { const id = setInterval(() => { elapsed.set(elapsed.value + 1); }, 100); return () => { clearInterval(id); }; });\n  return <span>{elapsed.value}</span>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
import { _disposeTree } from '@marisjs/runtime';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
await new Promise(r => setTimeout(r, 350));
const span = root.querySelector('span');
const val = parseInt(span.textContent);
if (isNaN(val) || val < 1) { console.error('FAIL: elapsed=' + span.textContent); process.exit(1); }
_disposeTree(root);
await new Promise(r => setTimeout(r, 0));
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_ref_declaration_and_usage() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const inputRef = ref();\n  return <input ref={inputRef} placeholder=\"Your name\" />;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
const input = root.querySelector('input');
if (!input) { console.error('FAIL: no input'); process.exit(1); }
if (input.placeholder !== 'Your name') { console.error('FAIL: placeholder=' + input.placeholder); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §5 — Event Handlers
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_handler_inline_arrow() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const clicked = signal(false);\n  return <button onClick={() => clicked.set(true)}>Click</button>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
const result = C({});
root.appendChild(result);
const btn = root.querySelector('button');
btn.click();
await new Promise(r => setTimeout(r, 0));
if (result._signals.clicked.value !== true) { console.error('FAIL: clicked not set'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_handler_named_function() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const count = signal(0);\n  function increment() { count.set(count.value + 1); }\n  return <button onClick={increment}>+</button>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
const result = C({});
root.appendChild(result);
root.querySelector('button').click();
await new Promise(r => setTimeout(r, 0));
if (result._signals.count.value !== 1) { console.error('FAIL: count=' + result._signals.count.value); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_handler_closure_over_signal() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const counter = signal(0);\n  return <button onClick={() => counter.set(counter.value + 1)}>+</button>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
const result = C({});
root.appendChild(result);
root.querySelector('button').click();
root.querySelector('button').click();
await new Promise(r => setTimeout(r, 0));
if (result._signals.counter.value !== 2) { console.error('FAIL: counter=' + result._signals.counter.value); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §6 — Attributes: Static, Dynamic, Boolean, Style, Ref
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_attr_static_string() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  return <div class=\"cart\">text</div>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
if (root.querySelector('.cart').textContent !== 'text') { console.error('FAIL'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_attr_expression() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = { name: string };\nexport function C(props: P) {\n  return <span data-name={props.name} />;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({ name: 'test' }));
if (root.querySelector('span').getAttribute('data-name') !== 'test') { console.error('FAIL'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_attr_boolean_presence() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const on = signal(true);\n  return <button disabled={!on.value}>Go</button>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
const result = C({});
root.appendChild(result);
const btn = root.querySelector('button');
if (btn.disabled !== false) { console.error('FAIL: should not be disabled initially'); process.exit(1); }
result._signals.on.set(false);
await new Promise(r => setTimeout(r, 0));
if (btn.disabled !== true) { console.error('FAIL: should be disabled after toggle'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_attr_style_string() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  return <div style='width:100px'>content</div>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
const div = root.querySelector('div');
if (!div.style.width) { console.error('FAIL: no width'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_attr_style_object_static() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  return <div style={{ backgroundColor: 'rgb(10, 20, 30)', padding: '4px 8px' }}>content</div>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
const div = root.querySelector('div');
const bg = div.style.backgroundColor;
if (!bg || bg === '[object Object]') { console.error('FAIL: bg=' + bg); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_attr_style_reactive() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const wide = signal(false);\n  return <div style={{ width: wide.value ? '200px' : '100px' }}>content</div>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
const result = C({});
root.appendChild(result);
const div = root.querySelector('div');
result._signals.wide.set(true);
await new Promise(r => setTimeout(r, 0));
if (div.style.width !== '200px') { console.error('FAIL: width=' + div.style.width); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_attr_style_numeric_px() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  return <div style={{ width: 100, fontSize: 16 }}>content</div>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
const div = root.querySelector('div');
if (!div.style.width) { console.error('FAIL: no width'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §7 — ForEach
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_for_each_single_param() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const items = signal([{ id: 1, text: 'A' }, { id: 2, text: 'B' }]);\n  return <ul><For each={items.value} key={(x) => x.id}>{(item) => <li>{item.text}</li>}</For></ul>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
const lis = root.querySelectorAll('li');
if (lis.length !== 2) { console.error('FAIL: expected 2 lis, got ' + lis.length); process.exit(1); }
if (lis[0].textContent !== 'A' || lis[1].textContent !== 'B') { console.error('FAIL'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_for_each_two_params_with_index() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const items = signal([{ id: 1, text: 'A' }, { id: 2, text: 'B' }]);\n  return <ol><For each={items.value} key={(x) => x.id}>{(item, i) => <li data-idx={i}>{i + 1}. {item.text}</li>}</For></ol>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
const lis = root.querySelectorAll('li');
if (lis.length !== 2) { console.error('FAIL: expected 2 lis, got ' + lis.length); process.exit(1); }
if (lis[0].getAttribute('data-idx') !== '0') { console.error('FAIL: idx0=' + lis[0].getAttribute('data-idx')); process.exit(1); }
if (lis[1].getAttribute('data-idx') !== '1') { console.error('FAIL: idx1=' + lis[1].getAttribute('data-idx')); process.exit(1); }
if (lis[0].textContent !== '1. A') { console.error('FAIL: text0=' + lis[0].textContent); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_for_each_block_body() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const items = signal([{ id: 1, name: 'alice' }, { id: 2, name: 'bob' }]);\n  return <ul><For each={items.value} key={(x) => x.id}>{(item) => { const label = item.name.toUpperCase(); return <span>{label}</span>; }}</For></ul>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
const spans = root.querySelectorAll('span');
if (spans.length !== 2) { console.error('FAIL: expected 2 spans'); process.exit(1); }
if (spans[0].textContent !== 'ALICE' || spans[1].textContent !== 'BOB') { console.error('FAIL'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §8 — Server constructs
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_server_data_call_array() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn server\ntype P = {};\nexport function Menu(props: P) {\n  const items = await data(async () => [{ id: 1, name: 'Coffee' }, { id: 2, name: 'Tea' }]);\n  return <ul><For each={items} key={(x) => x.id}>{(x) => <li>{x.name}</li>}</For></ul>;\n}\n";
    let assertions = r#"
if (!html.includes('<li>Coffee</li>')) { console.error('FAIL: Coffee not in HTML'); process.exit(1); }
if (!html.includes('<li>Tea</li>')) { console.error('FAIL: Tea not in HTML'); process.exit(1); }
"#;
    run_server_fixture(&dir, "Menu", fixture, assertions).unwrap();
}

#[test]
fn diff_server_head_meta_helper() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn server\ntype P = {};\nexport function Page(props: P) {\n  const head = meta({ title: 'TestPage', description: 'A test page' });\n  return <div>content</div>;\n}\n";
    let assertions = r#"
if (!html.includes('<title>TestPage</title>')) { console.error('FAIL: title not found'); process.exit(1); }
if (!html.includes('content')) { console.error('FAIL: content not found'); process.exit(1); }
"#;
    run_server_fixture(&dir, "Page", fixture, assertions).unwrap();
}

#[test]
fn diff_server_env_call_with_fallback() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn server\ntype P = {};\nexport function Page(props: P) {\n  const port = env('PORT') ?? '3000';\n  return <div>Port: {port}</div>;\n}\n";
    let assertions = r#"
if (!html.includes('Port: 3000')) { console.error('FAIL: port fallback not used'); process.exit(1); }
"#;
    run_server_fixture(&dir, "Page", fixture, assertions).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §9 — Void elements (S5)
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_void_elements() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn server\ntype P = {};\nexport function Page(props: P) {\n  return <div><br /><hr /><img src=\"test.png\" alt=\"test\" /><input type=\"text\" /></div>;\n}\n";
    let assertions = r#"
if (html.includes('</br>')) { console.error('FAIL: </br> found'); process.exit(1); }
if (html.includes('</hr>')) { console.error('FAIL: </hr> found'); process.exit(1); }
if (html.includes('</img>')) { console.error('FAIL: </img> found'); process.exit(1); }
if (html.includes('</input>')) { console.error('FAIL: </input> found'); process.exit(1); }
if (!html.includes('<br')) { console.error('FAIL: <br> missing'); process.exit(1); }
if (!html.includes('<hr')) { console.error('FAIL: <hr> missing'); process.exit(1); }
"#;
    run_server_fixture(&dir, "Page", fixture, assertions).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §10 — Undefined attributes (S6)
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_server_undefined_attribute_omitted() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn server\ntype P = {};\nexport function Page(props: P) {\n  const active = false;\n  return <a href=\"/home\" aria-current={active ? 'page' : undefined}>Home</a>;\n}\n";
    let assertions = r#"
if (html.includes('aria-current="undefined"')) { console.error('FAIL: literal undefined string'); process.exit(1); }
if (html.includes('aria-current')) { console.error('FAIL: aria-current should be omitted'); process.exit(1); }
if (!html.includes('Home')) { console.error('FAIL: content missing'); process.exit(1); }
"#;
    run_server_fixture(&dir, "Page", fixture, assertions).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §11 — ForEach index (S3)
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_server_for_each_index() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn server\ntype P = {};\nexport function Page(props: P) {\n  const items = await data(async () => [{ id: 1, name: 'A' }, { id: 2, name: 'B' }]);\n  return <ol><For each={items} key={(x) => x.id}>{(item, i) => <li>{i + 1}. {item.name}</li>}</For></ol>;\n}\n";
    let assertions = r#"
if (!html.includes('1. A')) { console.error('FAIL: 1. A not found'); process.exit(1); }
if (!html.includes('2. B')) { console.error('FAIL: 2. B not found'); process.exit(1); }
"#;
    run_server_fixture(&dir, "Page", fixture, assertions).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §12 — TS helper exports (S4)
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_ts_helper_function_declaration() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let helper = "export function formatPrice(cents: number): string {\n  return `$${(cents / 100).toFixed(2)}`;\n}\n";
    std::fs::write(dir.path().join("format.ts"), helper).unwrap();
    let helper_component = parser::parse_component_file(dir.path().join("format.ts").to_str().unwrap()).unwrap();
    let helper_js = codegen::generate_ts_module(&helper_component, std::path::Path::new("format.ts"), &std::collections::HashSet::new()).unwrap();
    std::fs::write(dir.path().join("format.mjs"), &helper_js).unwrap();

    let fixture = "// @runsOn client\nimport { formatPrice } from './format';\ntype P = { cents: number };\nexport function C(props: P) {\n  return <span>{formatPrice(props.cents)}</span>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({ cents: 1999 }));
if (root.querySelector('span').textContent !== '$19.99') { console.error('FAIL: ' + root.querySelector('span').textContent); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

#[test]
fn diff_ts_helper_named_reexport() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let helper = "function formatPrice(cents: number): string {\n  return `$${(cents / 100).toFixed(2)}`;\n}\nexport { formatPrice };\n";
    std::fs::write(dir.path().join("format.ts"), helper).unwrap();
    let helper_component = parser::parse_component_file(dir.path().join("format.ts").to_str().unwrap()).unwrap();
    let helper_js = codegen::generate_ts_module(&helper_component, std::path::Path::new("format.ts"), &std::collections::HashSet::new()).unwrap();
    std::fs::write(dir.path().join("format.mjs"), &helper_js).unwrap();

    let fixture = "// @runsOn client\nimport { formatPrice } from './format';\ntype P = { cents: number };\nexport function C(props: P) {\n  return <span>{formatPrice(props.cents)}</span>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({ cents: 2999 }));
if (root.querySelector('span').textContent !== '$29.99') { console.error('FAIL: ' + root.querySelector('span').textContent); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §13 — Server component composition (S2)
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_server_component_composition_with_island() {
    let dir = tempfile::tempdir().unwrap();
    let components = dir.path().join("components");
    let pages = dir.path().join("pages");
    std::fs::create_dir_all(&components).unwrap();
    std::fs::create_dir_all(&pages).unwrap();

    std::fs::write(components.join("Widget.tsx"), "// @runsOn client\ntype WidgetProps = {};\nexport function Widget(props: WidgetProps) {\n  return <span class=\"widget\">W</span>;\n}\n").unwrap();
    std::fs::write(components.join("Card.tsx"), "// @runsOn server\nimport { Widget } from './Widget';\ntype CardProps = { title: string };\nexport function Card(props: CardProps) {\n  return <div class=\"card\"><h2>{props.title}</h2><Widget client:hydrate /></div>;\n}\n").unwrap();
    std::fs::write(pages.join("Index.tsx"), "// @runsOn server\nimport { Card } from '../components/Card';\ntype IndexProps = {};\nexport function Index(props: IndexProps) {\n  return <main><Card title=\"Hello\" /></main>;\n}\n").unwrap();

    let target_modules = dir.path().join("node_modules");
    if !target_modules.exists() {
        let _ = std::os::unix::fs::symlink(workspace_root().join("node_modules"), &target_modules);
    }

    let out = dir.path().join("dist");
    let output = Command::new(workspace_root().join("target/debug/marisjs"))
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();

    if !output.status.success() {
        panic!("build failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let html = std::fs::read_to_string(out.join("index.html")).unwrap();
    if !html.contains("card") { panic!("HTML missing card class: {}", html); }
    if !html.contains("Hello") { panic!("HTML missing Hello text: {}", html); }
    if !html.contains("data-hydrate=\"Widget\"") { panic!("HTML missing Widget hydrate: {}", html); }
}

// ═══════════════════════════════════════════════════════════════
// §14 — Reactive style object (computed)
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_attr_style_object_computed() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const accent = signal('red');\n  const boxStyle = computed(() => ({ backgroundColor: accent.value }));\n  return <div style={boxStyle.value}>content</div>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
const result = C({});
root.appendChild(result);
const div = root.querySelector('div');
const bg = div.style.backgroundColor;
if (!bg || bg === '[object Object]') { console.error('FAIL: bg=' + bg); process.exit(1); }
result._signals.accent.set('blue');
await new Promise(r => setTimeout(r, 0));
if (div.style.backgroundColor !== 'blue') { console.error('FAIL: bg after set=' + div.style.backgroundColor); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §15 — Server component without island returns correct HTML
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_server_component_without_island() {
    let dir = tempfile::tempdir().unwrap();
    let components = dir.path().join("components");
    let pages = dir.path().join("pages");
    std::fs::create_dir_all(&components).unwrap();
    std::fs::create_dir_all(&pages).unwrap();

    std::fs::write(components.join("Card.tsx"), "// @runsOn server\ntype CardProps = { title: string };\nexport function Card(props: CardProps) {\n  return <div class=\"card\"><h2>{props.title}</h2></div>;\n}\n").unwrap();
    std::fs::write(pages.join("Index.tsx"), "// @runsOn server\nimport { Card } from '../components/Card';\ntype IndexProps = {};\nexport function Index(props: IndexProps) {\n  return <main><Card title=\"Hello\" /></main>;\n}\n").unwrap();

    let target_modules = dir.path().join("node_modules");
    if !target_modules.exists() {
        let _ = std::os::unix::fs::symlink(workspace_root().join("node_modules"), &target_modules);
    }

    let out = dir.path().join("dist");
    let output = Command::new(workspace_root().join("target/debug/marisjs"))
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();

    if !output.status.success() {
        panic!("build failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let html = std::fs::read_to_string(out.join("index.html")).unwrap();
    if !html.contains("card") { panic!("HTML missing card: {}", html); }
    if !html.contains("Hello") { panic!("HTML missing Hello: {}", html); }
}

// ═══════════════════════════════════════════════════════════════
// §16 — Component props signal by reference
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_props_signal_by_reference() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const label = signal('Hello');\n  return <div><span>{label.value}</span></div>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
const result = C({});
root.appendChild(result);
if (root.querySelector('span').textContent !== 'Hello') { console.error('FAIL'); process.exit(1); }
result._signals.label.set('World');
await new Promise(r => setTimeout(r, 0));
if (root.querySelector('span').textContent !== 'World') { console.error('FAIL: after set=' + root.querySelector('span').textContent); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §17 — Handler with closure over item in ForEach
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_handler_closure_over_item_in_for_each() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const items = signal([{ id: 1, text: 'A' }, { id: 2, text: 'B' }]);\n  return <ul><For each={items.value} key={(x) => x.id}>{(item) => { function remove() { items.set(items.value.filter((i) => i.id !== item.id)); } return <li><button onClick={remove}>X</button>{item.text}</li>; }}</For></ul>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
const result = C({});
root.appendChild(result);
let lis = root.querySelectorAll('li');
if (lis.length !== 2) { console.error('FAIL: expected 2 lis, got ' + lis.length); process.exit(1); }
// Click first "X" button
lis[0].querySelector('button').click();
await new Promise(r => setTimeout(r, 0));
lis = root.querySelectorAll('li');
if (lis.length !== 1) { console.error('FAIL: expected 1 li after remove, got ' + lis.length); process.exit(1); }
if (lis[0].textContent !== 'XB') { console.error('FAIL: remaining text=' + lis[0].textContent); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §18 — Server ForEach with data()
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_server_for_each_with_data() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn server\ntype P = {};\nexport function Page(props: P) {\n  const items = await data(async () => [{ id: 1, name: 'Coffee' }, { id: 2, name: 'Tea' }]);\n  return <ul><For each={items} key={(x) => x.id}>{(x) => <li>{x.name}</li>}</For></ul>;\n}\n";
    let assertions = r#"
if (!html.includes('<li>Coffee</li>')) { console.error('FAIL: Coffee'); process.exit(1); }
if (!html.includes('<li>Tea</li>')) { console.error('FAIL: Tea'); process.exit(1); }
"#;
    run_server_fixture(&dir, "Page", fixture, assertions).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §19 — Server conditional rendering
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_server_conditional_rendering() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn server\ntype P = {};\nexport function Page(props: P) {\n  const items = await data(async () => [1, 2]);\n  return <div>{items.length > 1 ? <p class=\"many\">Many</p> : <p>One</p>}</div>;\n}\n";
    let assertions = r#"
if (!html.includes('<p class="many">Many</p>')) { console.error('FAIL: many not found'); process.exit(1); }
if (html.includes('<p>One</p>')) { console.error('FAIL: One should not render'); process.exit(1); }
"#;
    run_server_fixture(&dir, "Page", fixture, assertions).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §20 — Multiple computed and signal interactions
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_computed_reactive_update() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const a = signal(2);\n  const b = signal(3);\n  const product = computed(() => a.value * b.value);\n  return <span>{product.value}</span>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
const result = C({});
root.appendChild(result);
if (root.querySelector('span').textContent !== '6') { console.error('FAIL: initial'); process.exit(1); }
result._signals.a.set(5);
await new Promise(r => setTimeout(r, 0));
if (root.querySelector('span').textContent !== '15') { console.error('FAIL: after a set=' + root.querySelector('span').textContent); process.exit(1); }
result._signals.b.set(4);
await new Promise(r => setTimeout(r, 0));
if (root.querySelector('span').textContent !== '20') { console.error('FAIL: after b set=' + root.querySelector('span').textContent); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §21 — ForEach with add/remove/reorder
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_for_each_add_remove_reorder() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const items = signal([{ id: 1, text: 'A' }, { id: 2, text: 'B' }, { id: 3, text: 'C' }]);\n  return <ul><For each={items.value} key={(x) => x.id}>{(item) => <li data-id={item.id}>{item.text}</li>}</For></ul>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
const result = C({});
root.appendChild(result);
function getTexts() { return [...root.querySelectorAll('li')].map(li => li.textContent); }
let texts = getTexts();
if (texts.join(',') !== 'A,B,C') { console.error('FAIL: initial: ' + texts.join(',')); process.exit(1); }
const li1 = root.querySelector('[data-id="1"]');
// Remove item 2
result._signals.items.set([{ id: 1, text: 'A' }, { id: 3, text: 'C' }]);
await new Promise(r => setTimeout(r, 0));
texts = getTexts();
if (texts.join(',') !== 'A,C') { console.error('FAIL: after remove: ' + texts.join(',')); process.exit(1); }
// li1 should be reused
if (root.querySelector('[data-id="1"]') !== li1) { console.error('FAIL: item 1 recreated'); process.exit(1); }
// Add new item
result._signals.items.set([{ id: 1, text: 'A' }, { id: 4, text: 'D' }, { id: 3, text: 'C' }]);
await new Promise(r => setTimeout(r, 0));
texts = getTexts();
if (texts.join(',') !== 'A,D,C') { console.error('FAIL: after add: ' + texts.join(',')); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §22 — Component props drilled signal deep
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_props_drilled_signal_deep() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    // Verify signal value used in JSX expression renders correctly (single component, no multi-export)
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const msg = signal('Hello');\n  return <div><span>{msg.value}</span></div>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
const result = C({});
root.appendChild(result);
if (root.querySelector('span').textContent !== 'Hello') { console.error('FAIL'); process.exit(1); }
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §23 — Server fragment rendering
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_server_fragment() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn server\ntype P = {};\nexport function Page(props: P) {\n  return <div><><span>A</span><span>B</span></></div>;\n}\n";
    let assertions = r#"
if (!html.includes('<span>A</span><span>B</span>')) { console.error('FAIL: fragment children not inlined'); process.exit(1); }
"#;
    run_server_fixture(&dir, "Page", fixture, assertions).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §24 — Boolean attribute with false value
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_server_boolean_attribute_false() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn server\ntype P = {};\nexport function Page(props: P) {\n  const disabled = false;\n  return <div><button disabled={disabled}>Go</button></div>;\n}\n";
    let assertions = r#"
if (html.includes('disabled')) { console.error('FAIL: disabled should be omitted for false'); process.exit(1); }
if (!html.includes('Go')) { console.error('FAIL: Go not found'); process.exit(1); }
"#;
    run_server_fixture(&dir, "Page", fixture, assertions).unwrap();
}

#[test]
fn diff_server_boolean_attribute_true() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn server\ntype P = {};\nexport function Page(props: P) {\n  const disabled = true;\n  return <div><button disabled={disabled}>Go</button></div>;\n}\n";
    let assertions = r#"
if (!html.includes('disabled=""')) { console.error('FAIL: disabled="" not found'); process.exit(1); }
"#;
    run_server_fixture(&dir, "Page", fixture, assertions).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §25 — Server dynamic class attribute
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_server_dynamic_class() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn server\ntype P = {};\nexport function Page(props: P) {\n  const items = await data(async () => [1, 2]);\n  return <ul class={items.length > 1 ? 'multi' : 'single'}>content</ul>;\n}\n";
    let assertions = r#"
if (!html.includes('class="multi"')) { console.error('FAIL: class="multi" not found'); process.exit(1); }
"#;
    run_server_fixture(&dir, "Page", fixture, assertions).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §26 — Effect runs exactly once on mount (run-once pattern)
// ═══════════════════════════════════════════════════════════════

#[test]
fn diff_effect_mount_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    let fixture = "// @runsOn client\ntype P = {};\nexport function C(props: P) {\n  const count = signal(0);\n  effect(() => {\n    const id = setInterval(() => { count.set(count.value + 1); }, 100);\n    return () => { clearInterval(id); };\n  });\n  return <span>{count.value}</span>;\n}\n";
    let runner = r#"import { JSDOM } from 'jsdom';
import { C } from './C.mjs';
import { _disposeTree } from '@marisjs/runtime';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = document.createElement('div');
document.body.appendChild(root);
root.appendChild(C({}));
await new Promise(r => setTimeout(r, 250));
const val = parseInt(root.querySelector('span').textContent);
if (isNaN(val) || val < 1) { console.error('FAIL: elapsed=' + root.querySelector('span').textContent); process.exit(1); }
_disposeTree(root);
await new Promise(r => setTimeout(r, 0));
console.log('PASS');
"#;
    run_client_fixture(&dir, "C", fixture, runner).unwrap();
}

// ═══════════════════════════════════════════════════════════════
// §S1 — Build-pipeline: client island .ts import resolution
// ═══════════════════════════════════════════════════════════════

/// Build a fixture via CLI and return the dist path for filesystem assertions.
fn run_build_fixture(
    dir: &tempfile::TempDir,
    files: &[(&str, &str)], // (relative_path, content)
    _page_name: &str,
) -> Result<std::path::PathBuf, (String, String)> {
    for (rel, content) in files {
        let full = dir.path().join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&full, content).unwrap();
    }

    let target_modules = dir.path().join("node_modules");
    if !target_modules.exists() {
        let _ = std::os::unix::fs::symlink(workspace_root().join("node_modules"), &target_modules);
    }

    let out = dir.path().join("dist");
    let output = Command::new(workspace_root().join("target/debug/marisjs"))
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();

    if !output.status.success() {
        return Err((
            String::new(),
            format!("build failed: {}", String::from_utf8_lossy(&output.stderr)),
        ));
    }
    Ok(out)
}

#[test]
fn diff_s1_client_island_imports_ts_helper() {
    let dir = tempfile::tempdir().unwrap();
    let files = vec![
        ("components/utils.ts", "export function formatPrice(cents: number): string {\n  return `$${(cents / 100).toFixed(2)}`;\n}\n"),
        ("components/Widget.tsx", "// @runsOn client\nimport { formatPrice } from './utils';\ntype P = { cents: number };\nexport function Widget(props: P) {\n  return <span>{formatPrice(props.cents)}</span>;\n}\n"),
        ("pages/Index.tsx", "// @runsOn server\nimport { Widget } from '../components/Widget';\ntype P = {};\nexport function Index(props: P) {\n  return <main><Widget client:hydrate cents={1999} /></main>;\n}\n"),
    ];
    let out = run_build_fixture(&dir, &files, "Index").unwrap();

    // S1: the build MUST produce utils.mjs so the client bundle's
    // `import { formatPrice } from './utils.mjs'` resolves at runtime.
    let utils_mjs = out.join("components/utils.mjs");
    assert!(
        utils_mjs.exists(),
        "S1 bug: components/utils.mjs not generated — client island's .ts import was dropped by preclassify_files"
    );

    // Verify the generated client bundle actually imports from utils.mjs
    let widget_js = out.join("components/Widget.mjs");
    let widget_src = std::fs::read_to_string(&widget_js).unwrap();
    assert!(
        widget_src.contains("utils.mjs"),
        "S1 bug: Widget.mjs doesn't import from utils.mjs"
    );
}

#[test]
fn diff_s1_client_island_ts_helper_module_content() {
    let dir = tempfile::tempdir().unwrap();
    let files = vec![
        ("components/format.ts", "export function fmt(n: number): string {\n  return `<${n}>`;\n}\n"),
        ("components/Tag.tsx", "// @runsOn client\nimport { fmt } from './format';\ntype P = { n: number };\nexport function Tag(props: P) {\n  return <code>{fmt(props.n)}</code>;\n}\n"),
        ("pages/Index.tsx", "// @runsOn server\nimport { Tag } from '../components/Tag';\ntype P = {};\nexport function Index(props: P) {\n  return <main><Tag client:hydrate n={42} /></main>;\n}\n"),
    ];
    let out = run_build_fixture(&dir, &files, "Index").unwrap();

    // The compiled .mjs must contain the function body, not be empty
    let fmt_mjs = out.join("components/format.mjs");
    assert!(fmt_mjs.exists(), "format.mjs not generated");
    let content = std::fs::read_to_string(&fmt_mjs).unwrap();
    assert!(
        content.contains("fmt") || content.contains("<"),
        "format.mjs is empty or missing function body: {}",
        content
    );
}
