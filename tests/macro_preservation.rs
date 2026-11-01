//! Opaque Rust macro invocations inside view expressions must retain their
//! original source while ordinary nested view macros remain formattable.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn run_formatter(input: &str) -> Output {
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
    let output = run_formatter(input);
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
fn preserves_multiline_opaque_macros_and_raw_strings_and_is_idempotent() {
    let macro_source = "opaque! {\r\n            first,\n            r###\"raw text\r\n                with leading whitespace\"###,\r\n            second\r\n        }";
    let input = format!(
        r###"fn render() -> View {{
    view! {{
        div {{
            ({{
                let a = format!(r#"{{}}"#, 1);
                let b = {macro_source};
                (a, b)
            }})
        }}
    }}
}}
"###
    );

    let output = formatted(&input);
    assert!(
        output.contains(macro_source),
        "opaque macro source changed:\n{output}"
    );
    assert!(output.contains("format!(r#\"{}\"#, 1)"), "{output}");
    assert_eq!(formatted(&output), output, "formatting was not idempotent");
}

#[test]
fn restores_macro_rules_statements_and_multiple_macro_invocations() {
    let macro_rules_source =
        r##"macro_rules! choose { () => { stringify!(view!{span{"hidden"}}) }; }"##;
    let call_source = "choose!()";
    let input = format!(
        r###"fn render() -> View {{
    view! {{
        div {{
            ({{
                {macro_rules_source}
                let value = {call_source};
                let other = opaque!(view!{{span{{"also hidden"}}}});
                (value, other)
            }})
        }}
    }}
}}
"###
    );

    let output = formatted(&input);
    assert!(
        output.contains(macro_rules_source),
        "macro_rules source changed:\n{output}"
    );
    assert!(output.contains(call_source), "{output}");
    assert!(
        output.contains("opaque!(view!{span{\"also hidden\"}})"),
        "opaque nested view tokens changed:\n{output}"
    );
    assert_eq!(output.matches("view! {").count(), 1, "{output}");
    assert_eq!(formatted(&output), output, "formatting was not idempotent");
}

#[test]
fn still_formats_nested_views_in_ordinary_expression_syntax() {
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
    assert_eq!(formatted(&output), output, "formatting was not idempotent");
}

fn rustc_stdout(source: &str, label: &str) -> String {
    let directory = tempfile::tempdir().expect("failed to create rustc temp directory");
    let source_path = directory.path().join(format!("{label}.rs"));
    let binary_path = directory
        .path()
        .join(format!("{label}{}", std::env::consts::EXE_SUFFIX));
    fs::write(&source_path, source).expect("failed to write rustc input");

    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let compile = Command::new(rustc)
        .arg("--edition=2024")
        .arg(&source_path)
        .arg("-o")
        .arg(&binary_path)
        .output()
        .expect("failed to invoke rustc");
    assert!(
        compile.status.success(),
        "rustc failed:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = Command::new(binary_path)
        .output()
        .expect("failed to execute stringify test binary");
    assert!(run.status.success(), "stringify test binary failed");
    String::from_utf8(run.stdout).expect("stringify output was not UTF-8")
}

#[test]
fn preserves_actual_stringify_semantics() {
    let input = r#"macro_rules! view {
    (div { ($expression:expr) }) => { $expression };
}
fn main() {
    print!("{}", view!{div{(stringify!(view!{span{"x"}}))}});
}
"#;
    let output = formatted(input);

    assert!(
        output.contains("stringify!(view!{span{\"x\"}})"),
        "stringify input was not preserved:\n{output}"
    );
    assert_eq!(
        rustc_stdout(input, "original"),
        rustc_stdout(&output, "formatted"),
        "formatting changed the runtime value of stringify!"
    );
}

#[test]
fn skips_a_view_when_an_opaque_macro_occurs_in_a_pattern() {
    let input = r#"fn render(value: u8) -> View {
    view! {
        div {
            ({ match value { pattern!() => stringify!(value) } })
        }
    }
}
"#;
    let output = run_formatter(input);

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), input);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("pattern"),
        "expected a warning about the unpreservable pattern macro: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
