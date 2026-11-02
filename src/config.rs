//! Formatting configuration shared across the CLI and the formatting pipeline.

use crate::error::FmtError;

/// Configuration controlling how `sycamorefmt` formats a piece of Rust source.
#[derive(Debug, Clone)]
pub struct Config {
    /// Maximum display-column width to aim for in the `view! { ... }` printer.
    /// Unicode ambiguous-width characters count as one column; embedded Rust
    /// expressions use prettyplease's fixed width.
    pub max_width: usize,
    /// Retained for CLI/API compatibility; parsing and printing are currently
    /// edition-independent.
    pub edition: String,
    /// Number of spaces per indentation level.
    pub tab_spaces: usize,
}

/// Allocate indentation without turning an unrepresentable public configuration
/// (or an allocation failure reported by the allocator) into a capacity panic.
/// This is not a resource limit: huge allocations may still exhaust OS memory.
pub(crate) fn indent_padding(width: usize) -> Result<String, FmtError> {
    let mut padding = String::new();
    padding.try_reserve_exact(width).map_err(|error| {
        FmtError::Internal(format!("could not allocate indentation padding: {error}"))
    })?;
    padding.extend(std::iter::repeat_n(' ', width));
    debug_assert_eq!(padding.len(), width);
    Ok(padding)
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
