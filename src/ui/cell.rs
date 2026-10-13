//! Typed cells and messages rendered in the editor feed.
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

use super::block::{Block, Element, HLSpan, Text};
use super::element::paste;
use super::render::maths;
use super::render::selection::Exclusions;
use super::render::{Bordered, FrameLayout, Highlights, RenderContext};
use super::wrap::WrappedText;
use crate::backend::{transcript::strip_m2_location, M2Value};
use crate::ui::config::UI;
use crate::ui::controller::settings::theme::groups::Groups;

macro_rules! row_count {
    ($rows:expr) => {
        u16::try_from($rows).unwrap_or(u16::MAX)
    };
}

/// How a successful input cell's result appears in the transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CellState {
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
    Invalid,
}

/// Local gutter geometry shared by source cells and ordinary cell messages.
#[derive(Clone, Copy)]
struct CellLayout(FrameLayout);

impl CellLayout {
    const fn numbered(number: u32) -> Self {
        let digits = if number == 0 {
            1
        } else {
            number.ilog10() as u16 + 1
        };
        Self(FrameLayout::gutter(
            UI.cell_margin + digits + UI.cell_gap,
            UI.cell_margin + digits,
        ))
    }

    const fn plain() -> Self {
        Self::numbered(1)
    }

    const fn frame(self) -> FrameLayout {
        self.0
    }

    const fn prefix(self) -> u16 {
        self.0.prefix()
    }

    fn render_separator(self, height: u16, style: Style, buf: &mut Buffer) {
        let x = self.0.inset();
        for y in 0..height {
            buf[(x, y)].set_symbol("│").set_style(style);
        }
    }
}

impl CellMarker {
    fn number(self) -> Option<u32> {
        match self {
            Self::Number(number, _) | Self::Failed(number) => Some(number),
            _ => None,
        }
    }

    fn width(self, layout: CellLayout) -> u16 {
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
            Self::Invalid => ("×".into(), groups.error()),
            Self::Bare | Self::Empty => return None,
        })
    }

    fn span(self, layout: CellLayout, base: Style) -> Span<'static> {
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
pub(crate) struct Cell {
    text: String,
    number: Option<(u32, CellState)>,
    spans: Vec<HLSpan>,
}

impl Cell {
    pub(super) fn new(text: String, number: Option<(u32, CellState)>, spans: Vec<HLSpan>) -> Self {
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
    pub(crate) fn resolved(
        text: String,
        number: u32,
        state: CellState,
        spans: Vec<HLSpan>,
    ) -> Self {
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
pub(crate) struct CodeBlock {
    cells: Vec<Cell>,
}

impl CodeBlock {
    /// Create a code block from lexical cells in display order.
    pub(crate) fn new(cells: impl IntoIterator<Item = Cell>) -> Self {
        Self {
            cells: cells.into_iter().collect(),
        }
    }

    fn layout(&self) -> CellLayout {
        self.cells
            .iter()
            .filter_map(|cell| cell.marker().number())
            .max()
            .map_or_else(CellLayout::plain, CellLayout::numbered)
    }

    fn gap(index: usize) -> u16 {
        u16::from(index > 0)
    }

    fn body_height(&self, context: &RenderContext) -> u16 {
        let layout = self.layout();
        self.cells
            .iter()
            .enumerate()
            .fold(0, |height, (index, cell)| {
                height.saturating_add(Self::gap(index)).saturating_add(
                    cell.wrapped(context.width().saturating_sub(cell.marker().width(layout)))
                        .height(),
                )
            })
    }

    fn lines(&self, context: &RenderContext, style: Style) -> Vec<Line<'static>> {
        let layout = self.layout();
        let mut rendered = vec![Line::default()];
        for (index, cell) in self.cells.iter().enumerate() {
            rendered.extend((index > 0).then(Line::default));
            rendered.extend(cell_lines(cell, cell.marker(), layout, context, style));
        }
        rendered
    }

    pub(super) fn preview_lines(&self, context: &RenderContext) -> Vec<Line<'static>> {
        let context = RenderContext::new(context.width(), 0);
        let layout = CellLayout::plain();
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

/// The single unfinished submission retained at the bottom of the feed.
pub(crate) struct IncompleteBlock {
    cell: Cell,
}

impl IncompleteBlock {
    /// Create an unfinished block from the parser's complete active source.
    pub(crate) fn new(source: String, spans: Vec<HLSpan>) -> Self {
        Self {
            cell: Cell::rejected(source, spans),
        }
    }

    /// Return the source when this block is moved back into the editor.
    pub(crate) fn into_source(self) -> String {
        self.cell.text
    }

    /// Borrow the retained source as highlighting context for the live input.
    pub(crate) fn source(&self) -> &str {
        &self.cell.text
    }
}

impl Block for IncompleteBlock {
    fn height(&self, context: &RenderContext) -> u16 {
        self.cell
            .wrapped(context.width().saturating_sub(CellLayout::plain().prefix()))
            .height()
            .saturating_add(1)
    }

    fn render_local(&self, context: &RenderContext, buf: &mut Buffer) {
        let area = *buf.area();
        let style = context.groups().pending();
        let mut lines = vec![Line::default()];
        lines.extend(cell_lines(
            &self.cell,
            CellMarker::Empty,
            CellLayout::plain(),
            context,
            style,
        ));
        Paragraph::new(lines).style(style).render(area, buf);
        for x in 0..area.width {
            buf[(x, 0)]
                .set_symbol("┈")
                .set_style(context.groups().muted().patch(style));
        }
    }

    fn local_selection_exclusions(&self, context: &RenderContext, exclusions: &mut Exclusions) {
        exclusions.push(Rect::new(0, 0, context.width(), 1));
    }
}

impl Block for CodeBlock {
    fn height(&self, context: &RenderContext) -> u16 {
        self.body_height(context).saturating_add(1)
    }

    fn render_local(&self, context: &RenderContext, buf: &mut Buffer) {
        let area = *buf.area();
        let style = context.groups().code();
        Paragraph::new(self.lines(context, style))
            .style(style)
            .render(area, buf);
        let layout = self.layout();
        layout.render_separator(area.height, context.groups().separator(), buf);
        let mut row = area.y.saturating_add(1);
        for (index, cell) in self.cells.iter().enumerate() {
            row = row.saturating_add(Self::gap(index));
            let marker = cell.marker();
            marker.render(context.groups(), style, area, row, buf);
            row = row.saturating_add(
                cell.wrapped(context.width().saturating_sub(marker.width(layout)))
                    .height(),
            );
        }
    }
    fn local_selection_exclusions(&self, context: &RenderContext, exclusions: &mut Exclusions) {
        exclusions.push(Rect::new(
            0,
            0,
            self.layout().prefix(),
            self.height(context),
        ));
    }
}

fn cell_lines(
    cell: &Cell,
    marker: CellMarker,
    layout: CellLayout,
    context: &RenderContext,
    style: Style,
) -> Vec<Line<'static>> {
    let width = marker.width(layout);
    cell.wrapped(context.width().saturating_sub(width))
        .source_lines()
        .enumerate()
        .map(|(index, (range, _))| {
            let marker = match (index, marker) {
                (_, CellMarker::Bare) | (0, _) => marker,
                _ => CellMarker::Empty,
            };
            let mut line = vec![marker.span(layout, style)];
            line.extend(cell.spans.paint(
                range.start,
                &cell.text[range.clone()],
                style,
                context.groups(),
            ));
            Line::from(line)
        })
        .collect()
}

/// Evaluation progress for one submitted input cell.
#[derive(Clone, Copy)]
enum EvaluationState {
    Pending,
    Queued,
    Succeeded(u32, CellState),
    Failed(u32),
    Invalid,
}

impl EvaluationState {
    fn marker(&self) -> CellMarker {
        match self {
            Self::Pending => CellMarker::Pending,
            Self::Queued => CellMarker::Queued,
            Self::Succeeded(number, state) => CellMarker::Number(*number, *state),
            Self::Failed(number) => CellMarker::Failed(*number),
            Self::Invalid => CellMarker::Invalid,
        }
    }
}

/// One rendered result retained by a submission.
enum EvaluationOutput {
    /// Any non-value result remains open to every [`Block`] implementation.
    Evaluation(Box<dyn Block>),
    /// Values stay typed so adjacent cells can share one rendered border.
    Value(OutputBlock),
}

impl EvaluationOutput {
    fn block(&self) -> &dyn Block {
        match self {
            Self::Evaluation(block) => block.as_ref(),
            Self::Value(block) => block,
        }
    }
}

/// Submitted cells that resolve in place while their M2 evaluation runs.
pub(crate) struct EvaluationBlock {
    id: u64,
    cells: Vec<(Cell, EvaluationState)>,
    outputs: Vec<EvaluationOutput>,
}

impl EvaluationBlock {
    const OUTPUT_MARGIN: u16 = 1;

    /// Create a pending submission from source cells and their highlights.
    pub(crate) fn new(id: u64, cells: impl IntoIterator<Item = (String, Vec<HLSpan>)>) -> Self {
        Self::with_state(id, cells, EvaluationState::Pending)
    }

    /// Create a submission waiting behind the active evaluation.
    pub(crate) fn queued(id: u64, cells: impl IntoIterator<Item = (String, Vec<HLSpan>)>) -> Self {
        Self::with_state(id, cells, EvaluationState::Queued)
    }

    /// Create a locally rejected evaluation whose syntax diagnostic is final.
    pub(crate) fn invalid(id: u64, source: String, spans: Vec<HLSpan>, message: String) -> Self {
        Self {
            id,
            cells: vec![(Cell::rejected(source, spans), EvaluationState::Invalid)],
            outputs: vec![EvaluationOutput::Evaluation(Box::new(ErrorMessageBlock {
                message,
            }))],
        }
    }

    fn with_state(
        id: u64,
        cells: impl IntoIterator<Item = (String, Vec<HLSpan>)>,
        first: EvaluationState,
    ) -> Self {
        Self {
            id,
            cells: cells
                .into_iter()
                .enumerate()
                .map(|(index, (text, spans))| {
                    (
                        Cell::rejected(text, spans),
                        match index {
                            0 => first,
                            _ => EvaluationState::Queued,
                        },
                    )
                })
                .collect(),
            outputs: Vec::new(),
        }
    }

    /// Promote the first unresolved cell when this submission reaches the queue front.
    pub(crate) fn start(&mut self) {
        if let Some((_, state @ EvaluationState::Queued)) = self
            .cells
            .iter_mut()
            .find(|(_, state)| matches!(state, EvaluationState::Queued))
        {
            *state = EvaluationState::Pending;
        }
    }

    /// Return the session-local submission identifier.
    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    /// Resolve one cell and append its non-value output and optional value.
    ///
    /// The value remains concrete so it can merge with an adjacent value block;
    /// every other result needs only the generic rendering contract.
    pub(crate) fn succeed(
        &mut self,
        index: usize,
        number: u32,
        cell_state: CellState,
        outputs: Vec<Box<dyn Block>>,
        value: Option<OutputBlock>,
    ) {
        if let Some((_, state)) = self.cells.get_mut(index) {
            *state = EvaluationState::Succeeded(number, cell_state);
        }
        if let Some((_, state)) = self.cells.get_mut(index + 1) {
            *state = EvaluationState::Pending;
        }
        self.outputs
            .extend(outputs.into_iter().map(EvaluationOutput::Evaluation));
        if let Some(value) = value {
            match self.outputs.last_mut() {
                Some(EvaluationOutput::Value(values)) => values.append(value),
                _ => self.outputs.push(EvaluationOutput::Value(value)),
            }
        }
    }

    /// Visit every nested result in transcript order.
    pub(super) fn outputs(&self) -> impl Iterator<Item = &dyn Block> {
        self.outputs.iter().map(EvaluationOutput::block)
    }

    /// Visit printed text, warnings, and errors without returned values.
    pub(super) fn evaluation_outputs(&self) -> impl Iterator<Item = &dyn Block> {
        self.outputs.iter().filter_map(|output| match output {
            EvaluationOutput::Evaluation(block) => Some(block.as_ref()),
            EvaluationOutput::Value(_) => None,
        })
    }

    /// Visit only returned value blocks.
    pub(super) fn values(&self) -> impl Iterator<Item = &dyn Block> {
        self.outputs.iter().filter_map(|output| match output {
            EvaluationOutput::Value(block) => Some(block as &dyn Block),
            EvaluationOutput::Evaluation(_) => None,
        })
    }

    /// Mark one cell failed and append its error while later cells remain queued.
    pub(crate) fn fail(
        &mut self,
        index: usize,
        number: u32,
        outputs: Vec<Box<dyn Block>>,
        message: String,
    ) {
        if let Some((_, state)) = self.cells.get_mut(index) {
            *state = EvaluationState::Failed(number);
        }
        self.outputs
            .extend(outputs.into_iter().map(EvaluationOutput::Evaluation));
        self.outputs
            .push(EvaluationOutput::Evaluation(Box::new(ErrorMessageBlock {
                message,
            })));
    }

    fn layout(&self) -> CellLayout {
        self.cells
            .iter()
            .filter_map(|(_, state)| state.marker().number())
            .max()
            .map_or_else(CellLayout::plain, CellLayout::numbered)
    }

    fn code_height(&self, context: &RenderContext) -> u16 {
        let layout = self.layout();
        self.cells.iter().fold(1, |height, (cell, state)| {
            let marker = state.marker();
            height.saturating_add(
                cell.wrapped(context.width().saturating_sub(marker.width(layout)))
                    .height(),
            )
        })
    }

    fn code_lines(&self, context: &RenderContext) -> Vec<Line<'static>> {
        let layout = self.layout();
        let mut lines = vec![Line::default()];
        for (cell, state) in &self.cells {
            let style = if matches!(state, EvaluationState::Failed(_) | EvaluationState::Invalid) {
                context.groups().error().patch(context.groups().code())
            } else {
                context.groups().code()
            };
            lines.extend(cell_lines(cell, state.marker(), layout, context, style));
        }
        lines
    }

    fn output_margin(&self) -> u16 {
        Self::OUTPUT_MARGIN * u16::from(!self.outputs.is_empty())
    }
}

impl Block for EvaluationBlock {
    fn height(&self, context: &RenderContext) -> u16 {
        self.outputs.iter().fold(
            self.code_height(context)
                .saturating_add(self.output_margin()),
            |height, output| height.saturating_add(output.block().height(context)),
        )
    }

    fn render_local(&self, context: &RenderContext, buf: &mut Buffer) {
        let area = *buf.area();
        let code_height = self.code_height(context);
        let code_area_height = code_height.saturating_add(self.output_margin());
        Paragraph::new(self.code_lines(context))
            .style(context.groups().code())
            .render(Rect::new(area.x, area.y, area.width, code_area_height), buf);
        let layout = self.layout();
        layout.render_separator(code_area_height, context.groups().separator(), buf);
        let mut row = area.y.saturating_add(1);
        for (cell, state) in &self.cells {
            let marker = state.marker();
            let style = if matches!(state, EvaluationState::Failed(_) | EvaluationState::Invalid) {
                context.groups().error().patch(context.groups().code())
            } else {
                context.groups().code()
            };
            marker.render(context.groups(), style, area, row, buf);
            row = row.saturating_add(
                cell.wrapped(context.width().saturating_sub(marker.width(layout)))
                    .height(),
            );
        }
        let mut y = area.y.saturating_add(code_area_height);
        for block in self.outputs() {
            let height = block.height(context);
            let mut nested = Buffer::empty(Rect::new(0, 0, area.width, height));
            // Outputs draw no background of their own, so they start from the surface
            // this block has already painted beneath them.
            for row in 0..height {
                for x in 0..area.width {
                    nested[(x, row)] = buf[(area.x + x, y + row)].clone();
                }
            }
            block.render_local(context, &mut nested);
            paste(
                &nested,
                0..height,
                Rect::new(area.x, y, area.width, height),
                buf,
            );
            y = y.saturating_add(height);
        }
    }

    fn local_selection_exclusions(&self, context: &RenderContext, exclusions: &mut Exclusions) {
        let code_height = self.code_height(context);
        let code_area_height = code_height + self.output_margin();
        exclusions.push(Rect::new(0, 0, self.layout().prefix(), code_area_height));
        let mut y = code_area_height;
        for block in self.outputs() {
            let height = block.height(context);
            Block::selection_exclusions(
                block,
                context,
                Rect::new(0, y, context.width(), height),
                exclusions,
            );
            y = y.saturating_add(height);
        }
    }
}

/// Unnumbered text written by Spectral or by M2 during evaluation.
pub(crate) struct TextBlock {
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
        let text = text.into();
        Self {
            text: text.trim_end().to_string(),
            kind,
        }
    }

    /// Create a Spectral status or help message.
    pub(crate) fn system(text: impl Into<String>) -> Self {
        Self::new(text, TextKind::System)
    }

    /// Create text printed by evaluated M2 code.
    pub(crate) fn printed(text: impl Into<String>) -> Self {
        Self::new(text, TextKind::Printed)
    }

    fn body(&self, context: &RenderContext) -> Vec<String> {
        let width = match self.kind {
            TextKind::System => Bordered::text_width(context, FrameLayout::middle_third()),
            TextKind::Printed => context.width().saturating_sub(CellLayout::plain().prefix()),
        };
        let mut lines = word_lines(&self.text, width);
        if lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines
    }
}

impl Block for TextBlock {
    fn height(&self, context: &RenderContext) -> u16 {
        row_count!(self.body(context).len()).saturating_add(match self.kind {
            TextKind::System => 2,
            TextKind::Printed => 0,
        })
    }

    fn render_local(&self, context: &RenderContext, buf: &mut Buffer) {
        let area = *buf.area();
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
                    FrameLayout::middle_third(),
                    Style::new(),
                    theme.info_border(),
                    None,
                    body,
                )
                .render(&(), area, buf);
            }
            TextKind::Printed => {
                let prefix = usize::from(CellLayout::plain().prefix());
                let lines = self
                    .body(context)
                    .into_iter()
                    .map(|line| Line::from(vec![Span::raw(" ".repeat(prefix)), Span::raw(line)]))
                    .collect::<Vec<_>>();
                Paragraph::new(lines)
                    .style(theme.printed())
                    .render(area, buf);
            }
        }
    }
    fn local_selection_exclusions(&self, context: &RenderContext, exclusions: &mut Exclusions) {
        match self.kind {
            TextKind::System => Bordered::exclude_frame(
                Rect::new(0, 0, context.width(), self.height(context)),
                FrameLayout::middle_third(),
                exclusions,
            ),
            TextKind::Printed => exclusions.push(Rect::new(
                0,
                0,
                CellLayout::plain().prefix(),
                self.height(context),
            )),
        }
    }
}

/// One result inside a possibly shared value border.
struct OutputCell {
    number: Option<u32>,
    text: String,
    value: Option<M2Value>,
    ty: Option<String>,
}

/// Consecutive values returned by M2 inside one border.
pub(crate) struct OutputBlock {
    cells: Vec<OutputCell>,
}

impl OutputBlock {
    /// Create a rendered value and its optional type.
    pub(crate) fn value(text: String, ty: Option<String>) -> Self {
        Self {
            cells: vec![OutputCell {
                number: None,
                text: text.trim_end().to_string(),
                value: None,
                ty: ty.map(|ty| ty.trim_end().to_string()),
            }],
        }
    }

    /// Retain a structured M2 value for width-aware terminal rendering.
    pub(crate) fn rendered(value: M2Value, ty: Option<String>) -> Self {
        Self {
            cells: vec![OutputCell {
                number: None,
                text: value.render(),
                value: Some(value),
                ty,
            }],
        }
    }

    /// Attach the `oN` assigned by Macaulay2 to this value.
    pub(crate) fn numbered(mut self, number: u32) -> Self {
        self.cells[0].number = Some(number);
        self
    }

    fn append(&mut self, other: Self) {
        self.cells.extend(other.cells);
    }

    fn border_layout(&self) -> FrameLayout {
        self.cells
            .iter()
            .filter_map(|cell| cell.number)
            .max()
            .map_or_else(
                || CellLayout::plain().frame(),
                |number| {
                    let digits = if number == 0 {
                        1
                    } else {
                        number.ilog10() as u16 + 1
                    };
                    FrameLayout::gutter(
                        UI.cell_margin + digits + UI.cell_gap,
                        UI.cell_margin + digits - u16::from(number >= 10),
                    )
                },
            )
    }

    fn text_width(&self, context: &RenderContext) -> u16 {
        Bordered::text_width(context, self.border_layout())
    }

    fn layout(&self, cell: &OutputCell, context: &RenderContext) -> Vec<String> {
        let width = self.text_width(context);
        let text = cell
            .value
            .as_ref()
            .map_or_else(|| cell.text.clone(), |value| value.render_at(width.into()));
        if text.is_empty() {
            Vec::new()
        } else {
            match cell.value.as_ref() {
                Some(
                    M2Value::HashTable(_) | M2Value::Collection { .. } | M2Value::Markup { .. },
                ) => text.lines().map(str::to_string).collect(),
                _ => word_lines(&text, width),
            }
        }
    }

    fn value_lines(&self, cell: &OutputCell, context: &RenderContext) -> Vec<Line<'static>> {
        let theme = context.groups();
        // A value whose TeX parses is drawn from its own structure, so a numeral, a
        // name and an operator each take their own colour. Anything else is flat text
        // and keeps the single value colour it always had.
        let width = usize::from(self.text_width(context));
        let fractions = context.fractions();
        let lines = match cell
            .value
            .as_ref()
            .map(|value| maths::spans(value, width, fractions))
        {
            Some(Ok(rows)) => rows
                .into_iter()
                .map(|row| Line::from(row.spans(theme)))
                .collect(),
            _ => self
                .layout(cell, context)
                .into_iter()
                .map(|line| Line::from(Span::styled(line, theme.value())))
                .collect(),
        };
        match context.output_limit() {
            Some(limit) => Self::fold(lines, limit, context),
            None => lines,
        }
    }

    fn fold(
        mut lines: Vec<Line<'static>>,
        limit: usize,
        context: &RenderContext,
    ) -> Vec<Line<'static>> {
        match lines.len() <= limit {
            true => lines,
            false => {
                let tail = limit.saturating_sub(1) / 2;
                let head = limit.saturating_sub(tail + 1);
                let hidden = lines.len() - head - tail;
                let mut ending = lines.split_off(lines.len() - tail);
                lines.truncate(head);
                lines.push(
                    Line::from(Span::styled(
                        format!("⋯ {hidden} rows hidden · output view shows all"),
                        context.groups().muted(),
                    ))
                    .alignment(Alignment::Center),
                );
                lines.append(&mut ending);
                lines
            }
        }
    }

    fn type_lines(&self, cell: &OutputCell, context: &RenderContext) -> Vec<Line<'static>> {
        cell.ty.as_deref().map_or_else(Vec::new, |ty| {
            word_lines(ty, self.text_width(context))
                .into_iter()
                .map(|line| Line::from(Span::styled(line, context.groups().type_info())))
                .collect()
        })
    }

    fn type_start(values: &[Line<'_>], type_width: usize, width: usize) -> usize {
        match values.last() {
            Some(value) if value.width() + type_width < width => values.len() - 1,
            _ => values.len(),
        }
    }

    fn lines(&self, cell: &OutputCell, context: &RenderContext) -> Vec<Line<'static>> {
        let mut lines = self.value_lines(cell, context);
        let types = self.type_lines(cell, context);
        let type_width = types.iter().map(Line::width).max().unwrap_or_default();
        if types.is_empty() {
            return lines;
        }
        if lines.is_empty() {
            lines.push(Line::default());
        }
        let width = usize::from(self.text_width(context));
        let start = Self::type_start(&lines, type_width, width);
        lines.resize(start + types.len(), Line::default());
        for (line, ty) in lines[start..].iter_mut().zip(types) {
            line.spans.push(Span::raw(
                " ".repeat(width.saturating_sub(line.width() + type_width)),
            ));
            line.spans.extend(ty.spans);
        }
        lines
    }

    fn gap(index: usize) -> u16 {
        u16::from(index > 0)
    }

    fn body(&self, context: &RenderContext) -> Vec<Line<'static>> {
        let mut body = Vec::new();
        for (index, cell) in self.cells.iter().enumerate() {
            body.extend((index > 0).then(Line::default));
            body.extend(self.lines(cell, context));
        }
        body
    }
}

impl Block for OutputBlock {
    fn height(&self, context: &RenderContext) -> u16 {
        row_count!(self.body(context).len()).saturating_add(2)
    }

    fn render_local(&self, context: &RenderContext, buf: &mut Buffer) {
        let area = *buf.area();
        let theme = context.groups();
        let layout = self.border_layout();
        Bordered::new(
            layout,
            Style::new(),
            theme.border(),
            None,
            self.body(context),
        )
        .render(&(), area, buf);
        let mut row = Bordered::inner(area, layout).y;
        for (index, cell) in self.cells.iter().enumerate() {
            row = row.saturating_add(Self::gap(index));
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
            row = row.saturating_add(row_count!(self.lines(cell, context).len()));
        }
    }
    fn local_selection_exclusions(&self, context: &RenderContext, exclusions: &mut Exclusions) {
        let area = Rect::new(0, 0, context.width(), self.height(context));
        let layout = self.border_layout();
        Bordered::exclude_frame(area, layout, exclusions);
        let inner = Bordered::inner(area, layout);
        let right = inner.right();
        let mut row = inner.y;
        for (index, cell) in self.cells.iter().enumerate() {
            row = row.saturating_add(Self::gap(index));
            let lines = self.lines(cell, context);
            exclusions.push(Rect::new(
                area.x,
                row,
                layout.prefix(),
                row_count!(lines.len()),
            ));
            let values = self.value_lines(cell, context);
            let types = self.type_lines(cell, context);
            let width = types.iter().map(Line::width).max().unwrap_or_default() as u16;
            let start = Self::type_start(&values, width.into(), inner.width.into()) as u16;
            for offset in 0..row_count!(types.len()) {
                exclusions.push(Rect::new(
                    right.saturating_sub(width),
                    row + start + offset,
                    width,
                    1,
                ));
            }
            row = row.saturating_add(row_count!(lines.len()));
        }
    }
}

/// A warning emitted while evaluating M2 code.
pub(crate) struct WarningBlock {
    pub(crate) text: String,
}

impl Block for WarningBlock {
    fn height(&self, context: &RenderContext) -> u16 {
        row_count!(word_lines(
            &self.text,
            Bordered::text_width(context, CellLayout::plain().frame()),
        )
        .len())
        .saturating_add(2)
    }

    fn render_local(&self, context: &RenderContext, buf: &mut Buffer) {
        let area = *buf.area();
        let theme = context.groups();
        let body = word_lines(
            &self.text,
            Bordered::text_width(context, CellLayout::plain().frame()),
        )
        .into_iter()
        .map(|line| Line::from(Span::styled(line, theme.warning())))
        .collect();
        let title = Some(Line::from(Span::styled(" Warning ", theme.warning())));
        Bordered::new(
            CellLayout::plain().frame(),
            Style::new(),
            theme.warning_border(),
            title,
            body,
        )
        .render(&(), area, buf);
    }

    fn local_selection_exclusions(&self, context: &RenderContext, exclusions: &mut Exclusions) {
        Bordered::exclude_frame(
            Rect::new(0, 0, context.width(), self.height(context)),
            CellLayout::plain().frame(),
            exclusions,
        );
    }
}

fn word_lines(text: &str, width: u16) -> Vec<String> {
    WrappedText::word(text.trim_end(), width)
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
        Bordered::text_width(context, CellLayout::plain().frame()),
    )
}

fn render_error_box(message: &str, context: &RenderContext, area: Rect, buf: &mut Buffer) {
    let theme = context.groups();
    let body = error_box_text(message, context)
        .into_iter()
        .map(|line| Line::from(Span::styled(line, theme.error())))
        .collect();
    Bordered::new(
        CellLayout::plain().frame(),
        Style::new(),
        theme.error_border(),
        None,
        body,
    )
    .render(&(), area, buf);
}

/// An evaluation error whose submitted code is already present in the feed.
pub(crate) struct ErrorMessageBlock {
    pub(crate) message: String,
}

impl ErrorMessageBlock {
    pub(crate) fn new(message: String) -> Self {
        Self { message }
    }
}

impl Block for ErrorMessageBlock {
    fn height(&self, context: &RenderContext) -> u16 {
        row_count!(error_box_text(&self.message, context).len()).saturating_add(2)
    }

    fn render_local(&self, context: &RenderContext, buf: &mut Buffer) {
        let area = *buf.area();
        render_error_box(&self.message, context, area, buf);
    }

    fn local_selection_exclusions(&self, context: &RenderContext, exclusions: &mut Exclusions) {
        Bordered::exclude_frame(
            Rect::new(0, 0, context.width(), self.height(context)),
            CellLayout::plain().frame(),
            exclusions,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::highlight::Highlighter;
    use crate::ui::controller::settings::theme::groups::{Groups, HLGroup};
    use crate::ui::syntax_tree::SyntaxTreeBlock;
    use m2_syn::{DelimiterKind, DoubleSpan, PrettyNode, Span as M2Span};

    fn rendered_buffer(block: &dyn Block, width: u16) -> Buffer {
        let context = RenderContext::new(width, 0);
        let area = Rect::new(0, 0, width, block.height(&context));
        let mut buffer = Buffer::empty(area);
        block.render_local(&context, &mut buffer);
        buffer
    }

    fn rendered_rows(block: &dyn Block, width: u16) -> Vec<String> {
        let buffer = rendered_buffer(block, width);
        let area = *buffer.area();
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn incomplete_source_has_an_inset_on_a_lighter_surface() {
        let groups = Groups::default();
        let block = IncompleteBlock::new("f(\nx\n".into(), Vec::new());
        let rows = rendered_rows(&block, 50);

        assert!(rows.iter().all(|row| !row.contains('│')));
        assert_eq!(rows[1].find("f("), Some(4));
        assert_eq!(rows[2].find('x'), Some(4));
        assert!(rows[0].chars().all(|character| character == '┈'));
        assert!(!rows[1..].iter().any(|row| row.contains('┈')));
        let buffer = rendered_buffer(&block, 50);
        assert_eq!(Some(buffer[(0, 1)].bg), groups.pending().bg);
        assert_eq!(Some(buffer[(0, 0)].fg), groups.muted().fg);
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
        let context = RenderContext::new(60, 0);
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
        let context = RenderContext::new(100, 0);
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
        let buffer = rendered_buffer(&syntax_tree_fixture(), width);
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
        let context = RenderContext::new(40, 0);
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
        let context = RenderContext::new(40, 0);
        let source = (0..20).map(|_| "long").collect::<Vec<_>>().join(" \n");
        let tree = PrettyNode::syntax("Root", Some(M2Span::for_text(source.clone())));
        let block = SyntaxTreeBlock::new(source, Vec::new(), tree);
        let line = &block.tree_lines(&context)[0];

        assert!(line.width() <= usize::from(context.width()));
        assert!(line.spans.iter().any(|span| span.content.contains('…')));
    }

    #[test]
    fn code_and_output_reserve_the_cell_marker_margin() {
        let width = 40;
        let input = CodeBlock::new([Cell::input("x".to_string(), Vec::new())]);
        let output = OutputBlock::value("x".to_string(), None);

        let input = rendered_rows(&input, width);
        let output = rendered_rows(&output, width);
        assert_eq!(
            input[1].chars().position(|character| character == 'x'),
            Some(4)
        );
        assert_eq!(output[0].chars().nth(2), Some('╭'));
    }

    #[test]
    fn successful_cell_and_output_numbers_align_with_distinct_emphasis() {
        use ratatui::style::Modifier;

        let groups = Groups::default();
        let input = CodeBlock::new([Cell::resolved("x".into(), 7, CellState::Value, Vec::new())]);
        let output = OutputBlock::value("x".into(), None).numbered(7);
        let input = rendered_buffer(&input, 20);
        let output = rendered_buffer(&output, 20);

        assert_eq!(input[(1, 1)].symbol(), "7");
        assert_eq!(output[(1, 1)].symbol(), "o");
        assert_eq!(output[(2, 1)].symbol(), "7");
        assert_eq!(output[(2, 0)].symbol(), "╭");
        assert_eq!(output[(2, 2)].symbol(), "╰");
        assert_eq!(input[(1, 1)].fg, groups.muted().fg.unwrap());
        assert_eq!(output[(2, 1)].fg, groups.output_number().fg.unwrap());
        assert_eq!(output[(2, 1)].fg, groups.prompt().fg.unwrap());
        assert!(!input[(1, 1)].modifier.contains(Modifier::BOLD));
        assert!(output[(2, 1)].modifier.contains(Modifier::BOLD));
        assert_eq!(input[(2, 0)].symbol(), "│");
        assert_eq!(input[(2, 0)].fg, groups.separator().fg.unwrap());

        let null = rendered_buffer(
            &CodeBlock::new([Cell::resolved("x".into(), 7, CellState::Null, Vec::new())]),
            20,
        );
        assert_eq!(null[(1, 1)].fg, groups.muted().fg.unwrap());
    }

    #[test]
    fn multi_digit_output_opens_one_cell_in_the_shared_border() {
        let output = OutputBlock::value("a\nb".into(), None).numbered(12);
        let output = rendered_buffer(&output, 20);

        assert_eq!(output[(2, 0)].symbol(), "╭");
        assert_eq!(output[(1, 1)].symbol(), "o");
        assert_eq!(output[(2, 1)].symbol(), "1");
        assert_eq!(output[(3, 1)].symbol(), "2");
        assert_eq!(output[(2, 2)].symbol(), "│");
        assert_eq!(output[(2, 3)].symbol(), "╰");
    }

    #[test]
    fn hard_wrapped_code_keeps_its_prefix() {
        let input = CodeBlock::new([Cell::input("abcdefgh".to_string(), Vec::new())]);

        let rows = rendered_rows(&input, 8);
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
            EvaluationBlock::new(1, ["a", "b"].map(|text| (text.to_string(), Vec::new())));
        submission.succeed(0, 9, CellState::Value, Vec::new(), None);
        submission.succeed(1, 10, CellState::Value, Vec::new(), None);

        let rows = rendered_rows(&submission, 20);
        assert_eq!(rows[0].chars().nth(3), Some('│'));
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
        assert_eq!(rows.last().unwrap().chars().nth(3), Some('│'));
    }

    #[test]
    fn printed_text_is_one_compact_inset_row() {
        let rows = rendered_rows(&TextBlock::printed("text"), 20);

        assert_eq!(rows.len(), 1);
        assert!(rows.iter().all(|row| !row.contains('│')));
        assert_eq!(
            rows[0].chars().position(|character| character == 't'),
            Some(4)
        );
    }

    #[test]
    fn blocks_drop_only_terminal_whitespace() {
        let text = TextBlock::printed("  text  \n\n");
        let value = OutputBlock::value("  value  \n\n".into(), Some("  Type  \n".into()));

        assert_eq!(text.text, "  text");
        assert_eq!(value.cells[0].text, "  value");
        assert_eq!(value.cells[0].ty.as_deref(), Some("  Type"));
        assert_eq!(text.height(&RenderContext::new(40, 0)), 1);
        assert_eq!(value.height(&RenderContext::new(40, 0)), 3);
    }

    #[test]
    fn wrapping_paints_each_source_range() {
        let cell = Cell::input(
            "abcdefgh".to_string(),
            vec![HLSpan::new(0..8, HLGroup::Variable)],
        );
        let wrapped = cell.wrapped(3);
        let rows: Vec<_> = wrapped
            .source_lines()
            .map(|(range, _)| {
                cell.spans.paint(
                    range.start,
                    &cell.text[range.clone()],
                    Style::new(),
                    &Groups::default(),
                )[0]
                .content
                .to_string()
            })
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
        let rows = rendered_rows(&code, 20);
        assert_eq!(
            rows[1].chars().position(|character| character == 'x'),
            Some(4)
        );
        assert_eq!(
            rows[3].chars().position(|character| character == 'y'),
            Some(4)
        );
        assert!(rows[2]
            .chars()
            .all(|character| character == ' ' || character == '│'));
    }

    #[test]
    fn submitted_cells_advance_through_the_queue_in_place() {
        let mut submission = EvaluationBlock::new(
            7,
            [
                ("a".to_string(), Vec::new()),
                ("b".to_string(), Vec::new()),
                ("c".to_string(), Vec::new()),
            ],
        );
        submission.succeed(0, 1, CellState::Value, Vec::new(), None);
        assert!(matches!(
            submission.cells[0].1,
            EvaluationState::Succeeded(1, CellState::Value)
        ));
        assert!(matches!(submission.cells[1].1, EvaluationState::Pending));
        submission.fail(1, 2, Vec::new(), "bad cell".to_string());
        assert!(matches!(submission.cells[1].1, EvaluationState::Failed(2)));
        assert!(matches!(submission.cells[2].1, EvaluationState::Queued));
        submission.start();
        assert!(matches!(submission.cells[2].1, EvaluationState::Pending));
    }

    #[test]
    fn non_values_do_not_add_output_cells() {
        let mut submission = EvaluationBlock::new(
            1,
            ["a", "b", "c"].map(|text| (text.to_string(), Vec::new())),
        );
        submission.succeed(
            0,
            1,
            CellState::Value,
            Vec::new(),
            Some(OutputBlock::value("a".to_string(), None).numbered(1)),
        );
        submission.succeed(1, 2, CellState::Muted, Vec::new(), None);
        submission.succeed(2, 3, CellState::Null, Vec::new(), None);

        assert_eq!(submission.outputs.len(), 1);
        let EvaluationOutput::Value(value) = &submission.outputs[0] else {
            panic!("expected a value output");
        };
        assert_eq!(value.cells.len(), 1);
    }

    #[test]
    fn output_keeps_the_code_margin_and_consecutive_values_share_one_border() {
        let mut submission =
            EvaluationBlock::new(1, ["a", "b"].map(|text| (text.to_string(), Vec::new())));
        for (index, number) in [1, 2].into_iter().enumerate() {
            submission.succeed(
                index,
                number,
                CellState::Value,
                Vec::new(),
                Some(OutputBlock::value(number.to_string(), None).numbered(number)),
            );
        }

        let groups = Groups::default();
        let rows = rendered_rows(&submission, 40);
        assert_eq!(submission.outputs.len(), 1);
        assert!(rows[3]
            .chars()
            .all(|character| character == ' ' || character == '│'));
        assert_eq!(
            rendered_buffer(&submission, 40)[(10, 3)].bg,
            groups.code().bg.unwrap()
        );
        assert!(rows[5].contains('1'), "{rows:?}");
        assert!(rows[6]
            .chars()
            .all(|character| character == ' ' || character == '│'));
        assert!(rows[7].contains('2'), "{rows:?}");
        assert_eq!(rows.iter().filter(|row| row.contains('╭')).count(), 1);
        assert_eq!(rows.iter().filter(|row| row.contains('╰')).count(), 1);
    }

    #[test]
    fn one_line_output_keeps_its_value_and_type_inside_the_border() {
        let width = 40;
        let block = OutputBlock::value("x + y".to_string(), Some("R".to_string()));

        let rows = rendered_rows(&block, width);
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
        let block = OutputBlock::value("abc".to_string(), None);

        let rows = rendered_rows(&block, 20);
        let input = rendered_rows(
            &CodeBlock::new([Cell::input("x".to_string(), Vec::new())]),
            20,
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
        let context = RenderContext::new(20, 0);
        let boxes: [Box<dyn Block>; 3] = [
            Box::new(OutputBlock::value("x".into(), None)),
            Box::new(WarningBlock { text: "x".into() }),
            Box::new(ErrorMessageBlock::new("x".into())),
        ];

        for block in boxes {
            let buffer = rendered_buffer(block.as_ref(), 20);
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
            let buffer = rendered_buffer(block.as_ref(), 20);
            let cell = &buffer[(5, 1)];
            assert_eq!(cell.fg, text.fg.unwrap());
            assert_eq!(cell.bg, ratatui::style::Color::Reset);
        }
    }

    #[test]
    fn multiline_output_keeps_the_type_beside_its_last_value_row() {
        let block = OutputBlock::value("x\ny".to_string(), Some("R".to_string()));

        let rows = rendered_rows(&block, 40);
        assert_eq!(rows.len(), 4);
        assert!(rows[1].contains('x'));
        assert!(rows[2].contains('y') && rows[2].contains('R'));
    }

    #[test]
    fn long_output_keeps_its_beginning_and_end_until_opened() {
        let context = RenderContext::new(40, 0);
        let block = OutputBlock::value(
            (0..40)
                .map(|row| format!("row {row:02}"))
                .collect::<Vec<_>>()
                .join("\n"),
            None,
        );
        let text = |lines: Vec<Line<'_>>| {
            lines
                .into_iter()
                .map(|line| {
                    line.spans
                        .into_iter()
                        .map(|span| span.content)
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
        };

        let folded = text(block.value_lines(&block.cells[0], &context));
        assert_eq!(folded.len(), UI.max_output_rows);
        assert!(folded.first().unwrap().contains("row 00"));
        assert!(folded.last().unwrap().contains("row 39"));
        assert!(folded.iter().any(|line| line.contains("17 rows hidden")));

        let full = text(block.value_lines(&block.cells[0], &context.full_output()));
        assert_eq!(full.len(), 40);
        assert!(!full.iter().any(|line| line.contains("rows hidden")));
    }

    #[test]
    fn multiline_type_keeps_its_net_shape_beside_the_value() {
        let block = OutputBlock::value("2".to_string(), Some("ZZ[x]\n-----\nx - 1".to_string()));

        let rows = rendered_rows(&block, 40);
        assert_eq!(rows.len(), 5);
        assert!(rows[1].contains('2') && rows[1].contains("ZZ[x]"));
        assert!(rows[2].contains("-----"));
        assert!(rows[3].contains("x - 1"));
        assert_eq!(rows[1].find("ZZ[x]"), rows[2].find("-----"));
        assert_eq!(rows[2].find("-----"), rows[3].find("x - 1"));
    }

    #[test]
    fn overflowing_type_moves_below_a_full_value_row() {
        let width = 20;
        let context = RenderContext::new(width, 0);
        let width = OutputBlock::value(String::new(), None).text_width(&context);
        let block = OutputBlock::value("x".repeat(width.into()), Some("R".into()));

        let rows = rendered_rows(&block, 20);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1].matches('x').count(), usize::from(width));
        assert!(!rows[1].contains('R'));
        assert!(rows[2].contains('R'));
    }

    #[test]
    fn a_result_shows_its_type() {
        let block = OutputBlock::value("x".to_string(), Some("R".to_string()));

        let rows = rendered_rows(&block, 40);
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
        let context = RenderContext::new(80, 0);
        let block = TextBlock::system("commands:\n  /settings\n  /code");
        assert_eq!(block.height(&context), 5);
    }

    #[test]
    fn system_block_is_a_centered_info_box() {
        let theme = Groups::default();
        let buffer = rendered_buffer(&TextBlock::system("message"), 60);

        assert_eq!(buffer[(20, 0)].symbol(), "╭");
        assert_eq!(buffer[(40, 0)].symbol(), "╮");
        assert_eq!(buffer[(27, 1)].symbol(), "m");
        assert_eq!(buffer[(27, 1)].fg, theme.info().fg.unwrap());
    }
}
