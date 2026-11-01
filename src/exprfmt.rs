//! In-process Rust AST formatting for expressions and patterns in views.
//!
//! `prettyplease` prints a `syn::File`, so we put each AST in a small
//! synthetic function and extract its rendering. Afterwards we format any
//! nested `view!` macros in that rendering using the normal splice pipeline.
//! Opaque macro source is restored by the enclosing view splice.

use proc_macro2::{TokenStream, TokenTree};
use quote::quote;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Expr, Item, ItemFn, Pat, Stmt, Visibility};

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

    let wrapper: syn::File = syn::parse2(quote! {
        fn __sycamorefmt_expr__() { (#expr) }
    })
    .map_err(FmtError::Parse)?;
    let formatted = prettyplease::unparse(&wrapper);
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
    let parsed = syn::parse_file(&formatted).map_err(FmtError::Parse)?;
    let function = wrapper_function(&parsed, "__sycamorefmt_expr__")?;
    let expr = function_expression(function)?;
    let Expr::Paren(paren) = expr else {
        return Err(FmtError::Internal(
            "expression wrapper lost its parentheses".into(),
        ));
    };
    if !paren.attrs.is_empty() {
        return Err(FmtError::Internal(
            "expression wrapper has unexpected attributes".into(),
        ));
    }
    let (open, close) = delimiter_ranges(paren.paren_token.span);
    let outer = expr.span().byte_range();
    let inner = checked_inner_range(&formatted, outer, open, close, "expression wrapper")?;
    Ok(reindent(inner.trim(), 4, indent, cfg.tab_spaces))
}

pub fn format_pattern(pattern: &Pat, indent: usize, cfg: &Config) -> Result<String, FmtError> {
    let mut guard = PatternMacroGuard::default();
    guard.visit_pat(pattern);
    if guard.found {
        return Err(FmtError::Internal(
            "cannot safely format a pattern containing a macro invocation".into(),
        ));
    }

    let wrapper: syn::File = syn::parse2(quote! {
        fn __sycamorefmt_pattern__() {
            match __sycamorefmt_value { #pattern => (), }
        }
    })
    .map_err(FmtError::Parse)?;
    let formatted = prettyplease::unparse(&wrapper);
    let parsed = syn::parse_file(&formatted).map_err(FmtError::Parse)?;
    let function = wrapper_function(&parsed, "__sycamorefmt_pattern__")?;
    let Expr::Match(match_expr) = function_expression(function)? else {
        return Err(FmtError::Internal(
            "pattern wrapper lost its synthetic match expression".into(),
        ));
    };

    if !match_expr.attrs.is_empty()
        || !is_placeholder_path(&match_expr.expr)
        || match_expr.arms.len() != 1
    {
        return Err(FmtError::Internal(
            "pattern wrapper has an unexpected match structure".into(),
        ));
    }
    let arm = &match_expr.arms[0];
    if !arm.attrs.is_empty() || !is_unit(&arm.body) {
        return Err(FmtError::Internal(
            "pattern wrapper has an unexpected synthetic match arm".into(),
        ));
    }

    let pattern_range = arm.pat.span().byte_range();
    let (open, close) = delimiter_ranges(match_expr.brace_token.span);
    let match_range = match_expr.span().byte_range();
    let pattern = checked_contained_range(
        &formatted,
        pattern_range,
        match_range,
        open,
        close,
        "match pattern",
    )?;
    let pattern = pattern.trim();
    // In syn 3, a match guard is part of Pat::Guard rather than a separate
    // Arm field, so the pattern span must cover both the pattern and guard.
    let reparsed: syn::Arm =
        syn::parse_str(&format!("{pattern} => (),")).map_err(FmtError::Parse)?;
    if reparsed.pat != arm.pat || !is_unit(&reparsed.body) {
        return Err(FmtError::Internal(
            "formatted match pattern span does not match its syntax tree".into(),
        ));
    }
    Ok(reindent(pattern, 8, indent, cfg.tab_spaces))
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

fn wrapper_function<'a>(file: &'a syn::File, expected_name: &str) -> Result<&'a ItemFn, FmtError> {
    if file.shebang.is_some() || file.frontmatter.is_some() || !file.attrs.is_empty() {
        return Err(FmtError::Internal(
            "formatted wrapper has unexpected file-level syntax".into(),
        ));
    }
    let [Item::Fn(function)] = file.items.as_slice() else {
        return Err(FmtError::Internal(
            "formatted wrapper is not exactly one synthetic function".into(),
        ));
    };
    let signature = &function.sig;
    if !function.attrs.is_empty()
        || !matches!(function.vis, Visibility::Inherited)
        || signature.ident != expected_name
        || signature.constness.is_some()
        || signature.asyncness.is_some()
        || !matches!(signature.safety, syn::Safety::Default)
        || signature.abi.is_some()
        || signature.generics.lt_token.is_some()
        || signature.generics.gt_token.is_some()
        || !signature.generics.params.is_empty()
        || signature.generics.where_clause.is_some()
        || !signature.inputs.is_empty()
        || signature.variadic.is_some()
        || !matches!(signature.output, syn::ReturnType::Default)
    {
        return Err(FmtError::Internal(
            "formatted wrapper has an unexpected synthetic function signature".into(),
        ));
    }
    Ok(function)
}

fn function_expression(function: &ItemFn) -> Result<&Expr, FmtError> {
    let [Stmt::Expr(expr, None)] = function.block.stmts.as_slice() else {
        return Err(FmtError::Internal(
            "formatted wrapper has an unexpected statement structure".into(),
        ));
    };
    Ok(expr)
}

fn is_placeholder_path(expr: &Expr) -> bool {
    let Expr::Path(path) = expr else {
        return false;
    };
    path.attrs.is_empty()
        && path.qself.is_none()
        && path.path.leading_colon.is_none()
        && path.path.segments.len() == 1
        && path.path.segments[0].ident == "__sycamorefmt_value"
        && matches!(path.path.segments[0].arguments, syn::PathArguments::None)
}

fn is_unit(expr: &Expr) -> bool {
    match expr {
        Expr::Tuple(tuple) => tuple.attrs.is_empty() && tuple.elems.is_empty(),
        Expr::Block(block) => {
            block.attrs.is_empty() && block.label.is_none() && block.block.stmts.is_empty()
        }
        _ => false,
    }
}

fn delimiter_ranges(
    span: proc_macro2::extra::DelimSpan,
) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
    (span.open().byte_range(), span.close().byte_range())
}

fn checked_range<'a>(
    src: &'a str,
    range: std::ops::Range<usize>,
    what: &str,
) -> Result<&'a str, FmtError> {
    if range.start > range.end
        || range.end > src.len()
        || !src.is_char_boundary(range.start)
        || !src.is_char_boundary(range.end)
    {
        return Err(FmtError::Internal(format!(
            "formatted {what} has an unsafe source range"
        )));
    }
    src.get(range)
        .ok_or_else(|| FmtError::Internal(format!("formatted {what} has an unsafe source range")))
}

fn checked_inner_range<'a>(
    src: &'a str,
    outer: std::ops::Range<usize>,
    open: std::ops::Range<usize>,
    close: std::ops::Range<usize>,
    what: &str,
) -> Result<&'a str, FmtError> {
    checked_range(src, outer.clone(), what)?;
    checked_range(src, open.clone(), what)?;
    checked_range(src, close.clone(), what)?;
    if open.end - open.start != 1
        || close.end - close.start != 1
        || open.start != outer.start
        || close.end != outer.end
        || open.end > close.start
        || checked_range(src, open.clone(), what)? != "("
        || checked_range(src, close.clone(), what)? != ")"
    {
        return Err(FmtError::Internal(format!(
            "formatted {what} lost its expected parentheses"
        )));
    }
    checked_range(src, open.end..close.start, what)
}

fn checked_contained_range<'a>(
    src: &'a str,
    content: std::ops::Range<usize>,
    outer: std::ops::Range<usize>,
    open: std::ops::Range<usize>,
    close: std::ops::Range<usize>,
    what: &str,
) -> Result<&'a str, FmtError> {
    checked_range(src, outer.clone(), what)?;
    checked_range(src, content.clone(), what)?;
    checked_range(src, open.clone(), what)?;
    checked_range(src, close.clone(), what)?;
    if open.end - open.start != 1
        || close.end - close.start != 1
        || open.start < outer.start
        || close.end != outer.end
        || open.end > close.start
        || content.start < open.end
        || content.end > close.start
        || checked_range(src, open.clone(), what)? != "{"
        || checked_range(src, close.clone(), what)? != "}"
    {
        return Err(FmtError::Internal(format!(
            "formatted {what} has an unsafe or unexpected source range"
        )));
    }
    checked_range(src, content, what)
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
