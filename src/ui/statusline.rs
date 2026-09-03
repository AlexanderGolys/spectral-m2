//! Event-driven status text rendered below the input.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::widgets::{Paragraph, Widget};

use super::config::UI;
use super::controller::settings::theme::groups::Groups;
use super::element::{Element, Text, TextSpan};

/// A request to replace or clear the status line.
pub enum StatusLineEvent {
    Show {
        alignment: Alignment,
        spans: Vec<TextSpan>,
    },
}

/// One aligned status message supplied by a UI controller.
struct StatusMessage {
    alignment: Alignment,
    spans: Vec<TextSpan>,
}

/// The current status message and its left, centered, and right composition.
#[derive(Default)]
pub struct StatusLine {
    message: Option<StatusMessage>,
}

/// Content supplied while the status line is rendered.
pub struct StatusLineContext {
    pub groups: Groups,
    pub idle_left: Option<TextSpan>,
    pub center: Option<Vec<TextSpan>>,
    pub right: Vec<TextSpan>,
}

impl StatusLineContext {
    pub fn new(groups: Groups) -> Self {
        Self {
            groups,
            idle_left: None,
            center: None,
            right: Vec::new(),
        }
    }
}

impl StatusLine {
    pub fn handle(&mut self, event: StatusLineEvent) {
        self.message = match event {
            StatusLineEvent::Show { alignment, spans } => Some(StatusMessage { alignment, spans }),
        };
    }
}

impl Element for StatusLine {
    type Context = StatusLineContext;

    fn render(&self, context: &StatusLineContext, area: Rect, buf: &mut Buffer) {
        let (left, center, right) = match &self.message {
            None => (
                context.idle_left.clone().into_iter().collect(),
                context.center.clone(),
                context.right.clone(),
            ),
            Some(message) => match message.alignment {
                Alignment::Left => (message.spans.clone(), context.center.clone(), Vec::new()),
                Alignment::Center => (
                    context.idle_left.clone().into_iter().collect(),
                    Some(message.spans.clone()),
                    Vec::new(),
                ),
                Alignment::Right => (
                    context.idle_left.clone().into_iter().collect(),
                    context.center.clone(),
                    message.spans.clone(),
                ),
            },
        };
        let area = Rect::new(
            area.x.saturating_add(UI.statusline_margin),
            area.y,
            area.width.saturating_sub(2 * UI.statusline_margin),
            area.height,
        );
        if area.width == 0 {
            return;
        }
        let left = left.line(&context.groups);
        let center = center.map(|spans| spans.line(&context.groups));
        let right = right.line(&context.groups);
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
