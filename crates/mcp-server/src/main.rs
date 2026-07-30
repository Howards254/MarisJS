//! MCP server exposing marisjs validator via the `validate_component` tool.
//!
//! Thin wrapper — calls the exact same `parser::parse_component_{file,source}` +
//! `validator::validate` code path the CLI uses. Output shape is identical to
//! `marisjs validate`: `{ valid, errors: [{ line, column, code, message, fix_hint }] }`.

use rmcp::{
    handler::server::wrapper::Parameters,
    schemars, tool, tool_router,
    ServiceExt,
    transport::stdio,
};
use serde::Serialize;

// ── Input ──────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ValidateParams {
    #[schemars(description = "Absolute path to a .tsx file, or raw TSX source code")]
    source: String,
}

// ── Output (identical shape to CLI `marisjs validate`) ──────────────

#[derive(Debug, Serialize)]
struct ValidateResult {
    valid: bool,
    errors: Vec<ErrorInfo>,
}

#[derive(Debug, Serialize)]
struct ErrorInfo {
    line: Option<usize>,
    column: Option<usize>,
    code: String,
    message: String,
    fix_hint: String,
}

// ── Server ──────────────────────────────────────────────────────────

#[derive(Clone)]
struct Marisjs;

#[tool_router(server_handler)]
impl Marisjs {
    #[tool(description = "Validate a marisjs .tsx component file. Accepts either an absolute file path or raw TSX source code. Returns { valid, errors: [{ line, column, code, message, fix_hint }] } — the same structured JSON that `marisjs validate` produces.")]
    fn validate_component(&self, Parameters(params): Parameters<ValidateParams>) -> String {
        let result = validate(&params.source);
        serde_json::to_string_pretty(&result).unwrap_or_else(|e| {
            format!(r#"{{"valid":false,"errors":[{{"code":"INTERNAL","message":"{}","fix_hint":"","line":null,"column":null}}]}}"#, e)
        })
    }
}

// ── Core validation logic (shared with CLI) ────────────────────────

pub(crate) fn validate(source: &str) -> ValidateResult {
    if source.contains('\n') || source.contains("export ") || source.contains("// @runsOn") {
        validate_source(source)
    } else {
        validate_file(source)
    }
}

pub(crate) fn validate_file(path: &str) -> ValidateResult {
    match parser::parse_component_file(path) {
        Ok(component) => diagnostics_to_result(validator::validate(&component)),
        Err(e) => ValidateResult {
            valid: false,
            errors: vec![ErrorInfo {
                line: None, column: None,
                code: "PARSE_ERROR".into(),
                message: e.message,
                fix_hint: "Check that the file exists and contains valid TypeScript/TSX.".into(),
            }],
        },
    }
}

pub(crate) fn validate_source(source: &str) -> ValidateResult {
    match parser::parse_component_source(source, "inline.tsx") {
        Ok(component) => diagnostics_to_result(validator::validate(&component)),
        Err(e) => ValidateResult {
            valid: false,
            errors: vec![ErrorInfo {
                line: None, column: None,
                code: "PARSE_ERROR".into(),
                message: e.message,
                fix_hint: "Check the source code for syntax errors.".into(),
            }],
        },
    }
}

fn diagnostics_to_result(diagnostics: Vec<validator::Diagnostic>) -> ValidateResult {
    ValidateResult {
        valid: diagnostics.is_empty(),
        errors: diagnostics.into_iter().map(|d| ErrorInfo {
            line: d.line,
            column: d.column,
            code: d.code.to_string(),
            message: d.message,
            fix_hint: d.fix_hint.to_string(),
        }).collect(),
    }
}

// ── Main ────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    eprintln!("marisjs MCP server starting on stdio…");
    let service = Marisjs.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests;
