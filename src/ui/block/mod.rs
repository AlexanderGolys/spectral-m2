//! Typed entries in the scrollable editor feed.
//!
//! Every [`Block`] owns text, measures and renders against a borrowed
//! [`RenderContext`], and inherits the same visual-line wrapper. Highlighted text
//! receives its highlighter separately because highlighting is an application-wide
//! service rather than editor state.

use m2_syn::{PrettyDetailRole, PrettyNode, PrettyNodeRole};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use self::layout::WrappedText;
use self::render::{
    apply_spans, band, border_label, box_inner_area, box_text_width, boxed, gutter_span,
    gutter_width, BoxLayout, CellMarker, CodeBlockStyle, RenderContext,
};
use crate::backend::strip_m2_location;
use crate::code::highlight::{HLSpan, MacaulayHighlight};

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
pub trait Block: Send {
    fn height(&self, context: &RenderContext<'_>) -> u16;
    fn render(&self, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer);
    fn kind(&self) -> BlockKind {
        BlockKind::Input
    }
    fn submission_mut(&mut self) -> Option<&mut SubmissionBlock> {
        None
    }
    fn outputs(&self) -> &[Box<dyn Block>] {
        &[]
    }
    fn selection_exclusions(
        &self,
        _context: &RenderContext<'_>,
        _area: Rect,
        _exclusions: &mut Vec<Rect>,
    ) {
    }
}

/// One lexical Macaulay2 input cell and the byte ranges highlighted within it.
pub struct Cell {
    text: String,
    number: Option<u32>,
    spans: Vec<HLSpan>,
}

impl Cell {
    fn new(text: String, number: Option<u32>, spans: Vec<HLSpan>) -> Self {
        Self {
            text,
            number,
            spans,
        }
    }

    /// Create a numbered input cell from source and spans produced for that source.
    pub fn input(text: String, number: u32, spans: Vec<HLSpan>) -> Self {
        Self::new(text, Some(number), spans)
    }

    /// Create an unnumbered cell that was not submitted to Macaulay2.
    pub fn rejected(text: String, spans: Vec<HLSpan>) -> Self {
        Self::new(text, None, spans)
    }

    /// Put this cell and another cell into one code block.
    #[cfg(test)]
    pub fn join(self, other: Cell) -> CodeBlock {
        CodeBlock::new([self, other])
    }

    fn wrapped(&self, width: u16) -> (WrappedText, Vec<HLSpan>) {
        let token_ends: Vec<_> = self.spans.iter().map(|span| span.range.end).collect();
        let wrapped = WrappedText::code(&self.text, width, &token_ends);
        let spans = self
            .spans
            .iter()
            .flat_map(|span| {
                wrapped
                    .relocate(&span.range)
                    .into_iter()
                    .map(|range| HLSpan {
                        range,
                        kind: span.kind,
                    })
            })
            .collect();
        (wrapped, spans)
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
    pub fn new(text: String, number: u32, spans: Vec<HLSpan>, tree: PrettyNode) -> Self {
        Self {
            code: CodeBlock::new([Cell::input(text, number, spans)]),
            tree,
        }
    }

    fn tree_lines(&self, context: &RenderContext<'_>) -> Vec<Line<'static>> {
        fn line(
            node: &PrettyNode,
            label: Option<&str>,
            prefix: &str,
            branch: &str,
            delimiter: bool,
            context: &RenderContext<'_>,
        ) -> Line<'static> {
            let groups = context.groups();
            let name = groups.macaulay(MacaulayHighlight::Type);
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
                    Span::styled(
                        label.to_string(),
                        groups.macaulay(MacaulayHighlight::Property),
                    ),
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
            context: &RenderContext<'_>,
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

    fn lines(&self, context: &RenderContext<'_>) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
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

impl Block for SyntaxTreeBlock {
    fn height(&self, context: &RenderContext<'_>) -> u16 {
        let (source, tree) = self.lines(context);
        1 + source.len() as u16 + tree.len() as u16
    }

    fn render(&self, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
        let groups = context.groups();
        let (source, tree) = self.lines(context);
        let label = format!(" i{} ", self.code.cells[0].number.unwrap());
        let remaining = usize::from(area.width).saturating_sub(label.chars().count());
        let separator = Line::from(vec![
            Span::styled("─".repeat(remaining / 2), groups.border()),
            Span::styled(label, groups.prompt().bold()),
            Span::styled("─".repeat(remaining.div_ceil(2)), groups.border()),
        ]);
        band(
            Rect::new(area.x, area.y, area.width, 1),
            buf,
            vec![separator],
            groups.code(),
        );
        let source_height = source.len() as u16;
        band(
            Rect::new(area.x, area.y + 1, area.width, source_height),
            buf,
            source,
            groups.transcript(),
        );
        band(
            Rect::new(
                area.x,
                area.y + 1 + source_height,
                area.width,
                area.height.saturating_sub(1 + source_height),
            ),
            buf,
            tree,
            groups.code(),
        );
    }
}

impl CodeBlock {
    /// Create a code block from lexical cells in display order.
    pub fn new(cells: impl IntoIterator<Item = Cell>) -> Self {
        Self {
            cells: cells.into_iter().collect(),
        }
    }

    fn body_height(&self, context: &RenderContext<'_>) -> u16 {
        let width = context.width().saturating_sub(context.gutter());
        self.cells
            .iter()
            .map(|cell| cell.wrapped(width).0.height())
            .sum()
    }

    fn lines(&self, context: &RenderContext<'_>, style: &CodeBlockStyle) -> Vec<Line<'static>> {
        let mut rendered = vec![Line::default()];
        for cell in &self.cells {
            rendered.extend(cell_lines(
                cell,
                cell.number.map_or(CellMarker::Empty, CellMarker::Number),
                context,
                style,
            ));
        }
        rendered.push(Line::default());
        rendered
    }

    fn preview_lines(&self, context: &RenderContext<'_>) -> Vec<Line<'static>> {
        let context = RenderContext::new(context.width(), 0, context.groups());
        let style = CodeBlockStyle::preview(context.groups());
        let mut rendered = Vec::new();
        for cell in &self.cells {
            rendered.extend(cell_lines(cell, CellMarker::Empty, &context, &style));
        }
        rendered.push(Line::default());
        rendered
    }
}

impl Block for CodeBlock {
    fn height(&self, context: &RenderContext<'_>) -> u16 {
        self.body_height(context) + 2
    }

    fn render(&self, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
        let style = CodeBlockStyle::input(context.groups());
        band(area, buf, self.lines(context, &style), style.base());
    }

    fn kind(&self) -> BlockKind {
        BlockKind::Input
    }
}

fn cell_lines(
    cell: &Cell,
    first_marker: CellMarker,
    context: &RenderContext<'_>,
    style: &CodeBlockStyle,
) -> Vec<Line<'static>> {
    let width = context.width().saturating_sub(context.gutter());
    let (wrapped, spans) = cell.wrapped(width);
    let mut hard_continuations = wrapped.hard_continuations().peekable();
    let mut lines = Vec::new();
    let mut offset = 0;
    for (index, text) in wrapped.text().split('\n').enumerate() {
        let marker = match (index, hard_continuations.next_if_eq(&offset).is_some()) {
            (0, _) => first_marker,
            (_, true) => CellMarker::HardContinuation,
            _ => CellMarker::Empty,
        };
        let mut line = style.column(marker, context.gutter());
        line.extend(apply_spans(
            offset,
            text,
            &spans,
            style.base(),
            context.groups(),
        ));
        lines.push(Line::from(line));
        offset += text.len() + 1;
    }
    lines
}

/// Evaluation progress for one submitted input cell.
enum SubmissionState {
    Pending,
    Queued,
    Succeeded(u32),
    Failed(u32),
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

    pub fn succeed(&mut self, index: usize, number: u32, outputs: Vec<Box<dyn Block>>) {
        if let Some((_, state)) = self.cells.get_mut(index) {
            *state = SubmissionState::Succeeded(number);
        }
        if let Some((_, state)) = self.cells.get_mut(index + 1) {
            *state = SubmissionState::Pending;
        }
        self.outputs.extend(outputs);
    }

    pub fn fail(&mut self, index: usize, number: u32, message: String) {
        if let Some((_, state)) = self.cells.get_mut(index) {
            *state = SubmissionState::Failed(number);
        }
        for (_, state) in self.cells.iter_mut().skip(index + 1) {
            *state = SubmissionState::Rejected;
        }
        self.outputs.push(Box::new(ErrorMessageBlock { message }));
    }

    fn code_height(&self, context: &RenderContext<'_>) -> u16 {
        self.cells
            .iter()
            .map(|(cell, _)| {
                cell.wrapped(context.width().saturating_sub(context.gutter()))
                    .0
                    .height()
            })
            .sum::<u16>()
            + 2
    }

    fn code_lines(&self, context: &RenderContext<'_>) -> Vec<Line<'static>> {
        let mut lines = vec![Line::default()];
        for (cell, state) in &self.cells {
            let style = if matches!(state, SubmissionState::Failed(_)) {
                CodeBlockStyle::error(context.groups())
            } else {
                CodeBlockStyle::input(context.groups())
            };
            let marker = match state {
                SubmissionState::Pending => CellMarker::Pending(context.spinner()),
                SubmissionState::Queued => CellMarker::Queued,
                SubmissionState::Succeeded(number) | SubmissionState::Failed(number) => {
                    CellMarker::Number(*number)
                }
                SubmissionState::Rejected => CellMarker::Empty,
            };
            lines.extend(cell_lines(cell, marker, context, &style));
        }
        lines.push(Line::default());
        lines
    }
}

impl Block for SubmissionBlock {
    fn height(&self, context: &RenderContext<'_>) -> u16 {
        self.code_height(context)
            + self
                .outputs
                .iter()
                .map(|block| block.height(context))
                .sum::<u16>()
    }

    fn render(&self, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
        let code_height = self.code_height(context);
        band(
            Rect::new(area.x, area.y, area.width, code_height),
            buf,
            self.code_lines(context),
            context.groups().code(),
        );
        let mut y = area.y + code_height;
        for block in &self.outputs {
            let height = block.height(context);
            block.render(context, Rect::new(area.x, y, area.width, height), buf);
            y += height;
        }
    }

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
        context: &RenderContext<'_>,
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

    fn body(&self, context: &RenderContext<'_>) -> Vec<String> {
        let mut lines = word_lines(&self.text, context.width().saturating_sub(gutter_width(1)));
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

impl Block for TextBlock {
    fn height(&self, context: &RenderContext<'_>) -> u16 {
        (self.body(context).len() + self.top_margin()) as u16 + 1
    }

    fn render(&self, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
        let theme = context.groups();
        let base = theme.transcript();
        let gutter = theme.muted().patch(base);
        let text = match self.kind {
            TextKind::System => gutter,
            TextKind::Printed => theme.printed().patch(base),
        };
        let mut lines: Vec<Line<'static>> = vec![Line::default(); self.top_margin()];
        lines.extend(self.body(context).into_iter().map(|line| {
            Line::from(vec![
                gutter_span("", gutter_width(1), gutter),
                Span::styled(line, text),
            ])
        }));
        lines.push(Line::default());
        band(area, buf, lines, base);
    }

    fn kind(&self) -> BlockKind {
        match self.kind {
            TextKind::System => BlockKind::System,
            TextKind::Printed => BlockKind::Printed,
        }
    }
}

/// A value returned by M2, with optional type information. `number` is the `oN`
/// it was bound to, absent when M2 bound nothing -- a null result is reported
/// like any other, but there is no symbol to name it by.
pub struct OutputBlock {
    pub number: Option<u32>,
    pub text: String,
    pub ty: Option<String>,
}

/// Placement of a result's type inside its bordered body.
#[derive(Clone, Copy)]
enum TypePlacement {
    None,
    Inline,
    Newline,
}

impl OutputBlock {
    fn box_layout(&self, context: &RenderContext<'_>) -> BoxLayout {
        BoxLayout::Output {
            marker_gutter: self.number.map_or_else(|| gutter_width(1), gutter_width),
            content_gutter: context.gutter(),
        }
    }

    fn text_width(&self, context: &RenderContext<'_>) -> u16 {
        box_text_width(context, self.box_layout(context))
    }

    fn layout(&self, context: &RenderContext<'_>) -> (Vec<String>, TypePlacement) {
        let value = if self.text.is_empty() {
            Vec::new()
        } else {
            word_lines(&self.text, self.text_width(context))
        };
        let placement = match (&self.ty, value.as_slice()) {
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
}

impl Block for OutputBlock {
    fn height(&self, context: &RenderContext<'_>) -> u16 {
        let (value, placement) = self.layout(context);
        (value.len() + usize::from(matches!(placement, TypePlacement::Newline))) as u16 + 2
    }

    fn render(&self, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
        let theme = context.groups();
        let (value, placement) = self.layout(context);
        let width = self.text_width(context);
        let mut body: Vec<Line<'static>> = value
            .into_iter()
            .map(|line| Line::from(Span::styled(line, theme.value())))
            .collect();
        match (placement, &self.ty) {
            (TypePlacement::Inline, Some(ty)) => {
                let value = body[0].spans[0].content.to_string();
                let padding =
                    usize::from(width).saturating_sub(value.chars().count() + ty.chars().count());
                body[0] = Line::from(vec![
                    Span::styled(value, theme.value()),
                    Span::raw(" ".repeat(padding)),
                    Span::styled(ty.clone(), theme.type_info()),
                ]);
            }
            (TypePlacement::Newline, Some(ty)) => {
                let padding = usize::from(width).saturating_sub(ty.chars().count());
                body.push(Line::from(vec![
                    Span::raw(" ".repeat(padding)),
                    Span::styled(ty.clone(), theme.type_info()),
                ]));
            }
            _ => {}
        }
        boxed(
            area,
            buf,
            self.box_layout(context),
            theme.border(),
            None,
            None,
            body,
        );
        if let Some(number) = self.number {
            border_label(area, buf, format!("o{number}"), theme.prompt().bold());
        }
    }

    fn selection_exclusions(
        &self,
        context: &RenderContext<'_>,
        area: Rect,
        exclusions: &mut Vec<Rect>,
    ) {
        if self.number.is_some() {
            exclusions.push(Rect::new(
                area.x + 2,
                area.y,
                self.number.unwrap().to_string().len() as u16 + 1,
                1,
            ));
        }
        let Some(ty) = self.ty.as_ref() else {
            return;
        };
        let (value, placement) = self.layout(context);
        let row = match placement {
            TypePlacement::Inline => area.y + 1,
            TypePlacement::Newline => area.y + 1 + value.len() as u16,
            TypePlacement::None => return,
        };
        let right = box_inner_area(area, self.box_layout(context)).right();
        let width = ty.chars().count() as u16;
        exclusions.push(Rect::new(right.saturating_sub(width), row, width, 1));
    }

    fn kind(&self) -> BlockKind {
        BlockKind::Value
    }
}

/// The type annotation of a result M2 bound to `oN` but did not render, as a
/// `;`-suppressed input produces.
pub struct TypeLineBlock {
    pub number: u32,
    pub ty: String,
}

impl Block for TypeLineBlock {
    fn height(&self, _context: &RenderContext<'_>) -> u16 {
        2
    }

    fn render(&self, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
        let theme = context.groups();
        let line = Line::from(vec![
            gutter_span(
                &format!("o{}", self.number),
                gutter_width(self.number),
                theme.prompt().bold(),
            ),
            Span::styled(self.ty.clone(), theme.type_info()),
        ]);
        band(area, buf, vec![line, Line::default()], theme.transcript());
    }

    fn selection_exclusions(
        &self,
        _context: &RenderContext<'_>,
        area: Rect,
        exclusions: &mut Vec<Rect>,
    ) {
        exclusions.push(Rect::new(
            area.x + gutter_width(self.number),
            area.y,
            self.ty.chars().count() as u16,
            1,
        ));
    }

    fn kind(&self) -> BlockKind {
        BlockKind::Value
    }
}

/// A warning emitted while evaluating M2 code.
pub struct WarningBlock {
    pub text: String,
}

impl Block for WarningBlock {
    fn height(&self, context: &RenderContext<'_>) -> u16 {
        word_lines(&self.text, box_text_width(context, BoxLayout::Text)).len() as u16 + 2
    }

    fn render(&self, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
        let theme = context.groups();
        let body = word_lines(&self.text, box_text_width(context, BoxLayout::Text))
            .into_iter()
            .map(|line| Line::from(Span::styled(line, theme.warning())))
            .collect();
        let title = Some(Line::from(Span::styled(" Warning ", theme.warning())));
        boxed(
            area,
            buf,
            BoxLayout::Text,
            theme.warning_border(),
            title,
            None,
            body,
        );
    }

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

fn error_box_text(message: &str, context: &RenderContext<'_>) -> Vec<String> {
    let message = message
        .lines()
        .map(strip_m2_location)
        .collect::<Vec<_>>()
        .join("\n");
    word_lines(&message, box_text_width(context, BoxLayout::Text))
}

fn render_error_box(message: &str, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
    let theme = context.groups();
    let body = error_box_text(message, context)
        .into_iter()
        .map(|line| Line::from(Span::styled(line, theme.error())))
        .collect();
    boxed(
        area,
        buf,
        BoxLayout::Text,
        theme.error_border(),
        None,
        None,
        body,
    );
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

impl Block for ErrorMessageBlock {
    fn height(&self, context: &RenderContext<'_>) -> u16 {
        error_box_text(&self.message, context).len() as u16 + 2
    }

    fn render(&self, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
        render_error_box(&self.message, context, area, buf);
    }

    fn kind(&self) -> BlockKind {
        BlockKind::Error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::highlight::Highlighter;
    use crate::ui::block::render::gutter_width;
    use crate::ui::Groups;
    use m2_syn::{DelimiterKind, DoubleSpan, Span as M2Span};
    use ratatui::style::Modifier;

    fn rendered_buffer(block: &dyn Block, width: u16, groups: &Groups) -> Buffer {
        let context = RenderContext::new(width, gutter_width(99), groups);
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
        SyntaxTreeBlock::new("(x,y)".to_string(), 1, Vec::new(), root)
    }

    #[test]
    fn syntax_tree_labels_structures_after_types_and_encloses_delimited_content() {
        let groups = Groups::default();
        let context = RenderContext::new(60, gutter_width(1), &groups);
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
        assert_eq!(style("Root"), groups.macaulay(MacaulayHighlight::Type));
        assert_eq!(
            style("parameters"),
            groups.macaulay(MacaulayHighlight::Property)
        );
        assert_eq!(style("Symbol"), groups.macaulay(MacaulayHighlight::Type));
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
        let context = RenderContext::new(100, gutter_width(1), &groups);
        let block = SyntaxTreeBlock::new(source.to_string(), 1, Vec::new(), tree);
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

        assert_eq!(rows[0].chars().position(|cell| cell == 'i'), Some(29));
        assert!(left.abs_diff(width as usize - right - 1) <= 1, "{rows:#?}");
        assert_eq!(buffer[(0, 0)].bg, groups.code().bg.unwrap());
        assert_eq!(buffer[(0, 1)].bg, groups.transcript().bg.unwrap());
        assert_eq!(buffer[(0, 3)].bg, groups.code().bg.unwrap());
    }

    #[test]
    fn syntax_tree_shows_virtual_token_types_without_fake_source_text() {
        let groups = Groups::default();
        let context = RenderContext::new(40, gutter_width(1), &groups);
        let span = M2Span::for_text("xy");
        let mut tree = PrettyNode::syntax("Root", Some(span.clone()));
        tree.push_node("operator", PrettyNode::token("Adj", "Adj", span.at(1)));
        let block = SyntaxTreeBlock::new("xy".to_string(), 1, Vec::new(), tree);
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
        let context = RenderContext::new(40, gutter_width(1), &groups);
        let source = (0..20).map(|_| "long").collect::<Vec<_>>().join(" \n");
        let tree = PrettyNode::syntax("Root", Some(M2Span::for_text(source.clone())));
        let block = SyntaxTreeBlock::new(source, 1, Vec::new(), tree);
        let line = &block.tree_lines(&context)[0];

        assert!(line.width() <= usize::from(context.width()));
        assert!(line.spans.iter().any(|span| span.content.contains('…')));
    }

    #[test]
    fn input_and_output_markers_share_a_column_and_have_distinct_emphasis() {
        let theme = Groups::default();
        let width = 40;
        let input = CodeBlock::new([Cell::input("x".to_string(), 12, Vec::new())]);
        let output = OutputBlock {
            number: Some(12),
            text: "x".to_string(),
            ty: None,
        };

        let input_buffer = rendered_buffer(&input, width, &theme);
        let output_buffer = rendered_buffer(&output, width, &theme);
        let input_row = (0..width)
            .map(|x| input_buffer[(x, 1)].symbol())
            .collect::<String>();
        let output_row = (0..width)
            .map(|x| output_buffer[(x, 1)].symbol())
            .collect::<String>();
        let output_top = (0..width)
            .map(|x| output_buffer[(x, 0)].symbol())
            .collect::<String>();
        let input_x = (0..width)
            .find(|x| input_buffer[(*x, 1)].symbol() == "i")
            .expect("input marker");
        let output_x = (0..width)
            .find(|x| output_buffer[(*x, 0)].symbol() == "o")
            .expect("output marker");

        assert_eq!(input_x, output_x);
        assert_eq!(input_x, 2);
        assert_eq!(input_row.find('x'), Some(7));
        assert_eq!(
            output_row.chars().position(|character| character == 'x'),
            Some(7)
        );
        assert_eq!(output_top.chars().nth(3), Some('1'));
        assert_eq!(output_row.chars().nth(3), Some('│'));
        let input_marker = &input_buffer[(input_x, 1)];
        let output_marker = &output_buffer[(output_x, 0)];
        assert_eq!(input_marker.fg, theme.muted().fg.unwrap());
        assert!(!input_marker.modifier.contains(Modifier::BOLD));
        assert_eq!(output_marker.fg, theme.prompt().fg.unwrap());
        assert!(output_marker.modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn a_hard_wrapped_token_replaces_the_cell_number_with_a_continuation_marker() {
        let theme = Groups::default();
        let input = CodeBlock::new([Cell::input("abcdefgh".to_string(), 1, Vec::new())]);

        let rows = rendered_rows(&input, 10, &theme);
        assert!(rows[1].contains("i1"));
        assert!(rows[2].contains("󰌑"));
        assert!(!rows[2].contains("i1"));
    }

    #[test]
    fn ordinary_text_ignores_hard_wrap_markers() {
        let rows = rendered_rows(&TextBlock::system("abcdefgh"), 10, &Groups::default());

        assert!(rows.iter().all(|row| !row.contains("󰌑")));
    }

    #[test]
    fn wrapping_keeps_one_string_and_relocates_split_spans() {
        let cell = Cell::input(
            "abcdefgh".to_string(),
            1,
            vec![HLSpan {
                range: 0..8,
                kind: crate::code::highlight::MacaulayHighlight::Variable,
            }],
        );
        let (wrapped, spans) = cell.wrapped(3);

        assert_eq!(wrapped.text(), "abc\ndef\ngh");
        assert_eq!(wrapped.text().matches('\n').count(), 2);
        assert_eq!(spans[0].range, 0..3);
        assert_eq!(spans[1].range, 4..7);
        assert_eq!(spans[2].range, 8..10);
        assert_eq!(wrapped.hard_continuations().collect::<Vec<_>>(), [4, 8]);
    }

    #[test]
    fn joining_cells_preserves_their_owned_spans_and_numbers() {
        struct WholeCellHighlighter;

        impl Highlighter for WholeCellHighlighter {
            fn highlight(&mut self, source: &str) -> Vec<HLSpan> {
                vec![HLSpan {
                    range: 0..source.len(),
                    kind: crate::code::highlight::MacaulayHighlight::Variable,
                }]
            }
        }

        let mut highlighter = WholeCellHighlighter;
        let code = Cell::input("x".to_string(), 3, highlighter.highlight("x")).join(Cell::input(
            "y".to_string(),
            4,
            highlighter.highlight("y"),
        ));

        assert_eq!(code.cells[0].text, "x");
        assert_eq!(code.cells[0].number, Some(3));
        assert_eq!(code.cells[0].spans[0].range, 0..1);
        assert_eq!(code.cells[1].text, "y");
        assert_eq!(code.cells[1].number, Some(4));
        assert_eq!(code.cells[1].spans[0].range, 0..1);
        let rows = rendered_rows(&code, 20, &Groups::default());
        assert!(rows[1].contains("i3"));
        assert!(rows[2].contains("i4"));
    }

    #[test]
    fn submitted_cells_advance_from_spinner_through_queue_in_place() {
        let mut submission = SubmissionBlock::new(
            7,
            [
                ("a".to_string(), Vec::new()),
                ("b".to_string(), Vec::new()),
                ("c".to_string(), Vec::new()),
            ],
        );
        let rows = rendered_rows(&submission, 30, &Groups::default()).join("\n");
        assert!(rows.contains("⠋"));
        assert_eq!(rows.matches('…').count(), 2);

        submission.succeed(0, 4, Vec::new());
        assert!(matches!(
            submission.cells[0].1,
            SubmissionState::Succeeded(4)
        ));
        assert!(matches!(submission.cells[1].1, SubmissionState::Pending));
        submission.fail(1, 5, "bad cell".to_string());
        assert!(matches!(submission.cells[1].1, SubmissionState::Failed(5)));
        assert!(matches!(submission.cells[2].1, SubmissionState::Rejected));
    }

    #[test]
    fn one_line_output_keeps_its_number_value_and_type_inside_the_border() {
        let theme = Groups::default();
        let width = 40;
        let block = OutputBlock {
            number: Some(12),
            text: "x + y".to_string(),
            ty: Some("R".to_string()),
        };

        let rows = rendered_rows(&block, width, &theme);
        assert_eq!(rows.len(), 3);
        assert!(rows[0].contains("o12 "));
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
        assert_eq!(value - borders[0], 4);
        assert_eq!(borders[1] - ty, 3);
    }

    #[test]
    fn output_has_a_rounded_inset_border() {
        let theme = Groups::default();
        let block = OutputBlock {
            number: Some(1),
            text: "abc".to_string(),
            ty: None,
        };

        let rows = rendered_rows(&block, 20, &theme);
        assert_eq!(rows.len(), 3);
        assert!(rows[0].contains("o1 ─") && rows[0].contains('╮'));
        assert_eq!(rows[0].chars().nth(2), Some('o'));
        assert_eq!(rows[1].chars().nth(2), Some('│'));
        assert!(rows[2].contains('╰') && rows[2].contains('╯'));
    }

    #[test]
    fn multiline_output_keeps_the_type_on_its_own_last_row() {
        let theme = Groups::default();
        let block = OutputBlock {
            number: Some(12),
            text: "x\ny".to_string(),
            ty: Some("R".to_string()),
        };

        let rows = rendered_rows(&block, 40, &theme);
        assert_eq!(rows.len(), 5);
        assert!(rows[1].contains('x'));
        assert!(rows[2].contains('y'));
        assert!(rows[3].contains('R'));
    }

    #[test]
    fn an_unbound_result_shows_its_type_without_a_number() {
        let theme = Groups::default();
        let block = OutputBlock {
            number: None,
            text: "null".to_string(),
            ty: Some("Nothing".to_string()),
        };

        let rows = rendered_rows(&block, 40, &theme);
        assert!(rows[1].contains("null"));
        assert!(rows[1].contains("Nothing"));
        assert!(
            rows.iter()
                .all(|row| !row.chars().any(|c| c.is_ascii_digit())),
            "unbound result showed a number: {rows:?}"
        );
    }

    #[test]
    fn a_silent_binding_uses_the_output_column() {
        let theme = Groups::default();
        let block = TypeLineBlock {
            number: 5,
            ty: "ZZ".to_string(),
        };

        let rows = rendered_rows(&block, 40, &theme);
        assert!(
            rows[0].starts_with("  o5  ZZ"),
            "unexpected row: {:?}",
            rows[0]
        );
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
        let context = RenderContext::new(80, gutter_width(99), &theme);
        let block = TextBlock::system("commands:\n  /settings\n  /code");
        assert_eq!(block.height(&context), 4);
    }
}
