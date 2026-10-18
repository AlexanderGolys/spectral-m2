//! Syntax-tree preview element.

use m2_syn::{PrettyDetailRole, PrettyNode, PrettyNodeRole};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use super::block::{Block, HLSpan};
use super::cell::{Cell, CodeBlock};
use super::config::UI;
use super::render::RenderContext;
use crate::ui::controller::settings::theme::groups::HLGroup;

/// One committed cell followed by its typed syntax presentation.
pub struct SyntaxTreeBlock {
    code: CodeBlock,
    tree: PrettyNode,
}

/// One displayed tree row with the structural path leading to it.
struct TreeEntry<'a> {
    label: String,
    node: &'a PrettyNode,
}

fn tree_label(parent: &str, child: &str) -> String {
    match (parent, child) {
        (_, "contents" | "delimiter") => parent.to_string(),
        ("", child) => child.to_string(),
        (parent, child) if child.starts_with('[') => format!("{parent}{child}"),
        _ => format!("{parent}.{child}"),
    }
}

fn push_tree_node<'a>(node: &'a PrettyNode, label: String, entries: &mut Vec<TreeEntry<'a>>) {
    match node.role() {
        PrettyNodeRole::Absent => {}
        PrettyNodeRole::Collection if node.name() != "Delimited" => {
            push_tree_children(node, label, entries);
        }
        PrettyNodeRole::Syntax | PrettyNodeRole::Token | PrettyNodeRole::Collection => {
            entries.push(TreeEntry { label, node });
        }
    }
}

fn push_tree_edge<'a>(
    edge: &'a m2_syn::PrettyEdge,
    parent: &str,
    entries: &mut Vec<TreeEntry<'a>>,
) {
    push_tree_node(edge.node(), tree_label(parent, edge.label()), entries);
}

fn push_tree_children<'a>(node: &'a PrettyNode, label: String, entries: &mut Vec<TreeEntry<'a>>) {
    for edge in node.children() {
        push_tree_edge(edge, &label, entries);
    }
}

/// Rows in the order their source reads, which puts an opening bracket before what it
/// holds and its closing one after. The tree stores brackets apart from the contents
/// they enclose, so it lists them last. Rows with no location keep the order they have.
fn in_source_order(entries: &mut [TreeEntry<'_>]) {
    let starts = entries
        .iter()
        .map(|entry| entry.node.span().and_then(m2_syn::Span::start))
        .collect::<Option<Vec<_>>>();
    if starts.is_some() {
        entries.sort_by_key(|entry| entry.node.span().and_then(m2_syn::Span::start));
    }
}

fn tree_children(node: &PrettyNode) -> Vec<TreeEntry<'_>> {
    let mut entries = Vec::new();
    push_tree_children(node, String::new(), &mut entries);
    in_source_order(&mut entries);
    entries
}

fn compact_source(text: &str, width: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if width == 0 {
        return String::new();
    }
    if text.chars().count() <= width {
        text
    } else {
        text.chars()
            .take(width.saturating_sub(1))
            .chain(['…'])
            .collect()
    }
}

fn delimiter_boundaries(node: &PrettyNode) -> Option<(&PrettyNode, Option<&PrettyNode>)> {
    let boundaries = node
        .children()
        .iter()
        .find(|edge| edge.label() == "delimiter")?
        .node()
        .children();
    Some((
        boundaries
            .iter()
            .find(|edge| edge.label() == "opening")?
            .node(),
        boundaries
            .iter()
            .find(|edge| edge.label() == "closing")
            .map(m2_syn::PrettyEdge::node),
    ))
}

fn delimiter_text(node: &PrettyNode) -> &str {
    node.detail().map_or("", m2_syn::PrettyDetail::text)
}

impl SyntaxTreeBlock {
    /// Create a highlighted committed cell and its `m2-syn` pretty tree.
    pub fn new(text: String, spans: Vec<HLSpan>, tree: PrettyNode) -> Self {
        Self {
            code: CodeBlock::new([Cell::new(text, None, spans)]),
            tree,
        }
    }

    /// Resolve the tree into styled display rows, each row's source range set in one
    /// column after the widest row so the ranges read down the page.
    pub fn tree_lines(&self, context: &RenderContext) -> Vec<Line<'static>> {
        /// A row's structure, and the source range it covers.
        type Row = (Line<'static>, Option<String>);

        fn line(
            node: &PrettyNode,
            label: Option<&str>,
            prefix: &str,
            branch: &str,
            delimiter: bool,
            context: &RenderContext,
        ) -> Row {
            let groups = context.groups();
            let name = groups.style(HLGroup::Type);
            let mut spans = vec![Span::styled(format!("{prefix}{branch}"), groups.muted())];
            let token = (node.role() == PrettyNodeRole::Token)
                .then(|| {
                    node.name()
                        .strip_prefix("Token![")
                        .and_then(|name| name.strip_suffix(']'))
                })
                .flatten();
            if delimiter {
                spans.push(Span::styled(
                    delimiter_text(node).to_string(),
                    groups.tree_code(),
                ));
            } else if let Some(token) = token {
                spans.extend([
                    Span::styled("Token![", groups.info()),
                    Span::styled(token.to_string(), groups.tree_code()),
                    Span::styled("]", groups.info()),
                ]);
            } else {
                spans.push(Span::styled(node.name().to_string(), name));
            }
            if let Some(label) = label.filter(|label| {
                !label.is_empty()
                    && !label.bytes().all(|byte| byte.is_ascii_digit())
                    && (delimiter || node.role() != PrettyNodeRole::Token)
            }) {
                spans.extend([
                    Span::styled("  ", groups.muted()),
                    Span::styled(label.to_string(), groups.style(HLGroup::Property)),
                    Span::styled(":", groups.muted()),
                ]);
            }
            if !delimiter && token.is_none() {
                if let Some(detail) = node.detail().filter(|detail| {
                    detail.role() == PrettyDetailRole::Text
                        && node.span().and_then(|span| span.text()) == Some(detail.text())
                }) {
                    spans.push(Span::styled(
                        format!(" {}", detail.text()),
                        groups.tree_code(),
                    ));
                }
            }
            let location = node.span().map(|span| span.to_string());
            if !delimiter && node.role() != PrettyNodeRole::Token {
                let occupied = Line::from(spans.clone()).width()
                    + location.as_ref().map_or(0, |text| text.chars().count())
                    + UI.tree_location_gap
                    + 1;
                let width = usize::from(context.width())
                    .saturating_sub(occupied)
                    .min(48);
                if let Some(text) = node
                    .span()
                    .and_then(|span| span.text())
                    .map(|text| compact_source(text, width))
                    .filter(|text| !text.is_empty())
                {
                    spans.push(Span::styled(format!(" {text}"), groups.tree_code()));
                }
            }
            (Line::from(spans), location)
        }

        fn walk_entry(
            entry: &TreeEntry<'_>,
            prefix: String,
            last: bool,
            context: &RenderContext,
            lines: &mut Vec<Row>,
        ) {
            let branch = if last { "╰─" } else { "├─" };
            if let Some((opening, closing)) = (entry.node.name() == "Delimited")
                .then(|| delimiter_boundaries(entry.node))
                .flatten()
            {
                lines.push(line(
                    opening,
                    Some(&entry.label),
                    &prefix,
                    branch,
                    true,
                    context,
                ));
                let prefix = format!("{prefix}{}", if last { "  " } else { "│ " });
                let mut children: Vec<_> = entry
                    .node
                    .children()
                    .iter()
                    .filter(|edge| edge.label() != "delimiter")
                    .flat_map(|edge| {
                        let mut entries = Vec::new();
                        push_tree_edge(edge, "", &mut entries);
                        entries
                    })
                    .collect();
                in_source_order(&mut children);
                let trailing = closing.is_some();
                for (index, child) in children.iter().enumerate() {
                    walk_entry(
                        child,
                        prefix.clone(),
                        !trailing && index + 1 == children.len(),
                        context,
                        lines,
                    );
                }
                if let Some(closing) = closing {
                    lines.push(line(closing, None, &prefix, "╰─", true, context));
                }
                return;
            }
            lines.push(line(
                entry.node,
                Some(&entry.label),
                &prefix,
                branch,
                false,
                context,
            ));
            let children = tree_children(entry.node);
            let prefix = format!("{prefix}{}", if last { "  " } else { "│ " });
            for (index, entry) in children.iter().enumerate() {
                walk_entry(
                    entry,
                    prefix.clone(),
                    index + 1 == children.len(),
                    context,
                    lines,
                );
            }
        }

        let mut lines = vec![line(&self.tree, None, "", "", false, context)];
        let children = tree_children(&self.tree);
        for (index, entry) in children.iter().enumerate() {
            walk_entry(
                entry,
                String::new(),
                index + 1 == children.len(),
                context,
                &mut lines,
            );
        }
        let column = lines
            .iter()
            .map(|(structure, _)| structure.width())
            .max()
            .unwrap_or(0)
            + UI.tree_location_gap;
        let muted = context.groups().muted();
        let mut lines: Vec<Line<'static>> = lines
            .into_iter()
            .map(|(mut structure, location)| {
                if let Some(location) = location {
                    let room =
                        usize::from(context.width()).saturating_sub(location.chars().count());
                    let start = column
                        .min(room)
                        .max(structure.width() + UI.tree_location_gap);
                    structure
                        .spans
                        .push(Span::styled(" ".repeat(start - structure.width()), muted));
                    structure.spans.push(Span::styled(location, muted));
                }
                structure
            })
            .collect();
        lines.push(Line::default());
        lines
    }

    /// The block's rows inside the margin kept clear to its left.
    fn lines(&self, context: &RenderContext) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
        let inner = context.with_width(context.width().saturating_sub(UI.feed_margin));
        (self.code.preview_lines(&inner), self.tree_lines(&inner))
    }
}

/// Rows above the source taken by a rule and the margin inside it. The source brings
/// the matching margin and rule below itself, so both sides are the same.
const SOURCE_LEAD: u16 = 2;

impl Block for SyntaxTreeBlock {
    fn height(&self, context: &RenderContext) -> u16 {
        let (source, tree) = self.lines(context);
        let rows = |lines: &[Line<'static>]| u16::try_from(lines.len()).unwrap_or(u16::MAX);
        // A rule and an empty row above the source, a rule below its own empty row.
        SOURCE_LEAD
            .saturating_add(rows(&source))
            .saturating_add(1)
            .saturating_add(rows(&tree))
    }

    fn render_local(&self, context: &RenderContext, buf: &mut Buffer) {
        let area = *buf.area();
        let groups = context.groups();
        let (source, tree) = self.lines(context);
        buf.set_style(area, groups.code());
        let source_height = u16::try_from(source.len()).unwrap_or(u16::MAX);
        let margin = UI.feed_margin.min(area.width);
        let inner_width = area.width - margin;
        let mut rule = |row: u16| {
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(area.width.into()),
                groups.border(),
            )))
            .style(groups.code())
            .render(Rect::new(area.x, area.y.saturating_add(row), area.width, 1), buf);
        };
        let below_source = SOURCE_LEAD.saturating_add(source_height);
        rule(0);
        rule(below_source);
        Paragraph::new(source).style(groups.code()).render(
            Rect::new(
                area.x + margin,
                area.y.saturating_add(SOURCE_LEAD),
                inner_width,
                source_height,
            ),
            buf,
        );
        let tree_top = below_source.saturating_add(1);
        Paragraph::new(tree).style(groups.code()).render(
            Rect::new(
                area.x + margin,
                area.y.saturating_add(tree_top),
                inner_width,
                area.height.saturating_sub(tree_top),
            ),
            buf,
        );
    }
}
