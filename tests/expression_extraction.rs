//! Regression tests for expression and pattern extraction from prettyplease wrappers.

use sycamorefmt::{Config, format_source};

fn formatted(input: &str) -> String {
    let outcome = format_source(input, &Config::default()).expect("format_source failed");
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    outcome.output
}

fn assert_idempotent(input: &str) -> String {
    let first = formatted(input);
    let second = formatted(&first);
    assert_eq!(second, first, "formatted output was not idempotent");
    first
}

#[test]
fn extracts_multiline_block_and_struct_literal_expressions() {
    let input = r#"fn render(flag: bool) -> View {
    view! {
        div {
            ({
                let point = Point {
                    x: 1,
                    y: 2,
                };
                if flag {
                    Point { x: point.x, y: point.y }
                } else {
                    Point { x: 0, y: 0 }
                }
            })
        }
    }
}
"#;
    let output = assert_idempotent(input);
    assert!(output.contains("let point = Point {"), "{output}");
    assert!(
        output.contains("Point { x: point.x, y: point.y }"),
        "{output}"
    );
}

#[test]
fn retains_match_guards_complex_patterns_and_sentinel_strings() {
    let input = r#"fn render(value: Option<(&str, (i32, bool))>) -> View {
    view! {
        match value {
            Some((
                " => {}",
                (number, true),
            )) if check(("nested => {}", "}")) && number > 0 => {
                span { (number) }
            }
            _ => { span { "fallback" } }
        }
    }
}
"#;
    let output = assert_idempotent(input);
    assert!(
        output.contains("\" => {}\""),
        "pattern literal was lost: {output}"
    );
    assert!(
        output.contains("if check((\"nested => {}\", \"}\"))"),
        "guard was lost: {output}"
    );
    assert!(
        output.contains("number > 0 =>"),
        "guard condition was lost: {output}"
    );
}

#[test]
fn preserves_nested_string_literals_with_old_wrapper_sentinels() {
    let input = r##"fn render(flag: bool) -> View {
    view! {
        div {
            ({
                let text = if flag {
                    "inner } and => {} and fn __sycamorefmt_expr__"
                } else {
                    r#"nested } => {}"#
                };
                (text, "match __sycamorefmt_value {\n")
            })
        }
    }
}
"##;
    let output = assert_idempotent(input);
    assert!(
        output.contains("inner } and => {} and fn __sycamorefmt_expr__"),
        "string literal content changed: {output}"
    );
    assert!(
        output.contains("nested } => {}"),
        "raw string content changed: {output}"
    );
    assert!(
        output.contains("match __sycamorefmt_value {\\n"),
        "nested sentinel string was lost: {output}"
    );
}

#[test]
fn extracts_unicode_patterns_and_guard_expressions() {
    let input = r#"fn render(value: &str) -> View {
    view! {
        match value {
            "🦊" if valid("café") => { span { "✓" } }
            _ => { span { "other" } }
        }
    }
}
"#;
    let output = assert_idempotent(input);
    assert!(output.contains("\"🦊\" if valid(\"café\") =>"), "{output}");
    assert!(output.contains("\"✓\""), "{output}");
}

#[test]
fn keeps_opaque_expression_macro_tokens_and_is_idempotent() {
    let input = r#"fn render() -> View {
    view! {
        div {
            (opaque!(a  +  b, " => {}", { nested }))
        }
    }
}
"#;
    let output = assert_idempotent(input);
    assert!(
        output.contains("opaque!(a  +  b, \" => {}\", { nested })"),
        "opaque macro tokens changed: {output}"
    );
}
