//! Fixture-based and property-based tests for the formatting pipeline.
//!
//! These tests shell out to the real `rustfmt` binary (via
//! `sycamorefmt::format_source`), so they only run meaningfully in an
//! environment that has `rustfmt` on `PATH`. If it isn't available, the
//! tests print a message and skip themselves rather than failing with a
//! confusing error, since that's an environment issue rather than a bug in
//! this crate.

use std::path::Path;
use std::process::Command;

use sycamorefmt::{Config, format_source};

/// Returns `true` if a real `rustfmt` binary appears to be usable.
fn rustfmt_available() -> bool {
    Command::new("rustfmt")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

macro_rules! skip_without_rustfmt {
    () => {
        if !rustfmt_available() {
            eprintln!(
                "skipping {}: `rustfmt` is not available on PATH",
                concat!(module_path!(), "::", "test")
            );
            return;
        }
    };
}

fn read_fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

fn run_fixture(input_name: &str, expected_name: &str) {
    let input = read_fixture(input_name);
    let expected = read_fixture(expected_name);
    let cfg = Config::default();

    let outcome = format_source(&input, &cfg).unwrap_or_else(|e| {
        panic!("formatting {input_name} failed: {e}");
    });
    for warning in &outcome.warnings {
        eprintln!("warning while formatting {input_name}: {warning}");
    }
    assert!(
        outcome.warnings.is_empty(),
        "expected no warnings while formatting {input_name}, got: {:?}",
        outcome.warnings
    );
    assert_eq!(
        outcome.output, expected,
        "formatting {input_name} did not produce the expected output"
    );

    // Formatting an already-formatted file should be a no-op.
    let second = format_source(&outcome.output, &cfg)
        .unwrap_or_else(|e| panic!("re-formatting {input_name}'s output failed: {e}"));
    assert!(
        !second.changed,
        "formatting {input_name}'s output again should be a no-op, but it changed:\n{}",
        second.output
    );
    assert_eq!(second.output, outcome.output);
}

#[test]
fn minimal_view_is_left_untouched() {
    skip_without_rustfmt!();
    let input = read_fixture("minimal.input.rs");
    let cfg = Config::default();
    let outcome = format_source(&input, &cfg).expect("formatting should succeed");
    assert!(
        !outcome.changed,
        "already-canonical input should not be changed"
    );
    run_fixture("minimal.input.rs", "minimal.expected.rs");
}

#[test]
fn messy_whitespace_is_normalized() {
    skip_without_rustfmt!();
    run_fixture("messy.input.rs", "messy.expected.rs");
}

#[test]
fn long_prop_list_is_broken_across_lines() {
    skip_without_rustfmt!();
    run_fixture("props_break.input.rs", "props_break.expected.rs");
}

/// A grab-bag of trickier inputs that we don't hand-verify byte-for-byte,
/// but which should always: format without error, be idempotent, and
/// produce output whose `view!` bodies still parse successfully.
const ROBUSTNESS_CASES: &[&str] = &[
    // Empty view.
    r#"
fn f() -> View {
    view! {}
}
"#,
    // Hyphenated custom element, spread props, directives, a `ref`.
    r#"
fn f() -> View {
    view! {
        my-custom-element(..attrs, prop:checked=is_checked, on:click=handle, r#ref=node_ref) {
            "hello"
        }
    }
}
"#,
    // A quoted (non-identifier) attribute name.
    r#"
fn f() -> View {
    view! {
        div("data-testid"="thing") {
            "hi"
        }
    }
}
"#,
    // Nested `view!` inside a dyn expression (an if/else each branch of
    // which produces a `View` via its own `view!` call).
    r#"
fn f(show: bool) -> View {
    view! {
        div {
            ({
                if show {
                    view! { span { "shown" } }
                } else {
                    view! { span { "hidden" } }
                }
            })
        }
    }
}
"#,
    // A qualified macro path.
    r#"
fn f() -> View {
    sycamore::view! {
        div { "hi" }
    }
}
"#,
    // Multiple top-level sibling nodes (no wrapping element).
    r#"
fn f() -> View {
    view! {
        button(on:click=decrement) { "-" }
        button(on:click=increment) { "+" }
    }
}
"#,
];

#[test]
fn robustness_cases_format_without_error_and_are_idempotent() {
    skip_without_rustfmt!();
    let cfg = Config::default();

    for (i, input) in ROBUSTNESS_CASES.iter().enumerate() {
        let first = format_source(input, &cfg)
            .unwrap_or_else(|e| panic!("case {i} failed to format: {e}\ninput:\n{input}"));
        assert!(
            first.warnings.is_empty(),
            "case {i} produced warnings: {:?}",
            first.warnings
        );

        // Every `view!` body in the output must still parse successfully.
        assert_view_macros_parse(&first.output, i);

        let second = format_source(&first.output, &cfg)
            .unwrap_or_else(|e| panic!("case {i} failed to re-format its own output: {e}"));
        assert!(
            !second.changed,
            "case {i} was not idempotent; formatting its output again changed it:\n--- first ---\n{}\n--- second ---\n{}",
            first.output, second.output
        );
    }
}

/// Parses `src` as a Rust file and asserts that every `view!`-like macro
/// invocation it contains has a body that `sycamore-view-parser` accepts.
fn assert_view_macros_parse(src: &str, case_index: usize) {
    use syn::visit::Visit;

    struct Checker(Vec<syn::Macro>);
    impl<'ast> Visit<'ast> for Checker {
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            if node
                .path
                .segments
                .last()
                .map(|s| s.ident == "view")
                .unwrap_or(false)
            {
                self.0.push(node.clone());
            }
            syn::visit::visit_macro(self, node);
        }
    }

    let file = syn::parse_file(src)
        .unwrap_or_else(|e| panic!("case {case_index}'s output is not valid Rust: {e}\n{src}"));
    let mut checker = Checker(Vec::new());
    checker.visit_file(&file);

    for mac in &checker.0 {
        syn::parse2::<sycamore_view_parser::ir::Root>(mac.tokens.clone()).unwrap_or_else(|e| {
            panic!("case {case_index}'s output has an unparsable `view!` body: {e}\n{src}")
        });
    }
}
