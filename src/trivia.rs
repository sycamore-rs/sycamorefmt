//! Source trivia needed by the view printer.
//!
//! `sycamore_view_parser` deliberately discards whitespace between nodes, and
//! its IR spans are not reliable outside a procedural-macro expansion.  This
//! module therefore walks the body source and the parsed IR in lockstep.  The
//! scanner recovers each node's delimiter boundaries and records only whether
//! a sibling gap contained an empty line.

use sycamore_view_parser::ir::{Node, Root, TagNode};

/// Trivia associated with a parsed `Root`.
///
/// `between[i]` describes the source gap between `nodes[i]` and
/// `nodes[i + 1]`. The `nodes` vector is parallel to the IR root and carries
/// the same information for nested tag children.
pub(crate) struct SpacingRoot {
    between: Vec<bool>,
    nodes: Vec<SpacingNode>,
}

pub(crate) struct SpacingNode {
    pub(crate) children: SpacingRoot,
}

impl SpacingRoot {
    /// Scans `source`, which must be the raw body between the view macro's
    /// braces.  If the source and IR cannot be walked in lockstep, return a
    /// fully conservative tree: no guessed blank lines are preserved.
    pub(crate) fn from_root(root: &Root, source: &str) -> Self {
        scan_root(root, source, 0, source.len())
            .map(|(_, spacing)| spacing)
            .unwrap_or_else(|| Self::empty_for_root(root))
    }

    fn empty_for_root(root: &Root) -> Self {
        Self {
            between: vec![false; root.0.len().saturating_sub(1)],
            nodes: root
                .0
                .iter()
                .map(|node| SpacingNode {
                    children: match node {
                        Node::Tag(tag) => Self::empty_for_root(&tag.children),
                        Node::Text(_) | Node::Dyn(_) => Self::empty(),
                    },
                })
                .collect(),
        }
    }

    fn empty() -> Self {
        Self {
            between: Vec::new(),
            nodes: Vec::new(),
        }
    }

    pub(crate) fn node(&self, index: usize) -> &SpacingNode {
        debug_assert!(index < self.nodes.len());
        &self.nodes[index]
    }

    pub(crate) fn has_blank_line_between(&self, index: usize) -> bool {
        self.between.get(index).copied().unwrap_or(false)
    }

    pub(crate) fn has_blank_line(&self) -> bool {
        self.between.iter().any(|blank| *blank)
    }
}

/// Scans one IR root and returns the cursor immediately after its last node.
/// Leading and trailing whitespace is intentionally not represented: blank
/// lines adjacent to generated opening/closing braces are always removed.
fn scan_root(
    root: &Root,
    source: &str,
    start: usize,
    limit: usize,
) -> Option<(usize, SpacingRoot)> {
    let mut cursor = start;
    let mut previous_end = None;
    let mut between = Vec::with_capacity(root.0.len().saturating_sub(1));
    let mut nodes = Vec::with_capacity(root.0.len());

    for node in &root.0 {
        cursor = skip_whitespace(source, cursor, limit);
        if cursor >= limit {
            return None;
        }
        let node_start = cursor;
        let (node_end, spacing_node) = scan_node(node, source, node_start, limit)?;
        if node_end <= node_start || node_end > limit {
            return None;
        }
        if let Some(previous_end) = previous_end {
            between.push(contains_empty_line(&source[previous_end..node_start]));
        }
        previous_end = Some(node_end);
        cursor = node_end;
        nodes.push(spacing_node);
    }

    Some((cursor, SpacingRoot { between, nodes }))
}

fn scan_node(
    node: &Node,
    source: &str,
    start: usize,
    limit: usize,
) -> Option<(usize, SpacingNode)> {
    match node {
        Node::Text(_) => {
            let end = scan_text(source, start, limit)?;
            Some((
                end,
                SpacingNode {
                    children: SpacingRoot::empty(),
                },
            ))
        }
        Node::Dyn(_) => {
            if source.as_bytes().get(start) != Some(&b'(') {
                return None;
            }
            let end = matching_delimiter(source, start)? + 1;
            (end <= limit).then(|| {
                (
                    end,
                    SpacingNode {
                        children: SpacingRoot::empty(),
                    },
                )
            })
        }
        Node::Tag(tag) => scan_tag(tag, source, start, limit),
    }
}

fn scan_text(source: &str, start: usize, limit: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let end = if bytes.get(start) == Some(&b'"') {
        skip_quoted(bytes, start)
    } else {
        raw_string_end(bytes, start)?
    };
    (end <= limit).then_some(end)
}

fn scan_tag(
    tag: &TagNode,
    source: &str,
    start: usize,
    limit: usize,
) -> Option<(usize, SpacingNode)> {
    let (open, kind) = find_next_tag_delimiter(source, start, limit)?;
    let (end, children) = match kind {
        b'(' => {
            let prop_close = matching_delimiter(source, open)?;
            if prop_close >= limit {
                return None;
            }
            match next_non_whitespace(source, prop_close + 1, limit) {
                Some(child_open) if source.as_bytes()[child_open] == b'{' => {
                    let child_close = matching_delimiter(source, child_open)?;
                    if child_close >= limit {
                        return None;
                    }
                    let (_, children) =
                        scan_root(&tag.children, source, child_open + 1, child_close)?;
                    (child_close + 1, children)
                }
                _ => (prop_close + 1, SpacingRoot::empty()),
            }
        }
        b'{' => {
            let child_close = matching_delimiter(source, open)?;
            if child_close >= limit {
                return None;
            }
            let (_, children) = scan_root(&tag.children, source, open + 1, child_close)?;
            (child_close + 1, children)
        }
        _ => return None,
    };
    Some((end, SpacingNode { children }))
}

/// Returns whether `gap` contains one or more whitespace-only lines between
/// two nodes. A normal newline (`"left\n    right"`) is not a blank line;
/// two newline sequences with only horizontal whitespace between them are.
fn contains_empty_line(gap: &str) -> bool {
    let lines: Vec<_> = gap.split('\n').collect();
    lines.len() >= 3
        && lines[1..lines.len() - 1]
            .iter()
            .any(|line| line.trim().is_empty())
}

fn skip_whitespace(source: &str, mut cursor: usize, limit: usize) -> usize {
    let bytes = source.as_bytes();
    while cursor < limit && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    cursor
}

fn find_next_tag_delimiter(source: &str, start: usize, limit: usize) -> Option<(usize, u8)> {
    source.as_bytes()[start..limit]
        .iter()
        .enumerate()
        .find_map(|(offset, byte)| match byte {
            b'(' | b'{' => Some((start + offset, *byte)),
            _ => None,
        })
}

fn next_non_whitespace(source: &str, start: usize, limit: usize) -> Option<usize> {
    source.as_bytes()[start..limit]
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map(|offset| start + offset)
}

/// Finds the matching delimiter, skipping strings, character literals, and
/// comments. Comments are rejected by the pipeline, but handling them here
/// keeps delimiter recovery conservative when used independently.
fn matching_delimiter(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let expected = match bytes.get(open)? {
        b'(' => b')',
        b'{' => b'}',
        b'[' => b']',
        _ => return None,
    };
    let mut stack = vec![expected];
    let mut i = open + 1;
    while i < bytes.len() {
        if let Some(end) = raw_string_end(bytes, i) {
            i = end;
            continue;
        }
        if bytes[i] == b'"' {
            i = skip_quoted(bytes, i);
            continue;
        }
        if bytes[i] == b'\'' {
            i = skip_char(bytes, i);
            continue;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i = bytes[i..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|offset| i + offset + 1)
                .unwrap_or(bytes.len());
            continue;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }

        let close = match bytes[i] {
            b'(' => Some(b')'),
            b'{' => Some(b'}'),
            b'[' => Some(b']'),
            b')' | b'}' | b']' => None,
            _ => {
                i += 1;
                continue;
            }
        };
        if let Some(close) = close {
            stack.push(close);
        } else if stack.last() == Some(&bytes[i]) {
            stack.pop();
            if stack.is_empty() {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn raw_string_end(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.get(i) != Some(&b'r') {
        return None;
    }
    let mut quote = i + 1;
    while quote < bytes.len() && bytes[quote] == b'#' {
        quote += 1;
    }
    if bytes.get(quote) != Some(&b'"') {
        return None;
    }
    let hashes = quote - i - 1;
    let mut cursor = quote + 1;
    while cursor < bytes.len() {
        if bytes[cursor] == b'"' && bytes[cursor + 1..].starts_with(&vec![b'#'; hashes]) {
            return Some(cursor + 1 + hashes);
        }
        cursor += 1;
    }
    Some(bytes.len())
}

fn skip_quoted(bytes: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i = (i + 2).min(bytes.len());
        } else if bytes[i] == b'"' {
            return i + 1;
        } else {
            i += 1;
        }
    }
    bytes.len()
}

fn skip_char(bytes: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i = (i + 2).min(bytes.len());
        } else if bytes[i] == b'\'' {
            return i + 1;
        } else {
            i += 1;
        }
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use sycamore_view_parser::ir::Root;

    use super::SpacingRoot;

    fn spacing(source: &str) -> SpacingRoot {
        let root: Root = syn::parse_str(source).expect("valid view body");
        SpacingRoot::from_root(&root, source)
    }

    #[test]
    fn scans_hyphenated_tag_siblings_without_spans() {
        let result = spacing("my-custom-tag {}\n\nother-tag {}");
        assert!(result.has_blank_line_between(0));
    }

    #[test]
    fn scans_dynamic_and_text_siblings() {
        let result = spacing("(value)\n\n\"text\"");
        assert!(result.has_blank_line_between(0));
    }

    #[test]
    fn recognizes_crlf_empty_lines() {
        let result = spacing("first {}\r\n\t\r\nsecond {}");
        assert!(result.has_blank_line_between(0));
    }

    #[test]
    fn ignores_edges_and_single_newlines() {
        let result = spacing("\n\nfirst {}\nsecond {}\n\n");
        assert!(!result.has_blank_line_between(0));
    }

    #[test]
    fn matches_nested_delimiters_in_props_and_children() {
        let result = spacing("div(value = { let _ = r#\"}\"#; value }) {\n\nspan {}\n\ntext {}\n}");
        assert!(result.node(0).children.has_blank_line_between(0));
    }
}
