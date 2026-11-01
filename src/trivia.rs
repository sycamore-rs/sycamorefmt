//! Source trivia needed by the view printer.
//!
//! `sycamore_view_parser` deliberately discards whitespace between nodes, and
//! its IR spans are not reliable outside a procedural-macro expansion.  This
//! module therefore walks the body source and the parsed IR in lockstep.  The
//! scanner recovers each node's delimiter boundaries and records only whether
//! a sibling gap contained an empty line.

use sycamore_view_parser::ir::{IfNode, MatchNode, Node, Root, TagNode};

use crate::lex::{block_comment_end, literal_end};

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
    /// Recovers trivia from the raw view body, returning `None` if the source
    /// and IR cannot be walked in lockstep.
    pub(crate) fn try_from_root(root: &Root, source: &str) -> Option<Self> {
        if root.0.is_empty() {
            return Some(Self::empty());
        }
        let delimiters = DelimiterMap::new(source);
        scan_root(root, source, &delimiters, 0, source.len()).map(|(_, spacing)| spacing)
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
    delimiters: &DelimiterMap,
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
        let (node_end, spacing_node) = scan_node(node, source, delimiters, node_start, limit)?;
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
    delimiters: &DelimiterMap,
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
            let end = delimiters.matching(start)? + 1;
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
        Node::Tag(tag) => scan_tag(tag, source, delimiters, start, limit),
        Node::If(if_node) => scan_if(if_node, source, delimiters, start, limit),
        Node::Match(match_node) => scan_match(match_node, source, delimiters, start, limit),
    }
}

fn scan_text(source: &str, start: usize, limit: usize) -> Option<usize> {
    let end = literal_end(source.as_bytes(), start)?;
    (end <= limit).then_some(end)
}

fn scan_tag(
    tag: &TagNode,
    source: &str,
    delimiters: &DelimiterMap,
    start: usize,
    limit: usize,
) -> Option<(usize, SpacingNode)> {
    let (open, kind) = find_next_tag_delimiter(source, delimiters, start, limit)?;
    let (end, children) = match kind {
        b'(' => {
            let prop_close = delimiters.matching(open)?;
            if prop_close >= limit {
                return None;
            }
            match next_non_whitespace(source, prop_close + 1, limit) {
                Some(child_open) if source.as_bytes()[child_open] == b'{' => {
                    let child_close = delimiters.matching(child_open)?;
                    if child_close >= limit {
                        return None;
                    }
                    let (_, children) = scan_root(
                        &tag.children,
                        source,
                        delimiters,
                        child_open + 1,
                        child_close,
                    )?;
                    (child_close + 1, children)
                }
                _ => (prop_close + 1, SpacingRoot::empty()),
            }
        }
        b'{' => {
            let child_close = delimiters.matching(open)?;
            if child_close >= limit {
                return None;
            }
            let (_, children) =
                scan_root(&tag.children, source, delimiters, open + 1, child_close)?;
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
    delimiters: &DelimiterMap,
    start: usize,
    limit: usize,
) -> Option<(usize, SpacingNode)> {
    if !keyword_at(source, start, limit, "if") {
        return None;
    }

    let mut brace_cursor = start + 2;
    while let Some(open) = next_top_level_brace(source, delimiters, &mut brace_cursor, limit)? {
        let close = delimiters.matching(open)?;
        if close >= limit {
            continue;
        }
        let Some((then_end, then_spacing)) =
            scan_root(&if_node.then, source, delimiters, open + 1, close)
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
                let (end, spacing) = scan_root(else_root, source, delimiters, after_else, limit)?;
                if else_root.0.len() != 1 {
                    return None;
                }
                (end, Some(spacing), true)
            } else if source.as_bytes().get(after_else) == Some(&b'{') {
                let else_close = delimiters.matching(after_else)?;
                if else_close >= limit {
                    continue;
                }
                let else_root = if_node.else_branch.as_ref()?;
                let (else_end, spacing) =
                    scan_root(else_root, source, delimiters, after_else + 1, else_close)?;
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
    delimiters: &DelimiterMap,
    start: usize,
    limit: usize,
) -> Option<(usize, SpacingNode)> {
    if !keyword_at(source, start, limit, "match") {
        return None;
    }

    let mut brace_cursor = start + 5;
    while let Some(open) = next_top_level_brace(source, delimiters, &mut brace_cursor, limit)? {
        let close = delimiters.matching(open)?;
        if close >= limit {
            continue;
        }
        if let Some(arms) = scan_match_arms(match_node, source, delimiters, open + 1, close) {
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
    delimiters: &DelimiterMap,
    start: usize,
    close: usize,
) -> Option<Vec<SpacingRoot>> {
    let mut cursor = skip_whitespace(source, start, close);
    let mut arms = Vec::with_capacity(match_node.arms.len());

    for arm in &match_node.arms {
        let arrow = find_fat_arrow(source, delimiters, cursor, close)?;
        cursor = skip_whitespace(source, arrow + 2, close);
        if cursor >= close {
            return None;
        }

        let body_spacing = if source.as_bytes()[cursor] == b'{' {
            let body_close = delimiters.matching(cursor)?;
            if body_close >= close {
                return None;
            }
            let (body_end, spacing) =
                scan_root(&arm.body, source, delimiters, cursor + 1, body_close)?;
            if skip_whitespace(source, body_end, body_close) != body_close {
                return None;
            }
            cursor = body_close + 1;
            spacing
        } else {
            let [body_node] = arm.body.0.as_slice() else {
                return None;
            };
            let (body_end, spacing) = scan_node(body_node, source, delimiters, cursor, close)?;
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

/// Finds the next top-level brace lazily. Returning one candidate at a time
/// avoids scanning the rest of each nested control body just to build a vector
/// that its caller usually consumes only through the first candidate.
fn next_top_level_brace(
    source: &str,
    delimiters: &DelimiterMap,
    cursor: &mut usize,
    limit: usize,
) -> Option<Option<usize>> {
    let bytes = source.as_bytes();
    while *cursor < limit {
        if let Some(end) = skip_non_code(source, *cursor, limit) {
            *cursor = end;
            continue;
        }
        match bytes[*cursor] {
            b'(' | b'[' => {
                let close = delimiters.matching(*cursor)?;
                if close >= limit {
                    return None;
                }
                *cursor = close + 1;
            }
            b'{' => {
                let open = *cursor;
                let close = delimiters.matching(open)?;
                if close >= limit {
                    return None;
                }
                *cursor = close + 1;
                return Some(Some(open));
            }
            _ => *cursor += 1,
        }
    }
    Some(None)
}

fn find_fat_arrow(
    source: &str,
    delimiters: &DelimiterMap,
    mut cursor: usize,
    limit: usize,
) -> Option<usize> {
    let bytes = source.as_bytes();
    while cursor + 1 < limit {
        if let Some(end) = skip_non_code(source, cursor, limit) {
            cursor = end;
            continue;
        }
        match bytes[cursor] {
            b'(' | b'[' | b'{' => {
                let close = delimiters.matching(cursor)?;
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

/// Skip a comment or literal using the shared source lexer.
fn skip_non_code(source: &str, cursor: usize, limit: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if cursor >= limit {
        return None;
    }
    if bytes.get(cursor..cursor + 2) == Some(b"//") {
        return Some(
            bytes[cursor..limit]
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|offset| cursor + offset + 1)
                .unwrap_or(limit),
        );
    }
    if bytes.get(cursor..cursor + 2) == Some(b"/*") {
        return Some(block_comment_end(&bytes[..limit], cursor));
    }
    literal_end(bytes, cursor)
}

fn keyword_at(source: &str, start: usize, limit: usize, keyword: &str) -> bool {
    let Some(end) = start.checked_add(keyword.len()) else {
        return false;
    };
    if end > limit || source.get(start..end) != Some(keyword) {
        return false;
    }

    // Match Rust's identifier-continuation rules, not just ASCII: a tag
    // such as `elseé` or `else` followed by a combining mark is not `else`.
    let next = if end < limit {
        source
            .get(end..limit)
            .and_then(|suffix| suffix.chars().next())
    } else {
        None
    };
    !next.is_some_and(|character| character == '_' || unicode_ident::is_xid_continue(character))
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

fn find_next_tag_delimiter(
    source: &str,
    delimiters: &DelimiterMap,
    start: usize,
    limit: usize,
) -> Option<(usize, u8)> {
    let bytes = source.as_bytes();
    let mut cursor = start;
    let mut generic_depth = 0usize;
    while cursor < limit {
        if let Some(end) = skip_non_code(source, cursor, limit) {
            cursor = end;
            continue;
        }
        match bytes[cursor] {
            b'<' => generic_depth += 1,
            b'>' if generic_depth > 0 => generic_depth -= 1,
            b'(' | b'{' if generic_depth == 0 => return Some((cursor, bytes[cursor])),
            b'(' | b'{' | b'[' => {
                // Function types and const expressions inside generic
                // arguments may contain delimiters of their own. Skip those
                // balanced groups instead of mistaking them for props or
                // children of the tag.
                let close = delimiters.matching(cursor)?;
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

/// One-pass delimiter pairing for a complete raw view body.
struct DelimiterMap {
    matching: Vec<usize>,
}

impl DelimiterMap {
    fn new(source: &str) -> Self {
        let bytes = source.as_bytes();
        let mut matching = vec![usize::MAX; bytes.len()];
        let mut stack: Vec<(usize, u8)> = Vec::new();
        let mut cursor = 0;

        while cursor < bytes.len() {
            if let Some(end) = skip_non_code(source, cursor, bytes.len()) {
                cursor = end.max(cursor + 1);
                continue;
            }
            match bytes[cursor] {
                b'(' => stack.push((cursor, b')')),
                b'{' => stack.push((cursor, b'}')),
                b'[' => stack.push((cursor, b']')),
                b')' | b'}' | b']'
                    if stack
                        .last()
                        .is_some_and(|(_, expected)| *expected == bytes[cursor]) =>
                {
                    let (open, _) = stack.pop().expect("delimiter stack was just checked");
                    matching[open] = cursor;
                    matching[cursor] = open;
                }
                _ => {}
            }
            cursor += 1;
        }

        Self { matching }
    }

    fn matching(&self, open: usize) -> Option<usize> {
        match self.matching.get(open).copied()? {
            usize::MAX => None,
            close => Some(close),
        }
    }
}

#[cfg(test)]
mod tests {
    use sycamore_view_parser::ir::Root;

    use super::{ControlSpacing, DelimiterMap, SpacingRoot};

    fn spacing(source: &str) -> SpacingRoot {
        let root: Root = syn::parse_str(source).expect("valid view body");
        SpacingRoot::try_from_root(&root, source).expect("source and IR match")
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
    fn delimiter_map_is_one_pass_and_handles_deep_nesting() {
        let depth = 1_024;
        let source = format!("{}x{}", "(".repeat(depth), ")".repeat(depth));
        let delimiters = DelimiterMap::new(&source);
        assert_eq!(delimiters.matching(0), Some(source.len() - 1));
        assert_eq!(delimiters.matching(depth - 1), Some(depth + 1));
    }

    #[test]
    fn delimiter_map_skips_rust_literals_lifetimes_and_nested_comments() {
        let source = r####"{ let _ = r#"}"#; let _ = br#")]}"#; let _ = c"}"; let _ = b"}"; let _ = cr#"}"#; let _ = b'}'; let _ = '"'; let _ = '\"'; let _ = 'a; // } ignored
/* outer /* } */ nested */ { inner } }"####;
        let delimiters = DelimiterMap::new(source);
        assert_eq!(delimiters.matching(0), Some(source.len() - 1));
        let nested_open = source.find("{ inner }").expect("nested token group");
        assert_eq!(delimiters.matching(nested_open), Some(nested_open + 8));
    }

    #[test]
    fn quote_char_literals_do_not_hide_following_comments() {
        let source = "{ let _ = '\"'; // ordinary char quote, then comment with }\n let _ = b'\"'; // byte char quote, then }\n value }";
        let delimiters = DelimiterMap::new(source);
        assert_eq!(delimiters.matching(0), Some(source.len() - 1));
    }

    #[test]
    fn try_from_root_reports_mismatch() {
        let root: Root = syn::parse_str("first {}\n\nsecond {}").expect("valid view body");
        assert!(SpacingRoot::try_from_root(&root, "not a view body").is_none());
    }

    #[test]
    fn scans_component_turbofish_const_blocks_before_child_delimiters() {
        let source = r##"Foo::<{ make!(" :: ") }> {
    first {}

    second {}
}"##;
        let result = spacing(source);
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
