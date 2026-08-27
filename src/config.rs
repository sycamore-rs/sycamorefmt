//! Formatting configuration shared across the CLI and the formatting pipeline.

/// Configuration controlling how `sycamorefmt` formats a piece of Rust source.
#[derive(Debug, Clone)]
pub struct Config {
    /// Maximum line width to aim for, both for the surrounding Rust code
    /// (passed through to `rustfmt` as `max_width`) and for the `view! { ... }`
    /// pretty-printer.
    pub max_width: usize,
    /// Rust edition to pass to `rustfmt` (and to use when parsing input, if
    /// that ever becomes edition-sensitive).
    pub edition: String,
    /// Number of spaces per indentation level.
    pub tab_spaces: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_width: 100,
            edition: "2021".to_string(),
            tab_spaces: 4,
        }
    }
}

impl Config {
    pub fn new(max_width: usize, edition: String) -> Self {
        Self {
            max_width,
            edition,
            ..Default::default()
        }
    }
}
