//! Typed entries in the scrollable editor feed.
//!
//! Every [`Block`] owns text, measures and renders against a borrowed
//! [`RenderContext`], and inherits the same visual-line wrapper. Highlighted text
//! receives its highlighter separately because highlighting is an application-wide
//! service rather than editor state.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::block::highlight::{HighlightSpan, Highlighter};
use crate::block::layout::{wrap, TextLayout, WordWrap};
use crate::block::render::{
    band, box_text_width, boxed, code_band_lines, gutter_span, CodeBandStyle, RenderContext,
    GUTTER_W,
};
use crate::transcript::strip_m2_location;

pub mod highlight;
pub mod layout;
pub mod render;

pub trait Highlighted {
    fn highlighted_lines<'a>(
        &'a mut self,
        context: &RenderContext<'_>,
        highlighter: &mut dyn Highlighter,
    ) -> Vec<Line<'a>>;
}

pub trait Block {
    fn height(&self, context: &RenderContext<'_>) -> u16;
    fn render(&self, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer);
}

fn joined_text(cells: Vec<String>) -> (String, Vec<usize>) {
    let mut starts = Vec::with_capacity(cells.len());
    let mut offset = 0;
    for cell in &cells {
        starts.push(offset);
        offset += cell.len() + 1;
    }
    (cells.join("\n"), starts)
}

pub struct CodeBlock {
    text: String,
    cell_starts: Vec<usize>,
    spans: Vec<HighlightSpan>,
    start_number: u32,
}

impl CodeBlock {
    pub fn new(cells: Vec<String>, spans: Vec<HighlightSpan>, start_number: u32) -> Self {
        let (text, cell_starts) = joined_text(cells);
        Self {
            text,
            cell_starts,
            spans,
            start_number,
        }
    }
}

impl TextLayout for CodeBlock {
    type Wrapping = WordWrap;

    fn text(&self) -> &str {
        &self.text
    }
}

impl Block for CodeBlock {
    fn height(&self, context: &RenderContext<'_>) -> u16 {
        self.lines(context.width().saturating_sub(GUTTER_W)).len() as u16 + 2
    }

    fn render(&self, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
        let theme = context.theme();
        let base = theme.code();
        let lines = self.lines(context.width().saturating_sub(GUTTER_W));
        let rendered = code_band_lines(
            self.text(),
            &lines,
            &self.cell_starts,
            &self.spans,
            Some(self.start_number),
            CodeBandStyle {
                marker: theme.prompt().patch(base),
                base,
            },
            theme,
        );
        band(area, buf, rendered, base);
    }
}

pub struct OutputBlock {
    pub number: u32,
    pub text: String,
    pub ty: Option<String>,
}

impl OutputBlock {
    fn body(&self, context: &RenderContext<'_>) -> Vec<String> {
        if self.text.is_empty() {
            return Vec::new();
        }
        self.lines(box_text_width(context.width()) as u16)
            .into_iter()
            .map(|line| self.text[line.range].to_string())
            .collect()
    }
}

impl TextLayout for OutputBlock {
    type Wrapping = WordWrap;

    fn text(&self) -> &str {
        &self.text
    }
}

impl Block for OutputBlock {
    fn height(&self, context: &RenderContext<'_>) -> u16 {
        self.body(context).len() as u16 + 2
    }

    fn render(&self, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
        let theme = context.theme();
        let body = self
            .body(context)
            .into_iter()
            .map(|line| Line::from(Span::styled(line, theme.value())))
            .collect();
        let title = Some(Line::from(Span::styled(
            format!(" o{} ", self.number),
            theme.muted(),
        )));
        let bottom = self.ty.as_ref().map(|ty| {
            Line::from(Span::styled(format!(" {ty} "), theme.type_info())).left_aligned()
        });
        boxed(area, buf, theme.border(), title, bottom, body);
    }
}

pub struct WarningBlock {
    pub text: String,
}

impl TextLayout for WarningBlock {
    type Wrapping = WordWrap;

    fn text(&self) -> &str {
        &self.text
    }
}

impl Block for WarningBlock {
    fn height(&self, context: &RenderContext<'_>) -> u16 {
        self.lines(box_text_width(context.width()) as u16).len() as u16 + 2
    }

    fn render(&self, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
        let theme = context.theme();
        let body = self
            .lines(box_text_width(context.width()) as u16)
            .into_iter()
            .map(|line| {
                Line::from(Span::styled(
                    self.text[line.range].to_string(),
                    theme.warning(),
                ))
            })
            .collect();
        let title = Some(Line::from(Span::styled(" Warning ", theme.warning())));
        boxed(area, buf, theme.warning_border(), title, None, body);
    }
}

pub struct RuntimeErrorBlock {
    pub number: u32,
    pub text: String,
    pub spans: Vec<HighlightSpan>,
    pub message: String,
}

fn cleaned_error(message: &str) -> String {
    message
        .lines()
        .map(strip_m2_location)
        .collect::<Vec<_>>()
        .join("\n")
}

fn error_box_text(message: &str, context: &RenderContext<'_>) -> Vec<String> {
    let message = cleaned_error(message);
    wrap::<WordWrap>(&message, box_text_width(context.width()) as u16)
        .into_iter()
        .map(|line| message[line.range].to_string())
        .collect()
}

fn render_error_box(message: &str, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
    let theme = context.theme();
    let body = error_box_text(message, context)
        .into_iter()
        .map(|line| Line::from(Span::styled(line, theme.error())))
        .collect();
    boxed(area, buf, theme.error_border(), None, None, body);
}

fn split_band(area: Rect, band_height: u16) -> (Rect, Rect) {
    let band = Rect::new(area.x, area.y, area.width, band_height);
    let rest = Rect::new(
        area.x,
        area.y + band_height,
        area.width,
        area.height.saturating_sub(band_height),
    );
    (band, rest)
}

impl TextLayout for RuntimeErrorBlock {
    type Wrapping = WordWrap;

    fn text(&self) -> &str {
        &self.text
    }
}

impl Block for RuntimeErrorBlock {
    fn height(&self, context: &RenderContext<'_>) -> u16 {
        self.lines(context.width().saturating_sub(GUTTER_W)).len() as u16
            + 2
            + error_box_text(&self.message, context).len() as u16
            + 2
    }

    fn render(&self, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
        let theme = context.theme();
        let base = theme.code();
        let lines = self.lines(context.width().saturating_sub(GUTTER_W));
        let band_height = lines.len() as u16 + 2;
        let (band_area, box_area) = split_band(area, band_height);
        let rendered = code_band_lines(
            self.text(),
            &lines,
            &[0],
            &self.spans,
            Some(self.number),
            CodeBandStyle {
                marker: theme.error().patch(base),
                base,
            },
            theme,
        );
        band(band_area, buf, rendered, base);
        render_error_box(&self.message, context, box_area, buf);
    }
}

pub struct RejectedBlock {
    text: String,
    cell_starts: Vec<usize>,
    spans: Vec<HighlightSpan>,
}

impl RejectedBlock {
    pub fn new(cells: Vec<String>, spans: Vec<HighlightSpan>) -> Self {
        let (text, cell_starts) = joined_text(cells);
        Self {
            text,
            cell_starts,
            spans,
        }
    }
}

impl TextLayout for RejectedBlock {
    type Wrapping = WordWrap;

    fn text(&self) -> &str {
        &self.text
    }
}

impl Block for RejectedBlock {
    fn height(&self, context: &RenderContext<'_>) -> u16 {
        self.lines(context.width().saturating_sub(GUTTER_W)).len() as u16 + 2
    }

    fn render(&self, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
        let theme = context.theme();
        let base = theme.code();
        let lines = self.lines(context.width().saturating_sub(GUTTER_W));
        let rendered = code_band_lines(
            self.text(),
            &lines,
            &self.cell_starts,
            &self.spans,
            None,
            CodeBandStyle {
                marker: theme.muted().patch(base),
                base,
            },
            theme,
        );
        band(area, buf, rendered, base);
    }
}

pub struct SyntaxErrorBlock {
    pub text: String,
    pub message: String,
}

impl TextLayout for SyntaxErrorBlock {
    type Wrapping = WordWrap;

    fn text(&self) -> &str {
        &self.text
    }
}

impl Block for SyntaxErrorBlock {
    fn height(&self, context: &RenderContext<'_>) -> u16 {
        self.lines(context.width().saturating_sub(GUTTER_W)).len() as u16
            + 2
            + error_box_text(&self.message, context).len() as u16
            + 2
    }

    fn render(&self, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
        let theme = context.theme();
        let base = theme.code();
        let lines = self.lines(context.width().saturating_sub(GUTTER_W));
        let band_height = lines.len() as u16 + 2;
        let (band_area, box_area) = split_band(area, band_height);
        let rendered = code_band_lines(
            self.text(),
            &lines,
            &[0],
            &[],
            None,
            CodeBandStyle {
                marker: theme.muted().patch(base),
                base,
            },
            theme,
        );
        band(band_area, buf, rendered, base);
        render_error_box(&self.message, context, box_area, buf);
    }
}

pub struct SystemBlock {
    pub text: String,
}

impl TextLayout for SystemBlock {
    type Wrapping = WordWrap;

    fn text(&self) -> &str {
        &self.text
    }
}

impl Block for SystemBlock {
    fn height(&self, context: &RenderContext<'_>) -> u16 {
        self.lines(context.width().saturating_sub(GUTTER_W)).len() as u16 + 1
    }

    fn render(&self, context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
        let theme = context.theme();
        let base = theme.transcript();
        let muted = theme.muted().patch(base);
        let mut lines: Vec<Line<'static>> = self
            .lines(context.width().saturating_sub(GUTTER_W))
            .into_iter()
            .map(|line| {
                Line::from(vec![
                    gutter_span("", muted),
                    Span::styled(self.text[line.range].to_string(), muted),
                ])
            })
            .collect();
        lines.push(Line::default());
        band(area, buf, lines, base);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_wrap_preserves_empty_lines_and_m2_identifiers() {
        let block = SystemBlock {
            text: "x' y$z\n\nabcdefghij".to_string(),
        };
        let lines = block.lines(4);
        assert_eq!(lines.len(), 6);
        assert_eq!(&block.text[lines[0].range.clone()], "x'");
        assert_eq!(&block.text[lines[1].range.clone()], "y$z");
        assert!(lines.iter().any(|line| line.range.is_empty()));
    }

    #[test]
    fn system_block_height_includes_each_message_line_and_margin() {
        let theme = crate::theme::Theme::default();
        let context = RenderContext::new(80, &theme);
        let block = SystemBlock {
            text: "commands:\n  $help\n  $code".to_string(),
        };
        assert_eq!(block.height(&context), 4);
    }
}
