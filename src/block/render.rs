//! Shared shape renderers behind every [`Block`](super::Block). Two shapes cover
//! the feed: a full-width *code band* (a gutter column of `iN` markers beside
//! highlighted, soft-wrapped source) and an inset bordered *box* (results,
//! warnings, errors). Each block's `render` is a thin call into here, so no block
//! re-implements drawing. Measurement and rendering consume the same visual-line
//! ranges produced by the shared layout policies.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as TuiBlock, BorderType, Padding, Paragraph, Widget};

use crate::block::highlight::HighlightSpan;
use crate::block::layout::VisualLine;
use crate::theme::Theme;

/// Runtime rendering inputs shared by every block without owning application state.
pub struct RenderContext<'a> {
    width: u16,
    theme: &'a Theme,
}

impl<'a> RenderContext<'a> {
    pub fn new(width: u16, theme: &'a Theme) -> Self {
        Self { width, theme }
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn theme(&self) -> &'a Theme {
        self.theme
    }
}

/// Width of the left status column (right-aligned `iN` / prompt marker), in cells.
pub const GUTTER_W: u16 = 5;
const MIN_BOX_W: u16 = 8;

pub struct CodeBandStyle {
    pub marker: Style,
    pub base: Style,
}

/// The left status column: a right-aligned marker padded to [`GUTTER_W`], so it
/// reads like an editor's gutter.
pub fn gutter_span(label: &str, style: Style) -> Span<'static> {
    let w = GUTTER_W as usize;
    let label_w = label.chars().count();
    let pad = w.saturating_sub(label_w + 1);
    let trailing = w.saturating_sub(pad + label_w);
    let text = format!("{}{}{}", " ".repeat(pad), label, " ".repeat(trailing));
    Span::styled(text, style.bold())
}

/// Build the lines of a code band: a top margin, then each cell's source soft-
/// wrapped and coloured by `spans`, the first visual row of each cell carrying
/// its `iN` marker (when `start_number` is set), then a bottom margin.
pub fn code_band_lines(
    text: &str,
    lines: &[VisualLine],
    cell_starts: &[usize],
    spans: &[HighlightSpan],
    start_number: Option<u32>,
    style: CodeBandStyle,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut rendered = vec![Line::default()];
    for line in lines {
        let cell = cell_starts
            .iter()
            .position(|start| *start == line.range.start);
        let marker = match (cell, start_number) {
            (Some(index), Some(start)) => format!("i{}", start + index as u32),
            _ => String::new(),
        };
        let segment = &text[line.range.clone()];
        let mut output = vec![gutter_span(&marker, style.marker)];
        output.extend(apply_spans(
            line.range.start,
            segment,
            spans,
            style.base,
            theme,
        ));
        rendered.push(Line::from(output));
    }
    rendered.push(Line::default());
    rendered
}

/// Colour one byte-range slice `seg` (starting at absolute offset `abs_start` in
/// the highlighted source) into styled spans: gaps between highlight spans take
/// `base`, covered ranges take `base` patched with the kind's style. Assumes
/// `spans` are source-ordered and non-overlapping, as the highlighter emits them.
pub fn apply_spans(
    abs_start: usize,
    seg: &str,
    spans: &[HighlightSpan],
    base: Style,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let seg_end = abs_start + seg.len();
    let mut out = Vec::new();
    let mut cur = abs_start;
    for s in spans
        .iter()
        .filter(|s| s.range.start < seg_end && s.range.end > abs_start)
    {
        let st = s.range.start.max(abs_start);
        let en = s.range.end.min(seg_end);
        if st > cur {
            out.push(Span::styled(
                seg[cur - abs_start..st - abs_start].to_string(),
                base,
            ));
        }
        out.push(Span::styled(
            seg[st - abs_start..en - abs_start].to_string(),
            base.patch(theme.syntax(s.kind)),
        ));
        cur = en;
    }
    if cur < seg_end {
        out.push(Span::styled(seg[cur - abs_start..].to_string(), base));
    }
    if out.is_empty() {
        out.push(Span::styled(seg.to_string(), base));
    }
    out
}

/// Render a full-width band (code or system message) filling `area`.
pub fn band(area: Rect, buf: &mut Buffer, lines: Vec<Line<'static>>, base: Style) {
    Paragraph::new(lines).style(base).render(area, buf);
}

/// The usable text width inside a result box at terminal `width` — the box minus
/// its two borders and two padding columns. Used to pre-wrap box bodies.
pub fn box_text_width(width: u16) -> usize {
    (width.saturating_sub(2 * GUTTER_W).max(MIN_BOX_W) as usize).saturating_sub(4)
}

/// Render a bordered result box, inset by one [`GUTTER_W`] gutter on each side so
/// it lines up under the code column. `area.height` must be `body.len() + 2`.
pub fn boxed(
    area: Rect,
    buf: &mut Buffer,
    border_style: Style,
    title: Option<Line<'static>>,
    bottom: Option<Line<'static>>,
    body: Vec<Line<'static>>,
) {
    let box_w = area.width.saturating_sub(2 * GUTTER_W).max(MIN_BOX_W);
    let box_area = Rect::new(area.x + GUTTER_W, area.y, box_w, area.height);
    let mut block = TuiBlock::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .padding(Padding::horizontal(1));
    if let Some(t) = title {
        block = block.title(t);
    }
    if let Some(b) = bottom {
        block = block.title_bottom(b);
    }
    let inner = block.inner(box_area);
    block.render(box_area, buf);
    Paragraph::new(body).render(inner, buf);
}
