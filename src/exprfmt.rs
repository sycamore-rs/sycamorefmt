//! Formatting of Rust expressions (`view!` prop values and dyn-node bodies).
//!
//! Rather than hand-rolling a Rust expression pretty-printer, we delegate to
//! `rustfmt` by re-serializing the expression's tokens with `quote!` and
//! wrapping them in a throwaway function body, then extracting the formatted
//! result back out. This is run through the *same* [`crate::format::format_source`]
//! pipeline used for whole files, which means any `view! { ... }` macro
//! nested inside the expression (a common pattern, e.g. an `if`/`else` that
//! each produce a `View` via their own `view!` call) gets recursively found
//! and formatted too.

use quote::quote;
use syn::Expr;

use crate::config::Config;
use crate::error::FmtError;
use crate::format;

const WRAPPER_FN: &str = "__sycamorefmt_expr__";

/// Renders `expr` as formatted Rust source.
///
/// The returned string's first line has no leading indentation (the caller
/// is expected to place it after some prefix on the current line), but any
/// additional lines are indented with `indent` spaces, ready to be inserted
/// as-is.
pub fn format_expr(expr: &Expr, indent: usize, cfg: &Config) -> Result<String, FmtError> {
    // Some expressions (struct literals in particular, e.g. `Foo { x: 1 }`)
    // are not allowed to appear directly at the start of a statement/tail
    // expression position -- Rust's grammar requires them to be
    // parenthesized there to avoid ambiguity with a block. Since `quote!`
    // re-serializes the expression without whatever parentheses its
    // *original* surrounding syntax happened to provide (e.g. a `view!`
    // dyn-node's own `( ... )`), we always add one layer of parentheses
    // ourselves before wrapping, and strip it back off after formatting.
    let raw = quote!(#expr).to_string();
    let wrapped = format!("fn {WRAPPER_FN}() {{\n({raw})\n}}\n");

    let outcome = format::format_source(&wrapped, cfg)?;

    let body = extract_fn_body(&outcome.output).ok_or_else(|| {
        FmtError::Internal(format!(
            "could not extract expression body from rustfmt output:\n{}",
            outcome.output
        ))
    })?;
    let body = strip_wrapping_parens(&body);

    Ok(reindent(&body, cfg.tab_spaces, indent))
}

/// Given rustfmt's output for `fn __sycamorefmt_expr__() { <body> }`,
/// extracts just `<body>` (still indented one level, i.e. by
/// `cfg.tab_spaces` spaces, since rustfmt always places a non-empty fn body
/// on its own indented lines).
fn extract_fn_body(formatted: &str) -> Option<String> {
    let mut lines: Vec<&str> = formatted.lines().collect();
    while matches!(lines.last(), Some(l) if l.trim().is_empty()) {
        lines.pop();
    }
    if lines.len() < 2 {
        return None;
    }
    let first = *lines.first()?;
    if !first.trim_end().ends_with('{') {
        return None;
    }
    let last = *lines.last()?;
    if last.trim() != "}" {
        return None;
    }
    let body_lines = &lines[1..lines.len() - 1];
    Some(body_lines.join("\n"))
}

/// Strips exactly one layer of wrapping parentheses added by
/// [`format_expr`]: an opening `(` that is the first non-whitespace
/// character on the body's first line, and a closing `)` that is the last
/// non-whitespace character on the body's last line. `rustfmt` never
/// removes or relocates parentheses on its own, so these are guaranteed
/// (barring a bug elsewhere) to be exactly the pair we added.
fn strip_wrapping_parens(body: &str) -> String {
    let mut lines: Vec<String> = body.lines().map(|s| s.to_string()).collect();
    if let Some(first) = lines.first_mut() {
        if let Some(pos) = first.find('(') {
            if first[..pos].trim().is_empty() {
                first.remove(pos);
            }
        }
    }
    if let Some(last) = lines.last_mut() {
        if let Some(pos) = last.rfind(')') {
            if last[pos + 1..].trim().is_empty() {
                last.remove(pos);
            }
        }
    }
    lines.join("\n")
}

/// Shifts every line of `body` from a base indentation of `from` spaces to a
/// base indentation of `to` spaces, preserving any additional nested
/// indentation. The first line is left with no leading whitespace at all,
/// since the caller is responsible for placing it after existing text on the
/// current line.
fn reindent(body: &str, from: usize, to: usize) -> String {
    let from_prefix = " ".repeat(from);
    let to_prefix = " ".repeat(to);
    let mut out_lines = Vec::with_capacity(body.lines().count());
    for (i, line) in body.lines().enumerate() {
        if line.trim().is_empty() {
            out_lines.push(String::new());
            continue;
        }
        let stripped = line
            .strip_prefix(&from_prefix)
            .unwrap_or_else(|| line.trim_start());
        if i == 0 {
            out_lines.push(stripped.to_string());
        } else {
            out_lines.push(format!("{to_prefix}{stripped}"));
        }
    }
    out_lines.join("\n")
}
