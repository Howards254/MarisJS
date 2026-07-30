//! Thin binary. Wires parser → validator → codegen.
//! Exposes validate, build, and dev subcommands.

use clap::{Parser, Subcommand};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command as ProcCommand;
use std::sync::mpsc;
use std::time::SystemTime;

const RUNTIME_JS: &str = include_str!("../../../packages/runtime/src/index.js");

#[derive(Parser)]
#[command(name = "marisjs", about = "Framework compiler and validation tool", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Validate { file: String },
    Build {
        source: String,
        #[arg(long, default_value = "dist")]
        out: String,
    },
    Dev {
        source: String,
        #[arg(long, default_value = "dist")]
        out: String,
        #[arg(long, default_value = "3000")]
        port: u16,
    },
}

#[derive(Serialize)]
struct ErrorOutput<'a> {
    line: Option<usize>,
    column: Option<usize>,
    code: &'a str,
    message: &'a str,
    fix_hint: &'a str,
}

#[derive(Serialize)]
struct ValidateOutput<'a> {
    valid: bool,
    errors: Vec<ErrorOutput<'a>>,
}

// ── build-time metadata ───────────────────────────────────────────────

struct ComponentMeta {
    render_tree: Option<parser::JsxNode>,
    imports: Vec<parser::ImportInfo>,
    css_imports: Vec<String>,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Validate { file } => {
            match parser::parse_component_file(&file) {
                Ok(component) => {
                    let diagnostics = validator::validate(&component);
                    let valid = diagnostics.is_empty();
                    let errors: Vec<ErrorOutput> = diagnostics.iter().map(|d| ErrorOutput {
                        line: d.line, column: d.column, code: d.code,
                        message: &d.message, fix_hint: d.fix_hint,
                    }).collect();
                    println!("{}", serde_json::to_string_pretty(&ValidateOutput { valid, errors }).unwrap());
                    if !valid { std::process::exit(1); }
                }
                Err(e) => { eprintln!("Parse error: {}", e); std::process::exit(2); }
            }
        }
        Command::Build { source, out } => {
            match build_all(&source, &out) {
                Ok(files) => eprintln!("Built {} file(s) to {}", files, out),
                Err(e) => { eprintln!("Build error: {}", e); std::process::exit(1); }
            }
        }
        Command::Dev { source, out, port } => {
            if let Err(e) = run_dev(&source, &out, port) {
                eprintln!("dev error: {}", e); std::process::exit(1);
            }
        }
    }
}

// ── build ──────────────────────────────────────────────────────────────

fn build_all(source: &str, out: &str) -> Result<usize, String> {
    let source_dir = Path::new(source);
    if !source_dir.is_dir() { return Err(format!("'{}' is not a directory", source)); }
    if !source_dir.exists() { return Err(format!("directory '{}' does not exist", source)); }

    let mut count = 0usize;
    let out_dir = Path::new(out);
    let _ = std::fs::remove_dir_all(out_dir);
    std::fs::create_dir_all(out_dir).map_err(|e| format!("failed to create out dir: {}", e))?;

    let mut page_routes: Vec<(String, String, Vec<(String, String)>, Vec<String>, bool)> = Vec::new();
    let mut component_meta: HashMap<PathBuf, ComponentMeta> = HashMap::new();

    walk_and_build(source_dir, source_dir, out_dir, &mut count, &mut page_routes, &mut component_meta)?;

    // Write runtime (embedded at compile time)
    let runtime_dest = out_dir.join("runtime.mjs");
    std::fs::write(&runtime_dest, RUNTIME_JS).map_err(|e| format!("failed to write runtime: {}", e))?;
    eprintln!("  wrote runtime → runtime.mjs");

    // Create node_modules shim so Node can resolve @maris/runtime during prerendering.
    // The dist/ directory is self-contained: runtime.mjs sits at the root, and the shim
    // tells Node's module resolver to look there. Browser code uses the import map
    // injected into generated HTML instead.
    let nm_runtime = out_dir.join("node_modules/@maris/runtime");
    std::fs::create_dir_all(&nm_runtime).map_err(|e| format!("create node_modules: {}", e))?;
    std::fs::write(nm_runtime.join("package.json"), r#"{"name":"@maris/runtime","type":"module","main":"../../../runtime.mjs"}"#)
        .map_err(|e| format!("write runtime package.json: {}", e))?;

    // Collect transitive CSS for each page
    let page_routes = resolve_page_css(page_routes, &component_meta, source_dir, out_dir)?;

    // Pre-render pages to static HTML
    prerender_pages(out_dir, &page_routes)?;

    // Generate routes.json manifest
    generate_routes_json(out_dir, &page_routes)?;
    eprintln!("  generated routes.json");

    write_reload_timestamp(out_dir);
    Ok(count)
}

fn write_reload_timestamp(out_dir: &Path) {
    let ts = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis();
    let _ = std::fs::write(out_dir.join("__build_timestamp.txt"), ts.to_string());
}

fn walk_and_build(
    base: &Path, current: &Path, out_base: &Path, count: &mut usize,
    page_routes: &mut Vec<(String, String, Vec<(String, String)>, Vec<String>, bool)>,
    component_meta: &mut HashMap<PathBuf, ComponentMeta>,
) -> Result<(), String> {
    for entry in std::fs::read_dir(current).map_err(|e| format!("read_dir: {}", e))? {
        let entry = entry.map_err(|e| format!("entry: {}", e))?;
        let path = entry.path();
        if path.is_dir() {
            walk_and_build(base, &path, out_base, count, page_routes, component_meta)?;
        } else if path.extension().map_or(false, |ext| ext == "tsx") {
            let rel = path.strip_prefix(base).map_err(|e| format!("strip_prefix: {}", e))?;
            let out_path = out_base.join(rel).with_extension("mjs");
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("create_dir_all: {}", e))?;
            }
            eprintln!("  building {}", rel.display());

            let file_path = path.to_str().ok_or("invalid path")?;
            let component = parser::parse_component_file(file_path)
                .map_err(|e| format!("parse error in {}: {}", rel.display(), e))?;
            let diagnostics = validator::validate(&component);
            if !diagnostics.is_empty() {
                for d in &diagnostics {
                    eprintln!("    {}:{} — {}: {} (fix: {})", d.line.unwrap_or(0), d.column.unwrap_or(0), d.code, d.message, d.fix_hint);
                }
                return Err(format!("validation failed in {}: {} error(s)", rel.display(), diagnostics.len()));
            }
            let js = codegen::generate(&component)
                .map_err(|e| format!("codegen error in {}: {}", rel.display(), e))?;

            // Store component metadata for transitive CSS collection
            let css_paths: Vec<String> = component.imports.iter()
                .filter(|i| i.is_css)
                .map(|i| i.source.clone())
                .collect();
            component_meta.insert(rel.to_path_buf(), ComponentMeta {
                render_tree: component.render_tree.clone(),
                imports: component.imports.clone(),
                css_imports: css_paths,
            });

            // Is this a page file? (under pages/ directory)
            let is_page = rel.starts_with("pages") || rel.starts_with("pages/");
            if is_page && component.runs_on == Some(parser::RunsOn::Server) {
                let route = page_file_to_route(rel);
                let html_file = route_to_html_file(&route);

                // Collect hydrate roots for THIS page specifically
                let mut page_roots: Vec<(String, String)> = Vec::new();
                if let Some(ref tree) = component.render_tree {
                    for root_name in codegen::collect_hydrate_roots(tree) {
                        let import_path = resolve_client_import(rel, &root_name, &component.imports)?;
                        page_roots.push((root_name, import_path));
                    }
                }
                page_routes.push((route, html_file, page_roots, Vec::new(), component.has_data_call));
            }

            std::fs::write(&out_path, js)
                .map_err(|e| format!("write error for {}: {}", out_path.display(), e))?;
            *count += 1;
        }
    }
    Ok(())
}

// ── CSS collection and copying ────────────────────────────────────────

/// Resolves the transitive CSS for each page by walking the component dependency tree.
/// Copies CSS files into the output directory preserving relative structure.
fn resolve_page_css(
    page_routes: Vec<(String, String, Vec<(String, String)>, Vec<String>, bool)>,
    component_meta: &HashMap<PathBuf, ComponentMeta>,
    source_dir: &Path,
    out_dir: &Path,
) -> Result<Vec<(String, String, Vec<(String, String)>, Vec<String>, bool)>, String> {
    let mut result = Vec::new();

    for (route, html_file, roots, _, has_data) in page_routes {
        // Determine which file is the page: convert route back to file path
        let page_rel = route_to_page_file(&route);
        let page_path = PathBuf::from(&page_rel);

        let mut css_files: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        collect_css_recursive(&page_path, component_meta, source_dir, &mut css_files, &mut seen);

        // Copy each CSS file into the output directory
        for css_path in &css_files {
            let css_rel = PathBuf::from(css_path);
            let dest = out_dir.join(&css_rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("create_dir_all: {}", e))?;
            }
            let src = source_dir.join(&css_rel);
            if src.exists() {
                std::fs::copy(&src, &dest)
                    .map_err(|e| format!("copy css {}: {}", css_path, e))?;
                eprintln!("  copied css {}", css_path);
            }
        }

        result.push((route, html_file, roots, css_files, has_data));
    }

    Ok(result)
}

fn collect_css_recursive(
    comp_rel: &Path,
    component_meta: &HashMap<PathBuf, ComponentMeta>,
    source_dir: &Path,
    css_files: &mut Vec<String>,
    seen: &mut HashSet<PathBuf>,
) {
    if !seen.insert(comp_rel.to_path_buf()) {
        return; // already visited
    }

    if let Some(meta) = component_meta.get(comp_rel) {
        for css_import in &meta.css_imports {
            // css_import is like "./Cart.css" — resolve relative to the component's directory
            let comp_dir = comp_rel.parent().unwrap_or(Path::new("."));
            let resolved = comp_dir.join(css_import);
            // Normalize: strip leading ./ or ../
            let css_path = normalize_path(&resolved);
            if !css_files.contains(&css_path) {
                css_files.push(css_path);
            }
        }

        // Recurse into child components
        if let Some(ref tree) = meta.render_tree {
            let child_tags = codegen::collect_child_component_tags(tree);
            for tag in child_tags {
                if let Some(child_rel) = resolve_component_import_path(&tag, &meta.imports, comp_rel) {
                    collect_css_recursive(&child_rel, component_meta, source_dir, css_files, seen);
                }
            }
        }
    }
}

fn resolve_component_import_path(
    tag: &str,
    imports: &[parser::ImportInfo],
    current_rel: &Path,
) -> Option<PathBuf> {
    for imp in imports {
        if imp.is_css { continue; }
        if imp.imported_names.contains(&tag.to_string()) {
            let import_src = imp.source.clone();
            let comp_dir = current_rel.parent().unwrap_or(Path::new("."));
            let resolved = comp_dir.join(
                import_src.trim_end_matches(".tsx").trim_end_matches(".ts")
            ).with_extension("tsx");
            return Some(normalize_pathbuf(&resolved));
        }
    }
    None
}

fn normalize_path(p: &Path) -> String {
    let mut components = Vec::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => { components.pop(); }
            std::path::Component::CurDir => {}
            std::path::Component::Normal(s) => components.push(s.to_str().unwrap_or("")),
            _ => {}
        }
    }
    components.join("/")
}

fn normalize_pathbuf(p: &Path) -> PathBuf {
    PathBuf::from(normalize_path(p))
}

fn route_to_page_file(route: &str) -> String {
    if route == "/" {
        "pages/Index.tsx".to_string()
    } else {
        let name = route.trim_start_matches('/');
        format!("pages/{}.tsx", capitalize_first(name))
    }
}

fn page_file_to_route(rel: &Path) -> String {
    let s = rel.with_extension("").to_str().unwrap_or("").to_string();
    let path = s.strip_prefix("pages/").or_else(|| s.strip_prefix("pages")).unwrap_or(&s);
    let path = path.to_lowercase(); // lowercase for URL convention
    if path.is_empty() || path == "index" || path.ends_with("/index") {
        let parent = Path::new(&path).parent().and_then(|p| p.to_str()).unwrap_or("");
        if parent.is_empty() { return "/".to_string(); }
        return format!("/{}", parent);
    }
    format!("/{}", path)
}

fn route_to_html_file(route: &str) -> String {
    if route == "/" { "index.html".to_string() }
    else { format!("{}.html", route.trim_start_matches('/')) }
}

fn resolve_client_import(server_rel: &Path, root_name: &str, imports: &[parser::ImportInfo]) -> Result<String, String> {
    let source = imports.iter()
        .find(|i| i.imported_names.contains(&root_name.to_string()))
        .map(|i| i.source.clone())
        .ok_or_else(|| format!("client root '{}' referenced but no import found", root_name))?;
    let import_src = format!("{}.mjs", source.trim_end_matches(".tsx").trim_end_matches(".ts"));
    let server_parent = server_rel.with_extension("mjs").parent().unwrap_or(Path::new(".")).to_path_buf();
    let resolved = server_parent.join(&import_src);
    let import_path = if resolved.starts_with(".") { resolved.to_str().unwrap_or("").to_string() } else { format!("./{}", resolved.display()) };
    Ok(import_path)
}

// ── pre-render pages ──────────────────────────────────────────────────

fn prerender_pages(out_dir: &Path, page_routes: &[(String, String, Vec<(String, String)>, Vec<String>, bool)]) -> Result<(), String> {
    for (route, html_file, page_roots, css_files, _has_data) in page_routes {
        let html_path = out_dir.join(html_file);
        if let Some(parent) = html_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create_dir_all: {}", e))?;
        }
        let html = generate_page_html(out_dir, route, page_roots, css_files)?
            .unwrap_or_else(|| format!("<h1>{} — page not found</h1>", route));
        std::fs::write(&html_path, html).map_err(|e| format!("write {}: {}", html_file, e))?;
        eprintln!("  prerendered {} → {}", route, html_file);
    }
    Ok(())
}

fn generate_page_html(out_dir: &Path, route: &str, client_roots: &[(String, String)], css_files: &[String]) -> Result<Option<String>, String> {
    // Find the .mjs file for this route and execute it via Node to get server HTML
    let mjs_path = route_to_mjs_path(out_dir, route);
    let server_html = if mjs_path.exists() {
        Some(prerender_with_node(out_dir, &mjs_path)?)
    } else {
        None
    };

    let Some(html_content) = server_html else { return Ok(None); };

    let mut html = String::new();
    html.push_str("<!DOCTYPE html>\n<html>\n<head>\n");
    html.push_str("  <script type=\"importmap\">\n  {\n    \"imports\": {\n      \"@maris/runtime\": \"./runtime.mjs\"\n    }\n  }\n  </script>\n");
    for css_file in css_files {
        html.push_str(&format!("  <link rel=\"stylesheet\" href=\"{}\">\n", css_file));
    }
    html.push_str("</head>\n<body>\n");
    html.push_str("  <div id=\"root\">\n");
    html.push_str(&format!("  {}\n", unescape_html(&html_content)));
    html.push_str("  </div>\n");
    html.push_str("  <script type=\"module\">\n");
    if !client_roots.is_empty() {
        html.push_str("    import { mount } from '@maris/runtime';\n");
        for (name, path) in client_roots {
            html.push_str(&format!("    import {{ {} }} from '{}';\n", name, path));
        }
        html.push_str("    const root = document.getElementById('root');\n");
        for (name, _path) in client_roots {
            html.push_str(&format!("    mount(root, () => {}({{}}));\n", name));
        }
    }
    html.push_str("  </script>\n</body>\n</html>\n");
    Ok(Some(html))
}

fn route_to_mjs_path(out_dir: &Path, route: &str) -> std::path::PathBuf {
    if route == "/" {
        out_dir.join("pages/Index.mjs")
    } else {
        let name = route.trim_start_matches('/');
        out_dir.join("pages").join(format!("{}.mjs", capitalize_first(name)))
    }
}

fn unescape_html(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
}

fn capitalize_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

fn prerender_with_node(_out_dir: &Path, mjs_path: &Path) -> Result<String, String> {
    let abs = std::fs::canonicalize(mjs_path).map_err(|e| format!("canonicalize: {}", e))?;
    let abs_str = abs.to_str().ok_or("invalid path")?;

    // Extract component name from filename
    let stem = mjs_path.file_stem().and_then(|s| s.to_str()).unwrap_or("Index");
    let component_name = capitalize_first(stem);

    // Build a small ESM script that imports the component and calls it.
    // Top-level await handles both sync and async component functions.
    let script = format!(
        "import {{ {} }} from '{}';\nconst _result = await {}({{}});\nconsole.log(JSON.stringify(_result));\n",
        component_name,
        abs_str.replace('\\', "/"),
        component_name,
    );

    let child = ProcCommand::new("node")
        .arg("--input-type=module")
        .arg("-e")
        .arg(&script)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn node: {}", e))?;

    let output = child.wait_with_output().map_err(|e| format!("node wait: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("node prerender failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() { return Ok(String::new()); }

    // Parse as JSON — the server component returns { html, clientBundles } or a plain string
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(html) = val.get("html").and_then(|v| v.as_str()) {
            return Ok(html.to_string());
        }
        if let Some(s) = val.as_str() {
            return Ok(s.to_string());
        }
    }
    // Not JSON? Treat as raw HTML string
    Ok(trimmed.to_string())
}

// ── routes manifest ────────────────────────────────────────────────────

#[derive(Serialize)]
struct ClientModuleEntry {
    name: String,
    path: String,
}

#[derive(Serialize)]
struct RouteEntry<'a> {
    path: &'a str,
    file: &'a str,
    mode: &'a str,
    css: &'a [String],
    #[serde(rename = "clientModules", skip_serializing_if = "Vec::is_empty")]
    client_modules: Vec<ClientModuleEntry>,
}

#[derive(Serialize)]
struct RoutesManifest<'a> {
    version: u32,
    runtime: &'a str,
    routes: Vec<RouteEntry<'a>>,
}

fn generate_routes_json(out_dir: &Path, page_routes: &[(String, String, Vec<(String, String)>, Vec<String>, bool)]) -> Result<(), String> {
    let entries: Vec<RouteEntry> = page_routes.iter().map(|(route, html_file, client_roots, css_files, has_data)| {
        let modules: Vec<ClientModuleEntry> = client_roots.iter().map(|(name, path)| {
            ClientModuleEntry { name: name.clone(), path: path.clone() }
        }).collect();
        let mode = if *has_data { "server" } else { "static" };
        RouteEntry {
            path: route,
            file: html_file,
            mode,
            css: css_files,
            client_modules: modules,
        }
    }).collect();

    let manifest = RoutesManifest {
        version: 1,
        runtime: "./runtime.mjs",
        routes: entries,
    };

    let json = serde_json::to_string_pretty(&manifest).map_err(|e| format!("json: {}", e))?;
    std::fs::write(out_dir.join("routes.json"), json).map_err(|e| format!("write routes.json: {}", e))?;
    Ok(())
}

fn load_routes(out_dir: &Path) -> HashMap<String, String> {
    let path = out_dir.join("routes.json");
    if let Ok(content) = std::fs::read_to_string(&path) {
        // Try v1 manifest format first
        if let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(routes) = manifest.get("routes").and_then(|v| v.as_array()) {
                let mut map = HashMap::new();
                for entry in routes {
                    if let (Some(p), Some(f)) = (entry.get("path").and_then(|v| v.as_str()), entry.get("file").and_then(|v| v.as_str())) {
                        map.insert(p.to_string(), f.to_string());
                    }
                }
                return map;
            }
            // Legacy flat format: { "/": "index.html", "/about": "about.html" }
            if let Some(obj) = manifest.as_object() {
                let mut map = HashMap::new();
                for (k, v) in obj {
                    if let Some(f) = v.as_str() {
                        map.insert(k.clone(), f.to_string());
                    }
                }
                return map;
            }
        }
    }
    // Fallback: no manifest → serve index.html at /
    let mut map = HashMap::new();
    map.insert("/".to_string(), "index.html".to_string());
    map
}

// ── dev server ─────────────────────────────────────────────────────────

fn run_dev(source: &str, out: &str, port: u16) -> Result<(), String> {
    eprintln!("=== initial build ===");
    build_all(source, out)?;

    let out_dir = Path::new(out).to_path_buf();
    let source_dir = Path::new(source).to_path_buf();
    let routes = load_routes(&out_dir);

    let (tx, rx) = mpsc::channel::<()>();
    start_file_watcher(source_dir.clone(), tx)?;

    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).map_err(|e| format!("bind {}: {}", addr, e))?;
    eprintln!("\n  dev server listening on http://localhost:{}", port);
    eprintln!("  watching {} for changes...\n", source);

    let out_dir_clone = out_dir.clone();
    let routes_clone = routes.clone();
    let source_str = source.to_string();
    let out_str = out.to_string();

    listener.set_nonblocking(true).map_err(|e| format!("set_nonblocking: {}", e))?;

    loop {
        if rx.try_recv().is_ok() {
            eprintln!("── rebuilding ──");
            std::thread::sleep(std::time::Duration::from_millis(100));
            while rx.try_recv().is_ok() {}
            match build_all(&source_str, &out_str) {
                Ok(n) => {
                    eprintln!(" rebuilt {} file(s)\n", n);
                    // Don't update routes — build_all regenerates them
                }
                Err(e) => eprintln!(" rebuild error: {}\n", e),
            }
        }

        if let Ok((mut stream, _)) = listener.accept() {
            handle_http(&out_dir_clone, &routes_clone, &mut stream);
        }

        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn start_file_watcher(source_dir: std::path::PathBuf, tx: mpsc::Sender<()>) -> Result<(), String> {
    use notify::{Event, EventKind, RecursiveMode, Watcher};
    let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
        if let Ok(event) = res {
            match event.kind {
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                    if event.paths.iter().any(|p| p.extension().map_or(false, |e| e == "tsx")) {
                        let _ = tx.send(());
                    }
                }
                _ => {}
            }
        }
    }).map_err(|e| format!("watcher: {}", e))?;
    watcher.watch(&source_dir, RecursiveMode::Recursive).map_err(|e| format!("watch: {}", e))?;
    std::mem::forget(watcher);
    Ok(())
}

fn handle_http(out_dir: &Path, routes: &HashMap<String, String>, stream: &mut std::net::TcpStream) {
    let mut buf = [0u8; 2048];
    let n = stream.read(&mut buf).unwrap_or(0);
    if n == 0 { return; }

    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    let path = if parts.len() >= 2 { parts[1] } else { "/" };

    match path {
        "/__build_timestamp" => {
            let ts = std::fs::read_to_string(out_dir.join("__build_timestamp.txt")).unwrap_or_default();
            let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}", ts.len(), ts);
            let _ = stream.write_all(resp.as_bytes());
        }
        _ => {
            // Check routes manifest first
            let file_name = routes.get(path).cloned();
            let file_path = if let Some(f) = file_name {
                out_dir.join(&f)
            } else if path == "/" {
                out_dir.join("index.html") // fallback
            } else {
                out_dir.join(path.trim_start_matches('/'))
            };

            match std::fs::read(&file_path) {
                Ok(content) => {
                    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    let mime = match ext { "html" => "text/html", "js"|"mjs" => "application/javascript", "json" => "application/json", "css" => "text/css", _ => "text/plain" };
                    let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: {}\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\n\r\n", mime, content.len());
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.write_all(&content);
                }
                Err(_) => {
                    let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                    let _ = stream.write_all(resp.as_bytes());
                }
            }
        }
    }
}
