//! Shared shape renderers behind every [`Block`](super::Block). Two shapes cover
//! the feed: a full-width *code band* and an inset bordered *box*. Each block's
//! `render` is a thin call into here, so no block re-implements drawing. Measurement
//! and rendering consume the same visual-line ranges produced by the shared layout
//! policies.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as TuiBlock, BorderType, Padding, Paragraph, Widget};

use crate::code::highlight::HLSpan;
use crate::ui::Groups;

/// Runtime rendering inputs shared by every block without owning application state.
pub struct RenderContext<'a> {
    width: u16,
    gutter: u16,
    /// Is M2 sitting in its debugger? Frame-wide, because the whole editor
    /// changes appearance rather than any one block.
    debugging: bool,
    animation_frame: usize,
    groups: &'a Groups,
}

impl<'a> RenderContext<'a> {
    pub fn new(width: u16, gutter: u16, groups: &'a Groups) -> Self {
        Self {
            width,
            gutter,
            debugging: false,
            animation_frame: 0,
            groups,
        }
    }

    pub fn debugging(mut self, debugging: bool) -> Self {
        self.debugging = debugging;
        self
    }

    pub fn animation_frame(mut self, frame: usize) -> Self {
        self.animation_frame = frame;
        self
    }

    pub fn is_debugging(&self) -> bool {
        self.debugging
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    /// Width of the left marker column for this frame.
    pub fn gutter(&self) -> u16 {
        self.gutter
    }

    pub fn groups(&self) -> &'a Groups {
        self.groups
    }

    pub fn spinner(&self) -> &'static str {
        const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        FRAMES[self.animation_frame % FRAMES.len()]
    }
}

const MARKER_MARGIN: u16 = 2;
const BORDER_MARGIN: u16 = 1;
const MARKER_TEXT_GAP: u16 = 2;
const BORDER_LABEL_GAP: u16 = 1;
const RIGHT_MARGIN: u16 = 3;
const MIN_BOX_W: u16 = 8;

fn marker_width(number: u32) -> u16 {
    1 + number.max(1).ilog10() as u16 + 1
}

pub fn gutter_width(number: u32) -> u16 {
    MARKER_MARGIN + marker_width(number) + MARKER_TEXT_GAP
}

pub fn prompt_gutter_width() -> u16 {
    MARKER_MARGIN + 1 + MARKER_TEXT_GAP
}

pub fn border_label(area: Rect, buf: &mut Buffer, label: String, style: Style) {
    let width = label.chars().count() as u16 + BORDER_LABEL_GAP;
    Paragraph::new(Span::styled(format!("{label} "), style))
        .render(Rect::new(area.x + MARKER_MARGIN, area.y, width, 1), buf);
}

/// Geometry of an ordinary text box or an output box aligned to the code gutter.
#[derive(Clone, Copy)]
pub enum BoxLayout {
    Text,
    Output {
        marker_gutter: u16,
        content_gutter: u16,
    },
}

pub fn gutter_span(label: &str, gutter: u16, style: Style) -> Span<'static> {
    let trailing =
        usize::from(gutter).saturating_sub(usize::from(MARKER_MARGIN) + label.chars().count());
    let text = format!(
        "{}{}{}",
        " ".repeat(MARKER_MARGIN.into()),
        label,
        " ".repeat(trailing)
    );
    Span::styled(text, style)
}

/// Semantic content displayed by the code block's cell-number column.
#[derive(Clone, Copy)]
pub enum CellMarker {
    Empty,
    Number(u32),
    Pending(&'static str),
    Queued,
    HardContinuation,
}

/// Paint and layout of a code block, including its visually independent cell
/// marker column.
pub struct CodeBlockStyle {
    base: Style,
    column: Style,
}

impl CodeBlockStyle {
    /// Create the ordinary input-cell style for the active theme.
    pub fn input(groups: &Groups) -> Self {
        let base = groups.code();
        Self {
            base,
            column: groups.muted().patch(base),
        }
    }

    /// Create the inverted source style used above a syntax tree.
    pub fn preview(groups: &Groups) -> Self {
        let base = groups.transcript();
        Self {
            base,
            column: groups.muted().patch(base),
        }
    }

    /// Create the failed-input style for the active theme.
    pub fn error(groups: &Groups) -> Self {
        let base = groups.code();
        Self {
            base,
            column: groups.error().patch(base),
        }
    }

    /// Return the background style shared by the source and marker column.
    pub fn base(&self) -> Style {
        self.base
    }

    /// Render the complete marker column for one code row.
    pub fn column(&self, marker: CellMarker, width: u16) -> Vec<Span<'static>> {
        let label = match marker {
            CellMarker::Empty => String::new(),
            CellMarker::Number(number) => format!("i{number}"),
            CellMarker::Pending(spinner) => spinner.to_string(),
            CellMarker::Queued => "…".to_string(),
            CellMarker::HardContinuation => "󰌑".to_string(),
        };
        vec![gutter_span(&label, width, self.column)]
    }
}

/// Colour one byte-range slice `seg` (starting at absolute offset `abs_start` in
/// the highlighted source) into styled spans: gaps between highlight spans take
/// `base`, covered ranges take `base` patched with the kind's style. Assumes
/// `spans` are source-ordered and non-overlapping, as the highlighter emits them.
pub fn apply_spans(
    abs_start: usize,
    seg: &str,
    spans: &[HLSpan],
    base: Style,
    groups: &Groups,
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
            base.patch(groups.macaulay(s.kind)),
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

fn box_left_padding() -> u16 {
    MARKER_TEXT_GAP + MARKER_MARGIN - BORDER_MARGIN
}

impl BoxLayout {
    fn inset(self) -> u16 {
        match self {
            Self::Text => BORDER_MARGIN + marker_width(1) - 1,
            Self::Output { marker_gutter, .. } => {
                marker_gutter.saturating_sub(MARKER_MARGIN + MARKER_TEXT_GAP)
            }
        }
    }

    fn padding(self) -> Padding {
        match self {
            Self::Text => Padding::new(box_left_padding(), 1, 0, 0),
            Self::Output { content_gutter, .. } => {
                Padding::new(content_gutter.saturating_sub(self.inset() + 1), 2, 0, 0)
            }
        }
    }
}

pub fn box_area(area: Rect, layout: BoxLayout) -> Rect {
    let inset = layout.inset();
    let width = area
        .width
        .saturating_sub(inset + RIGHT_MARGIN)
        .max(MIN_BOX_W);
    Rect::new(area.x + inset, area.y, width, area.height)
}

pub fn box_inner_area(area: Rect, layout: BoxLayout) -> Rect {
    TuiBlock::bordered()
        .padding(layout.padding())
        .inner(box_area(area, layout))
}

pub fn box_text_width(context: &RenderContext<'_>, layout: BoxLayout) -> u16 {
    box_inner_area(Rect::new(0, 0, context.width(), 0), layout).width
}

pub fn boxed(
    area: Rect,
    buf: &mut Buffer,
    layout: BoxLayout,
    border_style: Style,
    title: Option<Line<'static>>,
    bottom: Option<Line<'static>>,
    body: Vec<Line<'static>>,
) {
    let box_area = box_area(area, layout);
    let mut block = TuiBlock::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .padding(layout.padding());
    if let Some(t) = title {
        block = block.title(t);
    }
    if let Some(b) = bottom {
        block = block.title_bottom(b);
    }
    let inner = box_inner_area(area, layout);
    block.render(box_area, buf);
    Paragraph::new(body).render(inner, buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_gutter_tracks_the_number_width() {
        assert_eq!(gutter_width(9), 6);
        assert_eq!(gutter_width(99), 7);
        assert_eq!(gutter_width(100), 8);
        assert_eq!(gutter_width(1000), 9);
    }

    #[test]
    fn markers_have_two_spaces_on_each_side() {
        let narrow = gutter_span("i9", gutter_width(9), Style::default());
        let wide = gutter_span("i99", gutter_width(99), Style::default());
        assert_eq!(narrow.content.as_ref(), "  i9  ");
        assert_eq!(wide.content.as_ref(), "  i99  ");
    }

    #[test]
    fn output_border_crosses_the_second_last_marker_character() {
        for number in [1u32, 42, 100, 5000] {
            let layout = BoxLayout::Output {
                marker_gutter: gutter_width(number),
                content_gutter: gutter_width(5000),
            };
            assert_eq!(layout.inset(), BORDER_MARGIN + marker_width(number) - 1);
        }
    }

    #[test]
    fn box_body_starts_two_spaces_after_the_output_marker() {
        for number in [1u32, 42, 100, 5000] {
            let layout = BoxLayout::Output {
                marker_gutter: gutter_width(number),
                content_gutter: gutter_width(5000),
            };
            assert_eq!(
                layout.inset() + 1 + layout.padding().left,
                gutter_width(5000),
                "misaligned at o{number}"
            );
        }
    }
}
