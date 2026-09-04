//! Common composition traits for terminal UI elements.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use std::ops::Range;

use super::controller::settings::theme::groups::{Groups, HLGroup};

/// Semantic content whose style is resolved from a highlight group at render time.
#[derive(Clone)]
pub struct HLSpan<T = Range<usize>> {
    pub content: T,
    pub kind: HLGroup,
}

impl<T> HLSpan<T> {
    pub fn new(content: impl Into<T>, kind: HLGroup) -> Self {
        Self {
            content: content.into(),
            kind,
        }
    }
}

/// Owned semantic text used by UI elements.
pub type TextSpan = HLSpan<String>;

/// A renderable UI element with an explicit rendering context.
pub trait Element: Send {
    type Context: ?Sized;

    fn render(&self, context: &Self::Context, area: Rect, buf: &mut Buffer);

    fn height(&self, _context: &Self::Context) -> u16 {
        1
    }
}

/// An element renderedh over a styled rectangular surface.
pub trait StyleElement: Element {
    fn render_styled(&self, context: &Self::Context, area: Rect, buf: &mut Buffer, style: Style) {
        buf.set_style(area, style);
        self.render(context, area, buf);
    }
}

impl<T: Element + ?Sized> StyleElement for T {}

/// A UI element represented by one line of styled text.
pub trait Text: Send {
    type Context: ?Sized;

    fn spans(&self, context: &Self::Context) -> Vec<Span<'static>>;

    fn line(&self, context: &Self::Context) -> Line<'static> {
        Line::from(self.spans(context))
    }
}

impl HLSpan<String> {
    fn resolved(&self, groups: &Groups) -> Span<'static> {
        Span::styled(self.content.clone(), groups.style(self.kind))
    }
}

impl Text for TextSpan {
    type Context = Groups;
    fn spans(&self, groups: &Groups) -> Vec<Span<'static>> {
        vec![self.resolved(groups)]
    }
}

impl Text for [TextSpan] {
    type Context = Groups;
    fn spans(&self, groups: &Groups) -> Vec<Span<'static>> {
        self.iter().map(|span| span.resolved(groups)).collect()
    }
}

impl<T: Text + ?Sized> Element for T {
    type Context = T::Context;

    fn render(&self, context: &Self::Context, area: Rect, buf: &mut Buffer) {
        Paragraph::new(self.line(context)).render(area, buf);
    }
}

/// Range operations on already-styled text spans.
pub trait SpanText {
    fn slice(&self, range: &Range<usize>) -> Vec<Span<'static>>;
}

impl SpanText for [Span<'static>] {
    fn slice(&self, range: &Range<usize>) -> Vec<Span<'static>> {
        let mut offset = 0;
        self.iter()
            .filter_map(|span| {
                let start = offset;
                let end = start + span.content.len();
                offset = end;
                let from = range.start.max(start);
                let to = range.end.min(end);
                (from < to).then(|| {
                    Span::styled(
                        span.content[from - start..to - start].to_string(),
                        span.style,
                    )
                })
            })
            .collect()
    }
}
