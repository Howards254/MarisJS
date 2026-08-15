//! Wraps an existing TSX parser (SWC) and produces this framework's typed AST from a .tsx file. Does not validate framework rules — only produces a tree.

use std::path::Path;
use std::sync::Arc;

use swc_common::comments::SingleThreadedComments;
use swc_common::input::StringInput;
use swc_common::{BytePos, FileName, SourceMap, Span, Spanned};
use swc_common::SourceMapper;
use swc_ecma_ast::*;
use swc_ecma_parser::{Parser, Syntax, TsConfig};
use swc_ecma_visit::{Visit, VisitWith};

#[derive(Debug, Clone, PartialEq)]
pub enum RunsOn {
    Client,
    Server,
    Api,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExportKind {
    NamedFunction,
    DefaultExport,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportInfo {
    pub name: String,
    pub kind: ExportKind,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeAnnotation {
    Named(String),
    Any,
    Untyped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PropsInfo {
    pub name: String,
    pub type_annotation: TypeAnnotation,
    pub is_destructured: bool,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportInfo {
    pub source: String,
    pub imported_names: Vec<String>,
    pub line: usize,
    pub column: usize,
    pub is_css: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TopLevelBinding {
    Let {
        name: String,
        exported: bool,
        line: usize,
        column: usize,
    },
    Var {
        name: String,
        exported: bool,
        line: usize,
        column: usize,
    },
    Const {
        name: String,
        exported: bool,
        line: usize,
        column: usize,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum JsxExpression {
    AndConditional,
    InlineMap,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsxExprInfo {
    pub kind: JsxExpression,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BodyStmtKind {
    Signal,
    DerivedConst,
    EventHandler,
    Return,
    Let,
    Var,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BodyStmt {
    pub kind: BodyStmtKind,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SignalKind {
    Signal,
    Computed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SignalDecl {
    pub name: String,
    pub kind: SignalKind,
    pub initial_value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JsxAttrValue {
    String(String),
    Expr(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsxAttr {
    pub name: String,
    pub value: JsxAttrValue,
    /// AST-based: true when the attribute's expression contains an env()
    /// call anywhere within it (direct call, chained method, template
    /// literal interpolation, nested expression). Set by the parser from the
    /// raw syntax tree — never a text-shape heuristic.
    pub contains_env_call: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JsxNode {
    Element {
        tag: String,
        attrs: Vec<JsxAttr>,
        children: Vec<JsxNode>,
        is_hydrate_root: bool,
        is_component: bool,
    },
    Text(String),
    Expr(String),
    Conditional {
        test: String,
        cons: Box<JsxNode>,
        alt: Box<JsxNode>,
    },
    ForEach {
        each: String,
        key_fn: String,
        item_param: String,
        body: Box<JsxNode>,
        /// Block-local declarations (function/const) inside the For item
        /// arrow's block body, to be emitted inside the render function _rX.
        /// Empty for expression-bodied arrows.
        for_body_decls: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComponentFile {
    pub filename: String,
    pub runs_on: Option<RunsOn>,
    pub runs_on_count: usize,
    pub runs_on_line: usize,
    pub runs_on_column: usize,
    pub exports: Vec<ExportInfo>,
    pub props: Option<PropsInfo>,
    pub imports: Vec<ImportInfo>,
    pub top_level_bindings: Vec<TopLevelBinding>,
    pub jsx_expressions: Vec<JsxExprInfo>,
    pub has_data_call: bool,
    pub data_call_line: usize,
    pub data_call_column: usize,
    pub has_env_call: bool,
    pub env_call_line: usize,
    pub env_call_column: usize,
    /// The string-literal keys referenced by env('KEY') call sites. Codegen
    /// bakes ONLY these keys into the module-scope env helper — never the
    /// whole process environment (a full snapshot would leak unrelated
    /// shell secrets into dist output).
    pub env_call_keys: Vec<String>,
    /// session()/setSession() call-site detection — the same mechanism as
    /// env()/data(), so the validator's CLIENT_SESSION_ACCESS rejection and
    /// codegen's session-block emission share one AST-based source of truth.
    pub has_session_call: bool,
    pub session_call_line: usize,
    pub session_call_column: usize,
    pub body_stmts: Vec<BodyStmt>,
    pub has_component_body: bool,
    pub render_tree: Option<JsxNode>,
    pub signals: Vec<SignalDecl>,
    pub handler_decls: Vec<String>,
    pub handler_has_jsx: Vec<bool>,
    pub derived_consts: Vec<String>,
    pub module_consts: Vec<String>,
    /// Ordered top-level statements (source text, TS annotations stripped) —
    /// EVERYTHING at module scope except imports, exported function
    /// declarations (see exported_fn_sources), and the directive comment.
    /// Component codegen emits module_consts (const-only view); the API
    /// codegen path emits this full ordered list (module-level consts,
    /// non-exported helper functions, and other module-level statements like
    /// setup code), because api files are ordinary TypeScript modules.
    pub module_statements: Vec<String>,
    /// Stripped source text of every named function export (name, source) —
    /// used by the API codegen path, which emits handler functions verbatim.
    /// TS annotations are stripped via the same splice mechanism as
    /// handler_decls.
    pub exported_fn_sources: Vec<(String, String)>,
    pub unsupported_errors: Vec<ParserError>,
}

impl ComponentFile {
    pub fn new(filename: impl Into<String>) -> Self {
        Self {
            filename: filename.into(),
            runs_on: None,
            runs_on_count: 0,
            runs_on_line: 0,
            runs_on_column: 0,
            exports: Vec::new(),
            props: None,
            imports: Vec::new(),
            top_level_bindings: Vec::new(),
            jsx_expressions: Vec::new(),
            has_data_call: false,
            data_call_line: 0,
            data_call_column: 0,
            has_env_call: false,
            env_call_line: 0,
            env_call_column: 0,
            env_call_keys: Vec::new(),
            has_session_call: false,
            session_call_line: 0,
            session_call_column: 0,
            body_stmts: Vec::new(),
            has_component_body: false,
            render_tree: None,
            signals: Vec::new(),
            handler_decls: Vec::new(),
            handler_has_jsx: Vec::new(),
            derived_consts: Vec::new(),
            module_consts: Vec::new(),
            module_statements: Vec::new(),
            exported_fn_sources: Vec::new(),
            unsupported_errors: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParserError {
    pub code: &'static str,
    pub message: String,
    pub fix_hint: &'static str,
}

#[derive(Debug)]
pub struct ParseError {
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ParseError {}

fn pos(cm: &SourceMap, span: Span) -> (usize, usize) {
    if span.is_dummy() {
        return (0, 0);
    }
    let loc = cm.lookup_char_pos(span.lo);
    (loc.line, loc.col_display)
}

/// Parses a `.tsx` file using SWC and walks the resulting AST to produce this framework's typed AST.
/// Every field in `ComponentFile` comes from a real AST node with a real Span, so line/column
/// positions are populated from actual parser output.
pub fn parse_component_file(path: &str) -> Result<ComponentFile, ParseError> {
    let cm: Arc<SourceMap> = Default::default();
    let fm = cm
        .load_file(Path::new(path))
        .map_err(|e| ParseError {
            message: format!("Failed to read '{}': {}", path, e),
        })?;

    let filename = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());

    let syntax = Syntax::Typescript(TsConfig {
        tsx: true,
        decorators: false,
        dts: false,
        ..Default::default()
    });

    let comments = SingleThreadedComments::default();
    let input = StringInput::from(&*fm);

    let mut parser = Parser::new(syntax, input, Some(&comments));

    let module = parser
        .parse_module()
        .map_err(|e| ParseError {
            message: format!("Failed to parse '{}': {:?}", path, e),
        })?;

    let first_code_pos = module
        .body
        .first()
        .map(|item| item.span().lo);

    let mut file = ComponentFile::new(filename);

    extract_runs_on_from_comments(&cm, &comments, first_code_pos, &mut file);
    walk_module(&cm, &module, &mut file);

    Ok(file)
}

/// Like `parse_component_file` but accepts raw source text instead of reading
/// from disk. The `filename` is used in diagnostics (line/column references
/// still come from the real AST spans within the source).
pub fn parse_component_source(source: &str, filename: &str) -> Result<ComponentFile, ParseError> {
    let cm: Arc<SourceMap> = Default::default();
    let fm = cm.new_source_file(FileName::Custom(filename.to_string()).into(), source.to_string());

    let syntax = Syntax::Typescript(TsConfig {
        tsx: true,
        decorators: false,
        dts: false,
        ..Default::default()
    });

    let comments = SingleThreadedComments::default();
    let input = StringInput::from(&*fm);

    let mut parser = Parser::new(syntax, input, Some(&comments));

    let module = parser
        .parse_module()
        .map_err(|e| ParseError {
            message: format!("Failed to parse '{}': {:?}", filename, e),
        })?;

    let first_code_pos = module
        .body
        .first()
        .map(|item| item.span().lo);

    let mut file = ComponentFile::new(filename.to_string());

    extract_runs_on_from_comments(&cm, &comments, first_code_pos, &mut file);
    walk_module(&cm, &module, &mut file);

    Ok(file)
}

fn extract_runs_on_from_comments(
    cm: &SourceMap,
    comments: &SingleThreadedComments,
    first_code_pos: Option<BytePos>,
    file: &mut ComponentFile,
) {
    let (leading, trailing) = comments.clone().take_all();
    let leading = leading.borrow();
    let trailing = trailing.borrow();

    for comment_list in leading.values().chain(trailing.values()) {
        for comment in comment_list.iter() {
            if let Some(code_start) = first_code_pos {
                if comment.span.lo >= code_start {
                    continue;
                }
            }
            if let Some(runs_on) = parse_runs_on_comment(&comment.text) {
                let loc = cm.lookup_char_pos(comment.span.lo);
                file.runs_on_count += 1;
                if file.runs_on.is_none() {
                    file.runs_on = Some(runs_on);
                    file.runs_on_line = loc.line;
                    file.runs_on_column = loc.col_display;
                }
            }
        }
    }
}

fn parse_runs_on_comment(text: &str) -> Option<RunsOn> {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("@runsOn") {
        let rest = rest.trim();
        // Exact match only (E4-11): a misspelled or prefix value like
        // "@runsOn apiserver" must NOT silently compile as "api" — an
        // unknown value is a directive error, surfaced by the validator
        // (RUNS_ON_DIRECTIVE), never a silent reclassification.
        if rest == "client" {
            return Some(RunsOn::Client);
        } else if rest == "server" {
            return Some(RunsOn::Server);
        } else if rest == "api" {
            return Some(RunsOn::Api);
        }
    }
    None
}

/// AST-based reactivity detection: does `expr` read a known signal/computed
/// identifier's `.value` property (or a signal drilled through the component's
/// `props`, e.g. `props.tasks.value`)?
///
/// The expression is actually PARSED and its MemberExprs are walked, instead
/// of substring-matching the source text, so the classic false positives are
/// gone: a plain object's unrelated `.value` field (`config.value` where
/// `config` is not a signal) and `.value` inside string literals
/// (`'a.value.b'`) are correctly NOT reactive. Reads still detected:
///   - `count.value` / `total.value` — identifier in `signal_names`
///   - `store.list.count.value` — chain ending in a signal identifier
///   - `props.tasks.value` / `props['tasks'].value` — drilled signal prop
///   - nested reads anywhere: `arr[count.value]`, `Math.max(a.value, b.value)`
///
/// If the text cannot be parsed as an expression (JSX conditionals are
/// separate JsxNode kinds, so this shouldn't happen for attr/text exprs),
/// falls back to the historical text-containment check so behavior can never
/// silently regress.
pub fn expr_reads_signal_value(expr: &str, signal_names: &[String], props_param: &str) -> bool {
    let syntax = Syntax::Typescript(TsConfig {
        tsx: true,
        ..Default::default()
    });
    let mut parser = Parser::new(
        syntax,
        StringInput::new(expr, BytePos(0), BytePos(expr.len() as u32)),
        None,
    );
    match parser.parse_expr() {
        Ok(parsed) => {
            let mut visitor = SignalValueReader {
                names: signal_names,
                props_param,
                found: false,
            };
            parsed.visit_with(&mut visitor);
            visitor.found
        }
        Err(_) => {
            signal_names.iter().any(|name| expr.contains(&format!("{}.value", name)))
                || expr.contains(".value")
        }
    }
}

struct SignalValueReader<'a> {
    names: &'a [String],
    props_param: &'a str,
    found: bool,
}

impl Visit for SignalValueReader<'_> {
    fn visit_member_expr(&mut self, n: &MemberExpr) {
        if is_value_prop(&n.prop) && chain_reads_signal(&n.obj, self.names, self.props_param) {
            self.found = true;
        }
        n.visit_children_with(self);
    }
}

fn is_value_prop(prop: &MemberProp) -> bool {
    match prop {
        MemberProp::Ident(id) => id.sym == "value",
        MemberProp::Computed(computed) => {
            matches!(&*computed.expr, Expr::Lit(Lit::Str(s)) if s.value == "value")
        }
        _ => false,
    }
}

/// The object chain immediately left of a `.value` member. Reactive when the
/// chain ultimately originates from the component's `props` parameter at ANY
/// depth (drilled signal — the framework contract is "pass the signal by
/// reference, read .value in the child", so `props.<name>.value` is a signal
/// read by definition), or when it ends in a known signal/computed identifier.
///
/// The walk is fully general: it iterates the entire member-access chain to
/// its root identifier, so depth never matters — `props.count.value` (1 level),
/// `props.nested.count.value` (2), `props.deeply.nested.count.value` (3), and
/// arbitrarily many more all resolve identically. There is no fixed-depth
/// ceiling (regression: depth ≥ 2 silently rendered once and never updated).
fn chain_reads_signal(obj: &Expr, names: &[String], props_param: &str) -> bool {
    // The identifier of the LAST member before `.value` (None when the chain
    // is a bare identifier like `count.value`). Kept so `store.list.count.value`
    // still reacts when `count` is a known signal (historical behavior).
    let mut rightmost: Option<String> = None;
    let mut cur = obj;
    loop {
        match cur {
            Expr::Ident(id) => {
                if id.sym == props_param {
                    return true;
                }
                if names.iter().any(|n| n == id.sym.as_str()) {
                    return true;
                }
                return rightmost
                    .as_ref()
                    .map_or(false, |r| names.iter().any(|n| n == r));
            }
            Expr::Member(m) => {
                match &m.prop {
                    MemberProp::Ident(id) => {
                        if rightmost.is_none() {
                            rightmost = Some(id.sym.to_string());
                        }
                    }
                    MemberProp::Computed(computed) => {
                        match &*computed.expr {
                            Expr::Lit(Lit::Str(s)) => {
                                if rightmost.is_none() {
                                    rightmost = Some(s.value.to_string());
                                }
                            }
                            // A non-string index can't contribute a name, but
                            // it must not defeat the props-root check either
                            // (`props[idx].value` still originates from props).
                            _ => {}
                        }
                    }
                    MemberProp::PrivateName(_) => return false,
                }
                cur = &m.obj;
            }
            _ => return false,
        }
    }
}

struct Extractor<'a> {
    cm: &'a SourceMap,
    file: &'a mut ComponentFile,
    depth: usize,
}

fn walk_module(cm: &SourceMap, module: &Module, file: &mut ComponentFile) {
    let mut extractor = Extractor {
        cm,
        file,
        depth: 0,
    };
    module.visit_with(&mut extractor);
}

impl Visit for Extractor<'_> {
    fn visit_export_decl(&mut self, n: &ExportDecl) {
        let (line, col) = pos(self.cm, n.span);

        match &n.decl {
            Decl::Fn(fn_decl) => {
                self.file.exports.push(ExportInfo {
                    name: fn_decl.ident.sym.to_string(),
                    kind: ExportKind::NamedFunction,
                    line,
                    column: col,
                });
                if let Ok(src) = self.cm.span_to_snippet(n.span) {
                    let stripped = strip_ts_annotations(&src, n.span, fn_decl);
                    self.file
                        .exported_fn_sources
                        .push((fn_decl.ident.sym.to_string(), stripped));
                }
                self.extract_fn_props(
                    &fn_decl.function,
                    fn_decl.ident.sym.to_string(),
                );
            }
            Decl::Var(var_decl) => {
                let (vline, vcol) = pos(self.cm, var_decl.span);
                for decl in &var_decl.decls {
                    if let Pat::Ident(ident) = &decl.name {
                        match var_decl.kind {
                            VarDeclKind::Let => self.file.top_level_bindings.push(
                                TopLevelBinding::Let {
                                    name: ident.id.sym.to_string(),
                                    exported: true,
                                    line: vline,
                                    column: vcol,
                                },
                            ),
                            VarDeclKind::Var => self.file.top_level_bindings.push(
                                TopLevelBinding::Var {
                                    name: ident.id.sym.to_string(),
                                    exported: true,
                                    line: vline,
                                    column: vcol,
                                },
                            ),
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }

        n.visit_children_with(self);
    }

    fn visit_export_default_decl(&mut self, n: &ExportDefaultDecl) {
        let (line, col) = pos(self.cm, n.span);

        if let DefaultDecl::Fn(fn_expr) = &n.decl {
            let name = fn_expr
                .ident
                .as_ref()
                .map(|i| i.sym.to_string())
                .unwrap_or_else(|| "default".to_string());
            self.file.exports.push(ExportInfo {
                name: name.clone(),
                kind: ExportKind::DefaultExport,
                line,
                column: col,
            });
            self.extract_fn_props(&fn_expr.function, name);
        }

        n.visit_children_with(self);
    }

    fn visit_fn_decl(&mut self, n: &FnDecl) {
        self.depth += 1;
        n.visit_children_with(self);
        self.depth -= 1;
    }

    fn visit_function(&mut self, n: &Function) {
        self.depth += 1;
        n.visit_children_with(self);
        self.depth -= 1;
    }

    fn visit_arrow_expr(&mut self, n: &ArrowExpr) {
        self.depth += 1;
        n.visit_children_with(self);
        self.depth -= 1;
    }

    /// Ordered module-scope capture for API files: every top-level statement
/// except imports, exported function declarations, and the directive
/// comment, in source order, with TS annotations stripped. This is what the
/// api codegen emits verbatim — api files are ordinary TypeScript modules,
/// and dropping module-level statements (setup code, helper functions,
/// consts) would silently break handlers that reference them.
fn visit_module(&mut self, n: &Module) {
    for item in &n.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::Import(_)) => continue,
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                // Exported functions are captured separately (they become the
                // route handlers); exported consts/lets/vars are captured by
                // visit_var_decl into module_consts/top_level_bindings and are
                // ALSO part of the ordered statement list here — the api
                // codegen emits module_statements and skips module_consts, so
                // there is no double emission.
                if matches!(export.decl, Decl::Fn(_)) {
                    continue;
                }
                if let Decl::Var(var) = &export.decl {
                    // Keep the `export` keyword so the emitted module still
                    // exports the binding (export const X = 1 must stay
                    // importable by sibling modules).
                    if let Ok(src) = self.cm.span_to_snippet(export.span) {
                        let stripped = strip_var_ts(&src, export.span, var);
                        self.file.module_statements.push(stripped);
                    }
                    continue;
                }
                continue;
            }
            ModuleItem::ModuleDecl(_) => continue,
            ModuleItem::Stmt(stmt) => match stmt {
                Stmt::Decl(Decl::Var(var)) => {
                    if let Ok(src) = self.cm.span_to_snippet(var.span) {
                        let stripped = strip_var_ts(&src, var.span, var);
                        self.file.module_statements.push(stripped);
                    }
                }
                Stmt::Decl(Decl::Fn(fn_decl)) => {
                    if let Ok(src) = self.cm.span_to_snippet(fn_decl.span()) {
                        let stripped = strip_ts_annotations(&src, fn_decl.span(), fn_decl);
                        self.file.module_statements.push(stripped);
                    }
                }
                _ => {
                    if let Ok(src) = self.cm.span_to_snippet(stmt.span()) {
                        self.file.module_statements.push(src);
                    }
                }
            },
        }
    }

    n.visit_children_with(self);
}

fn visit_import_decl(&mut self, n: &ImportDecl) {
    let (line, col) = pos(self.cm, n.span);
    let source = n.src.value.to_string();
    let is_css = source.ends_with(".css") || source.ends_with(".CSS");

    let mut names = Vec::new();

    for spec in &n.specifiers {
        match spec {
            ImportSpecifier::Named(named) => {
                names.push(named.local.sym.to_string());
            }
            ImportSpecifier::Default(default) => {
                names.push(default.local.sym.to_string());
            }
            ImportSpecifier::Namespace(ns) => {
                names.push(ns.local.sym.to_string());
            }
        }
    }

    if is_css || !names.is_empty() {
        self.file.imports.push(ImportInfo {
            source,
            imported_names: names,
            line,
            column: col,
            is_css,
        });
    }

    n.visit_children_with(self);
}

    fn visit_var_decl(&mut self, n: &VarDecl) {
        if self.depth == 0 {
            let (line, col) = pos(self.cm, n.span);
            for decl in &n.decls {
                if let Pat::Ident(ident) = &decl.name {
                    match n.kind {
                        VarDeclKind::Let => self.file.top_level_bindings.push(
                            TopLevelBinding::Let {
                                name: ident.id.sym.to_string(),
                                exported: false,
                                line,
                                column: col,
                            },
                        ),
                        VarDeclKind::Var => self.file.top_level_bindings.push(
                            TopLevelBinding::Var {
                                name: ident.id.sym.to_string(),
                                exported: false,
                                line,
                                column: col,
                            },
                        ),
                        VarDeclKind::Const => {
                            // Module-level const (e.g. a shared config array/object):
                            // capture the source text (TS annotations stripped) the
                            // same way in-component derived consts are captured, so
                            // codegen can emit it at module scope of the output.
                            // The NAME is recorded too (TopLevelBinding::Const) so
                            // the validator can detect collisions with the emitted
                            // session/env runtime declarations.
                            if let Ok(src) = self.cm.span_to_snippet(n.span) {
                                let stripped = strip_var_ts(&src, n.span, n);
                                self.file.module_consts.push(stripped);
                            }
                            self.file.top_level_bindings.push(TopLevelBinding::Const {
                                name: ident.id.sym.to_string(),
                                exported: false,
                                line,
                                column: col,
                            });
                        }
                    }
                }
            }
        }

        self.depth += 1;
        n.visit_children_with(self);
        self.depth -= 1;
    }

    fn visit_jsx_expr_container(&mut self, n: &JSXExprContainer) {
        let (line, col) = pos(self.cm, n.span);

        match &n.expr {
            JSXExpr::Expr(expr) => {
                self.check_jsx_expr(expr, line, col);
            }
            JSXExpr::JSXEmptyExpr(_) => {}
        }

        n.visit_children_with(self);
    }

    fn visit_call_expr(&mut self, n: &CallExpr) {
        // The callee may hide behind cheap AST shapes that resolve to the
        // same target: a comma-sequence `(0, session)()` (the LAST element is
        // the actual call target) and, via visit_opt_chain_expr, `session?.()`.
        // Parenthesized callees (`(session)()`) unwrap first — swc keeps
        // ParenExpr nodes in the tree.
        let mut callee_expr: &Expr = match &n.callee {
            Callee::Expr(expr) => &**expr,
            _ => return,
        };
        while let Expr::Paren(paren) = callee_expr {
            callee_expr = &paren.expr;
        }
        let target: Option<&Ident> = match callee_expr {
            Expr::Ident(ident) => Some(ident),
            Expr::Seq(seq) => seq.exprs.last().and_then(|e| match &**e {
                Expr::Ident(ident) => Some(ident),
                _ => None,
            }),
            _ => None,
        };

        if let Some(ident) = target {
            let name = ident.sym.as_ref();
            let (line, col) = pos(self.cm, n.span);
            if name == "data" {
                self.file.has_data_call = true;
                self.file.data_call_line = line;
                self.file.data_call_column = col;
            }
            if name == "env" {
                self.file.has_env_call = true;
                self.file.env_call_line = line;
                self.file.env_call_column = col;
                // Record the referenced key when it is a string
                // literal — this is the bake-allowlist (see
                // env_call_keys). Dynamic keys (env(name)) are
                // allowed to compile but can never be baked; they
                // yield undefined.
                if let Some(arg) = n.args.first() {
                    if let Some(lit) = arg.expr.as_lit() {
                        if let swc_ecma_ast::Lit::Str(s) = lit {
                            self.file.env_call_keys.push(s.value.to_string());
                        }
                    }
                }
            }
            if name == "session" || name == "setSession" {
                self.file.has_session_call = true;
                self.file.session_call_line = line;
                self.file.session_call_column = col;
            }
        }

        n.visit_children_with(self);
    }

    /// `session?.()` parses as an optional chain whose base is the call
    /// expression — surface the base call to the normal detection so the
    /// boundary checks cannot be dodged by optional-call syntax.
    fn visit_opt_chain_expr(&mut self, n: &OptChainExpr) {
        if let OptChainBase::Call(call) = &*n.base {
            self.visit_call_expr(&CallExpr {
                span: call.span,
                callee: Callee::Expr(call.callee.clone()),
                args: call.args.clone(),
                type_args: call.type_args.clone(),
            });
        }
        n.visit_children_with(self);
    }
}

impl Extractor<'_> {
    fn extract_fn_props(&mut self, function: &Function, _component_name: String) {
        if self.file.props.is_some() {
            return;
        }

        let params = &function.params;

        if !params.is_empty() {
            let first_param = &params[0];
            let (line, col) = pos(self.cm, first_param.span);

            match &first_param.pat {
                Pat::Ident(ident) => {
                    let type_ann = extract_type_annotation(&ident.type_ann);
                    self.file.props = Some(PropsInfo {
                        name: ident.id.sym.to_string(),
                        type_annotation: type_ann,
                        is_destructured: false,
                        line,
                        column: col,
                    });
                }
                Pat::Object(obj) => {
                    let type_ann = extract_type_annotation(&obj.type_ann);
                    self.file.props = Some(PropsInfo {
                        name: "props".to_string(),
                        type_annotation: type_ann,
                        is_destructured: true,
                        line,
                        column: col,
                    });
                }
                _ => {}
            }
        }

        self.extract_body_stmts(function);
    }

    fn extract_body_stmts(&mut self, function: &Function) {
        self.file.has_component_body = true;
        let stmts = match &function.body {
            Some(body) => &body.stmts,
            None => return,
        };

        for stmt in stmts {
            let (line, col) = pos(self.cm, stmt.span());
            let kind = self.classify_body_stmt(stmt);
            self.file.body_stmts.push(BodyStmt { kind, line, column: col });

            if kind == BodyStmtKind::Signal {
                if let Stmt::Decl(Decl::Var(var_decl)) = stmt {
                    if let Some(decl) = var_decl.decls.first() {
                        if let Pat::Ident(ident) = &decl.name {
                            if let Some(init) = &decl.init {
                                if let Some((sig_kind, val)) = extract_signal_decl(init, &mut self.file.unsupported_errors) {
                                    self.file.signals.push(SignalDecl {
                                        name: ident.id.sym.to_string(),
                                        kind: sig_kind,
                                        initial_value: val,
                                    });
                                }
                            }
                        }
                    }
                }
            }

            if kind == BodyStmtKind::EventHandler {
                let has_jsx = stmt_contains_jsx(stmt);
                if let Ok(src) = self.cm.span_to_snippet(stmt.span()) {
                    if let Stmt::Decl(Decl::Fn(fn_decl)) = stmt {
                        self.file.handler_decls.push(strip_ts_annotations(&src, stmt.span(), fn_decl));
                        self.file.handler_has_jsx.push(has_jsx);
                    }
                }
            }

            if kind == BodyStmtKind::DerivedConst {
                if let Ok(src) = self.cm.span_to_snippet(stmt.span()) {
                    if let Stmt::Decl(Decl::Var(var_decl)) = stmt {
                        self.file.derived_consts.push(strip_var_ts(&src, stmt.span(), var_decl));
                    }
                }
            }

            if let Stmt::Return(ret) = stmt {
                if let Some(arg) = &ret.arg {
                    self.file.render_tree = extract_jsx_from_expr(arg, self.cm, &mut self.file.unsupported_errors);
                }
            }
        }
    }

    fn classify_body_stmt(&self, stmt: &Stmt) -> BodyStmtKind {
        match stmt {
            Stmt::Decl(Decl::Fn(_)) => BodyStmtKind::EventHandler,
            Stmt::Decl(Decl::Var(var_decl)) => {
                match var_decl.kind {
                    VarDeclKind::Const => {
                        if let Some(decl) = var_decl.decls.first() {
                            if let Some(init) = &decl.init {
                                if is_signal_or_computed(init) {
                                    return BodyStmtKind::Signal;
                                }
                                if matches!(&**init, Expr::Arrow(_) | Expr::Fn(_)) {
                                    return BodyStmtKind::EventHandler;
                                }
                            }
                        }
                        BodyStmtKind::DerivedConst
                    }
                    VarDeclKind::Let => BodyStmtKind::Let,
                    VarDeclKind::Var => BodyStmtKind::Var,
                }
            }
            Stmt::Return(_) => BodyStmtKind::Return,
            _ => BodyStmtKind::Other,
        }
    }

    fn check_jsx_expr(&mut self, expr: &Expr, line: usize, col: usize) {
        match expr {
            Expr::Bin(bin) if bin.op == BinaryOp::LogicalAnd => {
                self.file.jsx_expressions.push(JsxExprInfo {
                    kind: JsxExpression::AndConditional,
                    line,
                    column: col,
                });
            }
            Expr::Call(call) => {
                if is_dot_map_call(call) {
                    self.file.jsx_expressions.push(JsxExprInfo {
                        kind: JsxExpression::InlineMap,
                        line,
                        column: col,
                    });
                }
            }
            _ => {}
        }

        if let Expr::Bin(bin) = expr {
            let (l_line, l_col) = pos(self.cm, bin.left.span());
            self.check_jsx_expr(&bin.left, l_line, l_col);
            let (r_line, r_col) = pos(self.cm, bin.right.span());
            self.check_jsx_expr(&bin.right, r_line, r_col);
        }
    }
}

fn extract_type_annotation(ts_type_ann: &Option<Box<TsTypeAnn>>) -> TypeAnnotation {
    match ts_type_ann {
        None => TypeAnnotation::Untyped,
        Some(type_ann) => match &*type_ann.type_ann {
            TsType::TsKeywordType(kw) => match kw.kind {
                TsKeywordTypeKind::TsAnyKeyword => TypeAnnotation::Any,
                _ => TypeAnnotation::Named(format_type(&*type_ann.type_ann)),
            },
            other => TypeAnnotation::Named(format_type(other)),
        },
    }
}

fn format_type(ty: &TsType) -> String {
    format!("{:?}", ty)
}

fn is_dot_map_call(call: &CallExpr) -> bool {
    match &call.callee {
        Callee::Expr(callee_expr) => match &**callee_expr {
            Expr::Member(member) => matches!(
                &member.prop,
                MemberProp::Ident(ident) if ident.sym.as_ref() == "map"
            ),
            _ => false,
        },
        _ => false,
    }
}

fn is_signal_or_computed(expr: &Expr) -> bool {
    match expr {
        Expr::Call(call) => match &call.callee {
            Callee::Expr(callee_expr) => match &**callee_expr {
                Expr::Ident(ident) => {
                    let name = ident.sym.as_ref();
                    name == "signal" || name == "computed"
                }
                _ => false,
            },
            _ => false,
        },
        _ => false,
    }
}

fn extract_jsx_from_expr(expr: &Expr, cm: &dyn swc_common::SourceMapper, errors: &mut Vec<ParserError>) -> Option<JsxNode> {
    match expr {
        Expr::Paren(paren) => extract_jsx_from_expr(&paren.expr, cm, errors),
        Expr::JSXElement(jsx) => Some(convert_jsx_element(jsx, cm, errors)),
        Expr::JSXFragment(frag) => Some(convert_jsx_fragment(frag, cm, errors)),
        _ => None,
    }
}

fn convert_jsx_element(el: &JSXElement, cm: &dyn swc_common::SourceMapper, errors: &mut Vec<ParserError>) -> JsxNode {
    let tag = jsx_element_name_to_string(&el.opening.name);

    if tag == "For" {
        return extract_for_element(el, cm, errors);
    }

    let mut is_hydrate = false;
    let attrs: Vec<JsxAttr> = el
        .opening
        .attrs
        .iter()
        .filter_map(|a| {
            if is_client_hydrate_attr(a) {
                is_hydrate = true;
                None
            } else {
                convert_jsx_attr_or_spread(a, errors)
            }
        })
        .collect();
    let children = el.children.iter().map(|c| convert_jsx_child(c, cm, errors)).collect();

    let is_component = tag
        .chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false)
        && tag != "For";

    JsxNode::Element {
        tag,
        attrs,
        children,
        is_hydrate_root: is_hydrate,
        is_component,
    }
}

fn convert_jsx_fragment(frag: &JSXFragment, cm: &dyn swc_common::SourceMapper, errors: &mut Vec<ParserError>) -> JsxNode {
    let children = frag.children.iter().map(|c| convert_jsx_child(c, cm, errors)).collect();
    JsxNode::Element {
        tag: String::new(),
        attrs: Vec::new(),
        children,
        is_hydrate_root: false,
        is_component: false,
    }
}

fn convert_jsx_child(child: &JSXElementChild, cm: &dyn swc_common::SourceMapper, errors: &mut Vec<ParserError>) -> JsxNode {
    match child {
        JSXElementChild::JSXElement(el) => convert_jsx_element(el, cm, errors),
        JSXElementChild::JSXFragment(frag) => convert_jsx_fragment(frag, cm, errors),
        JSXElementChild::JSXText(text) => {
            JsxNode::Text(jsx_text_to_string(&text.value))
        }
        JSXElementChild::JSXExprContainer(container) => match &container.expr {
            JSXExpr::Expr(expr) => {
                if let Some(cond) = extract_conditional(expr, cm, errors) {
                    cond
                } else {
                    JsxNode::Expr(format_jsx_expr(expr, errors))
                }
            }
            JSXExpr::JSXEmptyExpr(_) => JsxNode::Text(String::new()),
        },
        JSXElementChild::JSXSpreadChild(_) => {
            errors.push(ParserError {
                code: "UNSUPPORTED_JSX_CONSTRUCT",
                message: "JSX spread children ({...X}) are not yet supported.".into(),
                fix_hint: "Use <For> to iterate, or render each child explicitly.",
            });
            JsxNode::Text(String::new())
        }
    }
}

fn convert_jsx_attr_or_spread(attr: &JSXAttrOrSpread, errors: &mut Vec<ParserError>) -> Option<JsxAttr> {
    match attr {
        JSXAttrOrSpread::JSXAttr(attr) => Some(JsxAttr {
            name: jsx_attr_name_to_string(&attr.name),
            contains_env_call: jsx_attr_value_contains_env_call(&attr.value),
            value: attr
                .value
                .as_ref()
                .map(|v| convert_jsx_attr_value(v, errors))
                .unwrap_or(JsxAttrValue::String(String::new())),
        }),
        JSXAttrOrSpread::SpreadElement(_) => {
            errors.push(ParserError {
                code: "UNSUPPORTED_JSX_CONSTRUCT",
                message: "JSX spread attributes ({...props}) are not yet supported.".into(),
                fix_hint: "Pass each prop explicitly: <Component propA={a} propB={b}>.",
            });
            None
        }
    }
}

fn is_client_hydrate_attr(attr: &JSXAttrOrSpread) -> bool {
    match attr {
        JSXAttrOrSpread::JSXAttr(attr) => matches!(
            &attr.name,
            JSXAttrName::JSXNamespacedName(ns)
                if ns.ns.sym.as_ref() == "client" && ns.name.sym.as_ref() == "hydrate"
        ),
        JSXAttrOrSpread::SpreadElement(_) => false,
    }
}

/// AST-based env() detection scoped to a single expression subtree — the
/// same callee-shape check Extractor::visit_call_expr applies, so the
/// validator can flag env leaks without text heuristics. Fires for an env()
/// call anywhere inside the expression: direct calls, chained methods
/// (`env('K').trim()`), template interpolation (`Bearer ${env('K')}`).
struct EnvCallFinder {
    found: bool,
}

impl Visit for EnvCallFinder {
    fn visit_call_expr(&mut self, n: &CallExpr) {
        if matches!(
            &n.callee,
            Callee::Expr(expr) if matches!(&**expr, Expr::Ident(ident) if ident.sym == "env")
        ) {
            self.found = true;
        }
        n.visit_children_with(self);
    }
}

fn expr_contains_env_call(expr: &Expr) -> bool {
    let mut finder = EnvCallFinder { found: false };
    expr.visit_with(&mut finder);
    finder.found
}

fn jsx_attr_value_contains_env_call(value: &Option<JSXAttrValue>) -> bool {
    let Some(value) = value else {
        return false;
    };
    let JSXAttrValue::JSXExprContainer(container) = value else {
        return false;
    };
    let JSXExpr::Expr(expr) = &container.expr else {
        return false;
    };
    expr_contains_env_call(expr)
}

fn convert_jsx_attr_value(value: &JSXAttrValue, errors: &mut Vec<ParserError>) -> JsxAttrValue {
    match value {
        JSXAttrValue::Lit(lit) => match lit {
            Lit::Str(s) => JsxAttrValue::String(s.value.to_string()),
            _ => JsxAttrValue::String(format_lit(lit, errors)),
        },
        JSXAttrValue::JSXExprContainer(container) => match &container.expr {
            JSXExpr::Expr(expr) => JsxAttrValue::Expr(format_jsx_expr(expr, errors)),
            JSXExpr::JSXEmptyExpr(_) => JsxAttrValue::String(String::new()),
        },
        JSXAttrValue::JSXElement(_el) => {
            errors.push(ParserError {
                code: "UNSUPPORTED_JSX_CONSTRUCT",
                message: "JSX elements as attribute values are not yet supported.".into(),
                fix_hint: "Render the element as a child instead.",
            });
            JsxAttrValue::String("<jsx element>".to_string())
        }
        JSXAttrValue::JSXFragment(_frag) => {
            errors.push(ParserError {
                code: "UNSUPPORTED_JSX_CONSTRUCT",
                message: "JSX fragments as attribute values are not yet supported.".into(),
                fix_hint: "Render the fragment as a child instead.",
            });
            JsxAttrValue::String("<jsx fragment>".to_string())
        }
    }
}

fn jsx_element_name_to_string(name: &JSXElementName) -> String {
    match name {
        JSXElementName::Ident(ident) => ident.sym.to_string(),
        JSXElementName::JSXMemberExpr(member) => format_member_expr_name(member),
        JSXElementName::JSXNamespacedName(ns) => {
            format!("{}:{}", ns.ns.sym, ns.name.sym)
        }
    }
}

fn jsx_attr_name_to_string(name: &JSXAttrName) -> String {
    match name {
        JSXAttrName::Ident(ident) => ident.sym.to_string(),
        JSXAttrName::JSXNamespacedName(ns) => {
            format!("{}:{}", ns.ns.sym, ns.name.sym)
        }
    }
}

fn format_member_expr_name(member: &JSXMemberExpr) -> String {
    let obj = match &member.obj {
        JSXObject::Ident(ident) => ident.sym.to_string(),
        JSXObject::JSXMemberExpr(inner) => format_member_expr_name(inner),
    };
    format!("{}.{}", obj, member.prop.sym)
}

fn jsx_text_to_string(value: &str) -> String {
    let s = value.replace('\n', " ").replace('\r', " ").replace('\t', " ");
    let mut result = String::new();
    let mut prev_was_space = false;
    for c in s.chars() {
        if c == ' ' {
            if !prev_was_space {
                result.push(' ');
                prev_was_space = true;
            }
        } else {
            result.push(c);
            prev_was_space = false;
        }
    }
    result
}

fn format_jsx_expr(expr: &Expr, errors: &mut Vec<ParserError>) -> String {
    match expr {
        Expr::Ident(ident) => ident.sym.to_string(),
        Expr::Member(member) => format_member_expr(member, errors),
        Expr::Lit(lit) => format_lit(lit, errors),
        Expr::Call(call) => format_call_expr(call, errors),
        Expr::Bin(bin) => format_bin_expr(bin, errors),
        Expr::Unary(unary) => format!("{}{}", unary.op, format_jsx_expr(&unary.arg, errors)),
        Expr::Paren(paren) => format!("({})", format_jsx_expr(&paren.expr, errors)),
        Expr::Cond(cond) => format!(
            "{} ? {} : {}",
            format_jsx_expr(&cond.test, errors),
            format_jsx_expr(&cond.cons, errors),
            format_jsx_expr(&cond.alt, errors)
        ),
        Expr::Arrow(arrow) => format_arrow_expr(arrow, errors),
        Expr::Array(arr) => {
            let elems: Vec<String> = arr
                .elems
                .iter()
                .map(|o| match o {
                    Some(expr) => {
                        let mut s = String::new();
                        if expr.spread.is_some() {
                            s.push_str("...");
                        }
                        s.push_str(&format_jsx_expr(&expr.expr, errors));
                        s
                    }
                    None => String::new(),
                })
                .collect();
            format!("[{}]", elems.join(", "))
        }
        Expr::Object(obj) => {
            let props: Vec<String> = obj
                .props
                .iter()
                .map(|p| match p {
                    PropOrSpread::Prop(prop) => match &**prop {
                        Prop::KeyValue(kv) => {
                            let key = prop_key_to_string(&kv.key, errors);
                            format!("{}: {}", key, format_jsx_expr(&kv.value, errors))
                        }
                        Prop::Shorthand(sh) => sh.sym.to_string(),
                        _ => {
                            errors.push(ParserError {
                                code: "UNSUPPORTED_SYNTAX",
                                message: "Method, getter, or setter shorthand in an object literal is not yet supported.".into(),
                                fix_hint: "Use a plain key: value pair instead.",
                            });
                            String::from("/* prop */")
                        }
                    },
                    PropOrSpread::Spread(spread) => {
                        format!("...{}", format_jsx_expr(&spread.expr, errors))
                    }
                })
                .collect();
            format!("{{ {} }}", props.join(", "))
        }
        Expr::JSXElement(_) => {
            errors.push(ParserError {
                code: "UNSUPPORTED_EXPRESSION",
                message: "JSX elements are not supported in expression position (e.g., as prop initializers).".into(),
                fix_hint: "Wrap in a helper function or use a conditional {cond ? <X/> : null} in the JSX tree instead.",
            });
            "/* jsx element */".to_string()
        }
        Expr::JSXFragment(_) => {
            errors.push(ParserError {
                code: "UNSUPPORTED_EXPRESSION",
                message: "JSX fragments are not supported in expression position.".into(),
                fix_hint: "Wrap in a helper function or use <></> directly in the JSX tree.",
            });
            "/* jsx fragment */".to_string()
        }
        Expr::Tpl(tpl) => format_tpl(tpl, errors),
        Expr::TaggedTpl(_) => { errors.push(ParserError { code: "UNSUPPORTED_EXPRESSION", message: "Tagged template literals are not yet supported.".into(), fix_hint: "Use a regular function call instead." }); "/* expr */".to_string() }
        Expr::OptChain(opt) => format_opt_chain(opt, errors),
        Expr::New(_) => { errors.push(ParserError { code: "UNSUPPORTED_EXPRESSION", message: "The 'new' operator is not yet supported in expression position.".into(), fix_hint: "Use a factory function instead." }); "/* expr */".to_string() }
        Expr::Update(_) => { errors.push(ParserError { code: "UNSUPPORTED_EXPRESSION", message: "Update expressions (++/--) are not yet supported.".into(), fix_hint: "Use explicit assignment instead." }); "/* expr */".to_string() }
        Expr::Assign(_) => { errors.push(ParserError { code: "UNSUPPORTED_EXPRESSION", message: "Assignment expressions (=, +=, etc.) are not yet supported in inline expressions.".into(), fix_hint: "Perform the assignment before the expression." }); "/* expr */".to_string() }
        Expr::Seq(_) => { errors.push(ParserError { code: "UNSUPPORTED_EXPRESSION", message: "Comma-separated expressions are not yet supported.".into(), fix_hint: "Split into separate statements or use a helper function." }); "/* expr */".to_string() }
        Expr::Await(_) => { errors.push(ParserError { code: "UNSUPPORTED_EXPRESSION", message: "Await expressions are not yet supported in inline expressions.".into(), fix_hint: "Move the await to a separate variable assignment before the JSX." }); "/* expr */".to_string() }
        Expr::Yield(_) => { errors.push(ParserError { code: "UNSUPPORTED_EXPRESSION", message: "Yield expressions are not yet supported in inline expressions.".into(), fix_hint: "Move the yield to a separate statement." }); "/* expr */".to_string() }
        Expr::Class(_) => { errors.push(ParserError { code: "UNSUPPORTED_EXPRESSION", message: "Class expressions are not yet supported.".into(), fix_hint: "Define the class as a regular declaration or use an object." }); "/* expr */".to_string() }
        Expr::MetaProp(_) => { errors.push(ParserError { code: "UNSUPPORTED_EXPRESSION", message: "Meta property (e.g. new.target) is not yet supported.".into(), fix_hint: "Not applicable." }); "/* expr */".to_string() }
        Expr::SuperProp(_) => { errors.push(ParserError { code: "UNSUPPORTED_EXPRESSION", message: "super.prop access is not yet supported.".into(), fix_hint: "Access the property directly." }); "/* expr */".to_string() }
        Expr::PrivateName(_) => { errors.push(ParserError { code: "UNSUPPORTED_EXPRESSION", message: "Private name (#X) is not yet supported in expression context.".into(), fix_hint: "Use a regular property instead." }); "/* expr */".to_string() }
        Expr::Invalid(_) => { errors.push(ParserError { code: "UNSUPPORTED_EXPRESSION", message: "Invalid expression found — this may indicate a parser error.".into(), fix_hint: "Check syntax and try a simpler expression." }); "/* expr */".to_string() }
        Expr::This(_) => { errors.push(ParserError { code: "UNSUPPORTED_EXPRESSION", message: "'this' is not yet supported in inline expressions — components should be pure functions.".into(), fix_hint: "Pass the value through props or signal()." }); "/* expr */".to_string() }
        Expr::Fn(_) => { errors.push(ParserError { code: "UNSUPPORTED_EXPRESSION", message: "Function expressions are not yet supported in inline expressions.".into(), fix_hint: "Define the function as a named declaration in the component body." }); "/* expr */".to_string() }
        // TypeScript-only expressions — stripped by SWC in many cases, but handle defensively
        Expr::TsTypeAssertion(_) | Expr::TsConstAssertion(_) | Expr::TsNonNull(_) | Expr::TsAs(_) | Expr::TsInstantiation(_) | Expr::TsSatisfies(_) => {
            errors.push(ParserError {
                code: "UNSUPPORTED_EXPRESSION",
                message: "TypeScript-only expressions (type assertions, satisfies, etc.) should be stripped by the parser.".into(),
                fix_hint: "Remove type annotations from expressions. Use them in declarations only.",
            });
            "/* expr */".to_string()
        }
        // JSX namespace/member expressions in non-JSX position
        Expr::JSXMember(_) | Expr::JSXNamespacedName(_) | Expr::JSXEmpty(_) => {
            errors.push(ParserError {
                code: "UNSUPPORTED_EXPRESSION",
                message: "JSX-specific expression found outside JSX context.".into(),
                fix_hint: "Move this expression inside JSX children or attributes.",
            });
            "/* expr */".to_string()
        }
    }
}

fn format_member_expr(member: &MemberExpr, errors: &mut Vec<ParserError>) -> String {
    let obj = format_jsx_expr(&member.obj, errors);
    match &member.prop {
        MemberProp::Ident(ident) => format!("{}.{}", obj, ident.sym),
        MemberProp::Computed(comp) => format!("{}[{}]", obj, format_jsx_expr(&comp.expr, errors)),
        _ => {
            errors.push(ParserError {
                code: "UNSUPPORTED_EXPRESSION",
                message: "Private member access (#X) is not yet supported.".into(),
                fix_hint: "Use a regular property instead.",
            });
            format!("{}[?]", obj)
        }
    }
}

fn format_opt_chain(opt: &OptChainExpr, errors: &mut Vec<ParserError>) -> String {
    match &*opt.base {
        OptChainBase::Member(member) => {
            let obj = format_jsx_expr(&member.obj, errors);
            match &member.prop {
                MemberProp::Ident(ident) => format!("{}?.{}", obj, ident.sym),
                MemberProp::Computed(comp) => format!("{}?.[{}]", obj, format_jsx_expr(&comp.expr, errors)),
                _ => {
                    errors.push(ParserError {
                        code: "UNSUPPORTED_EXPRESSION",
                        message: "Private member access in optional chain is not yet supported.".into(),
                        fix_hint: "Use a regular property instead.",
                    });
                    format!("{}?.[?]", obj)
                }
            }
        }
        OptChainBase::Call(opt_call) => {
            let callee = format_jsx_expr(&opt_call.callee, errors);
            let args: Vec<String> = opt_call.args.iter().map(|arg| {
                let mut s = if arg.spread.is_some() { "..." } else { "" }.to_string();
                s.push_str(&format_jsx_expr(&arg.expr, errors));
                s
            }).collect();
            format!("{}?.({})", callee, args.join(", "))
        }
    }
}

fn format_call_expr(call: &CallExpr, errors: &mut Vec<ParserError>) -> String {
    let callee = match &call.callee {
        Callee::Expr(expr) => format_jsx_expr(expr, errors),
        _ => {
            errors.push(ParserError {
                code: "UNSUPPORTED_EXPRESSION",
                message: "super() or import() calls are not yet supported.".into(),
                fix_hint: "Use a regular function call instead.",
            });
            String::from("/* callee */")
        }
    };
    let args: Vec<String> = call.args.iter().map(|arg| {
        let mut s = if arg.spread.is_some() { "..." } else { "" }.to_string();
        s.push_str(&format_jsx_expr(&arg.expr, errors));
        s
    }).collect();
    format!("{}({})", callee, args.join(", "))
}

fn format_bin_expr(bin: &BinExpr, errors: &mut Vec<ParserError>) -> String {
    let op = format_bin_op(bin.op);
    format!("{} {} {}", format_jsx_expr(&bin.left, errors), op, format_jsx_expr(&bin.right, errors))
}

fn format_bin_op(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::EqEq => "==",
        BinaryOp::NotEq => "!=",
        BinaryOp::EqEqEq => "===",
        BinaryOp::NotEqEq => "!==",
        BinaryOp::Lt => "<",
        BinaryOp::LtEq => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::GtEq => ">=",
        BinaryOp::LShift => "<<",
        BinaryOp::RShift => ">>",
        BinaryOp::ZeroFillRShift => ">>>",
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "%",
        BinaryOp::BitOr => "|",
        BinaryOp::BitXor => "^",
        BinaryOp::BitAnd => "&",
        BinaryOp::LogicalOr => "||",
        BinaryOp::LogicalAnd => "&&",
        BinaryOp::In => " in ",
        BinaryOp::InstanceOf => " instanceof ",
        BinaryOp::Exp => "**",
        BinaryOp::NullishCoalescing => "??",
    }
}

fn format_lit(lit: &Lit, errors: &mut Vec<ParserError>) -> String {
    match lit {
        Lit::Str(s) => {
            let escaped = s.value.to_string()
                .replace('\\', "\\\\")
                .replace('\'', "\\'")
                .replace('\n', "\\n");
            format!("'{}'", escaped)
        }
        Lit::Num(n) => n.value.to_string(),
        Lit::Bool(b) => b.value.to_string(),
        Lit::Null(_) => "null".to_string(),
        Lit::Regex(r) => format!("/{}/{}", r.exp, r.flags),
        _ => {
            errors.push(ParserError {
                code: "UNSUPPORTED_EXPRESSION",
                message: "BigInt literals are not yet supported.".into(),
                fix_hint: "Use a Number or String instead.",
            });
            "undefined".to_string()
        }
    }
}

fn format_tpl(tpl: &Tpl, errors: &mut Vec<ParserError>) -> String {
    let mut result = String::from("`");
    for (i, quasi) in tpl.quasis.iter().enumerate() {
        let raw = quasi.cooked.as_ref().map(|c| c.to_string()).unwrap_or_else(|| quasi.raw.to_string());
        let escaped = raw.replace('\\', "\\\\").replace('`', "\\`").replace("${", "\\${");
        result.push_str(&escaped);
        if i < tpl.exprs.len() {
            result.push_str("${");
            result.push_str(&format_jsx_expr(&tpl.exprs[i], errors));
            result.push('}');
        }
    }
    result.push('`');
    result
}

fn prop_key_to_string(key: &PropName, errors: &mut Vec<ParserError>) -> String {
    match key {
        PropName::Ident(ident) => ident.sym.to_string(),
        PropName::Str(s) => format!("'{}'", s.value),
        PropName::Num(n) => n.value.to_string(),
        PropName::Computed(comp) => format!("[{}]", format_jsx_expr(&comp.expr, errors)),
        _ => {
            errors.push(ParserError {
                code: "UNSUPPORTED_SYNTAX",
                message: "BigInt property keys are not yet supported.".into(),
                fix_hint: "Use a string or identifier property key instead.",
            });
            String::from("/* key */")
        }
    }
}

fn format_arrow_expr(arrow: &ArrowExpr, errors: &mut Vec<ParserError>) -> String {
    let params: Vec<String> = arrow
        .params
        .iter()
        .map(|p| format_pat(p, errors))
        .collect();
    match &*arrow.body {
        BlockStmtOrExpr::BlockStmt(block) => {
            let mut body = String::new();
            for stmt in &block.stmts {
                body.push_str(&format_stmt(stmt, errors));
                body.push('\n');
            }
            format!("({}) => {{\n{}}}", params.join(", "), body)
        }
        BlockStmtOrExpr::Expr(expr) => {
            format!("({}) => {}", params.join(", "), format_jsx_expr(expr, errors))
        }
    }
}

fn format_stmt(stmt: &Stmt, errors: &mut Vec<ParserError>) -> String {
    match stmt {
        Stmt::Expr(expr_stmt) => format!("  {};", format_jsx_expr(&expr_stmt.expr, errors)),
        Stmt::Return(ret) => {
            if let Some(arg) = &ret.arg {
                format!("  return {};", format_jsx_expr(arg, errors))
            } else {
                "  return;".to_string()
            }
        }
        Stmt::Decl(Decl::Var(var)) => {
            let kind = match var.kind {
                VarDeclKind::Const => "const",
                VarDeclKind::Let => "let",
                VarDeclKind::Var => "var",
            };
            let decls: Vec<String> = var.decls.iter().map(|d| {
                let name = match &d.name {
                    Pat::Ident(i) => i.id.sym.to_string(),
                    _ => {
                        errors.push(ParserError {
                            code: "UNSUPPORTED_SYNTAX",
                            message: "Destructured or complex variable declarations are not yet supported in arrow function bodies.".into(),
                            fix_hint: "Use a simple identifier declaration instead.",
                        });
                        "_".to_string()
                    }
                };
                if let Some(init) = &d.init {
                    format!("{} = {}", name, format_jsx_expr(init, errors))
                } else {
                    name
                }
            }).collect();
            format!("  {} {};", kind, decls.join(", "))
        }
        Stmt::If(if_stmt) => {
            let test = format_jsx_expr(&if_stmt.test, errors);
            let cons = format_block_like(&if_stmt.cons, errors);
            let alt = if let Some(alt) = &if_stmt.alt {
                format!(" else {}", format_block_like(alt, errors))
            } else {
                String::new()
            };
            format!("  if ({}) {}{}", test, cons, alt)
        }
        _ => {
            let desc = stmt_description(stmt);
            errors.push(ParserError {
                code: "UNSUPPORTED_STATEMENT",
                message: format!("{} statements are not yet supported in arrow function bodies.", desc),
                fix_hint: "Refactor to use only const/let/var declarations, if statements, return, and expression statements.",
            });
            format!("  /* stmt */")
        }
    }
}

fn stmt_description(stmt: &Stmt) -> &'static str {
    match stmt {
        Stmt::Empty(_) => "Empty",
        Stmt::Debugger(_) => "Debugger",
        Stmt::With(_) => "With",
        Stmt::While(_) => "While",
        Stmt::DoWhile(_) => "DoWhile",
        Stmt::For(_) => "For",
        Stmt::ForIn(_) => "ForIn",
        Stmt::ForOf(_) => "ForOf",
        Stmt::Switch(_) => "Switch",
        Stmt::Labeled(_) => "Labeled",
        Stmt::Break(_) => "Break",
        Stmt::Continue(_) => "Continue",
        Stmt::Throw(_) => "Throw",
        Stmt::Try(_) => "Try",
        Stmt::Decl(_) => "Declaration (non-var)",
        _ => "Unrecognized",
    }
}

fn format_block_like(stmt: &Stmt, errors: &mut Vec<ParserError>) -> String {
    match stmt {
        Stmt::Block(block) => {
            let mut body = String::from("{\n");
            for s in &block.stmts {
                body.push_str(&format_stmt(s, errors));
                body.push('\n');
            }
            body.push('}');
            body
        }
        _ => format_stmt(stmt, errors),
    }
}

fn format_pat(pat: &Pat, errors: &mut Vec<ParserError>) -> String {
    match pat {
        Pat::Ident(ident) => ident.id.sym.to_string(),
        Pat::Object(obj) => {
            let props: Vec<String> = obj.props.iter().map(|p| match p {
                ObjectPatProp::KeyValue(kv) => {
                    let key = prop_key_to_string(&kv.key, errors);
                    format!("{}: {}", key, format_pat(&kv.value, errors))
                }
                ObjectPatProp::Assign(assign) => {
                    let name = assign.key.sym.to_string();
                    if let Some(val) = &assign.value {
                        format!("{} = {}", name, format_jsx_expr(val, errors))
                    } else {
                        name
                    }
                }
                ObjectPatProp::Rest(rest) => {
                    format!("...{}", format_pat(&rest.arg, errors))
                }
            }).collect();
            format!("{{ {} }}", props.join(", "))
        }
        Pat::Array(arr) => {
            let elems: Vec<String> = arr.elems.iter().map(|o| match o {
                Some(p) => format_pat(p, errors),
                None => String::new(),
            }).collect();
            format!("[{}]", elems.join(", "))
        }
        Pat::Rest(rest) => {
            format!("...{}", format_pat(&rest.arg, errors))
        }
        Pat::Assign(assign) => {
            format!("{} = {}", format_pat(&assign.left, errors), format_jsx_expr(&assign.right, errors))
        }
        _ => {
            errors.push(ParserError {
                code: "UNSUPPORTED_SYNTAX",
                message: "Unrecognized parameter pattern is not yet supported.".into(),
                fix_hint: "Use a simple identifier parameter instead.",
            });
            "_".to_string()
        }
    }
}

fn extract_signal_decl(expr: &Expr, errors: &mut Vec<ParserError>) -> Option<(SignalKind, String)> {
    match expr {
        Expr::Call(call) => match &call.callee {
            Callee::Expr(callee_expr) => match &**callee_expr {
                Expr::Ident(ident) => {
                    let name = ident.sym.as_ref();
                    if name == "signal" {
                        let val = call
                            .args
                            .first()
                            .map(|arg| format_jsx_expr(&arg.expr, errors))
                            .unwrap_or_else(|| "undefined".to_string());
                        Some((SignalKind::Signal, val))
                    } else if name == "computed" {
                        let body = call
                            .args
                            .first()
                            .map(|arg| format_jsx_expr(&arg.expr, errors))
                            .unwrap_or_else(|| "() => undefined".to_string());
                        Some((SignalKind::Computed, body))
                    } else {
                        None
                    }
                }
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

fn extract_conditional(expr: &Expr, cm: &dyn swc_common::SourceMapper, errors: &mut Vec<ParserError>) -> Option<JsxNode> {
    match expr {
        Expr::Cond(cond) => {
            let cons = expr_to_jsx_node(&cond.cons, cm, errors)?;
            let alt = expr_to_jsx_node(&cond.alt, cm, errors)?;
            Some(JsxNode::Conditional {
                test: format_jsx_expr(&cond.test, errors),
                cons: Box::new(cons),
                alt: Box::new(alt),
            })
        }
        _ => None,
    }
}

fn expr_to_jsx_node(expr: &Expr, cm: &dyn swc_common::SourceMapper, errors: &mut Vec<ParserError>) -> Option<JsxNode> {
    match expr {
        Expr::Paren(paren) => expr_to_jsx_node(&paren.expr, cm, errors),
        Expr::JSXElement(el) => Some(convert_jsx_element(el, cm, errors)),
        Expr::JSXFragment(frag) => Some(convert_jsx_fragment(frag, cm, errors)),
        // null and undefined are valid empty branches in ternaries
        Expr::Lit(Lit::Null(_)) => Some(JsxNode::Text(String::new())),
        Expr::Ident(ident) if ident.sym.as_ref() == "undefined" => Some(JsxNode::Text(String::new())),
        _ => None,
    }
}

fn extract_for_element(el: &JSXElement, cm: &dyn swc_common::SourceMapper, errors: &mut Vec<ParserError>) -> JsxNode {
    let mut each = String::new();
    let mut key_fn = String::new();

    for attr in &el.opening.attrs {
        if let JSXAttrOrSpread::JSXAttr(attr) = attr {
            let name = jsx_attr_name_to_string(&attr.name);
            let value = attr
                .value
                .as_ref()
                .map(|v| convert_jsx_attr_value(v, errors))
                .unwrap_or(JsxAttrValue::String(String::new()));

            match name.as_str() {
                "each" => {
                    each = match &value {
                        JsxAttrValue::Expr(e) => e.clone(),
                        JsxAttrValue::String(s) => s.clone(),
                    };
                }
                "key" => {
                    key_fn = match &value {
                        JsxAttrValue::Expr(e) => e.clone(),
                        JsxAttrValue::String(s) => s.clone(),
                    };
                }
                _ => {}
            }
        }
    }

    let mut item_param = String::from("item");
    let mut body: Option<JsxNode> = None;
    let mut for_body_decls: Vec<String> = Vec::new();

    for child in &el.children {
        if let JSXElementChild::JSXExprContainer(container) = child {
            if let JSXExpr::Expr(expr) = &container.expr {
                if let Some(b) = extract_arrow_body_jsx(expr, cm, errors) {
                    item_param = b.0;
                    body = Some(b.1);
                    for_body_decls = b.2;
                    break;
                }
            }
        }
    }

    JsxNode::ForEach {
        each,
        key_fn,
        item_param,
        body: Box::new(body.unwrap_or(JsxNode::Text(String::new()))),
        for_body_decls,
    }
}

fn extract_arrow_body_jsx(expr: &Expr, cm: &dyn swc_common::SourceMapper, errors: &mut Vec<ParserError>) -> Option<(String, JsxNode, Vec<String>)> {
    match expr {
        Expr::Paren(paren) => extract_arrow_body_jsx(&paren.expr, cm, errors),
        Expr::Arrow(arrow) => {
            let param = arrow
                .params
                .first()
                .and_then(|p| match p {
                    Pat::Ident(ident) => Some(ident.id.sym.to_string()),
                    _ => None,
                })
                .unwrap_or_else(|| "item".to_string());

            match &*arrow.body {
                BlockStmtOrExpr::BlockStmt(block) => {
                    let mut decls = Vec::new();
                    for stmt in &block.stmts {
                        match stmt {
                            Stmt::Return(ret) => {
                                if let Some(arg) = &ret.arg {
                                    return expr_to_jsx_node(arg, cm, errors)
                                        .map(|node| (param, node, decls));
                                }
                                return None;
                            }
                            Stmt::Decl(Decl::Fn(fn_decl)) => {
                                let span = fn_decl.span();
                                if let Ok(src) = cm.span_to_snippet(span) {
                                    decls.push(strip_ts_annotations(&src, span, fn_decl));
                                }
                            }
                            Stmt::Decl(Decl::Var(var_decl)) => {
                                if var_decl.kind == VarDeclKind::Const {
                                    if let Ok(src) = cm.span_to_snippet(var_decl.span) {
                                        decls.push(strip_var_ts(&src, var_decl.span, var_decl));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    None
                }
                BlockStmtOrExpr::Expr(body_expr) => {
                    expr_to_jsx_node(body_expr, cm, errors).map(|node| (param, node, Vec::new()))
                }
            }
        }
        _ => None,
    }
}

/// Strip TypeScript type annotations from a function declaration using AST span
/// information from SWC's parse tree. Collects spans of all type-annotation nodes
/// (type params, parameter types, return type) and splices them out of the source
/// text. Object-literal types like `{ target: { value: string } }` are handled
/// correctly because each type annotation's span covers the full expression
/// regardless of internal nesting.
fn strip_ts_annotations(src: &str, stmt_span: Span, fn_decl: &FnDecl) -> String {
    let spans = collect_fn_ts_spans(&fn_decl.function);
    splice_out_spans(src, stmt_span, &spans)
}

/// Strip TS type annotations from a variable declaration using AST span info.
fn strip_var_ts(src: &str, stmt_span: Span, var_decl: &VarDecl) -> String {
    let mut spans = Vec::new();
    for decl in &var_decl.decls {
        collect_pat_type_span(&decl.name, &mut spans);
    }
    splice_out_spans(src, stmt_span, &spans)
}

fn collect_fn_ts_spans(func: &Function) -> Vec<Span> {
    let mut spans = Vec::new();

    // Type parameters: function name<T>(...) => strip <T>
    if let Some(type_params) = &func.type_params {
        spans.push(type_params.span);
    }

    // Return type annotation: function name(): Type => strip : Type
    if let Some(ret_type) = &func.return_type {
        spans.push(ret_type.span);
    }

    // Parameter type annotations: (x: Type, y: Type) => strip each : Type
    for param in &func.params {
        collect_pat_type_span(&param.pat, &mut spans);
    }

    spans
}

fn collect_pat_type_span(pat: &Pat, spans: &mut Vec<Span>) {
    match pat {
        Pat::Ident(binding) => {
            if let Some(type_ann) = &binding.type_ann {
                spans.push(type_ann.span);
            }
        }
        Pat::Rest(rest) => {
            collect_pat_type_span(&rest.arg, spans);
        }
        _ => {}
    }
}

/// Splice source text by removing the given spans. Spans are relative to the
/// base_span (the snippet extracted via span_to_snippet).
fn splice_out_spans(src: &str, base_span: Span, spans: &[Span]) -> String {
    if spans.is_empty() {
        return src.to_string();
    }

    let mut sorted: Vec<_> = spans.to_vec();
    sorted.sort_by_key(|s| s.lo);
    sorted.dedup_by_key(|s| s.lo);

    let base = base_span.lo.0 as usize;
    let mut out = String::with_capacity(src.len());
    let mut cursor = 0usize;

    for span in &sorted {
        let start = span.lo.0 as usize;
        let end = span.hi.0 as usize;
        if start < base || end < base {
            continue;
        }
        let rel_start = start - base;
        let rel_end = (end - base).min(src.len());
        if rel_start < cursor {
            continue;
        }
        if rel_start > cursor {
            out.push_str(&src[cursor..rel_start]);
        }
        cursor = rel_end;
    }

    if cursor < src.len() {
        out.push_str(&src[cursor..]);
    }

    out
}

/// Returns true if a statement's AST subtree contains any JSX element or fragment.
fn stmt_contains_jsx(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Decl(Decl::Fn(fn_decl)) => match &fn_decl.function.body {
            Some(body) => block_contains_jsx(body),
            None => false,
        },
        Stmt::Return(ret) => match &ret.arg {
            Some(expr) => expr_contains_jsx(expr),
            None => false,
        },
        _ => false,
    }
}

fn block_contains_jsx(block: &BlockStmt) -> bool {
    for stmt in &block.stmts {
        if stmt_contains_jsx(stmt) {
            return true;
        }
    }
    false
}

fn expr_contains_jsx(expr: &Expr) -> bool {
    match expr {
        Expr::JSXElement(_) | Expr::JSXFragment(_) => true,
        Expr::Paren(inner) => expr_contains_jsx(&inner.expr),
        Expr::Cond(cond) => {
            expr_contains_jsx(&cond.cons) || expr_contains_jsx(&cond.alt)
        }
        Expr::Arrow(arrow) => match &*arrow.body {
            BlockStmtOrExpr::BlockStmt(block) => block_contains_jsx(block),
            BlockStmtOrExpr::Expr(body_expr) => expr_contains_jsx(body_expr),
        },
        Expr::Call(call) => {
            for arg in &call.args {
                if expr_contains_jsx(&arg.expr) { return true; }
            }
            false
        }
        _ => false,
    }
}
