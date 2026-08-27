//! Shared error type for the formatting pipeline.

use std::fmt;

use crate::rustfmt::RustfmtError;

#[derive(Debug)]
pub enum FmtError {
    /// The input could not be parsed as a Rust file at all.
    Parse(syn::Error),
    /// Invoking `rustfmt` failed.
    Rustfmt(RustfmtError),
    /// Some other internal invariant was violated (e.g. an unexpected byte
    /// span). These should be rare/never happen, but we surface them as
    /// errors rather than panicking so a single malformed file can't crash
    /// the whole run.
    Internal(String),
    Io(std::io::Error),
}

impl fmt::Display for FmtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FmtError::Parse(e) => write!(f, "failed to parse Rust source: {e}"),
            FmtError::Rustfmt(e) => write!(f, "{e}"),
            FmtError::Internal(msg) => write!(f, "internal error: {msg}"),
            FmtError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for FmtError {}

impl From<RustfmtError> for FmtError {
    fn from(e: RustfmtError) -> Self {
        FmtError::Rustfmt(e)
    }
}

impl From<std::io::Error> for FmtError {
    fn from(e: std::io::Error) -> Self {
        FmtError::Io(e)
    }
}
