//! Typed cells and messages rendered in the editor feed.
//!
//! Every [`Block`] owns text, measures and renders against a borrowed
//! [`RenderContext`], and inherits the same visual-line wrapper. Highlighted text
//! receives its highlighter separately because highlighting is an application-wide
//! service rather than editor state.

use crate::ui::glyphs::{Charset, Glyphs, Remark, Spelled};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::ops::Range;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::{iter, ptr};

use ratatui::buffer::Buffer;
use ratatui::layout::{Offset, Position, Rect, Size};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use super::block::{Block, CellPoint, Element, HLSpan, Link, Text};
use super::element::paste;
use super::fold::{FoldRanges, LineFolds};
use super::graphics::cell_size;
use super::graphics::{self, kitty::ImageId};
use super::item::{Item, ItemKind};
use super::present;
use super::relayout::{self, Pending};
use super::render::maths::MathDisplay;
use super::render::selection::Inclusions;
use super::render::{markup, maths, spin, typeset};
use super::render::{Bordered, FrameLayout, Highlights, RenderContext};
use super::settings::rendering::RendererPosition;
use super::wrap::{offset_at_column, SpanWrap, WrapPolicy, WrappedText};
use crate::backend::rod::Shape;
use crate::backend::transcript::{captured_error, captured_output, strip_m2_location};
use crate::backend::{M2Response, PreviewMode, Rod, ValuePreview};
use crate::code::highlight::{Highlighter, SyntaxHighlighter};
use crate::code::semantic::SourceId;
use crate::ui::config::UI;
use crate::ui::render::maths::MathStyle;
use crate::ui::theme::groups::Groups;

/// A source file's gutter is as wide as its last line number times this: two
/// columns more, for a space and the fold marker after the number.
const LINE_NUMBER_ROOM: u32 = 100;
/// Stands in for a value a worker is still laying out, or for its text before a
/// worker has drawn it.
const LAYING_OUT: Remark = Remark("laying out");
/// Stands in for the rows a preview leaves out, over the render limit.
const PREVIEW_TRUNCATED: Remark = Remark("output preview truncated (render limit)");

macro_rules! row_count {
    ($rows:expr) => {
        u16::try_from($rows).unwrap_or(u16::MAX)
    };
}

/// How a successful input cell's result appears in the transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellState {
    Value,
    Muted,
    Null,
}

impl CellState {
    /// How the cell M2 answered with `response` appears: its value shown, bound
    /// but suppressed, or nothing bound at all.
    pub fn of(response: &M2Response) -> Self {
        match (response.has_value, response.value.is_some()) {
            (true, true) => Self::Value,
            (true, false) => Self::Muted,
            _ => Self::Null,
        }
    }
}

/// The blocks a successful `response` to `code` adds after it: warnings, the
/// text it printed, and its value, not yet numbered.
pub fn response_blocks(
    response: &M2Response,
    code: &str,
) -> (Vec<Box<dyn Block>>, Option<TextBlock>, Option<OutputBlock>) {
    // Capture extraction only needs value presence. Formatting it here would
    // duplicate the output block's work before its preview budget can apply.
    let value = response
        .value
        .as_ref()
        .map(|value| value.text().unwrap_or(""));
    let output = captured_output(
        &response.transcript,
        code,
        value,
        response.type_info.as_deref(),
    );
    let warnings = warning_blocks(output.warnings);
    let printed = (!output.printed.is_empty()).then(|| TextBlock::printed(output.printed));
    let value = match (response.value.clone(), output.value, response.has_value) {
        (Some(value), Some(rendered), true) => Some(
            OutputBlock::rendered(value, rendered.type_info)
                .with_previews(response.previews.clone())
                .annotated(response.type_drawing.clone())
                .drawing_text(),
        ),
        _ => None,
    };
    (warnings, printed, value)
}

/// The blocks a failed `response` to `code` adds after it: warnings raised before
/// the error, the text printed before it, and the error's own message.
pub fn error_blocks(
    response: &M2Response,
    code: &str,
) -> (Vec<Box<dyn Block>>, Option<TextBlock>, String) {
    let error = captured_error(&response.transcript, code);
    let printed = (!error.printed.is_empty()).then(|| TextBlock::printed(error.printed));
    (warning_blocks(error.warnings), printed, error.diagnostic)
}

fn warning_blocks(warnings: Vec<String>) -> Vec<Box<dyn Block>> {
    warnings
        .into_iter()
        .map(|text| Box::new(WarningBlock { text }) as Box<dyn Block>)
        .collect()
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
    /// The digits laid out for `number`: its own, and never fewer than every
    /// number is given room for.
    const fn digits(number: u32) -> u16 {
        let own = if number == 0 {
            1
        } else {
            number.ilog10() as u16 + 1
        };
        if own > UI::MIN_NUMBER_DIGITS {
            own
        } else {
            UI::MIN_NUMBER_DIGITS
        }
    }

    const fn numbered(number: u32) -> Self {
        let digits = Self::digits(number);
        Self(FrameLayout::gutter(
            UI::CELL_MARGIN + digits + UI::CELL_GAP,
            UI::CELL_MARGIN + digits,
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
            buf[(x, y)]
                .set_symbol(Glyphs::CellSeparator.current())
                .set_style(style);
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
        match self {
            Self::Bare => 0,
            _ => layout.prefix(),
        }
    }

    fn label(self, context: &RenderContext) -> Option<(String, Style)> {
        let groups = context.groups();
        Some(match self {
            // The numbers of the cells behind the latest outputs wear the colour of
            // the labels those outputs carry.
            Self::Number(number, state) => (
                number.to_string(),
                match state {
                    CellState::Value | CellState::Muted | CellState::Null
                        if context.is_recent_output(number) =>
                    {
                        groups.recent_cell_number()
                    }
                    CellState::Value | CellState::Muted | CellState::Null => groups.cell_number(),
                },
            ),
            Self::Pending => {
                let frame = spin::clock().current_frame();
                (frame.content.into(), frame.style(groups))
            }
            Self::Queued => (
                Glyphs::UnevaluatedCell.current().into(),
                groups.queued_cell_marker(),
            ),
            Self::Failed(number) => (number.to_string(), groups.failed_cell_marker()),
            Self::Invalid => (
                Glyphs::RejectedCell.current().into(),
                groups.rejected_cell_marker(),
            ),
            Self::Bare | Self::Empty => return None,
        })
    }

    fn span(self, layout: CellLayout, base: Style) -> Span<'static> {
        Span::styled(" ".repeat(self.width(layout).into()), base)
    }

    fn render(self, context: &RenderContext, base: Style, area: Rect, row: u16, buf: &mut Buffer) {
        if let Some((label, style)) = self.label(context) {
            let width = label.chars().count() as u16;
            Paragraph::new(Span::styled(label, style.patch(base)))
                .render(Rect::new(area.x + UI::CELL_MARGIN, row, width, 1), buf);
        }
    }
}

/// One lexical Macaulay2 input cell and the byte ranges highlighted within it.
pub struct M2CodeCell {
    text: String,
    number: Option<(u32, CellState)>,
    spans: Vec<HLSpan>,
    wrap_ranges: Vec<Range<usize>>,
}

impl M2CodeCell {
    pub(super) fn new(text: String, number: Option<(u32, CellState)>, spans: Vec<HLSpan>) -> Self {
        let wrap_ranges = spans.iter().map(|span| span.content.clone()).collect();
        Self {
            text,
            number,
            spans,
            wrap_ranges,
        }
    }

    /// Highlight identified source while keeping wrapping tied to lexical boundaries.
    pub fn highlighted(
        text: String,
        number: Option<(u32, CellState)>,
        id: SourceId,
        highlighter: &mut dyn Highlighter,
    ) -> Self {
        let lexical = highlighter.highlight(&text);
        let mut cell = Self::new(text, number, lexical);
        cell.rehighlight(id, highlighter);
        cell
    }

    /// Create an unnumbered cell that was not submitted to Macaulay2.
    fn rejected(text: String, spans: Vec<HLSpan>) -> Self {
        Self::new(text, None, spans)
    }

    fn rehighlight(&mut self, id: SourceId, highlighter: &mut dyn Highlighter) -> bool {
        let spans = highlighter.highlight_source(id, &self.text);
        if self.spans == spans {
            return false;
        }
        self.spans = spans;
        true
    }

    fn wrapped(&self, width: u16, mode: crate::ui::wrap::WrapMode) -> WrappedText {
        WrappedText::code(&self.text, width, &self.wrap_ranges, mode)
    }

    fn marker(&self) -> CellMarker {
        self.number.map_or(CellMarker::Empty, |(number, state)| {
            CellMarker::Number(number, state)
        })
    }
}

/// One feed block containing one or more lexical input cells.
pub struct CodeBlock {
    cells: Vec<M2CodeCell>,
    /// The widest number the gutter makes room for, beyond its cells' own.
    widest_number: u32,
}

/// A source document with semantic colours and an optional referenced fragment,
/// its foldable stretches folded to their first line on a double click beside it.
pub struct SourceBlock {
    code: RefCell<CodeBlock>,
    page: u64,
    focus: Option<Range<usize>>,
    folds: LineFolds,
}

/// One row of a source's full layout: the line it shows and the part of the text on it.
struct SourceRow {
    line: u32,
    text: Range<usize>,
}

impl SourceBlock {
    pub fn new(source: String, page: u64, focus: Option<Range<usize>>) -> Self {
        let lines = source.lines().count().max(1) as u32;
        let cell = M2CodeCell::highlighted(
            source,
            None,
            SourceId::Page { page, index: 0 },
            &mut SyntaxHighlighter::new(),
        );
        let mut code = CodeBlock::new([cell]);
        // Room for the last line's number, a space, and the fold marker.
        code.widest_number = lines.saturating_mul(LINE_NUMBER_ROOM);
        Self {
            code: RefCell::new(code),
            page,
            focus,
            folds: LineFolds::default(),
        }
    }

    /// Fold the stretches in `ranges`, which may be learned only later.
    pub fn with_folds(mut self, ranges: FoldRanges) -> Self {
        self.folds = LineFolds::new(ranges);
        self
    }

    fn focused_cells(&self, context: &RenderContext) -> Vec<Rect> {
        let Some(focus) = &self.focus else {
            return Vec::new();
        };
        let code = self.code.borrow();
        let cell = &code.cells[0];
        let gutter = cell.marker().width(code.layout());
        cell.wrapped(context.width().saturating_sub(gutter), context.code_wrap())
            .source_lines()
            .enumerate()
            .filter_map(|(row, (range, _))| {
                let start = range.start.max(focus.start);
                let end = range.end.min(focus.end);
                (start < end).then(|| {
                    Rect::new(
                        gutter + cell.text[range.start..start].width() as u16,
                        row as u16 + 1,
                        cell.text[start..end].width() as u16,
                        1,
                    )
                })
            })
            .collect()
    }

    /// Every row of the layout with nothing folded, none for the padding around the code.
    fn rows(&self, context: &RenderContext) -> Vec<Option<SourceRow>> {
        let code = self.code.borrow();
        let cell = &code.cells[0];
        let gutter = cell.marker().width(code.layout());
        let mut rows = vec![None];
        rows.extend(
            cell.wrapped(context.width().saturating_sub(gutter), context.code_wrap())
                .source_lines()
                .map(|(range, _)| {
                    Some(SourceRow {
                        line: cell.text[..range.start].matches('\n').count() as u32,
                        text: range.clone(),
                    })
                }),
        );
        rows.resize_with(usize::from(code.height(context)), || None);
        rows
    }

    /// Where each row of the unfolded layout is shown, none when it is folded away.
    fn shown_rows(&self, rows: &[Option<SourceRow>]) -> Vec<Option<u16>> {
        let mut next = 0;
        rows.iter()
            .map(|row| {
                let hidden = row.as_ref().is_some_and(|row| self.folds.hides(row.line));
                (!hidden).then(|| {
                    next += 1;
                    next - 1
                })
            })
            .collect()
    }

    /// The rows that begin a line, by their row in the unfolded layout.
    fn line_starts(rows: &[Option<SourceRow>]) -> impl Iterator<Item = (usize, &SourceRow)> {
        rows.iter().enumerate().filter_map(|(index, row)| {
            let row = row.as_ref()?;
            let previous = index
                .checked_sub(1)
                .and_then(|previous| rows[previous].as_ref());
            previous
                .is_none_or(|previous| previous.line != row.line)
                .then_some((index, row))
        })
    }

    fn gutter(&self) -> (u16, u16) {
        let code = self.code.borrow();
        let layout = code.layout();
        (layout.frame().inset(), code.cells[0].marker().width(layout))
    }
}

impl Block for SourceBlock {
    fn height(&self, context: &RenderContext) -> u16 {
        let rows = self.rows(context);
        self.shown_rows(&rows).iter().flatten().count() as u16
    }

    fn focus_row(&self, context: &RenderContext) -> Option<u16> {
        let row = self.focused_cells(context).first()?.y;
        let rows = self.rows(context);
        self.shown_rows(&rows)
            .get(usize::from(row))
            .copied()
            .flatten()
    }

    fn render_local(&self, context: &RenderContext, buffer: &mut Buffer) {
        let area = *buffer.area();
        let rows = self.rows(context);
        let mut full = Buffer::empty(Rect::new(area.x, area.y, area.width, rows.len() as u16));
        {
            let mut code = self.code.borrow_mut();
            let cell = &mut code.cells[0];
            cell.spans = markup::source_highlights(self.page, &cell.text);
            code.render_local(context, &mut full);
        }
        if let Some(background) = context.groups().selection().bg {
            for cells in self.focused_cells(context) {
                for position in (cells + Offset::new(area.x as i32, area.y as i32))
                    .intersection(*full.area())
                    .positions()
                {
                    full[position].set_bg(background);
                }
            }
        }
        let shown = self.shown_rows(&rows);
        for (row, y) in shown.iter().enumerate() {
            let Some(y) = y.filter(|y| *y < area.height) else {
                continue;
            };
            for x in area.left()..area.right() {
                buffer[(x, area.y + y)] = full[(x, area.y + row as u16)].clone();
            }
        }
        let (separator, gutter) = self.gutter();
        let style = context.groups().fold_marker();
        let number_style = context.groups().cell_number();
        let number_width = usize::from(separator.saturating_sub(UI::CELL_MARGIN + 2));
        let code = self.code.borrow();
        for (index, row) in Self::line_starts(&rows) {
            let Some(y) = shown[index]
                .map(|y| area.y + y)
                .filter(|y| *y < area.bottom())
            else {
                continue;
            };
            buffer.set_string(
                area.x + UI::CELL_MARGIN,
                y,
                format!("{:>number_width$}", row.line + 1),
                number_style,
            );
            let Some(range) = self.folds.starting_at(row.line) else {
                continue;
            };
            let folded = self.folds.is_folded(row.line);
            let glyph = match folded {
                true => Glyphs::FoldClosed,
                false => Glyphs::FoldOpen,
            };
            buffer.set_string(
                area.x + separator.saturating_sub(1),
                y,
                glyph.current(),
                style,
            );
            if folded {
                let x = area.x + gutter + code.cells[0].text[row.text.clone()].width() as u16 + 1;
                let hidden = range.end() - range.start();
                buffer.set_stringn(
                    x,
                    y,
                    format!("{} {hidden} lines", Glyphs::FoldedLines.current()),
                    usize::from(area.right().saturating_sub(x)),
                    style,
                );
            }
        }
    }

    fn toggle_fold(&mut self, position: Position, context: &RenderContext) -> bool {
        let (_, gutter) = self.gutter();
        if position.x >= gutter {
            return false;
        }
        let rows = self.rows(context);
        let shown = self.shown_rows(&rows);
        let Some(row) = shown
            .iter()
            .position(|y| *y == Some(position.y))
            .and_then(|row| rows[row].as_ref())
        else {
            return false;
        };
        self.folds.toggle(row.line)
    }

    fn local_selection_inclusions(&self, context: &RenderContext, inclusions: &mut Inclusions) {
        let mut unfolded = Inclusions::default();
        self.code
            .borrow()
            .local_selection_inclusions(context, &mut unfolded);
        let shown = self.shown_rows(&self.rows(context));
        *inclusions += unfolded.folded(|row| shown.get(usize::from(row)).copied().flatten());
    }
}

impl CodeBlock {
    /// Create a code block from lexical cells in display order.
    pub fn new(cells: impl IntoIterator<Item = M2CodeCell>) -> Self {
        Self {
            cells: cells.into_iter().collect(),
            widest_number: 0,
        }
    }

    fn layout(&self) -> CellLayout {
        self.cells
            .iter()
            .filter_map(|cell| cell.marker().number())
            .chain([self.widest_number])
            .max()
            .filter(|widest| *widest > 0)
            .map_or_else(CellLayout::plain, CellLayout::numbered)
    }

    fn body_height(&self, context: &RenderContext) -> u16 {
        let layout = self.layout();
        self.cells
            .iter()
            .map(|cell| {
                cell.wrapped(
                    context.width().saturating_sub(cell.marker().width(layout)),
                    context.code_wrap(),
                )
                .height()
            })
            .fold(0, u16::saturating_add)
    }

    fn lines(&self, context: &RenderContext, style: Style) -> Vec<Line<'static>> {
        let layout = self.layout();
        let mut rendered = vec![Line::default()];
        for cell in &self.cells {
            rendered.extend(cell_lines(
                cell,
                cell.marker(),
                layout,
                context,
                style,
                true,
            ));
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
                context.code_style(),
                true,
            ));
        }
        rendered.push(Line::default());
        rendered
    }
}

/// The single unfinished submission retained at the bottom of the feed.
pub struct IncompleteBlock {
    code: CodeBlock,
}

impl IncompleteBlock {
    /// Create an unfinished block from the parser's complete active source.
    pub fn new(source: String, spans: Vec<HLSpan>) -> Self {
        Self {
            code: CodeBlock::new([M2CodeCell::rejected(source, spans)]),
        }
    }

    /// Refresh this unfinished source without replacing the block.
    pub fn rehighlight(&mut self, id: SourceId, highlighter: &mut dyn Highlighter) -> bool {
        self.code.cells[0].rehighlight(id, highlighter)
    }

    /// Borrow the retained source as highlighting context for the live input.
    pub fn source(&self) -> &str {
        &self.code.cells[0].text
    }
}

impl Block for IncompleteBlock {
    fn height(&self, context: &RenderContext) -> u16 {
        self.code.height(context)
    }

    fn render_local(&self, context: &RenderContext, buf: &mut Buffer) {
        self.code.render_local(context, buf);
    }

    fn local_selection_inclusions(&self, context: &RenderContext, inclusions: &mut Inclusions) {
        self.code.local_selection_inclusions(context, inclusions);
    }
}

impl Block for CodeBlock {
    fn height(&self, context: &RenderContext) -> u16 {
        self.body_height(context).saturating_add(2)
    }

    fn render_local(&self, context: &RenderContext, buf: &mut Buffer) {
        let area = *buf.area();
        let style = context.code_style();
        Paragraph::new(self.lines(context, style))
            .style(style)
            .render(area, buf);
        let layout = self.layout();
        layout.render_separator(area.height, context.groups().separator(), buf);
        let mut row = area.y.saturating_add(1);
        for cell in &self.cells {
            let marker = cell.marker();
            marker.render(context, style, area, row, buf);
            row = row.saturating_add(
                cell.wrapped(
                    context.width().saturating_sub(marker.width(layout)),
                    context.code_wrap(),
                )
                .height(),
            );
        }
    }
    fn local_selection_inclusions(&self, context: &RenderContext, inclusions: &mut Inclusions) {
        let layout = self.layout();
        inclusions.push(beside_gutter(
            layout.prefix(),
            context,
            self.height(context),
        ));
        let mut row = 1;
        for cell in &self.cells {
            let width = context.width().saturating_sub(cell.marker().width(layout));
            let wrapped = cell.wrapped(width, context.code_wrap());
            inclusions.mark_wrapped_rows(
                row,
                context.width(),
                wrapped.text().split('\n'),
                &cell.text,
            );
            row = row.saturating_add(wrapped.height());
        }
    }
}

/// The columns right of a `prefix`-wide gutter down a block `height` rows tall, which
/// hold its text.
fn beside_gutter(prefix: u16, context: &RenderContext, height: u16) -> Rect {
    Rect::new(prefix, 0, context.width().saturating_sub(prefix), height)
}

fn cell_lines(
    cell: &M2CodeCell,
    marker: CellMarker,
    layout: CellLayout,
    context: &RenderContext,
    style: Style,
    highlight: bool,
) -> Vec<Line<'static>> {
    let width = marker.width(layout);
    cell.wrapped(context.width().saturating_sub(width), context.code_wrap())
        .source_lines()
        .enumerate()
        .map(|(index, (range, _))| {
            let marker = match (index, marker) {
                (_, CellMarker::Bare) | (0, _) => marker,
                _ => CellMarker::Empty,
            };
            let mut line = vec![marker.span(layout, style)];
            if highlight {
                line.extend(cell.spans.paint(
                    range.start,
                    &cell.text[range.clone()],
                    style,
                    context.groups(),
                ));
            } else {
                line.push(Span::styled(cell.text[range.clone()].to_string(), style));
            }
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
    /// Code shown rather than submitted, such as documentation's usage.
    Shown,
}

impl EvaluationState {
    fn marker(&self) -> CellMarker {
        match self {
            Self::Pending => CellMarker::Pending,
            Self::Queued => CellMarker::Queued,
            Self::Succeeded(number, state) => CellMarker::Number(*number, *state),
            Self::Failed(number) => CellMarker::Failed(*number),
            Self::Invalid => CellMarker::Invalid,
            Self::Shown => CellMarker::Empty,
        }
    }
}

/// Rows a fold takes in a value: a rule, its label and a rule.
const FOLD_ROWS: u16 = 3;

/// One rendered result retained by a submission.
enum EvaluationOutput {
    /// Any other result remains open to every [`Block`] implementation.
    Evaluation(Box<dyn Block>),
    /// Printed text stays typed so it can be shown apart from everything else.
    Printed(TextBlock),
    /// Values stay typed so adjacent cells can share one rendered border.
    Value(OutputBlock),
}

impl EvaluationOutput {
    fn block(&self) -> &dyn Block {
        match self {
            Self::Evaluation(block) => block.as_ref(),
            Self::Printed(block) => block,
            Self::Value(block) => block,
        }
    }
}

/// Submitted cells that resolve in place while their M2 evaluation runs.
pub struct EvaluationBlock {
    id: u64,
    cells: Vec<(M2CodeCell, EvaluationState)>,
    outputs: Vec<EvaluationOutput>,
    /// Whether the code wraps within its results' frame rather than the full width.
    framed_code: bool,
}

impl EvaluationBlock {
    /// Source of an evaluation cell, used to count the symbols it references.
    pub fn source(&self, index: usize) -> Option<&str> {
        self.cells.get(index).map(|(cell, _)| cell.text.as_str())
    }

    fn code_style(state: &EvaluationState, after_error: bool, context: &RenderContext) -> Style {
        let groups = context.groups();
        if matches!(state, EvaluationState::Failed(_) | EvaluationState::Invalid) {
            groups.error_code().patch(groups.error())
        } else if after_error {
            groups.error_code().patch(groups.muted_code_text())
        } else {
            context.code_style()
        }
    }

    const OUTPUT_MARGIN: u16 = 1;

    /// Create a pending submission from source cells and their highlights.
    pub fn new(id: u64, cells: impl IntoIterator<Item = (String, Vec<HLSpan>)>) -> Self {
        Self::with_state(id, cells, EvaluationState::Pending)
    }

    /// Code shown as a submission's is, but never evaluated: nothing pending,
    /// numbered, or following it.
    pub fn shown(cells: impl IntoIterator<Item = (String, Vec<HLSpan>)>) -> Self {
        Self {
            id: 0,
            cells: cells
                .into_iter()
                .map(|(text, spans)| (M2CodeCell::rejected(text, spans), EvaluationState::Shown))
                .collect(),
            outputs: Vec::new(),
            framed_code: false,
        }
    }

    /// Create a submission waiting behind the active evaluation.
    pub fn queued(id: u64, cells: impl IntoIterator<Item = (String, Vec<HLSpan>)>) -> Self {
        Self::with_state(id, cells, EvaluationState::Queued)
    }

    /// Create a locally rejected evaluation whose syntax diagnostic is final.
    pub fn invalid(id: u64, source: String, spans: Vec<HLSpan>, message: String) -> Self {
        Self {
            id,
            cells: vec![(
                M2CodeCell::rejected(source, spans),
                EvaluationState::Invalid,
            )],
            outputs: vec![EvaluationOutput::Evaluation(Box::new(ErrorMessageBlock {
                message,
            }))],
            framed_code: false,
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
                        M2CodeCell::rejected(text, spans),
                        match index {
                            0 => first,
                            _ => EvaluationState::Queued,
                        },
                    )
                })
                .collect(),
            outputs: Vec::new(),
            framed_code: false,
        }
    }

    /// Set the code as wide as its results, as documentation sets its examples.
    pub fn with_framed_code(mut self) -> Self {
        self.framed_code = true;
        self
    }

    /// Promote the first unresolved cell when this submission reaches the queue front.
    pub fn start(&mut self) {
        if let Some((_, state @ EvaluationState::Queued)) = self
            .cells
            .iter_mut()
            .find(|(_, state)| matches!(state, EvaluationState::Queued))
        {
            *state = EvaluationState::Pending;
        }
    }

    /// Refresh mapped cells while retaining their evaluation state and outputs.
    pub fn rehighlight(
        &mut self,
        sources: &HashMap<(u64, usize), SourceId>,
        highlighter: &mut dyn Highlighter,
    ) -> bool {
        let mut changed = false;
        for (index, (cell, _)) in self.cells.iter_mut().enumerate() {
            if let Some(&source) = sources.get(&(self.id, index)) {
                changed |= cell.rehighlight(source, highlighter);
            }
        }
        changed
    }

    /// Return the session-local submission identifier.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Resolve one cell and append its non-value output and optional value.
    ///
    /// The value remains concrete so it can merge with an adjacent value block;
    /// every other result needs only the generic rendering contract.
    pub fn succeed(
        &mut self,
        index: usize,
        number: u32,
        cell_state: CellState,
        outputs: Vec<Box<dyn Block>>,
        printed: Option<TextBlock>,
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
        self.outputs
            .extend(printed.map(|text| EvaluationOutput::Printed(text.numbered(number))));
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

    /// Visit only the text cells printed, each numbered by its cell.
    pub(super) fn printed(&self) -> impl Iterator<Item = &dyn Block> {
        self.outputs.iter().filter_map(|output| match output {
            EvaluationOutput::Printed(block) => Some(block as &dyn Block),
            EvaluationOutput::Evaluation(_) | EvaluationOutput::Value(_) => None,
        })
    }

    /// The numbers of the lines this submission has evaluated to something: muted
    /// lines included, since they still hold their value, null ones not.
    pub fn valued_numbers(&self) -> impl Iterator<Item = u32> + '_ {
        self.cells.iter().filter_map(|(_, state)| match state {
            EvaluationState::Succeeded(number, CellState::Value | CellState::Muted) => {
                Some(*number)
            }
            _ => None,
        })
    }

    /// Visit only returned value blocks.
    pub(super) fn values(&self) -> impl Iterator<Item = &dyn Block> {
        self.outputs.iter().filter_map(|output| match output {
            EvaluationOutput::Value(block) => Some(block as &dyn Block),
            EvaluationOutput::Evaluation(_) | EvaluationOutput::Printed(_) => None,
        })
    }

    /// Mark one cell failed and append its error while later cells remain queued.
    pub fn fail(
        &mut self,
        index: usize,
        number: u32,
        outputs: Vec<Box<dyn Block>>,
        printed: Option<TextBlock>,
        message: String,
    ) {
        if let Some((_, state)) = self.cells.get_mut(index) {
            *state = EvaluationState::Failed(number);
        }
        self.outputs
            .extend(outputs.into_iter().map(EvaluationOutput::Evaluation));
        self.outputs
            .extend(printed.map(|text| EvaluationOutput::Printed(text.numbered(number))));
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

    /// The gutter beside the code: its numbers, its separator and the gap
    /// before the code, down every row the code takes.
    pub fn code_gutter(&self, context: &RenderContext) -> Rect {
        Rect::new(
            0,
            0,
            self.layout().prefix(),
            self.code_height(context) + self.output_margin(),
        )
    }

    /// The columns an output's frame takes.
    pub fn frame_columns(&self, context: &RenderContext) -> Range<u16> {
        let width = Rect::new(0, 0, context.width(), 1);
        let frame = Bordered::area(width, self.layout().frame());
        frame.left()..frame.right().min(context.width())
    }

    fn code_width(&self, context: &RenderContext) -> u16 {
        match self.framed_code {
            true => self.frame_columns(context).end,
            false => context.width(),
        }
    }

    fn code_height(&self, context: &RenderContext) -> u16 {
        let layout = self.layout();
        let width = self.code_width(context);
        self.cells.iter().fold(1, |height, (cell, state)| {
            let marker = state.marker();
            height.saturating_add(
                cell.wrapped(
                    width.saturating_sub(marker.width(layout)),
                    context.code_wrap(),
                )
                .height(),
            )
        })
    }

    fn code_lines(&self, context: &RenderContext) -> Vec<Line<'static>> {
        let layout = self.layout();
        let code_context = context.with_width(self.code_width(context));
        let mut lines = vec![Line::default()];
        let mut after_error = false;
        for (cell, state) in &self.cells {
            let failed = matches!(state, EvaluationState::Failed(_) | EvaluationState::Invalid);
            let style = Self::code_style(state, after_error, context);
            lines.extend(cell_lines(
                cell,
                state.marker(),
                layout,
                &code_context,
                style,
                !after_error,
            ));
            after_error |= failed;
        }
        lines
    }

    /// The code keeps its bottom margin from the moment it is submitted, so the block
    /// does not change height when its outputs arrive.
    fn output_margin(&self) -> u16 {
        Self::OUTPUT_MARGIN
    }

    /// Expand or fold the returned value covering local `position`; false when none
    /// folds there.
    pub fn toggle_fold(&mut self, position: Position, context: &RenderContext) -> bool {
        let mut top = self.code_height(context) + self.output_margin();
        for output in &mut self.outputs {
            let bottom = top.saturating_add(output.block().height(context));
            if (top..bottom).contains(&position.y) {
                return match output {
                    EvaluationOutput::Value(block) => {
                        block.toggle_fold(Position::new(position.x, position.y - top), context)
                    }
                    EvaluationOutput::Evaluation(_) | EvaluationOutput::Printed(_) => false,
                };
            }
            top = bottom;
        }
        false
    }
}

impl EvaluationBlock {
    /// Share a retained formatter choice with the results of an embedded example.
    pub fn share_formatter(&mut self, chosen: Rc<Cell<Option<PreviewMode>>>) {
        for output in &mut self.outputs {
            if let EvaluationOutput::Value(block) = output {
                for cell in &mut block.cells {
                    cell.chosen = chosen.clone();
                }
            }
        }
    }

    /// Draw the returned value whose renderer icon is at local `position` by that
    /// renderer; false when no other renderer's icon is there.
    pub fn cycle_renderer(&mut self, position: Position, context: &RenderContext) -> bool {
        let mut top = self.code_height(context) + self.output_margin();
        for output in &mut self.outputs {
            let bottom = top.saturating_add(output.block().height(context));
            if (top..bottom).contains(&position.y) {
                return match output {
                    EvaluationOutput::Value(block) => {
                        block.cycle_renderer(Position::new(position.x, position.y - top), context)
                    }
                    EvaluationOutput::Evaluation(_) | EvaluationOutput::Printed(_) => false,
                };
            }
            top = bottom;
        }
        false
    }
}

impl Block for EvaluationBlock {
    fn pictures(&self) -> Vec<ImageId> {
        self.outputs
            .iter()
            .flat_map(|output| output.block().pictures())
            .collect()
    }

    fn animated(&self) -> bool {
        self.cells
            .iter()
            .any(|(_, state)| matches!(state, EvaluationState::Pending))
    }

    fn toggle_fold(&mut self, position: Position, context: &RenderContext) -> bool {
        Self::toggle_fold(self, position, context)
    }

    fn cycle_renderer(&mut self, position: Position, context: &RenderContext) -> bool {
        Self::cycle_renderer(self, position, context)
    }

    fn code_at(&self, context: &RenderContext, position: Position) -> Option<CellPoint> {
        let layout = self.layout();
        let code_width = self.code_width(context);
        let mut row = 1;
        for (index, (cell, state)) in self.cells.iter().enumerate() {
            let prefix = state.marker().width(layout);
            let wrapped = cell.wrapped(code_width.saturating_sub(prefix), context.code_wrap());
            let next = row + wrapped.height();
            if (row..next).contains(&position.y) {
                let (range, _) = wrapped.source_lines().nth(usize::from(position.y - row))?;
                let column = usize::from(position.x.checked_sub(prefix)?);
                let offset = range.start + offset_at_column(&cell.text[range.clone()], column);
                return Some(CellPoint {
                    submission: self.id,
                    index,
                    offset,
                });
            }
            row = next;
        }
        None
    }

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
            .style(context.code_style())
            .render(Rect::new(area.x, area.y, area.width, code_area_height), buf);
        let layout = self.layout();
        layout.render_separator(code_area_height, context.groups().separator(), buf);
        let code_width = self.code_width(context);
        let mut row = area.y.saturating_add(1);
        let mut after_error = false;
        for (index, (cell, state)) in self.cells.iter().enumerate() {
            let marker = state.marker();
            let failed = matches!(state, EvaluationState::Failed(_) | EvaluationState::Invalid);
            let style = Self::code_style(state, after_error, context);
            after_error |= failed;
            let next = row.saturating_add(
                cell.wrapped(
                    code_width.saturating_sub(marker.width(layout)),
                    context.code_wrap(),
                )
                .height(),
            );
            if after_error {
                // Without any code run cleanly before it, the error claims the top
                // margin too, so the whole block reads as failed.
                let top = if index == 0 { area.y } else { row };
                if let Some(bg) = context.groups().error_code().bg {
                    for y in top..next {
                        for x in area.x..area.right() {
                            buf[(x, y)].set_bg(bg);
                        }
                    }
                }
            }
            marker.render(context, style, area, row, buf);
            row = next;
        }
        if after_error {
            if let Some(bg) = context.groups().error_code().bg {
                for y in row..area.y.saturating_add(code_area_height) {
                    for x in area.x..area.right() {
                        buf[(x, y)].set_bg(bg);
                    }
                }
            }
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

    fn items(&self, context: &RenderContext) -> Vec<Item> {
        let layout = self.layout();
        let code_width = self.code_width(context);
        let mut row = 1;
        let mut items = Vec::new();
        for (cell, state) in &self.cells {
            let marker = state.marker();
            let height = cell
                .wrapped(
                    code_width.saturating_sub(marker.width(layout)),
                    context.code_wrap(),
                )
                .height();
            items.push(Item::new(
                ItemKind::Input,
                row..row.saturating_add(height),
                marker.number(),
                cell.text.clone(),
            ));
            row = row.saturating_add(height);
        }
        let mut top = self.code_height(context) + self.output_margin();
        for block in self.outputs() {
            items.extend(block.items(context).into_iter().map(|item| item.moved(top)));
            top = top.saturating_add(block.height(context));
        }
        items
    }

    fn links(&self, context: &RenderContext) -> Vec<Link> {
        let mut top = self.code_height(context) + self.output_margin();
        let mut links = Vec::new();
        for block in self.outputs() {
            links.extend(
                block
                    .links(context)
                    .into_iter()
                    .map(|link| link.moved(Offset::new(0, i32::from(top)))),
            );
            top = top.saturating_add(block.height(context));
        }
        links
    }

    fn link_at(&self, context: &RenderContext, position: Position) -> Option<Link> {
        let mut top = self.code_height(context) + self.output_margin();
        for block in self.outputs() {
            let bottom = top.saturating_add(block.height(context));
            if (top..bottom).contains(&position.y) {
                return block
                    .link_at(context, Position::new(position.x, position.y - top))
                    .map(|link| link.moved(Offset::new(0, i32::from(top))));
            }
            top = bottom;
        }
        None
    }

    fn local_selection_inclusions(&self, context: &RenderContext, inclusions: &mut Inclusions) {
        let code_height = self.code_height(context);
        let code_area_height = code_height + self.output_margin();
        let layout = self.layout();
        inclusions.push(beside_gutter(layout.prefix(), context, code_area_height));
        let code_width = self.code_width(context);
        let mut row = 1;
        for (cell, state) in &self.cells {
            let wrapped = cell.wrapped(
                code_width.saturating_sub(state.marker().width(layout)),
                context.code_wrap(),
            );
            inclusions.mark_wrapped_rows(
                row,
                context.width(),
                wrapped.text().split('\n'),
                &cell.text,
            );
            row = row.saturating_add(wrapped.height());
        }
        let mut y = code_area_height;
        for block in self.outputs() {
            let height = block.height(context);
            Block::selection_inclusions(
                block,
                context,
                Rect::new(0, y, context.width(), height),
                inclusions,
            );
            y = y.saturating_add(height);
        }
    }
}

/// Text written by Spectral, or printed by M2 during evaluation and then
/// numbered as the cell that printed it is.
pub struct TextBlock {
    text: String,
    kind: TextKind,
}

/// Origin and presentation of a text block.
enum TextKind {
    System,
    Printed { number: Option<u32> },
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
    pub fn system(text: impl Into<String>) -> Self {
        Self::new(text, TextKind::System)
    }

    /// The frame of a system message, wide enough for its longest line when the
    /// pane allows.
    fn frame(&self) -> FrameLayout {
        let widest = self
            .text
            .lines()
            .map(UnicodeWidthStr::width)
            .max()
            .unwrap_or(0);
        FrameLayout::middle_third(u16::try_from(widest).unwrap_or(u16::MAX))
    }

    /// Create text printed by evaluated M2 code.
    pub fn printed(text: impl Into<String>) -> Self {
        Self::new(text, TextKind::Printed { number: None })
    }

    /// Number printed text as the cell that printed it; system text has none.
    pub fn numbered(mut self, number: u32) -> Self {
        if let TextKind::Printed { number: numbered } = &mut self.kind {
            *numbered = Some(number);
        }
        self
    }

    /// The gutter printed text is inset by, as wide as a code cell's carrying
    /// the same number.
    fn printed_layout(number: Option<u32>) -> CellLayout {
        number.map_or_else(CellLayout::plain, CellLayout::numbered)
    }

    fn body(&self, context: &RenderContext) -> Vec<String> {
        let width = match self.kind {
            TextKind::System => Bordered::text_width(context, self.frame()),
            TextKind::Printed { number } => context
                .width()
                .saturating_sub(Self::printed_layout(number).prefix()),
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
            TextKind::Printed { .. } => 2 * UI::PRINTED_MARGIN,
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
                    .map(|line| Line::from(Span::styled(line, theme.Info)).centered())
                    .collect();
                Bordered::new(self.frame(), Style::new(), theme.info_border(), None, body).render(
                    &(),
                    area,
                    buf,
                );
            }
            TextKind::Printed { number } => {
                let prefix = usize::from(Self::printed_layout(number).prefix());
                let number = number.filter(|_| context.numbers_printed());
                let margin = || (0..UI::PRINTED_MARGIN).map(|_| Line::default());
                // The number sits where a code cell's does, on the first line only.
                let gutter = |row: usize| match number.filter(|_| row == 0) {
                    Some(number) => Span::styled(
                        format!(
                            "{:margin$}{number:<width$}",
                            "",
                            margin = usize::from(UI::CELL_MARGIN),
                            width = prefix - usize::from(UI::CELL_MARGIN),
                        ),
                        theme.printed_cell_number(),
                    ),
                    None => Span::raw(" ".repeat(prefix)),
                };
                let lines = margin()
                    .chain(
                        self.body(context)
                            .into_iter()
                            .enumerate()
                            .map(|(row, line)| Line::from(vec![gutter(row), Span::raw(line)])),
                    )
                    .chain(margin())
                    .collect::<Vec<_>>();
                Paragraph::new(lines)
                    .style(theme.printed())
                    .render(area, buf);
            }
        }
    }
    fn items(&self, context: &RenderContext) -> Vec<Item> {
        let (margin, number) = match self.kind {
            TextKind::System => (0, None),
            TextKind::Printed { number } => (UI::PRINTED_MARGIN, number),
        };
        let rows = margin..self.height(context).saturating_sub(margin);
        vec![Item::new(ItemKind::Note, rows, number, self.text.clone())]
    }

    fn local_selection_inclusions(&self, context: &RenderContext, inclusions: &mut Inclusions) {
        if matches!(self.kind, TextKind::Printed { .. }) {
            let body = self.body(context);
            inclusions.mark_wrapped_rows(
                UI::PRINTED_MARGIN,
                context.width(),
                body.iter().map(String::as_str),
                &self.text,
            );
        }
        match self.kind {
            TextKind::System => inclusions.push(Bordered::inner(
                Rect::new(0, 0, context.width(), self.height(context)),
                self.frame(),
            )),
            TextKind::Printed { number } => inclusions.push(beside_gutter(
                Self::printed_layout(number).prefix(),
                context,
                self.height(context),
            )),
        }
    }
}

/// One result inside a possibly shared value border.
struct OutputCell {
    layout: RefCell<Option<ValueLayout>>,
    /// The layout being computed beside the drawing thread, and what it is for.
    relayout: RefCell<Option<(LayoutKey, Pending<Laid>)>>,
    source: Arc<ValueSource>,
    /// The pictures the rows last laid out for this cell hold.
    pictures: RefCell<Vec<ImageId>>,
    number: Option<u32>,
    ty: Option<String>,
    /// The annotation's pieces, set from their structure when M2 sent them.
    type_drawing: Option<Rod>,
    /// The formatter choice shared by every value in the enclosing block.
    chosen: Rc<Cell<Option<PreviewMode>>>,
}

impl Spelled for PreviewMode {
    type Spelling = &'static str;

    fn spelled(&self, charset: Charset) -> &'static str {
        match self {
            Self::Rod => Glyphs::RendererRod,
            Self::Net => Glyphs::RendererNet,
            Self::String => Glyphs::RendererString,
            Self::Tex => Glyphs::RendererTex,
            Self::Markup => Glyphs::RendererMarkup,
            Self::External => Glyphs::RendererExternal,
            Self::Peek => Glyphs::RendererPeek,
            Self::Speek => Glyphs::RendererSpeek,
        }
        .spelled(charset)
    }
}

impl OutputCell {
    fn default_preview(&self) -> PreviewMode {
        match self.source.value.as_ref() {
            Some(value) if value.html.is_some() => PreviewMode::Markup,
            Some(value) if value.is_structured() => PreviewMode::Rod,
            Some(_) => PreviewMode::Net,
            None => PreviewMode::Net,
        }
    }

    /// The formatter chosen, unless this cell has nothing from it: a recorded
    /// example's answer is M2's string even when its computed twin shares a
    /// choice of `rod`.
    fn preview_mode(&self) -> PreviewMode {
        self.chosen
            .get()
            .filter(|mode| self.offers(*mode))
            .unwrap_or_else(|| self.default_preview())
    }

    fn offers(&self, mode: PreviewMode) -> bool {
        match mode {
            PreviewMode::Rod => self.source.value.as_ref().is_some_and(Rod::is_structured),
            PreviewMode::Markup => self
                .source
                .value
                .as_ref()
                .is_some_and(|value| value.html.is_some()),
            _ => self.preview_text(mode).is_some(),
        }
    }

    fn preview_text(&self, mode: PreviewMode) -> Option<&str> {
        self.source.preview_text(mode)
    }

    fn markup(&self) -> Option<(&str, usize)> {
        match self.preview_mode() {
            PreviewMode::Markup => self
                .source
                .value
                .as_ref()?
                .markup()
                .map(|(html, width)| (html, *width)),
            PreviewMode::Rod => self
                .source
                .value
                .as_ref()?
                .markup()
                .map(|(html, width)| (html, *width)),
            _ => None,
        }
    }
}

/// Retained ordinary value rows, independent of folding and output labels.
struct ValueLayout {
    key: LayoutKey,
    lines: Rc<Vec<Line<'static>>>,
    /// The links drawn among `lines`, placed from their top left corner.
    links: Rc<Vec<Link>>,
}

/// Everything a value's layout depends on besides the value itself.
#[derive(Clone, Copy)]
struct LayoutKey {
    renderer: PreviewMode,
    /// Zero for a value without markup, which no markup revision moves.
    markup_revision: u64,
    renderer_position: RendererPosition,
    width: u16,
    /// The block's own width, which bounds how many rows a preview keeps.
    block_width: u16,
    pixel_cell: Option<Size>,
    math: MathStyle,
    groups: &'static Groups,
}

impl PartialEq for LayoutKey {
    fn eq(&self, other: &Self) -> bool {
        self.renderer == other.renderer
            && self.markup_revision == other.markup_revision
            && self.renderer_position == other.renderer_position
            && self.width == other.width
            && self.block_width == other.block_width
            && self.pixel_cell == other.pixel_cell
            && self.math == other.math
            && ptr::eq(self.groups, other.groups)
    }
}

impl fmt::Debug for LayoutKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LayoutKey")
            .field("renderer", &self.renderer)
            .field("width", &self.width)
            .field("block_width", &self.block_width)
            .field("math", &self.math)
            .finish_non_exhaustive()
    }
}

/// What a value is laid out from, shared with the worker laying it out again.
#[derive(Clone, Debug)]
struct ValueSource {
    previews: Vec<ValuePreview>,
    /// The value as plain text, which without M2's own may take a whole drawing;
    /// filled on a worker, so the drawing thread only reads it once there.
    text: OnceLock<String>,
    value: Option<Rod>,
}

impl ValueSource {
    /// The plain text, drawn here when no one has yet: never on the drawing thread.
    fn text(&self) -> &str {
        self.text.get_or_init(|| {
            self.value.as_ref().map_or_else(String::new, |value| {
                value.text().map_or_else(|| value.render(), output_preview)
            })
        })
    }

    /// The plain text if a worker has drawn it, else a placeholder.
    fn text_drawn(&self) -> String {
        self.text
            .get()
            .cloned()
            .unwrap_or_else(|| LAYING_OUT.current())
    }

    fn preview_text(&self, mode: PreviewMode) -> Option<&str> {
        self.previews
            .iter()
            .find(|preview| preview.mode == mode)
            .map(|preview| preview.text.as_str())
    }

    /// The rows M2's own text wraps into for the text formatters.
    fn text_lines(&self, renderer: PreviewMode, width: u16) -> Vec<String> {
        let text = match renderer {
            PreviewMode::Net => self
                .preview_text(PreviewMode::Net)
                .map(str::to_owned)
                .or_else(|| self.value.as_ref().map(Rod::ascii)),
            mode @ (PreviewMode::String | PreviewMode::Peek | PreviewMode::Speek) => {
                self.preview_text(mode).map(str::to_owned)
            }
            _ => None,
        }
        .unwrap_or_else(|| self.text().to_string());
        word_lines(&text, width)
    }

    /// The value laid out under `key`, without markup, which is laid out apart.
    fn laid(&self, key: &LayoutKey) -> Laid {
        let LayoutKey {
            renderer,
            width,
            math: style,
            groups: theme,
            ..
        } = *key;
        let width = usize::from(width);
        let mut links = Vec::new();
        let lines = if renderer == PreviewMode::External {
            let source = self
                .preview_text(PreviewMode::External)
                .unwrap_or(self.text());
            code_lines(source, width as u16, theme)
        } else if renderer == PreviewMode::Tex {
            let drawn = typeset::tex_preview(
                self.preview_text(PreviewMode::Tex).unwrap_or(""),
                width,
                theme,
                style,
            );
            links = drawn.links;
            drawn.lines
        } else if renderer == PreviewMode::Rod {
            let mut preview = self.value.clone();
            // A value drawn as one atom shows the text cut as every text formatter cuts it.
            if let Some(Shape::Atom { text, .. }) =
                preview.as_mut().map(|value| &mut value.drawing.shape)
            {
                *text = self.text().to_string();
            }
            match preview.as_ref() {
                Some(value) if style.display == MathDisplay::Typeset => {
                    let drawn = typeset::drawn(value, width, theme, style);
                    links = drawn.links;
                    drawn.lines
                }
                Some(value) if style.display == MathDisplay::Ascii => {
                    word_lines(&value.ascii(), width as u16)
                        .into_iter()
                        .map(|line| Line::from(Span::styled(line, theme.value())))
                        .collect()
                }
                Some(value) => maths::spans(value, width, style)
                    .map(|rows| {
                        links = present::links(&rows);
                        rows.into_iter()
                            .map(|row| Line::from(row.spans(theme)))
                            .collect()
                    })
                    .unwrap_or_default(),
                None => Vec::new(),
            }
        } else {
            self.text_lines(renderer, width as u16)
                .into_iter()
                .map(|line| Line::from(Span::styled(line, theme.value())))
                .collect()
        };
        let mut laid = Laid { lines, links };
        for span in laid.lines.iter_mut().flat_map(|line| &mut line.spans) {
            span.style.bg = None;
        }
        laid.truncated(key.block_width)
    }
}

/// A value's rows and the links among them.
#[derive(Debug, Default)]
struct Laid {
    lines: Vec<Line<'static>>,
    links: Vec<Link>,
}

impl Laid {
    /// The rows a preview keeps within the render limit, the middle cut out.
    fn truncated(mut self, block_width: u16) -> Self {
        let limit = (UI::MAX_OUTPUT_PREVIEW_CELLS / usize::from(block_width.max(1))).max(1);
        if self.lines.len() > limit {
            let head = limit.saturating_sub(1) / 2;
            let tail = limit.saturating_sub(head + 1);
            let end = self.lines.len().saturating_sub(tail);
            self.lines
                .splice(head..end, [Line::raw(PREVIEW_TRUNCATED.current())]);
            // Only the rows above the cut stay where they were laid out.
            self.links
                .retain(|link| link.cells.iter().all(|cells| usize::from(cells.y) < head));
        }
        self
    }
}

struct RendererTitle {
    area: Rect,
    line: Line<'static>,
    icons: Vec<(Rect, PreviewMode)>,
}

/// Consecutive values returned by M2 inside one border.
pub struct OutputBlock {
    cells: Vec<OutputCell>,
    /// Whether values taller than the output limit show every row.
    expanded: bool,
    framed: bool,
}

impl OutputBlock {
    /// Create a rendered value and its optional type.
    pub fn value(text: String, ty: Option<String>) -> Self {
        Self {
            cells: vec![OutputCell {
                layout: RefCell::default(),
                relayout: RefCell::default(),
                source: Arc::new(ValueSource {
                    previews: vec![ValuePreview::new(
                        PreviewMode::Net,
                        text.trim_end().to_string(),
                    )],
                    text: OnceLock::from(text.trim_end().to_string()),
                    value: None,
                }),
                pictures: RefCell::default(),
                number: None,
                ty: ty.map(|ty| ty.trim_end().to_string()),
                type_drawing: None,
                chosen: Rc::default(),
            }],
            expanded: false,
            framed: true,
        }
    }

    /// Retain a structured M2 value for width-aware terminal rendering.
    pub fn rendered(value: Rod, ty: Option<String>) -> Self {
        if let Some(html) = &value.html {
            markup::remember_types(html, &value.type_names);
        }
        Self {
            cells: vec![OutputCell {
                layout: RefCell::default(),
                relayout: RefCell::default(),
                source: Arc::new(ValueSource {
                    previews: value
                        .fallback
                        .as_deref()
                        .or(value.text())
                        .map(|text| ValuePreview::new(PreviewMode::Net, output_preview(text)))
                        .into_iter()
                        .collect(),
                    text: OnceLock::new(),
                    value: Some(value),
                }),
                pictures: RefCell::default(),
                number: None,
                ty,
                type_drawing: None,
                chosen: Rc::default(),
            }],
            expanded: false,
            framed: true,
        }
    }

    /// Draw each value's plain text on a worker, once the block is whole and its
    /// sources are shared rather than copied on change.
    fn drawing_text(self) -> Self {
        for cell in &self.cells {
            let source = cell.source.clone();
            relayout::detach(move || {
                source.text();
            });
        }
        self
    }

    /// Attach the M2 formatter results offered by this output's preview selector.
    pub fn with_previews(mut self, previews: Vec<ValuePreview>) -> Self {
        Arc::make_mut(&mut self.cells[0].source).previews = previews
            .into_iter()
            .map(|mut preview| {
                if matches!(
                    preview.mode,
                    PreviewMode::Net
                        | PreviewMode::String
                        | PreviewMode::External
                        | PreviewMode::Peek
                        | PreviewMode::Speek
                ) {
                    preview.text = output_preview(&preview.text);
                }
                preview
            })
            .collect();
        self
    }

    /// Present this value without its enclosing output border.
    pub fn without_frame(mut self) -> Self {
        self.framed = false;
        self
    }

    /// Set the type annotation from its drawing rather than its text.
    pub fn annotated(mut self, drawing: Option<Rod>) -> Self {
        self.cells[0].type_drawing = drawing;
        self
    }

    /// Attach the `oN` assigned by Macaulay2 to this value.
    pub fn numbered(mut self, number: u32) -> Self {
        self.cells[0].number = Some(number);
        self
    }

    fn append(&mut self, other: Self) {
        let chosen = self.cells[0].chosen.clone();
        if chosen.get().is_none() {
            chosen.set(Some(self.cells[0].default_preview()));
        }
        for mut cell in other.cells {
            cell.chosen = chosen.clone();
            self.cells.push(cell);
        }
    }

    /// The frame of a code cell carrying the largest number here, so a result's
    /// border runs down the column its code's separator does.
    fn border_layout(&self) -> FrameLayout {
        let number = self.cells.iter().filter_map(|cell| cell.number).max();
        number
            .map_or_else(CellLayout::plain, CellLayout::numbered)
            .frame()
    }

    fn text_width(&self, context: &RenderContext) -> u16 {
        Bordered::text_width(context, self.border_layout())
    }

    /// The columns `cell`'s value wraps to: short of its type and the gap before
    /// it, when the type is narrow enough to stand beside the value; else all of
    /// them, the type then going below wherever the value leaves it no room.
    fn value_width(&self, cell: &OutputCell, context: &RenderContext) -> u16 {
        let width = self.text_width(context);
        let column = self
            .type_lines(cell, context)
            .iter()
            .map(Line::width)
            .max()
            .map_or(0, |type_width| type_width + UI::TYPE_GAP);
        match column as f32 <= f32::from(width) * UI::MAX_TYPE_SHARE {
            true => u16::try_from(usize::from(width) - column).unwrap_or(width),
            false => width,
        }
    }

    fn unfolded_lines(&self, cell: &OutputCell, context: &RenderContext) -> Rc<Vec<Line<'static>>> {
        let style = context.math();
        let markup = cell.markup();
        let key = LayoutKey {
            renderer: cell.preview_mode(),
            markup_revision: markup.map_or(0, |_| markup::revision()),
            renderer_position: context.renderer_position(),
            width: self.value_width(cell, context),
            block_width: context.width(),
            pixel_cell: (style.display == MathDisplay::Typeset)
                .then(cell_size)
                .flatten(),
            math: style,
            groups: context.groups(),
        };
        if let Some(layout) = cell
            .layout
            .borrow()
            .as_ref()
            .filter(|layout| layout.key == key)
        {
            return layout.lines.clone();
        }
        let laid = match markup {
            Some((html, example_width)) => {
                let lines = markup::lines(html, example_width, &context.with_width(key.width))
                    .map(|lines| lines.as_ref().clone())
                    .unwrap_or_else(|| {
                        cell.source
                            .text_lines(key.renderer, key.width)
                            .into_iter()
                            .map(Line::raw)
                            .collect()
                    });
                Laid {
                    lines,
                    links: Vec::new(),
                }
                .truncated(key.block_width)
            }
            None => match Self::relaid(cell, &key, context) {
                Ok(laid) => laid,
                Err(stale) => return stale,
            },
        };
        let lines = Rc::new(laid.lines);
        cell.layout.replace(Some(ValueLayout {
            key,
            lines: lines.clone(),
            links: Rc::new(laid.links),
        }));
        lines
    }

    /// `cell`'s value laid out under `key`, always by a worker unless the context
    /// asks for it on the spot. Until the worker is done, the rows laid out last
    /// stay on screen whatever moved, and a placeholder row only when nothing was
    /// laid out before.
    fn relaid(
        cell: &OutputCell,
        key: &LayoutKey,
        context: &RenderContext,
    ) -> Result<Laid, Rc<Vec<Line<'static>>>> {
        let adopted = context.adopted_relayouts();
        let finished = cell
            .relayout
            .borrow()
            .as_ref()
            .filter(|(pending, _)| pending == key)
            .and_then(|(_, pending)| pending.take(adopted));
        if let Some(laid) = finished {
            cell.relayout.take();
            return Ok(laid);
        }
        let stale = cell
            .layout
            .borrow()
            .as_ref()
            .map(|layout| layout.lines.clone());
        let stale = match stale {
            Some(stale) => stale,
            None if context.inline_layout() => {
                cell.relayout.take();
                return Ok(cell.source.laid(key));
            }
            None => Rc::new(vec![Line::raw(LAYING_OUT.current())]),
        };
        let requested = cell
            .relayout
            .borrow()
            .as_ref()
            .is_some_and(|(pending, _)| pending == key);
        if !requested {
            let (source, key) = (cell.source.clone(), *key);
            cell.relayout
                .replace(Some((key, Pending::spawn(move || source.laid(&key)))));
        }
        Err(stale)
    }

    /// The path that laid out `cell`'s value rows.
    fn renderer(&self, cell: &OutputCell, context: &RenderContext) -> PreviewMode {
        let _ = context;
        cell.preview_mode()
    }

    /// Value rows kept above the fold and rows hidden by it, when `rows` exceed
    /// the output limit and this block is not expanded.
    fn fold(&self, rows: usize, context: &RenderContext) -> Option<(usize, usize)> {
        let limit = context.output_limit().filter(|_| !self.expanded)?;
        let tail = limit.saturating_sub(3) / 2;
        let head = limit.saturating_sub(tail + 3);
        (rows > limit).then(|| (head, rows - head - tail))
    }

    fn cell_fold(&self, cell: &OutputCell, context: &RenderContext) -> Option<(usize, usize)> {
        self.fold(self.unfolded_lines(cell, context).len(), context)
    }

    /// The text among `cell`'s value rows before any fold: a page says where in it
    /// the text lies, any other value is text across `width`.
    fn value_inclusions(
        &self,
        cell: &OutputCell,
        width: u16,
        context: &RenderContext,
    ) -> Inclusions {
        let page = cell.markup().and_then(|(html, example_width)| {
            markup::inclusions(
                html,
                example_width,
                &context.with_width(self.value_width(cell, context)),
            )
        });
        page.unwrap_or_else(|| {
            let rows = row_count!(self.unfolded_lines(cell, context).len());
            [Rect::new(0, 0, width, rows)].into()
        })
    }

    /// What `cell`'s value reads as on one row, before any wrapping.
    fn unwrapped_text(cell: &OutputCell, context: &RenderContext) -> String {
        let _ = context;
        cell.preview_text(cell.preview_mode())
            .map_or_else(|| cell.source.text_drawn(), str::to_string)
    }

    /// Value rows with three rows for the fold control.
    fn value_lines(&self, cell: &OutputCell, context: &RenderContext) -> Vec<Line<'static>> {
        let lines = self.unfolded_lines(cell, context);
        if let Some((head, hidden)) = self.fold(lines.len(), context) {
            lines[..head]
                .iter()
                .cloned()
                .chain(iter::repeat_n(Line::default(), usize::from(FOLD_ROWS)))
                .chain(lines[head + hidden..].iter().cloned())
                .collect()
        } else {
            lines.as_ref().clone()
        }
    }

    /// Separator, message, separator, each crossing the output border.
    fn fold_lines(hidden: usize, width: u16, context: &RenderContext) -> [Line<'static>; 3] {
        let style = context.groups().output_number();
        let rule = Line::from(Span::styled(
            Glyphs::FoldRule.current().repeat(usize::from(width)),
            style,
        ));
        let label = format!(
            "{} {hidden} rows hidden {} double-click to expand",
            Glyphs::FoldedLines.current(),
            Glyphs::HintSeparator.current()
        );
        let padding = usize::from(width).saturating_sub(label.width()) / 2;
        [
            rule.clone(),
            Line::from(Span::styled(
                format!("{}{label}", " ".repeat(padding)),
                style,
            )),
            rule,
        ]
    }

    /// Expand every folded value from anywhere on the block, or fold them back from
    /// its border at local `position`; false when nothing toggled.
    pub fn toggle_fold(&mut self, position: Position, context: &RenderContext) -> bool {
        let foldable = context.output_limit().is_some_and(|limit| {
            self.cells
                .iter()
                .any(|cell| self.unfolded_lines(cell, context).len() > limit)
        });
        let area = Rect::new(0, 0, context.width(), self.height(context));
        let toggled = foldable
            && (!self.expanded || Bordered::on_border(area, self.border_layout(), position));
        self.expanded ^= toggled;
        toggled
    }

    /// Mode controls share the border geometry used for painting and clicking.
    fn renderer_title(&self, index: usize, context: &RenderContext) -> Option<RendererTitle> {
        if !self.framed || index != 0 || context.renderer_position() == RendererPosition::Off {
            return None;
        }
        let cell = &self.cells[index];
        let current = self.renderer(cell, context);
        let mut modes = self.renderers(cell, context);
        modes.retain(|mode| {
            self.cells
                .iter()
                .all(|cell| self.renderers(cell, context).contains(mode))
        });
        if !modes.contains(&current) {
            modes.push(current);
        }
        let frame = Bordered::area(
            Rect::new(0, 0, context.width(), self.height(context)),
            self.border_layout(),
        );
        loop {
            let mut spans = vec![Span::raw(" ")];
            for mode in &modes {
                let style = if *mode == current {
                    context.groups().value().add_modifier(Modifier::BOLD)
                } else {
                    context.groups().muted().add_modifier(Modifier::DIM)
                };
                spans.push(Span::styled(mode.current().to_string(), style));
                spans.push(Span::raw(" "));
            }
            let line = Line::from(spans);
            if line.width() <= usize::from(frame.width.saturating_sub(2)) || modes.len() <= 1 {
                let area = context
                    .renderer_position()
                    .area(frame, row_count!(line.width()));
                let mut column = area.x + 1;
                let icons = modes
                    .iter()
                    .map(|mode| {
                        let width = row_count!(mode.current().width());
                        let rect = Rect::new(column, area.y, width, 1).intersection(area);
                        column += width + 1;
                        (rect, *mode)
                    })
                    .collect();
                return Some(RendererTitle { area, line, icons });
            }
            // The renderer in use stays, as the one the title shows is chosen.
            if let Some(dropped) = modes.iter().rposition(|mode| *mode != current) {
                modes.remove(dropped);
            }
        }
    }

    /// The renderers `cell`'s value can be drawn by, the ones that would be
    /// ignored for its kind of value left out.
    fn renderers(&self, cell: &OutputCell, context: &RenderContext) -> Vec<PreviewMode> {
        let _ = context;
        PreviewMode::CHOICES
            .iter()
            .copied()
            .filter(|mode| cell.offers(*mode))
            .collect()
    }

    /// Draw the value whose renderer icon is at local `position` by that
    /// renderer, so what an author wrote can be told from what it was drawn as.
    fn cycle_renderer(&mut self, position: Position, context: &RenderContext) -> bool {
        let area = Rect::new(0, 0, context.width(), self.height(context));
        let inner = Bordered::inner(area, self.border_layout());
        let mut row = inner.y;
        for (index, cell) in self.cells.iter().enumerate() {
            row += Self::gap(index);
            if let Some((html, example_width)) = cell.markup() {
                if position.y >= row && position.x >= inner.x {
                    let mut y = position.y - row;
                    if let Some((head, hidden)) = self.cell_fold(cell, context) {
                        if usize::from(y) >= head + usize::from(FOLD_ROWS) {
                            y = y
                                .saturating_add(row_count!(hidden))
                                .saturating_sub(FOLD_ROWS);
                        }
                    }
                    if markup::cycle_renderer(
                        html,
                        example_width,
                        &context.with_width(self.value_width(cell, context)),
                        Position::new(position.x - inner.x, y),
                    ) {
                        return true;
                    }
                }
            }
            row += row_count!(self.lines(cell, context).len());
        }
        for (index, cell) in self.cells.iter().enumerate() {
            let Some(title) = self.renderer_title(index, context) else {
                continue;
            };
            let Some(clicked) = title
                .icons
                .iter()
                .find(|(area, _)| area.contains(position))
                .map(|(_, mode)| *mode)
            else {
                continue;
            };
            let changed = clicked != self.renderer(cell, context);
            cell.chosen.set(Some(clicked));
            return changed;
        }
        false
    }

    fn type_lines(&self, cell: &OutputCell, context: &RenderContext) -> Vec<Line<'static>> {
        let theme = context.groups();
        let width = usize::from(self.text_width(context));
        let math = context.math();
        let drawing = cell.type_drawing.as_ref().filter(|_| !math.original_text);
        if let Some(drawing) = drawing.filter(|_| math.display == MathDisplay::Typeset) {
            return typeset::annotation(drawing, width, theme, context.math());
        }
        if let Some(drawing) = drawing {
            return maths::annotation(drawing, width, context.math())
                .into_iter()
                .map(|row| {
                    let mut spans = row.spans(theme);
                    for span in &mut spans {
                        span.style.bg = None;
                    }
                    Line::from(spans)
                })
                .collect();
        }
        cell.ty.as_deref().map_or_else(Vec::new, |ty| {
            word_lines(ty, self.text_width(context))
                .into_iter()
                .map(|line| Line::from(Span::styled(line, context.groups().type_info())))
                .collect()
        })
    }

    /// The row the type's first row goes on: beside the value, its last row on the
    /// value's last, when every value row it shares leaves it room; else below.
    fn type_start(values: &[Line<'_>], types: usize, type_width: usize, width: usize) -> usize {
        let fits = |value: &Line<'_>| value.width() + type_width < width;
        let start = values.len().saturating_sub(types.max(1));
        match !values.is_empty() && values[start..].iter().all(fits) {
            true => start,
            false => values.len(),
        }
    }

    fn lines(&self, cell: &OutputCell, context: &RenderContext) -> Vec<Line<'static>> {
        let lines = self.composed_lines(cell, context);
        let pictures = match context.math().display {
            MathDisplay::Typeset => graphics::shown_in_lines(&lines).into_iter().collect(),
            _ => Vec::new(),
        };
        cell.pictures.replace(pictures);
        lines
    }

    fn composed_lines(&self, cell: &OutputCell, context: &RenderContext) -> Vec<Line<'static>> {
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
        let start = Self::type_start(&lines, types.len(), type_width, width);
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

impl OutputBlock {
    /// The links among `cell`'s value rows, before any fold.
    fn value_links(&self, cell: &OutputCell, context: &RenderContext) -> Rc<Vec<Link>> {
        self.unfolded_lines(cell, context);
        cell.layout
            .borrow()
            .as_ref()
            .map_or_else(Rc::default, |layout| layout.links.clone())
    }

    /// The links `cell`'s value draws, as shown from `origin`: a source location
    /// whole, or what the value links among its parts. Documentation is laid out
    /// apart, by its own renderer.
    fn drawn_links<'a>(
        &'a self,
        cell: &'a OutputCell,
        context: &'a RenderContext,
        origin: Position,
    ) -> impl Iterator<Item = Link> + 'a {
        self.location_link(cell, context)
            .into_iter()
            .chain(
                self.value_links(cell, context)
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>(),
            )
            .map(move |link| self.placed_link(cell, context, origin, link))
            .filter(|link| !link.cells.is_empty())
    }

    /// The link a source location draws: every row it wraps onto, before any fold.
    fn location_link(&self, cell: &OutputCell, context: &RenderContext) -> Option<Link> {
        let target = cell
            .source
            .value
            .as_ref()
            .filter(|_| {
                context.math().display == MathDisplay::Ascii || context.math().original_text
            })
            .and_then(|value| value.drawing.target.as_ref())?;
        let cells = self
            .unfolded_lines(cell, context)
            .iter()
            .enumerate()
            .filter_map(|(row, line)| {
                let width = line.to_string().trim_end().width();
                (width > 0).then(|| Rect::new(0, row as u16, width as u16, 1))
            })
            .collect();
        Some(Link {
            target: target.as_str().into(),
            cells,
        })
    }

    fn placed_link(
        &self,
        cell: &OutputCell,
        context: &RenderContext,
        origin: Position,
        link: Link,
    ) -> Link {
        let fold = self.cell_fold(cell, context);
        let cells = link
            .cells
            .iter()
            .filter_map(|cells| {
                let unfolded = usize::from(cells.y);
                let shown = match fold {
                    Some((head, hidden)) if (head..head + hidden).contains(&unfolded) => {
                        return None
                    }
                    Some((head, hidden)) if unfolded >= head + hidden => unfolded + 3 - hidden,
                    _ => unfolded,
                };
                Some(Rect::new(
                    cells.x + origin.x,
                    origin.y + shown as u16,
                    cells.width,
                    1,
                ))
            })
            .collect();
        Link { cells, ..link }
    }
}

impl Block for OutputBlock {
    fn pictures(&self) -> Vec<ImageId> {
        self.cells
            .iter()
            .flat_map(|cell| cell.pictures.borrow().clone())
            .collect()
    }

    fn toggle_fold(&mut self, position: Position, context: &RenderContext) -> bool {
        Self::toggle_fold(self, position, context)
    }

    fn cycle_renderer(&mut self, position: Position, context: &RenderContext) -> bool {
        Self::cycle_renderer(self, position, context)
    }

    fn height(&self, context: &RenderContext) -> u16 {
        row_count!(self.body(context).len()).saturating_add(2)
    }

    fn render_local(&self, context: &RenderContext, buf: &mut Buffer) {
        let area = *buf.area();
        let theme = context.groups();
        let layout = self.border_layout();
        if self.framed {
            Bordered::new(
                layout,
                Style::new(),
                theme.border(),
                None,
                self.body(context),
            )
            .render(&(), area, buf);
        } else {
            Paragraph::new(self.body(context)).render(Bordered::inner(area, layout), buf);
        }
        let mut row = Bordered::inner(area, layout).y;
        for (index, cell) in self.cells.iter().enumerate() {
            row = row.saturating_add(Self::gap(index));
            if let Some(number) = cell.number {
                let label = format!("o{number}");
                let width = label.chars().count() as u16;
                let column = (layout.inset() + 2).saturating_sub(width);
                let style = match context.is_recent_output(number) {
                    true => theme.recent_output(),
                    false => theme.output_number(),
                };
                Paragraph::new(Span::styled(label, style))
                    .render(Rect::new(area.x + column, row, width, 1), buf);
            }
            if let Some(title) = self.renderer_title(index, context) {
                title.line.render(title.area, buf);
            }
            if let Some((head, hidden)) = self.cell_fold(cell, context) {
                for (offset, line) in Self::fold_lines(hidden, area.width, context)
                    .into_iter()
                    .enumerate()
                {
                    line.render(
                        Rect::new(
                            area.x,
                            row.saturating_add(row_count!(head))
                                .saturating_add(offset as u16),
                            area.width,
                            1,
                        ),
                        buf,
                    );
                }
            }
            row = row.saturating_add(row_count!(self.lines(cell, context).len()));
        }
    }

    fn items(&self, context: &RenderContext) -> Vec<Item> {
        let area = Rect::new(0, 0, context.width(), self.height(context));
        let mut row = Bordered::inner(area, self.border_layout()).y;
        let mut items = Vec::new();
        for (index, cell) in self.cells.iter().enumerate() {
            row = row.saturating_add(Self::gap(index));
            let shown = row_count!(self.lines(cell, context).len());
            items.push(
                Item::new(
                    ItemKind::Output,
                    row..row.saturating_add(shown),
                    cell.number,
                    cell.source.text_drawn(),
                )
                .with_type(match &cell.type_drawing {
                    Some(drawing) if !context.math().original_text => Some(drawing.render()),
                    _ => cell.ty.clone(),
                }),
            );
            row = row.saturating_add(shown);
        }
        items
    }

    fn links(&self, context: &RenderContext) -> Vec<Link> {
        let area = Rect::new(0, 0, context.width(), self.height(context));
        let inner = Bordered::inner(area, self.border_layout());
        let mut row = inner.y;
        let mut links = Vec::new();
        for (index, cell) in self.cells.iter().enumerate() {
            row = row.saturating_add(Self::gap(index));
            if let Some((html, example_width)) = cell.markup() {
                links.extend(
                    markup::links(
                        html,
                        example_width,
                        &context.with_width(self.value_width(cell, context)),
                    )
                    .into_iter()
                    .map(|link| self.placed_link(cell, context, Position::new(inner.x, row), link))
                    .filter(|link| !link.cells.is_empty()),
                );
            }
            links.extend(self.drawn_links(cell, context, Position::new(inner.x, row)));
            row = row.saturating_add(row_count!(self.lines(cell, context).len()));
        }
        links
    }

    fn link_at(&self, context: &RenderContext, position: Position) -> Option<Link> {
        let area = Rect::new(0, 0, context.width(), self.height(context));
        let inner = Bordered::inner(area, self.border_layout());
        let mut row = inner.y;
        for (index, cell) in self.cells.iter().enumerate() {
            row = row.saturating_add(Self::gap(index));
            let shown = row_count!(self.lines(cell, context).len());
            if !(row..row.saturating_add(shown)).contains(&position.y) {
                row = row.saturating_add(shown);
                continue;
            }
            let under = self
                .drawn_links(cell, context, Position::new(inner.x, row))
                .find(|link| link.cells.iter().any(|cells| cells.contains(position)));
            if under.is_some() {
                return under;
            }
            let (html, example_width) = cell.markup()?;
            // A folded value shows its head, three fold rows, then its tail.
            let fold = self.cell_fold(cell, context);
            let unfolded = |shown: usize| match fold {
                Some((head, _)) if (head..head + 3).contains(&shown) => None,
                Some((head, hidden)) if shown >= head + 3 => Some(shown + hidden - 3),
                _ => Some(shown),
            };
            let link = markup::link_at(
                html,
                example_width,
                &context.with_width(self.value_width(cell, context)),
                Position::new(
                    position.x.checked_sub(inner.x)?,
                    unfolded(usize::from(position.y - row))? as u16,
                ),
            )?;
            return Some(self.placed_link(cell, context, Position::new(inner.x, row), link));
        }
        None
    }

    fn local_selection_inclusions(&self, context: &RenderContext, inclusions: &mut Inclusions) {
        let area = Rect::new(0, 0, context.width(), self.height(context));
        let layout = self.border_layout();
        let inner = Bordered::inner(area, layout);
        let right = inner.right();
        let mut row = inner.y;
        for (index, cell) in self.cells.iter().enumerate() {
            let gap = Self::gap(index);
            inclusions.push(Rect::new(inner.x, row, inner.width, gap));
            row = row.saturating_add(gap);
            let lines = self.lines(cell, context);
            let unfolded = self.value_inclusions(cell, inner.width, context);
            let at = |row: u16| Offset::new(i32::from(inner.x), i32::from(row));
            match self.cell_fold(cell, context) {
                Some((head, hidden)) => {
                    let (head, tail) = (row_count!(head), row_count!(head + hidden));
                    let total = row_count!(self.unfolded_lines(cell, context).len());
                    inclusions.merge(unfolded.project(0..head) + at(row));
                    inclusions.merge(unfolded.project(tail..total) + at(row + head + FOLD_ROWS));
                }
                None => inclusions.merge(unfolded + at(row)),
            }
            let values = self.value_lines(cell, context);
            let drawn: Vec<String> = values.iter().map(ToString::to_string).collect();
            inclusions.mark_wrapped_rows(
                row,
                area.width,
                drawn.iter().map(String::as_str),
                &Self::unwrapped_text(cell, context),
            );
            let types = self.type_lines(cell, context);
            let width = types.iter().map(Line::width).max().unwrap_or_default() as u16;
            let start =
                Self::type_start(&values, types.len(), width.into(), inner.width.into()) as u16;
            for offset in 0..row_count!(types.len()) {
                inclusions.push_type(Rect::new(
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
pub struct WarningBlock {
    pub text: String,
}

impl Block for WarningBlock {
    fn items(&self, context: &RenderContext) -> Vec<Item> {
        let rows = 0..self.height(context);
        vec![Item::new(ItemKind::Note, rows, None, self.text.clone())]
    }

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

    fn local_selection_inclusions(&self, context: &RenderContext, inclusions: &mut Inclusions) {
        inclusions.push(Bordered::inner(
            Rect::new(0, 0, context.width(), self.height(context)),
            CellLayout::plain().frame(),
        ));
    }
}

/// Bound synchronous work before wrapping flat output, retaining both ends.
/// M2 source wrapped at its token boundaries to `width` and coloured as code.
fn code_lines(source: &str, width: u16, groups: &Groups) -> Vec<Line<'static>> {
    thread_local! {
        static CODE: RefCell<SyntaxHighlighter> = RefCell::new(SyntaxHighlighter::new());
    }
    let spans = CODE.with_borrow_mut(|highlighter| highlighter.highlight(source));
    let boundaries: Vec<Range<usize>> = spans.iter().map(|span| span.content.clone()).collect();
    SpanWrap(&boundaries)
        .wrap(source, width)
        .into_iter()
        .map(|row| {
            let text = &source[row.range.clone()];
            Line::from(spans.paint(row.range.start, text, groups.value(), groups))
        })
        .collect()
}

fn output_preview(text: &str) -> String {
    let budget = UI::MAX_OUTPUT_PREVIEW_BYTES;
    if text.len() <= budget {
        return text.to_string();
    }
    let mut head = budget / 2;
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = text.len() - budget / 2;
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    format!(
        "{}\n{}\n{}",
        &text[..head],
        PREVIEW_TRUNCATED.current(),
        &text[tail..]
    )
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
    let message = message
        .strip_prefix("error:")
        .or_else(|| message.strip_prefix("Error:"))
        .map_or(message.as_str(), str::trim_start);
    word_lines(
        message,
        Bordered::text_width(context, CellLayout::plain().frame()),
    )
}

fn render_error_box(message: &str, context: &RenderContext, area: Rect, buf: &mut Buffer) {
    let theme = context.groups();
    let body = error_box_text(message, context)
        .into_iter()
        .map(|line| Line::from(Span::styled(line, theme.error_message())))
        .collect();
    let title = Some(Line::from(Span::styled(" Error ", theme.error_title())));
    Bordered::new(
        CellLayout::plain().frame(),
        Style::new(),
        theme.error_border(),
        title,
        body,
    )
    .render(&(), area, buf);
}

/// An evaluation error whose submitted code is already present in the feed.
pub struct ErrorMessageBlock {
    pub message: String,
}

impl Block for ErrorMessageBlock {
    fn items(&self, context: &RenderContext) -> Vec<Item> {
        let rows = 0..self.height(context);
        vec![Item::new(ItemKind::Note, rows, None, self.message.clone())]
    }

    fn height(&self, context: &RenderContext) -> u16 {
        row_count!(error_box_text(&self.message, context).len()).saturating_add(2)
    }

    fn render_local(&self, context: &RenderContext, buf: &mut Buffer) {
        let area = *buf.area();
        render_error_box(&self.message, context, area, buf);
    }

    fn local_selection_inclusions(&self, context: &RenderContext, inclusions: &mut Inclusions) {
        inclusions.push(Bordered::inner(
            Rect::new(0, 0, context.width(), self.height(context)),
            CellLayout::plain().frame(),
        ));
    }
}
