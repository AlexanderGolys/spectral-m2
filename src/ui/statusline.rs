//! Event-driven status text rendered below the input.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Modifier;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};

use super::block::{Element, Text, TextSpan};
use super::config::UI;
use super::controller::settings::theme::groups::{scheme, Groups, HLGroup};

/// A request to replace or clear the status line.
pub(crate) enum StatusLineEvent {
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
pub(crate) struct StatusLine {
    message: Option<StatusMessage>,
}

/// Content supplied while the status line is rendered, painted in the published
/// scheme.
#[derive(Default)]
pub struct StatusLineContext {
    pub idle_left: Option<TextSpan>,
    pub center: Option<Vec<TextSpan>>,
    pub right: Vec<TextSpan>,
}

impl StatusLine {
    pub(crate) fn handle(&mut self, event: StatusLineEvent) {
        self.message = match event {
            StatusLineEvent::Show { alignment, spans } => Some(StatusMessage { alignment, spans }),
        };
    }

    fn line(spans: &[TextSpan], groups: &Groups, group: HLGroup) -> Line<'static> {
        let mut line = spans.line(groups);
        for (source, span) in spans.iter().zip(&mut line.spans) {
            if !matches!(
                source.kind,
                HLGroup::KeymapHint
                    | HLGroup::Muted
                    | HLGroup::StatuslineError
                    | HLGroup::StatuslineWarning
            ) {
                span.style = groups.style(group);
            }
            span.style = span.style.remove_modifier(Modifier::UNDERLINED);
            span.style.underline_color = None;
        }
        line
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
        let groups = scheme();
        let left = Self::line(&left, groups, HLGroup::Statusline1);
        let center = center.map(|spans| Self::line(&spans, groups, HLGroup::Statusline2));
        let right = Self::line(&right, groups, HLGroup::Statusline3);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::controller::settings::theme::groups::HLGroup;

    #[test]
    fn rendered_text_never_keeps_an_underline() {
        let groups = *scheme();
        let context = StatusLineContext {
            center: Some(vec![TextSpan::new("status", HLGroup::Documentation)]),
            ..StatusLineContext::default()
        };
        let area = Rect::new(0, 0, 20, 1);
        let mut buffer = Buffer::empty(area);

        StatusLine::default().render(&context, area, &mut buffer);

        let cell = &buffer[(7, 0)];
        assert_eq!(cell.fg, groups.style(HLGroup::Statusline2).fg.unwrap());
        assert!(!cell.modifier.contains(Modifier::UNDERLINED));
        assert_eq!(cell.underline_color, ratatui::style::Color::Reset);
    }

    #[test]
    fn key_hints_keep_their_accent_while_labels_stay_muted() {
        let groups = Groups::default();
        let line = StatusLine::line(
            &[
                TextSpan::new("ctrl+/ ", HLGroup::KeymapHint),
                TextSpan::new("commands", HLGroup::Muted),
            ],
            &groups,
            HLGroup::Statusline3,
        );

        assert_eq!(line.spans[0].style.fg, groups.style(HLGroup::KeymapHint).fg);
        assert_eq!(line.spans[1].style.fg, groups.style(HLGroup::Muted).fg);
    }
}
