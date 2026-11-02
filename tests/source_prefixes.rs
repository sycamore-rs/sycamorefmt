//! Parser spans must be translated back past any BOM or shebang prefix.

use std::process::Command;

use sycamorefmt::{Config, format_source};

#[test]
fn prefixed_sources_format_like_unprefixed_sources() {
    let prefixes = [
        "\u{feff}",
        "#!/usr/bin/env script\n",
        "#!/usr/bin/env script\r\n",
        "#!/é\n",
        "\u{feff}#!/usr/bin/env script\n",
        "\u{feff}#!/é\r\n",
    ];
    let sources = [
        "fn main(){view!{}}\n",
        "fn main(){view!{ }}\n",
        "fn main(){view!{div{\"hello\"}}}\n",
        "fn main(){view!{div { div {} }}}\n",
        "fn main(){view!{div{(view!{span{\"nested\"}})}}}\n",
        "fn main(){let é=\"🦀\";view!{div{\"é\"}};view!{span{}}}\n",
        "// untouched\nfn main() {\n\tview!{div{\"hello\"}}\n}\n",
        "#![allow(unused)]\nfn main(){view!{div{}}}\n",
    ];
    let cfg = Config::default();

    for prefix in prefixes {
        for source in sources {
            let expected = format_source(source, &cfg).unwrap();
            assert!(expected.warnings.is_empty());
            let input = format!("{prefix}{source}");
            let outcome = format_source(&input, &cfg).unwrap();
            assert!(
                outcome.warnings.is_empty(),
                "{input:?}: {:?}",
                outcome.warnings
            );
            assert_eq!(
                outcome.output,
                format!("{prefix}{}", expected.output),
                "{input:?}"
            );
            assert_eq!(outcome.changed, expected.changed, "{input:?}");
            // Besides checking the exact splice, verify syntax and idempotence.
            syn::parse_file(&outcome.output).unwrap();
            let second = format_source(&outcome.output, &cfg).unwrap();
            assert!(second.warnings.is_empty());
            assert!(!second.changed, "{input:?}");
            assert_eq!(second.output, outcome.output);
        }
    }
}

#[test]
fn prefixes_without_views_are_unchanged() {
    for input in [
        "\u{feff}",
        "#!/usr/bin/env script",
        "\u{feff}#!/usr/bin/env script",
        "\u{feff}#!/usr/bin/env script\nfn main() {}\n",
    ] {
        let outcome = format_source(input, &Config::default()).unwrap();
        assert_eq!(outcome.output, input);
        assert!(!outcome.changed);
        assert!(outcome.warnings.is_empty());
    }
}

#[test]
fn inner_attributes_are_not_treated_as_shebangs() {
    for prefix in ["", "\u{feff}"] {
        let input = format!("{prefix}#![allow(unused)]\nfn main(){{view!{{}}}}\n");
        let outcome = format_source(&input, &Config::default()).unwrap();
        assert_eq!(
            outcome.output,
            format!("{prefix}#![allow(unused)]\nfn main(){{view! {{}}}}\n")
        );
        assert!(outcome.warnings.is_empty());
    }
}

#[test]
fn file_mode_preserves_prefixes_and_formats_only_views() {
    let directory = tempfile::tempdir().unwrap();
    for (index, prefix) in ["\u{feff}", "#!/usr/bin/env script\n", "\u{feff}#!/é\n"]
        .iter()
        .enumerate()
    {
        let path = directory.path().join(format!("prefixed_{index}.rs"));
        let input = format!("{prefix}fn main(){{view!{{}}}}\n");
        std::fs::write(&path, &input).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_sycamorefmt"))
            .arg("--fail-on-warnings")
            .arg(&path)
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}");
        assert!(result.stdout.is_empty());
        assert!(result.stderr.is_empty(), "{result:?}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{prefix}fn main(){{view! {{}}}}\n")
        );
    }
}
