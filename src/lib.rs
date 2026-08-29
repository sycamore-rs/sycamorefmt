//! `sycamorefmt`: a formatter for Sycamore's `view! { ... }` macro syntax,
//! built on top of `rustfmt` and the project's own `sycamore-view-parser`
//! crate.
//!
//! The public entry point is [`format_source`], which formats a complete
//! Rust source file (or self-contained snippet) and reports whether
//! anything changed.

mod config;
mod error;
mod exprfmt;
mod finder;
mod format;
mod printer;
mod rustfmt;
mod trivia;

pub use config::Config;
pub use error::FmtError;
pub use format::{FormatOutcome, format_source};
