//! CSS class-name collision visibility.
//!
//! marisjs deliberately does NOT implement CSS scoping (no class-name rewriting,
//! no CSS Modules) — that would break compatibility with external CSS frameworks
//! (Tailwind, Bootstrap, ...) that depend on exact, predictable global class
//! names. Instead, collision RISK is made visible: when the same class name is
//! defined in two different .css files that are both transitively imported into
//! the same page, the build surfaces a warning naming both files and the class.
//!
//! Calibration — the warning must not fire on legitimate, expected overlap:
//!
//! 1. Cascade-override pattern (spec §2a, B2/B2.3): a class intentionally
//!    overridden by a LATER stylesheet. In the per-page `<link>` order (DFS
//!    pre-order of the page's component tree), an ancestor component's
//!    stylesheet always loads before a descendant's — so when the two
//!    importing components stand in a strict ancestor/descendant relation,
//!    the descendant's redefinition is the documented override pattern.
//! 2. Site-wide stylesheet convention (Layout pattern): a stylesheet imported
//!    by a component rendered by more than one page is the shared base layer
//!    that page-specific stylesheets legitimately refine.
//!
//! The result is a warning (never a hard error): colliding class names across
//! two libraries is sometimes intentional and harmless.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One .css file loaded into a page, with its import site and the class names
/// it defines.
pub struct CssFileRef {
    /// Relative path of the .css file (source-tree relative).
    pub file: PathBuf,
    /// Relative path of the .tsx component that imports it.
    pub import_site: PathBuf,
    /// Class names defined by this file (`.name` selectors).
    pub classes: BTreeSet<String>,
}

/// A class name defined by two different .css files in the same page closure.
#[derive(Debug, Clone, PartialEq)]
pub struct CssClassCollision {
    /// The colliding class name, WITHOUT the leading `.`.
    pub class: String,
    pub file_a: PathBuf,
    pub file_b: PathBuf,
    pub site_a: PathBuf,
    pub site_b: PathBuf,
}

/// Extracts the set of class names defined by a .css file — every `.name`
/// selector, ignoring comments, strings, numeric values, and url() contents.
/// Only `.` tokens at selector position are definitions: a `.` directly after
/// an ident char is a decimal point (`1.5em`, `url(x.png)`), and a class name
/// starting with a digit is invalid CSS (a number like `.5em` in shorthand).
pub fn extract_class_names(css: &str) -> BTreeSet<String> {
    let bytes = css.as_bytes();
    let mut names = BTreeSet::new();
    let mut i = 0usize;
    let n = bytes.len();
    let mut in_comment = false;

    while i < n {
        if in_comment {
            if i + 1 < n && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                in_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        match bytes[i] {
            b'/' if i + 1 < n && bytes[i + 1] == b'*' => {
                in_comment = true;
                i += 2;
            }
            b'"' | b'\'' => {
                let quote = bytes[i];
                i += 1;
                while i < n && bytes[i] != quote {
                    if bytes[i] == b'\\' && i + 1 < n {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if i < n {
                    i += 1;
                }
            }
            b'.' => {
                // A `.` is at selector position unless it is a decimal point:
                // preceded by an ident char (`1.5em`, `url(x.png)`, `-.5`) —
                // EXCEPT when that ident came from a class match directly
                // before, which makes it a compound selector continuation
                // (`.b.c` — the ident before the pair must itself start at a
                // non-ident char; `x.5.png` fails that, so it stays a number).
                let at_selector_position = i == 0
                    || !is_ident_char(bytes[i - 1])
                    || (i >= 2
                        && bytes[i - 2] == b'.'
                        && (i < 3 || !is_ident_char(bytes[i - 3])));
                if at_selector_position {
                    let mut j = i + 1;
                    let mut name = String::new();
                    while j < n && is_ident_char(bytes[j]) {
                        if bytes[j] == b'\\' && j + 1 < n {
                            // CSS escape: backslash + escaped char are both
                            // part of the ident (consume, keeping the char).
                            name.push(bytes[j + 1] as char);
                            j += 2;
                        } else {
                            name.push(bytes[j] as char);
                            j += 1;
                        }
                    }
                    let first = name.chars().next();
                    // A class name must not start with a digit (`.5em` is a
                    // number in property shorthand, not a selector).
                    if let Some(f) = first {
                        if !f.is_ascii_digit() {
                            names.insert(name);
                        }
                    }
                    i = j;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    names
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'\\'
}

/// Finds class names defined by two different .css files in the same page
/// closure, applying the calibration exemptions:
///
/// - `is_ancestor(a, b)` — true when component `a` is a strict transitive
///   ancestor of component `b` in the page's render tree. Ancestor/descendant
///   import sites are the cascade-override pattern (ancestor's file loads
///   first; the descendant deliberately redefines) → exempt.
/// - `is_site_wide(component)` — true when the importing component renders on
///   more than one page (Layout pattern: a shared component's stylesheet is
///   the base layer others legitimately refine) → exempt.
///
/// Output is sorted (class, file_a, file_b) for deterministic tests.
pub fn find_css_class_collisions(
    files: &[CssFileRef],
    is_ancestor: impl Fn(&Path, &Path) -> bool,
    is_site_wide: impl Fn(&Path) -> bool,
) -> Vec<CssClassCollision> {
    let mut out: Vec<CssClassCollision> = Vec::new();
    for i in 0..files.len() {
        for j in (i + 1)..files.len() {
            let a = &files[i];
            let b = &files[j];
            if a.file == b.file {
                continue;
            }
            for class in a.classes.intersection(&b.classes) {
                let intentional = is_ancestor(&a.import_site, &b.import_site)
                    || is_ancestor(&b.import_site, &a.import_site)
                    || is_site_wide(&a.import_site)
                    || is_site_wide(&b.import_site);
                if intentional {
                    continue;
                }
                out.push(CssClassCollision {
                    class: class.clone(),
                    file_a: a.file.clone(),
                    file_b: b.file.clone(),
                    site_a: a.import_site.clone(),
                    site_b: b.import_site.clone(),
                });
            }
        }
    }
    out.sort_by(|x, y| {
        (
            &x.class, &x.file_a, &x.file_b, &x.site_a, &x.site_b,
        )
            .cmp(&(&y.class, &y.file_a, &y.file_b, &y.site_a, &y.site_b))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    // ── extract_class_names ────────────────────────────────────────────

    #[test]
    fn extracts_simple_compound_and_list_selectors() {
        let css = ".a { color: red; } .b.c { background: blue; }";
        assert_eq!(extract_class_names(css), set(&["a", "b", "c"]));
    }

    #[test]
    fn extracts_selector_lists_and_descendants() {
        let css = ".a, .b { margin: 0; } .parent .child:hover {}";
        assert_eq!(extract_class_names(css), set(&["a", "b", "parent", "child"]));
    }

    #[test]
    fn ignores_comments() {
        let css = "/* .fake { color: red; } */ .real { color: green; }";
        assert_eq!(extract_class_names(css), set(&["real"]));
    }

    #[test]
    fn ignores_string_contents() {
        let css = r#".a::before { content: ".b"; } .c { color: red; }"#;
        assert_eq!(extract_class_names(css), set(&["a", "c"]));
    }

    #[test]
    fn ignores_numeric_and_url_dots() {
        let css = "p { margin: 1.5em; padding: .5em; font-size: 12.5px; background: url(x.5.png); }";
        assert_eq!(extract_class_names(css), BTreeSet::new());
    }

    #[test]
    fn ignores_keyframes_and_media_conditions() {
        let css = "@keyframes spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } } @media (min-width: 1.5px) { .a { animation: spin 1.5s; } }";
        assert_eq!(extract_class_names(css), set(&["a"]));
    }

    #[test]
    fn ignores_attribute_selector_values() {
        let css = "a[href=\".b\"], [data-x='.c'] { color: red; } .d { color: blue; }";
        assert_eq!(extract_class_names(css), set(&["d"]));
    }

    #[test]
    fn handles_escaped_class_names() {
        let css = ".foo\\:bar { color: red; }";
        assert_eq!(extract_class_names(css), set(&["foo:bar"]));
    }

    // ── find_css_class_collisions ──────────────────────────────────────

    fn file_ref(file: &str, site: &str, classes: &[&str]) -> CssFileRef {
        CssFileRef {
            file: PathBuf::from(file),
            import_site: PathBuf::from(site),
            classes: set(classes),
        }
    }

    #[test]
    fn sibling_imports_sharing_a_class_collide() {
        let files = vec![
            file_ref("components/A.css", "components/A.tsx", &["header"]),
            file_ref("components/B.css", "components/B.tsx", &["header"]),
        ];
        let cols = find_css_class_collisions(&files, |_, _| false, |_| false);
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].class, "header");
        assert_eq!(cols[0].file_a, PathBuf::from("components/A.css"));
        assert_eq!(cols[0].file_b, PathBuf::from("components/B.css"));
    }

    #[test]
    fn ancestor_descendant_imports_are_the_override_pattern_and_silent() {
        let files = vec![
            file_ref("components/Base.css", "components/Wrapper.tsx", &["box"]),
            file_ref("components/Override.css", "components/StyledBox.tsx", &["box"]),
        ];
        // Wrapper is a strict ancestor of StyledBox in the page tree.
        let cols = find_css_class_collisions(&files, |a, b| {
            a == Path::new("components/Wrapper.tsx") && b == Path::new("components/StyledBox.tsx")
        }, |_| false);
        assert!(cols.is_empty(), "override pattern must not warn: {:?}", cols);
    }

    #[test]
    fn site_wide_import_is_the_base_layer_and_silent() {
        let files = vec![
            file_ref("components/styles.css", "components/Layout.tsx", &["btn"]),
            file_ref("components/Button.css", "components/Button.tsx", &["btn"]),
        ];
        // Layout renders on multiple pages → its stylesheet is the base layer.
        let cols = find_css_class_collisions(&files, |_, _| false, |c| {
            c == Path::new("components/Layout.tsx")
        });
        assert!(cols.is_empty(), "site-wide convention must not warn: {:?}", cols);
    }

    #[test]
    fn disjoint_classes_do_not_collide() {
        let files = vec![
            file_ref("components/A.css", "components/A.tsx", &["a"]),
            file_ref("components/B.css", "components/B.tsx", &["b"]),
        ];
        let cols = find_css_class_collisions(&files, |_, _| false, |_| false);
        assert!(cols.is_empty());
    }

    #[test]
    fn mixed_page_finds_only_the_genuine_collision() {
        let files = vec![
            file_ref("components/Base.css", "components/Wrapper.tsx", &["box", "shared"]),
            file_ref("components/Override.css", "components/StyledBox.tsx", &["box"]),
            file_ref("components/Card.css", "components/Card.tsx", &["shared"]),
        ];
        // Wrapper > StyledBox override (.box) exempt; Card is a sibling of
        // Wrapper → "shared" is a genuine collision.
        let cols = find_css_class_collisions(&files, |a, b| {
            a == Path::new("components/Wrapper.tsx") && b == Path::new("components/StyledBox.tsx")
        }, |_| false);
        assert_eq!(cols.len(), 1, "{:?}", cols);
        assert_eq!(cols[0].class, "shared");
        assert_eq!(cols[0].file_a, PathBuf::from("components/Base.css"));
        assert_eq!(cols[0].file_b, PathBuf::from("components/Card.css"));
    }
}
