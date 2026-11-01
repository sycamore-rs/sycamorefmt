//! Opaque macro inputs are not recursively interpreted as Sycamore views.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn run(input: &str) -> Output {
    let binary = env!("CARGO_BIN_EXE_sycamorefmt");
    assert!(Path::new(binary).is_absolute());

    let mut child = Command::new(binary)
        .env_remove("PATH")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run sycamorefmt");
    child
        .stdin
        .take()
        .expect("stdin was not piped")
        .write_all(input.as_bytes())
        .expect("failed to write formatter input");
    child
        .wait_with_output()
        .expect("failed to collect formatter output")
}

fn formatted(input: &str) -> String {
    let output = run(input);
    assert!(
        output.status.success(),
        "sycamorefmt failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "unexpected formatter diagnostics: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("formatter output was not UTF-8")
}

#[test]
fn leaves_view_tokens_in_a_top_level_opaque_macro_untouched() {
    let input = "opaque!(view!{div{\"top-level\"}});\n";

    assert_eq!(formatted(input), input);
}

#[test]
fn leaves_view_tokens_in_an_expression_macro_untouched() {
    let input = r#"fn render() -> View {
    view! {
        div {
            (opaque!(view!{span{"opaque"}}))
        }
    }
}
"#;
    let output = formatted(input);

    assert!(
        output.contains("opaque!(view!{span{\"opaque\"}})"),
        "opaque macro input was rewritten as a Sycamore view:\n{output}"
    );
}

#[test]
fn formats_raw_identifier_view_paths() {
    let input = "fn f(){ r#view!{div{(r#view!{span{\"x\"}})}} }\n";
    let output = formatted(input);
    assert_eq!(output.matches("r#view! {").count(), 2, "{output}");
    assert!(output.contains("span { \"x\" }"), "{output}");
    assert_eq!(formatted(&output), output);
}

#[test]
fn formats_views_nested_in_ordinary_expression_syntax() {
    let input = r#"fn render(show: bool) -> View {
    view! {
        div {
            ({
                if show {
                    view!{span{"shown"}}
                } else {
                    view!{span{"hidden"}}
                }
            })
        }
    }
}
"#;
    let output = formatted(input);

    assert_eq!(output.matches("view! {").count(), 3, "{output}");
    assert_eq!(output.matches("span { \"shown\" }").count(), 1, "{output}");
    assert_eq!(output.matches("span { \"hidden\" }").count(), 1, "{output}");
}
