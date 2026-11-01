//! Locates `view! { ... }` macro invocations represented as nodes in a `syn` AST.
//!
//! The finder checks macro invocations reached while walking ordinary Rust
//! syntax, but never examines a macro's token stream. `syn` stores those tokens
//! as opaque input because it cannot know the invoked macro's grammar. Thus a
//! `view!` inside an arbitrary macro invocation (including a top-level call or
//! a call appearing in an expression) is not discovered and must not be
//! rewritten: that macro may inspect or stringify its input. By contrast,
//! [`crate::exprfmt`] parses Rust expressions found in an actual Sycamore view
//! body and runs this finder on their ASTs, so a `view!` nested in ordinary
//! expression syntax can be formatted recursively. That recursion stops at
//! every macro-token boundary.

use syn::Macro;
use syn::ext::IdentExt;
use syn::visit::{self, Visit};

/// A `view!`-like macro invocation found while walking a `syn::File`.
#[derive(Debug, Clone)]
pub struct FoundMacro {
    pub mac: Macro,
}

/// Visitor that collects visible macro AST nodes whose path's last segment is
/// `view`, in source order. It does not search macro token streams.
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
        .map(|seg| seg.ident.unraw() == "view")
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
