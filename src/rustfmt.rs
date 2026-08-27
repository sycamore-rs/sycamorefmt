//! Thin wrapper around invoking the system `rustfmt` binary as a subprocess.
//!
//! `sycamorefmt` does not reimplement general Rust formatting: it delegates
//! everything outside of `view! { ... }` bodies to `rustfmt`, and only adds
//! its own pretty-printer for the view-macro grammar itself.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::config::Config;

#[derive(Debug)]
pub enum RustfmtError {
    /// The `rustfmt` executable could not be found or spawned.
    NotFound(std::io::Error),
    /// `rustfmt` ran but reported a formatting/parse error.
    Failed { stderr: String },
    /// Its output was not valid UTF-8, or some other I/O error occurred.
    Io(std::io::Error),
}

impl std::fmt::Display for RustfmtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RustfmtError::NotFound(e) => {
                write!(f, "could not run `rustfmt` (is it installed and on PATH?): {e}")
            }
            RustfmtError::Failed { stderr } => {
                write!(f, "rustfmt failed:\n{stderr}")
            }
            RustfmtError::Io(e) => write!(f, "I/O error while running rustfmt: {e}"),
        }
    }
}

impl std::error::Error for RustfmtError {}

/// Runs `rustfmt` on `input`, returning the formatted source.
///
/// Uses `--emit stdout` with no file argument, which makes `rustfmt` read the
/// full source from stdin and write the formatted result to stdout.
pub fn run(input: &str, cfg: &Config) -> Result<String, RustfmtError> {
    let mut child = Command::new("rustfmt")
        .arg("--edition")
        .arg(&cfg.edition)
        .arg("--config")
        .arg(format!("max_width={}", cfg.max_width))
        .arg("--config")
        .arg(format!("tab_spaces={}", cfg.tab_spaces))
        .arg("--emit")
        .arg("stdout")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(RustfmtError::NotFound)?;

    // Write on a scoped block so the stdin handle is dropped (closing the
    // pipe) before we wait for the child to exit.
    {
        let stdin = child.stdin.as_mut().expect("stdin was piped");
        stdin
            .write_all(input.as_bytes())
            .map_err(RustfmtError::Io)?;
    }

    let output = child.wait_with_output().map_err(RustfmtError::Io)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(RustfmtError::Failed { stderr });
    }

    String::from_utf8(output.stdout).map_err(|e| RustfmtError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        e,
    )))
}
