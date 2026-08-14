//! MCP server exposing marisjs validator via the `validate_component` tool.
//!
//! Thin wrapper — calls the exact same `parser::parse_component_{file,source}` +
//! `validator::validate` code path the CLI uses. Output shape is identical to
//! `marisjs validate`: `{ valid, errors: [...], warnings: [...] }`.

use rmcp::{
    handler::server::wrapper::Parameters,
    schemars, tool, tool_router,
    ServiceExt,
    transport::stdio,
};
use serde::Serialize;

// ── Input ──────────────────────────────────────────────────────────
//
// Explicit, unambiguous input contract: the caller states its intent by
// providing EXACTLY ONE of the two variants. There is no heuristic guessing
// whether a string is a path or source code (the previous single-string
// interface misclassified paths containing "\n", "export ", or "// @runsOn").
// The JSON schema is an object with oneOf of the two variants — MCP-compliant
// (root type "object") and strict (additionalProperties: false, so passing
// both variants or an unknown field is rejected).

#[derive(Debug, schemars::JsonSchema)]
#[schemars(schema_with = "validate_input_schema")]
enum ValidateInput {
    Path { path: String },
    Source { source: String },
}

// Manual Deserialize so ambiguous input produces an ACTIONABLE error instead
// of serde's internal "data did not match any variant of untagged enum
// ValidateInput" (which names a Rust type and tells the caller nothing about
// what to do — this project's fail-loud standard: errors say what to do).
// The schema is unaffected (still the strict oneOf object from
// validate_input_schema); unknown fields are still rejected via
// deny_unknown_fields on the raw shape.
impl<'de> serde::Deserialize<'de> for ValidateInput {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            path: Option<String>,
            source: Option<String>,
        }
        let raw = Raw::deserialize(deserializer)?;
        match (raw.path, raw.source) {
            (Some(path), None) => Ok(ValidateInput::Path { path }),
            (None, Some(source)) => Ok(ValidateInput::Source { source }),
            _ => Err(serde::de::Error::custom(
                "Provide exactly one of `path` or `source` — both or neither were given.",
            )),
        }
    }
}

fn validate_input_schema(gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
    use schemars::Schema;
    use serde_json::{json, Map};

    let string_prop = |description: &str| -> serde_json::Value {
        json!({ "type": "string", "description": description })
    };

    let required_variant = |name: &str| -> serde_json::Value {
        json!({ "required": [name] })
    };

    let mut obj = Map::new();
    obj.insert("type".into(), json!("object"));
    obj.insert(
        "properties".into(),
        json!({
            "path": string_prop("Absolute path to an existing .tsx file to validate from disk"),
            "source": string_prop("Raw TSX source code to validate in-memory (reported filename: inline.tsx)"),
        }),
    );
    // Exactly one of the two fields; anything else is rejected.
    obj.insert("additionalProperties".into(), json!(false));
    obj.insert(
        "oneOf".into(),
        json!([
            required_variant("path"),
            required_variant("source"),
        ]),
    );
    let _ = gen;
    Schema::from(obj)
}

// ── Output (identical shape to CLI `marisjs validate`) ──────────────

#[derive(Debug, Serialize)]
struct ValidateResult {
    valid: bool,
    errors: Vec<ErrorInfo>,
    warnings: Vec<ErrorInfo>,
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
    #[tool(description = "Validate a marisjs .tsx component or .ts API route file. Provide EXACTLY ONE of: 'path' (absolute path to an existing file) or 'source' (raw source code, validated in-memory). Returns { valid, errors, warnings } — the same structured JSON that `marisjs validate` produces.")]
    fn validate_component(&self, Parameters(params): Parameters<ValidateInput>) -> String {
        let result = match params {
            ValidateInput::Path { path } => validate_file(&path),
            ValidateInput::Source { source } => validate_source(&source),
        };
        serde_json::to_string_pretty(&result).unwrap_or_else(|e| {
            format!(r#"{{"valid":false,"errors":[{{"code":"INTERNAL","message":"{}","fix_hint":"","line":null,"column":null}}],"warnings":[]}}"#, e)
        })
    }
}

// ── Core validation logic (shared with CLI) ────────────────────────

/// Total dispatch over the explicit input contract — no guessing.
fn dispatch(input: ValidateInput) -> ValidateResult {
    match input {
        ValidateInput::Path { path } => validate_file(&path),
        ValidateInput::Source { source } => validate_source(&source),
    }
}

/// §7b: an api/ file validates with the API rule set, not the component
/// rule set (handlers are not components). Everything else uses the full
/// component rules. Single dispatch rule, shared with the CLI: the
/// validator's validate_for_path (ancestor-api-dir or @runsOn api
/// directive) — no second, possibly-divergent classification here.
fn validate_path_dispatch(path: &str) -> ValidateResult {
    match parser::parse_component_file(path) {
        Ok(component) => {
            diagnostics_to_result(validator::validate_for_path(&component, std::path::Path::new(path)))
        }
        Err(e) => ValidateResult {
            valid: false,
            errors: vec![ErrorInfo {
                line: None, column: None,
                code: "PARSE_ERROR".into(),
                message: e.message,
                fix_hint: "Check that the file exists and contains valid TypeScript/TSX.".into(),
            }],
            warnings: Vec::new(),
        },
    }
}

pub(crate) fn validate_file(path: &str) -> ValidateResult {
    validate_path_dispatch(path)
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
            warnings: Vec::new(),
        },
    }
}

fn diagnostics_to_result(diagnostics: Vec<validator::Diagnostic>) -> ValidateResult {
    let (errors, warnings): (Vec<_>, Vec<_>) = diagnostics
        .into_iter()
        .partition(|d| !d.is_warning);
    ValidateResult {
        valid: errors.is_empty(),
        errors: errors.into_iter().map(|d| ErrorInfo {
            line: d.line,
            column: d.column,
            code: d.code.to_string(),
            message: d.message,
            fix_hint: d.fix_hint.to_string(),
        }).collect(),
        warnings: warnings.into_iter().map(|d| ErrorInfo {
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
