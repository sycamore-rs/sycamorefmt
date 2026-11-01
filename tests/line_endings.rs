use sycamorefmt::{Config, format_source};

fn assert_idempotent(src: &str, cfg: &Config) -> String {
    let first = format_source(src, cfg).expect("source should parse");
    assert!(first.warnings.is_empty(), "{:?}", first.warnings);
    let second = format_source(&first.output, cfg).expect("formatted source should parse");
    assert!(
        !second.changed,
        "second formatting changed output:\n{}",
        second.output
    );
    assert_eq!(second.output, first.output);
    first.output
}

fn has_bare_lf(src: &str) -> bool {
    src.as_bytes()
        .windows(2)
        .any(|pair| pair[1] == b'\n' && pair[0] != b'\r')
}

#[test]
fn preserves_mixed_tab_and_space_prefix_for_nested_layout() {
    let input = "fn f() {\n \t view! { div { ({ view! { span { \"nested\" } } }) } }\n}\n";
    let cfg = Config {
        tab_spaces: 4,
        ..Config::default()
    };
    let output = assert_idempotent(input, &cfg);

    // The macro line's exact prefix is retained, and subsequent layout is
    // expressed relative to that prefix using the configured tab stop.
    let macro_lines: Vec<_> = output
        .lines()
        .filter(|line| {
            line.contains("div {") || line.contains("span {") || line.contains("view! {")
        })
        .collect();
    assert!(macro_lines.len() >= 3, "{output}");
    assert!(
        macro_lines.iter().all(|line| line.starts_with(" \t ")),
        "macro body lines lost the source prefix: {output}"
    );
    assert!(output.contains("\n \t }"), "{output}");
}

#[test]
fn generated_layout_uses_crlf_without_normalizing_literal_bytes() {
    let input = concat!(
        "fn f() {\r\n",
        "\tview! { div(value={ consume(\r\n",
        "        \"ordinary first\r\n  ordinary second\".to_string(),\r\n",
        "        r#\"raw first\r\n   raw second\"#,\r\n",
        "        br#\"byte first\r\n byte second\"#,\r\n",
        "        c\"c first\r\n  c second\",\r\n",
        "    ) }) { r#\"text first\r\n text second\"# } }\r\n",
        "}\r\n",
    );
    let output = assert_idempotent(input, &Config::default());

    assert!(
        !has_bare_lf(&output),
        "generated output mixed LF into CRLF source:\n{output}"
    );
    for literal in [
        "\"ordinary first\r\n  ordinary second\"",
        "r#\"raw first\r\n   raw second\"#",
        "br#\"byte first\r\n byte second\"#",
        "c\"c first\r\n  c second\"",
        "r#\"text first\r\n text second\"#",
    ] {
        assert!(
            output.contains(literal),
            "literal changed: {literal:?}\n{output}"
        );
    }
}

#[test]
fn final_layout_conversion_preserves_opaque_macro_token_whitespace() {
    let input =
        "fn f() {\n    view! { div(value={ stringify!(left   +\n            right) }) {} }\n}\n";
    let output = assert_idempotent(input, &Config::default());
    assert!(
        output.contains("stringify!(left   +\n            right)"),
        "opaque macro tokens changed:\n{output}"
    );
}

#[test]
fn macro_in_path_generic_arguments_stays_opaque_to_layout_conversion() {
    let path_input = "fn f() { view! { Foo::<{ stringify!(left   +\n            right) }> {} } }\n";
    let path_output = format_source(path_input, &Config::default()).unwrap();
    assert!(
        path_output.warnings.is_empty(),
        "{:?}",
        path_output.warnings
    );
    assert!(
        path_output
            .output
            .contains("stringify!(left   +\n            right)"),
        "macro in path generic arguments changed:\n{}",
        path_output.output
    );
}

#[test]
fn tab_stop_zero_is_safe_and_does_not_change_prefix() {
    let input = "fn f() {\n\tview! { div { span {} } }\n}\n";
    let cfg = Config {
        tab_spaces: 0,
        ..Config::default()
    };
    let output = assert_idempotent(input, &cfg);
    assert!(output.contains("\n\tdiv"), "{output}");
}

#[test]
fn comment_after_quote_character_literals_is_not_dropped() {
    let cases = [
        r###"fn f() { view! { div { ('"') /*KEEP*/ "text" } } }"###,
        r###"fn f() { view! { div { (b'"') /*KEEP*/ "text" } } }"###,
        r###"fn f() { view! { div { ('\'') /*KEEP*/ "text" } } }"###,
        r###"fn f<'a>(value: &'a str) { view! { div { (&'a value) /*KEEP*/ "text" } } }"###,
    ];

    for input in cases {
        let output = format_source(input, &Config::default()).expect("source should parse");
        assert_eq!(output.output, input, "comment-bearing view was modified");
        assert_eq!(
            output.warnings.len(),
            1,
            "comment should be reported: {input}"
        );
        assert!(
            output.warnings[0].contains("comments"),
            "{:?}",
            output.warnings
        );
    }
}

#[test]
fn unsupported_macro_delimiters_are_warned_and_left_unchanged() {
    let input = "fn f() { view!(div {}); view![span {}]; }\n";
    let output = format_source(input, &Config::default()).expect("source should parse");
    assert_eq!(output.output, input);
    assert_eq!(output.warnings.len(), 2, "{:?}", output.warnings);
    assert!(
        output
            .warnings
            .iter()
            .all(|warning| warning.contains("unsupported delimiter"))
    );
}
