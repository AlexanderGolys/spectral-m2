//! Typed scrollable editor feed entries.
//!
//! Every [`Block`] owns text, measures and renders against a borrowed
//! [`RenderContext`], and inherits the same visual-line wrapper. Highlighted text
//! receives its highlighter separately because highlighting is an application-wide
//! service rather than editor state.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use self::render::{BorderLayout, Bordered, Highlights, RenderContext};
use self::wrap::WrappedText;
use crate::backend::transcript::strip_m2_location;
use crate::ui::config::UI;
use crate::ui::controller::settings::theme::groups::Groups;
use crate::ui::element::{Element, HLSpan, SpanText};

pub mod render;
pub mod syntax_tree;
pub mod wrap;

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

/// Content displayed before one submitted source cell.
#[derive(Clone, Copy)]
enum CellMarker {
    Bare,
    Empty,
    Number(u32, CellState),
    Pending,
    Queued,
    Failed(u32),
}

impl CellMarker {
    fn number(self) -> Option<u32> {
        match self {
            Self::Number(number, _) | Self::Failed(number) => Some(number),
            _ => None,
        }
    }

    fn width(self, layout: BorderLayout) -> u16 {
        if matches!(self, Self::Bare) {
            0
        } else {
            layout.prefix()
        }
    }

    fn label(self, groups: &Groups) -> Option<(String, Style)> {
        Some(match self {
            Self::Number(number, state) => (
                number.to_string(),
                match state {
                    CellState::Value | CellState::Muted | CellState::Null => groups.muted(),
                },
            ),
            Self::Pending => (":".into(), groups.muted()),
            Self::Queued => ("…".into(), groups.muted()),
            Self::Failed(number) => (number.to_string(), groups.error()),
            Self::Bare | Self::Empty => return None,
        })
    }

    fn span(self, layout: BorderLayout, base: Style) -> Span<'static> {
        Span::styled(" ".repeat(self.width(layout).into()), base)
    }

    fn render(self, groups: &Groups, base: Style, area: Rect, row: u16, buf: &mut Buffer) {
        if let Some((label, style)) = self.label(groups) {
            let width = label.chars().count() as u16;
            Paragraph::new(Span::styled(label, style.patch(base)))
                .render(Rect::new(area.x + UI.cell_margin, row, width, 1), buf);
        }
    }
}

/// One lexical Macaulay2 input cell and the byte ranges highlighted within it.
pub struct Cell {
    text: String,
    number: Option<(u32, CellState)>,
    spans: Vec<HLSpan>,
}

impl Cell {
    fn new(text: String, number: Option<(u32, CellState)>, spans: Vec<HLSpan>) -> Self {
        Self {
            text,
            number,
            spans,
        }
    }

    /// Create a numbered input cell from source and spans produced for that source.
    #[cfg(test)]
    fn input(text: String, spans: Vec<HLSpan>) -> Self {
        Self::new(text, None, spans)
    }

    /// Create a numbered input cell with its result state.
    pub fn resolved(text: String, number: u32, state: CellState, spans: Vec<HLSpan>) -> Self {
        Self::new(text, Some((number, state)), spans)
    }

    /// Create an unnumbered cell that was not submitted to Macaulay2.
    fn rejected(text: String, spans: Vec<HLSpan>) -> Self {
        Self::new(text, None, spans)
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

    fn marker(&self) -> CellMarker {
        self.number.map_or(CellMarker::Empty, |(number, state)| {
            CellMarker::Number(number, state)
        })
    }
}

/// One feed block containing one or more lexical input cells.
pub struct CodeBlock {
    cells: Vec<Cell>,
}

impl CodeBlock {
    /// Create a code block from lexical cells in display order.
    pub fn new(cells: impl IntoIterator<Item = Cell>) -> Self {
        Self {
            cells: cells.into_iter().collect(),
        }
    }

    fn layout(&self) -> BorderLayout {
        self.cells
            .iter()
            .filter_map(|cell| cell.marker().number())
            .max()
            .map_or_else(BorderLayout::plain, BorderLayout::numbered)
    }

    fn body_height(&self, context: &RenderContext) -> u16 {
        let layout = self.layout();
        self.cells
            .iter()
            .map(|cell| {
                cell.wrapped(context.width().saturating_sub(cell.marker().width(layout)))
                    .height()
            })
            .sum()
    }

    fn lines(&self, context: &RenderContext, style: Style) -> Vec<Line<'static>> {
        let layout = self.layout();
        let mut rendered = vec![Line::default()];
        for cell in &self.cells {
            rendered.extend(cell_lines(cell, cell.marker(), layout, context, style));
        }
        rendered.push(Line::default());
        rendered
    }

    fn preview_lines(&self, context: &RenderContext) -> Vec<Line<'static>> {
        let context = RenderContext::new(context.width(), 0, context.groups());
        let layout = BorderLayout::plain();
        let mut rendered = Vec::new();
        for cell in &self.cells {
            rendered.extend(cell_lines(
                cell,
                CellMarker::Bare,
                layout,
                &context,
                context.groups().transcript(),
            ));
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
        let layout = self.layout();
        layout.render_separator(area, context.groups().separator(), buf);
        let mut row = area.y + 1;
        for cell in &self.cells {
            let marker = cell.marker();
            marker.render(context.groups(), style, area, row, buf);
            row += cell
                .wrapped(context.width().saturating_sub(marker.width(layout)))
                .height();
        }
    }
}

impl Block for CodeBlock {
    fn kind(&self) -> BlockKind {
        BlockKind::Input
    }

    fn selection_exclusions(
        &self,
        context: &RenderContext,
        area: Rect,
        exclusions: &mut Vec<Rect>,
    ) {
        exclusions.push(Rect::new(
            area.x,
            area.y,
            self.layout().prefix(),
            self.height(context),
        ));
    }
}

fn cell_lines(
    cell: &Cell,
    marker: CellMarker,
    layout: BorderLayout,
    context: &RenderContext,
    style: Style,
) -> Vec<Line<'static>> {
    let width = marker.width(layout);
    let spans = cell.spans.paint(0, &cell.text, style, context.groups());
    cell.wrapped(context.width().saturating_sub(width))
        .source_lines()
        .enumerate()
        .map(|(index, (range, _))| {
            let marker = if index == 0 {
                marker
            } else {
                CellMarker::Empty
            };
            let mut line = vec![marker.span(layout, style)];
            line.extend(spans.slice(range));
            Line::from(line)
        })
        .collect()
}

/// Evaluation progress for one submitted input cell.
enum SubmissionState {
    Pending,
    Queued,
    Succeeded(u32, CellState),
    Failed(u32),
    Rejected,
}

impl SubmissionState {
    fn marker(&self) -> CellMarker {
        match self {
            Self::Pending => CellMarker::Pending,
            Self::Queued => CellMarker::Queued,
            Self::Succeeded(number, state) => CellMarker::Number(*number, *state),
            Self::Failed(number) => CellMarker::Failed(*number),
            Self::Rejected => CellMarker::Empty,
        }
    }
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

    pub fn succeed(
        &mut self,
        index: usize,
        number: u32,
        cell_state: CellState,
        outputs: Vec<Box<dyn Block>>,
    ) {
        if let Some((_, state)) = self.cells.get_mut(index) {
            *state = SubmissionState::Succeeded(number, cell_state);
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

    pub fn fail(&mut self, index: usize, number: u32, message: String) {
        if let Some((_, state)) = self.cells.get_mut(index) {
            *state = SubmissionState::Failed(number);
        }
        for (_, state) in self.cells.iter_mut().skip(index + 1) {
            *state = SubmissionState::Rejected;
        }
        self.outputs.push(Box::new(ErrorMessageBlock { message }));
    }

    fn layout(&self) -> BorderLayout {
        self.cells
            .iter()
            .filter_map(|(_, state)| state.marker().number())
            .max()
            .map_or_else(BorderLayout::plain, BorderLayout::numbered)
    }

    fn code_height(&self, context: &RenderContext) -> u16 {
        let layout = self.layout();
        self.cells
            .iter()
            .map(|(cell, state)| {
                let marker = state.marker();
                cell.wrapped(context.width().saturating_sub(marker.width(layout)))
                    .height()
            })
            .sum::<u16>()
            + 2
    }

    fn code_lines(&self, context: &RenderContext) -> Vec<Line<'static>> {
        let layout = self.layout();
        let mut lines = vec![Line::default()];
        for (cell, state) in &self.cells {
            let style = if matches!(state, SubmissionState::Failed(_)) {
                context.groups().error().patch(context.groups().code())
            } else {
                context.groups().code()
            };
            lines.extend(cell_lines(cell, state.marker(), layout, context, style));
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
        let layout = self.layout();
        layout.render_separator(
            Rect::new(area.x, area.y, area.width, code_height),
            context.groups().separator(),
            buf,
        );
        let mut row = area.y + 1;
        for (cell, state) in &self.cells {
            let marker = state.marker();
            let style = if matches!(state, SubmissionState::Failed(_)) {
                context.groups().error().patch(context.groups().code())
            } else {
                context.groups().code()
            };
            marker.render(context.groups(), style, area, row, buf);
            row += cell
                .wrapped(context.width().saturating_sub(marker.width(layout)))
                .height();
        }
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
        let code_height = self.code_height(context);
        exclusions.push(Rect::new(
            area.x,
            area.y,
            self.layout().prefix(),
            code_height,
        ));
        let mut y = area.y + code_height;
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
        let width = match self.kind {
            TextKind::System => Bordered::text_width(context, BorderLayout::middle_third()),
            TextKind::Printed => context
                .width()
                .saturating_sub(BorderLayout::plain().prefix()),
        };
        let mut lines = word_lines(&self.text, width);
        if lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines
    }
}

impl Element for TextBlock {
    type Context = RenderContext;

    fn height(&self, context: &RenderContext) -> u16 {
        self.body(context).len() as u16 + 2
    }

    fn render(&self, context: &RenderContext, area: Rect, buf: &mut Buffer) {
        let theme = context.groups();
        match self.kind {
            TextKind::System => {
                let body = self
                    .body(context)
                    .into_iter()
                    .map(|line| {
                        Line::from(Span::styled(line, theme.info())).alignment(Alignment::Center)
                    })
                    .collect();
                Bordered::new(
                    BorderLayout::middle_third(),
                    Style::new(),
                    theme.info_border(),
                    None,
                    body,
                )
                .render(&(), area, buf);
            }
            TextKind::Printed => {
                let prefix = usize::from(BorderLayout::plain().prefix());
                let margin = Line::from(Span::raw(" ".repeat(prefix)));
                let mut lines = vec![margin.clone()];
                lines.extend(
                    self.body(context).into_iter().map(|line| {
                        Line::from(vec![Span::raw(" ".repeat(prefix)), Span::raw(line)])
                    }),
                );
                lines.push(margin);
                Paragraph::new(lines)
                    .style(theme.printed())
                    .render(area, buf);
            }
        }
    }
}

impl Block for TextBlock {
    fn kind(&self) -> BlockKind {
        match self.kind {
            TextKind::System => BlockKind::System,
            TextKind::Printed => BlockKind::Printed,
        }
    }

    fn selection_exclusions(
        &self,
        _context: &RenderContext,
        area: Rect,
        exclusions: &mut Vec<Rect>,
    ) {
        if matches!(self.kind, TextKind::Printed) {
            exclusions.push(Rect::new(
                area.x,
                area.y,
                BorderLayout::plain().prefix(),
                area.height,
            ));
        }
    }
}

/// One result inside a possibly shared value border.
struct OutputCell {
    number: Option<u32>,
    text: String,
    ty: Option<String>,
}

/// Consecutive values returned by M2 inside one border.
pub struct OutputBlock {
    cells: Vec<OutputCell>,
}

impl OutputBlock {
    /// Create a rendered value and its optional type.
    pub fn value(text: String, ty: Option<String>) -> Self {
        Self {
            cells: vec![OutputCell {
                number: None,
                text,
                ty,
            }],
        }
    }

    /// Attach the `oN` assigned by Macaulay2 to this value.
    pub fn numbered(mut self, number: u32) -> Self {
        self.cells[0].number = Some(number);
        self
    }

    fn border_layout(&self) -> BorderLayout {
        self.cells
            .iter()
            .filter_map(|cell| cell.number)
            .max()
            .map_or_else(BorderLayout::plain, BorderLayout::output)
    }

    fn text_width(&self, context: &RenderContext) -> u16 {
        Bordered::text_width(context, self.border_layout())
    }

    fn layout(&self, cell: &OutputCell, context: &RenderContext) -> Vec<String> {
        if cell.text.is_empty() {
            Vec::new()
        } else {
            word_lines(&cell.text, self.text_width(context))
        }
    }

    fn lines(&self, cell: &OutputCell, context: &RenderContext) -> Vec<Line<'static>> {
        let theme = context.groups();
        let width = self.text_width(context);
        let mut lines: Vec<_> = self
            .layout(cell, context)
            .into_iter()
            .map(|line| Line::from(Span::styled(line, theme.value())))
            .collect();
        if let Some(ty) = &cell.ty {
            if lines.is_empty() {
                lines.push(Line::default());
            }
            let ty = Span::styled(ty.clone(), theme.type_info());
            let padding =
                usize::from(width).saturating_sub(lines.last().unwrap().width() + ty.width());
            match padding {
                0 => lines.push(Line::from(vec![
                    Span::raw(" ".repeat(usize::from(width).saturating_sub(ty.width()))),
                    ty,
                ])),
                _ => lines
                    .last_mut()
                    .unwrap()
                    .spans
                    .extend([Span::raw(" ".repeat(padding)), ty]),
            }
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
        let layout = self.border_layout();
        Bordered::new(layout, Style::new(), theme.border(), None, body).render(&(), area, buf);
        let mut row = Bordered::inner(area, layout).y;
        for cell in &self.cells {
            if let Some(number) = cell.number {
                let label = format!("o{number}");
                Paragraph::new(Span::styled(label.clone(), theme.output_number())).render(
                    Rect::new(
                        area.x + UI.cell_margin,
                        row,
                        label.chars().count() as u16,
                        1,
                    ),
                    buf,
                );
            }
            row += self.lines(cell, context).len() as u16;
        }
    }
}

impl Block for OutputBlock {
    fn selection_exclusions(
        &self,
        context: &RenderContext,
        area: Rect,
        exclusions: &mut Vec<Rect>,
    ) {
        let layout = self.border_layout();
        let inner = Bordered::inner(area, layout);
        let right = inner.right();
        let mut row = inner.y;
        for cell in &self.cells {
            let lines = self.lines(cell, context);
            exclusions.push(Rect::new(area.x, row, layout.prefix(), lines.len() as u16));
            if let Some(ty) = &cell.ty {
                let width = ty.chars().count() as u16;
                exclusions.push(Rect::new(
                    right.saturating_sub(width),
                    row + lines.len().saturating_sub(1) as u16,
                    width,
                    1,
                ));
            }
            row += lines.len() as u16;
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
        word_lines(
            &self.text,
            Bordered::text_width(context, BorderLayout::plain()),
        )
        .len() as u16
            + 2
    }

    fn render(&self, context: &RenderContext, area: Rect, buf: &mut Buffer) {
        let theme = context.groups();
        let body = word_lines(
            &self.text,
            Bordered::text_width(context, BorderLayout::plain()),
        )
        .into_iter()
        .map(|line| Line::from(Span::styled(line, theme.warning())))
        .collect();
        let title = Some(Line::from(Span::styled(" Warning ", theme.warning())));
        Bordered::new(
            BorderLayout::plain(),
            Style::new(),
            theme.warning_border(),
            title,
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
    word_lines(
        &message,
        Bordered::text_width(context, BorderLayout::plain()),
    )
}

fn render_error_box(message: &str, context: &RenderContext, area: Rect, buf: &mut Buffer) {
    let theme = context.groups();
    let body = error_box_text(message, context)
        .into_iter()
        .map(|line| Line::from(Span::styled(line, theme.error())))
        .collect();
    Bordered::new(
        BorderLayout::plain(),
        Style::new(),
        theme.error_border(),
        None,
        body,
    )
    .render(&(), area, buf);
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
    use super::syntax_tree::SyntaxTreeBlock;
    use super::*;
    use crate::code::highlight::Highlighter;
    use crate::ui::controller::settings::theme::groups::{Groups, HLGroup};
    use m2_syn::{DelimiterKind, DoubleSpan, PrettyNode, Span as M2Span};

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
    fn syntax_tree_uses_the_full_width_and_inverts_the_source_band() {
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
        assert!(rows[0].chars().all(|cell| cell == '─'));
        assert!(rows[1].starts_with("(x,y)"), "{rows:#?}");
        assert!(rows[3].starts_with("Root"), "{rows:#?}");
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
    fn code_and_output_reserve_the_cell_marker_margin() {
        let theme = Groups::default();
        let width = 40;
        let input = CodeBlock::new([Cell::input("x".to_string(), Vec::new())]);
        let output = OutputBlock::value("x".to_string(), None);

        let input = rendered_rows(&input, width, &theme);
        let output = rendered_rows(&output, width, &theme);
        assert_eq!(
            input[1].chars().position(|character| character == 'x'),
            Some(4)
        );
        assert_eq!(output[0].chars().nth(2), Some('╭'));
    }

    #[test]
    fn successful_cell_and_output_numbers_align_without_bold() {
        use ratatui::style::Modifier;

        let groups = Groups::default();
        let input = CodeBlock::new([Cell::resolved("x".into(), 7, CellState::Value, Vec::new())]);
        let output = OutputBlock::value("x".into(), None).numbered(7);
        let input = rendered_buffer(&input, 20, &groups);
        let output = rendered_buffer(&output, 20, &groups);

        assert_eq!(input[(1, 1)].symbol(), "7");
        assert_eq!(output[(1, 1)].symbol(), "o");
        assert_eq!(output[(2, 1)].symbol(), "7");
        assert_eq!(output[(2, 0)].symbol(), "╭");
        assert_eq!(output[(2, 2)].symbol(), "╰");
        assert_eq!(input[(1, 1)].fg, groups.muted().fg.unwrap());
        assert_eq!(output[(2, 1)].fg, groups.output_number().fg.unwrap());
        assert!(!input[(1, 1)].modifier.contains(Modifier::BOLD));
        assert!(!output[(2, 1)].modifier.contains(Modifier::BOLD));
        assert_eq!(input[(2, 0)].symbol(), "▏");
        assert_eq!(input[(2, 0)].fg, groups.separator().fg.unwrap());

        let null = rendered_buffer(
            &CodeBlock::new([Cell::resolved("x".into(), 7, CellState::Null, Vec::new())]),
            20,
            &groups,
        );
        assert_eq!(null[(1, 1)].fg, groups.muted().fg.unwrap());
    }

    #[test]
    fn multi_digit_output_opens_one_cell_in_the_shared_border() {
        let groups = Groups::default();
        let output = OutputBlock::value("a\nb".into(), None).numbered(12);
        let output = rendered_buffer(&output, 20, &groups);

        assert_eq!(output[(2, 0)].symbol(), "╭");
        assert_eq!(output[(1, 1)].symbol(), "o");
        assert_eq!(output[(2, 1)].symbol(), "1");
        assert_eq!(output[(3, 1)].symbol(), "2");
        assert_eq!(output[(2, 2)].symbol(), "│");
        assert_eq!(output[(2, 3)].symbol(), "╰");
    }

    #[test]
    fn hard_wrapped_code_keeps_its_prefix() {
        let theme = Groups::default();
        let input = CodeBlock::new([Cell::input("abcdefgh".to_string(), Vec::new())]);

        let rows = rendered_rows(&input, 8, &theme);
        assert_eq!(
            rows[1].chars().position(|character| character == 'a'),
            Some(4)
        );
        assert_eq!(
            rows[2].chars().position(|character| character == 'e'),
            Some(4)
        );
    }

    #[test]
    fn merged_code_uses_its_longest_number_and_continuous_separator() {
        let mut submission =
            SubmissionBlock::new(1, ["a", "b"].map(|text| (text.to_string(), Vec::new())));
        submission.succeed(0, 9, CellState::Value, Vec::new());
        submission.succeed(1, 10, CellState::Value, Vec::new());

        let rows = rendered_rows(&submission, 20, &Groups::default());
        assert_eq!(rows[0].chars().nth(3), Some('▏'));
        assert_eq!(rows[1].chars().nth(1), Some('9'));
        assert_eq!(rows[2].chars().nth(1), Some('1'));
        assert_eq!(rows[2].chars().nth(2), Some('0'));
        assert_eq!(
            rows[1].chars().position(|character| character == 'a'),
            Some(5)
        );
        assert_eq!(
            rows[2].chars().position(|character| character == 'b'),
            Some(5)
        );
        assert_eq!(rows[3].chars().nth(3), Some('▏'));
    }

    #[test]
    fn printed_text_keeps_padding_without_a_separator() {
        let rows = rendered_rows(&TextBlock::printed("text"), 20, &Groups::default());

        assert!(rows.iter().all(|row| !row.contains('▏')));
        assert_eq!(
            rows[1].chars().position(|character| character == 't'),
            Some(4)
        );
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
        assert_eq!(
            rows[1].chars().position(|character| character == 'x'),
            Some(4)
        );
        assert_eq!(
            rows[2].chars().position(|character| character == 'y'),
            Some(4)
        );
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
        submission.succeed(0, 1, CellState::Value, Vec::new());
        assert!(matches!(
            submission.cells[0].1,
            SubmissionState::Succeeded(1, CellState::Value)
        ));
        assert!(matches!(submission.cells[1].1, SubmissionState::Pending));
        submission.fail(1, 2, "bad cell".to_string());
        assert!(matches!(submission.cells[1].1, SubmissionState::Failed(2)));
        assert!(matches!(submission.cells[2].1, SubmissionState::Rejected));
    }

    #[test]
    fn non_values_do_not_add_output_cells() {
        let mut submission = SubmissionBlock::new(
            1,
            ["a", "b", "c"].map(|text| (text.to_string(), Vec::new())),
        );
        submission.succeed(
            0,
            1,
            CellState::Value,
            vec![Box::new(
                OutputBlock::value("a".to_string(), None).numbered(1),
            )],
        );
        submission.succeed(1, 2, CellState::Muted, Vec::new());
        submission.succeed(2, 3, CellState::Null, Vec::new());

        assert_eq!(submission.outputs.len(), 1);
        assert_eq!(submission.outputs[0].value_mut().unwrap().cells.len(), 1);
    }

    #[test]
    fn consecutive_values_share_one_border() {
        let mut submission =
            SubmissionBlock::new(1, ["a", "b"].map(|text| (text.to_string(), Vec::new())));
        for (index, number) in [1, 2].into_iter().enumerate() {
            submission.succeed(
                index,
                number,
                CellState::Value,
                vec![Box::new(
                    OutputBlock::value(number.to_string(), None).numbered(number),
                )],
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
        let border = cells
            .iter()
            .rposition(|character| *character == '│')
            .unwrap();
        let value = cells
            .iter()
            .position(|character| *character == 'x')
            .unwrap();
        let ty = cells
            .iter()
            .position(|character| *character == 'R')
            .unwrap();
        assert_eq!(value, 4);
        assert_eq!(border - ty, 2);
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
        assert_eq!(rows[0].chars().nth(2), Some('╭'));
        assert_eq!(
            input[1].chars().position(|character| character == 'x'),
            Some(4)
        );
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
            assert_eq!(buffer[(2, 0)].symbol(), "╭");
            assert_eq!(buffer[(2, block.height(&context) - 1)].symbol(), "╰");
        }
    }

    #[test]
    fn diagnostic_boxes_leave_their_surfaces_unfilled() {
        let theme = Groups::default();
        for (block, text) in [
            (
                Box::new(WarningBlock {
                    text: "warning".into(),
                }) as Box<dyn Block>,
                theme.warning(),
            ),
            (
                Box::new(ErrorMessageBlock::new("error".into())) as Box<dyn Block>,
                theme.error(),
            ),
        ] {
            let buffer = rendered_buffer(block.as_ref(), 20, &theme);
            let cell = &buffer[(5, 1)];
            assert_eq!(cell.fg, text.fg.unwrap());
            assert_eq!(cell.bg, ratatui::style::Color::Reset);
        }
    }

    #[test]
    fn multiline_output_keeps_the_type_beside_its_last_value_row() {
        let theme = Groups::default();
        let block = OutputBlock::value("x\ny".to_string(), Some("R".to_string()));

        let rows = rendered_rows(&block, 40, &theme);
        assert_eq!(rows.len(), 4);
        assert!(rows[1].contains('x'));
        assert!(rows[2].contains('y') && rows[2].contains('R'));
    }

    #[test]
    fn overflowing_type_moves_below_a_full_value_row() {
        let theme = Groups::default();
        let width = 20;
        let context = RenderContext::new(width, 0, &theme);
        let width = OutputBlock::value(String::new(), None).text_width(&context);
        let block = OutputBlock::value("x".repeat(width.into()), Some("R".into()));

        let rows = rendered_rows(&block, 20, &theme);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1].matches('x').count(), usize::from(width));
        assert!(!rows[1].contains('R'));
        assert!(rows[2].contains('R'));
    }

    #[test]
    fn a_result_shows_its_type() {
        let theme = Groups::default();
        let block = OutputBlock::value("x".to_string(), Some("R".to_string()));

        let rows = rendered_rows(&block, 40, &theme);
        assert!(rows[1].contains('x'));
        assert!(rows[1].contains('R'));
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
        assert_eq!(block.height(&context), 5);
    }

    #[test]
    fn system_block_is_a_centered_info_box() {
        let theme = Groups::default();
        let buffer = rendered_buffer(&TextBlock::system("message"), 60, &theme);

        assert_eq!(buffer[(20, 0)].symbol(), "╭");
        assert_eq!(buffer[(40, 0)].symbol(), "╮");
        assert_eq!(buffer[(27, 1)].symbol(), "m");
        assert_eq!(buffer[(27, 1)].fg, theme.info().fg.unwrap());
    }
}
