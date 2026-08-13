use std::process::Command;

fn workspace_root() -> &'static std::path::Path {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    // crates/codegen → workspace root
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

fn cli_binary() -> std::path::PathBuf {
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let bin = if cfg!(target_os = "windows") { "marisjs.exe" } else { "marisjs" };
    workspace_root().join("target").join(profile).join(bin)
}

fn setup_test_dir(dir: &tempfile::TempDir) {
    let root = workspace_root();
    let ws_nm = root.join("node_modules");

    // Ensure @marisjs/runtime exists in workspace node_modules (shared, one-time setup).
    // create_dir and symlink tolerate "already exists" so concurrent tests are safe.
    let maris_dir = ws_nm.join("@marisjs");
    let _ = std::fs::create_dir(&maris_dir);
    let runtime_link = maris_dir.join("runtime");
    if !runtime_link.exists() {
        let _ = std::os::unix::fs::symlink(&root.join("packages/runtime"), &runtime_link);
    }

    // Make the temp dir see the same node_modules via symlink.
    let target_modules = dir.path().join("node_modules");
    std::os::unix::fs::symlink(&ws_nm, &target_modules).unwrap();
}

fn parse_validate_generate(
    dir: &tempfile::TempDir,
    name: &str,
    fixture: &str,
) -> String {
    let fixture_path = dir.path().join(format!("{}.tsx", name));
    std::fs::write(&fixture_path, fixture).unwrap();

    let component = parser::parse_component_file(fixture_path.to_str().unwrap()).unwrap();
    let diags = validator::validate(&component);
    assert!(
        diags.is_empty(),
        "Validation errors for {}: {:?}",
        name,
        diags
    );

    let js = codegen::generate(&component).unwrap();
    eprintln!("--- Generated JS for {} ---\n{}", name, js);

    let js_path = dir.path().join(format!("{}.mjs", name));
    std::fs::write(&js_path, &js).unwrap();

    js
}

fn run_node(dir: &tempfile::TempDir, runner_src: &str) {
    let runner_path = dir.path().join("runner.mjs");
    std::fs::write(&runner_path, runner_src).unwrap();

    let output = Command::new("node")
        .arg(&runner_path)
        .current_dir(dir.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "node failed: stdout={} stderr={}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("PASS"),
        "Expected PASS, got stdout={} stderr={}",
        stdout,
        stderr
    );
}

/// Builds a tiny TSX component, generates JS, executes it in Node+jsdom,
/// and asserts the resulting DOM structure.
#[test]
fn static_jsx_with_props() {
    let fixture = concat!(
        "// @runsOn client\n",
        "type CartProps = { title: string; buttonLabel: string; };\n",
        "export function Cart(props: CartProps) {\n",
        "  return (\n",
        "    <div class=\"cart\">\n",
        "      <h1>{props.title}</h1>\n",
        "      <button>{props.buttonLabel}</button>\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();

    // Write fixture with correct filename (must match component name)
    let fixture_path = dir.path().join("Cart.tsx");
    std::fs::write(&fixture_path, fixture).unwrap();

    // Parse
    let component = parser::parse_component_file(fixture_path.to_str().unwrap()).unwrap();

    // Validate
    let diags = validator::validate(&component);
    assert!(
        diags.is_empty(),
        "Validation errors: {:?}",
        diags
    );

    // Generate JS
    let js = codegen::generate(&component).unwrap();
    eprintln!("--- Generated JS ---\n{}", js);

    // Write generated module
    let js_path = dir.path().join("Cart.mjs");
    std::fs::write(&js_path, &js).unwrap();

    // Write test runner that imports the component and uses jsdom
    let runner_src = format!(
        r#"import {{ JSDOM }} from 'jsdom';
import {{ Cart }} from './Cart.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = Cart({{ title: 'My Cart', buttonLabel: 'Checkout' }});
root.appendChild(result);

const h1 = root.querySelector('h1');
const btn = root.querySelector('button');

const ok = h1 !== null
    && h1.textContent === 'My Cart'
    && btn !== null
    && btn.textContent === 'Checkout';

if (ok) {{
    console.log('PASS');
}} else {{
    console.error('FAIL', JSON.stringify({{
        h1Text: h1 ? h1.textContent : null,
        btnText: btn ? btn.textContent : null,
    }}));
    process.exit(1);
}}
"#
    );
    let runner_path = dir.path().join("runner.mjs");
    std::fs::write(&runner_path, runner_src).unwrap();

    // Symlink node_modules so jsdom resolves from the temp dir
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let source_modules = workspace_root.join("node_modules");
    let target_modules = dir.path().join("node_modules");
    std::os::unix::fs::symlink(&source_modules, &target_modules).unwrap();

    // Run node
    let output = Command::new("node")
        .arg(&runner_path)
        .current_dir(dir.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "node exited with {}: stdout={} stderr={}",
        output.status,
        stdout,
        stderr
    );
    assert!(stdout.contains("PASS"), "Expected PASS, got stdout={}", stdout);
}

/// Component that returns a single element with no children (self-closing).
#[test]
fn self_closing_element() {
    let fixture = concat!(
        "// @runsOn client\n",
        "type IconProps = { name: string; };\n",
        "export function Icon(props: IconProps) {\n",
        "  return <span class=\"icon\" data-name={props.name} />;\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    let fixture_path = dir.path().join("Icon.tsx");
    std::fs::write(&fixture_path, fixture).unwrap();

    let component = parser::parse_component_file(fixture_path.to_str().unwrap()).unwrap();
    let diags = validator::validate(&component);
    assert!(diags.is_empty());

    let js = codegen::generate(&component).unwrap();
    eprintln!("--- Generated JS ---\n{}", js);

    let js_path = dir.path().join("Icon.mjs");
    std::fs::write(&js_path, &js).unwrap();

    let runner_src = r#"import { JSDOM } from 'jsdom';
import { Icon } from './Icon.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = Icon({ name: 'star' });
root.appendChild(result);

const span = root.querySelector('span');
const ok = span !== null
    && span.getAttribute('class') === 'icon'
    && span.getAttribute('data-name') === 'star'
    && span.textContent === '';

if (ok) {
    console.log('PASS');
} else {
    console.error('FAIL', { tag: span?.tagName, class: span?.getAttribute('class'), dataName: span?.getAttribute('data-name') });
    process.exit(1);
}
"#;

    let runner_path = dir.path().join("runner.mjs");
    std::fs::write(&runner_path, runner_src).unwrap();

    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let source_modules = workspace_root.join("node_modules");
    let target_modules = dir.path().join("node_modules");
    std::os::unix::fs::symlink(&source_modules, &target_modules).unwrap();

    let output = Command::new("node")
        .arg(&runner_path)
        .current_dir(dir.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "node failed: stderr={}", stderr);
    assert!(stdout.contains("PASS"), "Expected PASS, got: {}", stdout);
}

/// Component with deeply nested elements and static text.
#[test]
fn nested_elements_with_text() {
    let fixture = concat!(
        "// @runsOn client\n",
        "type LayoutProps = {};\n",
        "export function Layout(props: LayoutProps) {\n",
        "  return (\n",
        "    <nav class=\"top-nav\">\n",
        "      <div class=\"logo\">MyApp</div>\n",
        "      <ul>\n",
        "        <li>Home</li>\n",
        "        <li>About</li>\n",
        "      </ul>\n",
        "    </nav>\n",
        "  );\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    let fixture_path = dir.path().join("Layout.tsx");
    std::fs::write(&fixture_path, fixture).unwrap();

    let component = parser::parse_component_file(fixture_path.to_str().unwrap()).unwrap();
    let diags = validator::validate(&component);
    assert!(diags.is_empty(), "Validation errors: {:?}", diags);

    let js = codegen::generate(&component).unwrap();
    eprintln!("--- Generated JS ---\n{}", js);

    let js_path = dir.path().join("Layout.mjs");
    std::fs::write(&js_path, &js).unwrap();

    let runner_src = r#"import { JSDOM } from 'jsdom';
import { Layout } from './Layout.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
root.appendChild(Layout({}));

const nav = root.querySelector('nav');
const logo = root.querySelector('.logo');
const items = root.querySelectorAll('li');

const ok = nav !== null
    && nav.getAttribute('class') === 'top-nav'
    && logo !== null
    && logo.textContent === 'MyApp'
    && items.length === 2
    && items[0].textContent === 'Home'
    && items[1].textContent === 'About';

if (ok) {
    console.log('PASS');
} else {
    console.error('FAIL');
    process.exit(1);
}
"#;

    let runner_path = dir.path().join("runner.mjs");
    std::fs::write(&runner_path, runner_src).unwrap();

    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let source_modules = workspace_root.join("node_modules");
    let target_modules = dir.path().join("node_modules");
    std::os::unix::fs::symlink(&source_modules, &target_modules).unwrap();

    let output = Command::new("node")
        .arg(&runner_path)
        .current_dir(dir.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "node failed: stderr={}", stderr);
    assert!(stdout.contains("PASS"), "Expected PASS, got: {}", stdout);
}

// ═══════════════════════════════════════════════════════════════
// Signal reactivity tests — the core thing this framework delivers
// ═══════════════════════════════════════════════════════════════

/// A signal that updates a bind-wrapped text node when .set() is called.
#[test]
fn signal_drives_dom_text_update() {
    let fixture = concat!(
        "// @runsOn client\n",
        "type CounterProps = {};\n",
        "export function Counter(props: CounterProps) {\n",
        "  const count = signal(0);\n",
        "  return <span>{count.value}</span>;\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    parse_validate_generate(&dir, "Counter", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { Counter } from './Counter.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = Counter({});
root.appendChild(result);

const span = root.querySelector('span');
if (!span) { console.error('FAIL: no span'); process.exit(1); }
if (span.textContent !== '0') {
    console.error('FAIL initial: ' + span.textContent);
    process.exit(1);
}

result._signals.count.set(5);
await new Promise(r => setTimeout(r, 0));

if (span.textContent === '5') {
    console.log('PASS');
} else {
    console.error('FAIL after set: ' + span.textContent);
    process.exit(1);
}
"#;

    run_node(&dir, runner);
}

/// Computed that recalculates when its signal dependency changes,
/// and the DOM updates through the reactive bind.
#[test]
fn computed_chain_updates_dom() {
    let fixture = concat!(
        "// @runsOn client\n",
        "type TotalProps = {};\n",
        "export function Total(props: TotalProps) {\n",
        "  const price = signal(10);\n",
        "  const quantity = signal(2);\n",
        "  const total = computed(() => price.value * quantity.value);\n",
        "  return <div>Total: {total.value}</div>;\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    parse_validate_generate(&dir, "Total", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { Total } from './Total.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = Total({});
root.appendChild(result);

const div = root.querySelector('div');
if (!div) { console.error('FAIL: no div'); process.exit(1); }

// initial: 10 * 2 = 20
if (div.textContent !== 'Total: 20') {
    console.error('FAIL initial: ' + JSON.stringify(div.textContent));
    process.exit(1);
}

result._signals.price.set(5);
await new Promise(r => setTimeout(r, 0));
if (div.textContent !== 'Total: 10') {
    console.error('FAIL after price=5: ' + JSON.stringify(div.textContent));
    process.exit(1);
}

result._signals.quantity.set(3);
await new Promise(r => setTimeout(r, 0));
if (div.textContent !== 'Total: 15') {
    console.error('FAIL after quantity=3: ' + JSON.stringify(div.textContent));
    process.exit(1);
}

result._signals.price.set(5);
await new Promise(r => setTimeout(r, 0));
if (div.textContent !== 'Total: 15') {
    console.error('FAIL after same-value set: ' + JSON.stringify(div.textContent));
    process.exit(1);
}

console.log('PASS');
"#;

    run_node(&dir, runner);
}

/// Multiple independent signals update independent DOM locations.
#[test]
fn multiple_signals_independent_bindings() {
    let fixture = concat!(
        "// @runsOn client\n",
        "type ProfileProps = {};\n",
        "export function Profile(props: ProfileProps) {\n",
        "  const name = signal('Alice');\n",
        "  const age = signal(30);\n",
        "  return (\n",
        "    <div>\n",
        "      <span class=\"name\">{name.value}</span>\n",
        "      <span class=\"age\">{age.value}</span>\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    parse_validate_generate(&dir, "Profile", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { Profile } from './Profile.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = Profile({});
root.appendChild(result);

const nameEl = root.querySelector('.name');
const ageEl = root.querySelector('.age');
if (!nameEl || !ageEl) { console.error('FAIL: missing elements'); process.exit(1); }
if (nameEl.textContent !== 'Alice' || ageEl.textContent !== '30') {
    console.error('FAIL initial');
    process.exit(1);
}

result._signals.name.set('Bob');
await new Promise(r => setTimeout(r, 0));
if (nameEl.textContent !== 'Bob') {
    console.error('FAIL name update: ' + nameEl.textContent);
    process.exit(1);
}
if (ageEl.textContent !== '30') {
    console.error('FAIL age should not have changed: ' + ageEl.textContent);
    process.exit(1);
}

result._signals.age.set(25);
await new Promise(r => setTimeout(r, 0));
if (ageEl.textContent !== '25') {
    console.error('FAIL age update: ' + ageEl.textContent);
    process.exit(1);
}
if (nameEl.textContent !== 'Bob') {
    console.error('FAIL name should still be Bob: ' + nameEl.textContent);
    process.exit(1);
}

console.log('PASS');
"#;

    run_node(&dir, runner);
}

// ═══════════════════════════════════════════════════════════════
// Control-flow tests — ternary conditionals and <For>
// ═══════════════════════════════════════════════════════════════

/// Ternary conditional: DOM subtrees swap when a signal-driven condition changes.
#[test]
fn ternary_conditional_swaps_dom() {
    let fixture = concat!(
        "// @runsOn client\n",
        "type ToggleProps = {};\n",
        "export function Toggle(props: ToggleProps) {\n",
        "  const show = signal(true);\n",
        "  return (\n",
        "    <div>\n",
        "      {show.value ? <span class=\"on\">ON</span> : <span class=\"off\">OFF</span>}\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    parse_validate_generate(&dir, "Toggle", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { Toggle } from './Toggle.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = Toggle({});
root.appendChild(result);

// Initial: show=true → ON
const div = root.querySelector('div');
const onSpan = root.querySelector('.on');
const offSpan = root.querySelector('.off');
if (!onSpan) { console.error('FAIL: no on span'); process.exit(1); }
if (offSpan) { console.error('FAIL: off span should not exist'); process.exit(1); }
if (onSpan.textContent !== 'ON') { console.error('FAIL initial'); process.exit(1); }

// Toggle to false
result._signals.show.set(false);
await new Promise(r => setTimeout(r, 0));

const onSpan2 = root.querySelector('.on');
const offSpan2 = root.querySelector('.off');
if (onSpan2) { console.error('FAIL: on span should be gone'); process.exit(1); }
if (!offSpan2) { console.error('FAIL: no off span'); process.exit(1); }
if (offSpan2.textContent !== 'OFF') { console.error('FAIL after toggle'); process.exit(1); }

// Toggle back
result._signals.show.set(true);
await new Promise(r => setTimeout(r, 0));

const onSpan3 = root.querySelector('.on');
const offSpan3 = root.querySelector('.off');
if (!onSpan3) { console.error('FAIL: on span should be back'); process.exit(1); }
if (offSpan3) { console.error('FAIL: off span should be gone again'); process.exit(1); }

// Verify it is the same DOM node (not recreated)
if (onSpan3 !== onSpan) {
    console.error('FAIL: on span was recreated instead of reused');
    process.exit(1);
}

console.log('PASS');
"#;

    run_node(&dir, runner);
}

/// <For> with keyed reconciliation: add, remove, reorder items.
#[test]
fn for_each_add_remove_reorder() {
    let fixture = concat!(
        "// @runsOn client\n",
        "type ListProps = {};\n",
        "export function TaskList(props: ListProps) {\n",
        "  const tasks = signal([\n",
        "    { id: 1, text: 'Buy milk' },\n",
        "    { id: 2, text: 'Walk dog' },\n",
        "    { id: 3, text: 'Read book' },\n",
        "  ]);\n",
        "  return (\n",
        "    <ul>\n",
        "      <For each={tasks.value} key={(x) => x.id}>\n",
        "        {(item) => <li data-id={item.id}>{item.text}</li>}\n",
        "      </For>\n",
        "    </ul>\n",
        "  );\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    parse_validate_generate(&dir, "TaskList", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { TaskList } from './TaskList.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = TaskList({});
root.appendChild(result);

const ul = root.querySelector('ul');

function getLiTexts() {
    return [...ul.querySelectorAll('li')].map(li => li.textContent);
}

// --- initial state ---
let texts = getLiTexts();
if (texts.join(',') !== 'Buy milk,Walk dog,Read book') {
    console.error('FAIL initial: ' + texts.join(','));
    process.exit(1);
}

const li1 = ul.querySelector('[data-id="1"]');
const li2 = ul.querySelector('[data-id="2"]');
const li3 = ul.querySelector('[data-id="3"]');
if (!li1 || !li2 || !li3) {
    console.error('FAIL: missing initial li elements');
    process.exit(1);
}

// --- remove item 2 ---
result._signals.tasks.set([
    { id: 1, text: 'Buy milk' },
    { id: 3, text: 'Read book' },
]);
await new Promise(r => setTimeout(r, 0));

texts = getLiTexts();
if (texts.join(',') !== 'Buy milk,Read book') {
    console.error('FAIL after remove: ' + texts.join(','));
    process.exit(1);
}

// Reused nodes should be the SAME DOM references
const li1b = ul.querySelector('[data-id="1"]');
const li3b = ul.querySelector('[data-id="3"]');
if (li1b !== li1) {
    console.error('FAIL: item 1 was recreated after unrelated removal');
    process.exit(1);
}
if (li3b !== li3) {
    console.error('FAIL: item 3 was recreated after unrelated removal');
    process.exit(1);
}

// --- add a new item ---
result._signals.tasks.set([
    { id: 1, text: 'Buy milk' },
    { id: 4, text: 'Write code' },
    { id: 3, text: 'Read book' },
]);
await new Promise(r => setTimeout(r, 0));

texts = getLiTexts();
if (texts.join(',') !== 'Buy milk,Write code,Read book') {
    console.error('FAIL after add: ' + texts.join(','));
    process.exit(1);
}

const li1c = ul.querySelector('[data-id="1"]');
const li3c = ul.querySelector('[data-id="3"]');
if (li1c !== li1) {
    console.error('FAIL: item 1 recreated after add');
    process.exit(1);
}
if (li3c !== li3) {
    console.error('FAIL: item 3 recreated after add');
    process.exit(1);
}
const li4 = ul.querySelector('[data-id="4"]');
if (!li4) { console.error('FAIL: missing new item 4'); process.exit(1); }
if (li4.textContent !== 'Write code') { console.error('FAIL: wrong text for item 4'); process.exit(1); }

// --- reorder ---
result._signals.tasks.set([
    { id: 3, text: 'Read book' },
    { id: 4, text: 'Write code' },
    { id: 1, text: 'Buy milk' },
]);
await new Promise(r => setTimeout(r, 0));

texts = getLiTexts();
if (texts.join(',') !== 'Read book,Write code,Buy milk') {
    console.error('FAIL after reorder: ' + texts.join(','));
    process.exit(1);
}

const li1d = ul.querySelector('[data-id="1"]');
const li3d = ul.querySelector('[data-id="3"]');
const li4d = ul.querySelector('[data-id="4"]');
if (li1d !== li1c) {
    console.error('FAIL: item 1 recreated on reorder');
    process.exit(1);
}
if (li3d !== li3c) {
    console.error('FAIL: item 3 recreated on reorder');
    process.exit(1);
}
if (li4d !== li4) {
    console.error('FAIL: item 4 recreated on reorder');
    process.exit(1);
}

console.log('PASS');
"#;

    run_node(&dir, runner);
}

// ═══════════════════════════════════════════════════════════════
// Event handler and server/client split tests
// ═══════════════════════════════════════════════════════════════

/// onClick handler fires when the button is clicked, and the spy callback runs.
#[test]
fn click_handler_fires_and_spy_runs() {
    let fixture = concat!(
        "// @runsOn client\n",
        "type ClickerProps = {};\n",
        "export function Clicker(props: ClickerProps) {\n",
        "  const clicked = signal(false);\n",
        "  return <button onClick={() => clicked.set(true)}>{clicked.value ? 'CLICKED' : 'NOT'}</button>;\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    parse_validate_generate(&dir, "Clicker", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { Clicker } from './Clicker.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = Clicker({});
root.appendChild(result);

const btn = root.querySelector('button');
if (!btn) { console.error('FAIL: no button'); process.exit(1); }
if (btn.textContent !== 'NOT') {
    console.error('FAIL initial: ' + btn.textContent);
    process.exit(1);
}

btn.click();
await new Promise(r => setTimeout(r, 0));

if (btn.textContent === 'CLICKED') {
    console.log('PASS');
} else {
    console.error('FAIL after click: ' + btn.textContent);
    process.exit(1);
}
"#;

    run_node(&dir, runner);
}

/// Server component produces static HTML string with no client bundle.
#[test]
fn server_component_produces_html() {
    let fixture = concat!(
        "// @runsOn server\n",
        "type HeaderProps = { title: string; };\n",
        "export function Header(props: HeaderProps) {\n",
        "  return <h1 class=\"title\">{props.title}</h1>;\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    parse_validate_generate(&dir, "Header", fixture);

    let runner = r#"import { Header } from './Header.mjs';

const result = Header({ title: 'My App' });
if (typeof result === 'string') {
    console.log('PASS');
} else {
    console.error('FAIL: expected string, got', typeof result, result);
    process.exit(1);
}
"#;

    run_node(&dir, runner);
}

/// Server component with client:hydrate child produces html + clientBundles.
#[test]
fn server_component_with_hydrate_produces_split_output() {
    let fixture = concat!(
        "// @runsOn server\n",
        "type PageProps = {};\n",
        "export function Page(props: PageProps) {\n",
        "  return (\n",
        "    <div>\n",
        "      <h1>Welcome</h1>\n",
        "      <Widget client:hydrate />\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    parse_validate_generate(&dir, "Page", fixture);

    let runner = r#"import { Page } from './Page.mjs';

const result = Page({});
if (typeof result === 'object' && result !== null && 'html' in result && 'clientBundles' in result) {
    if (Array.isArray(result.clientBundles) && result.clientBundles.includes('./Widget.js')) {
        console.log('PASS');
    } else {
        console.error('FAIL: clientBundles', result.clientBundles);
        process.exit(1);
    }
} else {
    console.error('FAIL: expected { html, clientBundles }, got', result);
    process.exit(1);
}
"#;

    run_node(&dir, runner);
}

// ═══════════════════════════════════════════════════════════════
// Component composition and mount() integration
// ═══════════════════════════════════════════════════════════════

/// Parent component renders a child component, passing props.
#[test]
fn parent_renders_child_component() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let child_fixture = concat!(
        "// @runsOn client\n",
        "type ItemListProps = { items: string[]; };\n",
        "export function ItemList(props: ItemListProps) {\n",
        "  return (\n",
        "    <ul>\n",
        "      <For each={props.items} key={(x) => x}>\n",
        "        {(item) => <li class=\"li-item\">{item}</li>}\n",
        "      </For>\n",
        "    </ul>\n",
        "  );\n",
        "}\n",
    );
    parse_validate_generate(&dir, "ItemList", child_fixture);

    let parent_fixture = concat!(
        "// @runsOn client\n",
        "import { ItemList } from './ItemList';\n",
        "type CartProps = { heading: string; items: string[]; };\n",
        "export function Cart(props: CartProps) {\n",
        "  return (\n",
        "    <div class=\"cart\">\n",
        "      <h1>{props.heading}</h1>\n",
        "      <ItemList items={props.items} />\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    parse_validate_generate(&dir, "Cart", parent_fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { Cart } from './Cart.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = Cart({ heading: 'My Cart', items: ['Apple', 'Banana'] });
root.appendChild(result);

const h1 = root.querySelector('h1');
const ul = root.querySelector('ul');
const lis = root.querySelectorAll('.li-item');

let ok = h1 !== null && h1.textContent === 'My Cart'
    && ul !== null
    && lis.length === 2
    && lis[0].textContent === 'Apple'
    && lis[1].textContent === 'Banana';

if (ok) {
    console.log('PASS');
} else {
    console.error('FAIL', {
        h1: h1 ? h1.textContent : null,
        liCount: lis.length,
        li0: lis[0] ? lis[0].textContent : null,
    });
    process.exit(1);
}
"#;

    run_node(&dir, runner);
}

/// Two generated components wired through the runtime mount() primitive.
#[test]
fn mount_integration_with_two_components() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let child_fixture = concat!(
        "// @runsOn client\n",
        "type GreetingProps = { name: string; };\n",
        "export function Greeting(props: GreetingProps) {\n",
        "  return <span class=\"greeting\">Hello, {props.name}</span>;\n",
        "}\n",
    );
    parse_validate_generate(&dir, "Greeting", child_fixture);

    let parent_fixture = concat!(
        "// @runsOn client\n",
        "import { Greeting } from './Greeting';\n",
        "type AppProps = {};\n",
        "export function App(props: AppProps) {\n",
        "  return (\n",
        "    <div class=\"app\">\n",
        "      <Greeting name={'World'} />\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    parse_validate_generate(&dir, "App", parent_fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { mount } from '@marisjs/runtime';
import { App } from './App.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const rootContainer = dom.window.document.createElement('div');
mount(rootContainer, () => App({}));

const appDiv = rootContainer.querySelector('.app');
const greetSpan = rootContainer.querySelector('.greeting');

const ok = appDiv !== null
    && greetSpan !== null
    && greetSpan.textContent === 'Hello, World';

if (ok) {
    console.log('PASS');
} else {
    console.error('FAIL', {
        appDiv: !!appDiv,
        greetSpan: !!greetSpan,
        text: greetSpan ? greetSpan.textContent : null,
    });
    process.exit(1);
}
"#;

    run_node(&dir, runner);
}

/// Parent signal passed as prop to child — mutating the parent signal
/// must cause the child's DOM to update (cross-component reactivity).
#[test]
fn parent_signal_propagates_to_child_dom() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let child_fixture = concat!(
        "// @runsOn client\n",
        "type ListProps = { items: string[]; };\n",
        "export function List(props: ListProps) {\n",
        "  return (\n",
        "    <ul>\n",
        "      <For each={props.items.value} key={(x) => x}>\n",
        "        {(item) => <li class=\"item\">{item}</li>}\n",
        "      </For>\n",
        "    </ul>\n",
        "  );\n",
        "}\n",
    );
    parse_validate_generate(&dir, "List", child_fixture);

    let parent_fixture = concat!(
        "// @runsOn client\n",
        "import { List } from './List';\n",
        "type AppProps = {};\n",
        "export function App(props: AppProps) {\n",
        "  const items = signal(['A', 'B']);\n",
        "  return (\n",
        "    <div>\n",
        "      <List items={items} />\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    parse_validate_generate(&dir, "App", parent_fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { App } from './App.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;

const root = dom.window.document.createElement('div');
const result = App({});
root.appendChild(result);

function getItemTexts() {
    return [...root.querySelectorAll('.item')].map(li => li.textContent);
}

// initial: 2 items
let texts = getItemTexts();
if (texts.join(',') !== 'A,B') {
    console.error('FAIL initial: ' + texts.join(','));
    process.exit(1);
}

// mutate parent signal — child should update
result._signals.items.set(['A', 'B', 'C']);
await new Promise(r => setTimeout(r, 0));

texts = getItemTexts();
if (texts.join(',') === 'A,B,C') {
    console.log('PASS');
} else {
    console.error('FAIL after set: ' + texts.join(','));
    process.exit(1);
}
"#;

    run_node(&dir, runner);
}

/// Signal passed by reference: child reads .value internally. Changing the
/// parent's signal updates the child's bound DOM nodes without recreating
/// the whole component — internal state survives.
#[test]
fn child_internal_state_survives_parent_prop_change() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let child_fixture = concat!(
        "// @runsOn client\n",
        "type ChildProps = { label: string; };\n",
        "export function Child(props: ChildProps) {\n",
        "  const counter = signal(0);\n",
        "  return (\n",
        "    <div>\n",
        "      <span class=\"label\">{props.label.value}</span>\n",
        "      <span class=\"counter\">{counter.value}</span>\n",
        "      <button onClick={() => counter.set(counter.value + 1)}>+</button>\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    parse_validate_generate(&dir, "Child", child_fixture);

    let parent_fixture = concat!(
        "// @runsOn client\n",
        "import { Child } from './Child';\n",
        "type AppProps = {};\n",
        "export function App(props: AppProps) {\n",
        "  const label = signal('Hello');\n",
        "  return <Child label={label} />;\n",
        "}\n",
    );
    parse_validate_generate(&dir, "App", parent_fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { App } from './App.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;

const root = dom.window.document.createElement('div');
const result = App({});
root.appendChild(result);

const counterEl = root.querySelector('.counter');
const labelEl = root.querySelector('.label');
const btn = root.querySelector('button');

// Bump internal counter to 3
btn.click(); btn.click(); btn.click();
await new Promise(r => setTimeout(r, 0));
if (counterEl.textContent !== '3') {
    console.error('FAIL counter after clicks: ' + counterEl.textContent);
    process.exit(1);
}

// Change parent's label — signal passed by reference, child reads
// props.label.value inside its own bind. Internal state survives.
result._signals.label.set('World');
await new Promise(r => setTimeout(r, 0));

const labelNow = root.querySelector('.label').textContent;
const counterNow = root.querySelector('.counter').textContent;

if (labelNow === 'World' && counterNow === '3') {
    console.log('PASS');
} else {
    console.error('FAIL', { labelNow, counterNow, wantLabel: 'World', wantCounter: '3' });
    process.exit(1);
}
"#;

    run_node(&dir, runner);
}

/// Non-signal .value access (e.g. a plain object with a `value` field)
/// must NOT trigger PROP_UNWRAPPED_SIGNAL — only known signal/computed
/// identifiers are checked.
#[test]
fn plain_object_value_is_not_flagged_as_unwrapped_signal() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let child_fixture = concat!(
        "// @runsOn client\n",
        "type CardProps = { title: string; };\n",
        "export function Card(props: CardProps) {\n",
        "  return <span class=\"card-title\">{props.title}</span>;\n",
        "}\n",
    );
    parse_validate_generate(&dir, "Card", child_fixture);

    let parent_fixture = concat!(
        "// @runsOn client\n",
        "import { Card } from './Card';\n",
        "type AppProps = { config: { value: string; }; };\n",
        "export function App(props: AppProps) {\n",
        "  // props.config is NOT a signal — props.config.value is a plain access\n",
        "  return <Card title={props.config.value} />;\n",
        "}\n",
    );
    parse_validate_generate(&dir, "App", parent_fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { App } from './App.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;

const root = dom.window.document.createElement('div');
const result = App({ config: { value: 'selected' } });
root.appendChild(result);

const span = root.querySelector('.card-title');
if (span && span.textContent === 'selected') {
    console.log('PASS');
} else {
    console.error('FAIL', { text: span ? span.textContent : null });
    process.exit(1);
}
"#;

    run_node(&dir, runner);
}

/// Named function-declaration handler (function handleSubmit() {})
/// must be emitted by codegen and fire when the event is dispatched.
/// This was a blind spot — all prior fixtures used inline arrows.
#[test]
fn named_function_handler_is_emitted_and_fires() {
    let fixture = concat!(
        "// @runsOn client\n",
        "type FormProps = {};\n",
        "export function MyForm(props: FormProps) {\n",
        "  const submitted = signal(false);\n",
        "  function handleSubmit(e) {\n",
        "    e.preventDefault();\n",
        "    submitted.set(true);\n",
        "  }\n",
        "  return (\n",
        "    <div>\n",
        "      <span class=\"status\">{submitted.value ? 'OK' : 'WAIT'}</span>\n",
        "      <form onSubmit={handleSubmit}>\n",
        "        <button type=\"submit\">Go</button>\n",
        "      </form>\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    parse_validate_generate(&dir, "MyForm", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { MyForm } from './MyForm.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
global.Event = dom.window.Event;

const root = dom.window.document.createElement('div');
const result = MyForm({});
root.appendChild(result);

const status = root.querySelector('.status');
const form = root.querySelector('form');

if (!status || status.textContent !== 'WAIT') {
    console.error('FAIL initial: ' + (status ? status.textContent : 'null'));
    process.exit(1);
}

// Dispatch submit on the form — named handler must fire
form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
await new Promise(r => setTimeout(r, 0));

if (status.textContent === 'OK') {
    console.log('PASS');
} else {
    console.error('FAIL after submit: ' + status.textContent);
    process.exit(1);
}
"#;

    run_node(&dir, runner);
}

// ── Task 5 integration tests: newly-supported constructs ──────────────

#[test]
fn binary_operators_in_jsx_expressions() {
    let fixture = concat!(
        "// @runsOn client\n",
        "import { signal } from '@marisjs/runtime';\n",
        "type MathProps = {};\n",
        "export function MathDemo(props: MathProps) {\n",
        "  const a = signal(10);\n",
        "  const b = signal(3);\n",
        "  return (\n",
        "    <div>\n",
        "      <span class=\"lt\">{a.value < b.value ? 'YES' : 'NO'}</span>\n",
        "      <span class=\"gt\">{a.value > b.value ? 'YES' : 'NO'}</span>\n",
        "      <span class=\"eq\">{a.value === 10 ? 'Y' : 'N'}</span>\n",
        "      <span class=\"neq\">{a.value !== 10 ? 'Y' : 'N'}</span>\n",
        "      <span class=\"and\">{a.value === 10 && b.value === 3 ? 'OK' : 'NO'}</span>\n",
        "      <span class=\"or\">{a.value === 0 || b.value === 3 ? 'OK' : 'NO'}</span>\n",
        "      <span class=\"nullish\">{null ?? 'default'}</span>\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    parse_validate_generate(&dir, "MathDemo", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { MathDemo } from './MathDemo.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = MathDemo({});
root.appendChild(result);

function check(cls, expected) {
    const el = root.querySelector('.' + cls);
    if (!el || el.textContent !== expected) {
        console.error('FAIL ' + cls + ': got ' + (el ? el.textContent : 'null') + ' expected ' + expected);
        process.exit(1);
    }
}
check('lt', 'NO');
check('gt', 'YES');
check('eq', 'Y');
check('neq', 'N');
check('and', 'OK');
check('or', 'OK');
check('nullish', 'default');
console.log('PASS');
"#;

    run_node(&dir, runner);
}

#[test]
fn array_and_object_spread_in_expressions() {
    let fixture = concat!(
        "// @runsOn client\n",
        "type SpreadProps = {};\n",
        "export function SpreadDemo(props: SpreadProps) {\n",
        "  return (\n",
        "    <div>\n",
        "      <span class=\"obj\">{JSON.stringify({ ...({ a: 1 }), b: 2 })}</span>\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    parse_validate_generate(&dir, "SpreadDemo", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { SpreadDemo } from './SpreadDemo.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = SpreadDemo({});
root.appendChild(result);

const el = root.querySelector('.obj');
if (!el || el.textContent !== '{"a":1,"b":2}') {
    console.error('FAIL: got ' + (el ? el.textContent : 'null'));
    process.exit(1);
}
console.log('PASS');
"#;

    run_node(&dir, runner);
}

#[test]
fn optional_chaining_in_expression() {
    let fixture = concat!(
        "// @runsOn client\n",
        "type OptProps = {};\n",
        "export function OptDemo(props: OptProps) {\n",
        "  return (\n",
        "    <div>\n",
        "      <span class=\"has\">{({ x: { y: 42 } })?.x?.y ?? 'missing'}</span>\n",
        "      <span class=\"miss\">{(null)?.x?.y ?? 'missing'}</span>\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    parse_validate_generate(&dir, "OptDemo", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { OptDemo } from './OptDemo.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = OptDemo({});
root.appendChild(result);

const has = root.querySelector('.has');
const miss = root.querySelector('.miss');
if (has.textContent !== '42') { console.error('FAIL has: ' + has.textContent); process.exit(1); }
if (miss.textContent !== 'missing') { console.error('FAIL miss: ' + miss.textContent); process.exit(1); }
console.log('PASS');
"#;

    run_node(&dir, runner);
}

#[test]
fn template_literal_in_expression() {
    let fixture = concat!(
        "// @runsOn client\n",
        "type TplProps = {};\n",
        "export function TplDemo(props: TplProps) {\n",
        "  return (\n",
        "    <div>\n",
        "      <span class=\"greet\">{`hello ${'world'}, you have ${5} items`}</span>\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    parse_validate_generate(&dir, "TplDemo", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { TplDemo } from './TplDemo.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = TplDemo({});
root.appendChild(result);

const el = root.querySelector('.greet');
if (!el || el.textContent !== 'hello world, you have 5 items') {
    console.error('FAIL: got ' + (el ? el.textContent : 'null'));
    process.exit(1);
}
console.log('PASS');
"#;

    run_node(&dir, runner);
}

// ── CSS import tests (B2.2) ───────────────────────────────────────────

#[test]
fn bare_css_import_parses_and_validates() {
    let fixture = concat!(
        "// @runsOn client\n",
        "import \"./theme.css\";\n",
        "type StyledProps = {};\n",
        "export function Styled(props: StyledProps) {\n",
        "  return (\n",
        "    <div class=\"container\">\n",
        "      <span>Hello</span>\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture_path = dir.path().join("Styled.tsx");
    std::fs::write(&fixture_path, fixture).unwrap();

    let component = parser::parse_component_file(fixture_path.to_str().unwrap()).unwrap();
    let diags = validator::validate(&component);
    assert!(diags.is_empty(), "bare CSS import should validate cleanly, got: {:?}", diags);

    let css_imports: Vec<_> = component.imports.iter().filter(|i| i.is_css).collect();
    assert_eq!(css_imports.len(), 1, "expected exactly one CSS import");
    assert_eq!(css_imports[0].source, "./theme.css");
    assert!(css_imports[0].imported_names.is_empty(), "bare CSS import should have no bindings");
}

#[test]
fn transitive_css_collected_and_linked_in_page_html() {
    use std::process::Command;

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    // Create CSS files at the same level as their importing components
    std::fs::create_dir_all(dir.path().join("components")).unwrap();
    std::fs::write(dir.path().join("components/Base.css"), ".box { color: red; background: white; }").unwrap();
    std::fs::write(dir.path().join("components/Override.css"), ".box { color: blue; }").unwrap();

    // Leaf component with its own CSS — fixture in components/ subdirectory
    let leaf_fixture = concat!(
        "// @runsOn client\n",
        "import \"./Override.css\";\n",
        "type BoxProps = { label: string; };\n",
        "export function StyledBox(props: BoxProps) {\n",
        "  return (\n",
        "    <div class=\"box\">{props.label}</div>\n",
        "  );\n",
        "}\n",
    );
    let leaf_path = dir.path().join("components/StyledBox.tsx");
    std::fs::write(&leaf_path, leaf_fixture).unwrap();

    // Middle component with its own CSS
    let mid_fixture = concat!(
        "// @runsOn client\n",
        "import \"./Base.css\";\n",
        "import { StyledBox } from './StyledBox';\n",
        "type WrapperProps = {};\n",
        "export function Wrapper(props: WrapperProps) {\n",
        "  return (\n",
        "    <div class=\"wrapper\">\n",
        "      <StyledBox label={'Hello'} />\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    let mid_path = dir.path().join("components/Wrapper.tsx");
    std::fs::write(&mid_path, mid_fixture).unwrap();

    // Page component that imports Wrapper
    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    let page_fixture = concat!(
        "// @runsOn server\n",
        "import { Wrapper } from '../components/Wrapper';\n",
        "type IndexProps = {};\n",
        "export function Index(props: IndexProps) {\n",
        "  return (\n",
        "    <div class=\"page\">\n",
        "      <Wrapper client:hydrate />\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    let page_path = pages_dir.join("Index.tsx");
    std::fs::write(&page_path, page_fixture).unwrap();

    // Build
    let bin = cli_binary();
    let out_dir = dir.path().join("dist");
    let status = Command::new(&bin)
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out_dir)
        .output()
        .unwrap();

    eprintln!("build stderr: {}", String::from_utf8_lossy(&status.stderr));

    if !status.status.success() {
        // Debug: try direct parse/validate
        let component = parser::parse_component_file(page_path.to_str().unwrap()).unwrap();
        let diags = validator::validate(&component);
        eprintln!("page diags: {:?}", diags.iter().map(|d| d.code).collect::<Vec<_>>());
        panic!("build failed: {}", String::from_utf8_lossy(&status.stderr));
    }

    let html = std::fs::read_to_string(out_dir.join("index.html")).unwrap();
    eprintln!("HTML:\n{}", html);

    // Both CSS files should appear as <link> tags in the HTML
    assert!(html.contains("Base.css"), "HTML should link Base.css");
    assert!(html.contains("Override.css"), "HTML should link Override.css");

    // Base.css should come BEFORE Override.css (Wrapper is encountered before StyledBox in tree walk)
    let base_pos = html.find("Base.css").unwrap();
    let override_pos = html.find("Override.css").unwrap();
    assert!(
        base_pos < override_pos,
        "Base.css ({}) should appear before Override.css ({}) in <link> order",
        base_pos, override_pos
    );

    // Verify CSS files were copied to output
    assert!(out_dir.join("components/Base.css").exists(), "Base.css should be copied to out dir");
    assert!(out_dir.join("components/Override.css").exists(), "Override.css should be copied to out dir");
}

#[test]
fn css_class_collision_warns_for_sibling_components_with_same_class() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    // Two SIBLING components (page → ProductCard, page → ReviewCard), each
    // with its own stylesheet, BOTH defining `.header` — a genuine
    // unintentional collision: neither import site is an ancestor of the
    // other, and neither component renders on more than one page.
    std::fs::create_dir_all(dir.path().join("components")).unwrap();
    std::fs::write(dir.path().join("components/ProductCard.css"), ".header { color: red; }").unwrap();
    std::fs::write(dir.path().join("components/ReviewCard.css"), ".header { font-weight: bold; }").unwrap();

    let card_fixture = |name: &str, css: &str| format!(
        "// @runsOn client\nimport \"{}\";\ntype P = {{ label: string; }};\nexport function {}(props: P) {{\n  return <div class=\"header\">{{props.label}}</div>;\n}}\n",
        css, name
    );
    std::fs::write(
        dir.path().join("components/ProductCard.tsx"),
        card_fixture("ProductCard", "./ProductCard.css"),
    ).unwrap();
    std::fs::write(
        dir.path().join("components/ReviewCard.tsx"),
        card_fixture("ReviewCard", "./ReviewCard.css"),
    ).unwrap();

    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    let page_fixture = concat!(
        "// @runsOn server\n",
        "import { ProductCard } from '../components/ProductCard';\n",
        "import { ReviewCard } from '../components/ReviewCard';\n",
        "type IndexProps = {};\n",
        "export function Index(props: IndexProps) {\n",
        "  return (\n",
        "    <div class=\"page\">\n",
        "      <ProductCard client:hydrate label={'a'} />\n",
        "      <ReviewCard client:hydrate label={'b'} />\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    std::fs::write(pages_dir.join("Index.tsx"), page_fixture).unwrap();

    let bin = cli_binary();
    let out_dir = dir.path().join("dist");
    let status = Command::new(&bin)
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out_dir)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&status.stderr).to_string();

    // The warning fires but the build still succeeds (warning, not error).
    assert!(status.status.success(), "build must succeed: {}", stderr);
    assert!(stderr.contains("CSS_CLASS_COLLISION"), "warning code expected: {}", stderr);
    assert!(stderr.contains(".header"), "colliding class named: {}", stderr);
    assert!(
        stderr.contains("ProductCard.css") && stderr.contains("ReviewCard.css"),
        "both source files named: {}",
        stderr
    );
}

#[test]
fn css_class_collision_silent_for_ancestor_override_pattern() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    // The B2/B2.3 cascade-order pattern: Wrapper (ancestor) imports Base.css,
    // StyledBox (descendant) imports Override.css, both defining `.box`. The
    // ancestor's file loads FIRST in the <link> order (DFS tree walk), so the
    // descendant's redefinition is an intentional override — must NOT warn.
    std::fs::create_dir_all(dir.path().join("components")).unwrap();
    std::fs::write(dir.path().join("components/Base.css"), ".box { color: red; background: white; }").unwrap();
    std::fs::write(dir.path().join("components/Override.css"), ".box { color: blue; }").unwrap();

    let leaf_fixture = concat!(
        "// @runsOn client\n",
        "import \"./Override.css\";\n",
        "type BoxProps = { label: string; };\n",
        "export function StyledBox(props: BoxProps) {\n",
        "  return (\n",
        "    <div class=\"box\">{props.label}</div>\n",
        "  );\n",
        "}\n",
    );
    std::fs::write(dir.path().join("components/StyledBox.tsx"), leaf_fixture).unwrap();

    let mid_fixture = concat!(
        "// @runsOn client\n",
        "import \"./Base.css\";\n",
        "import { StyledBox } from './StyledBox';\n",
        "type WrapperProps = {};\n",
        "export function Wrapper(props: WrapperProps) {\n",
        "  return (\n",
        "    <div class=\"wrapper\">\n",
        "      <StyledBox label={'Hello'} />\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    std::fs::write(dir.path().join("components/Wrapper.tsx"), mid_fixture).unwrap();

    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    let page_fixture = concat!(
        "// @runsOn server\n",
        "import { Wrapper } from '../components/Wrapper';\n",
        "type IndexProps = {};\n",
        "export function Index(props: IndexProps) {\n",
        "  return (\n",
        "    <div class=\"page\">\n",
        "      <Wrapper client:hydrate />\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    std::fs::write(pages_dir.join("Index.tsx"), page_fixture).unwrap();

    let bin = cli_binary();
    let out_dir = dir.path().join("dist");
    let status = Command::new(&bin)
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out_dir)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&status.stderr).to_string();

    assert!(status.status.success(), "build failed: {}", stderr);
    assert!(
        !stderr.contains("CSS_CLASS_COLLISION"),
        "cascade-override pattern must not warn: {}",
        stderr
    );
}

#[test]
fn css_class_collision_silent_for_site_wide_layout_stylesheet() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    // The site-wide stylesheet convention (Layout pattern): Layout imports the
    // shared styles.css (defining `.btn`), and a Button component (rendered
    // alongside Layout on EVERY page) imports Button.css, also defining
    // `.btn`. Layout renders on two pages → its stylesheet is the base layer
    // the page-specific Button.css legitimately refines — must NOT warn.
    std::fs::create_dir_all(dir.path().join("components")).unwrap();
    std::fs::write(dir.path().join("components/styles.css"), ".btn { padding: 4px; }\n.nav { display: block; }\n").unwrap();
    std::fs::write(dir.path().join("components/Button.css"), ".btn { color: white; }\n").unwrap();

    let layout_fixture = concat!(
        "// @runsOn client\n",
        "import \"./styles.css\";\n",
        "type LayoutProps = {};\n",
        "export function Layout(props: LayoutProps) {\n",
        "  return <nav class=\"nav\">header</nav>;\n",
        "}\n",
    );
    std::fs::write(dir.path().join("components/Layout.tsx"), layout_fixture).unwrap();

    let button_fixture = concat!(
        "// @runsOn client\n",
        "import \"./Button.css\";\n",
        "type ButtonProps = { label: string; };\n",
        "export function Button(props: ButtonProps) {\n",
        "  return <button class=\"btn\">{props.label}</button>;\n",
        "}\n",
    );
    std::fs::write(dir.path().join("components/Button.tsx"), button_fixture).unwrap();

    let page_fixture = |name: &str| format!(
        "// @runsOn server\nimport {{ Layout }} from '../components/Layout';\nimport {{ Button }} from '../components/Button';\ntype P = {{}};\nexport function {}(props: P) {{\n  return (\n    <div>\n      <Layout client:hydrate />\n      <Button client:hydrate label={{\"go\"}} />\n    </div>\n  );\n}}\n",
        name
    );
    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    std::fs::write(pages_dir.join("Index.tsx"), page_fixture("Index")).unwrap();
    std::fs::write(pages_dir.join("About.tsx"), page_fixture("About")).unwrap();

    let bin = cli_binary();
    let out_dir = dir.path().join("dist");
    let status = Command::new(&bin)
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out_dir)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&status.stderr).to_string();

    assert!(status.status.success(), "build failed: {}", stderr);
    assert!(
        !stderr.contains("CSS_CLASS_COLLISION"),
        "site-wide Layout pattern must not warn: {}",
        stderr
    );
}

#[test]
fn page_head_meta_is_injected_into_built_html() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    let page_fixture = concat!(
        "// @runsOn server\n",
        "type IndexProps = {};\n",
        "export function Index(props: IndexProps) {\n",
        "  const head = '<title>My Page</title><meta name=\"description\" content=\"A test page\">';\n",
        "  return (<div class=\"page\"><h1>Meta test</h1></div>);\n",
        "}\n",
    );
    let page_path = pages_dir.join("Index.tsx");
    std::fs::write(&page_path, page_fixture).unwrap();

    let bin = cli_binary();
    let out_dir = dir.path().join("dist");
    let status = Command::new(&bin)
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out_dir)
        .output()
        .unwrap();

    if !status.status.success() {
        panic!("build failed: {}", String::from_utf8_lossy(&status.stderr));
    }

    let html = std::fs::read_to_string(out_dir.join("index.html")).unwrap();

    let head_start = html.find("<head>").expect("page should have a <head>");
    let head_end = html.find("</head>").expect("page should have a </head>");
    let head = &html[head_start..head_end];

    assert!(
        head.contains("<title>My Page</title>"),
        "built <head> should contain the page title, got: {}",
        head
    );
    assert!(
        head.contains("<meta name=\"description\" content=\"A test page\">"),
        "built <head> should contain the page meta description, got: {}",
        head
    );

    assert!(html.contains("<h1>Meta test</h1>"), "body should still contain the rendered page");
}

#[test]
fn page_head_meta_works_with_hydrate_islands() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let components_dir = dir.path().join("components");
    std::fs::create_dir(&components_dir).unwrap();
    let widget_fixture = concat!(
        "// @runsOn client\n",
        "type WidgetProps = { label: string };\n",
        "export function Widget(props: WidgetProps) {\n",
        "  return <button>{props.label}</button>;\n",
        "}\n",
    );
    std::fs::write(components_dir.join("Widget.tsx"), widget_fixture).unwrap();

    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    let page_fixture = concat!(
        "// @runsOn server\n",
        "import { Widget } from '../components/Widget';\n",
        "type IndexProps = {};\n",
        "export function Index(props: IndexProps) {\n",
        "  const head = '<title>Hybrid</title><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">';\n",
        "  return (<div><Widget label=\"Go\" client:hydrate /></div>);\n",
        "}\n",
    );
    std::fs::write(pages_dir.join("Index.tsx"), page_fixture).unwrap();

    let bin = cli_binary();
    let out_dir = dir.path().join("dist");
    let status = Command::new(&bin)
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out_dir)
        .output()
        .unwrap();

    if !status.status.success() {
        panic!("build failed: {}", String::from_utf8_lossy(&status.stderr));
    }

    let html = std::fs::read_to_string(out_dir.join("index.html")).unwrap();

    let head_start = html.find("<head>").expect("page should have a <head>");
    let head_end = html.find("</head>").expect("page should have a </head>");
    let head = &html[head_start..head_end];

    assert!(
        head.contains("<title>Hybrid</title>"),
        "built <head> should contain the page title, got: {}",
        head
    );
    assert!(
        head.contains("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"),
        "built <head> should contain the viewport meta, got: {}",
        head
    );

    let mjs = std::fs::read_to_string(out_dir.join("pages/Index.mjs")).unwrap();
    assert!(
        mjs.contains("head: head"),
        "compiled page should return the head key, got: {}",
        mjs
    );
    assert!(
        mjs.contains("clientBundles"),
        "compiled page should still return clientBundles, got: {}",
        mjs
    );

    assert!(
        html.contains("data-hydrate=\"Widget\""),
        "hydrate root should still be present"
    );
}

#[test]
fn hydrate_islands_target_correct_data_hydrate_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let components_dir = dir.path().join("components");
    std::fs::create_dir(&components_dir).unwrap();
    let widget_a = concat!(
        "// @runsOn client\n",
        "type WidgetAProps = {};\n",
        "export function WidgetA(props: WidgetAProps) {\n",
        "  return <span class=\"first\">First</span>;\n",
        "}\n",
    );
    std::fs::write(components_dir.join("WidgetA.tsx"), widget_a).unwrap();
    let widget_b = concat!(
        "// @runsOn client\n",
        "type WidgetBProps = {};\n",
        "export function WidgetB(props: WidgetBProps) {\n",
        "  return <span class=\"second\">Second</span>;\n",
        "}\n",
    );
    std::fs::write(components_dir.join("WidgetB.tsx"), widget_b).unwrap();

    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    let page_fixture = concat!(
        "// @runsOn server\n",
        "import { WidgetA } from '../components/WidgetA';\n",
        "import { WidgetB } from '../components/WidgetB';\n",
        "type IndexProps = {};\n",
        "export function Index(props: IndexProps) {\n",
        "  return (<div><h1>Before</h1><WidgetA client:hydrate /><hr /><WidgetB client:hydrate /><h2>After</h2></div>);\n",
        "}\n",
    );
    std::fs::write(pages_dir.join("Index.tsx"), page_fixture).unwrap();

    let bin = cli_binary();
    let out_dir = dir.path().join("dist");
    let status = Command::new(&bin)
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out_dir)
        .output()
        .unwrap();

    if !status.status.success() {
        panic!("build failed: {}", String::from_utf8_lossy(&status.stderr));
    }

    let html = std::fs::read_to_string(out_dir.join("index.html")).unwrap();

    // Each island's mount() call MUST target its specific data-hydrate placeholder
    assert!(
        html.contains("for (const el of document.querySelectorAll('[data-hydrate=\"WidgetA\"]')) { mount(el, () => WidgetA(el.dataset.props ? JSON.parse(el.dataset.props) : {})); }"),
        "WidgetA mount should target [data-hydrate=\"WidgetA\"], got:\n{}",
        &html
    );
    assert!(
        html.contains("for (const el of document.querySelectorAll('[data-hydrate=\"WidgetB\"]')) { mount(el, () => WidgetB(el.dataset.props ? JSON.parse(el.dataset.props) : {})); }"),
        "WidgetB mount should target [data-hydrate=\"WidgetB\"], got:\n{}",
        &html
    );

    // The old pattern (generic root) must NOT appear
    assert!(
        !html.contains("getElementById('root')"),
        "mount script should NOT use generic getElementById('root'); islands must target their specific placeholder"
    );

    // Verify DOM order: static content between islands is preserved in the SSR output
    let before_pos = html.find("<h1>Before</h1>").expect("static <h1>Before</h1> should be present");
    let widget_a_pos = html.find("data-hydrate=\"WidgetA\"").expect("WidgetA placeholder should be present");
    let hr_pos = html.find("<hr").expect("<hr/> between islands should be present");
    let widget_b_pos = html.find("data-hydrate=\"WidgetB\"").expect("WidgetB placeholder should be present");
    let after_pos = html.find("<h2>After</h2>").expect("static <h2>After</h2> should be present");

    assert!(before_pos < widget_a_pos, "static <h1>Before</h1> must come before WidgetA placeholder");
    assert!(widget_a_pos < hr_pos, "WidgetA placeholder must come before <hr/>");
    assert!(hr_pos < widget_b_pos, "<hr/> must come before WidgetB placeholder");
    assert!(widget_b_pos < after_pos, "WidgetB placeholder must come before static <h2>After</h2>");
}

#[test]
fn hydrate_islands_verified_by_playwright_dom_positions() {
    // Requires both the `playwright` npm package and an installed browser
    // binary (Chromium). The npm package alone isn't enough — the browser
    // binary under ~/.cache/ms-playwright must also exist.
    let playwright_available = std::process::Command::new("npx")
        .arg("playwright")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    let home = std::env::var("HOME").unwrap_or_default();
    let pw_cache = std::path::PathBuf::from(home).join(".cache/ms-playwright");
    let browser_found = pw_cache.exists()
        && std::fs::read_dir(&pw_cache)
            .map(|entries| {
                entries.filter_map(|e| e.ok())
                    .any(|e| e.file_name().to_str().map_or(false, |n| n.starts_with("chromium")))
            })
            .unwrap_or(false);

    if !playwright_available || !browser_found {
        eprintln!("SKIP: playwright or Chromium browser not installed — skipping real-browser DOM test");
        return;
    }

    // Build the same fixture used by the static test above
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let components_dir = dir.path().join("components");
    std::fs::create_dir(&components_dir).unwrap();
    std::fs::write(components_dir.join("WidgetA.tsx"), concat!(
        "// @runsOn client\n",
        "type WidgetAProps = {};\n",
        "export function WidgetA(props: WidgetAProps) {\n",
        "  return <span class=\"first\">First</span>;\n",
        "}\n",
    )).unwrap();
    std::fs::write(components_dir.join("WidgetB.tsx"), concat!(
        "// @runsOn client\n",
        "type WidgetBProps = {};\n",
        "export function WidgetB(props: WidgetBProps) {\n",
        "  return <span class=\"second\">Second</span>;\n",
        "}\n",
    )).unwrap();

    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    std::fs::write(pages_dir.join("Index.tsx"), concat!(
        "// @runsOn server\n",
        "import { WidgetA } from '../components/WidgetA';\n",
        "import { WidgetB } from '../components/WidgetB';\n",
        "type IndexProps = {};\n",
        "export function Index(props: IndexProps) {\n",
        "  return (<div><h1>Before</h1><WidgetA client:hydrate /><hr /><WidgetB client:hydrate /><h2>After</h2></div>);\n",
        "}\n",
    )).unwrap();

    let bin = cli_binary();
    let out_dir = dir.path().join("dist");
    let status = std::process::Command::new(&bin)
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out_dir)
        .output()
        .unwrap();
    assert!(status.status.success(), "build failed: {}", String::from_utf8_lossy(&status.stderr));

    // Start the dev server on a random port
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let mut server = std::process::Command::new(&bin)
        .arg("dev")
        .arg(dir.path())
        .arg("--out")
        .arg(&out_dir)
        .arg("--port")
        .arg(port.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();

    // Wait for server — the dev server binds only after its initial build
    // (which spawns Node for prerendering), so under parallel-suite load this
    // can take a while.
    let start = std::time::Instant::now();
    let mut ready = false;
    while start.elapsed() < std::time::Duration::from_secs(30) {
        if std::net::TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", port).parse().unwrap(),
            std::time::Duration::from_millis(200),
        ).is_ok() {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert!(ready, "dev server did not start on port {}", port);

    // Run the Playwright spec
    let spec = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/hydrate-dom-position.spec.mjs");
    let status = std::process::Command::new("npx")
        .arg("playwright")
        .arg("test")
        .arg(spec)
        .arg("--reporter=line")
        .env("MARISJS_DEV_URL", format!("http://127.0.0.1:{}", port))
        .status()
        .unwrap_or_else(|e| {
            server.kill().unwrap();
            let _ = server.wait();
            panic!("failed to run playwright: {}", e);
        });

    server.kill().unwrap();
    let _ = server.wait();

    assert!(status.success(), "Playwright DOM-position test failed");
}

#[test]
fn destructured_arrow_param_in_handler() {
    let fixture = concat!(
        "// @runsOn client\n",
        "type DestructProps = {};\n",
        "export function DestructDemo(props: DestructProps) {\n",
        "  const log = () => {\n",
        "    const fn = ({ x, y }) => x + y;\n",
        "    console.log('computed');\n",
        "  };\n",
        "  return (\n",
        "    <div>\n",
        "      <span class=\"val\">{(({ a, b }) => a + b)({ a: 10, b: 20 })}</span>\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    parse_validate_generate(&dir, "DestructDemo", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { DestructDemo } from './DestructDemo.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = DestructDemo({});
root.appendChild(result);

const el = root.querySelector('.val');
if (!el || el.textContent !== '30') {
    console.error('FAIL: got ' + (el ? el.textContent : 'null'));
    process.exit(1);
}
console.log('PASS');
"#;

    run_node(&dir, runner);
}

// ── data() integration tests ──────────────────────────────────────────

#[test]
fn server_data_emits_derived_const_in_generated_js() {
    let fixture = concat!(
        "// @runsOn server\n",
        "type ShopProps = {};\n",
        "export function Shop(props: ShopProps) {\n",
        "  const products = await data(async () => [\n",
        "    { id: 1, name: 'A' },\n",
        "    { id: 2, name: 'B' },\n",
        "  ]);\n",
        "  return (\n",
        "    <ul>\n",
        "      <For each={products} key={(p) => p.id}>\n",
        "        {(p) => <li>{p.name}</li>}\n",
        "      </For>\n",
        "    </ul>\n",
        "  );\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture_path = dir.path().join("Shop.tsx");
    std::fs::write(&fixture_path, fixture).unwrap();

    let component = parser::parse_component_file(fixture_path.to_str().unwrap()).unwrap();
    let diags = validator::validate(&component);
    assert!(diags.is_empty(), "Validation errors: {:?}", diags);

    let js = codegen::generate(&component).unwrap();
    eprintln!("--- Generated JS (server with data()) ---\n{}", js);

    assert!(js.contains("import { data } from '@marisjs/runtime'"), "missing data import");
    assert!(js.contains("async function Shop"), "function not async");
    assert!(js.contains("await data(async ()"), "missing await data(...) declaration");
    assert!(js.contains(".map("), "missing .map() for <For>");
    assert!(js.contains("id: 1"), "missing data literal");
}

#[test]
fn data_call_in_client_component_is_rejected() {
    let fixture = concat!(
        "// @runsOn client\n",
        "type BadProps = {};\n",
        "export function Bad(props: BadProps) {\n",
        "  const x = data(async () => 1);\n",
        "  return (<div>{x}</div>);\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    let fixture_path = dir.path().join("Bad.tsx");
    std::fs::write(&fixture_path, fixture).unwrap();

    let component = parser::parse_component_file(fixture_path.to_str().unwrap()).unwrap();
    let diags = validator::validate(&component);
    assert!(
        diags.iter().any(|d| d.code == "CLIENT_DATA_CALL"),
        "Expected CLIENT_DATA_CALL error, got: {:?}",
        diags.iter().map(|d| d.code).collect::<Vec<_>>()
    );
}

#[test]
fn server_component_with_data_prerenders_correct_html() {
    use std::process::Command;

    let fixture = concat!(
        "// @runsOn server\n",
        "type MenuProps = {};\n",
        "export function Menu(props: MenuProps) {\n",
        "  const items = await data(async () => [\n",
        "    { id: 1, name: 'Coffee' },\n",
        "    { id: 2, name: 'Tea' },\n",
        "  ]);\n",
        "  return (\n",
        "    <ul>\n",
        "      <For each={items} key={(x) => x.id}>\n",
        "        {(x) => <li>{x.name}</li>}\n",
        "      </For>\n",
        "    </ul>\n",
        "  );\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    let fixture_path = pages_dir.join("Menu.tsx");
    std::fs::write(&fixture_path, fixture).unwrap();

    let bin = cli_binary();
    let out_dir = dir.path().join("dist");
    let status = Command::new(bin)
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out_dir)
        .output()
        .unwrap();

    eprintln!("stderr: {}", String::from_utf8_lossy(&status.stderr));

    if !status.status.success() {
        let component = parser::parse_component_file(fixture_path.to_str().unwrap()).unwrap();
        eprintln!("data_call: {} at {}:{}, has_component_body: {}", component.has_data_call, component.data_call_line, component.data_call_column, component.has_component_body);
        let diags = validator::validate(&component);
        eprintln!("diags: {:?}", diags.iter().map(|d| d.code).collect::<Vec<_>>());
        if let Ok(js) = codegen::generate(&component) {
            eprintln!("generated:\n{}", js);
        } else {
            eprintln!("codegen failed");
        }
        panic!("build failed: {}", String::from_utf8_lossy(&status.stderr));
    }

    let html = std::fs::read_to_string(out_dir.join("menu.html")).unwrap();
    eprintln!("HTML:\n{}", html);

    assert!(html.contains("Coffee"), "HTML should contain Coffee");
    assert!(html.contains("Tea"), "HTML should contain Tea");
    assert!(html.contains("<li>"), "HTML should contain <li> elements");
}

#[test]
fn signal_prop_drills_three_levels_deep() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    // Leaf (level 3) — the only component that actually reads the signal
    let leaf_fixture = concat!(
        "// @runsOn client\n",
        "type LeafProps = { theme: { color: string; }; };\n",
        "export function ThemeLeaf(props: LeafProps) {\n",
        "  return (\n",
        "    <span class=\"leaf-color\">{props.theme.value.color}</span>\n",
        "  );\n",
        "}\n",
    );
    parse_validate_generate(&dir, "ThemeLeaf", leaf_fixture);

    // Middle (level 2) — pass-through, does NOT read theme
    let mid_fixture = concat!(
        "// @runsOn client\n",
        "import { ThemeLeaf } from './ThemeLeaf';\n",
        "type MidProps = { theme: { color: string; }; title: string; };\n",
        "export function ThemeMiddle(props: MidProps) {\n",
        "  return (\n",
        "    <div class=\"middle\">\n",
        "      <h1>{props.title}</h1>\n",
        "      <ThemeLeaf theme={props.theme} />\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    parse_validate_generate(&dir, "ThemeMiddle", mid_fixture);

    // Root (level 1) — creates the theme signal
    let root_fixture = concat!(
        "// @runsOn client\n",
        "import { ThemeMiddle } from './ThemeMiddle';\n",
        "type AppProps = {};\n",
        "export function App(props: AppProps) {\n",
        "  const theme = signal({ color: 'red' });\n",
        "  return (\n",
        "    <div class=\"app\">\n",
        "      <ThemeMiddle theme={theme} title={'My App'} />\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    parse_validate_generate(&dir, "App", root_fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { App } from './App.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;

const root = dom.window.document.createElement('div');
const result = App({});
root.appendChild(result);

const colorSpan = root.querySelector('.leaf-color');
if (!colorSpan || colorSpan.textContent !== 'red') {
    console.error('FAIL initial: ' + (colorSpan ? colorSpan.textContent : 'null'));
    process.exit(1);
}

// Change the signal at the root — leaf should update 3 levels down
result._signals.theme.set({ color: 'blue' });
await new Promise(r => setTimeout(r, 0));

if (colorSpan.textContent === 'blue') {
    console.log('PASS');
} else {
    console.error('FAIL after set: ' + colorSpan.textContent);
    process.exit(1);
}
"#;

    run_node(&dir, runner);
}

#[test]
fn data_fetcher_rejection_fails_the_build() {
    use std::process::Command;

    let fixture = concat!(
        "// @runsOn server\n",
        "type FailProps = {};\n",
        "export function Fail(props: FailProps) {\n",
        "  const x = await data(async () => {\n",
        "    throw new Error('FETCH_FAILED');\n",
        "  });\n",
        "  return (<div>{x}</div>);\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    let fixture_path = pages_dir.join("Fail.tsx");
    std::fs::write(&fixture_path, fixture).unwrap();

    let bin = cli_binary();
    let out_dir = dir.path().join("dist");
    let status = Command::new(bin)
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out_dir)
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&status.stderr);
    eprintln!("build stderr: {}", stderr);

    assert!(!status.status.success(), "build should fail when data() fetcher rejects");
    assert!(
        stderr.contains("FETCH_FAILED") || stderr.contains("prerender failed"),
        "error output should mention fetch failure, got: {}",
        stderr
    );

    // Verify no HTML file was produced
    let html_path = out_dir.join("fail.html");
    assert!(!html_path.exists(), "no HTML file should be produced on failed build");
}

#[test]
fn derived_const_emitted_and_accessible_in_jsx() {
    let fixture = concat!(
        "// @runsOn client\n",
        "type GreetProps = {};\n",
        "export function Greet(props: GreetProps) {\n",
        "  const name = 'world';\n",
        "  const count = 5;\n",
        "  const message = `hello ${name}, you have ${count} items`;\n",
        "  return (\n",
        "    <div>\n",
        "      <span class=\"greet\">{message}</span>\n",
        "      <span class=\"count\">{count}</span>\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    parse_validate_generate(&dir, "Greet", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { Greet } from './Greet.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = Greet({});
root.appendChild(result);

const greet = root.querySelector('.greet');
const count = root.querySelector('.count');
if (!greet || greet.textContent !== 'hello world, you have 5 items') {
    console.error('FAIL greet: got ' + (greet ? greet.textContent : 'null'));
    process.exit(1);
}
if (!count || count.textContent !== '5') {
    console.error('FAIL count: got ' + (count ? count.textContent : 'null'));
    process.exit(1);
}
console.log('PASS');
"#;

    run_node(&dir, runner);
}

// ── nested data() tests (B3 App 3) ───────────────────────────────────

#[test]
fn nested_server_data_renders_deep_author_info() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let child_fixture = concat!(
        "// @runsOn server\n",
        "type AuthorProps = { authorId: number; };\n",
        "export function Author(props: AuthorProps) {\n",
        "  const author = await data(async () => ({ name: 'Alice', bio: 'Writer' }));\n",
        "  return (<div class=\"author\"><span class=\"name\">{author.name}</span><span class=\"bio\">{author.bio}</span></div>);\n",
        "}\n",
    );
    let child_path = dir.path().join("Author.tsx");
    std::fs::write(&child_path, child_fixture).unwrap();

    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    let page_fixture = concat!(
        "// @runsOn server\n",
        "import { Author } from '../Author';\n",
        "type IndexProps = {};\n",
        "export function Index(props: IndexProps) {\n",
        "  const title = await data(async () => 'Blog');\n",
        "  return (<div class=\"page\"><span class=\"title\">{title}</span><Author authorId={1} /></div>);\n",
        "}\n",
    );
    let page_path = pages_dir.join("Index.tsx");
    std::fs::write(&page_path, page_fixture).unwrap();

    use std::process::Command;
    let bin = cli_binary();
    let out_dir = dir.path().join("dist");
    let status = Command::new(&bin)
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out_dir)
        .output()
        .unwrap();

    if !status.status.success() {
        panic!("build failed: {}", String::from_utf8_lossy(&status.stderr));
    }

    let html = std::fs::read_to_string(out_dir.join("index.html")).unwrap();
    assert!(html.contains("Blog"), "title");
    assert!(html.contains("Alice"), "author name");
    assert!(html.contains("Writer"), "author bio");
}

#[test]
fn nested_data_rejection_fails_build() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let child_fixture = concat!(
        "// @runsOn server\n",
        "type FailChildProps = {};\n",
        "export function FailChild(props: FailChildProps) {\n",
        "  const x = await data(async () => { throw new Error('NESTED_FETCH_FAIL'); });\n",
        "  return (<span>{x}</span>);\n",
        "}\n",
    );
    let child_path = dir.path().join("FailChild.tsx");
    std::fs::write(&child_path, child_fixture).unwrap();

    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    let page_fixture = concat!(
        "// @runsOn server\n",
        "import { FailChild } from '../FailChild';\n",
        "type IndexProps = {};\n",
        "export function Index(props: IndexProps) {\n",
        "  const ok = await data(async () => 'ok');\n",
        "  return (<div><span class=\"ok\">{ok}</span><FailChild /></div>);\n",
        "}\n",
    );
    let page_path = pages_dir.join("Index.tsx");
    std::fs::write(&page_path, page_fixture).unwrap();

    use std::process::Command;
    let bin = cli_binary();
    let out_dir = dir.path().join("dist");
    let status = Command::new(&bin)
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out_dir)
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&status.stderr);
    assert!(!status.status.success(), "build should fail when nested data() throws, stderr: {}", stderr);
    assert!(stderr.contains("NESTED_FETCH_FAIL") || stderr.contains("prerender failed"),
        "error should mention nested fetch failure, got: {}", stderr);
}

// ── For-item signal patterns (spec §4b) ─────────────────────────────

/// Proves the working pattern: when a For item template reads an
/// outer-scope signal via .value directly, the codegen emits per-item
/// bind() wrappers inside _r9, and existing bar widths update when the
/// signal changes — no reconciliation restructure needed.
#[test]
fn for_item_direct_signal_read_updates_correctly() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type ChartProps = {};\n",
        "export function Chart(props: ChartProps) {\n",
        "  const items = signal([\n",
        "    { id: 1, val: 100 },\n",
        "    { id: 2, val: 200 },\n",
        "  ]);\n",
        "  const maxVal = computed(() => Math.max(...items.value.map(i => i.val)));\n",
        "  return (\n",
        "    <div>\n",
        "      <For each={items.value} key={(x) => x.id}>\n",
        "        {(item) => <div class=\"bar\" style={'width:' + Math.round((item.val / maxVal.value) * 100) + 'px'}>{item.val}</div>}\n",
        "      </For>\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    parse_validate_generate(&dir, "Chart", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { Chart } from './Chart.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;

const root = dom.window.document.createElement('div');
const result = Chart({});
root.appendChild(result);

function getWidths() {
    return [...root.querySelectorAll('.bar')].map(b => b.getAttribute('style'));
}

// Initial: vals 100,200 → maxVal=200 → widths 50px, 100px
const w0 = getWidths();

// Add a new item with val=500 → maxVal becomes 500
// maxVal.value IS read directly in the expression → bind wrapper fires → existing items update
result._signals.items.set([
    { id: 1, val: 100 },
    { id: 2, val: 200 },
    { id: 3, val: 500 },
]);
await new Promise(r => setTimeout(r, 50));
const w1 = getWidths();

// Existing items (id=1: 100/500=20px, id=2: 200/500=40px) MUST update
// New item (id=3: 500/500=100px) gets fresh render
if (w1.length !== 3) { console.error('FAIL: expected 3 bars'); process.exit(1); }
if (!w1[2].includes('100px')) { console.error('FAIL: new bar should be 100px'); process.exit(1); }

// The critical assertion: direct .value read in template → existing items DO update
if (w1[0].includes('20px') && w1[1].includes('40px')) {
    console.log('PASS');
} else {
    console.error('FAIL: existing bars did not update, got ' + w1.join('|'));
    process.exit(1);
}
"#;

    run_node(&dir, runner);
}

/// Documents the intentional limitation: when item data is pre-computed
/// through computed().map() producing plain objects, item.percentage
/// carries no signal identity and the compiler emits no bind wrapper.
/// Existing items keep stale widths when the underlying signal changes.
/// The fix is to read .value directly in the template (see above test).
#[test]
fn for_item_precomputed_plain_value_does_not_update() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type ChartProps = {};\n",
        "export function ChartPlain(props: ChartProps) {\n",
        "  const items = signal([\n",
        "    { id: 1, val: 100 },\n",
        "    { id: 2, val: 200 },\n",
        "  ]);\n",
        "  const maxVal = computed(() => Math.max(...items.value.map(i => i.val)));\n",
        "  const barData = computed(() => items.value.map(i => ({\n",
        "    id: i.id,\n",
        "    percentage: Math.round((i.val / maxVal.value) * 100),\n",
        "  })));\n",
        "  return (\n",
        "    <div>\n",
        "      <For each={barData.value} key={(x) => x.id}>\n",
        "        {(item) => <div class=\"bar\" style={'width:' + item.percentage + 'px'}>{item.percentage}</div>}\n",
        "      </For>\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    parse_validate_generate(&dir, "ChartPlain", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { ChartPlain } from './ChartPlain.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;

const root = dom.window.document.createElement('div');
const result = ChartPlain({});
root.appendChild(result);

function getWidths() {
    return [...root.querySelectorAll('.bar')].map(b => b.getAttribute('style'));
}

// Initial: vals 100,200 → maxVal=200 → percentages 50, 100 → widths 50px, 100px
const w0 = getWidths();

// Add a new item with val=500 → maxVal becomes 500
// barData recomputes → new percentages 20, 40, 100 → but item.percentage
// has no .value → no bind wrapper → existing items keep OLD widths
result._signals.items.set([
    { id: 1, val: 100 },
    { id: 2, val: 200 },
    { id: 3, val: 500 },
]);
await new Promise(r => setTimeout(r, 50));
const w1 = getWidths();

// New item (id=3) should get fresh render with 100px
if (w1.length !== 3) { console.error('FAIL: expected 3 bars'); process.exit(1); }
if (!w1[2].includes('100px')) { console.error('FAIL: new bar should be 100px'); process.exit(1); }

// Existing items should STILL have old widths (50px, 100px) — this is the
// documented limitation: plain values from computed().map() don't update
if (w1[0].includes('50px') && w1[1].includes('100px')) {
    console.log('PASS');
} else if (w1[0].includes('20px') && w1[1].includes('40px')) {
    console.log('SURPRISE: limitation no longer applies — existing items updated');
    console.log('PASS');
} else {
    console.error('UNEXPECTED: got ' + w1.join('|'));
    process.exit(1);
}
"#;

    run_node(&dir, runner);
}


#[test]
fn client_boolean_attr_presence_semantics() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type Props = {};\n",
        "export function Btn(props: Props) {\n",
        "  const on = signal(true);\n",
        "  return <button disabled={on.value}>Go</button>;\n",
        "}\n",
    );
    parse_validate_generate(&dir, "Btn", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { Btn } from './Btn.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document; global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const btn = Btn({}); root.appendChild(btn);
if (!btn.hasAttribute('disabled')) { console.error('FAIL: truthy signal should set disabled'); process.exit(1); }
btn._signals.on.set(false);
await new Promise(r => setTimeout(r, 50));
if (btn.hasAttribute('disabled')) { console.error('FAIL: falsy signal should remove disabled'); process.exit(1); }
btn._signals.on.set(true);
await new Promise(r => setTimeout(r, 50));
if (!btn.hasAttribute('disabled')) { console.error('FAIL: re-set should re-add disabled'); process.exit(1); }
console.log('PASS');
"#;

    run_node(&dir, runner);
}


#[test]
fn client_fragment_renders_children_inline() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type Props = {};\n",
        "export function Frag(props: Props) {\n",
        "  return (<>\n",
        "    <span class=\"a\">A</span>\n",
        "    <span class=\"b\">B</span>\n",
        "  </>);\n",
        "}\n",
    );
    parse_validate_generate(&dir, "Frag", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { Frag } from './Frag.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document; global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = Frag({}); root.appendChild(result);
const spans = root.querySelectorAll('span');
if (spans.length !== 2) { console.error('FAIL: expected 2 fragment children, got ' + spans.length); process.exit(1); }
if (spans[0].textContent !== 'A' || spans[1].textContent !== 'B') { console.error('FAIL: fragment children order/content'); process.exit(1); }
console.log('PASS');
"#;

    run_node(&dir, runner);
}

// ── Regression: .value property assignment on input elements ───────



/// Verifies that reactive `value` bindings use direct DOM property
/// assignment (`el.value = ...`) rather than `setAttribute('value', ...)`.
#[test]
fn value_attr_uses_dom_property_not_setattribute() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type Props = {};\n",
        "export function InputTest(props: Props) {\n",
        "  const text = signal('hello');\n",
        "  return <input type=\"text\" value={text.value} />;\n",
        "}\n",
    );
    let js = parse_validate_generate(&dir, "InputTest", fixture);

    assert!(
        !js.contains("setAttribute('value'"),
        "value attr should NOT use setAttribute — should use .value property assignment.\nGenerated:\n{}",
        js
    );
    assert!(
        js.contains(".value = ") || js.contains(".value="),
        "value attr should use .value property assignment.\nGenerated:\n{}",
        js
    );
}

// ── Regression: derived const reading signal.value emits computed() ─

#[test]
fn reactive_derived_const_emitted_as_computed() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type Props = {};\n",
        "export function DerivedTest(props: Props) {\n",
        "  const count = signal(5);\n",
        "  const doubled = count.value * 2;\n",
        "  return <div>{doubled.value}</div>;\n",
        "}\n",
    );
    let js = parse_validate_generate(&dir, "DerivedTest", fixture);

    assert!(
        js.contains("computed(()"),
        "Reactive derived const should emit computed().\nGenerated:\n{}",
        js
    );
}

// ── Regression: TS type annotations stripped from handler source ────

#[test]
fn ts_type_annotations_stripped_from_handler_source() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type Props = {};\n",
        "export function HandlerTest(props: Props) {\n",
        "  const count = signal(0);\n",
        "  function increment(amount: number): void {\n",
        "    count.set(count.value + amount);\n",
        "  }\n",
        "  return <button onClick={increment}>+</button>;\n",
        "}\n",
    );
    let js = parse_validate_generate(&dir, "HandlerTest", fixture);

    assert!(
        !(js.contains("amount: number") || js.contains("): number") || js.contains("): void")),
        "Handler source must not contain TS type annotations.\nGenerated:\n{}",
        js
    );
    assert!(
        js.contains("function increment(amount)"),
        "Handler should have stripped parameter type.\nGenerated:\n{}",
        js
    );
}

// ── Regression: object-literal types in TS annotations ─────────────
// The old raw-text brace-counting approach broke on object-literal
// types like { target: { value: string } } because the braces in the
// type annotation were confused with the function body's opening brace.
// AST-based stripping (parser uses SWC span info) handles these correctly.

/// Call site 1: handler function with object-literal parameter and return types.
#[test]
fn object_literal_types_stripped_from_handler() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type Props = {};\n",
        "export function HandlerObjTest(props: Props) {\n",
        "  const count = signal(0);\n",
        "  function handleChange(e: { target: { value: string } }): { ok: boolean } {\n",
        "    count.set(count.value + 1);\n",
        "    return { ok: true };\n",
        "  }\n",
        "  return <button onClick={handleChange}>change</button>;\n",
        "}\n",
    );
    let js = parse_validate_generate(&dir, "HandlerObjTest", fixture);

    assert!(
        !(js.contains("target: { value: string }") || js.contains("): { ok: boolean }")),
        "Handler source must not contain object-literal TS type annotations.\nGenerated:\n{}",
        js
    );
    assert!(
        js.contains("function handleChange(e)"),
        "Handler should have stripped object-literal param type.\nGenerated:\n{}",
        js
    );
    // The body braces in { target: { value: string } } must NOT be treated
    // as the function body — the return statement must survive.
    assert!(
        js.contains("return { ok: true }"),
        "Handler body with return object must survive stripping.\nGenerated:\n{}",
        js
    );
}

/// Call site 2: derived const with an object-literal type annotation.
#[test]
fn object_literal_type_stripped_from_derived_const() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type Props = {};\n",
        "export function DerivedObjTest(props: Props) {\n",
        "  const count = signal(0);\n",
        "  const doubled: { val: number } = count.value * 2;\n",
        "  return <div>{doubled.value}</div>;\n",
        "}\n",
    );
    let js = parse_validate_generate(&dir, "DerivedObjTest", fixture);

    assert!(
        !js.contains("{ val: number }"),
        "Derived const must not contain object-literal type annotation.\nGenerated:\n{}",
        js
    );
    assert!(
        js.contains("const doubled = "),
        "Derived const should have stripped type, keeping 'const doubled ='.\nGenerated:\n{}",
        js
    );
}

/// Call site 3: For-body captured const with an object-literal type.
#[test]
fn object_literal_type_stripped_from_for_body_const() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type Props = {};\n",
        "export function ForObjTest(props: Props) {\n",
        "  const items = signal([{ name: 'a' }]);\n",
        "  return (\n",
        "    <ul>\n",
        "      <For each={items.value} key={(x) => x.name}>\n",
        "        {(item) => {\n",
        "          const label: { text: string } = item.name;\n",
        "          return <li>{label.text}</li>;\n",
        "        }}\n",
        "      </For>\n",
        "    </ul>\n",
        "  );\n",
        "}\n",
    );
    let js = parse_validate_generate(&dir, "ForObjTest", fixture);

    assert!(
        !js.contains("{ text: string }"),
        "For-body const must not contain object-literal type annotation.\nGenerated:\n{}",
        js
    );
    assert!(
        js.contains("const label = "),
        "For-body const should have stripped type, keeping 'const label ='.\nGenerated:\n{}",
        js
    );
}

/// Nested object-literal type (3 levels deep): confirms AST-based approach
/// handles arbitrary nesting since each type annotation span covers the
/// full expression regardless of internal brace depth.
#[test]
fn deeply_nested_object_type_stripped_from_handler() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type Props = {};\n",
        "export function DeepNestedTest(props: Props) {\n",
        "  const count = signal(0);\n",
        "  function handleEvent(e: { level1: { level2: { level3: { val: string } } } }): void {\n",
        "    count.set(count.value + 1);\n",
        "  }\n",
        "  return <button onClick={handleEvent}>click</button>;\n",
        "}\n",
    );
    let js = parse_validate_generate(&dir, "DeepNestedTest", fixture);

    assert!(
        !(js.contains("level1: {") || js.contains("level2: {") || js.contains("level3: {")),
        "Handler source must not contain any deeply-nested type annotation braces.\nGenerated:\n{}",
        js
    );
    assert!(
        js.contains("function handleEvent(e)"),
        "Handler should have stripped deeply-nested param type.\nGenerated:\n{}",
        js
    );
    assert!(
        !js.contains("): void"),
        "Handler should have stripped return type.\nGenerated:\n{}",
        js
    );
}

// ── Regression: block-bodied For arrow extracts JSX from return ─────

#[test]
fn block_body_for_arrow_extracts_jsx_from_return() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type Props = {};\n",
        "export function ForBlockTest(props: Props) {\n",
        "  const items = signal([{ id: 1, name: 'a' }]);\n",
        "  return (\n",
        "    <div>\n",
        "      <For each={items.value} key={(x) => x.id}>\n",
        "        {(item) => {\n",
        "          const label = item.name.toUpperCase();\n",
        "          return <span>{label}</span>;\n",
        "        }}\n",
        "      </For>\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    let js = parse_validate_generate(&dir, "ForBlockTest", fixture);

    assert!(
        js.contains("createElement('span')"),
        "For render function should create span from block body return.\nGenerated:\n{}",
        js
    );
}

// ── Regression: For block-body handler is per-item scoped ──────────

/// Verifies that a handler function declared inside a `<For>` item's
/// block body is correctly emitted inside `_rX` and scoped per-item —
/// deleting a specific item only removes that item, not others, and
/// each handler closes over its own item data.
#[test]
fn for_block_body_handler_is_per_item_scoped() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type Props = {};\n",
        "export function ScopedDeleteTest(props: Props) {\n",
        "  const items = signal([\n",
        "    { id: 1, label: 'first' },\n",
        "    { id: 2, label: 'second' },\n",
        "  ]);\n",
        "  return (\n",
        "    <ul>\n",
        "      <For each={items.value} key={(x) => x.id}>\n",
        "        {(item) => {\n",
        "          function remove() {\n",
        "            items.set(items.value.filter((i) => i.id !== item.id));\n",
        "          }\n",
        "          return <li><span>{item.label}</span><button onClick={remove}>X</button></li>;\n",
        "        }}\n",
        "      </For>\n",
        "    </ul>\n",
        "  );\n",
        "}\n",
    );
    parse_validate_generate(&dir, "ScopedDeleteTest", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { ScopedDeleteTest } from './ScopedDeleteTest.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;

const root = dom.window.document.createElement('div');
const result = ScopedDeleteTest({});
root.appendChild(result);

function getLabels() {
    return [...root.querySelectorAll('span')].map(s => s.textContent);
}

// Initial state: two items
const labels0 = getLabels();
if (labels0.length !== 2) { console.error('FAIL: expected 2 items, got', labels0); process.exit(1); }
if (labels0[0] !== 'first' || labels0[1] !== 'second') { console.error('FAIL: wrong labels', labels0); process.exit(1); }

// Click delete on the SECOND item's button
const buttons = root.querySelectorAll('button');
if (buttons.length !== 2) { console.error('FAIL: expected 2 buttons, got', buttons.length); process.exit(1); }
buttons[1].dispatchEvent(new dom.window.Event('click', { bubbles: true }));
await new Promise(r => setTimeout(r, 50));

const labels1 = getLabels();
if (labels1.length !== 1) { console.error('FAIL: expected 1 item after delete, got', labels1.length); process.exit(1); }
// The REMAINING item should be 'first' (the one we DIDN'T click)
if (labels1[0] !== 'first') { console.error('FAIL: wrong remaining item:', labels1[0], '- should be first'); process.exit(1); }

// Verify the deleted item was 'second' — its handler was per-item scoped
console.log('PASS');
"#;

    run_node(&dir, runner);
}

// ── Regression: multiple sibling ternaries with null branches ──────

#[test]
fn multiple_sibling_ternaries_each_swap_independently() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type Props = {};\n",
        "export function MultiTabs(props: Props) {\n",
        "  const active = signal('a');\n",
        "  return (\n",
        "    <div>\n",
        "      <button onClick={() => active.set('a')}>A</button>\n",
        "      <button onClick={() => active.set('b')}>B</button>\n",
        "      {active.value === 'a' ? <p id=\"ta\">Content A</p> : null}\n",
        "      {active.value === 'b' ? <p id=\"tb\">Content B</p> : null}\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    parse_validate_generate(&dir, "MultiTabs", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { MultiTabs } from './MultiTabs.mjs';
const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document; global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = MultiTabs({}); root.appendChild(result);

const btnA = root.querySelectorAll('button')[0];
const btnB = root.querySelectorAll('button')[1];
if (!root.querySelector('#ta')) { console.error('FAIL: Content A initial'); process.exit(1); }
if (root.querySelector('#tb')) { console.error('FAIL: Content B initial'); process.exit(1); }
btnB.dispatchEvent(new dom.window.Event('click', { bubbles: true }));
await new Promise(r => setTimeout(r, 50));
if (root.querySelector('#ta')) { console.error('FAIL: A hidden after B'); process.exit(1); }
if (!root.querySelector('#tb')) { console.error('FAIL: B visible after click'); process.exit(1); }
btnA.dispatchEvent(new dom.window.Event('click', { bubbles: true }));
await new Promise(r => setTimeout(r, 50));
if (!root.querySelector('#ta')) { console.error('FAIL: A visible after A'); process.exit(1); }
if (root.querySelector('#tb')) { console.error('FAIL: B hidden after A'); process.exit(1); }
console.log('PASS');
"#;

    run_node(&dir, runner);
}

// ───────────────────────────────────────────────────────────────────────────
// Nested route path handling (bug class: /docs/api/signals at 2+ levels)
// ───────────────────────────────────────────────────────────────────────────

/// Writes a project with a two-level nested route. The page file uses
/// source-preserved casing (pages/Docs/Api/Signals.tsx → route /docs/api/signals)
/// and imports a hydrate island from a sibling directory with parent-dir
/// traversal (../../components/Widget), which exercises relative import
/// resolution at depth. The island owns a CSS file (transitive CSS collection).
fn write_nested_fixture(dir: &tempfile::TempDir, server_mode: bool) {
    let src = dir.path();
    std::fs::create_dir_all(src.join("pages/Docs/Api")).unwrap();
    std::fs::create_dir_all(src.join("pages/components")).unwrap();
    std::fs::create_dir_all(src.join("images")).unwrap();

    std::fs::write(src.join("pages/Index.tsx"), concat!(
        "// @runsOn server\n",
        "type IndexProps = {};\n",
        "export function Index(props: IndexProps) { return <div>Home</div>; }\n",
    ))
    .unwrap();

    std::fs::write(src.join("pages/components/Widget.tsx"), concat!(
        "// @runsOn client\n",
        "import './Widget.css';\n",
        "type WidgetProps = { label: string };\n",
        "export function Widget(props: WidgetProps) {\n",
        "  return (\n",
        "    <div class=\"widget\"><span class=\"widget-label\">{props.label}</span></div>\n",
        "  );\n",
        "}\n",
    ))
    .unwrap();

    std::fs::write(src.join("pages/components/Widget.css"), ".widget { color: green; }\n").unwrap();

    let signals = if server_mode {
        concat!(
            "// @runsOn server\n",
            "import { Widget } from '../../components/Widget';\n",
            "type SignalsProps = {};\n",
            "export function Signals(props: SignalsProps) {\n",
            "  const items = await data(async () => [{ id: 1, name: 'Alpha' }, { id: 2, name: 'Beta' }]);\n",
            "  return (\n",
            "    <div class=\"signals-page\">\n",
            "      <h1>Signals API</h1>\n",
            "      <ul>\n",
            "        <For each={items} key={(x) => x.id}>\n",
            "          {(x) => <li>{x.name}</li>}\n",
            "        </For>\n",
            "      </ul>\n",
            "      <Widget label=\"Go\" client:hydrate />\n",
            "    </div>\n",
            "  );\n",
            "}\n",
        )
    } else {
        concat!(
            "// @runsOn server\n",
            "import { Widget } from '../../components/Widget';\n",
            "type SignalsProps = {};\n",
            "export function Signals(props: SignalsProps) {\n",
            "  return (\n",
            "    <div class=\"signals-page\">\n",
            "      <h1>Signals API</h1>\n",
            "      <Widget label=\"Go\" client:hydrate />\n",
            "    </div>\n",
            "  );\n",
            "}\n",
        )
    };
    std::fs::write(src.join("pages/Docs/Api/Signals.tsx"), signals).unwrap();
    std::fs::write(src.join("images/logo.png"), b"PNGDATA").unwrap();
}

fn build_fixture(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let out = dir.path().join("dist");
    let status = Command::new(cli_binary())
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    out
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

fn wait_for_port(port: u16) {
    use std::net::TcpStream;
    use std::time::{Duration, Instant};
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        if TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", port).parse().unwrap(),
            Duration::from_millis(200),
        )
        .is_ok()
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("port {} did not open within 10s", port);
}

fn http_get(port: u16, path: &str) -> String {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        path, port
    );
    stream.write_all(req.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

/// Bug #1 + #2 + #3: a route 2 levels deep must resolve to the REAL compiled
/// file (source-preserved casing), emit its transitive CSS, and reference
/// imports at the correct relative depth from the page's output location.
#[test]
fn nested_route_two_levels_resolves_files_css_and_imports() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    write_nested_fixture(&dir, false);
    let out = build_fixture(&dir);

    // (1) Correct file resolution: the real output path exists with
    // source-preserved casing, and the prerendered page is NOT the
    // "page not found" placeholder.
    assert!(
        out.join("pages/Docs/Api/Signals.mjs").exists(),
        "real mjs output should exist at pages/Docs/Api/Signals.mjs"
    );
    assert!(
        !out.join("pages/Docs/Api/signals.mjs").exists(),
        "no case-mangled mjs file should exist"
    );

    let html = std::fs::read_to_string(out.join("docs/api/signals.html")).unwrap();
    assert!(
        html.contains("Signals API") && !html.contains("page not found"),
        "prerendered HTML should contain real SSR content, got: {}",
        html
    );

    // (2) Correct CSS links present, at the right depth (3 segments → ../../../).
    assert!(
        html.contains("../../../pages/components/Widget.css"),
        "CSS link should be depth-aware, got: {}",
        html
    );
    assert!(
        out.join("pages/components/Widget.css").exists(),
        "transitive CSS should be copied to dist"
    );

    // (3) Correct relative imports at depth: import map + client module.
    assert!(
        html.contains("\"@marisjs/runtime\": \"../../../runtime.mjs\""),
        "import map should be depth-aware, got: {}",
        html
    );
    assert!(
        html.contains("import { Widget } from '../../../pages/components/Widget.mjs';"),
        "client import should be depth-aware, got: {}",
        html
    );
    assert!(
        out.join("pages/components/Widget.mjs").exists(),
        "hydrate island module should exist in dist"
    );

    // Island props are serialized into the placeholder at SSR time so the
    // client-side mount can rehydrate with the SAME props (not {}).
    assert!(
        html.contains("data-props='{\"label\":\"Go\"}'"),
        "island props should be serialized into the hydrate placeholder, got: {}",
        html
    );

    // The manifest carries the canonical route → real file mapping.
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("routes.json")).unwrap()).unwrap();
    let route = manifest["routes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["path"] == "/docs/api/signals")
        .expect("manifest should contain /docs/api/signals");
    assert_eq!(route["mjs"], "pages/Docs/Api/Signals.mjs");
    assert_eq!(route["file"], "docs/api/signals.html");
    assert_eq!(route["css"][0], "pages/components/Widget.css");
    assert_eq!(route["clientModules"][0]["path"], "pages/components/Widget.mjs");

    // Static assets from src/ are copied into dist (dev server + adapters).
    assert!(
        out.join("images/logo.png").exists(),
        "static assets should be copied into dist"
    );
}

/// Bug #4: adapter-node SSR must serve the nested route using the manifest's
/// real mjs path and emit depth-aware references, and unescape the SSR html
/// the same way the compiler's prerender path does.
#[test]
fn adapter_node_ssr_serves_nested_route_with_depth_aware_paths() {
    use std::process::{Command as ProcCommand, Stdio};

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    write_nested_fixture(&dir, true);
    let out = build_fixture(&dir);

    let port = free_port();
    let adapter = workspace_root().join("packages/adapter-node/server.mjs");
    let mut child = ProcCommand::new("node")
        .arg(&adapter)
        .arg(&out)
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn adapter-node");
    wait_for_port(port);

    let response = http_get(port, "/docs/api/signals");
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "nested route should 200, got: {}",
        response.lines().next().unwrap_or("")
    );

    // SSR content is unescaped (not literal &lt; entities).
    assert!(
        response.contains("<h1>Signals API</h1>") && response.contains("<li>Alpha</li>"),
        "SSR html should be unescaped with real data, got: {}",
        response
    );
    assert!(
        !response.contains("&lt;h1&gt;"),
        "SSR html must not be entity-escaped"
    );

    // Depth-aware references from the adapter shell.
    assert!(
        response.contains("\"@marisjs/runtime\": \"../../../runtime.mjs\""),
        "SSR import map should be depth-aware, got: {}",
        response
    );
    assert!(
        response.contains("../../../pages/components/Widget.css"),
        "SSR CSS link should be depth-aware, got: {}",
        response
    );
    assert!(
        response.contains("import { Widget } from '../../../pages/components/Widget.mjs';"),
        "SSR client import should be depth-aware, got: {}",
        response
    );
    assert!(
        response.contains("data-props='{\"label\":\"Go\"}'"),
        "SSR output should serialize island props, got: {}",
        response
    );
    assert!(
        response.contains("for (const el of document.querySelectorAll('[data-hydrate=\"Widget\"]')) { mount(el, () => Widget(el.dataset.props ? JSON.parse(el.dataset.props) : {})); }"),
        "SSR mount should read props back from data-props, got: {}",
        response
    );

    child.kill().unwrap();
    let _ = child.wait();
}

/// adapter-static folder-URL convention: /docs/api/signals must land at
/// docs/api/signals/index.html (URL depth preserved, so the compiler's
/// depth-aware relative paths resolve unchanged), with routes.json rewritten
/// to match.
#[test]
fn adapter_static_uses_folder_url_convention_for_nested_route() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);
    write_nested_fixture(&dir, false);
    let out = build_fixture(&dir);

    let public = dir.path().join("public");
    let status = Command::new("node")
        .arg(workspace_root().join("packages/adapter-static/cli.mjs"))
        .arg(&out)
        .arg(&public)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "adapter-static failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );

    // Folder-URL output at the same depth as the URL (/docs/api/signals).
    assert!(
        public.join("docs/api/signals/index.html").exists(),
        "nested route should be written to docs/api/signals/index.html"
    );
    assert!(
        !public.join("docs/api/signals.html").exists(),
        "flat html should not be copied when folder-URL convention applies"
    );
    assert!(public.join("index.html").exists(), "root stays index.html");

    // Relative asset paths stay correct at the folder-URL depth (3 segments).
    let html = std::fs::read_to_string(public.join("docs/api/signals/index.html")).unwrap();
    assert!(
        html.contains("\"@marisjs/runtime\": \"../../../runtime.mjs\""),
        "import map should survive folder-URL move, got: {}",
        html
    );
    assert!(
        html.contains("../../../pages/components/Widget.css")
            && html.contains("../../../pages/components/Widget.mjs"),
        "CSS and client module references should survive folder-URL move, got: {}",
        html
    );
    assert!(
        html.contains("data-props='{\"label\":\"Go\"}'"),
        "serialized island props should survive folder-URL move, got: {}",
        html
    );

    // routes.json rewritten to match the folder-URL layout.
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(public.join("routes.json")).unwrap()).unwrap();
    let route = manifest["routes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["path"] == "/docs/api/signals")
        .expect("manifest should contain /docs/api/signals");
    assert_eq!(route["file"], "docs/api/signals/index.html");
    assert_eq!(manifest["routes"][0]["file"], "index.html");
}

// ───────────────────────────────────────────────────────────────────────────
// Bug class: server/client codegen parity (found via live production testing)
// ───────────────────────────────────────────────────────────────────────────

/// Bug: the same client island used twice on one page emitted TWO `import`
/// statements for the same identifier — a SyntaxError that aborted the page's
/// module script. Regression: exactly one import per island component, and
/// BOTH instances mounted (each with its own SSR-serialized data-props).
#[test]
fn same_island_twice_emits_single_import_and_two_props() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let components_dir = dir.path().join("components");
    std::fs::create_dir(&components_dir).unwrap();
    std::fs::write(components_dir.join("Widget.tsx"), concat!(
        "// @runsOn client\n",
        "type WidgetProps = { label: string };\n",
        "export function Widget(props: WidgetProps) {\n",
        "  return <div class=\"widget\"><span class=\"widget-label\">{props.label}</span></div>;\n",
        "}\n",
    )).unwrap();

    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    std::fs::write(pages_dir.join("Index.tsx"), concat!(
        "// @runsOn server\n",
        "import { Widget } from '../components/Widget';\n",
        "type IndexProps = {};\n",
        "export function Index(props: IndexProps) {\n",
        "  return (\n",
        "    <div>\n",
        "      <Widget label=\"Alpha\" client:hydrate />\n",
        "      <Widget label=\"Beta\" client:hydrate />\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    )).unwrap();

    let out = build_fixture(&dir);
    let html = std::fs::read_to_string(out.join("index.html")).unwrap();

    // Exactly ONE import per island component (duplicates are a SyntaxError).
    let import_count = html.matches("import { Widget } from").count();
    assert_eq!(import_count, 1, "must emit exactly one import, got {}:\n{}", import_count, html);

    // Both instances present, each with its OWN serialized props.
    assert!(html.contains("data-props='{\"label\":\"Alpha\"}'"), "first instance props, got: {}", html);
    assert!(html.contains("data-props='{\"label\":\"Beta\"}'"), "second instance props, got: {}", html);
    // Two placeholder DIVS (the mount-loop selector string also contains
    // data-hydrate="Widget" but is not a placeholder element).
    assert_eq!(
        html.matches("data-hydrate=\"Widget\" data-props").count(),
        2,
        "two placeholders expected:\n{}",
        html
    );

    // Mounts target ALL instances via querySelectorAll.
    assert!(
        html.contains("for (const el of document.querySelectorAll('[data-hydrate=\"Widget\"]'))"),
        "mounts must iterate every instance, got:\n{}",
        html
    );

    // routes.json lists the client module once.
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("routes.json")).unwrap()).unwrap();
    let modules = manifest["routes"][0]["clientModules"].as_array().unwrap();
    assert_eq!(modules.len(), 1, "clientModules must be deduped, got: {:?}", modules);
    assert_eq!(modules[0]["name"], "Widget");
}

/// Bug: JSX attribute expressions in `@runsOn server` components were
/// stringified into the literal source text (`class="{expr}"`) instead of
/// evaluated — while client codegen evaluates them. Regression: the SSR html
/// must contain the EVALUATED values for class={expr}, href={expr}, per-item
/// expressions, and boolean-presence attributes.
#[test]
fn server_expression_attributes_evaluate_in_ssr_html() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    std::fs::write(pages_dir.join("Menu.tsx"), concat!(
        "// @runsOn server\n",
        "type MenuProps = {};\n",
        "export function Menu(props: MenuProps) {\n",
        "  const items = await data(async () => [{ id: 1, name: 'Coffee' }, { id: 2, name: 'Tea' }]);\n",
        "  return (\n",
        "    <ul class={items.length > 1 ? 'multi' : 'single'}>\n",
        "      <For each={items} key={(x) => x.id}>\n",
        "        {(x) => <li><a href={'/drinks/' + x.id} class={x.id === 1 ? 'hot' : 'cold'}>{x.name}</a></li>}\n",
        "      </For>\n",
        "      <input disabled={items.length === 0} />\n",
        "      <input disabled={items.length > 0} />\n",
        "      <><span>FragA</span><span>FragB</span></>\n",
        "      {items.length > 1 ? <p class=\"many\">Many</p> : <p>One</p>}\n",
        "    </ul>\n",
        "  );\n",
        "}\n",
    )).unwrap();

    let out = build_fixture(&dir);
    let html = std::fs::read_to_string(out.join("menu.html")).unwrap();

    // Expressions must be EVALUATED, not stringified as literal source text.
    assert!(html.contains("<ul class=\"multi\">"), "class expr evaluated, got: {}", html);
    assert!(html.contains("<a href=\"/drinks/1\" class=\"hot\">"), "href+class exprs per item 1, got: {}", html);
    assert!(html.contains("<a href=\"/drinks/2\" class=\"cold\">"), "href+class exprs per item 2, got: {}", html);

    // No literal expression source may leak into the html.
    assert!(!html.contains("{items.length"), "no stringified expr source, got: {}", html);
    assert!(!html.contains("${"), "no template-literal leak, got: {}", html);

    // Boolean attribute presence semantics: falsy → omitted, truthy → present.
    assert!(html.contains("<input>"), "falsy boolean expr omits the attribute, got: {}", html);
    assert!(html.contains("<input disabled=\"\">"), "truthy boolean expr emits the attribute, got: {}", html);

    // Fragments render their children inline on the server path too.
    assert!(html.contains("<span>FragA</span><span>FragB</span>"), "fragment children in SSR html, got: {}", html);

    // Conditional-in-children (ternary returning elements) on the server path.
    assert!(html.contains("<p class=\"many\">Many</p>"), "conditional element SSR html, got: {}", html);
    assert!(!html.contains("<p>One</p>"), "unselected branch must not render, got: {}", html);
}

/// Client path regression for SPEC §8 #2: a const declared at MODULE level
/// (outside the component function) must be captured by the parser and emitted
/// at module scope of the generated output, above the component function.
/// Verified by EXECUTING the generated module in jsdom — a ReferenceError
/// happens at import time if the emission is missing.
#[test]
fn client_component_references_module_level_const() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type CatalogProps = {};\n",
        "const products = [\n",
        "  { id: 'latte', name: 'Latte', price: 3.5 },\n",
        "  { id: 'espresso', name: 'Espresso', price: 2.0 },\n",
        "];\n",
        "const TAX_RATE: number = 0.2;\n",
        "export function Catalog(props: CatalogProps) {\n",
        "  return (\n",
        "    <ul class=\"catalog\">\n",
        "      <For each={products} key={(x) => x.id}>\n",
        "        {(x) => <li>{x.name} — {x.price}</li>}\n",
        "      </For>\n",
        "      <p class=\"tax\">Tax: {TAX_RATE}</p>\n",
        "    </ul>\n",
        "  );\n",
        "}\n",
    );
    let js = parse_validate_generate(&dir, "Catalog", fixture);

    // The const must be emitted at module scope, ABOVE the component function,
    // and with TS annotations stripped.
    let fn_pos = js.find("export function Catalog").unwrap();
    let products_pos = js.find("const products").expect("module const emitted");
    let tax_pos = js.find("const TAX_RATE = 0.2").expect("ts annotation stripped");
    assert!(products_pos < fn_pos, "const must precede the component fn:\n{}", js);
    assert!(tax_pos < fn_pos, "const must precede the component fn:\n{}", js);
    assert!(!js.contains("TAX_RATE: number"), "no TS annotation may leak:\n{}", js);

    let runner = r#"import { JSDOM } from 'jsdom';
import { Catalog } from './Catalog.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = Catalog({});
root.appendChild(result);

const lis = root.querySelectorAll('li');
const tax = root.querySelector('.tax');

const ok = lis.length === 2
    && lis[0].textContent.includes('Latte')
    && lis[0].textContent.includes('3.5')
    && lis[1].textContent.includes('Espresso')
    && tax !== null
    && tax.textContent.includes('0.2');

if (!ok) {
    console.error('FAIL', {
        liCount: lis.length,
        lis: Array.from(lis).map((l) => l.textContent),
        tax: tax ? tax.textContent : null,
    });
    process.exit(1);
}
console.log('PASS');
"#;

    run_node(&dir, runner);
}

/// Server path regression for SPEC §8 #2: the same module-level const pattern
/// on the SSR path. Verified by building through the real CLI and reading the
/// prerendered html.
#[test]
fn server_page_references_module_level_const() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    std::fs::write(pages_dir.join("Menu.tsx"), concat!(
        "// @runsOn server\n",
        "type MenuProps = {};\n",
        "const sections = ['Drinks', 'Desserts', 'Savory'];\n",
        "const BADGE: string = 'fresh';\n",
        "export function Menu(props: MenuProps) {\n",
        "  return (\n",
        "    <nav class=\"menu\">\n",
        "      <For each={sections} key={(x) => x}>\n",
        "        {(x) => <a class=\"sec\">{BADGE}: {x}</a>}\n",
        "      </For>\n",
        "    </nav>\n",
        "  );\n",
        "}\n",
    )).unwrap();

    let out = build_fixture(&dir);
    let html = std::fs::read_to_string(out.join("menu.html")).unwrap();

    // The fixture is a PURE server component: no imports, no hydrate islands,
    // no client components anywhere in its tree — the client codegen path is
    // provably never invoked for this file, so a server-only regression cannot
    // be "covered for" by the client implementation. Assert BOTH layers:
    // (1) the server-emitted module itself carries the consts at module scope
    // (direct server-path output — fails the instant server emission breaks),
    // (2) the prerendered HTML contains the evaluated values (fails if the
    // consts were emitted but not evaluated).
    let mjs = std::fs::read_to_string(out.join("pages/Menu.mjs")).unwrap();
    assert!(
        mjs.contains("const sections = ['Drinks', 'Desserts', 'Savory'];"),
        "server module must emit the sections const at module scope, got:\n{}",
        mjs
    );
    assert!(
        mjs.contains("const BADGE = 'fresh';"),
        "server module must emit BADGE with the TS annotation stripped, got:\n{}",
        mjs
    );

    // Both module consts must be evaluated during prerender — a missing emit
    // produces a ReferenceError and an empty/broken page instead.
    assert!(html.contains("<a class=\"sec\">fresh: Drinks</a>"), "section 1 rendered, got: {}", html);
    assert!(html.contains("<a class=\"sec\">fresh: Desserts</a>"), "section 2 rendered, got: {}", html);
    assert!(html.contains("<a class=\"sec\">fresh: Savory</a>"), "section 3 rendered, got: {}", html);
    assert!(!html.contains("sections"), "no raw identifier may leak, got: {}", html);
}

/// Follow-up hardening (independent verification pass, 2026-08-14): style
/// objects must not emit invalid CSS. (1) null/undefined property VALUES are
/// omitted entirely (previously emitted literally as "background: null;" —
/// invalid CSS that silently did nothing); (2) bare numbers on DIMENSIONAL
/// properties get an automatic px unit ("width: 100;" is invalid CSS and is
/// ignored by browsers), while React's unitless-exempt list (opacity, zIndex,
/// lineHeight, flexGrow, ...) never gets px.
#[test]
fn client_style_null_values_omitted_and_numeric_dimensional_gets_px() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type BadgeProps = {};\n",
        "export function Badge(props: BadgeProps) {\n",
        "  return (\n",
        "    <span class=\"badge\" style={{ width: 100, height: 50, fontSize: 16, background: null, color: undefined, opacity: 1, zIndex: 5, lineHeight: 1.5 }}>\n",
        "      hi\n",
        "    </span>\n",
        "  );\n",
        "}\n",
    );
    parse_validate_generate(&dir, "Badge", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { Badge } from './Badge.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
// Attach to the live document: jsdom's getComputedStyle does not refresh
// cached values for elements in DETACHED trees (probe-verified).
dom.window.document.body.appendChild(root);
const result = Badge({});
root.appendChild(result);

const el = root.querySelector('.badge');
if (!el) { console.error('FAIL: no badge element'); process.exit(1); }

const styleAttr = el.getAttribute('style');
// null/undefined properties omitted; dimensional numbers got px; unitless
// properties (opacity, zIndex, lineHeight) did NOT.
const expected = 'width: 100px; height: 50px; font-size: 16px; opacity: 1; z-index: 5; line-height: 1.5;';
if (styleAttr !== expected) {
    console.error('FAIL style attr:', JSON.stringify(styleAttr), 'want:', JSON.stringify(expected));
    process.exit(1);
}
if (styleAttr.includes('null') || styleAttr.includes('undefined')) {
    console.error('FAIL: null/undefined leaked into style:', styleAttr);
    process.exit(1);
}

// The px forms must be VALID CSS — otherwise computed styles come back empty
// (bare "width: 100;" is ignored by browsers).
const cs = dom.window.getComputedStyle(el);
if (cs.width !== '100px') { console.error('FAIL computed width:', cs.width); process.exit(1); }
if (cs.fontSize !== '16px') { console.error('FAIL computed font-size:', cs.fontSize); process.exit(1); }
if (cs.opacity !== '1') { console.error('FAIL computed opacity:', cs.opacity); process.exit(1); }
if (cs.lineHeight !== '1.5') { console.error('FAIL computed line-height:', cs.lineHeight); process.exit(1); }

console.log('PASS');
"#;

    run_node(&dir, runner);
}

/// Server-path half of the style-serializer hardening: the prerendered HTML
/// must also get px for bare dimensional numbers and omit null/undefined
/// properties — never "width: 120;" or "background: null;".
#[test]
fn server_style_numeric_px_and_null_omitted_in_prerendered_html() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    std::fs::write(pages_dir.join("Menu.tsx"), concat!(
        "// @runsOn server\n",
        "type MenuProps = {};\n",
        "export function Menu(props: MenuProps) {\n",
        "  return (\n",
        "    <nav class=\"menu\" style={{ width: 120, background: null, color: undefined, opacity: 1 }}>\n",
        "      Menu\n",
        "    </nav>\n",
        "  );\n",
        "}\n",
    )).unwrap();

    let out = build_fixture(&dir);
    let html = std::fs::read_to_string(out.join("menu.html")).unwrap();

    assert!(
        html.contains("style=\"width: 120px; opacity: 1;\""),
        "px appended and null/undefined omitted in html, got: {}",
        html
    );
    assert!(
        !html.contains("null") && !html.contains("undefined"),
        "no null/undefined may leak into html, got: {}",
        html
    );
}

/// Client path regression for SPEC §8 #6: a STATIC `style={{ ... }}` object
/// must serialize to a proper CSS string (camelCase → kebab-case), never
/// `[object Object]`. Verified against the DOM's actual computed style.
#[test]
fn client_static_style_object_serializes_to_css_string() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type BadgeProps = {};\n",
        "export function Badge(props: BadgeProps) {\n",
        "  return (\n",
        "    <span class=\"badge\" style={{ backgroundColor: 'rgb(10, 20, 30)', padding: '4px 8px', fontSize: 14 }}>\n",
        "      hi\n",
        "    </span>\n",
        "  );\n",
        "}\n",
    );
    parse_validate_generate(&dir, "Badge", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { Badge } from './Badge.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = Badge({});
root.appendChild(result);

const el = root.querySelector('.badge');
if (!el) { console.error('FAIL: no badge element'); process.exit(1); }

const styleAttr = el.getAttribute('style');
    // fontSize: 14 is a bare dimensional number → px appended (follow-up
    // hardening: "font-size: 14;" is invalid CSS and silently ignored).
    const expected = 'background-color: rgb(10, 20, 30); padding: 4px 8px; font-size: 14px;';
if (styleAttr !== expected) {
    console.error('FAIL style attr:', JSON.stringify(styleAttr));
    process.exit(1);
}
if (styleAttr.includes('[object Object]')) {
    console.error('FAIL: serialized as [object Object]');
    process.exit(1);
}

const computed = dom.window.getComputedStyle(el);
if (computed.backgroundColor !== 'rgb(10, 20, 30)') {
    console.error('FAIL computed backgroundColor:', computed.backgroundColor);
    process.exit(1);
}

console.log('PASS');
"#;

    run_node(&dir, runner);
}

/// Client path regression for SPEC §8 #6: a REACTIVE style object (one value
/// read from a signal) must be wrapped in bind() and update live after the
/// signal changes — confirmed via the DOM's computed style, same rigor as the
/// other reactive-attribute tests.
#[test]
fn client_reactive_style_object_updates_computed_style() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type PanelProps = {};\n",
        "export function Panel(props: PanelProps) {\n",
        "  const wide = signal(false);\n",
        "  const color = signal('blue');\n",
        "  return (\n",
        "    <div class=\"panel\" style={{ width: wide.value ? '200px' : '100px', backgroundColor: color.value }}>\n",
        "      p\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    parse_validate_generate(&dir, "Panel", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { Panel } from './Panel.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
// Attach to the live document: jsdom's getComputedStyle does not refresh
// cached values for elements in DETACHED trees (probe-verified).
dom.window.document.body.appendChild(root);
const result = Panel({});
root.appendChild(result);

const el = root.querySelector('.panel');
if (!el) { console.error('FAIL: no panel element'); process.exit(1); }

const initial = el.getAttribute('style');
if (initial !== 'width: 100px; background-color: blue;') {
    console.error('FAIL initial style attr:', JSON.stringify(initial));
    process.exit(1);
}
if (dom.window.getComputedStyle(el).backgroundColor !== 'rgb(0, 0, 255)') {
    console.error('FAIL initial computed backgroundColor:', dom.window.getComputedStyle(el).backgroundColor);
    process.exit(1);
}

result._signals.wide.set(true);
await new Promise(r => setTimeout(r, 0));
if (el.getAttribute('style') !== 'width: 200px; background-color: blue;') {
    console.error('FAIL style attr after wide.set(true):', JSON.stringify(el.getAttribute('style')));
    process.exit(1);
}
if (dom.window.getComputedStyle(el).width !== '200px') {
    console.error('FAIL computed width after wide.set(true):', dom.window.getComputedStyle(el).width);
    process.exit(1);
}

result._signals.color.set('red');
await new Promise(r => setTimeout(r, 0));
if (el.getAttribute('style') !== 'width: 200px; background-color: red;') {
    console.error('FAIL style attr after color.set(red):', JSON.stringify(el.getAttribute('style')));
    process.exit(1);
}
if (dom.window.getComputedStyle(el).backgroundColor !== 'rgb(255, 0, 0)') {
    console.error('FAIL computed backgroundColor after color.set(red):', dom.window.getComputedStyle(el).backgroundColor);
    process.exit(1);
}

console.log('PASS');
"#;

    run_node(&dir, runner);
}

/// Client path regression for SPEC §8 #6 with the computed() form: the style
/// ATTRIBUTE expression evaluates to an object via a computed signal. Must be
/// reactive (bind-wrapped) and update after the dependency changes.
#[test]
fn client_computed_style_object_updates_computed_style() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type BoxProps = {};\n",
        "export function Box(props: BoxProps) {\n",
        "  const accent = signal('green');\n",
        "  const boxStyle = computed(() => ({ backgroundColor: accent.value, borderRadius: accent.value === 'green' ? '8px' : '0px' }));\n",
        "  return <div class=\"box\" style={boxStyle.value}>x</div>;\n",
        "}\n",
    );
    parse_validate_generate(&dir, "Box", fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { Box } from './Box.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
// Attach to the live document: jsdom's getComputedStyle does not refresh
// cached values for elements in DETACHED trees (probe-verified).
dom.window.document.body.appendChild(root);
const result = Box({});
root.appendChild(result);

const el = root.querySelector('.box');
if (!el) { console.error('FAIL: no box element'); process.exit(1); }

const initial = el.getAttribute('style');
if (initial !== 'background-color: green; border-radius: 8px;') {
    console.error('FAIL initial style attr:', JSON.stringify(initial));
    process.exit(1);
}
if (dom.window.getComputedStyle(el).backgroundColor !== 'rgb(0, 128, 0)') {
    console.error('FAIL initial computed backgroundColor:', dom.window.getComputedStyle(el).backgroundColor);
    process.exit(1);
}

result._signals.accent.set('red');
await new Promise(r => setTimeout(r, 0));
if (el.getAttribute('style') !== 'background-color: red; border-radius: 0px;') {
    console.error('FAIL style attr after accent.set(red):', JSON.stringify(el.getAttribute('style')));
    process.exit(1);
}
if (dom.window.getComputedStyle(el).backgroundColor !== 'rgb(255, 0, 0)') {
    console.error('FAIL computed backgroundColor after accent.set(red):', dom.window.getComputedStyle(el).backgroundColor);
    process.exit(1);
}

console.log('PASS');
"#;

    run_node(&dir, runner);
}

/// Server path regression for SPEC §8 #6 (parity — the project's history says
/// never assume one path implies the other): the prerendered html must contain
/// the style object serialized as a CSS string, evaluated at render time.
#[test]
fn server_style_object_serializes_in_prerendered_html() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    std::fs::write(pages_dir.join("Menu.tsx"), concat!(
        "// @runsOn server\n",
        "type MenuProps = {};\n",
        "const THEME = { backgroundColor: 'navy' };\n",
        "export function Menu(props: MenuProps) {\n",
        "  return (\n",
        "    <nav class=\"menu\" style={{ backgroundColor: 'navy', color: THEME.backgroundColor === 'navy' ? 'white' : 'black' }}>\n",
        "      Menu\n",
        "    </nav>\n",
        "  );\n",
        "}\n",
    )).unwrap();

    let out = build_fixture(&dir);
    let html = std::fs::read_to_string(out.join("menu.html")).unwrap();

    assert!(
        html.contains("style=\"background-color: navy; color: white;\""),
        "style object serialized in html, got: {}",
        html
    );
    assert!(
        !html.contains("[object Object]"),
        "no [object Object] may leak into html, got: {}",
        html
    );
}

/// Follow-up hardening (independent verification pass, 2026-08-14): drilled
/// signal props must be reactive at ANY member-access depth, not just one
/// level. `props.one.value`, `props.two.inner.value`,
/// `props.three.nested.count.value`, and `props.four.a.b.c.d.value` (1, 2, 3,
/// and 4 levels — the 4th proves there is no fixed ceiling) must ALL be
/// bind()-wrapped and update live when their parent signal changes. Pre-fix,
/// chain_reads_signal only recognized a DIRECT props base, so every depth ≥ 2
/// silently rendered once and never updated again.
#[test]
fn props_drilled_signal_value_reactive_at_any_depth() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let child_fixture = concat!(
        "// @runsOn client\n",
        "type DeepProps = {\n",
        "  one: string;\n",
        "  two: { inner: string; };\n",
        "  three: { nested: { count: string; }; };\n",
        "  four: { a: { b: { c: { d: string; }; }; }; };\n",
        "};\n",
        "export function Deep(props: DeepProps) {\n",
        "  return (\n",
        "    <div>\n",
        "      <span class=\"d1\">{props.one.value}</span>\n",
        "      <span class=\"d2\">{props.two.inner.value}</span>\n",
        "      <span class=\"d3\">{props.three.nested.count.value}</span>\n",
        "      <span class=\"d4\">{props.four.a.b.c.d.value}</span>\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    let js = parse_validate_generate(&dir, "Deep", child_fixture);

    // Every depth must be bind-wrapped — one per reactive text node, and the
    // 4th proves the walk generalizes past any fixed ceiling.
    let bind_count = js.matches("bind(").count();
    assert!(
        bind_count >= 4,
        "all four depths must be reactive ({} bind() calls), got:\n{}",
        bind_count, js
    );

    let parent_fixture = concat!(
        "// @runsOn client\n",
        "import { Deep } from './Deep';\n",
        "type AppProps = {};\n",
        "export function App(props: AppProps) {\n",
        "  const one = signal('one');\n",
        "  const two = signal('two');\n",
        "  const three = signal('three');\n",
        "  const four = signal('four');\n",
        "  return <Deep one={one} two={{ inner: two }} three={{ nested: { count: three } }} four={{ a: { b: { c: { d: four } } } }} />;\n",
        "}\n",
    );
    parse_validate_generate(&dir, "App", parent_fixture);

    let runner = r#"import { JSDOM } from 'jsdom';
import { App } from './App.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = App({});
root.appendChild(result);

const read = () => [
    root.querySelector('.d1').textContent,
    root.querySelector('.d2').textContent,
    root.querySelector('.d3').textContent,
    root.querySelector('.d4').textContent,
].join(',');

if (read() !== 'one,two,three,four') {
    console.error('FAIL initial: ' + read());
    process.exit(1);
}

// Mutate each signal, one depth at a time — every level must update live.
result._signals.one.set('ONE');
await new Promise(r => setTimeout(r, 0));
result._signals.two.set('TWO');
await new Promise(r => setTimeout(r, 0));
result._signals.three.set('THREE');
await new Promise(r => setTimeout(r, 0));
result._signals.four.set('FOUR');
await new Promise(r => setTimeout(r, 0));

const now = read();
if (now === 'ONE,TWO,THREE,FOUR') {
    console.log('PASS');
} else {
    console.error('FAIL after sets: ' + now);
    process.exit(1);
}
"#;

    run_node(&dir, runner);
}

/// Regression for AST-based reactivity detection (SPEC §8 round 3): a plain
/// non-signal object with an unrelated `.value` field — and a `.value` inside
/// a string literal — must NOT be treated as reactive. No bind() wrapper may
/// be emitted (substring matching on ".value" wrongly flagged both).
#[test]
fn plain_object_value_field_is_not_reactive() {
    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let fixture = concat!(
        "// @runsOn client\n",
        "type StatusProps = {};\n",
        "const CONFIG = { value: 'dark', mode: 'night' };\n",
        "export function Status(props: StatusProps) {\n",
        "  return (\n",
        "    <div>\n",
        "      <span class=\"theme\">{CONFIG.value}</span>\n",
        "      <span class=\"str\">{'a.value.b literal'}</span>\n",
        "      <span class=\"attr\" data-v={CONFIG.value}>x</span>\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    );
    let js = parse_validate_generate(&dir, "Status", fixture);

    // The generated module must NOT contain any bind() call: none of these
    // expressions reads a known signal/computed — CONFIG is a plain object.
    assert!(
        !js.contains("bind("),
        "no bind wrapper for non-signal .value reads, got:\n{}",
        js
    );

    let runner = r#"import { JSDOM } from 'jsdom';
import { Status } from './Status.mjs';

const dom = new JSDOM('<!DOCTYPE html><html><body></body></html>');
global.document = dom.window.document;
global.Node = dom.window.Node;
const root = dom.window.document.createElement('div');
const result = Status({});
root.appendChild(result);

const theme = root.querySelector('.theme');
const str = root.querySelector('.str');
const attr = root.querySelector('.attr');

const ok = theme !== null && theme.textContent === 'dark'
    && str !== null && str.textContent === 'a.value.b literal'
    && attr !== null && attr.getAttribute('data-v') === 'dark';

if (!ok) {
    console.error('FAIL', {
        theme: theme ? theme.textContent : null,
        str: str ? str.textContent : null,
        dataV: attr ? attr.getAttribute('data-v') : null,
    });
    process.exit(1);
}
console.log('PASS');
"#;

    run_node(&dir, runner);
}

/// Real-browser half of the duplicate-island regression: the page module must
/// execute WITHOUT a SyntaxError (duplicate import) and both islands must
/// mount with their own props. Skips gracefully when Playwright/Chromium are
/// unavailable, like hydrate_islands_verified_by_playwright_dom_positions.
#[test]
fn duplicate_island_mounts_both_instances_in_browser() {
    let playwright_available = std::process::Command::new("npx")
        .arg("playwright")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    let home = std::env::var("HOME").unwrap_or_default();
    let pw_cache = std::path::PathBuf::from(home).join(".cache/ms-playwright");
    let browser_found = pw_cache.exists()
        && std::fs::read_dir(&pw_cache)
            .map(|entries| {
                entries.filter_map(|e| e.ok())
                    .any(|e| e.file_name().to_str().map_or(false, |n| n.starts_with("chromium")))
            })
            .unwrap_or(false);

    if !playwright_available || !browser_found {
        eprintln!("SKIP: playwright or Chromium browser not installed — skipping real-browser duplicate-island test");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    setup_test_dir(&dir);

    let components_dir = dir.path().join("components");
    std::fs::create_dir(&components_dir).unwrap();
    std::fs::write(components_dir.join("Widget.tsx"), concat!(
        "// @runsOn client\n",
        "type WidgetProps = { label: string };\n",
        "export function Widget(props: WidgetProps) {\n",
        "  return <div class=\"widget\"><span class=\"widget-label\">{props.label}</span></div>;\n",
        "}\n",
    )).unwrap();

    let pages_dir = dir.path().join("pages");
    std::fs::create_dir(&pages_dir).unwrap();
    std::fs::write(pages_dir.join("Index.tsx"), concat!(
        "// @runsOn server\n",
        "import { Widget } from '../components/Widget';\n",
        "type IndexProps = {};\n",
        "export function Index(props: IndexProps) {\n",
        "  return (\n",
        "    <div>\n",
        "      <Widget label=\"Alpha\" client:hydrate />\n",
        "      <Widget label=\"Beta\" client:hydrate />\n",
        "    </div>\n",
        "  );\n",
        "}\n",
    )).unwrap();

    let bin = cli_binary();
    let out_dir = dir.path().join("dist");
    let status = std::process::Command::new(&bin)
        .arg("build")
        .arg(dir.path())
        .arg("--out")
        .arg(&out_dir)
        .output()
        .unwrap();
    assert!(status.status.success(), "build failed: {}", String::from_utf8_lossy(&status.stderr));

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let mut server = std::process::Command::new(&bin)
        .arg("dev")
        .arg(dir.path())
        .arg("--out")
        .arg(&out_dir)
        .arg("--port")
        .arg(port.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();

    let start = std::time::Instant::now();
    let mut ready = false;
    while start.elapsed() < std::time::Duration::from_secs(30) {
        if std::net::TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", port).parse().unwrap(),
            std::time::Duration::from_millis(200),
        ).is_ok() {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert!(ready, "dev server did not start on port {}", port);

    let spec = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/duplicate-island.spec.mjs");
    let status = std::process::Command::new("npx")
        .arg("playwright")
        .arg("test")
        .arg(spec)
        .arg("--reporter=line")
        .env("MARISJS_DEV_URL", format!("http://127.0.0.1:{}", port))
        .status()
        .unwrap_or_else(|e| {
            server.kill().unwrap();
            let _ = server.wait();
            panic!("failed to run playwright: {}", e);
        });

    server.kill().unwrap();
    let _ = server.wait();

    assert!(
        status.success(),
        "playwright duplicate-island spec failed (exit {:?})",
        status.code()
    );
}
