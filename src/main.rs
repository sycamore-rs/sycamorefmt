//! Command-line interface for `sycamorefmt`.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use sycamorefmt::{Config, format_source};

/// Format Sycamore's `view! { ... }` macro syntax.
#[derive(Debug, Parser)]
#[command(
    version,
    about,
    long_about = "A formatter for Sycamore's `view! { ... }` macro syntax."
)]
struct Cli {
    /// Don't write any files; exit with a non-zero status if any input would be reformatted.
    #[arg(long)]
    check: bool,

    /// Fail and leave affected inputs unchanged if formatting emits warnings.
    #[arg(long)]
    fail_on_warnings: bool,

    /// Accepted for compatibility; currently has no effect on formatting.
    #[arg(long, default_value = "2024", value_parser = parse_edition)]
    edition: String,

    /// Maximum line width.
    #[arg(long, default_value_t = 100, value_parser = parse_max_width)]
    max_width: usize,

    /// Files to format. With no files, read one Rust source file from stdin and write to stdout.
    #[arg(value_name = "FILES")]
    paths: Vec<PathBuf>,
}

fn parse_edition(value: &str) -> Result<String, String> {
    match value {
        "2015" | "2018" | "2021" | "2024" => Ok(value.to_owned()),
        _ => Err(format!(
            "invalid edition {value:?}; expected one of: 2015, 2018, 2021, 2024"
        )),
    }
}

fn parse_max_width(value: &str) -> Result<usize, String> {
    let width = value
        .parse::<usize>()
        .map_err(|_| format!("invalid max width {value:?}; expected a positive integer"))?;
    if width == 0 {
        return Err("max width must be greater than zero".to_owned());
    }
    Ok(width)
}

fn main() -> ExitCode {
    let args = Cli::parse();
    let config = Config::new(args.max_width, args.edition.clone());

    if args.paths.is_empty() {
        run_stdin(&args, &config)
    } else {
        run_files(&args, &config)
    }
}

fn run_stdin(args: &Cli, config: &Config) -> ExitCode {
    let mut input = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut input) {
        eprintln!("error: failed to read stdin: {e}");
        return ExitCode::from(2);
    }

    match format_source(&input, config) {
        Ok(outcome) => {
            for warning in &outcome.warnings {
                eprintln!("warning: {warning}");
            }
            if args.fail_on_warnings && !outcome.warnings.is_empty() {
                return ExitCode::from(2);
            }
            if args.check {
                if outcome.changed {
                    eprintln!("input would be reformatted");
                    ExitCode::from(1)
                } else {
                    ExitCode::SUCCESS
                }
            } else {
                if io::stdout().write_all(outcome.output.as_bytes()).is_err() {
                    return ExitCode::from(2);
                }
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

fn write_file_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    // Resolve the destination first so replacing a symlink updates its target instead of
    // replacing the symlink itself with a regular file.
    let destination = fs::canonicalize(path)?;
    let metadata = fs::metadata(&destination)?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination is not a regular file",
        ));
    }
    // Replacing a file only requires write permission on its directory, but
    // formatting a read-only source should still fail as an in-place write
    // would. Opening without truncation checks that permission safely.
    fs::OpenOptions::new().write(true).open(&destination)?;
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::other("destination has no parent directory"))?;

    // NamedTempFile creates a collision-resistant file with create_new semantics and removes
    // it on drop if any write, flush, permission, or replacement step fails.
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(contents)?;
    temporary.flush()?;
    temporary
        .as_file()
        .set_permissions(metadata.permissions())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&destination)
        .map_err(|error| error.error)?;
    Ok(())
}

fn run_files(args: &Cli, config: &Config) -> ExitCode {
    let mut any_changed = false;
    let mut any_error = false;

    for path in &args.paths {
        let input = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: failed to read {}: {e}", path.display());
                any_error = true;
                continue;
            }
        };

        match format_source(&input, config) {
            Ok(outcome) => {
                for warning in &outcome.warnings {
                    eprintln!("{}: warning: {warning}", path.display());
                }
                if args.fail_on_warnings && !outcome.warnings.is_empty() {
                    any_error = true;
                    continue;
                }
                if outcome.changed {
                    any_changed = true;
                    if args.check {
                        eprintln!("{}: would be reformatted", path.display());
                    } else if let Err(e) = write_file_atomically(path, outcome.output.as_bytes()) {
                        eprintln!("error: failed to write {}: {e}", path.display());
                        any_error = true;
                    }
                }
            }
            Err(e) => {
                eprintln!("{}: error: {e}", path.display());
                any_error = true;
            }
        }
    }

    if any_error {
        ExitCode::from(2)
    } else if args.check && any_changed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::write_file_atomically;
    use std::io::ErrorKind;

    #[test]
    fn refuses_to_replace_non_regular_destinations() {
        let dir = tempfile::tempdir().unwrap();
        let error = write_file_atomically(dir.path(), b"formatted").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert!(dir.path().is_dir());
    }
}
