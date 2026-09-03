//! Typed scrollable editor feed entries.
//!
//! Every [`Block`] owns text, measures and renders against a borrowed
//! [`RenderContext`], and inherits the same visual-line wrapper. Highlighted text
//! receives its highlighter separately because highlighting is an application-wide
//! service rather than editor state.

use m2_syn::{PrettyDetailRole, PrettyNode, PrettyNodeRole};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use self::layout::WrappedText;
use self::render::{Bordered, Highlights, RenderContext};
use crate::backend::transcript::strip_m2_location;
use crate::ui::controller::settings::theme::groups::HLGroup;
use crate::ui::element::{Element, HLSpan, SpanText};

pub mod layout;
pub mod render;

/// The semantic role of one feed block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Input,
    System,
    Printed,
    Value,
    Warning,
    Error,
}

/// A measured and renderable entry in the transcript feed.
pub trait Block: Element<Context = RenderContext> {
    fn kind(&self) -> BlockKind {
        BlockKind::Input
    }
    fn submission_mut(&mut self) -> Option<&mut SubmissionBlock> {
        None
    }
    fn outputs(&self) -> &[Box<dyn Block>] {
        &[]
    }
    fn value_mut(&mut self) -> Option<&mut OutputBlock> {
        None
    }
    fn selection_exclusions(
        &self,
        _context: &RenderContext,
        _area: Rect,
        _exclusions: &mut Vec<Rect>,
    ) {
    }
}

/// How a successful input cell's result appears in the transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellState {
    Value,
    Muted,
    Null,
}

/// One lexical Macaulay2 input cell and the byte ranges highlighted within it.
pub struct Cell {
    text: String,
    spans: Vec<HLSpan>,
}

impl Cell {
    fn new(text: String, spans: Vec<HLSpan>) -> Self {
        Self { text, spans }
    }

    /// Create a numbered input cell from source and spans produced for that source.
    #[cfg(test)]
    fn input(text: String, spans: Vec<HLSpan>) -> Self {
        Self::new(text, spans)
    }

    /// Create a numbered input cell with its result state.
    pub fn resolved(text: String, spans: Vec<HLSpan>) -> Self {
        Self::new(text, spans)
    }

    /// Create an unnumbered cell that was not submitted to Macaulay2.
    fn rejected(text: String, spans: Vec<HLSpan>) -> Self {
        Self::new(text, spans)
    }

    /// Put this cell and another cell into one code block.
    #[cfg(test)]
    fn join(self, other: Cell) -> CodeBlock {
        CodeBlock::new([self, other])
    }

    fn wrapped(&self, width: u16) -> WrappedText {
        let spans = self
            .spans
            .iter()
            .map(|span| span.content.clone())
            .collect::<Vec<_>>();
        WrappedText::code(&self.text, width, &spans)
    }
}

/// One feed block containing one or more lexical input cells.
pub struct CodeBlock {
    cells: Vec<Cell>,
}

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

fn tree_children(node: &PrettyNode) -> Vec<TreeEntry<'_>> {
    let mut entries = Vec::new();
    push_tree_children(node, String::new(), &mut entries);
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
            code: CodeBlock::new([Cell::resolved(text, spans)]),
            tree,
        }
    }

    fn tree_lines(&self, context: &RenderContext) -> Vec<Line<'static>> {
        fn line(
            node: &PrettyNode,
            label: Option<&str>,
            prefix: &str,
            branch: &str,
            delimiter: bool,
            context: &RenderContext,
        ) -> Line<'static> {
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
                    Span::styled("Token![", groups.preview_mode()),
                    Span::styled(token.to_string(), groups.tree_code()),
                    Span::styled("]", groups.preview_mode()),
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
            let location = node.span().map(|span| format!("  {span}"));
            if !delimiter && node.role() != PrettyNodeRole::Token {
                let occupied = Line::from(spans.clone()).width()
                    + location.as_ref().map_or(0, |text| text.chars().count())
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
            if let Some(location) = location {
                spans.push(Span::styled(location, groups.muted()));
            }
            Line::from(spans)
        }

        fn walk_entry(
            entry: &TreeEntry<'_>,
            prefix: String,
            last: bool,
            context: &RenderContext,
            lines: &mut Vec<Line<'static>>,
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
                let children: Vec<_> = entry
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
        lines.push(Line::default());
        lines
    }

    fn lines(&self, context: &RenderContext) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
        let mut source = self.code.preview_lines(context);
        let mut tree = self.tree_lines(context);
        let width = source
            .iter()
            .chain(&tree)
            .map(Line::width)
            .max()
            .unwrap_or_default();
        let padding = usize::from(context.width()).saturating_sub(width) / 2;
        for line in source.iter_mut().chain(&mut tree) {
            if !line.spans.is_empty() {
                line.spans.insert(0, Span::raw(" ".repeat(padding)));
            }
        }
        (source, tree)
    }
}

impl Element for SyntaxTreeBlock {
    type Context = RenderContext;

    fn height(&self, context: &RenderContext) -> u16 {
        let (source, tree) = self.lines(context);
        1 + source.len() as u16 + tree.len() as u16
    }

    fn render(&self, context: &RenderContext, area: Rect, buf: &mut Buffer) {
        let groups = context.groups();
        let (source, tree) = self.lines(context);
        let separator = Line::from(Span::styled("─".repeat(area.width.into()), groups.border()));
        Paragraph::new(vec![separator])
            .style(groups.code())
            .render(Rect::new(area.x, area.y, area.width, 1), buf);
        let source_height = source.len() as u16;
        Paragraph::new(source).style(groups.transcript()).render(
            Rect::new(area.x, area.y + 1, area.width, source_height),
            buf,
        );
        Paragraph::new(tree).style(groups.code()).render(
            Rect::new(
                area.x,
                area.y + 1 + source_height,
                area.width,
                area.height.saturating_sub(1 + source_height),
            ),
            buf,
        );
    }
}

impl Block for SyntaxTreeBlock {}

impl CodeBlock {
    /// Create a code block from lexical cells in display order.
    pub fn new(cells: impl IntoIterator<Item = Cell>) -> Self {
        Self {
            cells: cells.into_iter().collect(),
        }
    }

    fn body_height(&self, context: &RenderContext) -> u16 {
        self.cells
            .iter()
            .map(|cell| cell.wrapped(context.width()).height())
            .sum()
    }

    fn lines(&self, context: &RenderContext, style: Style) -> Vec<Line<'static>> {
        let mut rendered = vec![Line::default()];
        for cell in &self.cells {
            rendered.extend(cell_lines(cell, context, style));
        }
        rendered.push(Line::default());
        rendered
    }

    fn preview_lines(&self, context: &RenderContext) -> Vec<Line<'static>> {
        let context = RenderContext::new(context.width(), 0, context.groups());
        let mut rendered = Vec::new();
        for cell in &self.cells {
            rendered.extend(cell_lines(cell, &context, context.groups().transcript()));
        }
        rendered.push(Line::default());
        rendered
    }
}

impl Element for CodeBlock {
    type Context = RenderContext;

    fn height(&self, context: &RenderContext) -> u16 {
        self.body_height(context) + 2
    }

    fn render(&self, context: &RenderContext, area: Rect, buf: &mut Buffer) {
        let style = context.groups().code();
        Paragraph::new(self.lines(context, style))
            .style(style)
            .render(area, buf);
    }
}

impl Block for CodeBlock {
    fn kind(&self) -> BlockKind {
        BlockKind::Input
    }
}

fn cell_lines(cell: &Cell, context: &RenderContext, style: Style) -> Vec<Line<'static>> {
    let spans = cell.spans.paint(0, &cell.text, style, context.groups());
    cell.wrapped(context.width())
        .source_lines()
        .map(|(range, _)| Line::from(spans.slice(range)))
        .collect()
}

/// Evaluation progress for one submitted input cell.
enum SubmissionState {
    Pending,
    Queued,
    Succeeded,
    Failed,
    Rejected,
}

/// Submitted cells that resolve in place while their M2 evaluation runs.
pub struct SubmissionBlock {
    id: u64,
    cells: Vec<(Cell, SubmissionState)>,
    outputs: Vec<Box<dyn Block>>,
}

impl SubmissionBlock {
    pub fn new(id: u64, cells: impl IntoIterator<Item = (String, Vec<HLSpan>)>) -> Self {
        Self {
            id,
            cells: cells
                .into_iter()
                .enumerate()
                .map(|(index, (text, spans))| {
                    (
                        Cell::rejected(text, spans),
                        if index == 0 {
                            SubmissionState::Pending
                        } else {
                            SubmissionState::Queued
                        },
                    )
                })
                .collect(),
            outputs: Vec::new(),
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn succeed(&mut self, index: usize, outputs: Vec<Box<dyn Block>>) {
        if let Some((_, state)) = self.cells.get_mut(index) {
            *state = SubmissionState::Succeeded;
        }
        if let Some((_, state)) = self.cells.get_mut(index + 1) {
            *state = SubmissionState::Pending;
        }
        for mut output in outputs {
            if let Some((cells, next)) = self
                .outputs
                .last_mut()
                .and_then(|block| block.value_mut())
                .zip(output.value_mut())
            {
                cells.cells.append(&mut next.cells);
            } else {
                self.outputs.push(output);
            }
        }
    }

    pub fn fail(&mut self, index: usize, message: String) {
        if let Some((_, state)) = self.cells.get_mut(index) {
            *state = SubmissionState::Failed;
        }
        for (_, state) in self.cells.iter_mut().skip(index + 1) {
            *state = SubmissionState::Rejected;
        }
        self.outputs.push(Box::new(ErrorMessageBlock { message }));
    }

    fn code_height(&self, context: &RenderContext) -> u16 {
        self.cells
            .iter()
            .map(|(cell, _)| cell.wrapped(context.width()).height())
            .sum::<u16>()
            + 2
    }

    fn code_lines(&self, context: &RenderContext) -> Vec<Line<'static>> {
        let mut lines = vec![Line::default()];
        for (cell, state) in &self.cells {
            let style = if matches!(state, SubmissionState::Failed) {
                context.groups().error().patch(context.groups().code())
            } else {
                context.groups().code()
            };
            lines.extend(cell_lines(cell, context, style));
        }
        lines.push(Line::default());
        lines
    }
}

impl Element for SubmissionBlock {
    type Context = RenderContext;

    fn height(&self, context: &RenderContext) -> u16 {
        self.code_height(context)
            + self
                .outputs
                .iter()
                .map(|block| block.height(context))
                .sum::<u16>()
    }

    fn render(&self, context: &RenderContext, area: Rect, buf: &mut Buffer) {
        let code_height = self.code_height(context);
        Paragraph::new(self.code_lines(context))
            .style(context.groups().code())
            .render(Rect::new(area.x, area.y, area.width, code_height), buf);
        let mut y = area.y + code_height;
        for block in &self.outputs {
            let height = block.height(context);
            block.render(context, Rect::new(area.x, y, area.width, height), buf);
            y += height;
        }
    }
}

impl Block for SubmissionBlock {
    fn kind(&self) -> BlockKind {
        BlockKind::Input
    }

    fn submission_mut(&mut self) -> Option<&mut SubmissionBlock> {
        Some(self)
    }

    fn outputs(&self) -> &[Box<dyn Block>] {
        &self.outputs
    }

    fn selection_exclusions(
        &self,
        context: &RenderContext,
        area: Rect,
        exclusions: &mut Vec<Rect>,
    ) {
        let mut y = area.y + self.code_height(context);
        for block in &self.outputs {
            let height = block.height(context);
            block.selection_exclusions(
                context,
                Rect::new(area.x, y, area.width, height),
                exclusions,
            );
            y += height;
        }
    }
}

/// Unnumbered text written by Spectral or by M2 during evaluation.
pub struct TextBlock {
    text: String,
    kind: TextKind,
}

/// Origin and presentation of an unnumbered text block.
enum TextKind {
    System,
    Printed,
}

impl TextBlock {
    fn new(text: impl Into<String>, kind: TextKind) -> Self {
        Self {
            text: text.into(),
            kind,
        }
    }

    /// Create a Spectral status or help message.
    pub fn system(text: impl Into<String>) -> Self {
        Self::new(text, TextKind::System)
    }

    /// Create text printed by evaluated M2 code.
    pub fn printed(text: impl Into<String>) -> Self {
        Self::new(text, TextKind::Printed)
    }

    fn body(&self, context: &RenderContext) -> Vec<String> {
        let mut lines = word_lines(&self.text, context.width());
        if lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines
    }

    /// Blank rows drawn above the body. M2's printed output is framed like a code
    /// band, with a row of air on each side; Spectral's own messages keep their
    /// tighter footer-only spacing so the banner sits flush at the top.
    fn top_margin(&self) -> usize {
        match self.kind {
            TextKind::System => 0,
            TextKind::Printed => 1,
        }
    }
}

impl Element for TextBlock {
    type Context = RenderContext;

    fn height(&self, context: &RenderContext) -> u16 {
        (self.body(context).len() + self.top_margin()) as u16 + 1
    }

    fn render(&self, context: &RenderContext, area: Rect, buf: &mut Buffer) {
        let theme = context.groups();
        let base = theme.transcript();
        let text = match self.kind {
            TextKind::System => theme.muted().patch(base),
            TextKind::Printed => theme.printed().patch(base),
        };
        let mut lines: Vec<Line<'static>> = vec![Line::default(); self.top_margin()];
        lines.extend(
            self.body(context)
                .into_iter()
                .map(|line| Line::from(Span::styled(line, text))),
        );
        lines.push(Line::default());
        Paragraph::new(lines).style(base).render(area, buf);
    }
}

impl Block for TextBlock {
    fn kind(&self) -> BlockKind {
        match self.kind {
            TextKind::System => BlockKind::System,
            TextKind::Printed => BlockKind::Printed,
        }
    }
}

/// One result inside a possibly shared value border.
struct OutputCell {
    text: Option<String>,
    ty: Option<String>,
}

/// Consecutive values returned by M2 inside one border.
pub struct OutputBlock {
    cells: Vec<OutputCell>,
}

/// Placement of a result's type inside its bordered body.
#[derive(Clone, Copy)]
enum TypePlacement {
    None,
    Inline,
    Newline,
}

impl OutputBlock {
    /// Create a rendered value and its optional type.
    pub fn value(text: String, ty: Option<String>) -> Self {
        Self {
            cells: vec![OutputCell {
                text: Some(text),
                ty,
            }],
        }
    }

    /// Create the type-only result of a muted but bound value.
    pub fn muted(ty: String) -> Self {
        Self {
            cells: vec![OutputCell {
                text: None,
                ty: Some(ty),
            }],
        }
    }

    fn text_width(&self, context: &RenderContext) -> u16 {
        Bordered::text_width(context)
    }

    fn layout(&self, cell: &OutputCell, context: &RenderContext) -> (Vec<String>, TypePlacement) {
        let value = match cell.text.as_deref() {
            Some("") | None => Vec::new(),
            Some(text) => word_lines(text, self.text_width(context)),
        };
        let placement = match (&cell.ty, value.as_slice()) {
            (None, _) => TypePlacement::None,
            (Some(ty), [line])
                if line.chars().count() + ty.chars().count()
                    < usize::from(self.text_width(context)) =>
            {
                TypePlacement::Inline
            }
            (Some(_), _) => TypePlacement::Newline,
        };
        (value, placement)
    }

    fn lines(&self, cell: &OutputCell, context: &RenderContext) -> Vec<Line<'static>> {
        let theme = context.groups();
        let width = self.text_width(context);
        let (value, placement) = self.layout(cell, context);
        let mut lines: Vec<_> = value
            .into_iter()
            .map(|line| Line::from(Span::styled(line, theme.value())))
            .collect();
        match (placement, &cell.ty) {
            (TypePlacement::Inline, Some(ty)) => {
                let value = lines[0].spans[0].content.to_string();
                let padding =
                    usize::from(width).saturating_sub(value.chars().count() + ty.chars().count());
                lines[0] = Line::from(vec![
                    Span::styled(value, theme.value()),
                    Span::raw(" ".repeat(padding)),
                    Span::styled(ty.clone(), theme.type_info()),
                ]);
            }
            (TypePlacement::Newline, Some(ty)) => {
                let padding = usize::from(width).saturating_sub(ty.chars().count());
                lines.push(Line::from(vec![
                    Span::raw(" ".repeat(padding)),
                    Span::styled(ty.clone(), theme.type_info()),
                ]));
            }
            _ => {}
        }
        lines
    }
}

impl Element for OutputBlock {
    type Context = RenderContext;

    fn height(&self, context: &RenderContext) -> u16 {
        self.cells
            .iter()
            .map(|cell| self.lines(cell, context).len() as u16)
            .sum::<u16>()
            + 2
    }

    fn render(&self, context: &RenderContext, area: Rect, buf: &mut Buffer) {
        let theme = context.groups();
        let mut body = Vec::new();
        for cell in &self.cells {
            body.extend(self.lines(cell, context));
        }
        Bordered::new(Style::new(), theme.border(), None, None, body).render(&(), area, buf);
    }
}

impl Block for OutputBlock {
    fn selection_exclusions(
        &self,
        context: &RenderContext,
        area: Rect,
        exclusions: &mut Vec<Rect>,
    ) {
        let right = Bordered::inner(area).right();
        let mut row = area.y + 1;
        for cell in &self.cells {
            let (value, placement) = self.layout(cell, context);
            if let Some(ty) = &cell.ty {
                let ty_row = match placement {
                    TypePlacement::Inline => Some(row),
                    TypePlacement::Newline => Some(row + value.len() as u16),
                    TypePlacement::None => None,
                };
                if let Some(ty_row) = ty_row {
                    let width = ty.chars().count() as u16;
                    exclusions.push(Rect::new(right.saturating_sub(width), ty_row, width, 1));
                }
            }
            row += value.len() as u16 + u16::from(matches!(placement, TypePlacement::Newline));
        }
    }

    fn kind(&self) -> BlockKind {
        BlockKind::Value
    }

    fn value_mut(&mut self) -> Option<&mut OutputBlock> {
        Some(self)
    }
}

/// A warning emitted while evaluating M2 code.
pub struct WarningBlock {
    pub text: String,
}

impl Element for WarningBlock {
    type Context = RenderContext;

    fn height(&self, context: &RenderContext) -> u16 {
        word_lines(&self.text, Bordered::text_width(context)).len() as u16 + 2
    }

    fn render(&self, context: &RenderContext, area: Rect, buf: &mut Buffer) {
        let theme = context.groups();
        let body = word_lines(&self.text, Bordered::text_width(context))
            .into_iter()
            .map(|line| Line::from(Span::styled(line, theme.warning_bg())))
            .collect();
        let title = Some(Line::from(Span::styled(" Warning ", theme.warning_bg())));
        Bordered::new(
            theme.warning_bg(),
            theme.warning_border(),
            title,
            None,
            body,
        )
        .render(&(), area, buf);
    }
}

impl Block for WarningBlock {
    fn kind(&self) -> BlockKind {
        BlockKind::Warning
    }
}





fn word_lines(text: &str, width: u16) -> Vec<String> {
    WrappedText::word(text, width)
        .text()
        .split('\n')
        .map(str::to_string)
        .collect()
}

fn error_box_text(message: &str, context: &RenderContext) -> Vec<String> {
    let message = message
        .lines()
        .map(strip_m2_location)
        .collect::<Vec<_>>()
        .join("\n");
    word_lines(&message, Bordered::text_width(context))
}

fn render_error_box(message: &str, context: &RenderContext, area: Rect, buf: &mut Buffer) {
    let theme = context.groups();
    let body = error_box_text(message, context)
        .into_iter()
        .map(|line| Line::from(Span::styled(line, theme.error_bg())))
        .collect();
    Bordered::new(theme.error_bg(), theme.error_border(), None, None, body).render(&(), area, buf);
}

/// An evaluation error whose submitted code is already present in the feed.
pub struct ErrorMessageBlock {
    pub message: String,
}

impl ErrorMessageBlock {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl Element for ErrorMessageBlock {
    type Context = RenderContext;

    fn height(&self, context: &RenderContext) -> u16 {
        error_box_text(&self.message, context).len() as u16 + 2
    }

    fn render(&self, context: &RenderContext, area: Rect, buf: &mut Buffer) {
        render_error_box(&self.message, context, area, buf);
    }
}

impl Block for ErrorMessageBlock {
    fn kind(&self) -> BlockKind {
        BlockKind::Error
    }
}




#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::highlight::Highlighter;
    use crate::ui::controller::settings::theme::groups::Groups;
    use m2_syn::{DelimiterKind, DoubleSpan, Span as M2Span};

    fn rendered_buffer(block: &dyn Block, width: u16, groups: &Groups) -> Buffer {
        let context = RenderContext::new(width, 0, groups);
        let area = Rect::new(0, 0, width, block.height(&context));
        let mut buffer = Buffer::empty(area);
        block.render(&context, area, &mut buffer);
        buffer
    }

    fn rendered_rows(block: &dyn Block, width: u16, groups: &Groups) -> Vec<String> {
        let buffer = rendered_buffer(block, width, groups);
        let area = *buffer.area();
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    fn syntax_tree_fixture() -> SyntaxTreeBlock {
        let span = M2Span::for_text("(x,y)");
        let mut elements = PrettyNode::collection("Punctuated · 2 items", Some(span.subspan(1, 4)));
        elements.push_node("[0]", PrettyNode::token("Symbol", "x", span.subspan(1, 2)));
        elements.push_node(
            "punctuation",
            PrettyNode::token("Token![,]", ",", span.subspan(2, 3)),
        );
        elements.push_node("[1]", PrettyNode::token("Symbol", "y", span.subspan(3, 4)));
        let mut delimited = PrettyNode::collection("Delimited", Some(span.clone()));
        delimited.push_node("contents", elements);
        delimited.push_node(
            "delimiter",
            PrettyNode::delimiter(
                DelimiterKind::Parenthesis,
                DoubleSpan::new(span.subspan(0, 1), span.subspan(4, 5)),
            ),
        );
        let mut root = PrettyNode::syntax("Root", Some(span));
        root.push_node("parameters", delimited);
        root.push_node("semicolon", PrettyNode::absent());
        root.push_node(
            "comments",
            PrettyNode::collection("Comments · 0 items", None),
        );
        SyntaxTreeBlock::new("(x,y)".to_string(), Vec::new(), root)
    }

    #[test]
    fn syntax_tree_labels_structures_after_types_and_encloses_delimited_content() {
        let groups = Groups::default();
        let context = RenderContext::new(60, 0, &groups);
        let lines = syntax_tree_fixture().tree_lines(&context);
        let rows: Vec<String> = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect();
        let screen = rows.join("\n");

        assert!(rows[1].contains("(  parameters:"), "{screen}");
        assert!(rows[2].contains("  ├─Symbol x"), "{screen}");
        assert!(rows[3].contains("  ├─Token![,]"), "{screen}");
        assert!(rows[4].contains("  ├─Symbol y"), "{screen}");
        assert!(rows[5].contains("  ╰─)"), "{screen}");
        assert_eq!(rows[3].matches(',').count(), 1, "{screen}");
        for hidden in [
            "Delimited",
            "Punctuated",
            "None",
            "Comments",
            "opening",
            "closing",
            "semicolon",
            "[0]",
            "[1]",
            "punctuation",
            "\"x\"",
        ] {
            assert!(!screen.contains(hidden), "{hidden} leaked into {screen}");
        }

        let style = |text| {
            lines
                .iter()
                .flat_map(|line| &line.spans)
                .find(|span| span.content == text)
                .unwrap()
                .style
        };
        assert_eq!(style("Root"), groups.style(HLGroup::Type));
        assert_eq!(style("parameters"), groups.style(HLGroup::Property));
        assert_eq!(style("Symbol"), groups.style(HLGroup::Type));
        assert_eq!(style("Token!["), groups.preview_mode());
        assert_eq!(style(","), groups.tree_code());
        assert_eq!(style("]"), groups.preview_mode());
        assert_eq!(style(" x"), groups.tree_code());
        assert_eq!(style(" (x,y)"), groups.tree_code());
    }

    #[test]
    fn syntax_tree_encloses_a_real_method_parameter_list() {
        let source = "ZZ ZZ := ZZ => (x, y) -> x y";
        let crate::code::parser::ParseVerdict::Complete(mut cells) =
            crate::code::parser::check(source)
        else {
            panic!("example did not parse");
        };
        let tree = cells.remove(0).tree;
        let groups = Groups::default();
        let context = RenderContext::new(100, 0, &groups);
        let block = SyntaxTreeBlock::new(source.to_string(), Vec::new(), tree);
        let screen = block
            .tree_lines(&context)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(screen.contains("ExprPack  content:"), "{screen}");
        assert!(screen.contains("FixedArity  parameters:"), "{screen}");
        assert!(screen.contains("╰─("), "{screen}");
        assert!(screen.contains("╰─)"), "{screen}");
        assert!(!screen.contains("Delimited"), "{screen}");
        assert!(!screen.contains("method:"), "{screen}");
        assert!(!screen.contains("0[1]"), "{screen}");
        assert!(!screen.contains("  0:"), "{screen}");
        let assignment = screen
            .lines()
            .find(|line| line.contains("Token![:=]"))
            .unwrap();
        assert_eq!(assignment.matches(":=").count(), 1, "{screen}");
    }

    #[test]
    fn syntax_tree_centers_its_content_and_inverts_the_source_band() {
        let groups = Groups::default();
        let width = 60;
        let buffer = rendered_buffer(&syntax_tree_fixture(), width, &groups);
        let rows: Vec<_> = (0..buffer.area().height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect();
        let occupied: Vec<_> = rows[1..]
            .iter()
            .filter_map(|row| {
                let cells: Vec<_> = row.chars().collect();
                Some((
                    cells.iter().position(|cell| *cell != ' ')?,
                    cells.iter().rposition(|cell| *cell != ' ')?,
                ))
            })
            .collect();
        let left = occupied.iter().map(|(left, _)| left).min().unwrap();
        let right = occupied.iter().map(|(_, right)| right).max().unwrap();

        assert!(rows[0].chars().all(|cell| cell == '─'));
        assert!(left.abs_diff(width as usize - right - 1) <= 1, "{rows:#?}");
        assert_eq!(buffer[(0, 0)].bg, groups.code().bg.unwrap());
        assert_eq!(buffer[(0, 1)].bg, groups.transcript().bg.unwrap());
        assert_eq!(buffer[(0, 3)].bg, groups.code().bg.unwrap());
    }

    #[test]
    fn syntax_tree_shows_virtual_token_types_without_fake_source_text() {
        let groups = Groups::default();
        let context = RenderContext::new(40, 0, &groups);
        let span = M2Span::for_text("xy");
        let mut tree = PrettyNode::syntax("Root", Some(span.clone()));
        tree.push_node("operator", PrettyNode::token("Adj", "Adj", span.at(1)));
        let block = SyntaxTreeBlock::new("xy".to_string(), Vec::new(), tree);
        let screen: String = block
            .tree_lines(&context)
            .iter()
            .flat_map(|line| &line.spans)
            .map(|span| span.content.as_ref())
            .collect();

        assert!(screen.contains("Adj"), "{screen}");
        assert_eq!(screen.matches("Adj").count(), 1, "{screen}");
        assert!(!screen.contains("operator"), "{screen}");
    }

    #[test]
    fn syntax_tree_compacts_long_nonterminal_source_to_the_row() {
        let groups = Groups::default();
        let context = RenderContext::new(40, 0, &groups);
        let source = (0..20).map(|_| "long").collect::<Vec<_>>().join(" \n");
        let tree = PrettyNode::syntax("Root", Some(M2Span::for_text(source.clone())));
        let block = SyntaxTreeBlock::new(source, Vec::new(), tree);
        let line = &block.tree_lines(&context)[0];

        assert!(line.width() <= usize::from(context.width()));
        assert!(line.spans.iter().any(|span| span.content.contains('…')));
    }

    #[test]
    fn code_uses_the_full_width_and_outputs_share_the_fixed_inset() {
        let theme = Groups::default();
        let width = 40;
        let input = CodeBlock::new([Cell::input("x".to_string(), Vec::new())]);
        let output = OutputBlock::value("x".to_string(), None);

        let input = rendered_rows(&input, width, &theme);
        let output = rendered_rows(&output, width, &theme);
        assert_eq!(input[1].find('x'), Some(0));
        assert_eq!(output[0].chars().nth(1), Some('╭'));
    }

    #[test]
    fn hard_wrapped_code_has_no_prefix() {
        let theme = Groups::default();
        let input = CodeBlock::new([Cell::input("abcdefgh".to_string(), Vec::new())]);

        let rows = rendered_rows(&input, 4, &theme);
        assert_eq!(rows[1].find("abcd"), Some(0));
        assert_eq!(rows[2].find("efgh"), Some(0));
    }

    #[test]
    fn wrapping_splits_already_styled_spans() {
        let cell = Cell::input(
            "abcdefgh".to_string(),
            vec![HLSpan::new(0..8, HLGroup::Variable)],
        );
        let wrapped = cell.wrapped(3);
        let styled = cell
            .spans
            .paint(0, &cell.text, Style::new(), &Groups::default());
        let rows: Vec<_> = wrapped
            .source_lines()
            .map(|(range, _)| styled.slice(range)[0].content.to_string())
            .collect();

        assert_eq!(wrapped.text(), "abc\ndef\ngh");
        assert_eq!(wrapped.text().matches('\n').count(), 2);
        assert_eq!(rows, ["abc", "def", "gh"]);
        assert_eq!(
            wrapped
                .source_lines()
                .filter_map(|(range, hard)| hard.then_some(range.start))
                .collect::<Vec<_>>(),
            [3, 6]
        );
    }

    #[test]
    fn joining_cells_preserves_their_owned_spans() {
        struct WholeCellHighlighter;

        impl Highlighter for WholeCellHighlighter {
            fn highlight(&mut self, source: &str) -> Vec<HLSpan> {
                vec![HLSpan::new(0..source.len(), HLGroup::Variable)]
            }
        }

        let mut highlighter = WholeCellHighlighter;
        let code = Cell::input("x".to_string(), highlighter.highlight("x"))
            .join(Cell::input("y".to_string(), highlighter.highlight("y")));

        assert_eq!(code.cells[0].text, "x");
        assert_eq!(code.cells[0].spans[0].content, 0..1);
        assert_eq!(code.cells[1].text, "y");
        assert_eq!(code.cells[1].spans[0].content, 0..1);
        let rows = rendered_rows(&code, 20, &Groups::default());
        assert_eq!(rows[1].trim(), "x");
        assert_eq!(rows[2].trim(), "y");
    }

    #[test]
    fn submitted_cells_advance_through_the_queue_in_place() {
        let mut submission = SubmissionBlock::new(
            7,
            [
                ("a".to_string(), Vec::new()),
                ("b".to_string(), Vec::new()),
                ("c".to_string(), Vec::new()),
            ],
        );
        submission.succeed(0, Vec::new());
        assert!(matches!(submission.cells[0].1, SubmissionState::Succeeded));
        assert!(matches!(submission.cells[1].1, SubmissionState::Pending));
        submission.fail(1, "bad cell".to_string());
        assert!(matches!(submission.cells[1].1, SubmissionState::Failed));
        assert!(matches!(submission.cells[2].1, SubmissionState::Rejected));
    }

    #[test]
    fn result_states_merge() {
        let mut submission = SubmissionBlock::new(
            1,
            ["a", "b", "c"].map(|text| (text.to_string(), Vec::new())),
        );
        submission.succeed(0, vec![Box::new(OutputBlock::value("a".to_string(), None))]);
        submission.succeed(1, vec![Box::new(OutputBlock::muted("ZZ".to_string()))]);
        submission.succeed(
            2,
            vec![Box::new(OutputBlock::value(
                "null".to_string(),
                Some("Nothing".to_string()),
            ))],
        );

        assert_eq!(submission.outputs.len(), 1);
        assert_eq!(submission.outputs[0].value_mut().unwrap().cells.len(), 3);
    }

    #[test]
    fn consecutive_values_share_one_border() {
        let mut submission =
            SubmissionBlock::new(1, ["a", "b"].map(|text| (text.to_string(), Vec::new())));
        for (index, number) in [1, 2].into_iter().enumerate() {
            submission.succeed(
                index,
                vec![Box::new(OutputBlock::value(number.to_string(), None))],
            );
        }

        let rows = rendered_rows(&submission, 40, &Groups::default());
        assert_eq!(submission.outputs.len(), 1);
        assert!(rows[5].contains('1'), "{rows:?}");
        assert!(rows[6].contains('2'), "{rows:?}");
        assert_eq!(rows.iter().filter(|row| row.contains('╭')).count(), 1);
        assert_eq!(rows.iter().filter(|row| row.contains('╰')).count(), 1);
    }

    #[test]
    fn one_line_output_keeps_its_value_and_type_inside_the_border() {
        let theme = Groups::default();
        let width = 40;
        let block = OutputBlock::value("x + y".to_string(), Some("R".to_string()));

        let rows = rendered_rows(&block, width, &theme);
        assert_eq!(rows.len(), 3);
        assert!(rows[1].contains("x + y") && rows[1].contains('R'));
        let cells: Vec<_> = rows[1].chars().collect();
        let borders: Vec<_> = cells
            .iter()
            .enumerate()
            .filter_map(|(index, character)| (*character == '│').then_some(index))
            .collect();
        let value = cells
            .iter()
            .position(|character| *character == 'x')
            .unwrap();
        let ty = cells
            .iter()
            .position(|character| *character == 'R')
            .unwrap();
        assert_eq!(value, 4);
        assert_eq!(borders[1] - ty, 2);
    }

    #[test]
    fn output_has_a_rounded_inset_border() {
        let theme = Groups::default();
        let block = OutputBlock::value("abc".to_string(), None);

        let rows = rendered_rows(&block, 20, &theme);
        let input = rendered_rows(
            &CodeBlock::new([Cell::input("x".to_string(), Vec::new())]),
            20,
            &theme,
        );
        assert_eq!(rows.len(), 3);
        assert!(rows[0].contains('╭') && rows[0].contains('╮'));
        assert_eq!(rows[0].chars().nth(1), Some('╭'));
        assert_eq!(input[1].chars().next(), Some('x'));
        assert!(rows[2].contains('╰') && rows[2].contains('╯'));
    }

    #[test]
    fn every_box_shares_one_fixed_boundary() {
        let theme = Groups::default();
        let context = RenderContext::new(20, 0, &theme);
        let boxes: [Box<dyn Block>; 3] = [
            Box::new(OutputBlock::value("x".into(), None)),
            Box::new(WarningBlock { text: "x".into() }),
            Box::new(ErrorMessageBlock::new("x".into())),
        ];

        for block in boxes {
            let buffer = rendered_buffer(block.as_ref(), 20, &theme);
            assert_eq!(buffer[(1, 0)].symbol(), "╭");
            assert_eq!(buffer[(1, block.height(&context) - 1)].symbol(), "╰");
        }
    }

    #[test]
    fn diagnostic_boxes_use_their_tinted_surfaces() {
        let theme = Groups::default();
        for (block, surface) in [
            (
                Box::new(WarningBlock {
                    text: "warning".into(),
                }) as Box<dyn Block>,
                theme.warning_bg(),
            ),
            (
                Box::new(ErrorMessageBlock::new("error".into())) as Box<dyn Block>,
                theme.error_bg(),
            ),
        ] {
            let buffer = rendered_buffer(block.as_ref(), 20, &theme);
            let cell = &buffer[(5, 1)];
            assert_eq!(
                (cell.fg, cell.bg),
                (surface.fg.unwrap(), surface.bg.unwrap())
            );
        }
    }

    #[test]
    fn multiline_output_keeps_the_type_on_its_own_last_row() {
        let theme = Groups::default();
        let block = OutputBlock::value("x\ny".to_string(), Some("R".to_string()));

        let rows = rendered_rows(&block, 40, &theme);
        assert_eq!(rows.len(), 5);
        assert!(rows[1].contains('x'));
        assert!(rows[2].contains('y'));
        assert!(rows[3].contains('R'));
    }

    #[test]
    fn a_result_shows_its_type() {
        let theme = Groups::default();
        let block = OutputBlock::value("null".to_string(), Some("Nothing".to_string()));

        let rows = rendered_rows(&block, 40, &theme);
        assert!(rows[1].contains("null"));
        assert!(rows[1].contains("Nothing"));
    }

    #[test]
    fn a_silent_binding_shows_only_its_type() {
        let theme = Groups::default();
        let block = OutputBlock::muted("ZZ".to_string());

        let rows = rendered_rows(&block, 40, &theme);
        assert!(rows[1].contains("ZZ"), "unexpected rows: {rows:?}");
    }

    #[test]
    fn word_wrap_preserves_empty_lines_and_m2_identifiers() {
        let wrapped = WrappedText::word("x' y$z\n\nabcdefghij", 4);
        let lines: Vec<_> = wrapped.text().split('\n').collect();
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0], "x'");
        assert_eq!(lines[1], "y$z");
        assert!(lines.contains(&""));
    }

    #[test]
    fn system_block_height_includes_each_message_line_and_margin() {
        let theme = Groups::default();
        let context = RenderContext::new(80, 0, &theme);
        let block = TextBlock::system("commands:\n  /settings\n  /code");
        assert_eq!(block.height(&context), 4);
    }
}
