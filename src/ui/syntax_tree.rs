//! The syntax tree view's block: a committed cell's code as it was entered, over its
//! m2-syn tree one node a row. A click on a row picks its node, shown in colour in
//! the code above while the rest is grayed out; a row holding others folds them.

use std::collections::BTreeSet;
use std::ops::Range;

use m2_syn::{PrettyDetail, PrettyDetailRole, PrettyEdge, PrettyNode, PrettyNodeRole, Span};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span as Styled};
use ratatui::widgets::{Paragraph, Widget};

use super::block::{Block, CodeRange, Text, TextSpan};
use super::cell::M2CodeCell;
use super::render::selection::Inclusions;
use super::render::RenderContext;
use crate::app::session::CommittedCell;
use crate::code::highlight::Highlighter;
use crate::code::semantic::SourceId;
use crate::ui::glyphs::{Glyphs, Spelled};
use crate::ui::theme::groups::{Groups, HLGroup};

/// One committed cell: its code above the rows of its syntax tree, which node is
/// picked and which rows are folded.
pub struct SyntaxTreeBlock {
    code: M2CodeCell,
    tree: SyntaxTree,
    source: SourceId,
    picked: Option<usize>,
    /// Rows whose descendants are folded away.
    folded: BTreeSet<usize>,
}

/// A cell's syntax tree as the view lists it: every node a row, each followed by
/// its descendants, siblings in the order the source reads.
struct SyntaxTree {
    rows: Vec<TreeRow>,
}

/// One node of a syntax tree as a row of the view.
///
/// In
/// ```text
/// Binary
/// ├─ left: Identifier y
/// ╰─ right: Integer 1
/// ```
/// the last row has `lasts == [true]` and the text `right: Integer 1`, and the
/// first row has two descendants.
struct TreeRow {
    /// For each node from the root's child down to this one, whether it is the last
    /// of its siblings.
    lasts: Vec<bool>,
    /// The field it fills, then what it is.
    text: Vec<TextSpan>,
    /// The bytes of the shown code it covers.
    code: Range<usize>,
    /// How many rows after it are its descendants.
    descendants: usize,
}

/// A child as the tree stores it, its field named through the categories that
/// only say how a visitor dispatches on it.
struct TreeEntry<'a> {
    label: String,
    node: &'a PrettyNode,
}

/// How the tree's spans land in the code shown: moved by `origin` bytes, and kept
/// within its `len`.
#[derive(Clone, Copy)]
struct Placement {
    origin: isize,
    len: usize,
}

/// Where each part of the block sits, in its own rows.
struct Sections {
    code: Range<u16>,
    tree: Range<u16>,
}

impl Placement {
    fn place(self, bytes: Range<usize>) -> Range<usize> {
        let shown = |byte: usize| byte.saturating_add_signed(self.origin).min(self.len);
        shown(bytes.start)..shown(bytes.end)
    }
}

impl TreeEntry<'_> {
    /// The field a child fills, read through its parent's: `args` and `[0]` make
    /// `args[0]`, and a label that only says a value is held adds nothing.
    fn label(parent: &str, child: &str) -> String {
        match (parent, child) {
            (_, "" | "contents" | "delimiter") => parent.to_string(),
            ("", child) => child.to_string(),
            (parent, child) if child.starts_with('[') => format!("{parent}{child}"),
            _ => format!("{parent}.{child}"),
        }
    }

    /// The children of `node` the view shows, in the order their source reads.
    /// Rows with no location keep the order they have.
    fn children(node: &PrettyNode) -> Vec<TreeEntry<'_>> {
        let mut entries = Vec::new();
        Self::push_children(node, "", &mut entries);
        let starts = entries
            .iter()
            .map(|entry| entry.node.span().and_then(Span::start))
            .collect::<Option<Vec<_>>>();
        if starts.is_some() {
            entries.sort_by_key(|entry| entry.node.span().and_then(Span::start));
        }
        entries
    }

    fn push_children<'a>(node: &'a PrettyNode, label: &str, entries: &mut Vec<TreeEntry<'a>>) {
        for edge in node.children() {
            Self::push_edge(edge, label, entries);
        }
    }

    fn push_edge<'a>(edge: &'a PrettyEdge, parent: &str, entries: &mut Vec<TreeEntry<'a>>) {
        let (node, label) = (edge.node(), Self::label(parent, edge.label()));
        match node.role() {
            PrettyNodeRole::Absent => {}
            PrettyNodeRole::Collection if node.children().is_empty() => {}
            PrettyNodeRole::Category => Self::push_children(node, &label, entries),
            PrettyNodeRole::Syntax
            | PrettyNodeRole::Token
            | PrettyNodeRole::Delimiter
            | PrettyNodeRole::Collection => entries.push(TreeEntry { label, node }),
        }
    }
}

impl TreeRow {
    /// `label: Kind text` for a node, or the text alone for a token or bracket.
    fn text(label: &str, node: &PrettyNode) -> Vec<TextSpan> {
        let mut text = Vec::new();
        if !label.is_empty() {
            text.push(TextSpan::new(label, HLGroup::Property));
            text.push(TextSpan::new(": ", HLGroup::SyntaxTreeLabelSeparator));
        }
        match node.role() {
            PrettyNodeRole::Token | PrettyNodeRole::Delimiter => text.push(TextSpan::new(
                node.detail().map_or(node.name(), PrettyDetail::text),
                HLGroup::Symbol,
            )),
            _ => {
                text.push(TextSpan::new(node.name(), HLGroup::Type));
                let own = node.detail().filter(|detail| {
                    detail.role() == PrettyDetailRole::Text
                        && node.span().and_then(Span::text) == Some(detail.text())
                });
                if let Some(detail) = own {
                    text.push(TextSpan::new(
                        format!(" {}", detail.text()),
                        HLGroup::Symbol,
                    ));
                }
            }
        }
        text
    }

    /// The branches leading to the row: a trunk for every node above it that has
    /// siblings after it, and its own branch.
    fn limb(&self) -> String {
        let Some((&last, above)) = self.lasts.split_last() else {
            return String::new();
        };
        above
            .iter()
            .map(|&last| Glyphs::tree_limb(last).1.current())
            .chain([Glyphs::tree_limb(last).0.current()])
            .collect()
    }

    /// The column of the fold marker, which sits in the gap that ends the branch, on
    /// a row with descendants; the root's never fold.
    fn marker_column(&self) -> Option<u16> {
        (!self.lasts.is_empty() && self.descendants > 0)
            .then(|| Line::from(self.limb()).width().saturating_sub(1) as u16)
    }
}

impl SyntaxTree {
    fn new(root: &PrettyNode, placement: Placement) -> Self {
        let mut tree = Self { rows: Vec::new() };
        tree.push(root, "", Vec::new(), 0..placement.len, placement);
        tree
    }

    /// Add `node`'s row and its descendants'; a node with no location of its own
    /// covers its parent's code, `around`.
    fn push(
        &mut self,
        node: &PrettyNode,
        label: &str,
        lasts: Vec<bool>,
        around: Range<usize>,
        placement: Placement,
    ) {
        let index = self.rows.len();
        let code = node
            .span()
            .and_then(Span::range)
            .map_or(around, |bytes| placement.place(bytes));
        self.rows.push(TreeRow {
            text: TreeRow::text(label, node),
            lasts: lasts.clone(),
            code: code.clone(),
            descendants: 0,
        });
        let children = TreeEntry::children(node);
        let count = children.len();
        for (position, child) in children.into_iter().enumerate() {
            let mut lasts = lasts.clone();
            lasts.push(position + 1 == count);
            self.push(child.node, &child.label, lasts, code.clone(), placement);
        }
        self.rows[index].descendants = self.rows.len() - index - 1;
    }

    /// The deepest node covering byte `offset` of the code: the last row that does,
    /// as each node's row comes before its descendants'.
    fn deepest_at(&self, offset: usize) -> Option<usize> {
        self.rows.iter().rposition(|row| row.code.contains(&offset))
    }

    /// The rows above `row` that hold it.
    fn ancestors(&self, row: usize) -> impl Iterator<Item = usize> + '_ {
        (0..row).filter(move |&above| row <= above + self.rows[above].descendants)
    }
}

impl SyntaxTreeBlock {
    /// Rows a rule takes, and the blank rows between a rule and what it heads or ends.
    const RULE: u16 = 1;
    const PADDING: u16 = 1;

    /// The block for one committed cell, its code highlighted as the shell shows it.
    pub fn new(cell: &CommittedCell, highlighter: &mut dyn Highlighter) -> Self {
        let placement = Placement {
            origin: cell.tree_origin,
            len: cell.code.len(),
        };
        Self {
            code: M2CodeCell::highlighted(cell.code.clone(), None, cell.source_id, highlighter),
            tree: SyntaxTree::new(&cell.tree, placement),
            source: cell.source_id,
            picked: None,
            folded: BTreeSet::new(),
        }
    }

    /// The rows left after folding, in order.
    fn shown(&self) -> Vec<usize> {
        let mut shown = Vec::new();
        let mut next = 0;
        while let Some(row) = self.tree.rows.get(next) {
            let hidden = match self.folded.contains(&next) {
                true => row.descendants,
                false => 0,
            };
            shown.push(next);
            next += 1 + hidden;
        }
        shown
    }

    fn sections(&self, context: &RenderContext, shown: usize) -> Sections {
        let code_rows = self
            .code
            .wrapped(context.width(), context.code_layout())
            .height();
        let code = Self::RULE + Self::PADDING..Self::RULE + Self::PADDING + code_rows;
        let tree_top = code.end + Self::PADDING + Self::RULE;
        Sections {
            code,
            tree: tree_top..tree_top.saturating_add(shown as u16),
        }
    }

    /// The tree row drawn on local row `y`.
    fn row_at(&self, y: u16, context: &RenderContext) -> Option<usize> {
        let shown = self.shown();
        let tree = self.sections(context, shown.len()).tree;
        tree.contains(&y)
            .then(|| shown[usize::from(y - tree.start)])
    }

    /// Fold or unfold `row`; whether it holds anything to fold.
    fn toggle(&mut self, row: usize) -> bool {
        if self.tree.rows[row].marker_column().is_none() {
            return false;
        }
        if !self.folded.remove(&row) {
            self.folded.insert(row);
        }
        true
    }

    fn tree_line(&self, row: usize, groups: &Groups) -> Line<'static> {
        let tree_row = &self.tree.rows[row];
        let mut spans = vec![Styled::styled(tree_row.limb(), groups.syntax_tree_branch())];
        spans.extend(tree_row.text.spans(groups));
        if self.folded.contains(&row) {
            spans.push(Styled::styled(
                format!(
                    " {} {}",
                    Glyphs::FoldedLines.current(),
                    tree_row.descendants
                ),
                groups.fold_marker(),
            ));
        }
        Line::from(spans)
    }

    /// Gray out the code but for the picked node's, which keeps its colours and
    /// takes the selection's background.
    fn paint_picked(&self, context: &RenderContext, code: Rect, buf: &mut Buffer) {
        let Some(picked) = self.picked else {
            return;
        };
        let groups = context.groups();
        let lit = self
            .code
            .wrapped(context.width(), context.code_layout())
            .cells_of(&self.tree.rows[picked].code)
            .into_iter()
            .map(|cells| Rect {
                x: code.x + cells.x,
                y: code.y + cells.y,
                ..cells
            })
            .collect::<Vec<_>>();
        for position in code.positions() {
            let cell = &mut buf[position];
            match lit.iter().any(|cells| cells.contains(position)) {
                true => groups.selection().bg.map(|bg| cell.set_bg(bg)),
                false => groups.muted_code_text().fg.map(|fg| cell.set_fg(fg)),
            };
        }
    }
}

impl Block for SyntaxTreeBlock {
    fn height(&self, context: &RenderContext) -> u16 {
        self.sections(context, self.shown().len()).tree.end + Self::PADDING
    }

    fn render_local(&self, context: &RenderContext, buf: &mut Buffer) {
        let area = *buf.area();
        let groups = context.groups();
        let shown = self.shown();
        let sections = self.sections(context, shown.len());
        let band = |rows: &Range<u16>| {
            Rect::new(
                area.x,
                area.y + rows.start,
                area.width,
                rows.end - rows.start,
            )
        };
        buf.set_style(area, groups.code());
        let rule = Glyphs::TreeRule.current().repeat(area.width.into());
        for y in [0, sections.code.end + Self::PADDING] {
            buf.set_string(area.x, area.y + y, &rule, groups.syntax_tree_border());
        }
        let code = band(&sections.code);
        Paragraph::new(self.code.bare_lines(context))
            .style(groups.code())
            .render(code, buf);
        self.paint_picked(context, code, buf);
        let tree = band(&sections.tree);
        Paragraph::new(
            shown
                .iter()
                .map(|&row| self.tree_line(row, groups))
                .collect::<Vec<_>>(),
        )
        .style(groups.code())
        .render(tree, buf);
        for (&row, y) in shown.iter().zip(tree.y..) {
            if let Some(x) = self.tree.rows[row].marker_column() {
                let glyph = match self.folded.contains(&row) {
                    true => Glyphs::FoldClosed,
                    false => Glyphs::FoldOpen,
                };
                buf.set_string(tree.x + x, y, glyph.current(), groups.fold_marker());
            }
            if self.picked == Some(row) {
                if let Some(bg) = groups.selection().bg {
                    buf.set_style(Rect::new(tree.x, y, tree.width, 1), groups.code().bg(bg));
                }
            }
        }
    }

    fn local_selection_inclusions(&self, context: &RenderContext, inclusions: &mut Inclusions) {
        let sections = self.sections(context, self.shown().len());
        for rows in [sections.code, sections.tree] {
            inclusions.push(Rect::new(
                0,
                rows.start,
                context.width(),
                rows.end - rows.start,
            ));
        }
    }

    /// Fold a row clicked anywhere but on its marker, which folds on its own click.
    fn toggle_fold(&mut self, position: Position, context: &RenderContext) -> bool {
        self.row_at(position.y, context).is_some_and(|row| {
            self.tree.rows[row].marker_column() != Some(position.x) && self.toggle(row)
        })
    }

    /// Pick the node of a tree row, folding it too on its marker, or the deepest node
    /// under a click in the code; whatever folds hide it open.
    fn pick(&mut self, position: Position, context: &RenderContext) -> Option<CodeRange> {
        let row = match self.row_at(position.y, context) {
            Some(row) => {
                if self.tree.rows[row].marker_column() == Some(position.x) {
                    self.toggle(row);
                }
                row
            }
            None => {
                let code = self.sections(context, 0).code;
                let local = Position::new(position.x, position.y.checked_sub(code.start)?);
                let offset = self
                    .code
                    .wrapped(context.width(), context.code_layout())
                    .offset_at(local)?;
                self.tree.deepest_at(offset)?
            }
        };
        for above in self.tree.ancestors(row).collect::<Vec<_>>() {
            self.folded.remove(&above);
        }
        self.picked = Some(row);
        let SourceId::Cell {
            submission, index, ..
        } = self.source
        else {
            unreachable!("committed code is a cell of a submission")
        };
        Some(CodeRange {
            submission,
            index,
            bytes: self.tree.rows[row].code.clone(),
        })
    }

    fn unpick(&mut self) -> bool {
        self.picked.take().is_some()
    }

    fn rehighlight(&mut self, sources: &[SourceId], highlighter: &mut dyn Highlighter) -> bool {
        sources.contains(&self.source) && self.code.rehighlight(self.source, highlighter)
    }
}
