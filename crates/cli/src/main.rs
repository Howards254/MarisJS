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
        #[arg(default_value = "src")]
        source: String,
        #[arg(long, default_value = "dist")]
        out: String,
    },
    Dev {
        #[arg(default_value = "src")]
        source: String,
        #[arg(long, default_value = "dist")]
        out: String,
        #[arg(long, default_value = "3000")]
        port: u16,
    },
    #[command(about = "Scaffold a starter package.json with dev/build/serve scripts")]
    Init,
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

/// One built page route. `html_file` follows the URL convention (lowercase,
/// depth-consistent: `docs/api/signals.html`), while `mjs_rel` is the REAL
/// compiled output path (source-preserved casing: `pages/Docs/Api/Signals.mjs`).
/// Reconstructing file paths from route strings (with capitalize_first) caused
/// case mismatches for nested routes — this record is the single source of
/// truth for route → real file mapping, shared by the compiler, dev server,
/// adapter-node (via routes.json), and adapter-static.
struct PageRoute {
    route: String,
    html_file: String,
    mjs_rel: String,
    page_roots: Vec<(String, String)>,
    css_files: Vec<String>,
    has_data: bool,
}

/// Everything the CSS collision check needs about one page's transitive
/// closure: the ordered `<link>` files, each file's import site (the .tsx
/// component that imports it), every component in the closure (for site-wide
/// detection), and the parent map (child → parent) for ancestry checks.
struct PageCssClosure {
    css_files: Vec<String>,
    sites: Vec<(String, PathBuf)>,
    components: Vec<PathBuf>,
    parents: HashMap<PathBuf, PathBuf>,
}

impl Default for PageCssClosure {
    fn default() -> Self {
        Self {
            css_files: Vec::new(),
            sites: Vec::new(),
            components: Vec::new(),
            parents: HashMap::new(),
        }
    }
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
        Command::Init => {
            if let Err(e) = run_init() {
                eprintln!("init error: {}", e); std::process::exit(1);
            }
        }
    }
}

// ── init ────────────────────────────────────────────────────────────────

fn run_init() -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("current_dir: {}", e))?;
    let pkg_path = cwd.join("package.json");
    if pkg_path.exists() {
        return Err(format!("package.json already exists at {}", pkg_path.display()));
    }

    let name = cwd.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.to_lowercase().replace([' ', '_'], "-"))
        .unwrap_or_else(|| "marisjs-app".to_string());

    let pkg = format!(
        r#"{{
  "name": {},
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {{
    "dev": "marisjs dev",
    "build": "marisjs build",
    "serve": "npx @marisjs/adapter-node ./dist"
  }}
}}
"#,
        serde_json::to_string(&name).map_err(|e| format!("json name: {}", e))?
    );
    std::fs::write(&pkg_path, pkg).map_err(|e| format!("write package.json: {}", e))?;
    eprintln!("  wrote package.json");
    eprintln!("  next: mkdir -p src/pages && npm run dev");
    Ok(())
}

// ── build ──────────────────────────────────────────────────────────────

fn build_all(source: &str, out: &str) -> Result<usize, String> {
    let source_dir = Path::new(source);
    if !source_dir.exists() {
        return Err(format!("directory '{}' does not exist (run `marisjs init` to scaffold, or pass a source path)", source));
    }
    if !source_dir.is_dir() { return Err(format!("'{}' is not a directory", source)); }

    let mut count = 0usize;
    let out_dir = Path::new(out);
    let _ = std::fs::remove_dir_all(out_dir);
    std::fs::create_dir_all(out_dir).map_err(|e| format!("failed to create out dir: {}", e))?;

    let mut page_routes: Vec<PageRoute> = Vec::new();
    let mut component_meta: HashMap<PathBuf, ComponentMeta> = HashMap::new();

    walk_and_build(source_dir, source_dir, out_dir, &mut count, &mut page_routes, &mut component_meta)?;

    // Write runtime (embedded at compile time)
    let runtime_dest = out_dir.join("runtime.mjs");
    std::fs::write(&runtime_dest, RUNTIME_JS).map_err(|e| format!("failed to write runtime: {}", e))?;
    eprintln!("  wrote runtime → runtime.mjs");

    // Create node_modules shim so Node can resolve @marisjs/runtime during prerendering.
    // The dist/ directory is self-contained: runtime.mjs sits at the root, and the shim
    // tells Node's module resolver to look there. Browser code uses the import map
    // injected into generated HTML instead.
    let nm_runtime = out_dir.join("node_modules/@marisjs/runtime");
    std::fs::create_dir_all(&nm_runtime).map_err(|e| format!("create node_modules: {}", e))?;
    std::fs::write(nm_runtime.join("package.json"), r#"{"name":"@marisjs/runtime","type":"module","main":"../../../runtime.mjs"}"#)
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
    page_routes: &mut Vec<PageRoute>,
    component_meta: &mut HashMap<PathBuf, ComponentMeta>,
) -> Result<(), String> {
    for entry in std::fs::read_dir(current).map_err(|e| format!("read_dir: {}", e))? {
        let entry = entry.map_err(|e| format!("entry: {}", e))?;
        let path = entry.path();
        if path.is_dir() {
            // Never descend into dependency/vcs directories — their contents
            // are neither components nor assets, and copying node_modules
            // wholesale into dist bloats builds massively.
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name == "node_modules" || name == ".git" || name == ".marisjs" {
                    continue;
                }
            }
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
            // Safety net: every import must resolve to an existing file in the
            // source tree. Catches case mismatches and typos the emission logic
            // can't detect.
            let import_diags = validator::validate_imports(&component, rel, base);
            if !import_diags.is_empty() {
                for d in &import_diags {
                    eprintln!("    {}:{} — {}: {} (fix: {})", d.line.unwrap_or(0), d.column.unwrap_or(0), d.code, d.message, d.fix_hint);
                }
                return Err(format!("import validation failed in {}: {} error(s)", rel.display(), import_diags.len()));
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
                // REAL output path (source-preserved casing), NOT reconstructed
                // from the route string — this is the canonical mapping.
                let mjs_rel = normalize_path(&rel.with_extension("mjs"));

                // Collect hydrate roots for THIS page specifically
                let mut page_roots: Vec<(String, String)> = Vec::new();
                if let Some(ref tree) = component.render_tree {
                    for root_name in codegen::collect_hydrate_roots(tree) {
                        let import_path = resolve_client_import(rel, &root_name, &component.imports)?;
                        page_roots.push((root_name, import_path));
                    }
                }
                page_routes.push(PageRoute {
                    route,
                    html_file,
                    mjs_rel,
                    page_roots,
                    css_files: Vec::new(),
                    has_data: component.has_data_call,
                });
            }

            std::fs::write(&out_path, js)
                .map_err(|e| format!("write error for {}: {}", out_path.display(), e))?;
            *count += 1;
        } else if path.is_file() {
            // Static assets (images, fonts, etc.) are copied through so the
            // dev server and adapters can serve them.
            let rel = path.strip_prefix(base).map_err(|e| format!("strip_prefix: {}", e))?;
            let dest = out_base.join(rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("create_dir_all: {}", e))?;
            }
            std::fs::copy(&path, &dest)
                .map_err(|e| format!("copy asset {}: {}", rel.display(), e))?;
            eprintln!("  copied asset {}", rel.display());
        }
    }
    Ok(())
}

// ── CSS collection and copying ────────────────────────────────────────

/// Resolves the transitive CSS for each page by walking the component dependency tree.
/// Copies CSS files into the output directory preserving relative structure.
/// After all pages are walked, runs the CSS class-collision check per page
/// (CSS_COLLISION visibility — see validator::css_collision) and prints any
/// warnings to stderr. Warnings never fail the build.
fn resolve_page_css(
    page_routes: Vec<PageRoute>,
    component_meta: &HashMap<PathBuf, ComponentMeta>,
    source_dir: &Path,
    out_dir: &Path,
) -> Result<Vec<PageRoute>, String> {
    let mut result: Vec<PageRoute> = Vec::new();
    let mut closures: Vec<(usize, PageCssClosure)> = Vec::new();

    for (idx, mut page) in page_routes.into_iter().enumerate() {
        // Use the page's real source path (recorded at build time) — the
        // component_meta keys are the actual on-disk rel paths, so the
        // case must match the source tree exactly.
        let page_rel = page.mjs_rel.trim_end_matches(".mjs").to_string() + ".tsx";
        let page_path = PathBuf::from(&page_rel);

        let mut closure = PageCssClosure::default();
        let mut seen = HashSet::new();
        collect_page_css_closure(&page_path, component_meta, &mut closure, &mut seen);

        // Copy each CSS file into the output directory
        for css_path in &closure.css_files {
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

        page.css_files = closure.css_files.clone();
        closures.push((idx, closure));
        result.push(page);
    }

    // Site-wide detection: a component rendered by more than one page (the
    // Layout convention) owns the base stylesheet layer that other stylesheets
    // legitimately refine — overlaps involving it are expected, not collisions.
    let mut page_count: HashMap<PathBuf, usize> = HashMap::new();
    for (_, closure) in &closures {
        for comp in &closure.components {
            *page_count.entry(comp.clone()).or_insert(0) += 1;
        }
    }

    // Collision visibility: warn (never error) when the same class name is
    // defined in two different .css files loaded into the same page, unless
    // the overlap is the established intentional pattern (cascade override or
    // site-wide base layer).
    for (idx, closure) in &closures {
        let route = result[*idx].route.clone();
        let mut files: Vec<validator::css_collision::CssFileRef> = Vec::new();
        for (file, site) in &closure.sites {
            let classes = match std::fs::read_to_string(source_dir.join(file)) {
                Ok(content) => validator::css_collision::extract_class_names(&content),
                Err(_) => std::collections::BTreeSet::new(),
            };
            files.push(validator::css_collision::CssFileRef {
                file: PathBuf::from(file),
                import_site: site.clone(),
                classes,
            });
        }

        let is_ancestor = |a: &Path, b: &Path| {
            let mut cur = b.to_path_buf();
            while let Some(parent) = closure.parents.get(&cur) {
                if parent == a {
                    return true;
                }
                cur = parent.clone();
            }
            false
        };
        let is_site_wide = |comp: &Path| page_count.get(comp).copied().unwrap_or(0) >= 2;

        let collisions = validator::css_collision::find_css_class_collisions(
            &files, &is_ancestor, &is_site_wide,
        );
        for col in collisions {
            eprintln!(
                "    warning[CSS_CLASS_COLLISION]: class \".{}\" is defined in both {} (imported by {}) and {} (imported by {}) — both stylesheets are loaded into page {}; the later <link> silently wins. If this overlap is unintentional, rename one of the classes (see the component-name prefix convention, spec §2a).",
                col.class, col.file_a.display(), col.site_a.display(),
                col.file_b.display(), col.site_b.display(), route
            );
        }
    }

    Ok(result)
}

/// Walks a page's transitive component tree, recording the ordered CSS closure,
/// each CSS file's import site, every component in the closure, and the
/// parent map used by the collision check's ancestry test. The `seen` set
/// breaks cycles and dedupes components reached via multiple render paths.
fn collect_page_css_closure(
    comp_rel: &Path,
    component_meta: &HashMap<PathBuf, ComponentMeta>,
    closure: &mut PageCssClosure,
    seen: &mut HashSet<PathBuf>,
) {
    if !seen.insert(comp_rel.to_path_buf()) {
        return; // already visited
    }
    closure.components.push(comp_rel.to_path_buf());

    if let Some(meta) = component_meta.get(comp_rel) {
        for css_import in &meta.css_imports {
            // css_import is like "./Cart.css" — resolve relative to the component's directory
            let comp_dir = comp_rel.parent().unwrap_or(Path::new("."));
            let resolved = comp_dir.join(css_import);
            // Normalize: strip leading ./ or ../
            let css_path = normalize_path(&resolved);
            if !closure.sites.iter().any(|(f, _)| *f == css_path) {
                closure.sites.push((css_path.clone(), comp_rel.to_path_buf()));
            }
            if !closure.css_files.contains(&css_path) {
                closure.css_files.push(css_path);
            }
        }

        // Recurse into child components
        if let Some(ref tree) = meta.render_tree {
            let child_tags = codegen::collect_child_component_tags(tree);
            for tag in child_tags {
                if let Some(child_rel) = resolve_component_import_path(&tag, &meta.imports, comp_rel) {
                    closure.parents.insert(child_rel.clone(), comp_rel.to_path_buf());
                    collect_page_css_closure(&child_rel, component_meta, closure, seen);
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

fn route_to_html_file(route: &str) -> String {
    if route == "/" { "index.html".to_string() }
    else { format!("{}.html", route.trim_start_matches('/')) }
}

/// Depth prefix for relative references inside a page's HTML. A page rendered
/// at `/docs/api/signals` (3 segments) lives 3 directories below the dist
/// root, so every root-relative reference (`runtime.mjs`, CSS, client modules)
/// must be prefixed with `../../../` to resolve correctly in the browser.
fn depth_prefix(route: &str) -> String {
    let segs = route.trim_start_matches('/').split('/').filter(|s| !s.is_empty()).count();
    "../".repeat(segs)
}

/// Resolves a hydrate-root component's compiled output path, relative to the
/// dist root (e.g. `pages/components/Widget.mjs`). The depth prefix is applied
/// at emission time (see generate_page_html / adapter htmlShell).
fn resolve_client_import(server_rel: &Path, root_name: &str, imports: &[parser::ImportInfo]) -> Result<String, String> {
    let source = imports.iter()
        .find(|i| i.imported_names.contains(&root_name.to_string()))
        .map(|i| i.source.clone())
        .ok_or_else(|| format!("client root '{}' referenced but no import found", root_name))?;
    let import_src = format!("{}.mjs", source.trim_end_matches(".tsx").trim_end_matches(".ts"));
    let server_parent = server_rel.with_extension("mjs").parent().unwrap_or(Path::new(".")).to_path_buf();
    let resolved = server_parent.join(&import_src);
    Ok(normalize_path(&resolved))
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

// ── pre-render pages ──────────────────────────────────────────────────

fn prerender_pages(out_dir: &Path, page_routes: &[PageRoute]) -> Result<(), String> {
    for page in page_routes {
        let html_path = out_dir.join(&page.html_file);
        if let Some(parent) = html_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create_dir_all: {}", e))?;
        }
        let html = generate_page_html(out_dir, page)?
            .unwrap_or_else(|| format!("<h1>{} — page not found</h1>", page.route));
        std::fs::write(&html_path, html).map_err(|e| format!("write {}: {}", page.html_file, e))?;
        eprintln!("  prerendered {} → {}", page.route, page.html_file);
    }
    Ok(())
}

fn generate_page_html(out_dir: &Path, page: &PageRoute) -> Result<Option<String>, String> {
    // The .mjs path comes from the manifest record (source-preserved casing),
    // NOT reconstructed from the route string.
    let mjs_path = out_dir.join(&page.mjs_rel);
    let server_html = if mjs_path.exists() {
        Some(prerender_with_node(out_dir, &mjs_path)?)
    } else {
        None
    };

    let Some((html_content, head_content)) = server_html else { return Ok(None); };

    // Every root-relative reference must be prefixed by the route depth so it
    // resolves from the page's actual location in the output tree.
    let prefix = depth_prefix(&page.route);
    let runtime = if prefix.is_empty() { "./runtime.mjs".to_string() } else { format!("{}runtime.mjs", prefix) };

    let mut html = String::new();
    html.push_str("<!DOCTYPE html>\n<html>\n<head>\n");
    html.push_str(&format!("  <script type=\"importmap\">\n  {{\n    \"imports\": {{\n      \"@marisjs/runtime\": \"{}\"\n    }}\n  }}\n  </script>\n", runtime));
    if let Some(head) = head_content {
        for line in head.lines() {
            html.push_str(&format!("  {}\n", line));
        }
    }
    for css_file in &page.css_files {
        html.push_str(&format!("  <link rel=\"stylesheet\" href=\"{}{}\">\n", prefix, css_file));
    }
    html.push_str("</head>\n<body>\n");
    html.push_str("  <div id=\"root\">\n");
    html.push_str(&format!("  {}\n", unescape_html(&html_content)));
    html.push_str("  </div>\n");
    html.push_str("  <script type=\"module\">\n");
    if !page.page_roots.is_empty() {
        html.push_str("    import { mount } from '@marisjs/runtime';\n");
        let mut seen = std::collections::HashSet::new();
        for (name, path) in &page.page_roots {
            if !seen.insert(name.clone()) {
                continue; // exactly ONE import per island component
            }
            // A relative module specifier MUST start with ./ ../ or / —
            // at the root (no depth prefix) that means an explicit "./".
            let import_ref = if prefix.is_empty() {
                format!("./{}", path)
            } else {
                format!("{}{}", prefix, path)
            };
            html.push_str(&format!("    import {{ {} }} from '{}';\n", name, import_ref));
        }
        for (name, _path) in &page.page_roots {
            // Mount EVERY instance: a page may use the same island several
            // times (or inside a <For>), so target all matching placeholders.
            // Each placeholder carries its own data-props, serialized at SSR
            // render time (fallback {} for HTML not produced by this compiler).
            html.push_str(&format!(
                "    for (const el of document.querySelectorAll('[data-hydrate=\"{}\"]')) {{ mount(el, () => {}(el.dataset.props ? JSON.parse(el.dataset.props) : {{}})); }}\n",
                name, name
            ));
        }
    }
    html.push_str("  </script>\n</body>\n</html>\n");
    Ok(Some(html))
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

fn prerender_with_node(_out_dir: &Path, mjs_path: &Path) -> Result<(String, Option<String>), String> {
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
    if trimmed.is_empty() { return Ok((String::new(), None)); }

    // Parse as JSON — the server component returns { html, head, clientBundles } or a plain string
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(html) = val.get("html").and_then(|v| v.as_str()) {
            let head = val.get("head").and_then(|v| v.as_str()).map(|s| s.to_string());
            return Ok((html.to_string(), head));
        }
        if let Some(s) = val.as_str() {
            return Ok((s.to_string(), None));
        }
    }
    // Not JSON? Treat as raw HTML string
    Ok((trimmed.to_string(), None))
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
    #[serde(rename = "mjs")]
    mjs: &'a str,
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

fn generate_routes_json(out_dir: &Path, page_routes: &[PageRoute]) -> Result<(), String> {
    let entries: Vec<RouteEntry> = page_routes.iter().map(|page| {
        let modules: Vec<ClientModuleEntry> = page.page_roots.iter().map(|(name, path)| {
            ClientModuleEntry { name: name.clone(), path: path.clone() }
        }).collect();
        let mode = if page.has_data { "server" } else { "static" };
        RouteEntry {
            path: &page.route,
            file: &page.html_file,
            mjs: &page.mjs_rel,
            mode,
            css: &page.css_files,
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
            // Ignore dependency/vcs noise so installing packages doesn't
            // trigger rebuild loops when the project root is the watch root.
            let is_noise = event.paths.iter().any(|p| {
                p.components().any(|c| match c {
                    std::path::Component::Normal(n) => {
                        n == "node_modules" || n == ".git" || n == ".marisjs"
                    }
                    _ => false,
                })
            });
            if is_noise {
                return;
            }
            match event.kind {
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                    // Any change triggers a rebuild: tsx files are recompiled,
                    // new/edited assets (images, css, fonts) are re-copied.
                    let _ = tx.send(());
                }
                _ => {}
            }
        }
    }).map_err(|e| format!("watcher: {}", e))?;
    watcher.watch(&source_dir, RecursiveMode::Recursive).map_err(|e| format!("watch: {}", e))?;
    std::mem::forget(watcher);
    Ok(())
}

fn mime_for_ext(ext: &str) -> &'static str {
    match ext {
        "html" => "text/html",
        "js" | "mjs" => "application/javascript",
        "json" => "application/json",
        "css" => "text/css",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "text/plain",
    }
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
                    let mime = mime_for_ext(ext);
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

#[cfg(test)]
mod mime_tests {
    use super::mime_for_ext;

    #[test]
    fn known_mime_types() {
        assert_eq!(mime_for_ext("html"), "text/html");
        assert_eq!(mime_for_ext("js"), "application/javascript");
        assert_eq!(mime_for_ext("mjs"), "application/javascript");
        assert_eq!(mime_for_ext("json"), "application/json");
        assert_eq!(mime_for_ext("css"), "text/css");
    }

    #[test]
    fn image_mime_types() {
        assert_eq!(mime_for_ext("png"), "image/png");
        assert_eq!(mime_for_ext("jpg"), "image/jpeg");
        assert_eq!(mime_for_ext("jpeg"), "image/jpeg");
        assert_eq!(mime_for_ext("svg"), "image/svg+xml");
        assert_eq!(mime_for_ext("ico"), "image/x-icon");
        assert_eq!(mime_for_ext("webp"), "image/webp");
        assert_eq!(mime_for_ext("gif"), "image/gif");
    }

    #[test]
    fn unknown_extension_falls_back_to_text_plain() {
        assert_eq!(mime_for_ext("wasm"), "text/plain");
        assert_eq!(mime_for_ext("woff2"), "text/plain");
        assert_eq!(mime_for_ext(""), "text/plain");
    }
}
