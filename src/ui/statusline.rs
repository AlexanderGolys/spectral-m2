//! Event-driven status text rendered below the input.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Modifier;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::block::{Element, Text, TextSpan};
use super::config::UI;
use super::controller::settings::theme::groups::{scheme, Groups, HLGroup};

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

/// Where the session works: its colour scheme and directory.
pub struct StatusLocation {
    pub scheme: String,
    pub directory: String,
}

/// Content supplied while the status line is rendered, painted in the published
/// scheme.
#[derive(Default)]
pub struct StatusLineContext {
    /// Leads the idle left side, its directory trimmed to keep that side within
    /// half the line.
    pub location: Option<StatusLocation>,
    pub idle_left: Option<TextSpan>,
    pub center: Option<Vec<TextSpan>>,
    pub right: Vec<TextSpan>,
}

impl StatusLineContext {
    const SEPARATOR: &str = " · ";

    /// The location, then the session's state, within half of `width` columns.
    fn idle_left(&self, width: usize) -> Vec<TextSpan> {
        let Some(location) = &self.location else {
            return self.idle_left.clone().into_iter().collect();
        };
        let separator = || TextSpan::new(Self::SEPARATOR, HLGroup::StatuslineSeparator);
        let state = self
            .idle_left
            .iter()
            .flat_map(|state| [separator(), state.clone()])
            .collect::<Vec<_>>();
        let fixed = [&location.scheme, Self::SEPARATOR]
            .into_iter()
            .chain(state.iter().map(|span| &span.content[..]))
            .map(UnicodeWidthStr::width)
            .sum::<usize>();
        let directory = trimmed(&location.directory, (width / 2).saturating_sub(fixed));
        let mut spans = vec![TextSpan::new(
            location.scheme.clone(),
            HLGroup::StatuslineScheme,
        )];
        if !directory.is_empty() {
            spans.extend([separator(), TextSpan::new(directory, HLGroup::Statusline1)]);
        }
        spans.extend(state);
        spans
    }
}

/// `path` within `width` columns: its leading components give way to the trimmed
/// glyph first, then the leading characters of its last; empty when not even the
/// glyph fits beside one character.
fn trimmed(path: &str, width: usize) -> String {
    if path.width() <= width {
        return path.to_string();
    }
    let glyph = UI.glyphs.trimmed.current();
    let room = width.saturating_sub(glyph.width());
    if room == 0 {
        return String::new();
    }
    let kept = match path
        .match_indices('/')
        .map(|(slash, _)| &path[slash..])
        .find(|rest| rest.width() <= room)
    {
        Some(rest) => rest.to_string(),
        None => {
            let mut used = 0;
            let mut tail = path
                .chars()
                .rev()
                .take_while(|character| {
                    used += character.width().unwrap_or(0);
                    used <= room
                })
                .collect::<Vec<_>>();
            tail.reverse();
            tail.into_iter().collect()
        }
    };
    format!("{glyph}{kept}")
}

impl StatusLine {
    pub fn handle(&mut self, event: StatusLineEvent) {
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
                    | HLGroup::StatuslineHintLabel
                    | HLGroup::StatuslineSeparator
                    | HLGroup::StatuslineScheme
                    | HLGroup::StatuslineState
                    | HLGroup::ViewLabelActive
                    | HLGroup::ViewLabelInactive
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
        let area = Rect::new(
            area.x.saturating_add(UI.statusline_margin),
            area.y,
            area.width.saturating_sub(2 * UI.statusline_margin),
            area.height,
        );
        if area.width == 0 {
            return;
        }
        let width = usize::from(area.width);
        let (left, center, right) = match &self.message {
            None => (
                context.idle_left(width),
                context.center.clone(),
                context.right.clone(),
            ),
            Some(message) => match message.alignment {
                Alignment::Left => (message.spans.clone(), context.center.clone(), Vec::new()),
                Alignment::Center => (
                    context.idle_left(width),
                    Some(message.spans.clone()),
                    Vec::new(),
                ),
                Alignment::Right => (
                    context.idle_left(width),
                    context.center.clone(),
                    message.spans.clone(),
                ),
            },
        };
        let groups = scheme();
        let left = Self::line(&left, groups, HLGroup::Statusline1);
        let center = center.map(|spans| Self::line(&spans, groups, HLGroup::Statusline2));
        let right = Self::line(&right, groups, HLGroup::Statusline3);
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

    fn left_side(directory: &str, width: u16) -> String {
        let context = StatusLineContext {
            location: Some(StatusLocation {
                scheme: "everforest".into(),
                directory: directory.into(),
            }),
            ..StatusLineContext::default()
        };
        let area = Rect::new(0, 0, width, 1);
        let mut buffer = Buffer::empty(area);
        StatusLine::default().render(&context, area, &mut buffer);
        (0..width)
            .map(|x| buffer[(x, 0)].symbol())
            .collect::<String>()
            .trim()
            .to_string()
    }

    #[test]
    fn the_left_side_names_the_scheme_and_directory() {
        assert_eq!(
            left_side("~/m2/spectral-m2", 80),
            format!("everforest{}~/m2/spectral-m2", StatusLineContext::SEPARATOR)
        );
    }

    #[test]
    fn a_long_directory_loses_its_leading_components_to_stay_within_half_the_line() {
        let width = 60;
        let shown = left_side("~/projects/algebra/m2/spectral-m2", width);
        let inner = usize::from(width - 2 * UI.statusline_margin);

        assert!(shown.width() <= inner / 2, "{shown}");
        assert!(shown.starts_with("everforest"));
        assert!(
            shown.ends_with(&format!("{}/spectral-m2", UI.glyphs.trimmed.current())),
            "{shown}"
        );
    }

    #[test]
    fn trimming_keeps_whole_trailing_components_then_the_last_ones_tail() {
        let glyph = UI.glyphs.trimmed.current();
        assert_eq!(trimmed("~/a/bc", 10), "~/a/bc");
        assert_eq!(
            trimmed("~/abc/de", 3 + glyph.width()),
            format!("{glyph}/de")
        );
        assert_eq!(
            trimmed("~/abcdef", 3 + glyph.width()),
            format!("{glyph}def")
        );
        assert_eq!(trimmed("~/abcdef", glyph.width()), "");
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
