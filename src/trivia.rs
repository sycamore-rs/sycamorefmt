//! Source trivia needed by the view printer.
//!
//! `sycamore_view_parser` deliberately discards whitespace between nodes.  The
//! parser does, however, retain the spans of the tokens which make up most
//! nodes.  This module uses those spans, together with the original stage-one
//! source, to build a small tree of the trivia that matters to formatting:
//! whether a sibling gap contained an empty line.

use std::ops::Range;

use sycamore_view_parser::ir::{DynNode, Node, Root, TagIdent, TagNode};
use syn::spanned::Spanned;

/// Trivia associated with a parsed `Root`.
///
/// `between[i]` describes the source gap between `nodes[i]` and
/// `nodes[i + 1]`.  The `nodes` vector is parallel to the IR root and carries
/// the same information for nested tag children.
pub(crate) struct SpacingRoot {
    between: Vec<bool>,
    nodes: Vec<SpacingNode>,
}

pub(crate) struct SpacingNode {
    pub(crate) children: SpacingRoot,
}

impl SpacingRoot {
    pub(crate) fn from_root(root: &Root, source: &str) -> Self {
        let ranges: Vec<_> = root.0.iter().map(|node| node_range(node, source)).collect();
        let between = ranges
            .windows(2)
            .map(|pair| match (&pair[0], &pair[1]) {
                (Some(left), Some(right)) if left.end <= right.start => {
                    contains_empty_line(&source[left.end..right.start])
                }
                // A spanless node (currently hyphenated tags) must not make
                // us guess.  It is safer to retain the formatter's normal
                // layout than to accidentally introduce a blank line from a
                // gap inside that node.
                _ => false,
            })
            .collect();
        let nodes = root
            .0
            .iter()
            .map(|node| SpacingNode {
                children: match node {
                    Node::Tag(tag) => Self::from_root(&tag.children, source),
                    Node::Text(_) | Node::Dyn(_) => Self::empty(),
                },
            })
            .collect();
        Self { between, nodes }
    }

    pub(crate) fn empty() -> Self {
        Self {
            between: Vec::new(),
            nodes: Vec::new(),
        }
    }

    pub(crate) fn node(&self, index: usize) -> &SpacingNode {
        // Construction is deliberately parallel with the IR.  Keeping this
        // checked assertion here makes a future parser/IR mismatch fail close
        // to its cause instead of silently applying the wrong trivia.
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

/// Returns whether `gap` contains one or more whitespace-only lines between
/// two nodes.  A normal newline (`"left\n    right"`) is not a blank line;
/// two newline sequences with only horizontal whitespace between them are.
fn contains_empty_line(gap: &str) -> bool {
    let lines: Vec<_> = gap.split('\n').collect();
    lines.len() >= 3
        && lines[1..lines.len() - 1]
            .iter()
            .any(|line| line.trim().is_empty())
}

/// Computes a node's source range.  The parser does not expose delimiter
/// spans for dynamic nodes or tag children, so those delimiters are recovered
/// from the span of the node's meaningful token and balanced in the original
/// source.  Hyphenated tag identifiers are `Span::call_site()` in the parser's
/// IR; they are intentionally returned as spanless here.
fn node_range(node: &Node, source: &str) -> Option<Range<usize>> {
    match node {
        Node::Text(text) => valid_range(text.value.span().byte_range()),
        Node::Dyn(dynamic) => dyn_range(dynamic, source),
        Node::Tag(tag) => tag_range(tag, source),
    }
}

fn valid_range(range: Range<usize>) -> Option<Range<usize>> {
    if range.start < range.end {
        Some(range)
    } else {
        None
    }
}

fn tag_range(tag: &TagNode, source: &str) -> Option<Range<usize>> {
    let start = match &tag.ident {
        TagIdent::Path(path) => valid_range(path.span().byte_range())?.start,
        TagIdent::Hyphenated(_) => return None,
    };
    if start >= source.len() || !source.is_char_boundary(start) {
        return None;
    }

    let close = match find_next_tag_delimiter(source, start)? {
        (open, b'(') => {
            let prop_close = matching_delimiter(source, open)?;
            // The IR cannot distinguish `tag(props)` from
            // `tag(props) {}` when the child root is empty.  Look for the
            // optional child brace directly after the prop list so the
            // entire node still gets the correct source range.
            match next_non_whitespace(source, prop_close + 1) {
                Some(child_open) if source.as_bytes()[child_open] == b'{' => {
                    matching_delimiter(source, child_open)?
                }
                _ => prop_close,
            }
        }
        (open, b'{') => matching_delimiter(source, open)?,
        _ => return None,
    };
    Some(start..close + 1)
}

fn dyn_range(dynamic: &DynNode, source: &str) -> Option<Range<usize>> {
    let value = valid_range(dynamic.value.span().byte_range())?;
    if value.end > source.len() || !source.is_char_boundary(value.start) {
        return None;
    }
    let open = source[..value.start]
        .bytes()
        .rev()
        .skip_while(|byte| byte.is_ascii_whitespace())
        .next()?;
    if open != b'(' {
        return None;
    }
    let open_pos = source[..value.start].len().checked_sub(
        1 + source[..value.start]
            .bytes()
            .rev()
            .take_while(|b| b.is_ascii_whitespace())
            .count(),
    )?;
    Some(open_pos..matching_delimiter(source, open_pos)? + 1)
}

fn find_next_tag_delimiter(source: &str, start: usize) -> Option<(usize, u8)> {
    source.as_bytes()[start..]
        .iter()
        .enumerate()
        .find_map(|(offset, byte)| match byte {
            b'(' | b'{' => Some((start + offset, *byte)),
            _ => None,
        })
}

fn next_non_whitespace(source: &str, start: usize) -> Option<usize> {
    source.as_bytes()[start..]
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map(|offset| start + offset)
}

/// Finds the matching delimiter, skipping strings and comments.  Comments
/// are normally rejected by the pipeline, but skipping them here keeps this
/// helper correct when used independently and avoids treating punctuation in
/// literals as syntax.
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
            i = i + 2;
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
    use super::contains_empty_line;

    #[test]
    fn detects_whitespace_only_lines() {
        assert!(!contains_empty_line("\n    "));
        assert!(contains_empty_line("\n    \n  "));
        assert!(contains_empty_line("\n\t\r\n  "));
    }
}
