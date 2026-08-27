//! Command-line interface for `sycamorefmt`.

use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use sycamorefmt::{format_source, Config};

const USAGE: &str = "\
sycamorefmt: a formatter for Sycamore's `view! { ... }` macro syntax

USAGE:
    sycamorefmt [OPTIONS] [FILES...]

    With no FILES, reads a single Rust source file from stdin and writes the
    formatted result to stdout. With one or more FILES, formats each file in
    place (unless --check is given).

OPTIONS:
    --check              Don't write any files; exit with a non-zero status
                          if any input would be reformatted.
    --edition <EDITION>   Rust edition to format for (default: 2021).
    --max-width <WIDTH>   Maximum line width (default: 100).
    -h, --help            Print this help message.
";

struct Args {
    paths: Vec<PathBuf>,
    check: bool,
    config: Config,
}

fn parse_args() -> Result<Args, String> {
    let mut config = Config::default();
    let mut check = false;
    let mut paths = Vec::new();

    let mut raw = std::env::args().skip(1);
    while let Some(arg) = raw.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--check" => check = true,
            "--edition" => {
                let value = raw.next().ok_or("--edition requires a value")?;
                config.edition = value;
            }
            "--max-width" => {
                let value = raw.next().ok_or("--max-width requires a value")?;
                config.max_width = value
                    .parse()
                    .map_err(|_| format!("invalid --max-width value: {value}"))?;
            }
            other if other.starts_with("--edition=") => {
                config.edition = other["--edition=".len()..].to_string();
            }
            other if other.starts_with("--max-width=") => {
                let value = &other["--max-width=".len()..];
                config.max_width = value
                    .parse()
                    .map_err(|_| format!("invalid --max-width value: {value}"))?;
            }
            other if other.starts_with('-') => {
                return Err(format!("unrecognized option: {other}"));
            }
            other => paths.push(PathBuf::from(other)),
        }
    }

    Ok(Args {
        paths,
        check,
        config,
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(msg) => {
            eprintln!("error: {msg}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    if args.paths.is_empty() {
        run_stdin(&args)
    } else {
        run_files(&args)
    }
}

fn run_stdin(args: &Args) -> ExitCode {
    let mut input = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut input) {
        eprintln!("error: failed to read stdin: {e}");
        return ExitCode::from(2);
    }

    match format_source(&input, &args.config) {
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

fn run_files(args: &Args) -> ExitCode {
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

        match format_source(&input, &args.config) {
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
