//! Integration coverage for the standalone `view!` rewrite formatter.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn run_without_path(args: &[&str], input: &[u8]) -> Output {
    // Invoke Cargo's absolute binary path so removing PATH only prevents the
    // formatter from accidentally finding external tools such as rustfmt.
    let binary = env!("CARGO_BIN_EXE_sycamorefmt");
    assert!(
        Path::new(binary).is_absolute(),
        "expected an absolute binary path"
    );

    let mut child = Command::new(binary)
        .args(args)
        .env_remove("PATH")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run sycamorefmt by absolute path");
    child
        .stdin
        .take()
        .expect("stdin was not piped")
        .write_all(input)
        .expect("failed to write formatter input");
    child.wait_with_output().expect("failed to collect output")
}

fn formatted(input: &str) -> String {
    let output = run_without_path(&[], input.as_bytes());
    assert!(
        output.status.success(),
        "sycamorefmt failed without PATH: {}",
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
fn formats_views_without_rustfmt_on_path() {
    let input = "fn render() -> View { view!{div{\"hello\"}} }\n";
    let output = formatted(input);

    assert!(
        output.contains("view! {\n"),
        "view macro was not formatted:\n{output}"
    );
    assert!(
        output.contains("div { \"hello\" }"),
        "view body was not formatted:\n{output}"
    );
}

#[test]
fn surrounding_rust_code_and_comments_are_unchanged() {
    let prefix = "// leading comment: keep this spacing\nfn render(  value:i32)-> View{\n    let untouched= value+1; // trailing comment stays\n    let result = ";
    let suffix = ";\n    result\n}\n// final comment: unchanged\n";
    let input = format!("{prefix}view!{{div{{\"hello\"}}}}{suffix}");
    let output = formatted(&input);

    assert!(
        output.starts_with(prefix),
        "code and comments before the view changed:\n{output}"
    );
    assert!(
        output.ends_with(suffix),
        "code and comments after the view changed:\n{output}"
    );
    assert!(output.contains("div { \"hello\" }"));
}

#[test]
fn recursively_formats_nested_views_inside_expressions() {
    let input = r#"fn nested(show: bool) -> View {
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

    assert_eq!(
        output.matches("view! {").count(),
        3,
        "nested macros were not all retained:\n{output}"
    );
    assert_eq!(
        output.matches("span { \"shown\" }").count(),
        1,
        "first nested view was not formatted:\n{output}"
    );
    assert_eq!(
        output.matches("span { \"hidden\" }").count(),
        1,
        "second nested view was not formatted:\n{output}"
    );
}

#[test]
fn formats_match_arms_with_guards_and_destructured_patterns() {
    let input = r#"fn choose(value: Option<i32>) -> View {
    view! {
        match value {
            Some(number)if number>0=>{strong{(number)}}
            None=>"empty",
            _=>"other",
        }
    }
}
"#;
    let output = formatted(input);

    assert!(
        output.contains("Some(number) if number > 0 => {"),
        "guarded pattern was not formatted correctly:\n{output}"
    );
    assert!(
        output.contains("strong { (number) }"),
        "guarded arm body was not formatted:\n{output}"
    );
    assert!(
        output.contains("None => {"),
        "single-node match arm was not normalized:\n{output}"
    );
}

#[test]
fn formatting_is_idempotent() {
    let input = r#"fn render(value:i32)->View{view!{div{span{(value+1)}"text"}}}
"#;
    let first = formatted(input);
    let second = formatted(&first);

    assert_eq!(
        second, first,
        "formatting the output a second time changed it"
    );
}

#[test]
fn handles_hundreds_of_expressions() {
    const EXPRESSION_COUNT: usize = 500;
    let mut input = String::from("fn many() -> View {\n    view! {\n        div {\n");
    for value in 0..EXPRESSION_COUNT {
        input.push_str(&format!("            ({value}+1)\n"));
    }
    input.push_str("        }\n    }\n}\n");

    let output = formatted(&input);
    assert_eq!(
        output.matches(" + 1)").count(),
        EXPRESSION_COUNT,
        "not all dynamic expressions were formatted"
    );
    assert!(
        output.contains("(499 + 1)"),
        "last expression was lost:\n{output}"
    );
    assert_eq!(
        formatted(&output),
        output,
        "large output was not idempotent"
    );
}
