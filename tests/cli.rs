//! Integration tests for the `sycamorefmt` command-line interface.

use std::fs;
use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sycamorefmt"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("failed to run sycamorefmt")
}

fn run_with_stdin(args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sycamorefmt"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run sycamorefmt");
    child
        .stdin
        .take()
        .expect("stdin was not piped")
        .write_all(input)
        .expect("failed to write test input");
    child.wait_with_output().expect("failed to collect output")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn rustfmt_available() -> bool {
    Command::new("rustfmt")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[test]
fn version_and_help_are_provided_by_clap() {
    let version = run(&["--version"]);
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout),
        format!("sycamorefmt {}\n", env!("CARGO_PKG_VERSION"))
    );

    let help = run(&["--help"]);
    assert!(help.status.success());
    let help_text = String::from_utf8_lossy(&help.stdout);
    assert!(help_text.contains("Usage: sycamorefmt [OPTIONS] [FILES]..."));
    assert!(help_text.contains("--check"));
    assert!(help_text.contains("--edition <EDITION>"));
    assert!(help_text.contains("--max-width <MAX_WIDTH>"));
    assert!(help_text.contains("-V, --version"));
}

#[test]
fn edition_and_max_width_are_validated() {
    let edition = run(&["--edition", "2017"]);
    assert_eq!(edition.status.code(), Some(2));
    assert!(stderr(&edition).contains("expected one of: 2015, 2018, 2021, 2024"));

    let zero_width = run(&["--max-width", "0"]);
    assert_eq!(zero_width.status.code(), Some(2));
    assert!(stderr(&zero_width).contains("max width must be greater than zero"));

    for edition in ["2015", "2018", "2021", "2024"] {
        let output = run(&["--edition", edition, "--help"]);
        assert!(output.status.success(), "{}", stderr(&output));
    }

    let positive_width = run(&["--max-width", "1", "--help"]);
    assert!(
        positive_width.status.success(),
        "{}",
        stderr(&positive_width)
    );
}

#[test]
fn stdin_check_and_file_modes_keep_their_exit_behavior() {
    if !rustfmt_available() {
        eprintln!("skipping CLI formatting test: `rustfmt` is not available on PATH");
        return;
    }

    let input = b"fn main(){println!(\"hello\");}\n";
    let formatted = run_with_stdin(&[], input);
    assert!(formatted.status.success(), "{}", stderr(&formatted));
    assert!(!formatted.stdout.is_empty());
    assert_ne!(formatted.stdout, input);

    let stdin_check = run_with_stdin(&["--check"], input);
    assert_eq!(stdin_check.status.code(), Some(1));
    assert!(stdin_check.stdout.is_empty());
    assert!(stderr(&stdin_check).contains("input would be reformatted"));

    let already_formatted = run_with_stdin(&["--check"], b"fn main() {}\n");
    assert!(
        already_formatted.status.success(),
        "{}",
        stderr(&already_formatted)
    );
    assert!(already_formatted.stdout.is_empty());

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sycamorefmt-cli-{}-{unique}.rs",
        std::process::id()
    ));
    fs::write(&path, input).expect("failed to create temporary Rust file");

    let path_string = path.to_str().expect("temporary path is not UTF-8");
    let file_check = run(&["--check", path_string]);
    assert_eq!(file_check.status.code(), Some(1));
    assert_eq!(
        fs::read(&path).expect("failed to read temporary Rust file"),
        input
    );

    let file_format = run(&[path_string]);
    assert!(file_format.status.success(), "{}", stderr(&file_format));
    assert_ne!(
        fs::read(&path).expect("failed to read formatted Rust file"),
        input
    );

    let file_check_again = run(&["--check", path_string]);
    assert!(
        file_check_again.status.success(),
        "{}",
        stderr(&file_check_again)
    );
    fs::remove_file(path).expect("failed to remove temporary Rust file");
}
