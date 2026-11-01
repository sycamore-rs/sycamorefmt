//! Fixture-based and property-based tests for the formatting pipeline.

use std::path::Path;

use sycamorefmt::{Config, format_source};

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
    run_fixture("messy.input.rs", "messy.expected.rs");
}

#[test]
fn long_prop_list_is_broken_across_lines() {
    run_fixture("props_break.input.rs", "props_break.expected.rs");
}

#[test]
fn whitespace_is_preserved() {
    run_fixture("whitespace.input.rs", "whitespace.expected.rs");
}

#[test]
fn blank_line_trivia_is_preserved_and_normalized() {
    run_fixture("trivia.input.rs", "trivia.expected.rs");
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
    // Conditional view branches, including else-if and blank lines between
    // the nested branch's siblings.
    r#"
fn f(show: bool, ready: bool) -> View {
    view! {
        if show {
            span { "shown" }

            "more"
        } else if ready {
            span { "ready" }
        } else {
            "hidden"
        }
    }
}
"#,
    // Match branches exercise guarded/destructured patterns, block roots,
    // and the single-node arm-body grammar.
    r#"
fn f(choice: Option<i32>) -> View {
    view! {
        match choice {
            Some(value) if value > 0 => {
                strong { (value) }

                "positive"
            }
            None => "empty",
            _ => (fallback()),
        }
    }
}
"#,
];

#[test]
fn if_and_match_nodes_render_structured_branches() {
    let input = r#"
fn f(show: bool, ready: bool, choice: Option<i32>) -> View {
    view! {
        if show {
            span { "shown" }

            "more"
        } else if ready {
            span { "ready" }
        } else {
            "hidden"
        }
        match choice {
            Some(value) if value > 0 => {
                strong { (value) }

                "positive"
            }
            None => "empty",
            _ => (fallback()),
        }
    }
}
"#;
    let cfg = Config::default();
    let first = format_source(input, &cfg).expect("control-flow nodes should format");
    assert!(
        first.warnings.is_empty(),
        "unexpected warnings: {:?}",
        first.warnings
    );
    assert!(first.output.contains("} else if ready {"));
    assert!(first.output.contains("Some(value) if value > 0 => {"));
    assert!(
        first
            .output
            .contains("None => {\n                \"empty\"\n            },")
    );
    assert!(
        first
            .output
            .contains("span { \"shown\" }\n\n            \"more\"")
    );
    assert_view_macros_parse(&first.output, 0);

    let second =
        format_source(&first.output, &cfg).expect("formatted control flow should reformat");
    assert!(!second.changed, "control-flow output should be idempotent");
    assert_eq!(second.output, first.output);
}

#[test]
fn robustness_cases_format_without_error_and_are_idempotent() {
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

#[test]
fn component_paths_preserve_generic_argument_tokens() {
    use quote::quote;
    use sycamore_view_parser::ir::{Node, Root, TagIdent};

    fn path_tokens(src: &str) -> String {
        let file = syn::parse_file(src).unwrap();
        let syn::Item::Fn(function) = &file.items[0] else {
            panic!("expected a function")
        };
        let syn::Stmt::Macro(statement) = &function.block.stmts[0] else {
            panic!("expected a macro")
        };
        let root: Root = syn::parse2(statement.mac.tokens.clone()).unwrap();
        let [Node::Tag(tag)] = root.0.as_slice() else {
            panic!("expected one tag")
        };
        let TagIdent::Path(path) = &tag.ident else {
            panic!("expected a path")
        };
        quote!(#path).to_string()
    }

    let inputs = [
        r##"fn f() { view! { Foo::<{ make!(" :: ") }> {} } }"##,
        r###"fn f() { view! { ui::Foo::<{ make!(r#" :: "#) }>::Bar::<{ make!(b" :: ") }> {} } }"###,
        "fn f() { view! { ::ui::nested::Component {} } }",
    ];
    for input in inputs {
        let outcome = format_source(input, &Config::default()).unwrap();
        assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
        assert_eq!(
            path_tokens(input),
            path_tokens(&outcome.output),
            "{}",
            outcome.output
        );
        assert_eq!(
            format_source(&outcome.output, &Config::default())
                .unwrap()
                .output,
            outcome.output
        );
    }
}

#[test]
fn comments_between_macro_path_and_body_are_preserved() {
    let input = "fn f() { view /* important */ ! { div {} } }\n";
    let outcome = format_source(input, &Config::default()).unwrap();
    assert_eq!(outcome.output, input);
    assert!(!outcome.warnings.is_empty());
}

#[test]
fn malformed_nested_view_leaves_outer_view_unchanged() {
    let input = "fn f() { view! { div { (view! { div(=) }) } } }\n";
    let outcome = format_source(input, &Config::default()).unwrap();
    assert_eq!(outcome.output, input);
    assert!(!outcome.warnings.is_empty());
}

#[test]
fn nested_views_respect_nondefault_indentation() {
    let input = "fn f() {\n  view! { div { ({ view! { span { \"hello\" } } }) } }\n}\n";
    let cfg = Config {
        tab_spaces: 2,
        ..Config::default()
    };
    let first = format_source(input, &cfg).unwrap();
    assert!(first.warnings.is_empty(), "{:?}", first.warnings);
    let lines: Vec<_> = first.output.lines().collect();
    let nested = lines
        .iter()
        .position(|line| line.contains("view! {") && !line.trim_start().starts_with("view! {"))
        .unwrap_or_else(|| {
            lines
                .iter()
                .enumerate()
                .filter(|(_, l)| l.contains("view! {"))
                .nth(1)
                .unwrap()
                .0
        });
    let macro_indent = lines[nested].len() - lines[nested].trim_start().len();
    let child_indent = lines[nested + 1].len() - lines[nested + 1].trim_start().len();
    assert_eq!(
        child_indent,
        macro_indent + cfg.tab_spaces,
        "{}",
        first.output
    );
    assert_eq!(
        format_source(&first.output, &cfg).unwrap().output,
        first.output
    );
}

#[test]
fn multiline_string_contents_are_not_reindented() {
    let input = "fn f() {\n    view! { div(title={ make(\"first\n  second\nthird\") }) }\n}\n";
    let cfg = Config::default();
    let output = format_source(input, &cfg).unwrap();
    assert!(output.warnings.is_empty(), "{:?}", output.warnings);
    assert!(
        output.output.contains("\"first\n  second\nthird\""),
        "{}",
        output.output
    );
    assert_eq!(
        format_source(&output.output, &cfg).unwrap().output,
        output.output
    );
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
