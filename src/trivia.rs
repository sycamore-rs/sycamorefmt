//! Source trivia needed by the view printer.
//!
//! `sycamore_view_parser` deliberately discards whitespace between nodes, and
//! its IR spans are not reliable outside a procedural-macro expansion.  This
//! module therefore walks the body source and the parsed IR in lockstep.  The
//! scanner recovers each node's delimiter boundaries and records only whether
//! a sibling gap contained an empty line.

use sycamore_view_parser::ir::{IfNode, MatchNode, Node, Root, TagNode};

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
    pub(crate) control: ControlSpacing,
}

pub(crate) enum ControlSpacing {
    None,
    If {
        then_branch: SpacingRoot,
        else_branch: Option<SpacingRoot>,
        else_if: bool,
    },
    Match {
        arms: Vec<SpacingRoot>,
    },
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
                        Node::Text(_) | Node::Dyn(_) | Node::If(_) | Node::Match(_) => {
                            Self::empty()
                        }
                    },
                    control: match node {
                        Node::If(if_node) => ControlSpacing::If {
                            then_branch: Self::empty_for_root(&if_node.then),
                            else_branch: if_node.else_branch.as_ref().map(Self::empty_for_root),
                            else_if: false,
                        },
                        Node::Match(match_node) => ControlSpacing::Match {
                            arms: match_node
                                .arms
                                .iter()
                                .map(|arm| Self::empty_for_root(&arm.body))
                                .collect(),
                        },
                        Node::Tag(_) | Node::Text(_) | Node::Dyn(_) => ControlSpacing::None,
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
                    control: ControlSpacing::None,
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
                        control: ControlSpacing::None,
                    },
                )
            })
        }
        Node::Tag(tag) => scan_tag(tag, source, start, limit),
        Node::If(if_node) => scan_if(if_node, source, start, limit),
        Node::Match(match_node) => scan_match(match_node, source, start, limit),
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
    Some((
        end,
        SpacingNode {
            children,
            control: ControlSpacing::None,
        },
    ))
}

fn scan_if(
    if_node: &IfNode,
    source: &str,
    start: usize,
    limit: usize,
) -> Option<(usize, SpacingNode)> {
    if !keyword_at(source, start, limit, "if") {
        return None;
    }

    for open in top_level_braces(source, start + 2, limit)? {
        let close = matching_delimiter(source, open)?;
        if close >= limit {
            continue;
        }
        let Some((then_end, then_spacing)) = scan_root(&if_node.then, source, open + 1, close)
        else {
            continue;
        };
        if skip_whitespace(source, then_end, close) != close {
            continue;
        }

        let after_then = skip_whitespace(source, close + 1, limit);
        let (end, else_branch, else_if) = if keyword_at(source, after_then, limit, "else") {
            let after_else = skip_whitespace(source, after_then + 4, limit);
            if keyword_at(source, after_else, limit, "if") {
                let else_root = if_node.else_branch.as_ref()?;
                let (end, spacing) = scan_root(else_root, source, after_else, limit)?;
                if else_root.0.len() != 1 {
                    return None;
                }
                (end, Some(spacing), true)
            } else if source.as_bytes().get(after_else) == Some(&b'{') {
                let else_close = matching_delimiter(source, after_else)?;
                if else_close >= limit {
                    continue;
                }
                let else_root = if_node.else_branch.as_ref()?;
                let (else_end, spacing) = scan_root(else_root, source, after_else + 1, else_close)?;
                if skip_whitespace(source, else_end, else_close) != else_close {
                    continue;
                }
                (else_close + 1, Some(spacing), false)
            } else {
                continue;
            }
        } else {
            // A brace after an expression block in the condition is the
            // actual then-block, not the end of an `if` without an else.
            if source.as_bytes().get(after_then) == Some(&b'{') {
                continue;
            }
            (close + 1, None, false)
        };

        return Some((
            end,
            SpacingNode {
                children: SpacingRoot::empty(),
                control: ControlSpacing::If {
                    then_branch: then_spacing,
                    else_branch,
                    else_if,
                },
            },
        ));
    }
    None
}

fn scan_match(
    match_node: &MatchNode,
    source: &str,
    start: usize,
    limit: usize,
) -> Option<(usize, SpacingNode)> {
    if !keyword_at(source, start, limit, "match") {
        return None;
    }

    for open in top_level_braces(source, start + 5, limit)? {
        let close = matching_delimiter(source, open)?;
        if close >= limit {
            continue;
        }
        if let Some(arms) = scan_match_arms(match_node, source, open + 1, close) {
            if source
                .as_bytes()
                .get(skip_whitespace(source, close + 1, limit))
                == Some(&b'{')
            {
                continue;
            }
            return Some((
                close + 1,
                SpacingNode {
                    children: SpacingRoot::empty(),
                    control: ControlSpacing::Match { arms },
                },
            ));
        }
    }
    None
}

fn scan_match_arms(
    match_node: &MatchNode,
    source: &str,
    start: usize,
    close: usize,
) -> Option<Vec<SpacingRoot>> {
    let mut cursor = skip_whitespace(source, start, close);
    let mut arms = Vec::with_capacity(match_node.arms.len());

    for arm in &match_node.arms {
        let arrow = find_fat_arrow(source, cursor, close)?;
        cursor = skip_whitespace(source, arrow + 2, close);
        if cursor >= close {
            return None;
        }

        let body_spacing = if source.as_bytes()[cursor] == b'{' {
            let body_close = matching_delimiter(source, cursor)?;
            if body_close >= close {
                return None;
            }
            let (body_end, spacing) = scan_root(&arm.body, source, cursor + 1, body_close)?;
            if skip_whitespace(source, body_end, body_close) != body_close {
                return None;
            }
            cursor = body_close + 1;
            spacing
        } else {
            let [body_node] = arm.body.0.as_slice() else {
                return None;
            };
            let (body_end, spacing) = scan_node(body_node, source, cursor, close)?;
            cursor = body_end;
            SpacingRoot {
                between: Vec::new(),
                nodes: vec![spacing],
            }
        };
        arms.push(body_spacing);

        cursor = skip_whitespace(source, cursor, close);
        if source.as_bytes().get(cursor) == Some(&b',') {
            cursor += 1;
        }
        cursor = skip_whitespace(source, cursor, close);
    }

    (cursor == close).then_some(arms)
}

fn top_level_braces(source: &str, mut cursor: usize, limit: usize) -> Option<Vec<usize>> {
    let bytes = source.as_bytes();
    let mut braces = Vec::new();
    while cursor < limit {
        if let Some(end) = skip_non_code(source, cursor, limit) {
            cursor = end;
            continue;
        }
        match bytes[cursor] {
            b'(' | b'[' => {
                let close = matching_delimiter(source, cursor)?;
                if close >= limit {
                    return None;
                }
                cursor = close + 1;
            }
            b'{' => {
                braces.push(cursor);
                let close = matching_delimiter(source, cursor)?;
                if close >= limit {
                    return None;
                }
                cursor = close + 1;
            }
            _ => cursor += 1,
        }
    }
    Some(braces)
}

fn find_fat_arrow(source: &str, mut cursor: usize, limit: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    while cursor + 1 < limit {
        if let Some(end) = skip_non_code(source, cursor, limit) {
            cursor = end;
            continue;
        }
        match bytes[cursor] {
            b'(' | b'[' | b'{' => {
                let close = matching_delimiter(source, cursor)?;
                if close >= limit {
                    return None;
                }
                cursor = close + 1;
            }
            b'=' if bytes[cursor + 1] == b'>' => return Some(cursor),
            _ => cursor += 1,
        }
    }
    None
}

fn skip_non_code(source: &str, cursor: usize, limit: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if cursor >= limit {
        return None;
    }
    if let Some(end) = raw_string_end(bytes, cursor) {
        return Some(end);
    }
    match bytes[cursor] {
        b'"' => Some(skip_quoted(bytes, cursor)),
        b'\'' => Some(skip_char(bytes, cursor)),
        b'/' if bytes.get(cursor + 1) == Some(&b'/') => Some(
            bytes[cursor..limit]
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|offset| cursor + offset + 1)
                .unwrap_or(limit),
        ),
        b'/' if bytes.get(cursor + 1) == Some(&b'*') => {
            let mut end = cursor + 2;
            let mut depth = 1usize;
            while end + 1 < limit && depth > 0 {
                if bytes[end] == b'/' && bytes[end + 1] == b'*' {
                    depth += 1;
                    end += 2;
                } else if bytes[end] == b'*' && bytes[end + 1] == b'/' {
                    depth -= 1;
                    end += 2;
                } else {
                    end += 1;
                }
            }
            Some(end)
        }
        _ => None,
    }
}

fn keyword_at(source: &str, start: usize, limit: usize, keyword: &str) -> bool {
    let bytes = source.as_bytes();
    let end = start + keyword.len();
    if end > limit || &bytes[start..end] != keyword.as_bytes() {
        return false;
    }
    !bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
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
    let bytes = source.as_bytes();
    let mut cursor = start;
    let mut generic_depth = 0usize;
    while cursor < limit {
        match bytes[cursor] {
            b'<' => generic_depth += 1,
            b'>' if generic_depth > 0 => generic_depth -= 1,
            b'(' | b'{' if generic_depth == 0 => return Some((cursor, bytes[cursor])),
            b'(' | b'{' | b'[' => {
                // Function types and const expressions inside generic
                // arguments may contain delimiters of their own. Skip those
                // balanced groups instead of mistaking them for props or
                // children of the tag.
                let close = matching_delimiter(source, cursor)?;
                if close >= limit {
                    return None;
                }
                cursor = close;
            }
            _ => {}
        }
        cursor += 1;
    }
    None
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

fn skip_char(bytes: &[u8], i: usize) -> usize {
    let mut cursor = i + 1;
    if cursor >= bytes.len() || bytes[cursor].is_ascii_whitespace() {
        return i + 1;
    }
    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor += 1;
            if cursor >= bytes.len() {
                return i + 1;
            }
            if bytes[cursor] == b'u' && bytes.get(cursor + 1) == Some(&b'{') {
                cursor += 2;
                while cursor < bytes.len() && bytes[cursor] != b'}' {
                    cursor += 1;
                }
                cursor = (cursor + 1).min(bytes.len());
            } else {
                cursor += 1;
            }
        } else if bytes[cursor] == b'\'' {
            return cursor + 1;
        } else if bytes[cursor].is_ascii_whitespace() {
            return i + 1;
        } else {
            cursor += 1;
        }
    }
    i + 1
}

#[cfg(test)]
mod tests {
    use sycamore_view_parser::ir::Root;

    use super::{ControlSpacing, SpacingRoot};

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

    #[test]
    fn scans_if_and_else_if_branch_sibling_spacing() {
        let source = "if show {\n    first {}\n\n    second {}\n} else if ready {\n    \"ready\"\n} else {\n    \"hidden\"\n}";
        let result = spacing(source);
        let ControlSpacing::If {
            then_branch,
            else_branch: Some(else_branch),
            else_if,
        } = &result.node(0).control
        else {
            panic!("expected an if spacing tree");
        };
        assert!(then_branch.has_blank_line_between(0));
        assert!(*else_if);
        assert_eq!(else_branch.nodes.len(), 1);
    }

    #[test]
    fn scans_match_arm_roots_and_single_node_bodies() {
        let source = "match value {\n    Some(x) => {\n        first {}\n\n        second {}\n    }\n    None => \"empty\",\n}";
        let result = spacing(source);
        let ControlSpacing::Match { arms } = &result.node(0).control else {
            panic!("expected a match spacing tree");
        };
        assert_eq!(arms.len(), 2);
        assert!(arms[0].has_blank_line_between(0));
        assert_eq!(arms[1].nodes.len(), 1);
    }
}
