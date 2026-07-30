use super::*;
use serde_json::Value;

fn fixtures_dir() -> String {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../cli/tests/fixtures/"
        ).to_string()
    }

    fn run_validate_file(fixture: &str) -> Value {
        let path = format!("{}{}", fixtures_dir(), fixture);
        let result = validate_file(&path);
        serde_json::to_value(&result).unwrap()
    }

    fn error_by_code<'a>(output: &'a Value, code: &str) -> &'a Value {
        output["errors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["code"].as_str() == Some(code))
            .unwrap()
    }

    fn assert_no_code(output: &Value, forbidden_code: &str) {
        let found = output["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["code"].as_str() == Some(forbidden_code));
        assert!(!found, "expected NO error with code {forbidden_code}");
    }

    fn assert_position(output: &Value, code: &str, line: Option<u64>, col: Option<u64>) {
        let error = error_by_code(output, code);
        assert_eq!(error["line"].as_u64(), line);
        assert_eq!(error["column"].as_u64(), col);
    }

    // ── File-based tests (same fixtures CLI uses) ──────────────────

    #[test]
    fn test_missing_runs_on() {
        let output = run_validate_file("missing_runs_on.tsx");
        assert_eq!(output["valid"], false);
        let error = error_by_code(&output, "MISSING_RUNSON");
        assert!(!error["message"].as_str().unwrap().is_empty());
        assert!(error["fix_hint"].as_str().unwrap().contains("Add"));
    }

    #[test]
    fn test_forbidden_import() {
        let output = run_validate_file("forbidden_import.tsx");
        assert_eq!(output["valid"], false);
        assert_position(&output, "FORBIDDEN_IMPORT", Some(2), Some(0));
    }

    #[test]
    fn test_destructured_props() {
        let output = run_validate_file("destructured_props.tsx");
        assert_eq!(output["valid"], false);
        assert_position(&output, "PROPS_DESTRUCTURED", Some(8), Some(34));
    }

    #[test]
    fn test_nested_braces_jsx() {
        let output = run_validate_file("nested_braces_jsx.tsx");
        assert_eq!(output["valid"], false);
        assert_position(&output, "INLINE_MAP", Some(8), Some(8));
        assert_position(&output, "AND_CONDITIONAL", Some(13), Some(14));
    }

    #[test]
    fn test_string_looks_like_directive_is_clean() {
        let output = run_validate_file("string_looks_like_directive.tsx");
        // The real check: no DUPLICATE_RUNSON false positive from the string
        // that contains "// @runsOn client" as literal text. Other errors
        // (like FILENAME_MISMATCH) are allowed.
        assert_no_code(&output, "DUPLICATE_RUNSON");
        assert_no_code(&output, "MISSING_RUNSON");
    }

    #[test]
    fn test_string_looks_like_map_is_clean() {
        let output = run_validate_file("string_looks_like_map.tsx");
        // The real check: no INLINE_MAP/AND_CONDITIONAL false positive from
        // the string that contains ".map()" as literal text.
        assert_no_code(&output, "INLINE_MAP");
        assert_no_code(&output, "AND_CONDITIONAL");
    }

    #[test]
    fn test_output_shape() {
        let output = run_validate_file("missing_runs_on.tsx");
        assert!(output.is_object());
        assert!(output["valid"].is_boolean());
        let errors = output["errors"].as_array().unwrap();
        for error in errors {
            assert!(error["line"].is_number() || error["line"].is_null());
            assert!(error["column"].is_number() || error["column"].is_null());
            assert!(error["code"].is_string());
            assert!(error["message"].is_string());
            assert!(error["fix_hint"].is_string());
            let code = error["code"].as_str().unwrap();
            assert!(
                code.chars().all(|c| c.is_uppercase() || c == '_'),
                "error code should be SCREAMING_SNAKE_CASE, got: {code}"
            );
        }
    }

    // ── Source-based tests (raw string, not file path) ─────────────

    #[test]
    fn test_validate_raw_source_with_error() {
        let source = "// @runsOn client\nimport { useState } from 'react';\ntype P = {};\nexport function Foo(props: P) { return <div/>; }\n";
        let result = validate_source(source);
        assert!(!result.valid);
        let output = serde_json::to_value(&result).unwrap();
        let error = error_by_code(&output, "FORBIDDEN_IMPORT");
        assert_eq!(error["line"].as_u64(), Some(2));
        assert_eq!(error["column"].as_u64(), Some(0));
    }

    #[test]
    fn test_validate_raw_source_valid() {
        // Component name "inline" matches filename "inline.tsx"
        let source = "// @runsOn client\ntype P = { name: string };\nexport function inline(props: P) { return <div>{props.name}</div>; }\n";
        let result = validate_source(source);
        assert!(result.valid);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_validate_malformed_source() {
        let result = validate_source("this is not valid typescript }}} {{{");
        assert!(!result.valid);
        assert_eq!(result.errors[0].code, "PARSE_ERROR");
    }

    // ── PARSE_ERROR shape (same as CLI exit code 2 path) ───────────

    #[test]
    fn test_parse_error_output_shape() {
        let result = validate_file("/nonexistent/path.tsx");
        assert!(!result.valid);
        let error = &result.errors[0];
        assert_eq!(error.code, "PARSE_ERROR");
        assert!(error.line.is_none());
        assert!(error.column.is_none());
        assert!(!error.message.is_empty());
    assert!(!error.fix_hint.is_empty());
}
