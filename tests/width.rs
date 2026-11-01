use sycamorefmt::{Config, format_source};

const INPUT: &str =
    "fn f() {\n    view! { div { section(very_long_property_name=value) { \"x\" } } }\n}\n";

fn format_at(width: usize) -> String {
    let cfg = Config {
        max_width: width,
        ..Config::default()
    };
    let first = format_source(INPUT, &cfg).unwrap();
    assert!(first.warnings.is_empty(), "{:?}", first.warnings);
    let second = format_source(&first.output, &cfg).unwrap();
    assert_eq!(second.output, first.output);
    first.output
}

fn format_input(input: &str, width: usize) -> String {
    let cfg = Config {
        max_width: width,
        ..Config::default()
    };
    let first = format_source(input, &cfg).unwrap();
    assert!(first.warnings.is_empty(), "{:?}", first.warnings);
    assert_eq!(
        format_source(&first.output, &cfg).unwrap().output,
        first.output
    );
    first.output
}

#[test]
fn unicode_children_use_display_columns() {
    for text in ["界", "👩‍💻"] {
        let input = format!("fn f(){{view!{{div{{\"{text}\"}}}}}}\n");
        let fits = format_input(&input, 16);
        assert!(
            fits.contains(&format!("    div {{ \"{text}\" }}")),
            "{fits}"
        );
        let too_narrow = format_input(&input, 15);
        assert!(
            too_narrow.contains(&format!("    div {{\n        \"{text}\"\n    }}")),
            "{too_narrow}"
        );
    }
}

#[test]
fn combining_characters_do_not_consume_extra_columns() {
    let input = "fn f(){view!{div{\"e\u{301}\"}}}\n";
    let fits = format_input(input, 15);
    assert!(fits.contains("    div { \"e\u{301}\" }"), "{fits}");
    let too_narrow = format_input(input, 14);
    assert!(
        too_narrow.contains("    div {\n        \"e\u{301}\"\n    }"),
        "{too_narrow}"
    );
}

#[test]
fn unicode_props_use_display_columns() {
    let input = "fn f(){view!{div(title=\"界\")}}\n";
    let fits = format_input(input, 19);
    assert!(fits.contains("    div(title=\"界\")"), "{fits}");
    let too_narrow = format_input(input, 18);
    assert!(
        too_narrow.contains("    div(\n        title=\"界\",\n    )"),
        "{too_narrow}"
    );
}

#[test]
fn unicode_tag_names_use_display_columns() {
    let input = "fn f(){view!{组件(id=\"x\")}}\n";
    let fits = format_input(input, 16);
    assert!(fits.contains("    组件(id=\"x\")"), "{fits}");
    let too_narrow = format_input(input, 15);
    assert!(
        too_narrow.contains("    组件(\n        id=\"x\",\n    )"),
        "{too_narrow}"
    );
}

#[test]
fn literal_tabs_advance_to_configured_tab_stops() {
    let input = "fn f(){view!{div{r\"a\tb\"}}}\n";
    let fits = format_input(input, 20);
    assert!(fits.contains("    div { r\"a\tb\" }"), "{fits}");
    let too_narrow = format_input(input, 19);
    assert!(
        too_narrow.contains("    div {\n        r\"a\tb\"\n    }"),
        "{too_narrow}"
    );
}

#[test]
fn multiline_prop_closing_line_uses_its_absolute_column() {
    // The closing line is 12 spaces plus `) { "x" }`: exactly 21 columns.
    let fits = format_at(21);
    assert!(fits.contains("            ) { \"x\" }"), "{fits}");
    let too_narrow = format_at(20);
    assert!(
        too_narrow.contains("            ) {\n                \"x\"\n            }"),
        "{too_narrow}"
    );
}
