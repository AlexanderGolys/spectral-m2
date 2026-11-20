//! A small panel floating over the screen beside a point, showing any content
//! that can lay itself out: what a name is, the candidates for completing it,
//! or whatever else is told where it was asked for.

use std::fmt;

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block as TuiBlock, Clear, Paragraph, Widget};

use ratatui::text::Span;

use crate::code::semantic::query::Signature;
use crate::ui::config::FloatingPanelLimits;
use crate::ui::glyphs::{Glyphs, Spelled};
use crate::ui::render::{markdown, markup, RenderContext};
use crate::ui::theme::scheme;

/// What a floating panel can show.
pub trait FloatingContent {
    /// The content laid out as `context` draws, in its width.
    fn lines(&self, context: &RenderContext) -> Vec<Line<'static>>;

    /// The columns the content needs, never more than the context's width.
    fn width(&self, context: &RenderContext) -> u16 {
        self.lines(context)
            .iter()
            .map(|line| line.width() as u16)
            .max()
            .unwrap_or(0)
            .min(context.width())
    }
}

/// The point a panel floats beside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Anchor {
    /// The input's cursor, wherever it is when the panel is drawn.
    InputCursor,
    /// A point on screen, such as where something was clicked.
    At(Position),
}

/// Content floating beside its anchor until it is dismissed.
pub struct Floating {
    content: Box<dyn FloatingContent>,
    anchor: Anchor,
}

impl fmt::Debug for Floating {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Floating")
            .field("anchor", &self.anchor)
            .finish_non_exhaustive()
    }
}

impl Floating {
    pub fn new(content: impl FloatingContent + 'static, anchor: Anchor) -> Self {
        Self {
            content: Box::new(content),
            anchor,
        }
    }

    /// The panel's place within `area`: above its anchor where it fits and else
    /// below it, `cursor` standing for the input's cursor.
    pub fn area(
        &self,
        area: Rect,
        cursor: Option<Position>,
        context: &RenderContext,
    ) -> Option<Rect> {
        let anchor = match self.anchor {
            Anchor::InputCursor => cursor?,
            Anchor::At(position) => position,
        };
        let inner = self.content.width(
            &context.with_width(
                FloatingPanelLimits::MAX_WIDTH
                    .min(area.width)
                    .saturating_sub(2),
            ),
        );
        let rows = self.rows(&context.with_width(inner), area).len() as u16;
        if inner == 0 || rows == 0 {
            return None;
        }
        let (width, height) = (inner + 2, rows + 2);
        let y = if anchor.y.saturating_sub(area.y) >= height {
            anchor.y - height
        } else {
            (anchor.y + 1).min(area.bottom().saturating_sub(height))
        };
        let x = anchor.x.clamp(area.x, area.right().saturating_sub(width));
        Some(Rect::new(x, y.max(area.y), width, height.min(area.height)))
    }

    /// The content's rows in the context's width, as many as the panel shows
    /// within `area`.
    fn rows(&self, context: &RenderContext, area: Rect) -> Vec<Line<'static>> {
        let mut lines = self.content.lines(context);
        while lines.last().is_some_and(|line| line.width() == 0) {
            lines.pop();
        }
        lines.truncate(usize::from(
            FloatingPanelLimits::MAX_ROWS.min(area.height.saturating_sub(2)),
        ));
        lines
    }

    pub fn draw(
        &self,
        buffer: &mut Buffer,
        area: Rect,
        cursor: Option<Position>,
        context: &RenderContext,
    ) {
        let Some(panel) = self.area(area, cursor, context) else {
            return;
        };
        let block = TuiBlock::bordered()
            .border_set(Glyphs::SQUARE_BORDER.current())
            .style(scheme().transcript().patch(scheme().value()))
            .border_style(scheme().floating_border());
        let lines = self.rows(&context.with_width(panel.width - 2), area);
        Clear.render(panel, buffer);
        Paragraph::new(lines).block(block).render(panel, buffer);
    }
}

/// Documentation markup, drawn as a page is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkupContent {
    html: String,
}

impl MarkupContent {
    pub fn from_markdown(source: &str) -> Self {
        Self {
            html: markdown::html(source),
        }
    }
}

impl FloatingContent for MarkupContent {
    fn lines(&self, context: &RenderContext) -> Vec<Line<'static>> {
        markup::lines(&self.html, usize::from(context.width()), context)
            .map(|lines| lines.iter().cloned().collect())
            .unwrap_or_default()
    }
}

/// A call's signature, the parameter the cursor is at set apart.
impl FloatingContent for Signature {
    fn lines(&self, _context: &RenderContext) -> Vec<Line<'static>> {
        let code = scheme().value();
        let active = self.active.clone().unwrap_or(0..0);
        let parts = [
            (&self.label[..active.start], code),
            (
                &self.label[active.clone()],
                code.patch(scheme().signature_active_parameter()),
            ),
            (&self.label[active.end..], code),
        ];
        vec![Line::from(
            parts
                .into_iter()
                .filter(|(text, _)| !text.is_empty())
                .map(|(text, style)| Span::styled(text.to_string(), style))
                .collect::<Vec<_>>(),
        )]
    }
}

impl FloatingContent for Vec<Signature> {
    fn lines(&self, context: &RenderContext) -> Vec<Line<'static>> {
        self.iter()
            .flat_map(|signature| signature.lines(context))
            .collect()
    }
}
