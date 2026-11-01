//! Small shared lexer for preserving trivia while scanning view source.
//!
//! This recognizes literal and comment boundaries; it is not a Rust parser.
//! Callers operate on already-tokenized Rust and must leave source unchanged
//! when they cannot establish boundaries safely.

/// Returns the end of a string or character literal beginning at `start`.
/// Lifetimes are not character literals. Prefixes require a token boundary so
/// a suffix of an identifier cannot be mistaken for a raw-string prefix.
pub(crate) fn literal_end(bytes: &[u8], start: usize) -> Option<usize> {
    let byte = *bytes.get(start)?;
    if byte == b'\'' {
        return char_literal_end(bytes, start);
    }
    let boundary = start == 0 || !is_identifier_byte(bytes[start - 1]);
    if byte == b'b' && boundary && bytes.get(start + 1) == Some(&b'\'') {
        return char_literal_end(bytes, start + 1);
    }

    let (quote, hashes) = if byte == b'"' {
        (start, None)
    } else if boundary && matches!(byte, b'b' | b'c') && bytes.get(start + 1) == Some(&b'"') {
        (start + 1, None)
    } else {
        let raw = if boundary && matches!(byte, b'b' | b'c') && bytes.get(start + 1) == Some(&b'r')
        {
            start + 1
        } else if boundary && byte == b'r' {
            start
        } else {
            return None;
        };
        let mut quote = raw + 1;
        while bytes.get(quote) == Some(&b'#') {
            quote += 1;
        }
        if bytes.get(quote) != Some(&b'"') {
            return None;
        }
        (quote, Some(quote - raw - 1))
    };

    if let Some(hashes) = hashes {
        let mut cursor = quote + 1;
        while cursor < bytes.len() {
            if bytes[cursor] == b'"'
                && bytes
                    .get(cursor + 1..cursor + 1 + hashes)
                    .is_some_and(|marker| marker.iter().all(|byte| *byte == b'#'))
            {
                return Some(cursor + 1 + hashes);
            }
            cursor += 1;
        }
        return None;
    }

    let mut cursor = quote + 1;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' => cursor = (cursor + 2).min(bytes.len()),
            b'"' => return Some(cursor + 1),
            _ => cursor += 1,
        }
    }
    None
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

fn char_literal_end(bytes: &[u8], quote: usize) -> Option<usize> {
    let content = quote + 1;
    let first = *bytes.get(content)?;
    let end = if first == b'\\' {
        let escape = content + 1;
        match bytes.get(escape) {
            Some(b'u') if bytes.get(escape + 1) == Some(&b'{') => {
                let mut end = escape + 2;
                while end < bytes.len() && bytes[end] != b'}' {
                    end += 1;
                }
                if end == bytes.len() {
                    return None;
                }
                end + 1
            }
            Some(b'x') => escape + 3,
            Some(_) => escape + 1,
            None => return None,
        }
    } else if first < 0x80 {
        content + 1
    } else {
        // Decode only this character, rather than validating the entire
        // remaining source for every character literal.
        let width = match first {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => return None,
        };
        std::str::from_utf8(bytes.get(content..content + width)?).ok()?;
        content + width
    };
    (bytes.get(end) == Some(&b'\'')).then_some(end + 1)
}

/// Skip a nested Rust block comment starting at `start`.
pub(crate) fn block_comment_end(bytes: &[u8], start: usize) -> usize {
    let mut depth = 1usize;
    let mut cursor = start + 2;
    while cursor < bytes.len() && depth > 0 {
        if bytes.get(cursor..cursor + 2) == Some(b"/*") {
            depth += 1;
            cursor += 2;
        } else if bytes.get(cursor..cursor + 2) == Some(b"*/") {
            depth -= 1;
            cursor += 2;
        } else {
            cursor += 1;
        }
    }
    cursor
}

#[cfg(test)]
mod tests {
    use super::literal_end;

    #[test]
    fn distinguishes_characters_and_lifetimes() {
        for literal in [
            r#"'\''"#,
            r#"'"'"#,
            "' '",
            r#"'\u{1f600}'"#,
            "'é'",
            r#"b'\"'"#,
        ] {
            assert_eq!(
                literal_end(literal.as_bytes(), 0),
                Some(literal.len()),
                "{literal}"
            );
        }
        for lifetime in ["'a", "'static", "'a,'b"] {
            assert_eq!(literal_end(lifetime.as_bytes(), 0), None, "{lifetime}");
        }
    }
}
