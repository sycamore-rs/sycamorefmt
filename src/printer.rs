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
use sycamore_view_parser::ir::{DynNode, Node, Prop, PropType, Root, TagIdent, TagNode, TextNode};

use crate::config::Config;
use crate::error::FmtError;
use crate::exprfmt::format_expr;

/// Renders every node in `root`, one per line, with every line (including
/// the first) indented by `indent` spaces. Returns an empty string if `root`
/// has no nodes. Does not add a trailing newline.
pub fn print_root(root: &Root, indent: usize, cfg: &Config) -> Result<String, FmtError> {
    let pad = " ".repeat(indent);
    let mut lines = Vec::with_capacity(root.0.len());
    for node in &root.0 {
        let rendered = print_node(node, indent, cfg)?;
        lines.push(format!("{pad}{rendered}"));
    }
    Ok(lines.join("\n"))
}

/// Renders a single node. See the module docs for the indentation
/// convention used for the returned string.
fn print_node(node: &Node, indent: usize, cfg: &Config) -> Result<String, FmtError> {
    match node {
        Node::Tag(tag) => print_tag(tag, indent, cfg),
        Node::Text(text) => Ok(print_text(text)),
        Node::Dyn(d) => print_dyn(d, indent, cfg),
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

fn render(node: &Node, indent: usize, cfg: &Config) -> Result<Rendered, FmtError> {
    Ok(Rendered {
        text: print_node(node, indent, cfg)?,
    })
}

fn print_tag(tag: &TagNode, indent: usize, cfg: &Config) -> Result<String, FmtError> {
    let head = tag_ident_str(&tag.ident);
    let inner_indent = indent + cfg.tab_spaces;
    let pad = " ".repeat(indent);
    let inner_pad = " ".repeat(inner_indent);

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
        if indent + head.chars().count() + inline.chars().count() <= cfg.max_width {
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
            .map(|n| render(n, inner_indent, cfg))
            .collect::<Result<Vec<_>, _>>()?;
        let all_single_line = rendered_children.iter().all(Rendered::is_single_line);
        // Stacking more than one element/component child onto a shared
        // line reads poorly even when it would fit within the configured
        // width (there is no separator token between children to make the
        // boundary visually obvious), so we only ever attempt an inline
        // combination of multiple children when none of them are tags.
        let multiple_tag_children =
            tag.children.0.len() > 1 && tag.children.0.iter().any(|n| matches!(n, Node::Tag(_)));
        let inline_candidate = if all_single_line && !multiple_tag_children {
            let joined = rendered_children
                .iter()
                .map(|r| r.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let candidate = format!(" {{ {joined} }}");
            let current_col = absolute_column_after(indent, &format!("{head}{props_part}"));
            let full_width_used = current_col + candidate.chars().count();
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
                let block = print_root(&tag.children, inner_indent, cfg)?;
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
fn absolute_column_after(indent: usize, s: &str) -> usize {
    match s.rfind('\n') {
        Some(pos) => s[pos + 1..].chars().count(),
        None => indent + s.chars().count(),
    }
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
        TagIdent::Path(path) => tidy_path(&quote!(#path).to_string()),
        TagIdent::Hyphenated(s) => s.clone(),
    }
}

/// `quote!` renders paths with spaces around `::` (e.g. `foo :: Bar`); this
/// collapses that back down to idiomatic Rust spacing (`foo::Bar`).
fn tidy_path(s: &str) -> String {
    s.replace(" :: ", "::")
        .replace(":: ", "::")
        .replace(" ::", "::")
}
