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
