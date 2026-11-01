//! CLI behavior when skipped views are treated as errors.

use std::io::Write;
use std::process::{Command, Output, Stdio};

fn stdin(args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sycamorefmt"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

const WITH_WARNING: &str =
    "fn f() { view!{div{\"format me\"}}; view!{div{/* preserve */ \"skip\"}}; }\n";

#[test]
fn strict_stdin_withholds_partial_output() {
    for args in [
        &["--fail-on-warnings"][..],
        &["--fail-on-warnings", "--check"][..],
    ] {
        let output = stdin(args, WITH_WARNING);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("warning:"));
    }
    let default = stdin(&[], WITH_WARNING);
    assert!(default.status.success());
    assert!(!default.stdout.is_empty());
}

#[test]
fn strict_files_do_not_write_affected_inputs_and_continue_others() {
    let dir = tempfile::tempdir().unwrap();
    let affected = dir.path().join("affected.rs");
    let clean = dir.path().join("clean.rs");
    let clean_input = "fn f() { view!{div{\"format me\"}}; }\n";
    std::fs::write(&affected, WITH_WARNING).unwrap();
    std::fs::write(&clean, clean_input).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_sycamorefmt"))
        .arg("--fail-on-warnings")
        .arg(&affected)
        .arg(&clean)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(std::fs::read_to_string(&affected).unwrap(), WITH_WARNING);
    assert_ne!(std::fs::read_to_string(&clean).unwrap(), clean_input);
    assert!(String::from_utf8_lossy(&output.stderr).contains("warning:"));
}

#[test]
fn strict_check_reports_skipped_invalid_views() {
    let input = "fn f() { view! { div(=) } }\n";
    let output = stdin(&["--check", "--fail-on-warnings"], input);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to parse"));
}

#[test]
fn strict_check_reports_unsupported_view_delimiters() {
    for input in ["fn f() { view!(div()); }\n", "fn f() { view![div()]; }\n"] {
        let output = stdin(&["--check", "--fail-on-warnings"], input);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("warning:"));
    }
}

#[test]
fn strict_mode_keeps_normal_check_statuses() {
    assert!(
        stdin(&["--check", "--fail-on-warnings"], "fn f() {}\n")
            .status
            .success()
    );
    assert_eq!(
        stdin(
            &["--check", "--fail-on-warnings"],
            "fn f() { view!{div{}} }\n"
        )
        .status
        .code(),
        Some(1)
    );
}
