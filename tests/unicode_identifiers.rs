//! Regression tests for Unicode identifier boundaries in view control flow.

use sycamorefmt::{Config, format_source};

#[test]
fn unicode_identifier_continuations_are_not_else_keywords() {
    // These are all valid Rust identifier continuations: a non-ASCII letter,
    // a combining mark, and a non-ASCII digit.
    for suffix in ["é", "\u{301}", "١"] {
        let tag = format!("else{suffix}");
        let input =
            format!("fn f() {{ view! {{ if show {{ \"then\" }} {tag} {{ \"other\" }} }} }}\n");
        let outcome = format_source(&input, &Config::default()).unwrap();
        assert!(
            outcome.warnings.is_empty(),
            "{tag:?} unexpectedly skipped: {:?}",
            outcome.warnings
        );
        assert!(outcome.output.contains(&tag), "{}", outcome.output);
        assert!(outcome.output.contains("\"other\""), "{}", outcome.output);

        let second = format_source(&outcome.output, &Config::default()).unwrap();
        assert_eq!(second.output, outcome.output, "{tag:?} was not idempotent");
    }
}

#[test]
fn ascii_identifier_continuations_are_not_else_keywords() {
    for tag in ["else_name", "else9"] {
        let input =
            format!("fn f() {{ view! {{ if show {{ \"then\" }} {tag} {{ \"other\" }} }} }}\n");
        let outcome = format_source(&input, &Config::default()).unwrap();
        assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
        assert!(outcome.output.contains(tag), "{}", outcome.output);
        assert!(outcome.output.contains("\"other\""), "{}", outcome.output);
    }
}

#[test]
fn actual_else_and_else_if_are_still_recognized() {
    let input =
        "fn f() { view! { if show { \"yes\" } else if ready { \"ready\" } else { \"no\" } } }\n";
    let outcome = format_source(input, &Config::default()).unwrap();
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert!(
        outcome.output.contains("} else if ready {"),
        "{}",
        outcome.output
    );
    assert!(outcome.output.contains("} else {"), "{}", outcome.output);

    let second = format_source(&outcome.output, &Config::default()).unwrap();
    assert_eq!(second.output, outcome.output);
}
