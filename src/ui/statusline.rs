//! Event-driven status text rendered below the input.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

const MARGIN: u16 = 2;

/// A request to replace or clear the status line.
pub enum StatusLineEvent {
    Show {
        alignment: Alignment,
        spans: Vec<Span<'static>>,
    },
}

/// One aligned status message supplied by a UI controller.
struct StatusMessage {
    alignment: Alignment,
    spans: Vec<Span<'static>>,
}

/// The current status message and its left, centered, and right composition.
#[derive(Default)]
pub struct StatusLine {
    message: Option<StatusMessage>,
}

impl StatusLine {
    pub fn handle(&mut self, event: StatusLineEvent) {
        self.message = match event {
            StatusLineEvent::Show { alignment, spans } => Some(StatusMessage { alignment, spans }),
        };
    }

    pub fn render(
        &self,
        area: Rect,
        buf: &mut Buffer,
        idle_left: Option<Span<'static>>,
        center: Option<Line<'static>>,
    ) {
        let Some(message) = &self.message else {
            self.render_parts(
                area,
                buf,
                idle_left.into_iter().collect(),
                center,
                Vec::new(),
            );
            return;
        };
        match message.alignment {
            Alignment::Left => {
                self.render_parts(area, buf, message.spans.clone(), center, Vec::new())
            }
            Alignment::Center => self.render_parts(
                area,
                buf,
                idle_left.into_iter().collect(),
                Some(Line::from(message.spans.clone())),
                Vec::new(),
            ),
            Alignment::Right => self.render_parts(
                area,
                buf,
                idle_left.into_iter().collect(),
                center,
                message.spans.clone(),
            ),
        }
    }

    pub fn render_parts(
        &self,
        area: Rect,
        buf: &mut Buffer,
        left: Vec<Span<'static>>,
        center: Option<Line<'static>>,
        right: Vec<Span<'static>>,
    ) {
        let area = Rect::new(
            area.x.saturating_add(MARGIN),
            area.y,
            area.width.saturating_sub(2 * MARGIN),
            area.height,
        );
        if area.width == 0 {
            return;
        }
        let left = Line::from(left);
        let right = Line::from(right);
        let width = usize::from(area.width);
        let center_range = center.as_ref().map(|line| {
            let start = width.saturating_sub(line.width()) / 2;
            start..start + line.width().min(width)
        });
        if !left.spans.is_empty()
            && center_range
                .as_ref()
                .is_none_or(|center| left.width() < center.start)
            && (center_range.is_some()
                || right.spans.is_empty()
                || left.width() + right.width() <= width)
        {
            Paragraph::new(left).render(area, buf);
        }
        if !right.spans.is_empty()
            && center_range
                .as_ref()
                .is_none_or(|center| width.saturating_sub(right.width()) > center.end)
        {
            Paragraph::new(right)
                .alignment(Alignment::Right)
                .render(area, buf);
        }
        if let Some(center) = center {
            Paragraph::new(center)
                .alignment(Alignment::Center)
                .render(area, buf);
        }
    }
}
