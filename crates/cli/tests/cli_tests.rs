use serde_json::Value;
use std::process::Command;

fn run_validate(fixture: &str) -> Value {
    let fixture_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_marisjs"))
        .arg("validate")
        .arg(format!("{}{}", fixture_path, fixture))
        .output()
        .unwrap_or_else(|e| panic!("Failed to run CLI: {}", e));

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "Failed to parse JSON output for '{}':\n---\n{}\n---\nError: {}",
            fixture, stdout, e
        )
    })
}

fn get_error_by_code<'a>(output: &'a Value, code: &str) -> &'a Value {
    output["errors"]
        .as_array()
        .expect("errors should be an array")
        .iter()
        .find(|e| e["code"].as_str() == Some(code))
        .unwrap_or_else(|| {
            panic!(
                "Expected error code '{}' not found in output:\n{}",
                code,
                serde_json::to_string_pretty(output).unwrap()
            )
        })
}

fn assert_no_code(output: &Value, forbidden_code: &str) {
    let codes: Vec<&str> = output["errors"]
        .as_array()
        .expect("errors should be an array")
        .iter()
        .map(|e| e["code"].as_str().unwrap_or("MISSING_CODE"))
        .collect();
    assert!(
        !codes.contains(&forbidden_code),
        "Unexpected error code '{}' in output. Got codes: {:?}\nFull output: {}",
        forbidden_code,
        codes,
        serde_json::to_string_pretty(output).unwrap()
    );
}

fn assert_position(output: &Value, code: &str, expected: Option<(u64, u64)>) {
    let error = get_error_by_code(output, code);
    match expected {
        Some((expected_line, expected_column)) => {
            let line = error["line"].as_u64().unwrap_or(9999);
            let column = error["column"].as_u64().unwrap_or(9999);
            assert_eq!(
                (line, column),
                (expected_line, expected_column),
                "Wrong position for '{}'. Expected ({}, {}), got ({}, {}).\nError: {:#?}",
                code,
                expected_line,
                expected_column,
                line,
                column,
                error
            );
        }
        None => {
            assert!(
                error["line"].is_null(),
                "Expected 'line' to be null for '{}', got: {}",
                code,
                error["line"]
            );
            assert!(
                error["column"].is_null(),
                "Expected 'column' to be null for '{}', got: {}",
                code,
                error["column"]
            );
        }
    }
}

// ── baseline fixtures ──────────────────────────────────────────────────

#[test]
fn test_missing_runs_on() {
    let output = run_validate("missing_runs_on.tsx");
    assert_eq!(output["valid"], false);
    let error = get_error_by_code(&output, "MISSING_RUNSON");
    assert!(error["fix_hint"].as_str().unwrap().contains("Add"));
    assert!(!error["message"].as_str().unwrap().is_empty());
    // No directive → no source position
    assert_position(&output, "MISSING_RUNSON", None);
}

#[test]
fn test_missing_runs_on_outputs_null_position() {
    let output = run_validate("missing_runs_on.tsx");
    let error = get_error_by_code(&output, "MISSING_RUNSON");
    assert!(error["line"].is_null(), "MISSING_RUNSON line must be null, got: {}", error["line"]);
    assert!(error["column"].is_null(), "MISSING_RUNSON column must be null, got: {}", error["column"]);
}

#[test]
fn test_forbidden_import() {
    let output = run_validate("forbidden_import.tsx");
    assert_eq!(output["valid"], false);
    assert_position(&output, "FORBIDDEN_IMPORT", Some((2, 0)));
}

#[test]
fn test_destructured_props() {
    let output = run_validate("destructured_props.tsx");
    assert_eq!(output["valid"], false);
    assert_position(&output, "PROPS_DESTRUCTURED", Some((8, 34)));
}

// ── regex-breaking fixtures ────────────────────────────────────────────

/// This file has a genuine `// @runsOn client` directive on line 1
/// AND a string `"Every component needs // @runsOn client or server"` on line 5.
///
/// A regex-based parser scanning the raw source would count both as directives
/// (DUPLICATE_RUNSON). The AST-based parser only inspects comment spans
/// that appear before the first code statement, so it correctly counts exactly 1.
#[test]
fn test_string_looks_like_directive_is_not_duplicate() {
    let output = run_validate("string_looks_like_directive.tsx");
    assert_no_code(&output, "DUPLICATE_RUNSON");
    assert_no_code(&output, "MISSING_RUNSON");
}

/// The string `"Never use .map() inside JSX, use <For> instead"` on line 5
/// contains `.map()` and `&&` as literal text, not as executable code.
///
/// A regex-based parser scanning the raw source would false-positive on both.
/// The AST-based parser only checks actual `CallExpr` / `BinExpr` nodes,
/// so string literals are never flagged.
#[test]
fn test_string_looks_like_map_is_not_flagged() {
    let output = run_validate("string_looks_like_map.tsx");
    assert_no_code(&output, "INLINE_MAP");
    assert_no_code(&output, "AND_CONDITIONAL");
}

/// Multiline JSX with nested braces:
///
/// ```tsx
/// {props.items
///   .filter((x) => x.id > 0)
///   .map((item) => (
///     <li key={item.id}>
///       {item.name}
///       {isReady && <span>(ready)</span>}
///     </li>
///   ))}
/// ```
///
/// Contains both `.map()` and `&&` inside nested JSX braces — exactly the
/// shape that breaks naive regex-based parsing but is common in real components.
/// The AST parser correctly identifies both violations with exact source positions.
#[test]
fn test_nested_braces_jsx() {
    let output = run_validate("nested_braces_jsx.tsx");
    assert_eq!(output["valid"], false);
    assert_position(&output, "INLINE_MAP", Some((8, 8)));
    assert_position(&output, "AND_CONDITIONAL", Some((13, 14)));
}

// ── output contract ────────────────────────────────────────────────────

#[test]
fn test_output_shape() {
    let output = run_validate("missing_runs_on.tsx");

    assert!(output.is_object());
    assert!(output["valid"].is_boolean());

    let errors = output["errors"].as_array().expect("errors must be an array");
    for error in errors {
        // line and column can be numbers (for real positions) or null (for missing positions)
        assert!(
            error["line"].is_number() || error["line"].is_null(),
            "line must be a number or null"
        );
        assert!(
            error["column"].is_number() || error["column"].is_null(),
            "column must be a number or null"
        );
        assert!(error["code"].is_string(), "code must be a string");
        assert!(error["message"].is_string(), "message must be a string");
        assert!(error["fix_hint"].is_string(), "fix_hint must be a string");

        let code = error["code"].as_str().unwrap();
        assert!(
            code.chars().all(|c| c.is_uppercase() || c == '_'),
            "code '{}' must be SCREAMING_SNAKE_CASE",
            code
        );
    }
}

/// Verify that diagnostics with real AST positions produce non-null coordinates.
#[test]
fn test_positions_are_not_all_null() {
    let output = run_validate("forbidden_import.tsx");
    let errors = output["errors"].as_array().unwrap();
    assert!(!errors.is_empty(), "expected at least one error");

    let has_position = errors.iter().any(|e| {
        e["line"].is_number() && e["column"].is_number()
    });
    assert!(has_position, "expected at least one diagnostic with a real (line, column) position");
}

/// Verify that MISSING_RUNSON explicitly emits `null` for both line and column
/// in the JSON output — the external-facing proof the sentinel (0,0) is gone.
#[test]
fn test_missing_runs_on_json_has_null_positions() {
    let output = run_validate("missing_runs_on.tsx");
    let error = get_error_by_code(&output, "MISSING_RUNSON");
    assert!(error["line"].is_null(), "MISSING_RUNSON must emit 'line': null");
    assert!(error["column"].is_null(), "MISSING_RUNSON must emit 'column': null");
}

// ── Regression: server prerender resolves @maris/runtime via embedded runtime ─

/// Builds a @runsOn server component in a clean temp directory with NO
/// @maris/runtime npm package installed. Verifies the prerender step
/// successfully rewrites the import and produces static HTML.
#[test]
fn server_prerender_resolves_runtime_without_npm_install() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let pages = src.join("pages");
    std::fs::create_dir_all(&pages).unwrap();

    let fixture = concat!(
        "// @runsOn server\n",
        "import { data } from '@maris/runtime';\n",
        "type Props = {};\n",
        "export function Index(props: Props) {\n",
        "  const msg = data(async () => 'hello');\n",
        "  return <div>{msg.value}</div>;\n",
        "}\n",
    );
    std::fs::write(pages.join("Index.tsx"), fixture).unwrap();

    let out = dir.path().join("dist");

    let output = Command::new(env!("CARGO_BIN_EXE_marisjs"))
        .arg("build")
        .arg(src.to_str().unwrap())
        .arg("--out")
        .arg(out.to_str().unwrap())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "marisjs build failed (exit {}): {}",
        output.status.code().unwrap_or(-1),
        stderr
    );

    let html_path = out.join("index.html");
    assert!(
        html_path.exists(),
        "Prerendered HTML not produced at {}. Build output:\n{}",
        html_path.display(),
        stderr
    );

    let html = std::fs::read_to_string(&html_path).unwrap();
    assert!(
        html.contains("<div>"),
        "Prerendered HTML should contain rendered server output.\nHTML:\n{}",
        html
    );
}

#[test]
fn manifest_distinguishes_static_vs_data_pages() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let pages = src.join("pages");
    std::fs::create_dir_all(&pages).unwrap();

    // Page with data() — should be mode "server"
    let data_page = concat!(
        "// @runsOn server\n",
        "import { data } from '@maris/runtime';\n",
        "type Props = {};\n",
        "export function Index(props: Props) {\n",
        "  const msg = await data(async () => 'hello');\n",
        "  return <div>{msg.value}</div>;\n",
        "}\n",
    );
    std::fs::write(pages.join("Index.tsx"), data_page).unwrap();

    // Page without data() — should be mode "static"
    let static_page = concat!(
        "// @runsOn server\n",
        "type Props = {};\n",
        "export function About(props: Props) {\n",
        "  return <div>About page</div>;\n",
        "}\n",
    );
    std::fs::write(pages.join("About.tsx"), static_page).unwrap();

    let out = dir.path().join("dist");
    let output = Command::new(env!("CARGO_BIN_EXE_marisjs"))
        .arg("build")
        .arg(src.to_str().unwrap())
        .arg("--out")
        .arg(out.to_str().unwrap())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "marisjs build failed (exit {}): {}",
        output.status.code().unwrap_or(-1),
        stderr
    );

    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out.join("routes.json")).unwrap(),
    ).unwrap();

    let routes = manifest["routes"].as_array().expect("routes should be array");
    assert_eq!(routes.len(), 2, "expected 2 routes, got {}", routes.len());

    for route in routes {
        let path = route["path"].as_str().unwrap();
        let mode = route["mode"].as_str().unwrap();
        if path == "/" {
            assert_eq!(mode, "server", "route '/' with data() should be mode 'server'");
            assert_eq!(route["file"].as_str().unwrap(), "index.html");
        } else if path == "/about" {
            assert_eq!(mode, "static", "route '/about' without data() should be mode 'static'");
            assert_eq!(route["file"].as_str().unwrap(), "about.html");
        } else {
            panic!("unexpected route: {}", path);
        }
    }
}
