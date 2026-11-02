//! Pretty-printer for the Sycamore `view!` macro IR
//! (`sycamore_view_parser::ir`).
//!
//! Every rendering function follows one convention throughout this module:
//! the returned `String`'s *first* line has no leading indentation baked in
//! (the caller is expected to place it right after whatever precedes it on
//! the current line), but any *subsequent* lines already contain their full,
//! absolute leading whitespace. This makes it possible to freely compose
//! renderings (e.g. embed a multi-line prop value inside a single-line
//! `name=value` prop, itself embedded in a tag) without every layer needing
//! to know about the others' indentation math.

use quote::quote;
use sycamore_view_parser::ir::{
    DynNode, IfNode, MatchNode, Node, Prop, PropType, Root, TagIdent, TagNode, TextNode,
};
use unicode_width::UnicodeWidthStr;

use crate::config::{Config, indent_padding};
use crate::error::FmtError;
use crate::exprfmt::{format_expr, format_pattern};
use crate::trivia::{ControlSpacing, SpacingNode, SpacingRoot};

/// Renders every node in `root`, one per line, with every line (including
/// the first) indented by `indent` spaces. Returns an empty string if `root`
/// has no nodes. Does not add a trailing newline.
pub fn print_root(
    root: &Root,
    spacing: &SpacingRoot,
    indent: usize,
    cfg: &Config,
) -> Result<String, FmtError> {
    spacing.validate_shape(root)?;
    // Render the first node before allocating root padding, so a deeper
    // indentation overflow is reported without attempting a huge allocation.
    let mut pad = String::new();
    let mut output = String::new();
    for (index, node) in root.0.iter().enumerate() {
        if index > 0 {
            output.push_str(if spacing.has_blank_line_between(index - 1) {
                "\n\n"
            } else {
                "\n"
            });
        }
        let rendered = print_node(node, spacing.node(index), indent, cfg)?;
        if index == 0 {
            pad = indent_padding(indent)?;
        }
        output.push_str(&pad);
        output.push_str(&rendered);
    }
    Ok(output)
}

/// Renders a single node. See the module docs for the indentation
/// convention used for the returned string.
fn print_node(
    node: &Node,
    spacing: &SpacingNode,
    indent: usize,
    cfg: &Config,
) -> Result<String, FmtError> {
    match node {
        Node::Tag(tag) => {
            require_no_control_spacing(&spacing.control)?;
            print_tag(tag, &spacing.children, indent, cfg)
        }
        Node::Text(text) => {
            require_no_control_spacing(&spacing.control)?;
            Ok(print_text(text))
        }
        Node::Dyn(d) => {
            require_no_control_spacing(&spacing.control)?;
            print_dyn(d, indent, cfg)
        }
        Node::If(if_node) => print_if(if_node, spacing, indent, cfg),
        Node::Match(match_node) => print_match(match_node, spacing, indent, cfg),
    }
}

fn checked_indent(indent: usize, tab_spaces: usize) -> Result<usize, FmtError> {
    indent
        .checked_add(tab_spaces)
        .ok_or_else(|| FmtError::Internal("printer indentation overflow".into()))
}

fn require_no_control_spacing(control: &ControlSpacing) -> Result<(), FmtError> {
    if matches!(control, ControlSpacing::None) {
        Ok(())
    } else {
        Err(FmtError::Internal(
            "non-control node has control-flow trivia".into(),
        ))
    }
}

fn print_text(text: &TextNode) -> String {
    // `quote!` re-emits the literal's original token exactly (including its
    // original quoting style, e.g. `"foo"` vs `r"foo"`), so this preserves
    // the user's literal verbatim.
    let lit = &text.value;
    quote!(#lit).to_string()
}

fn print_dyn(d: &DynNode, indent: usize, cfg: &Config) -> Result<String, FmtError> {
    let expr = format_expr(&d.value, indent, cfg)?;
    Ok(format!("({expr})"))
}

fn print_if(
    if_node: &IfNode,
    spacing: &SpacingNode,
    indent: usize,
    cfg: &Config,
) -> Result<String, FmtError> {
    let condition = format_expr(&if_node.cond, indent, cfg)?;
    let (then_spacing, else_spacing, else_if) = match &spacing.control {
        ControlSpacing::If {
            then_branch,
            else_branch,
            else_if,
        } => (then_branch, else_branch.as_ref(), *else_if),
        _ => {
            return Err(FmtError::Internal(
                "if node has mismatched branch trivia".into(),
            ));
        }
    };
    if else_spacing.is_some() != if_node.else_branch.is_some() {
        return Err(FmtError::Internal(
            "if node and spacing disagree about else branch presence".into(),
        ));
    }
    if else_if && else_spacing.is_none() {
        return Err(FmtError::Internal(
            "if node marks a missing else branch as else-if".into(),
        ));
    }
    let then_body = print_block(&if_node.then, then_spacing, indent, cfg)?;
    let mut output = format!("if {condition}{then_body}");

    if let (Some(ir_else_branch), Some(else_spacing)) = (&if_node.else_branch, else_spacing) {
        else_spacing.validate_shape(ir_else_branch)?;
        if else_if {
            let [Node::If(nested)] = ir_else_branch.0.as_slice() else {
                return Err(FmtError::Internal(
                    "else-if node does not contain exactly one if branch".into(),
                ));
            };
            let nested_spacing = else_spacing.node(0);
            let rendered = print_if(nested, nested_spacing, indent, cfg)?;
            output.push_str(" else ");
            output.push_str(&rendered);
        } else {
            let rendered = print_block(ir_else_branch, else_spacing, indent, cfg)?;
            output.push_str(" else");
            output.push_str(&rendered);
        }
    }

    Ok(output)
}

fn print_match(
    match_node: &MatchNode,
    spacing: &SpacingNode,
    indent: usize,
    cfg: &Config,
) -> Result<String, FmtError> {
    let expression = format_expr(&match_node.expr, indent, cfg)?;
    let arm_spacings = match &spacing.control {
        ControlSpacing::Match { arms } if arms.len() == match_node.arms.len() => arms,
        _ => {
            return Err(FmtError::Internal(
                "match node is missing arm trivia".into(),
            ));
        }
    };
    let arm_indent = checked_indent(indent, cfg.tab_spaces)?;
    let arm_pad = indent_padding(arm_indent)?;
    let closing_pad = indent_padding(indent)?;
    let mut arms = Vec::with_capacity(match_node.arms.len());

    for (arm, arm_spacing) in match_node.arms.iter().zip(arm_spacings) {
        let pattern = format_pattern(&arm.pat, arm_indent, cfg)?;
        let body = print_block(&arm.body, arm_spacing, arm_indent, cfg)?;
        arms.push(format!("{arm_pad}{pattern} =>{body},"));
    }

    if arms.is_empty() {
        Ok(format!("match {expression} {{}}"))
    } else {
        Ok(format!(
            "match {expression} {{\n{}\n{closing_pad}}}",
            arms.join("\n")
        ))
    }
}

fn print_block(
    root: &Root,
    spacing: &SpacingRoot,
    indent: usize,
    cfg: &Config,
) -> Result<String, FmtError> {
    spacing.validate_shape(root)?;
    if root.0.is_empty() {
        Ok(" {}".to_string())
    } else {
        let body = print_root(root, spacing, checked_indent(indent, cfg.tab_spaces)?, cfg)?;
        Ok(format!(" {{\n{body}\n{}}}", indent_padding(indent)?))
    }
}

/// A node's rendering, along with whether it could be produced without any
/// internal line breaks (and is therefore safe to combine with siblings on
/// one shared line).
struct Rendered {
    text: String,
}

impl Rendered {
    fn is_single_line(&self) -> bool {
        !self.text.contains('\n')
    }
}

fn render(
    node: &Node,
    spacing: &SpacingNode,
    indent: usize,
    cfg: &Config,
) -> Result<Rendered, FmtError> {
    Ok(Rendered {
        text: print_node(node, spacing, indent, cfg)?,
    })
}

fn print_tag(
    tag: &TagNode,
    spacing: &SpacingRoot,
    indent: usize,
    cfg: &Config,
) -> Result<String, FmtError> {
    spacing.validate_shape(&tag.children)?;
    let inner_indent = checked_indent(indent, cfg.tab_spaces)?;
    let head = tag_ident_str(&tag.ident);
    let pad = indent_padding(indent)?;
    let inner_pad = indent_padding(inner_indent)?;

    let has_props = !tag.props.is_empty();
    let has_children = !tag.children.0.is_empty();

    // Render every prop once; whether we lay them out on one line or one
    // per line is decided below based on available width.
    let rendered_props = tag
        .props
        .iter()
        .map(|p| print_prop(p, inner_indent, cfg))
        .collect::<Result<Vec<_>, _>>()?;
    let props_all_single_line = rendered_props.iter().all(|s| !s.contains('\n'));

    let props_part = if !has_props {
        String::new()
    } else if props_all_single_line {
        let joined = rendered_props.join(", ");
        let inline = format!("({joined})");
        let column = display_column_after(indent, &head, cfg.tab_spaces);
        if display_column_after(column, &inline, cfg.tab_spaces) <= cfg.max_width {
            inline
        } else {
            render_props_broken(&rendered_props, &inner_pad, &pad)
        }
    } else {
        render_props_broken(&rendered_props, &inner_pad, &pad)
    };

    // Render children, deciding between an inline `{ a b c }` block and a
    // fully broken-out `{\n    a\n    b\n}` block.
    let children_part = if !has_children {
        if has_props {
            String::new()
        } else {
            // Grammar requires either parens or braces; with no props we
            // must emit an explicit (possibly empty) brace block.
            " {}".to_string()
        }
    } else {
        let rendered_children = tag
            .children
            .0
            .iter()
            .enumerate()
            .map(|(index, n)| render(n, spacing.node(index), inner_indent, cfg))
            .collect::<Result<Vec<_>, _>>()?;
        let all_single_line = rendered_children.iter().all(Rendered::is_single_line);
        // Stacking more than one element/component child onto a shared
        // line reads poorly even when it would fit within the configured
        // width (there is no separator token between children to make the
        // boundary visually obvious), so we only ever attempt an inline
        // combination of multiple children when none of them are tags.
        let multiple_tag_children =
            tag.children.0.len() > 1 && tag.children.0.iter().any(|n| matches!(n, Node::Tag(_)));
        let inline_candidate =
            if all_single_line && !multiple_tag_children && !spacing.has_blank_line() {
                let joined = rendered_children
                    .iter()
                    .map(|r| r.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let candidate = format!(" {{ {joined} }}");
                let current_col =
                    absolute_column_after(indent, &format!("{head}{props_part}"), cfg.tab_spaces);
                let full_width_used = display_column_after(current_col, &candidate, cfg.tab_spaces);
                if full_width_used <= cfg.max_width && !joined.is_empty() {
                    Some(candidate)
                } else {
                    None
                }
            } else {
                None
            };

        match inline_candidate {
            Some(c) => c,
            None => {
                let mut block = String::new();
                for (index, child) in rendered_children.iter().enumerate() {
                    if index > 0 {
                        block.push_str(if spacing.has_blank_line_between(index - 1) {
                            "\n\n"
                        } else {
                            "\n"
                        });
                    }
                    block.push_str(&inner_pad);
                    block.push_str(&child.text);
                }
                format!(" {{\n{block}\n{pad}}}")
            }
        }
    };

    Ok(format!("{head}{props_part}{children_part}"))
}

fn render_props_broken(rendered_props: &[String], inner_pad: &str, pad: &str) -> String {
    let lines: Vec<String> = rendered_props
        .iter()
        .map(|p| format!("{inner_pad}{p},"))
        .collect();
    format!("(\n{}\n{pad})", lines.join("\n"))
}

/// Given that a rendering `s` (following this module's indentation
/// convention: first line has no leading indent, later lines are already
/// absolute) is placed starting at column `indent`, returns the absolute
/// column immediately after the end of `s`.
fn absolute_column_after(indent: usize, s: &str, tab_spaces: usize) -> usize {
    match s.rfind('\n') {
        Some(pos) => display_column_after(0, &s[pos + 1..], tab_spaces),
        None => display_column_after(indent, s, tab_spaces),
    }
}

/// Measures display columns, including tab stops in verbatim source literals.
/// A zero-width indentation setting uses single-column tab stops, matching
/// the source indentation logic in the formatting pipeline.
fn display_column_after(mut column: usize, text: &str, tab_spaces: usize) -> usize {
    let tab_width = tab_spaces.max(1);
    for (index, segment) in text.split('\t').enumerate() {
        if index > 0 {
            column = column.saturating_add(tab_width - column % tab_width);
        }
        // Treat an unrepresentable width as usize::MAX rather than wrapping
        // around and making an over-wide line appear short.
        column = column.saturating_add(UnicodeWidthStr::width(segment));
    }
    column
}

fn print_prop(prop: &Prop, indent: usize, cfg: &Config) -> Result<String, FmtError> {
    let value = format_expr(&prop.value, indent, cfg)?;
    let rendered = match &prop.ty {
        PropType::Plain { ident } => format!("{ident}={value}"),
        PropType::PlainHyphenated { ident } => format!("{ident}={value}"),
        PropType::PlainQuoted { ident } => format!("{:?}={value}", ident),
        PropType::Directive { dir, ident } => format!("{dir}:{ident}={value}"),
        PropType::Ref => format!("r#ref={value}"),
        PropType::Spread => format!("..{value}"),
    };
    Ok(rendered)
}

fn tag_ident_str(ident: &TagIdent) -> String {
    match ident {
        TagIdent::Path(path) => {
            // Join path segments structurally: replacing ` :: ` in quoted
            // source would also rewrite literals and macro arguments inside
            // generic arguments, changing their meaning.
            let mut output = String::new();
            if path.leading_colon.is_some() {
                output.push_str("::");
            }
            for (index, segment) in path.segments.iter().enumerate() {
                if index > 0 {
                    output.push_str("::");
                }
                output.push_str(&segment.ident.to_string());
                match &segment.arguments {
                    syn::PathArguments::None => {}
                    syn::PathArguments::AngleBracketed(args) if args.colon2_token.is_some() => {
                        output.push_str("::");
                        let mut args = args.clone();
                        args.colon2_token = None;
                        output.push_str(&quote!(#args).to_string());
                    }
                    args => output.push_str(&quote!(#args).to_string()),
                }
            }
            output
        }
        TagIdent::Hyphenated(s) => s.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{display_column_after, print_root};
    use crate::{Config, trivia::SpacingRoot};
    use sycamore_view_parser::ir::Root;

    #[test]
    fn display_column_saturates_for_tabs_and_wide_text() {
        assert_eq!(display_column_after(usize::MAX, "\t", 4), usize::MAX);
        assert_eq!(display_column_after(usize::MAX, "x", 4), usize::MAX);
    }

    #[test]
    fn root_checks_nested_indent_before_allocating_padding() {
        let source = "div {}";
        let root: Root = syn::parse_str(source).expect("valid view body");
        let spacing = SpacingRoot::try_from_root(&root, source).expect("matching trivia");
        assert!(print_root(&root, &spacing, usize::MAX, &Config::default()).is_err());
    }
}
