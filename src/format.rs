//! The core formatting pipeline: run `rustfmt` over the whole input first
//! (so surrounding code and the indentation of each `view!` call site is
//! canonical), then locate every `view! { ... }` invocation, parse its body
//! with `sycamore-view-parser`, pretty-print it, and splice the result back
//! in place of the original tokens.

use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::MacroDelimiter;

use crate::config::Config;
use crate::error::FmtError;
use crate::finder::{FoundMacro, MacroFinder};
use crate::printer;

/// The result of formatting one piece of source text.
pub struct FormatOutcome {
    /// The formatted source.
    pub output: String,
    /// Whether `output` differs from the original input.
    pub changed: bool,
    /// Non-fatal issues encountered while formatting (e.g. a `view!` macro
    /// body that could not be parsed, or that was left untouched because it
    /// appears to contain comments). The overall run still succeeds; these
    /// spots are simply left as rustfmt produced them.
    pub warnings: Vec<String>,
}

/// Formats `input`, a complete, syntactically valid Rust source file (or a
/// self-contained snippet such as a single wrapped function -- see
/// [`crate::exprfmt`]).
pub fn format_source(input: &str, cfg: &Config) -> Result<FormatOutcome, FmtError> {
    // Step 1: run rustfmt over the whole file first. `rustfmt` cannot make
    // sense of the view-macro grammar inside `view! { ... }` bodies, so it
    // leaves their contents byte-for-byte untouched, but it *does* correctly
    // format and indent everything else, including the line the macro call
    // itself sits on. Doing this first means we can trust that line's
    // indentation when we go decide how to indent the view! body we
    // generate.
    let stage1 = crate::rustfmt::run(input, cfg)?;

    let file = syn::parse_file(&stage1).map_err(FmtError::Parse)?;
    let mut finder = MacroFinder::new();
    finder.visit_file(&file);

    let mut warnings = Vec::new();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();

    for found in &finder.found {
        match splice_one(&stage1, found, cfg) {
            Ok(Some(edit)) => edits.push(edit),
            Ok(None) => {}
            Err(msg) => warnings.push(msg),
        }
    }

    edits.sort_by_key(|(start, _, _)| *start);
    let output = apply_edits(&stage1, &edits);
    let changed = output != input;

    Ok(FormatOutcome {
        output,
        changed,
        warnings,
    })
}

/// Splices non-overlapping `(start, end, replacement)` edits (byte offsets
/// into `src`, in any order) into a new string.
fn apply_edits(src: &str, edits: &[(usize, usize, String)]) -> String {
    let mut output = String::with_capacity(src.len());
    let mut last = 0usize;
    for (start, end, text) in edits {
        if *start < last {
            // Overlapping edits should be impossible (macro invocations
            // can't overlap), but guard against it defensively rather than
            // panicking or corrupting output.
            continue;
        }
        output.push_str(&src[last..*start]);
        output.push_str(text);
        last = *end;
    }
    output.push_str(&src[last..]);
    output
}

/// Computes the replacement text for one `view!`-like macro invocation, or
/// `Ok(None)` if it should be left untouched (unsupported delimiter), or
/// `Err(warning)` if formatting was attempted but skipped/failed.
fn splice_one(
    src: &str,
    found: &FoundMacro,
    cfg: &Config,
) -> Result<Option<(usize, usize, String)>, String> {
    let mac = &found.mac;
    let mac_path = &mac.path;
    let macro_name = quote::quote!(#mac_path).to_string();

    let delim_span = match &mac.delimiter {
        MacroDelimiter::Brace(b) => &b.span,
        MacroDelimiter::Paren(_) | MacroDelimiter::Bracket(_) => {
            // `view!` is conventionally brace-delimited; leave anything else
            // alone rather than guess at reformatting it.
            return Ok(None);
        }
    };

    let open_range = delim_span.open().byte_range();
    let close_range = delim_span.close().byte_range();

    let (open_end, close_start, close_end) = (open_range.end, close_range.start, close_range.end);
    if open_end > src.len() || close_end > src.len() || open_end > close_start {
        return Err(format!(
            "skipping a `{macro_name}!` macro: could not determine its source span"
        ));
    }

    let raw_body = &src[open_end..close_start];
    if contains_comments(raw_body) {
        return Err(format!(
            "skipping a `{macro_name}!` macro: its body appears to contain comments, which \
             sycamorefmt does not support preserving yet"
        ));
    }

    let root = match syn::parse2::<sycamore_view_parser::ir::Root>(mac.tokens.clone()) {
        Ok(root) => root,
        Err(e) => {
            return Err(format!(
                "skipping a `{macro_name}!` macro: failed to parse its body: {e}"
            ));
        }
    };

    let path_end = mac
        .path
        .segments
        .last()
        .ok_or_else(|| format!("skipping a `{macro_name}!` macro: it has an empty path"))?
        .ident
        .span()
        .byte_range()
        .end;

    let indent = line_indent(src, path_end);
    let body_indent = indent + cfg.tab_spaces;
    let pad = " ".repeat(indent);

    let replacement = if root.0.is_empty() {
        "! {}".to_string()
    } else {
        let body = printer::print_root(&root, body_indent, cfg)
            .map_err(|e| format!("skipping a `{macro_name}!` macro: {e}"))?;
        format!("! {{\n{body}\n{pad}}}")
    };

    Ok(Some((path_end, close_end, replacement)))
}

/// Detects comments without mistaking `//` or `/*` inside string literals for
/// comments. The parser discards ordinary comments, so formatting those
/// bodies would lose user text; string-aware detection lets URL-valued props
/// and text nodes still be formatted safely.
fn contains_comments(src: &str) -> bool {
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'/' || bytes[i + 1] == b'*' {
                return true;
            }
        }

        // Skip ordinary quoted strings, including escaped quotes.
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }

        // Skip raw strings (r#"..."#, r##"..."##, ...).
        if bytes[i] == b'r' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] == b'#' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'"' {
                let hashes = j - i - 1;
                let mut k = j + 1;
                while k < bytes.len() {
                    if bytes[k] == b'"' && bytes[k + 1..].starts_with(&vec![b'#'; hashes]) {
                        i = k + 1 + hashes;
                        break;
                    }
                    k += 1;
                }
                if k >= bytes.len() {
                    i = bytes.len();
                }
                continue;
            }
        }

        i += 1;
    }
    false
}

/// Returns the number of leading space characters on the line containing
/// byte offset `pos` in `src`.
fn line_indent(src: &str, pos: usize) -> usize {
    let line_start = src[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    src[line_start..pos]
        .chars()
        .take_while(|c| *c == ' ')
        .count()
}

#[cfg(test)]
mod tests {
    use super::contains_comments;

    #[test]
    fn comment_detection_ignores_string_contents() {
        assert!(!contains_comments(r#"div { "https://example.test/a/*b" }"#));
        assert!(!contains_comments(r##"div { r#"// not a comment"# }"##));
        assert!(contains_comments("div { /* keep me */ }"));
        assert!(contains_comments("div { // keep me\n \"text\" }"));
    }
}
