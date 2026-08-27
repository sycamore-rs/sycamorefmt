//! Locates `view! { ... }` macro invocations within a parsed Rust file.
//!
//! Only "top-level" invocations that `syn`'s AST walker can see are found
//! here. A `view!` macro nested inside the token stream of another (already
//! opaque) macro invocation -- for example a `view! { ... }` written inside an
//! expression that is itself inside another `view! { ... }` -- is invisible to
//! `syn::visit::Visit`, because `syn` does not know how to parse the inner
//! macro's grammar and therefore stores its contents as an opaque
//! `TokenStream`. Those nested invocations are instead handled recursively by
//! [`crate::exprfmt`], which re-runs the whole pipeline over the
//! re-serialized tokens of any expression that contains them.

use syn::visit::{self, Visit};
use syn::Macro;

/// A `view!`-like macro invocation found while walking a `syn::File`.
#[derive(Debug, Clone)]
pub struct FoundMacro {
    pub mac: Macro,
}

/// Visitor that collects every macro invocation whose path's last segment is
/// `view`, in source order.
#[derive(Default)]
pub struct MacroFinder {
    pub found: Vec<FoundMacro>,
}

impl MacroFinder {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Returns true if `mac`'s path looks like an invocation of the `view!`
/// macro, i.e. its last path segment is literally `view`. This matches both
/// `view!{...}` and qualified paths such as `sycamore::view!{...}`.
pub fn is_view_macro(mac: &Macro) -> bool {
    mac.path
        .segments
        .last()
        .map(|seg| seg.ident == "view")
        .unwrap_or(false)
}

impl<'ast> Visit<'ast> for MacroFinder {
    fn visit_macro(&mut self, node: &'ast Macro) {
        if is_view_macro(node) {
            self.found.push(FoundMacro { mac: node.clone() });
        }
        // Do not recurse into `node`: `syn::visit::visit_macro`'s default
        // implementation only visits `node.path`, which is exactly what we
        // want since `node.tokens` is opaque to `syn` anyway.
        visit::visit_macro(self, node);
    }
}
