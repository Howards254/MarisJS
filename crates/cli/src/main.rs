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
    warnings: Vec<ErrorOutput<'a>>,
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
    page_roots: Vec<(String, String, bool)>,
    css_files: Vec<String>,
    has_data: bool,
    /// §E2.1: meta({ noindex: true }) — the route is excluded from sitemap.xml.
    noindex: bool,
}

/// One built API route. `file` is the REAL compiled output path
/// (source-preserved casing), `methods` the sanctioned HTTP methods
/// exported by the file (the router 405s anything else).
#[derive(Clone)]
struct ApiRoute {
    path: String,
    file: String,
    methods: Vec<String>,
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
                    // §7b: same dispatch as build — api/ files (or files
                    // with @runsOn api) validate with the API rule set, not
                    // the component rule set.
                    let diagnostics = validator::validate_for_path(&component, std::path::Path::new(&file));
                    let errors: Vec<ErrorOutput> = diagnostics.iter()
                        .filter(|d| !d.is_warning)
                        .map(|d| ErrorOutput {
                            line: d.line, column: d.column, code: d.code,
                            message: &d.message, fix_hint: d.fix_hint,
                        }).collect();
                    let warnings: Vec<ErrorOutput> = diagnostics.iter()
                        .filter(|d| d.is_warning)
                        .map(|d| ErrorOutput {
                            line: d.line, column: d.column, code: d.code,
                            message: &d.message, fix_hint: d.fix_hint,
                        }).collect();
                    let valid = errors.is_empty();
                    println!("{}", serde_json::to_string_pretty(&ValidateOutput { valid, errors, warnings }).unwrap());
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

    // §7a: the .gitignore excluding .env is the PRIMARY defense against
    // accidental secret commits. Append when a .gitignore already exists.
    let gitignore_path = cwd.join(".gitignore");
    let mut gitignore = String::new();
    if gitignore_path.exists() {
        gitignore = std::fs::read_to_string(&gitignore_path).map_err(|e| format!("read .gitignore: {}", e))?;
        if !gitignore.ends_with('\n') { gitignore.push('\n'); }
    }
    let env_line = ".env\n";
    let already = gitignore.lines().any(|l| l.trim() == ".env");
    if !already {
        gitignore.push_str(env_line);
        std::fs::write(&gitignore_path, gitignore).map_err(|e| format!("write .gitignore: {}", e))?;
        eprintln!("  wrote .gitignore (excludes .env)");
    } else {
        eprintln!("  .gitignore already excludes .env");
    }

    // .env.example documents the keys a project expects — with NO real values
    // (it is committed by design).
    let example_path = cwd.join(".env.example");
    if !example_path.exists() {
        std::fs::write(&example_path, "# Copy to .env and fill in real values. .env is gitignored.\n# Example:\n# MY_API_KEY=changeme\n").map_err(|e| format!("write .env.example: {}", e))?;
        eprintln!("  wrote .env.example");
    }

    eprintln!("  next: mkdir -p src/pages && npm run dev");
    Ok(())
}

// ── build ──────────────────────────────────────────────────────────────

/// §7a: Loads the project's environment snapshot for build/dev time.
/// 1. Real process env (always wins — standard dotenv non-override: this is
///    how CI injects real secrets without editing .env).
/// 2. `.env` from the project root (the directory `marisjs` was invoked in),
///    falling back to the source directory when different.
/// Keys are taken verbatim (dotenv convention); `#` comments and quoted
/// values are handled by the same tokenizer the tests exercise.
fn load_env(source: &str) -> codegen::EnvMap {
    let mut env: codegen::EnvMap = std::env::vars().collect();

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut candidates = vec![cwd.join(".env")];
    let source_dir = Path::new(source);
    if source_dir != cwd {
        candidates.push(source_dir.join(".env"));
    }
    for candidate in candidates {
        if let Ok(content) = std::fs::read_to_string(&candidate) {
            for (k, v) in parse_dotenv(&content) {
                // Non-override: a real process-env key is never shadowed.
                env.entry(k).or_insert(v);
            }
            eprintln!("  loaded {}", candidate.display());
            break;
        }
    }
    env
}

/// dotenv tokenizer: `KEY=value` lines, `#` comments (own-line, plus inline
/// after unquoted values), quoted values (single or double, escaped quotes),
/// blank lines skipped. Keys are not validated beyond being non-empty —
/// dotenv itself is permissive here.
fn parse_dotenv(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, rest) = match line.split_once('=') {
            Some((k, r)) => (k.trim(), r.trim()),
            None => continue,
        };
        if key.is_empty() {
            continue;
        }
        let value = if rest.starts_with('"') {
            // Scan for the closing quote, honoring escapes (\", \\).
            let inner = &rest[1..];
            let mut out = String::new();
            let mut chars = inner.chars().peekable();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => {
                        if let Some(&next) = chars.peek() {
                            if next == '"' || next == '\\' {
                                out.push(next);
                                chars.next();
                            } else {
                                out.push('\\');
                            }
                        } else {
                            out.push('\\');
                        }
                    }
                    _ => out.push(c),
                }
            }
            out
        } else if rest.starts_with('\'') {
            let inner = &rest[1..];
            let end = inner.find('\'').unwrap_or(inner.len());
            inner[..end].to_string()
        } else {
            // Unquoted: strip trailing comment and whitespace.
            match rest.find(" #") {
                Some(i) => rest[..i].trim().to_string(),
                None => rest.trim().to_string(),
            }
        };
        out.push((key.to_string(), value));
    }
    out
}

fn build_all(source: &str, out: &str) -> Result<usize, String> {
    let source_dir = Path::new(source);
    if !source_dir.exists() {
        return Err(format!("directory '{}' does not exist (run `marisjs init` to scaffold, or pass a source path)", source));
    }
    if !source_dir.is_dir() { return Err(format!("'{}' is not a directory", source)); }

    let env = load_env(source);

    let mut count = 0usize;
    let out_dir = Path::new(out);
    let _ = std::fs::remove_dir_all(out_dir);
    std::fs::create_dir_all(out_dir).map_err(|e| format!("failed to create out dir: {}", e))?;

    let mut page_routes: Vec<PageRoute> = Vec::new();
    let mut api_routes: Vec<ApiRoute> = Vec::new();
    let mut component_meta: HashMap<PathBuf, ComponentMeta> = HashMap::new();

    // E4-01: pre-classify every compilable source file as server-side or not
    // BEFORE emitting anything, so server modules can rewrite imports of
    // client modules (hydrate islands) correctly.
    let (server_files, runs_on_by_rel) = preclassify_files(source_dir)?;

    walk_and_build(source_dir, source_dir, out_dir, &mut count, &mut page_routes, &mut api_routes, &mut component_meta, &env, &server_files, &runs_on_by_rel)?;

    // §7d: build the project-root middleware.ts (if any) into
    // dist/_server/middleware.mjs. Must run AFTER walk_and_build's
    // client→server import check, which needs runs_on_by_rel populated with
    // the middleware classification.
    let has_middleware = build_middleware(source_dir, out_dir, &env)?;

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
    generate_routes_json(out_dir, &page_routes, &api_routes, has_middleware)?;
    eprintln!("  generated routes.json");

    // §E2.1: sitemap.xml (all pages, API routes excluded, noindex excluded,
    // SITE_URL required) + default robots.txt (only when the project provides
    // none at the source root).
    let site_url = env.get("SITE_URL").cloned();
    generate_sitemap(out_dir, &page_routes, site_url.as_deref())?;
    generate_default_robots(source_dir, out_dir, site_url.as_deref())?;

    write_reload_timestamp(out_dir);
    Ok(count)
}

fn write_reload_timestamp(out_dir: &Path) {
    let ts = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis();
    let _ = std::fs::write(out_dir.join("__build_timestamp.txt"), ts.to_string());
}

/// E4-01: walk the source tree once, parsing every compilable file to decide
/// whether it will be emitted under `_server/` (api files and @runsOn server
/// files). Keys are root-relative source paths (extension included), matching
/// how walk_and_build computes rel. The second map carries every parsed
/// file's runs_on — the E4-14 client→server import check needs the global
/// picture (a single file cannot know what its neighbors are).
fn preclassify_files(
    source_dir: &Path,
) -> Result<(HashSet<PathBuf>, HashMap<PathBuf, parser::RunsOn>), String> {
    let mut server_files = HashSet::new();
    let mut runs_on_by_rel: HashMap<PathBuf, parser::RunsOn> = HashMap::new();
    let mut stack: Vec<PathBuf> = vec![source_dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current).map_err(|e| format!("read_dir: {}", e))? {
            let entry = entry.map_err(|e| format!("entry: {}", e))?;
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name == "node_modules" || name == ".git" || name == ".marisjs" {
                        continue;
                    }
                }
                stack.push(path);
            } else if path.extension().map_or(false, |ext| ext == "tsx")
                || (path.extension().map_or(false, |ext| ext == "ts")
                    && path.strip_prefix(source_dir).map_or(false, |rel| {
                        rel == Path::new("api") || rel.starts_with("api/")
                    }))
            {
                let Ok(rel) = path.strip_prefix(source_dir) else { continue };
                let is_api = rel == Path::new("api") || rel.starts_with("api/");
                let Some(file_path) = path.to_str() else { continue };
                if let Ok(component) = parser::parse_component_file(file_path) {
                    let runs_on = if is_api {
                        parser::RunsOn::Api
                    } else {
                        component.runs_on.clone().unwrap_or(parser::RunsOn::Client)
                    };
                    runs_on_by_rel.insert(rel.to_path_buf(), runs_on);
                    if is_api || component.runs_on == Some(parser::RunsOn::Server) {
                        server_files.insert(rel.to_path_buf());
                    }
                }
            } else if path.extension().map_or(false, |ext| ext == "ts")
                && path.strip_prefix(source_dir).map_or(false, |rel| {
                    rel == Path::new("middleware.ts")
                })
            {
                // §7d: the project-root middleware.ts is a server-side module
                // (emitted under _server/) — a client file importing it must
                // fail the E4-14 client→server check like any other server
                // module.
                let Ok(rel) = path.strip_prefix(source_dir) else { continue };
                runs_on_by_rel.insert(rel.to_path_buf(), parser::RunsOn::Api);
                server_files.insert(rel.to_path_buf());
            }
        }
    }
    Ok((server_files, runs_on_by_rel))
}

fn walk_and_build(
    base: &Path, current: &Path, out_base: &Path, count: &mut usize,
    page_routes: &mut Vec<PageRoute>,
    api_routes: &mut Vec<ApiRoute>,
    component_meta: &mut HashMap<PathBuf, ComponentMeta>,
    env: &codegen::EnvMap,
    server_files: &HashSet<PathBuf>,
    runs_on_by_rel: &HashMap<PathBuf, parser::RunsOn>,
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
walk_and_build(base, &path, out_base, count, page_routes, api_routes, component_meta, env, server_files, runs_on_by_rel)?;
        } else if path.extension().map_or(false, |ext| ext == "tsx")
            // §7b: api/ files compile from .ts OR .tsx (a route handler has no
            // JSX — it is ordinary TypeScript). Everywhere else .ts is an
            // unknown extension, never a component.
            || (path.extension().map_or(false, |ext| ext == "ts")
                && path.strip_prefix(base).map_or(false, |rel| {
                    rel == Path::new("api") || rel.starts_with("api/")
                }))
        {
            let rel = path.strip_prefix(base).map_err(|e| format!("strip_prefix: {}", e))?;
            let is_api = rel == Path::new("api") || rel.starts_with("api/");

            let file_path = path.to_str().ok_or("invalid path")?;
            let component = parser::parse_component_file(file_path)
                .map_err(|e| format!("parse error in {}: {}", rel.display(), e))?;
            // E4-14: a @runsOn client file importing a server-side module
            // (api handler or @runsOn server component) is a hard error — the
            // target is emitted under _server/ with no public URL, so the
            // client bundle would reference an undefined import at runtime.
            // The check needs the global runs_on map; a single file cannot
            // know what its neighbors are.
            let client_side = !is_api && component.runs_on != Some(parser::RunsOn::Server);
            if client_side {
                let file_parent = rel.parent().unwrap_or(Path::new(""));
                for imp in &component.imports {
                    if imp.is_css
                        || !(imp.source.starts_with("./") || imp.source.starts_with("../"))
                    {
                        continue;
                    }
                    let trimmed = imp.source.trim_end_matches(".tsx").trim_end_matches(".ts");
                    let resolved = normalize_path(&file_parent.join(trimmed));
                    for ext in ["tsx", "ts"] {
                        let target = PathBuf::from(format!("{}.{}", resolved, ext));
                        if let Some(target_runs_on) = runs_on_by_rel.get(&target) {
                            if *target_runs_on != parser::RunsOn::Client {
                                return Err(format!(
                                    "{}:{} — CLIENT_IMPORTS_SERVER: client file imports '{}' which runs on {} (server-side modules have no public URL); move the import into a @runsOn server component and pass data down via props",
                                    rel.display(),
                                    imp.line,
                                    imp.source,
                                    match target_runs_on {
                                        parser::RunsOn::Server => "server",
                                        parser::RunsOn::Api => "api",
                                        parser::RunsOn::Client => "client",
                                    }
                                ));
                            }
                            break;
                        }
                    }
                }
            }
            // E4-01: server-side modules (api handlers, @runsOn server pages
            // and components) are emitted under `_server/` — the dev server
            // and adapter-node NEVER serve that prefix statically. They are
            // only reachable through the dispatchers, so the baked env
            // snapshot (SESSION_SECRET, API keys) can never be downloaded.
            // Client modules stay at their public path (hydration imports).
            let server_side = is_api || component.runs_on == Some(parser::RunsOn::Server);
            let out_path = if server_side {
                out_base.join("_server").join(rel).with_extension("mjs")
            } else {
                out_base.join(rel).with_extension("mjs")
            };
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("create_dir_all: {}", e))?;
            }
            eprintln!("  building {}", rel.display());

            // §7b: api files get their own, smaller rule set — an API route
            // handler is not a component (no props/ordering/signals/JSX
            // checks). Everything else — pages and components — gets the full
            // component rule set.
            let diagnostics = if is_api {
                validator::validate_api(&component)
            } else {
                validator::validate(&component)
            };
            // E4-13: a @runsOn server PAGE without data() is prerendered at
            // build time into static HTML served to everyone; env() values
            // evaluated during that prerender can leak into the public .html.
            let is_page = rel.starts_with("pages") || rel.starts_with("pages/");
            let mut diagnostics = diagnostics;
            if !is_api
                && is_page
                && component.runs_on == Some(parser::RunsOn::Server)
                && !component.has_data_call
                && component.has_env_call
            {
                diagnostics.push(validator::Diagnostic::warning(
                    "SERVER_PAGE_ENV_PRERENDER",
                    format!(
                        "this page is prerendered into static HTML at build time (no data()), but its module code reads env() — the value is evaluated during prerender and may end up in the public .html file served to everyone. Never read secrets (SESSION_SECRET, API keys) here."
                    ),
                    "Read secrets only from api routes or data(); for non-secret configuration, this warning is informational.",
                    Some(component.env_call_line.max(1)),
                    Some(component.env_call_column.max(1)),
                ));
            }
            let errors: Vec<&validator::Diagnostic> = diagnostics.iter().filter(|d| !d.is_warning).collect();
            for d in diagnostics.iter().filter(|d| d.is_warning) {
                eprintln!("    warning[{}]: {} (fix: {})", d.code, d.message, d.fix_hint);
            }
            if !errors.is_empty() {
                for d in &errors {
                    eprintln!("    {}:{} — {}: {} (fix: {})", d.line.unwrap_or(0), d.column.unwrap_or(0), d.code, d.message, d.fix_hint);
                }
                return Err(format!("validation failed in {}: {} error(s)", rel.display(), errors.len()));
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

            // §7c: a module that calls session()/setSession() MUST build with a
            // strong SESSION_SECRET present (in .env or the real environment).
            // Fails loudly here — never silently with a weak/default/empty
            // secret: empty, whitespace-only, or sub-16-byte values are
            // rejected (16+ bytes keeps the HMAC key at ≥128 bits).
            if component.has_session_call {
                let secret = env.get("SESSION_SECRET").map(|v| v.as_str()).unwrap_or("");
                let trimmed = secret.trim();
                if trimmed.is_empty() || trimmed.len() < 16 {
                    return Err(format!(
                        "SESSION_SECRET is missing or too weak, but {} uses session()/setSession() — session signing cannot work without it. Add a strong SESSION_SECRET (16+ characters) to .env or the environment (e.g. `openssl rand -base64 32`).",
                        rel.display()
                    ));
                }
            }

// §7e: UNEXPECTED_CHILDREN — a component call with nested JSX
            // whose Props type declares no children field. The call site's
            // file cannot know the TARGET's Props type, so this check is
            // build-orchestrated (same reason as CLIENT_IMPORTS_SERVER): the
            // tag is resolved through the file's imports, the target file is
            // parsed, and its declared Props fields are inspected. Targets
            // whose Props type the compiler cannot see (untyped/any, or
            // imported from a *.types.ts file) are skipped — their own
            // validation already flags broken props parameters, and the
            // check is exactly as strong as the view it has.
            let children_calls = component
                .render_tree
                .as_ref()
                .map(parser::collect_children_call_tags)
                .unwrap_or_default();
            let file_parent = rel.parent().unwrap_or(Path::new(""));
            for tag in children_calls {
                let target_props_type = if component
                    .exports
                    .first()
                    .map(|e| e.name == tag)
                    .unwrap_or(false)
                {
                    component.props_type.clone()
                } else {
                    let mut resolved: Option<parser::TypeDecl> = None;
                    for imp in &component.imports {
                        if imp.is_css
                            || !(imp.source.starts_with("./") || imp.source.starts_with("../"))
                            || !imp.imported_names.contains(&tag)
                        {
                            continue;
                        }
                        let trimmed = imp.source.trim_end_matches(".tsx").trim_end_matches(".ts");
                        let target_rel = normalize_path(&file_parent.join(trimmed));
                        for ext in ["tsx", "ts"] {
                            let target = base.join(format!("{}.{}", target_rel, ext));
                            if target.exists() {
                                if let Ok(target_file) = parser::parse_component_file(target.to_str().unwrap()) {
                                    resolved = target_file.props_type;
                                }
                                break;
                            }
                        }
                        break;
                    }
                    resolved
                };
                if let Some(diag) = validator::unexpected_children_diagnostic(&tag, target_props_type.as_ref()) {
                    return Err(format!(
                        "{}:{} — {}: {} (fix: {})",
                        rel.display(),
                        component
                            .imports
                            .iter()
                            .find(|i| i.imported_names.contains(&tag))
                            .map(|i| i.line)
                            .unwrap_or(1),
                        diag.code,
                        diag.message,
                        diag.fix_hint
                    ));
                }
            }

            if is_api {
                let js = codegen::generate_api(&component, env)
                    .map_err(|e| format!("codegen error in {}: {}", rel.display(), e))?;
                std::fs::write(&out_path, js)
                    .map_err(|e| format!("write error for {}: {}", out_path.display(), e))?;
                // §7b: file-based routing — api/checkout.ts → /api/checkout.
                // Supported methods ARE the exported handler names.
                let route = api_file_to_route(rel);
                let methods: Vec<String> = component.exports.iter()
                    .filter(|e| e.kind == parser::ExportKind::NamedFunction)
                    .map(|e| e.name.clone())
                    .collect();
                api_routes.push(ApiRoute {
                    path: route,
                    file: format!("_server/{}", normalize_path(&rel.with_extension("mjs"))),
                    methods,
                });
                *count += 1;
            } else {
                let js = codegen::generate_with_server_files(&component, env, rel, server_files)
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
                    // from the route string — this is the canonical mapping. Server
                    // pages live under _server/ (E4-01) — the field is only read by
                    // adapter-node's SSR dispatcher, never served statically.
                    let mjs_rel = format!("_server/{}", normalize_path(&rel.with_extension("mjs")));

                    // Collect hydrate roots for THIS page specifically.
                    // §7e (v1.1): the third tuple element reports whether the
                    // island receives children — its mount call must adopt
                    // el.firstChild as the children value (children are
                    // server-rendered inside the placeholder at SSR time).
                    let mut page_roots: Vec<(String, String, bool)> = Vec::new();
                    if let Some(ref tree) = component.render_tree {
                        for (root_name, has_children) in codegen::collect_hydrate_roots_with_children(tree) {
                            let import_path = resolve_client_import(rel, &root_name, &component.imports)?;
                            page_roots.push((root_name, import_path, has_children));
                        }
                    }
                    page_routes.push(PageRoute {
                        route,
                        html_file,
                        mjs_rel,
                        page_roots,
                        css_files: Vec::new(),
                        has_data: component.has_data_call,
                        noindex: component.head_noindex,
                    });
                }

                std::fs::write(&out_path, js)
                    .map_err(|e| format!("write error for {}: {}", out_path.display(), e))?;
                *count += 1;
            }
        } else if path.is_file() {
            // Static assets (images, fonts, etc.) are copied through so the
            // dev server and adapters can serve them. `.env` is NEVER copied:
            // it holds build-time secrets, and dist/ is deployable output.
            // middleware.ts is compiled into _server/middleware.mjs and must
            // NEVER be copied verbatim — it is the request-gate source and is
            // only reachable through the server dispatchers (E4-01).
            let rel = path.strip_prefix(base).map_err(|e| format!("strip_prefix: {}", e))?;
            if rel.file_name().map_or(false, |n| n == ".env" || n == "middleware.ts") {
                continue;
            }
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

/// §7d: build the project-root middleware.ts (when present) into
/// dist/_server/middleware.mjs. Same gates as api files: the middleware rule
/// set (matcher + sanctioned returns), the SESSION_SECRET gate when
/// session()/setSession() is used, and import resolution. Returns whether a
/// middleware.ts exists (so the manifest can record it).
fn build_middleware(source_dir: &Path, out_dir: &Path, env: &codegen::EnvMap) -> Result<bool, String> {
    let middleware_path = source_dir.join("middleware.ts");
    if !middleware_path.is_file() {
        return Ok(false);
    }

    let component = parser::parse_middleware_file(middleware_path.to_str().unwrap())
        .map_err(|e| format!("parse error in middleware.ts: {}", e))?;

    let diagnostics = validator::validate_middleware(&component);
    let errors: Vec<&validator::Diagnostic> = diagnostics.iter().filter(|d| !d.is_warning).collect();
    for d in diagnostics.iter().filter(|d| d.is_warning) {
        eprintln!("    warning[{}]: {} (fix: {})", d.code, d.message, d.fix_hint);
    }
    if !errors.is_empty() {
        for d in &errors {
            eprintln!("    {}:{} — {}: {} (fix: {})", d.line.unwrap_or(0), d.column.unwrap_or(0), d.code, d.message, d.fix_hint);
        }
        return Err(format!("validation failed in middleware.ts: {} error(s)", errors.len()));
    }

    let import_diags = validator::validate_imports(&component, Path::new("middleware.ts"), source_dir);
    if !import_diags.is_empty() {
        for d in &import_diags {
            eprintln!("    {}:{} — {}: {} (fix: {})", d.line.unwrap_or(0), d.column.unwrap_or(0), d.code, d.message, d.fix_hint);
        }
        return Err(format!("import validation failed in middleware.ts: {} error(s)", import_diags.len()));
    }

    if component.has_session_call {
        let secret = env.get("SESSION_SECRET").map(|v| v.as_str()).unwrap_or("");
        let trimmed = secret.trim();
        if trimmed.is_empty() || trimmed.len() < 16 {
            return Err(
                "SESSION_SECRET is missing or too weak, but middleware.ts uses session()/setSession() — session signing cannot work without it. Add a strong SESSION_SECRET (16+ characters) to .env or the environment (e.g. `openssl rand -base64 32`).".to_string(),
            );
        }
    }

    let js = codegen::generate_middleware(&component, env)
        .map_err(|e| format!("codegen error in middleware.ts: {}", e))?;
    let out_path = out_dir.join("_server/middleware.mjs");
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create_dir_all: {}", e))?;
    }
    std::fs::write(&out_path, js).map_err(|e| format!("write error for {}: {}", out_path.display(), e))?;
    eprintln!("  building middleware.ts");
    Ok(true)
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
        // case must match the source tree exactly. The manifest mjs path
        // carries the internal `_server/` prefix; the source rel never does.
        let page_rel = page
            .mjs_rel
            .strip_prefix("_server/")
            .unwrap_or(&page.mjs_rel)
            .trim_end_matches(".mjs")
            .to_string()
            + ".tsx";
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

/// §7b: file-based API routing — api/checkout.ts → /api/checkout, nested
/// like pages (api/billing/charge.ts → /api/billing/charge, api/Index.ts →
/// /api). Unlike pages, the URL segment is the file name as written (API
/// paths are case-sensitive by convention; no lowercasing).
fn api_file_to_route(rel: &Path) -> String {
    let s = rel.with_extension("").to_str().unwrap_or("").to_string();
    let path = s.strip_prefix("api/").or_else(|| s.strip_prefix("api")).unwrap_or(&s);
    if path.is_empty() || path == "Index" || path.ends_with("/Index") {
        let parent = Path::new(&path).parent().and_then(|p| p.to_str()).unwrap_or("");
        if parent.is_empty() { return "/api".to_string(); }
        return format!("/api{}", parent);
    }
    format!("/api/{}", path)
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
        for (name, path, _has_children) in &page.page_roots {
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
        for (name, _path, has_children) in &page.page_roots {
            // Mount EVERY instance: a page may use the same island several
            // times (or inside a <For>), so target all matching placeholders.
            // Each placeholder carries its own data-props, serialized at SSR
            // render time (fallback {} for HTML not produced by this compiler).
            // §7e (v1.1): islands with children adopt el.firstChild as the
            // children value — SSR rendered the children INTO the placeholder
            // (whitespace-only text is dropped, so firstChild is exactly the
            // children root, element or text node). Detach BEFORE mount: the
            // wrapper re-appends it at its {props.children} position, and an
            // instance that received no children gets an empty text node.
            if *has_children {
                html.push_str(&format!(
                    "    for (const el of document.querySelectorAll('[data-hydrate=\"{}\"]')) {{ const _children = el.firstChild; if (_children) _children.remove(); mount(el, () => {}({{ ...(el.dataset.props ? JSON.parse(el.dataset.props) : {{}}), children: _children }})); }}\n",
                    name, name
                ));
            } else {
                html.push_str(&format!(
                    "    for (const el of document.querySelectorAll('[data-hydrate=\"{}\"]')) {{ mount(el, () => {}(el.dataset.props ? JSON.parse(el.dataset.props) : {{}})); }}\n",
                    name, name
                ));
            }
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
    #[serde(rename = "apiRoutes", skip_serializing_if = "Vec::is_empty")]
    api_routes: Vec<ApiRouteEntry<'a>>,
    /// §7d: present only when the project has a middleware.ts. The file is
    /// the compiled dist/_server/middleware.mjs; dev server and adapter-node
    /// import it to evaluate request gating BEFORE route/api dispatch.
    #[serde(skip_serializing_if = "Option::is_none")]
    middleware: Option<MiddlewareEntry<'a>>,
}

#[derive(Serialize)]
struct MiddlewareEntry<'a> {
    file: &'a str,
}

#[derive(Serialize)]
struct ApiRouteEntry<'a> {
    path: &'a str,
    file: &'a str,
    methods: &'a [String],
}

fn generate_routes_json(
    out_dir: &Path,
    page_routes: &[PageRoute],
    api_routes: &[ApiRoute],
    has_middleware: bool,
) -> Result<(), String> {
    let entries: Vec<RouteEntry> = page_routes.iter().map(|page| {
        let modules: Vec<ClientModuleEntry> = page.page_roots.iter().map(|(name, path, _has_children)| {
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

    let api_entries: Vec<ApiRouteEntry> = api_routes.iter().map(|api| ApiRouteEntry {
        path: &api.path,
        file: &api.file,
        methods: &api.methods,
    }).collect();

    let middleware_entry = if has_middleware {
        Some(MiddlewareEntry { file: "_server/middleware.mjs" })
    } else {
        None
    };

    let manifest = RoutesManifest {
        version: 1,
        runtime: "./runtime.mjs",
        routes: entries,
        api_routes: api_entries,
        middleware: middleware_entry,
    };

    let json = serde_json::to_string_pretty(&manifest).map_err(|e| format!("json: {}", e))?;
    std::fs::write(out_dir.join("routes.json"), json).map_err(|e| format!("write routes.json: {}", e))?;
    Ok(())
}

/// §E2.1: generate sitemap.xml from the built page routes. API routes are
/// never listed (they are not documents); pages with meta({ noindex: true })
/// opt out. SITE_URL is required — the sitemap protocol mandates absolute
/// URLs — so the build warns and skips when it is absent.
fn generate_sitemap(
    out_dir: &Path,
    page_routes: &[PageRoute],
    site_url: Option<&str>,
) -> Result<(), String> {
    let Some(site_url) = site_url else {
        eprintln!("  skipping sitemap.xml — set SITE_URL (absolute site URL, e.g. https://example.com) to generate");
        return Ok(());
    };
    let base = site_url.trim_end_matches('/');
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
    for page in page_routes {
        if page.noindex {
            continue;
        }
        let loc = if page.route == "/" {
            format!("{base}/")
        } else {
            format!("{base}{}", xml_escape(&page.route))
        };
        xml.push_str(&format!("  <url><loc>{}</loc></url>\n", xml_escape(&loc)));
    }
    xml.push_str("</urlset>\n");
    std::fs::write(out_dir.join("sitemap.xml"), xml)
        .map_err(|e| format!("write sitemap.xml: {}", e))?;
    eprintln!("  generated sitemap.xml");
    Ok(())
}

/// §E2.1: write a default robots.txt ONLY when the project provides none at
/// the source root (a project-provided robots.txt is copied through by the
/// static-asset pass and must never be overwritten). The Sitemap line is
/// included only when SITE_URL is known.
fn generate_default_robots(
    source_dir: &Path,
    out_dir: &Path,
    site_url: Option<&str>,
) -> Result<(), String> {
    if source_dir.join("robots.txt").exists() {
        eprintln!("  keeping project-provided robots.txt");
        return Ok(());
    }
    let mut content = String::from("User-agent: *\nAllow: /\n");
    if let Some(site_url) = site_url {
        content.push_str(&format!(
            "\nSitemap: {}/sitemap.xml\n",
            site_url.trim_end_matches('/')
        ));
    }
    std::fs::write(out_dir.join("robots.txt"), content)
        .map_err(|e| format!("write robots.txt: {}", e))?;
    eprintln!("  generated robots.txt");
    Ok(())
}

/// §E2.1: XML-escape a URL for sitemap.xml (route segments are path-safe, but
/// the protocol is strict XML).
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
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

/// §7b: loads the apiRoutes section of the manifest (dev server dispatch).
fn load_api_routes(out_dir: &Path) -> Vec<ApiRoute> {
    let path = out_dir.join("routes.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Vec::new();
    };
    let Some(entries) = manifest.get("apiRoutes").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let path = entry.get("path").and_then(|v| v.as_str())?.to_string();
            let file = entry.get("file").and_then(|v| v.as_str())?.to_string();
            let methods = entry
                .get("methods")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            Some(ApiRoute { path, file, methods })
        })
        .collect()
}

// ── dev server ─────────────────────────────────────────────────────────

/// §7d: read the compiled middleware module path from routes.json
/// (None when the project has no middleware.ts).
fn load_middleware_path(out_dir: &Path) -> Option<String> {
    let path = out_dir.join("routes.json");
    let content = std::fs::read_to_string(&path).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&content).ok()?;
    let file = manifest.get("middleware")?.get("file")?.as_str()?;
    Some(file.to_string())
}

fn run_dev(source: &str, out: &str, port: u16) -> Result<(), String> {
    eprintln!("=== initial build ===");
    build_all(source, out)?;

    let out_dir = Path::new(out).to_path_buf();
    let source_dir = Path::new(source).to_path_buf();
    let mut routes = load_routes(&out_dir);
    let mut api_routes = load_api_routes(&out_dir);
    let mut middleware = load_middleware_path(&out_dir);

    let (tx, rx) = mpsc::channel::<()>();
    start_file_watcher(source_dir.clone(), tx)?;

    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).map_err(|e| format!("bind {}: {}", addr, e))?;
    eprintln!("\n  dev server listening on http://localhost:{}", port);
    eprintln!("  watching {} for changes...\n", source);

    let out_dir_clone = out_dir.clone();
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
                    // F4: reload the dispatch tables after every successful
                    // rebuild. A newly added middleware.ts (or a changed
                    // route set) must engage immediately — serving the stale
                    // pre-middlware state would be a silent fail-open until
                    // restart, on a 0.0.0.0 server.
                    routes = load_routes(&out_dir);
                    api_routes = load_api_routes(&out_dir);
                    middleware = load_middleware_path(&out_dir);
                }
                Err(e) => eprintln!(" rebuild error: {}\n", e),
            }
        }

        if let Ok((mut stream, _)) = listener.accept() {
            handle_http(&out_dir_clone, &routes, &api_routes, &middleware, &mut stream);
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


/// E4-02: lexically normalize a request path (resolve `.` and `..` without
/// following symlinks) and return None if it would escape the dist root.
fn safe_static_path(out_dir: &Path, path: &str) -> Option<PathBuf> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return None;
                }
            }
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(out_dir.join(parts.join("/")))
}

fn handle_http(
    out_dir: &Path,
    routes: &HashMap<String, String>,
    api_routes: &[ApiRoute],
    middleware: &Option<String>,
    stream: &mut std::net::TcpStream,
) {
    // Read the request head plus any body (Content-Length governs how much
    // more to read after the header terminator — API routes read POST bodies).
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let n = stream.read(&mut tmp).unwrap_or(0);
    if n == 0 { return; }
    buf.extend_from_slice(&tmp[..n]);

    let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n");
    let header_len = header_end.map(|i| i + 4).unwrap_or(buf.len());
    let content_length: usize = header_end
        .map(|i| {
            String::from_utf8_lossy(&buf[..i])
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase().strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0)
        })
        .unwrap_or(0);
    // E4-07: cap request bodies — a huge Content-Length must not force
    // unbounded memory growth (the dev server binds 0.0.0.0, so this is
    // reachable from the network).
    const MAX_BODY: usize = 10 * 1024 * 1024;
    if content_length > MAX_BODY {
        let resp = "HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes());
        return;
    }
    let mut have = buf.len().saturating_sub(header_len);
    while have < content_length {
        let n = stream.read(&mut tmp).unwrap_or(0);
        if n == 0 { break; }
        buf.extend_from_slice(&tmp[..n]);
        have += n;
    }

    let head = String::from_utf8_lossy(&buf[..header_len.min(buf.len())]).to_string();
    let body = buf[header_len.min(buf.len())..].to_vec();

    let first_line = head.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    let method = if parts.len() >= 1 { parts[0] } else { "GET" };
    let path = if parts.len() >= 2 { parts[1] } else { "/" };

    // §7d: middleware runs FIRST, before any dispatch — API routes AND
    // static pages are both gated. A matched middleware that returns
    // redirect()/respond() short-circuits; next()/nomatch continue to the
    // normal dispatch below. A middleware error is fail-closed (500), never
    // a silent pass-through.
    if let Some(mw_file) = middleware {
        match eval_middleware(out_dir, mw_file, method, path, &head, &body) {
            MiddlewareOutcome::Redirect { url, status } => {
                // F2: the middleware-controlled url lands in a raw HTTP
                // header line. Reject embedded CR/LF outright — a literal
                // newline here is header injection, never a legitimate
                // Location. Fail closed (500) rather than emitting a crafted
                // header block.
                if url.contains('\r') || url.contains('\n') {
                    let resp = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
                    let _ = stream.write_all(resp.as_bytes());
                    return;
                }
                let resp = format!(
                    "HTTP/1.1 {} {}\r\nLocation: {}\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 0\r\n\r\n",
                    status,
                    http_status_text(status),
                    url
                );
                let _ = stream.write_all(resp.as_bytes());
                return;
            }
            MiddlewareOutcome::Respond {
                status,
                headers,
                body_bytes,
            } => {
                let mut head_out = format!(
                    "HTTP/1.1 {} {}\r\nAccess-Control-Allow-Origin: *\r\n",
                    status,
                    http_status_text(status)
                );
                for (k, v) in &headers {
                    let lower = k.to_ascii_lowercase();
                    if lower == "content-length" || lower == "transfer-encoding" || lower == "connection" {
                        continue;
                    }
                    // F2: defensive — a header name or value with CR/LF is
                    // header injection; never emit it.
                    if k.contains('\r') || k.contains('\n') || v.contains('\r') || v.contains('\n') {
                        continue;
                    }
                    head_out.push_str(&format!("{}: {}\r\n", k, v));
                }
                head_out.push_str(&format!("Content-Length: {}\r\n\r\n", body_bytes.len()));
                let _ = stream.write_all(head_out.as_bytes());
                let _ = stream.write_all(&body_bytes);
                return;
            }
            MiddlewareOutcome::Error => {
                let resp = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
                let _ = stream.write_all(resp.as_bytes());
                return;
            }
            MiddlewareOutcome::Pass => {}
        }
    }

    // §7b: API routes dispatch FIRST — an /api/* path is never a static file.
    if let Some(api) = api_routes.iter().find(|a| a.path == path) {
        handle_api_route(out_dir, api, method, &head, &body, stream);
        return;
    }

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
                // E4-02: never join attacker-controlled path segments —
                // normalize lexically (resolve . and .. without following
                // symlinks) and 404 if any .. escapes the dist root.
                match safe_static_path(out_dir, path) {
                    Some(p) => p,
                    None => {
                        let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                        let _ = stream.write_all(resp.as_bytes());
                        return;
                    }
                }
            };

            // E4-01: server-side modules (_server/…) and api/ files are NEVER
            // served statically — they are only reachable through the API
            // dispatcher, so the baked env snapshot (SESSION_SECRET, API
            // keys) can never be downloaded.
            //
            // F1: compare case-insensitively — macOS APFS and Windows NTFS
            // are case-insensitive by default, so /_SERVER/middleware.mjs
            // would resolve to dist/_server/middleware.mjs on those hosts.
            if let Ok(rel) = file_path.strip_prefix(out_dir) {
                let first = rel
                    .components()
                    .next()
                    .and_then(|c| c.as_os_str().to_str())
                    .map(|s| s.to_ascii_lowercase());
                if matches!(first.as_deref(), Some("_server") | Some("api")) {
                    let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                    let _ = stream.write_all(resp.as_bytes());
                    return;
                }
            }

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

/// §7d dev-server middleware evaluation. Same pattern as handle_api_route: a
/// fresh node process per request imports the compiled module and runs it
/// against the exact contract the adapter uses. The module exports
/// `middleware`, `matcher`, and the canonical `__matchPath`; the script does
/// the matching HERE in one place (the regexes are baked into the compiled
/// module — no second matcher implementation in Rust).
///
/// Outcomes are fail-closed: an import failure, an exception, or a result
/// that is not one of the three sanctioned shapes becomes Error (500), never
/// a silent pass-through. nomatch/next both mean "continue to dispatch" —
/// the caller can't distinguish and doesn't need to.
#[derive(Debug)]
enum MiddlewareOutcome {
    Pass,
    Redirect { url: String, status: u64 },
    Respond { status: u64, headers: Vec<(String, String)>, body_bytes: Vec<u8> },
    Error,
}

fn eval_middleware(
    out_dir: &Path,
    mw_file: &str,
    method: &str,
    request_path: &str,
    head: &str,
    body: &[u8],
) -> MiddlewareOutcome {
    let mjs_path = out_dir.join(mw_file);
    let abs = match std::fs::canonicalize(&mjs_path) {
        Ok(p) => p,
        Err(_) => return MiddlewareOutcome::Error,
    };

    let headers: Vec<String> = head
        .lines()
        .skip(1)
        .filter(|l| {
            let lower = l.to_ascii_lowercase();
            !lower.starts_with("content-length:") && !l.trim().is_empty()
        })
        .map(|l| l.to_string())
        .collect();

    let payload = serde_json::json!({
        "method": method,
        "path": request_path,
        "headers": headers,
        "bodyBase64": if body.is_empty() { serde_json::Value::Null } else {
            serde_json::Value::String(base64_encode(body))
        },
    })
    .to_string();

    // F3: the interpreter communicates its envelope through a temp file
    // passed on argv, NOT stdout. A middleware that console.log()s debug
    // output must not corrupt (or, worse, be able to forge) the envelope
    // channel — with stdout as the channel, a stray `console.log` broke
    // every matched request and a crafted write could fake a pass.
    //
    // F3-hardening: Rust creates the file FIRST with O_EXCL + 0600. The path
    // is therefore a real empty file owned by this process (the temp dir's
    // sticky bit stops anyone else deleting/replacing it) before node ever
    // touches it — no attacker-planted symlink can redirect node's write.
    // node then overwrites the file; Rust reads and removes it.
    let envelope_path = std::env::temp_dir().join(format!(
        "marisjs-mw-{}-{}.json",
        std::process::id(),
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()
    ));
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let created = opts.open(&envelope_path);
        if created.is_err() {
            // Path collision (nanos makes it practically impossible) — fail
            // closed rather than risk reading a stale/foreign file.
            return MiddlewareOutcome::Error;
        }
    }

    // The contract interpreter: build the Request exactly like the api
    // dispatcher, then match and run. The three shapes are the ONLY results
    // the middleware can produce (validator-guaranteed at build); anything
    // else — including an object with an unknown __m — is fail-closed.
    let script = r#"
import { readFileSync, writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
const out = process.argv[2];
const emit = (obj) => { writeFileSync(out, JSON.stringify(obj)); process.exit(0); };
const input = JSON.parse(readFileSync(0, 'utf8'));
let mod;
try {
  mod = await import(pathToFileURL(process.argv[1]).href);
} catch (e) {
  emit({ kind: 'error' });
}
if (typeof mod.__matchPath !== 'function' || typeof mod.middleware !== 'function') {
  emit({ kind: 'error' });
}
if (!mod.__matchPath(input.path)) {
  emit({ kind: 'nomatch' });
}
const headers = {};
for (const line of input.headers) {
  const i = line.indexOf(':');
  if (i > 0) headers[line.slice(0, i).trim()] = line.slice(i + 1).trim();
}
const body = input.bodyBase64 ? Buffer.from(input.bodyBase64, 'base64') : undefined;
const req = new Request('http://localhost' + input.path, { method: input.method, headers, body });
try {
  const result = await mod.middleware(req);
  if (result && result.__m === 'next') {
    emit({ kind: 'next' });
  } else if (result && result.__m === 'redirect') {
    emit({ kind: 'redirect', url: String(result.url), status: Number(result.status) || 302 });
  } else if (result && result.__m === 'respond' && result.response instanceof Response) {
    const buf = Buffer.from(await result.response.arrayBuffer());
    const outHeaders = [];
    result.response.headers.forEach((v, k) => { outHeaders.push([k, v]); });
    emit({ kind: 'respond', status: result.response.status, headers: outHeaders, body: buf.toString('base64') });
  } else {
    // Not one of the three sanctioned shapes — fail closed.
    emit({ kind: 'error' });
  }
} catch (e) {
  emit({ kind: 'error' });
}
"#;

    let child = ProcCommand::new("node")
        .arg("--input-type=module")
        .arg("-e")
        .arg(script)
        .arg(abs.to_str().unwrap_or(""))
        .arg(&envelope_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let Ok(mut child) = child else {
        return MiddlewareOutcome::Error;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.as_bytes());
    }
    let out = child.wait_with_output();
    let Ok(out) = out else {
        return MiddlewareOutcome::Error;
    };

    let stderr_text = String::from_utf8_lossy(&out.stderr);
    if !stderr_text.trim().is_empty() {
        eprintln!("  [middleware] node stderr: {}", stderr_text.trim());
    }

    let response: serde_json::Value = std::fs::read_to_string(&envelope_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::json!({ "kind": "error" }));
    let _ = std::fs::remove_file(&envelope_path);
    match response.get("kind").and_then(|v| v.as_str()).unwrap_or("error") {
        "nomatch" | "next" => MiddlewareOutcome::Pass,
        "redirect" => MiddlewareOutcome::Redirect {
            url: response.get("url").and_then(|v| v.as_str()).unwrap_or("/").to_string(),
            status: response.get("status").and_then(|v| v.as_u64()).unwrap_or(302),
        },
        "respond" => {
            let status = response.get("status").and_then(|v| v.as_u64()).unwrap_or(200);
            let mut headers = Vec::new();
            if let Some(arr) = response.get("headers").and_then(|v| v.as_array()) {
                for pair in arr {
                    if let Some(arr) = pair.as_array() {
                        if arr.len() == 2 {
                            if let (Some(k), Some(v)) = (arr[0].as_str(), arr[1].as_str()) {
                                headers.push((k.to_string(), v.to_string()));
                            }
                        }
                    }
                }
            }
            let body_bytes = base64_decode(response.get("body").and_then(|v| v.as_str()).unwrap_or(""));
            MiddlewareOutcome::Respond { status, headers, body_bytes }
        }
        _ => MiddlewareOutcome::Error,
    }
}

/// §7b dev-server dispatch: spawns Node with the compiled route module,
/// hands it a standard Web Request (constructed from the raw HTTP line) via
/// stdin JSON, and writes the returned Web Response back out. This exercises
/// the exact handler contract used by adapter-node: `await handler(req)` →
/// `Response`. The handler import runs in a fresh process per request — no
/// cross-request state, no caching surprises (dev only; adapter-node keeps
/// modules loaded, see its server.mjs).
fn handle_api_route(
    out_dir: &Path,
    api: &ApiRoute,
    method: &str,
    head: &str,
    body: &[u8],
    stream: &mut std::net::TcpStream,
) {
    // 405 for methods the file does not export — supported methods ARE the
    // export list, so this is decided entirely from the manifest.
    if !api.methods.iter().any(|m| m == method) {
        let allowed = api.methods.join(", ");
        let resp = format!(
            "HTTP/1.1 405 Method Not Allowed\r\nAllow: {}\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 0\r\n\r\n",
            allowed
        );
        let _ = stream.write_all(resp.as_bytes());
        return;
    }

    let mjs_path = out_dir.join(&api.file);
    let abs = match std::fs::canonicalize(&mjs_path) {
        Ok(p) => p,
        Err(_) => {
            let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes());
            return;
        }
    };

    // Forward the request headers (minus the request line and hop-by-hop
    // framing) so the handler sees the real Host, Content-Type, etc.
    let headers: Vec<String> = head
        .lines()
        .skip(1)
        .filter(|l| {
            let lower = l.to_ascii_lowercase();
            !lower.starts_with("content-length:") && !l.trim().is_empty()
        })
        .map(|l| l.to_string())
        .collect();

    let payload = serde_json::json!({
        "method": method,
        "path": head.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("/"),
        "headers": headers,
        "bodyBase64": if body.is_empty() { serde_json::Value::Null } else {
            serde_json::Value::String(base64_encode(body))
        },
    })
    .to_string();

    let script = r#"
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
const input = JSON.parse(readFileSync(0, 'utf8'));
const handler = (await import(pathToFileURL(process.argv[1]).href))[input.method];
if (!handler) {
  console.log(JSON.stringify({ status: 405, headers: {}, body: '' }));
  process.exit(0);
}
const headers = {};
for (const line of input.headers) {
  const i = line.indexOf(':');
  if (i > 0) headers[line.slice(0, i).trim()] = line.slice(i + 1).trim();
}
const body = input.bodyBase64 ? Buffer.from(input.bodyBase64, 'base64') : undefined;
const req = new Request('http://localhost' + input.path, { method: input.method, headers, body });
try {
  const res = await handler(req);
  const buf = Buffer.from(await res.arrayBuffer());
  // E4-06: headers as an ARRAY of [k, v] pairs — an object would collapse
  // multiple Set-Cookie headers into the last one and silently drop
  // security cookies.
  const outHeaders = [];
  res.headers.forEach((v, k) => { outHeaders.push([k, v]); });
  console.log(JSON.stringify({ status: res.status, headers: outHeaders, body: buf.toString('base64') }));
} catch (e) {
  console.log(JSON.stringify({ status: 500, headers: { 'content-type': 'text/plain;charset=UTF-8' }, body: Buffer.from(String((e && e.stack) || e)).toString('base64') }));
  process.exit(0);
}
"#;

    let child = ProcCommand::new("node")
        .arg("--input-type=module")
        .arg("-e")
        .arg(script)
        .arg(abs.to_str().unwrap_or(""))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let Ok(mut child) = child else {
        let resp = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes());
        return;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.as_bytes());
    }
    let out = child.wait_with_output();
    let Ok(out) = out else {
        let resp = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes());
        return;
    };

    let stderr_text = String::from_utf8_lossy(&out.stderr);
    if !stderr_text.trim().is_empty() {
        eprintln!("  [api {} {}] node stderr: {}", method, api.path, stderr_text.trim());
    }

    let response: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::json!({
        "status": 500, "headers": {}, "body": ""
    }));
    let status = response.get("status").and_then(|v| v.as_u64()).unwrap_or(500);
    let headers_arr = response.get("headers").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let body_b64 = response.get("body").and_then(|v| v.as_str()).unwrap_or("");

    let mut head_out = format!(
        "HTTP/1.1 {} {}\r\nAccess-Control-Allow-Origin: *\r\n",
        status,
        http_status_text(status)
    );
    for pair in &headers_arr {
        if let Some(arr) = pair.as_array() {
            if arr.len() == 2 {
                if let (Some(k), Some(v)) = (arr[0].as_str(), arr[1].as_str()) {
                    // Never allow the handler to smuggle framing headers.
                    let lower = k.to_ascii_lowercase();
                    if lower == "content-length" || lower == "transfer-encoding" || lower == "connection" {
                        continue;
                    }
                    head_out.push_str(&format!("{}: {}\r\n", k, v));
                }
            }
        }
    }
    let body_bytes = base64_decode(body_b64);
    head_out.push_str(&format!("Content-Length: {}\r\n\r\n", body_bytes.len()));
    let _ = stream.write_all(head_out.as_bytes());
    let _ = stream.write_all(&body_bytes);
}

fn http_status_text(status: u64) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18) as usize & 63] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(triple >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[triple as usize & 63] as char } else { '=' });
    }
    out
}

fn base64_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for c in s.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            '=' => break,
            _ => continue,
        };
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8 & 0xFF);
        }
    }
    out
}

#[cfg(test)]
mod mime_tests {
    use super::{api_file_to_route, mime_for_ext, parse_dotenv};

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

    // ── §7a dotenv tokenizer ────────────────────────────────────────────

    #[test]
    fn dotenv_parses_plain_values() {
        let env = parse_dotenv("KEY=value\nEMPTY=\nNUMBER=42");
        assert_eq!(env.len(), 3);
        assert_eq!(env[0], ("KEY".into(), "value".into()));
        assert_eq!(env[1], ("EMPTY".into(), "".into()));
        assert_eq!(env[2], ("NUMBER".into(), "42".into()));
    }

    #[test]
    fn dotenv_skips_comments_and_blanks() {
        let env = parse_dotenv("# leading comment\n\nKEY=value # trailing comment\n  \n# another\nA=B");
        assert_eq!(env, vec![("KEY".into(), "value".into()), ("A".into(), "B".into())]);
    }

    #[test]
    fn dotenv_parses_quoted_values() {
        let env = parse_dotenv(
            "SINGLE='hello world'\nDOUBLE=\"a \\\"quoted\\\" value\"\nESCAPED=\"back\\\\slash\"",
        );
        assert_eq!(env.len(), 3);
        assert_eq!(env[0], ("SINGLE".into(), "hello world".into()));
        assert_eq!(env[1], ("DOUBLE".into(), "a \"quoted\" value".into()));
        assert_eq!(env[2], ("ESCAPED".into(), "back\\slash".into()));
    }

    #[test]
    fn dotenv_trims_whitespace_around_key_and_value() {
        let env = parse_dotenv("  PADDED  =   spaced value  ");
        assert_eq!(env, vec![("PADDED".into(), "spaced value".into())]);
    }

    // ── §7b api file routing ────────────────────────────────────────────

    #[test]
    fn api_routes_map_file_based_like_pages() {
        assert_eq!(api_file_to_route(std::path::Path::new("api/checkout.ts")), "/api/checkout");
        assert_eq!(api_file_to_route(std::path::Path::new("api/billing/charge.ts")), "/api/billing/charge");
        assert_eq!(api_file_to_route(std::path::Path::new("api/Index.ts")), "/api");
        assert_eq!(api_file_to_route(std::path::Path::new("api/webhooks/stripe.ts")), "/api/webhooks/stripe");
    }
}
