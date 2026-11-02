//! Locate `view!` invocations, pretty-print their parsed bodies, and splice
//! them into the original source without changing the surrounding Rust.

use syn::MacroDelimiter;
use syn::visit::Visit;

use crate::config::Config;
use crate::error::FmtError;
use crate::finder::{FoundMacro, MacroFinder};
use crate::lex::{block_comment_end, literal_end};
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
    /// spots are simply left as they appeared in the input.
    pub warnings: Vec<String>,
}

/// Formats view macros in a complete, syntactically valid Rust source file.
/// Source outside those macros is left unchanged.
pub fn format_source(input: &str, cfg: &Config) -> Result<FormatOutcome, FmtError> {
    let file = syn::parse_file(input).map_err(FmtError::Parse)?;
    // syn strips the BOM and shebang before tokenizing, so its byte spans are
    // relative to this suffix, not to the original input. Use syn's detected
    // shebang rather than reimplementing its distinction from inner attributes.
    // The shebang excludes its terminating newline, which syn leaves in place.
    let mut parsed_input = input.strip_prefix('\u{feff}').unwrap_or(input);
    if let Some(shebang) = &file.shebang {
        parsed_input = &parsed_input[shebang.len()..];
    }
    let source_offset = input.len() - parsed_input.len();
    let mut finder = MacroFinder::new();
    finder.visit_file(&file);

    let mut warnings = Vec::new();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    // Determine the convention once per file, not once per view.
    let newline = source_newline(input);

    for found in &finder.found {
        match splice_one(parsed_input, found, cfg, newline) {
            Ok(Some((start, end, replacement))) => {
                edits.push((start + source_offset, end + source_offset, replacement));
            }
            Ok(None) => {}
            Err(msg) => warnings.push(msg),
        }
    }

    edits.sort_by_key(|(start, _, _)| *start);
    let output = apply_edits(input, &edits);
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
/// `Ok(None)` if it should be left untouched, or `Err(warning)` if formatting
/// was skipped or failed.
fn splice_one(
    src: &str,
    found: &FoundMacro,
    cfg: &Config,
    newline: &str,
) -> Result<Option<(usize, usize, String)>, String> {
    let mac = &found.mac;
    let mac_path = &mac.path;
    let macro_name = quote::quote!(#mac_path).to_string();

    let delim_span = match &mac.delimiter {
        MacroDelimiter::Brace(b) => &b.span,
        MacroDelimiter::Paren(_) | MacroDelimiter::Bracket(_) => {
            return Err(format!(
                "skipping a `{macro_name}!` macro: unsupported delimiter; expected braces"
            ));
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

    let path_end = mac
        .path
        .segments
        .last()
        .ok_or_else(|| format!("skipping a `{macro_name}!` macro: it has an empty path"))?
        .ident
        .span()
        .byte_range()
        .end;
    if path_end > open_end {
        return Err(format!(
            "skipping a `{macro_name}!` macro: could not determine its source span"
        ));
    }

    let raw_body = &src[open_end..close_start];
    if contains_comments(raw_body) || contains_comments(&src[path_end..open_end]) {
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

    let prefix = line_indent_prefix(src, path_end);
    let indent = visual_indent(&prefix, cfg.tab_spaces);
    let body_indent = indent + cfg.tab_spaces;
    let pad = " ".repeat(indent);

    let spacing = crate::trivia::SpacingRoot::try_from_root(&root, raw_body).ok_or_else(|| {
        format!("skipping a `{macro_name}!` macro: could not safely recover source trivia")
    })?;
    let replacement = if root.0.is_empty() {
        "! {}".to_string()
    } else {
        let body = printer::print_root(&root, &spacing, body_indent, cfg)
            .map_err(|e| format!("skipping a `{macro_name}!` macro: {e}"))?;
        format!("! {{\n{body}\n{pad}}}")
    };
    let replacement = restore_opaque_macro_calls(&replacement, raw_body).ok_or_else(|| {
        format!("skipping a `{macro_name}!` macro: could not safely preserve opaque macro input")
    })?;
    let replacement = restore_layout(&replacement, &prefix, indent, newline);

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
        if let Some(end) = literal_end(bytes, i) {
            i = end;
            continue;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && (bytes[i + 1] == b'/' || bytes[i + 1] == b'*')
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Returns the exact leading spaces/tabs on the line containing `pos`.
fn line_indent_prefix(src: &str, pos: usize) -> String {
    let line_start = src[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    src[line_start..pos]
        .chars()
        .take_while(|c| matches!(c, ' ' | '\t'))
        .collect()
}

/// Measures indentation using `tab_spaces` as the width of a tab stop.
fn visual_indent(prefix: &str, tab_spaces: usize) -> usize {
    let tab_width = tab_spaces.max(1);
    prefix.chars().fold(0, |column, ch| match ch {
        '\t' => column + tab_width - column % tab_width,
        _ => column + 1,
    })
}

/// Converts the printer's absolute-space layout to the source indentation and
/// newline convention. Rust literal tokens are copied byte-for-byte, including
/// any physical newlines they contain.
fn restore_layout(text: &str, prefix: &str, base_indent: usize, newline: &str) -> String {
    let bytes = text.as_bytes();
    let opaque_macros = macro_invocation_ranges(text);
    let mut macro_index = 0;
    let mut output = String::with_capacity(text.len());
    let mut i = 0;
    let mut at_line_start = false;

    while i < bytes.len() {
        if let Some((start, end)) = opaque_macros.get(macro_index).copied()
            && i == start
        {
            output.push_str(&text[start..end]);
            i = end;
            macro_index += 1;
            at_line_start = false;
            continue;
        }

        if let Some(end) = literal_end(bytes, i) {
            output.push_str(&text[i..end]);
            i = end;
            at_line_start = false;
            continue;
        }

        if bytes[i] == b'\n' {
            output.push_str(newline);
            i += 1;
            at_line_start = true;
            continue;
        }
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            output.push_str(newline);
            i += 2;
            at_line_start = true;
            continue;
        }

        if at_line_start {
            let start = i;
            while bytes.get(i) == Some(&b' ') {
                i += 1;
            }
            let spaces = i - start;
            let blank_line = bytes.get(i) == Some(&b'\n')
                || (bytes.get(i) == Some(&b'\r') && bytes.get(i + 1) == Some(&b'\n'));
            if !blank_line && (spaces > 0 || i < bytes.len()) {
                output.push_str(prefix);
                output.push_str(&" ".repeat(spaces.saturating_sub(base_indent)));
            }
            at_line_start = false;
            if i == bytes.len() {
                break;
            }
            continue;
        }

        let ch = text[i..].chars().next().expect("valid UTF-8 boundary");
        output.push(ch);
        i += ch.len_utf8();
    }

    output
}

/// Finds opaque macro invocations in emitted Rust. Their complete calls are
/// copied as a unit so layout normalization cannot rewrite stringify-like
/// input. Literal contents and comments are skipped while balancing groups.
fn macro_invocation_ranges(src: &str) -> Vec<(usize, usize)> {
    let bytes = src.as_bytes();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(end) = literal_end(bytes, i) {
            i = end;
            continue;
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i = skip_block_comment(bytes, i);
            continue;
        }
        if bytes[i] == b'!' && macro_path_precedes(bytes, i) && !is_view_macro(bytes, i) {
            let start = macro_call_start(bytes, i);
            let mut open = i + 1;
            while matches!(bytes.get(open), Some(b' ' | b'\t' | b'\r' | b'\n')) {
                open += 1;
            }
            // A macro_rules definition has a name between `!` and its body.
            if src[start..i].trim() == "macro_rules" {
                if bytes.get(open..open + 2) == Some(b"r#") {
                    open += 2;
                }
                while bytes.get(open).is_some_and(|byte| {
                    byte.is_ascii_alphanumeric() || *byte == b'_' || *byte >= 0x80
                }) {
                    open += 1;
                }
                while matches!(bytes.get(open), Some(b' ' | b'\t' | b'\r' | b'\n')) {
                    open += 1;
                }
            }
            if matches!(bytes.get(open), Some(b'(' | b'[' | b'{'))
                && let Some(end) = delimiter_group_end(bytes, open)
            {
                ranges.push((start, end));
                i = end;
                continue;
            }
        }
        let ch = src[i..].chars().next().expect("valid UTF-8 boundary");
        i += ch.len_utf8();
    }
    ranges
}

/// Restore original macro bytes after AST printing, failing closed when the
/// count, path, definition name, or delimiter differs from the original.
fn restore_opaque_macro_calls(emitted: &str, original: &str) -> Option<String> {
    let original_ranges = macro_invocation_ranges(original);
    let emitted_ranges = macro_invocation_ranges(emitted);
    if original_ranges.len() != emitted_ranges.len() {
        return None;
    }

    for ((source_start, source_end), (output_start, output_end)) in
        original_ranges.iter().zip(&emitted_ranges)
    {
        let source_key = macro_identity(&original[*source_start..*source_end])?;
        let output_key = macro_identity(&emitted[*output_start..*output_end])?;
        if source_key != output_key {
            return None;
        }
    }

    let mut output = emitted.to_string();
    for ((source_start, source_end), (output_start, output_end)) in
        original_ranges.iter().zip(&emitted_ranges).rev()
    {
        output.replace_range(
            *output_start..*output_end,
            &original[*source_start..*source_end],
        );
    }
    Some(output)
}

/// Compare macro identities without comparing their opaque token contents.
fn macro_identity(source: &str) -> Option<(String, Option<String>, u8)> {
    let (mac, name) = if let Ok(item) = syn::parse_str::<syn::ItemMacro>(source) {
        (item.mac, item.ident.map(|ident| ident.to_string()))
    } else {
        (syn::parse_str::<syn::Macro>(source).ok()?, None)
    };
    let path = &mac.path;
    let delimiter = match mac.delimiter {
        MacroDelimiter::Brace(_) => b'{',
        MacroDelimiter::Paren(_) => b'(',
        MacroDelimiter::Bracket(_) => b'[',
    };
    Some((quote::quote!(#path).to_string(), name, delimiter))
}

/// Returns the start of the macro path preceding `!`, falling back to the
/// delimiter group for unusual paths that cannot be scanned simply.
fn macro_call_start(bytes: &[u8], bang: usize) -> usize {
    let mut end = bang;
    while end > 0 && matches!(bytes[end - 1], b' ' | b'\t' | b'\r' | b'\n') {
        end -= 1;
    }
    if end == 0 || bytes[end - 1] == b'>' {
        return bang;
    }

    let mut start = end;
    while start > 0
        && (bytes[start - 1].is_ascii_alphanumeric()
            || bytes[start - 1] == b'_'
            || bytes[start - 1] >= 0x80)
    {
        start -= 1;
    }
    if start >= 2 && bytes.get(start - 2..start) == Some(b"r#") {
        start -= 2;
    }

    loop {
        let mut separator = start;
        while separator > 0 && matches!(bytes[separator - 1], b' ' | b'\t') {
            separator -= 1;
        }
        if separator < 2 || bytes.get(separator - 2..separator) != Some(b"::") {
            break;
        }
        let mut segment_end = separator - 2;
        while segment_end > 0 && matches!(bytes[segment_end - 1], b' ' | b'\t') {
            segment_end -= 1;
        }
        let mut segment_start = segment_end;
        while segment_start > 0
            && (bytes[segment_start - 1].is_ascii_alphanumeric()
                || bytes[segment_start - 1] == b'_'
                || bytes[segment_start - 1] >= 0x80)
        {
            segment_start -= 1;
        }
        if segment_start == segment_end {
            break;
        }
        start = if segment_start >= 2 && bytes.get(segment_start - 2..segment_start) == Some(b"r#")
        {
            segment_start - 2
        } else {
            segment_start
        };
    }
    start
}

/// Returns whether `!` is preceded by a macro path rather than unary `!`.
fn macro_path_precedes(bytes: &[u8], bang: usize) -> bool {
    let mut end = bang;
    while end > 0 && matches!(bytes[end - 1], b' ' | b'\t' | b'\r' | b'\n') {
        end -= 1;
    }
    if end == 0 {
        return false;
    }
    let last = bytes[end - 1];
    if !(last.is_ascii_alphanumeric() || last == b'_' || last >= 0x80) {
        return false;
    }
    let mut start = end - 1;
    while start > 0
        && (bytes[start - 1].is_ascii_alphanumeric()
            || bytes[start - 1] == b'_'
            || bytes[start - 1] >= 0x80)
    {
        start -= 1;
    }
    !matches!(
        &bytes[start..end],
        b"if" | b"while" | b"return" | b"else" | b"match" | b"break" | b"continue"
    )
}

/// Identifies nested `view!` calls, whose body is formatter-owned layout and
/// must receive the enclosing indentation conversion rather than opaque-token
/// treatment.
fn is_view_macro(bytes: &[u8], bang: usize) -> bool {
    let mut end = bang;
    while end > 0 && matches!(bytes[end - 1], b' ' | b'\t' | b'\r' | b'\n') {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
        start -= 1;
    }
    let name = if start >= 2 && bytes.get(start - 2..start) == Some(b"r#") {
        &bytes[start - 2..end]
    } else {
        &bytes[start..end]
    };
    name == b"view" || name == b"r#view"
}

/// Finds the byte immediately following a balanced delimiter group.
fn delimiter_group_end(bytes: &[u8], open: usize) -> Option<usize> {
    let mut stack = vec![match bytes[open] {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        _ => return None,
    }];
    let mut i = open + 1;
    while i < bytes.len() {
        if let Some(end) = literal_end(bytes, i) {
            i = end;
            continue;
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i = skip_block_comment(bytes, i);
            continue;
        }
        match bytes[i] {
            b'(' => stack.push(b')'),
            b'[' => stack.push(b']'),
            b'{' => stack.push(b'}'),
            ch if stack.last() == Some(&ch) => {
                stack.pop();
                if stack.is_empty() {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    block_comment_end(bytes, start)
}

/// Selects the dominant source newline convention while ignoring literal
/// contents, so multiline strings cannot influence the generated layout.
fn source_newline(src: &str) -> &'static str {
    let bytes = src.as_bytes();
    let mut crlf = 0;
    let mut lf = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes.get(i..i + 2) == Some(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            if i < bytes.len() {
                if i > 0 && bytes[i - 1] == b'\r' {
                    crlf += 1;
                } else {
                    lf += 1;
                }
                i += 1;
            }
        } else if bytes.get(i..i + 2) == Some(b"/*") {
            let mut depth = 1usize;
            i += 2;
            while i < bytes.len() && depth > 0 {
                if bytes.get(i..i + 2) == Some(b"/*") {
                    depth += 1;
                    i += 2;
                } else if bytes.get(i..i + 2) == Some(b"*/") {
                    depth -= 1;
                    i += 2;
                } else if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                    crlf += 1;
                    i += 2;
                } else if bytes[i] == b'\n' {
                    lf += 1;
                    i += 1;
                } else {
                    i += 1;
                }
            }
        } else if let Some(end) = literal_end(bytes, i) {
            i = end;
        } else if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            crlf += 1;
            i += 2;
        } else if bytes[i] == b'\n' {
            lf += 1;
            i += 1;
        } else {
            let ch = src[i..].chars().next().expect("valid UTF-8 boundary");
            i += ch.len_utf8();
        }
    }
    if crlf > lf { "\r\n" } else { "\n" }
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
