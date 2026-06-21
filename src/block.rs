//! The feed model and its trait layer.
//!
//! The scrollable feed is a `Vec<Box<dyn Block>>` of homogeneous, typed entries — the
//! block *type* carries the fate, so there is no per-cell state enum. A submission maps
//! to a *sequence* of blocks: a successful run is a [`CodeBlock`] plus its
//! [`OutputBlock`]s / [`WarningBlock`]s; a runtime error splits into
//! [`CodeBlock`] + [`RuntimeErrorBlock`] + [`RejectedBlock`]; a syntax error is a lone
//! [`SyntaxErrorBlock`].
//!
//! A [`Cell`] is the unit of *execution* — `{ code }`, one complete, oracle-evaluable
//! statement we run on its own and stop at on error. Number, output, error, rejection
//! AND highlight are all properties of the *display blocks around it*, never fields on
//! the cell.
//!
//! Traits, by axis:
//! - [`Block`] — a renderable feed entry (`height` + `render`), by `&self`, kept
//!   separate from ratatui's `Widget` so `Box<dyn Block>` works and so the live input
//!   can be both a feed block and a pinned widget.
//! - [`TextContainer`] — text + measurement + wrapping; every text-bearing unit.
//! - [`Highlighted`] — highlight over text: carrying spans and rendering them colored.
//!   Highlight is computed *upstream* on the live buffer — where the context for
//!   accurate coloring exists, before cell boundaries are even knowable — via an
//!   injected [`Highlighter`](highlight::Highlighter); the code-bearing display blocks
//!   inherit a sliced subset. Worn by the live input, never by a `Cell` (an execution
//!   unit, not a display one).
//!
//! Storage is the canonical owner; render shape (`Vec<Line>`) is built per frame and
//! borrows back into these strings — no `Rc`, no pre-wrapped lines, no baked `Style`
//! (the theme resolves `HighlightKind` at render). Orthogonal to the two things that
//! change without the text: width and theme.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::block::highlight::{HighlightSpan, Highlighter};
use crate::block::render::{
    band, box_text_width, boxed, code_band_height, code_band_lines, gutter_span,
};
use crate::theme::Theme;
use crate::transcript::{strip_m2_location, wrap_chars};

pub mod highlight;
pub mod render;
pub mod wrap;

/// Title shown on a warning box, also the keyword stripped from its messages.
const WARN_TITLE: &str = "Warning";

/// The byte ranges of `cells` joined by `'\n'` — the coordinate space a code
/// band's `spans` index into. Borrowed; nothing is cloned.
fn cell_strs(cells: &[Cell]) -> Vec<&str> {
    cells.iter().map(|c| c.code.as_str()).collect()
}

/// Read access to the source a unit carries — the common supertrait of
/// [`Highlighted`], which highlights `self.text()`.
pub trait TextContainer {
    fn text(&self) -> &str;
}

/// Highlighted text over [`TextContainer`]: the general capability of carrying highlight
/// spans and rendering them coloured. Spans are a private cache — `rehighlight` writes
/// them through an injected [`Highlighter`](highlight::Highlighter), `highlighted_lines`
/// reads them; they never leave the impl.
pub trait Highlighted: TextContainer {
    fn rehighlight(&mut self, hl: &mut dyn Highlighter);
    fn highlighted_lines(&self, width: u16, theme: &Theme) -> Vec<Line<'_>>;
}

/// One renderable entry in the `Vec<Box<dyn Block>>` feed. By `&self` (not ratatui's
/// `Widget`, which consumes `self` and is `Sized`-gated) so trait objects work and the
/// block is redrawn every frame without being consumed.
pub trait Block {
    fn height(&self, width: u16) -> u16;
    fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme);
}

/// The unit of execution: one complete, oracle-evaluable statement, run on its own and
/// stopped at on error. Highlight is not its concern — that lives on the display blocks.
pub struct Cell {
    pub code: String,
}

impl TextContainer for Cell {
    fn text(&self) -> &str {
        &self.code
    }
}

/// The echoed input of a successful submission: its `n` cells, grouped and highlighted.
/// Each cell's `iN` is `start_number + index`; `spans` is the highlight inherited from
/// the live buffer at submit, over the code this block shows.
pub struct CodeBlock {
    pub cells: Vec<Cell>,
    pub spans: Vec<HighlightSpan>,
    pub start_number: u32,
}

impl Block for CodeBlock {
    fn height(&self, width: u16) -> u16 {
        code_band_height(&cell_strs(&self.cells), width)
    }

    fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let base = theme.ui.code_bg;
        let marker = theme.ui.prompt.patch(base);
        let lines = code_band_lines(
            &cell_strs(&self.cells),
            &self.spans,
            Some(self.start_number),
            marker,
            base,
            theme,
            area.width,
        );
        band(area, buf, lines, base);
    }
}

/// One executed cell's result — `oN = value : ty` in a bordered box. A suppressed cell
/// (`;`) yields no `OutputBlock` at all.
pub struct OutputBlock {
    pub number: u32,
    pub value: String,
    pub ty: Option<String>,
}

impl OutputBlock {
    /// The value text wrapped to the box's inner width — the box body, one entry
    /// per visual row. Empty when the cell produced only a type annotation.
    fn body_text(&self, width: u16) -> Vec<String> {
        let tw = box_text_width(width);
        self.value.lines().flat_map(|l| wrap_chars(l, tw)).collect()
    }
}

impl Block for OutputBlock {
    fn height(&self, width: u16) -> u16 {
        self.body_text(width).len() as u16 + 2
    }

    fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let body = self
            .body_text(area.width)
            .into_iter()
            .map(|l| Line::from(Span::styled(l, theme.ui.value)))
            .collect();
        let title = Some(Line::from(Span::styled(
            format!(" o{} ", self.number),
            theme.ui.muted,
        )));
        let bottom = self.ty.as_ref().map(|t| {
            Line::from(Span::styled(format!(" {t} "), theme.ui.type_info)).right_aligned()
        });
        boxed(area, buf, theme.ui.border, title, bottom, body);
    }
}

/// A warning M2 emitted, in an orange-bordered box.
pub struct WarningBlock {
    pub message: String,
}

impl WarningBlock {
    fn body_text(&self, width: u16) -> Vec<String> {
        wrap_chars(&self.message, box_text_width(width))
    }
}

impl Block for WarningBlock {
    fn height(&self, width: u16) -> u16 {
        self.body_text(width).len() as u16 + 2
    }

    fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let body = self
            .body_text(area.width)
            .into_iter()
            .map(|l| Line::from(Span::styled(l, theme.ui.warning)))
            .collect();
        let title = Some(Line::from(Span::styled(
            format!(" {WARN_TITLE} "),
            theme.ui.warning,
        )));
        boxed(area, buf, theme.ui.warning_border, title, None, body);
    }
}

/// The single cell whose evaluation raised a runtime error. Numbered — it reached the
/// reader — and carries M2's message. Execution stops here; the tail is a [`RejectedBlock`].
pub struct RuntimeErrorBlock {
    pub number: u32,
    pub cell: Cell,
    pub spans: Vec<HighlightSpan>,
    pub message: String,
}

/// The wrapped lines of an error message (each stripped of M2's location prefix).
fn error_box_text(message: &str, width: u16) -> Vec<String> {
    let tw = box_text_width(width);
    message
        .lines()
        .flat_map(|l| wrap_chars(strip_m2_location(l), tw))
        .collect()
}

/// Draw an error message as a red-bordered box filling `area`.
fn render_error_box(message: &str, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let body = error_box_text(message, area.width)
        .into_iter()
        .map(|l| Line::from(Span::styled(l, theme.ui.error)))
        .collect();
    boxed(area, buf, theme.ui.error_border, None, None, body);
}

/// Split `area` into a top band of `band_h` rows and the remaining box rows.
fn split_band(area: Rect, band_h: u16) -> (Rect, Rect) {
    let band = Rect::new(area.x, area.y, area.width, band_h);
    let rest = Rect::new(
        area.x,
        area.y + band_h,
        area.width,
        area.height.saturating_sub(band_h),
    );
    (band, rest)
}

impl Block for RuntimeErrorBlock {
    fn height(&self, width: u16) -> u16 {
        code_band_height(&[&self.cell.code], width)
            + error_box_text(&self.message, width).len() as u16
            + 2
    }

    fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let base = theme.ui.code_bg;
        let marker = theme.ui.error.patch(base);
        let band_h = code_band_height(&[&self.cell.code], area.width);
        let (band_area, box_area) = split_band(area, band_h);
        let lines = code_band_lines(
            &[&self.cell.code],
            &self.spans,
            Some(self.number),
            marker,
            base,
            theme,
            area.width,
        );
        band(band_area, buf, lines, base);
        render_error_box(&self.message, box_area, buf, theme);
    }
}

/// The unexecuted tail after a runtime error — never sent to M2, so unnumbered.
pub struct RejectedBlock {
    pub cells: Vec<Cell>,
    pub spans: Vec<HighlightSpan>,
}

impl Block for RejectedBlock {
    fn height(&self, width: u16) -> u16 {
        code_band_height(&cell_strs(&self.cells), width)
    }

    fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let base = theme.ui.code_bg;
        let marker = theme.ui.muted.patch(base);
        let lines = code_band_lines(
            &cell_strs(&self.cells),
            &self.spans,
            None,
            marker,
            base,
            theme,
            area.width,
        );
        band(area, buf, lines, base);
    }
}

/// A whole submission the validation gate refused before M2 ran it — unnumbered, shown
/// with its diagnostic.
pub struct SyntaxErrorBlock {
    pub code: String,
    pub message: String,
}

impl Block for SyntaxErrorBlock {
    fn height(&self, width: u16) -> u16 {
        code_band_height(&[&self.code], width)
            + error_box_text(&self.message, width).len() as u16
            + 2
    }

    fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let base = theme.ui.code_bg;
        let marker = theme.ui.muted.patch(base);
        let band_h = code_band_height(&[&self.code], area.width);
        let (band_area, box_area) = split_band(area, band_h);
        // The rejected input is never highlighted -- it did not parse.
        let lines = code_band_lines(&[&self.code], &[], None, marker, base, theme, area.width);
        band(band_area, buf, lines, base);
        render_error_box(&self.message, box_area, buf, theme);
    }
}

/// A REPL-originated note (banner, connection status, command result) — not M2 source.
pub struct SystemBlock {
    pub message: String,
}

impl Block for SystemBlock {
    fn height(&self, _width: u16) -> u16 {
        2
    }

    fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let base = theme.ui.transcript_bg;
        let muted = theme.ui.muted.patch(base);
        let lines = vec![
            Line::from(vec![
                gutter_span("", muted),
                Span::styled(self.message.clone(), muted),
            ]),
            Line::default(),
        ];
        band(area, buf, lines, base);
    }
}
