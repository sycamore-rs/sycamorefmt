//! Regression tests for interactions between layout and opaque macro input.

use sycamorefmt::{Config, format_source};

#[test]
fn multiline_opaque_macro_is_unchanged_in_tab_indented_crlf_view() {
    let input = "fn f() {\r\n\tview! { div { (stringify!(a  +\r\n   b)) } }\r\n}\r\n";
    let outcome = format_source(input, &Config::default()).unwrap();
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert!(
        outcome.output.contains("stringify!(a  +\r\n   b)"),
        "{}",
        outcome.output
    );
    assert!(
        outcome.output.contains("\r\n\tview! {\r\n"),
        "{:?}",
        outcome.output
    );
    let second = format_source(&outcome.output, &Config::default()).unwrap();
    assert_eq!(second.output, outcome.output);
}

#[test]
fn comparison_followed_by_unary_not_is_not_a_macro() {
    let input = "fn f() { view! { div { (if a > !(b) { 1 } else { 0 }) } } }\n";
    let outcome = format_source(input, &Config::default()).unwrap();
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert!(outcome.output.contains("a > !"), "{}", outcome.output);
}

#[test]
fn multiline_macro_rules_source_survives_layout_conversion() {
    let source = "macro_rules! example {\r\n   ($x:tt) => { stringify!($x) };\r\n}";
    let input =
        format!("fn f() {{\r\n\tview! {{ div {{ ({{ {source} example!(x) }}) }} }}\r\n}}\r\n");
    let outcome = format_source(&input, &Config::default()).unwrap();
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert!(outcome.output.contains(source), "{:?}", outcome.output);
    assert_eq!(
        format_source(&outcome.output, &Config::default())
            .unwrap()
            .output,
        outcome.output
    );
}

#[test]
fn character_literal_before_comment_never_loses_comment() {
    for input in [
        "fn f() {\r\n\tview! { div { ('\"') /*KEEP*/ \"text\" } }\r\n}\r\n",
        "fn f() { view! { div { ('\\\'') /*KEEP*/ \"text\" } } }\n",
        "fn f() { view! { div { (b'\\\'') /*KEEP*/ \"text\" } } }\n",
    ] {
        let outcome = format_source(input, &Config::default()).unwrap();
        assert_eq!(outcome.output, input);
        assert!(!outcome.warnings.is_empty());
    }
}
