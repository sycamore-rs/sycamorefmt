//! Command-line interface for `sycamorefmt`.

use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use sycamorefmt::{format_source, Config};

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

    /// Rust edition to format for.
    #[arg(long, default_value = "2021", value_parser = parse_edition)]
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
                if outcome.changed {
                    any_changed = true;
                    if args.check {
                        eprintln!("{}: would be reformatted", path.display());
                    } else if let Err(e) = fs::write(path, &outcome.output) {
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
