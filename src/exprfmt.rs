//! In-process Rust AST formatting for expressions and patterns in views.
//!
//! `prettyplease` prints a `syn::File`, so we put each AST in a small
//! synthetic function and extract its rendering. Afterwards we format any
//! nested `view!` macros in that rendering using the normal splice pipeline.
//! Opaque macro source is restored by the enclosing view splice.

use proc_macro2::{TokenStream, TokenTree};
use quote::quote;
use syn::visit::{self, Visit};
use syn::{Expr, Pat};

use crate::config::Config;
use crate::error::FmtError;

pub fn format_expr(expr: &Expr, indent: usize, cfg: &Config) -> Result<String, FmtError> {
    let mut guard = PatternMacroGuard::default();
    guard.visit_expr(expr);
    if guard.found {
        return Err(FmtError::Internal(
            "cannot safely format an expression containing a macro in a pattern".into(),
        ));
    }

    let file: syn::File = syn::parse2(quote! {
        fn __sycamorefmt_expr__() { (#expr) }
    })
    .map_err(FmtError::Parse)?;
    let formatted = prettyplease::unparse(&file);
    let mut finder = crate::finder::MacroFinder::new();
    finder.visit_expr(expr);
    let formatted = if finder.found.is_empty() {
        formatted
    } else {
        // The wrapper has prettyplease's four-space indentation. Render nested
        // views in that same coordinate system; reindent the whole expression
        // to the caller's configured width exactly once below.
        let mut wrapper_cfg = cfg.clone();
        wrapper_cfg.tab_spaces = 4;
        let outcome = crate::format::format_source(&formatted, &wrapper_cfg)?;
        if !outcome.warnings.is_empty() {
            return Err(FmtError::Internal(format!(
                "could not format nested view: {}",
                outcome.warnings.join("; ")
            )));
        }
        outcome.output
    };
    let body = function_body(&formatted)?;
    let body = body.trim();
    let body = body
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(|| FmtError::Internal("expression wrapper lost its parentheses".into()))?;
    Ok(reindent(body, 4, indent, cfg.tab_spaces))
}

pub fn format_pattern(pattern: &Pat, indent: usize, cfg: &Config) -> Result<String, FmtError> {
    let mut guard = PatternMacroGuard::default();
    guard.visit_pat(pattern);
    if guard.found {
        return Err(FmtError::Internal(
            "cannot safely format a pattern containing a macro invocation".into(),
        ));
    }

    let file: syn::File = syn::parse2(quote! {
        fn __sycamorefmt_pattern__() {
            match __sycamorefmt_value { #pattern => (), }
        }
    })
    .map_err(FmtError::Parse)?;
    let formatted = prettyplease::unparse(&file);
    let start = formatted
        .find("match __sycamorefmt_value {\n")
        .map(|p| p + "match __sycamorefmt_value {\n".len())
        .ok_or_else(|| FmtError::Internal("could not locate formatted match pattern".into()))?;
    let end = formatted
        .rfind(" => {}")
        .filter(|&p| p >= start)
        .ok_or_else(|| {
            FmtError::Internal(format!("could not locate formatted match arm: {formatted}"))
        })?;
    Ok(reindent(
        formatted[start..end].trim_start(),
        8,
        indent,
        cfg.tab_spaces,
    ))
}

// Pattern macros can have grammar and expansion behavior unknown to syn.
// Decline the entire view instead of guessing how to restore their tokens.
#[derive(Default)]
struct PatternMacroGuard {
    in_pattern: bool,
    found: bool,
}

impl<'ast> Visit<'ast> for PatternMacroGuard {
    fn visit_pat(&mut self, pattern: &'ast Pat) {
        let before = self.in_pattern;
        self.in_pattern = true;
        visit::visit_pat(self, pattern);
        self.in_pattern = before;
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if self.in_pattern {
            self.found = true;
        }
        visit::visit_macro(self, mac);
    }
}

fn function_body(formatted: &str) -> Result<&str, FmtError> {
    let start = formatted
        .find('{')
        .map(|p| p + 1)
        .ok_or_else(|| FmtError::Internal("could not locate expression wrapper".into()))?;
    let end = formatted
        .rfind('}')
        .filter(|&p| p >= start)
        .ok_or_else(|| FmtError::Internal("could not locate expression wrapper end".into()))?;
    Ok(&formatted[start..end])
}

// prettyplease uses four spaces per level. Translate indentation to the
// requested tab width when embedding the result in the view printer.
fn reindent(body: &str, from: usize, to: usize, tab_spaces: usize) -> String {
    // Leading whitespace inside a multiline literal is data, not indentation.
    fn literals(tokens: TokenStream, ranges: &mut Vec<std::ops::Range<usize>>) {
        for token in tokens {
            match token {
                TokenTree::Literal(lit) => ranges.push(lit.span().byte_range()),
                TokenTree::Group(group) => literals(group.stream(), ranges),
                _ => {}
            }
        }
    }
    let mut ranges = Vec::new();
    if let Ok(tokens) = body.parse() {
        literals(tokens, &mut ranges);
    }
    let mut offset = 0;
    body.split('\n')
        .enumerate()
        .map(|(i, line)| {
            let start = offset;
            offset += line.len() + 1;
            let preceding = ranges.partition_point(|range| range.start < start);
            if i == 0 || (preceding > 0 && start < ranges[preceding - 1].end) {
                return line.to_string();
            }
            if line.trim().is_empty() {
                return String::new();
            }
            let spaces = line.bytes().take_while(|b| *b == b' ').count();
            let relative = spaces.saturating_sub(from);
            let translated = (relative / 4) * tab_spaces + relative % 4;
            format!("{}{}", " ".repeat(to + translated), &line[spaces..])
        })
        .collect::<Vec<_>>()
        .join("\n")
}
