//! Integration tests for the `sycamorefmt` command-line interface.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
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

fn temporary_directory(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before the Unix epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "sycamorefmt-cli-{label}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&dir).expect("failed to create test directory");
    dir
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
    assert!(help_text.contains("Accepted for compatibility"));
    assert!(help_text.contains("no effect on formatting"));
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

    let input = b"fn main(){view!{div{\"hello\"}};}\n";
    let default = run_with_stdin(&[], input);
    assert!(default.status.success(), "{}", stderr(&default));
    for edition in ["2015", "2018", "2021", "2024"] {
        let output = run_with_stdin(&["--edition", edition], input);
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(
            output.stdout, default.stdout,
            "edition {edition} changed output"
        );
    }

    let positive_width = run(&["--max-width", "1", "--help"]);
    assert!(
        positive_width.status.success(),
        "{}",
        stderr(&positive_width)
    );
}

#[test]
fn file_format_replaces_the_original_with_formatted_contents() {
    let dir = temporary_directory("replace");
    let path = dir.join("input.rs");
    let input = b"fn main(){view!{div{\"hello\"}};}\n";
    fs::write(&path, input).expect("failed to create source file");

    let output = run(&[path.to_str().expect("test path is not UTF-8")]);
    assert!(output.status.success(), "{}", stderr(&output));
    let formatted = fs::read(&path).expect("failed to read replaced source file");
    assert_ne!(formatted, input);

    let check = run(&["--check", path.to_str().expect("test path is not UTF-8")]);
    assert!(check.status.success(), "{}", stderr(&check));
    fs::remove_dir_all(dir).expect("failed to remove test directory");
}

#[cfg(unix)]
#[test]
fn file_format_preserves_existing_mode() {
    use std::os::unix::fs::PermissionsExt;

    let dir = temporary_directory("mode");
    let path = dir.join("input.rs");
    fs::write(&path, b"fn main(){view!{div{\"hello\"}};}\n").expect("failed to create source file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640))
        .expect("failed to set source file mode");

    let output = run(&[path.to_str().expect("test path is not UTF-8")]);
    assert!(output.status.success(), "{}", stderr(&output));
    let mode = fs::metadata(&path)
        .expect("failed to inspect formatted source file")
        .permissions()
        .mode()
        & 0o7777;
    assert_eq!(mode, 0o640);
    fs::remove_dir_all(dir).expect("failed to remove test directory");
}

#[cfg(unix)]
#[test]
fn file_format_does_not_replace_read_only_source() {
    use std::os::unix::fs::PermissionsExt;

    let dir = temporary_directory("readonly");
    let path = dir.join("input.rs");
    let input = b"fn main(){view!{div{\"hello\"}};}\n";
    fs::write(&path, input).expect("failed to create source file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    // Privileged users can still open read-only files for writing.
    if fs::OpenOptions::new().write(true).open(&path).is_err() {
        let output = run(&[path.to_str().unwrap()]);
        assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
        assert_eq!(fs::read(&path).unwrap(), input);
    }
    fs::remove_dir_all(dir).expect("failed to remove test directory");
}

#[cfg(unix)]
#[test]
fn file_format_follows_symlinks_without_replacing_them() {
    use std::os::unix::fs::symlink;

    let dir = temporary_directory("symlink");
    let target = dir.join("target.rs");
    let link = dir.join("link.rs");
    let input = b"fn main(){view!{div{\"hello\"}};}\n";
    fs::write(&target, input).expect("failed to create target source file");
    symlink(&target, &link).expect("failed to create source symlink");

    let output = run(&[link.to_str().expect("test path is not UTF-8")]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        fs::symlink_metadata(&link)
            .expect("failed to inspect source symlink")
            .file_type()
            .is_symlink(),
        "formatter replaced the symlink"
    );
    let formatted = fs::read(&target).expect("failed to read symlink target");
    assert_ne!(formatted, input);
    assert_eq!(
        fs::read(&link).expect("failed to read through symlink"),
        formatted
    );
    fs::remove_dir_all(dir).expect("failed to remove test directory");
}

#[cfg(unix)]
#[test]
fn partial_temporary_write_failure_leaves_original_intact() {
    let dir = temporary_directory("partial-write");
    let path = dir.join("large.rs");
    let input = (0..64)
        .map(|index| format!("fn item_{index}(){{let _=view!{{div{{\"hello\"}}}};}}\n"))
        .collect::<String>();
    fs::write(&path, input.as_bytes()).expect("failed to create source file");

    // Ignore SIGXFSZ and limit regular-file writes to 512 bytes. This makes the
    // formatter's temporary output write fail after a partial write on Unix.
    let output = Command::new("sh")
        .arg("-c")
        .arg("trap '' XFSZ; ulimit -f 1 || exit 77; exec \"$1\" \"$2\"")
        .arg("sh")
        .arg(env!("CARGO_BIN_EXE_sycamorefmt"))
        .arg(&path)
        .output()
        .expect("failed to run sycamorefmt under a file-size limit");
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("failed to write"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        fs::read(&path).expect("failed to read original source"),
        input.as_bytes()
    );
    assert_eq!(
        fs::read_dir(&dir)
            .expect("failed to inspect test directory")
            .count(),
        1,
        "temporary output file was not cleaned up"
    );
    fs::remove_dir_all(dir).expect("failed to remove test directory");
}

#[test]
fn stdin_check_and_file_modes_keep_their_exit_behavior() {
    let input = b"fn main(){view!{div{\"hello\"}};}\n";
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
