//! The status line: the idle line the settings compose, with each side taken
//! over while the session's state has something to say there.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Modifier;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::block::{Element, Text, TextSpan};
use super::config::UI;
use super::controller::settings::theme::groups::{scheme, Groups, HLGroup};
use super::controller::settings::{IdleStatusLine, SegmentPlacement};

/// The status line's own memory between draws.
#[derive(Default)]
pub struct StatusLine {
    /// Keys the session's state calls for, in place of the right side.
    pub state_keys: Option<Vec<TextSpan>>,
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
    pub idle: IdleStatusLine,
    pub location: Option<StatusLocation>,
    pub hints: Vec<TextSpan>,
    /// What the session is doing, in place of the left side.
    pub state: Option<TextSpan>,
    pub center: Option<Vec<TextSpan>>,
    pub right: Vec<TextSpan>,
}

impl StatusLineContext {
    const SEPARATOR: &str = " · ";

    /// The idle segments placed on `side`, the directory trimmed to keep them
    /// within half of `width` columns.
    fn idle_side(&self, side: SegmentPlacement, width: usize) -> Vec<TextSpan> {
        let scheme = self
            .location
            .as_ref()
            .filter(|_| self.idle.scheme == side)
            .map(|location| {
                vec![TextSpan::new(
                    location.scheme.clone(),
                    HLGroup::StatuslineScheme,
                )]
            });
        let hints = (self.idle.hints == side && !self.hints.is_empty()).then(|| self.hints.clone());
        let fixed = scheme
            .iter()
            .chain(&hints)
            .map(|segment| {
                Self::SEPARATOR.width()
                    + segment
                        .iter()
                        .map(|span| span.content.width())
                        .sum::<usize>()
            })
            .sum::<usize>();
        let directory = self
            .location
            .as_ref()
            .filter(|_| self.idle.directory == side)
            .map(|location| trimmed(&location.directory, (width / 2).saturating_sub(fixed)))
            .filter(|directory| !directory.is_empty())
            .map(|directory| vec![TextSpan::new(directory, HLGroup::Statusline1)]);
        let separator = TextSpan::new(Self::SEPARATOR, HLGroup::StatuslineSeparator);
        [scheme, directory, hints]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(&separator)
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
    fn line(spans: &[TextSpan], groups: &Groups, group: HLGroup) -> Line<'static> {
        let mut line = spans.line(groups);
        for (source, span) in spans.iter().zip(&mut line.spans) {
            if !matches!(
                source.kind,
                HLGroup::KeymapHint
                    | HLGroup::Statusline1
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
        let left = context.state.clone().map_or_else(
            || context.idle_side(SegmentPlacement::Left, width),
            |state| vec![state],
        );
        let center = context.center.clone().or_else(|| {
            let idle = context.idle_side(SegmentPlacement::Center, width);
            (!idle.is_empty()).then_some(idle)
        });
        let right = self
            .state_keys
            .clone()
            .or_else(|| (!context.right.is_empty()).then(|| context.right.clone()))
            .unwrap_or_else(|| context.idle_side(SegmentPlacement::Right, width));
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

    fn context(directory: &str) -> StatusLineContext {
        StatusLineContext {
            location: Some(StatusLocation {
                scheme: "everforest".into(),
                directory: directory.into(),
            }),
            hints: vec![TextSpan::new("hints", HLGroup::StatuslineHintLabel)],
            ..StatusLineContext::default()
        }
    }

    fn drawn(line: &StatusLine, context: &StatusLineContext, width: u16) -> String {
        let area = Rect::new(0, 0, width, 1);
        let mut buffer = Buffer::empty(area);
        line.render(context, area, &mut buffer);
        (0..width)
            .map(|x| buffer[(x, 0)].symbol())
            .collect::<String>()
            .trim()
            .to_string()
    }

    fn left_side(directory: &str, width: u16) -> String {
        let context = StatusLineContext {
            hints: Vec::new(),
            ..context(directory)
        };
        drawn(&StatusLine::default(), &context, width)
    }

    #[test]
    fn the_idle_line_puts_each_segment_on_its_chosen_side() {
        use SegmentPlacement::{Center, Hidden, Left, Right};
        let idle = |scheme, directory, hints| StatusLineContext {
            idle: IdleStatusLine {
                scheme,
                directory,
                hints,
            },
            ..context("~/m2")
        };
        let separator = StatusLineContext::SEPARATOR;

        let swapped = drawn(&StatusLine::default(), &idle(Right, Hidden, Left), 60);
        assert!(swapped.starts_with("hints"), "{swapped}");
        assert!(swapped.ends_with("everforest"), "{swapped}");
        assert!(!swapped.contains("~/m2"), "{swapped}");

        let together = drawn(&StatusLine::default(), &idle(Center, Center, Center), 60);
        assert_eq!(
            together,
            format!("everforest{separator}~/m2{separator}hints")
        );

        assert_eq!(
            drawn(&StatusLine::default(), &idle(Hidden, Hidden, Hidden), 60),
            ""
        );
    }

    #[test]
    fn a_running_state_takes_over_only_the_left_side() {
        let running = StatusLineContext {
            state: Some(TextSpan::new("running", HLGroup::StatuslineWarning)),
            ..context("~/m2")
        };

        let shown = drawn(&StatusLine::default(), &running, 60);

        assert!(shown.starts_with("running"), "{shown}");
        assert!(!shown.contains("everforest"), "{shown}");
        assert!(shown.ends_with("hints"), "{shown}");
    }

    #[test]
    fn state_keys_take_over_only_the_right_side() {
        let line = StatusLine {
            state_keys: Some(vec![TextSpan::new("keys", HLGroup::StatuslineHintLabel)]),
        };

        let shown = drawn(&line, &context("~/m2"), 60);

        assert!(shown.starts_with("everforest"), "{shown}");
        assert!(shown.ends_with("keys"), "{shown}");
        assert!(!shown.contains("hints"), "{shown}");
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
