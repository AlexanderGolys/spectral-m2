//! Configurable content at fixed, evenly spaced statusline anchors.

use crate::code::semantic::SERVER_EXECUTABLE;
use lsp_types::ServerInfo;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use super::block::{Element, Text, TextSpan};
use super::config::UI;
use super::controller::settings::statusline::{
    StatusComponent, StatusLineSettings, STATUS_SLOT_COUNT,
};
use super::controller::settings::theme::groups::{scheme, Groups, HLGroup};

/// Runtime state reported by the configured language server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum LanguageServerState {
    Off,
    #[default]
    Starting,
    Running(ServerInfo),
    Unavailable,
}

/// Versions and availability supplied by the running processes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusRuntime {
    pub m2_version: Option<String>,
    pub language_server: LanguageServerState,
}

/// The statusline's transient key hints, retained independently of its layout.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusLine {
    pub state_keys: Option<Vec<TextSpan>>,
}

/// Theme and working directory supplied by the editor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusLocation {
    pub scheme: String,
    pub directory: String,
}

/// Content available to every configurable position.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusLineContext {
    pub layout: StatusLineSettings,
    pub location: Option<StatusLocation>,
    pub hints: Vec<TextSpan>,
    pub state: Option<TextSpan>,
    pub center: Option<Vec<TextSpan>>,
    pub right: Vec<TextSpan>,
    pub math_renderer: String,
    pub runtime: StatusRuntime,
}

impl StatusLineContext {
    fn segments(&self, state_keys: Option<&[TextSpan]>) -> [Vec<TextSpan>; STATUS_SLOT_COUNT] {
        let mut contents: [Vec<TextSpan>; STATUS_SLOT_COUNT] = std::array::from_fn(|_| Vec::new());
        for &component in StatusComponent::CHOICES {
            let Some(slot) = self.layout.placement(component).slot() else {
                continue;
            };
            let content = component.content(self, state_keys);
            if !content.iter().any(|span| !span.content.is_empty()) {
                continue;
            }
            if !contents[slot].is_empty() {
                contents[slot].push(TextSpan::new(" / ", HLGroup::StatuslineSeparator));
            }
            contents[slot].extend(content);
        }
        contents
    }
}

impl StatusComponent {
    /// Resolve the selected content without assigning it to a screen position.
    fn content(
        self,
        context: &StatusLineContext,
        state_keys: Option<&[TextSpan]>,
    ) -> Vec<TextSpan> {
        let text = |value: String, group| vec![TextSpan::new(value, group)];
        let version = |name: &str, value: Option<&str>| {
            text(
                format!("{name} {}", value.unwrap_or("unknown")),
                HLGroup::Statusline1,
            )
        };
        match self {
            Self::Empty => Vec::new(),
            Self::Theme => context.location.as_ref().map_or_else(Vec::new, |location| {
                text(location.scheme.clone(), HLGroup::StatuslineScheme)
            }),
            Self::Directory => context.location.as_ref().map_or_else(Vec::new, |location| {
                text(location.directory.clone(), HLGroup::Statusline1)
            }),
            Self::KeyHints => state_keys.map_or_else(
                || {
                    if context.right.is_empty() {
                        context.hints.clone()
                    } else {
                        context.right.clone()
                    }
                },
                <[TextSpan]>::to_vec,
            ),
            Self::View => context.center.clone().unwrap_or_default(),
            Self::State => context.state.iter().cloned().collect(),
            Self::MathRenderer => text(context.math_renderer.clone(), HLGroup::Statusline1),
            Self::LanguageServer => text(
                match &context.runtime.language_server {
                    LanguageServerState::Off => "LSP off".into(),
                    LanguageServerState::Starting => "LSP starting".into(),
                    LanguageServerState::Running(server) => format!("{} ready", server.name),
                    LanguageServerState::Unavailable => "LSP unavailable".into(),
                },
                HLGroup::StatuslineState,
            ),
            Self::M2Version => version("M2", context.runtime.m2_version.as_deref()),
            Self::Sp2Version => version("sp2", Some(env!("CARGO_PKG_VERSION"))),
            Self::LanguageServerVersion => match &context.runtime.language_server {
                LanguageServerState::Running(server) => {
                    version(&server.name, server.version.as_deref())
                }
                _ => version(SERVER_EXECUTABLE, None),
            },
            Self::RenderingSettings | Self::Versions => {
                let components: &[Self] = match self {
                    Self::RenderingSettings => {
                        &[Self::Theme, Self::MathRenderer, Self::LanguageServer]
                    }
                    _ => &[
                        Self::LanguageServerVersion,
                        Self::Sp2Version,
                        Self::M2Version,
                    ],
                };
                let separator = TextSpan::new(" / ", HLGroup::StatuslineSeparator);
                components
                    .iter()
                    .map(|component| component.content(context, state_keys))
                    .filter(|spans| spans.iter().any(|span| !span.content.is_empty()))
                    .collect::<Vec<_>>()
                    .join(&separator)
            }
        }
    }
}

/// A rendered component's natural anchor and the space its occupied neighbours allow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SlotRegion {
    anchor: usize,
    left: usize,
    right: usize,
}

impl SlotRegion {
    fn occupied(width: usize, contents: &[Vec<TextSpan>; STATUS_SLOT_COUNT]) -> Vec<(usize, Self)> {
        let occupied: Vec<_> = contents
            .iter()
            .enumerate()
            .filter(|(_, spans)| spans.iter().any(|span| !span.content.is_empty()))
            .map(|(slot, _)| (slot, slot * width.saturating_sub(1) / (contents.len() - 1)))
            .collect();
        occupied
            .iter()
            .enumerate()
            .map(|(index, &(slot, anchor))| {
                let left = index
                    .checked_sub(1)
                    .map_or(0, |previous| (occupied[previous].1 + anchor) / 2 + 1);
                let right = occupied
                    .get(index + 1)
                    .map_or(width, |next| (anchor + next.1) / 2 + 1);
                (
                    slot,
                    Self {
                        anchor,
                        left,
                        right,
                    },
                )
            })
            .collect()
    }

    fn start(self, slot: usize, width: usize) -> isize {
        match slot {
            0 => self.anchor as isize,
            slot if slot == STATUS_SLOT_COUNT - 1 => self.anchor as isize + 1 - width as isize,
            _ => self.anchor as isize - (width / 2) as isize,
        }
    }
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
        let margin = UI.statusline_margin.min(area.width / 2);
        let area = Rect::new(
            area.x + margin,
            area.y,
            area.width - 2 * margin,
            area.height.min(1),
        );
        if area.is_empty() {
            return;
        }
        let contents = context.segments(self.state_keys.as_deref());
        let groups = scheme();
        for (slot, region) in SlotRegion::occupied(usize::from(area.width), &contents) {
            let line = Self::line(&contents[slot], groups, HLGroup::Statusline1);
            let width = line.width().min(usize::from(u16::MAX));
            if width == 0 {
                continue;
            }
            let start = region.start(slot, width);
            let mut source = Buffer::empty(Rect::new(0, 0, width as u16, 1));
            let visible = region.left.max(start.max(0) as usize)
                ..region.right.min((start + width as isize).max(0) as usize);
            // Paragraphs patch their destination's style. Retain that surface
            // while rendering offscreen for clipping around fixed anchors.
            for x in visible.clone() {
                let column = (x as isize - start) as u16;
                source[(column, 0)].set_style(buf[(area.x + x as u16, area.y)].style());
            }
            Paragraph::new(line).render(source.area, &mut source);
            for x in visible {
                let column = (x as isize - start) as u16;
                let cell = &source[(column, 0)];
                if x + cell.symbol().width() <= region.right {
                    buf[(area.x + x as u16, area.y)] = cell.clone();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::controller::settings::statusline::StatusSegment;

    fn spans(text: &str) -> Vec<TextSpan> {
        vec![TextSpan::new(text, HLGroup::Statusline1)]
    }

    #[test]
    fn clipped_text_preserves_the_background_already_painted_under_it() {
        use ratatui::style::Color;
        let context = StatusLineContext {
            layout: StatusLineSettings {
                components: std::array::from_fn(|_| StatusSegment::Centre),
                ..StatusLineSettings::default()
            },
            hints: spans("surface probe"),
            ..StatusLineContext::default()
        };
        let area = Rect::new(0, 0, 83, 1);
        let mut buffer = Buffer::empty(area);
        for x in 0..area.width {
            buffer[(x, 0)].set_bg(Color::Rgb(x as u8, 73, 119));
        }
        let original = buffer.clone();
        StatusLine::default().render(&context, area, &mut buffer);
        assert!(buffer.content.iter().any(|cell| cell.symbol() != " "));
        for position in area.positions() {
            assert_eq!(buffer[position].bg, original[position].bg);
        }
    }

    #[test]
    fn shared_segments_follow_checklist_order_rather_than_assignment_order() {
        let mut context = StatusLineContext {
            location: Some(StatusLocation {
                scheme: "themeProbe".into(),
                directory: "hiddenDirectory".into(),
            }),
            math_renderer: "rendererProbe".into(),
            runtime: StatusRuntime {
                language_server: LanguageServerState::Running(ServerInfo {
                    name: "serverProbe".into(),
                    version: None,
                }),
                ..StatusRuntime::default()
            },
            ..StatusLineContext::default()
        };
        context.layout.components.fill(StatusSegment::Hidden);
        for component in [
            StatusComponent::LanguageServer,
            StatusComponent::MathRenderer,
            StatusComponent::Theme,
        ] {
            context
                .layout
                .set_placement(component, StatusSegment::Centre);
        }
        let contents = context.segments(None);
        let shown: String = contents
            .iter()
            .flatten()
            .map(|span| span.content.as_str())
            .collect();
        assert!(shown.find("themeProbe").unwrap() < shown.find("rendererProbe").unwrap());
        assert!(shown.find("rendererProbe").unwrap() < shown.find("serverProbe").unwrap());
        assert!(!shown.contains("hiddenDirectory"));
        assert_eq!(contents.iter().filter(|spans| !spans.is_empty()).count(), 1);
    }

    #[test]
    fn occupied_slots_are_uniformly_anchored_and_never_overlap() {
        let contents = std::array::from_fn(|_| spans("content"));
        let regions = SlotRegion::occupied(101, &contents);
        assert_eq!(
            regions
                .iter()
                .map(|(_, region)| region.anchor)
                .collect::<Vec<_>>(),
            [0, 25, 50, 75, 100]
        );
        for width in 1..120 {
            let regions = SlotRegion::occupied(width, &contents);
            for (_, region) in &regions {
                assert!(region.left <= region.right);
                assert!(region.right <= width);
            }
            for pair in regions.windows(2) {
                assert!(pair[0].1.right <= pair[1].1.left);
            }
        }
    }

    #[test]
    fn empty_neighbours_expand_space_without_moving_the_remaining_anchor() {
        let mut contents = std::array::from_fn(|_| spans("content"));
        let before = SlotRegion::occupied(101, &contents)[2].1;
        contents
            .iter_mut()
            .enumerate()
            .filter(|(slot, _)| *slot != 2)
            .for_each(|(_, spans)| spans.clear());
        let after = SlotRegion::occupied(101, &contents)[0].1;
        assert_eq!(after.anchor, before.anchor);
        assert!(after.left < before.left);
        assert!(after.right > before.right);
        assert_eq!(after.start(2, 17), before.start(2, 17));
    }

    #[test]
    fn transient_hints_follow_the_selected_component_instead_of_a_fixed_side() {
        let context = StatusLineContext {
            hints: spans("idle hints"),
            right: spans("view hints"),
            ..StatusLineContext::default()
        };
        let active = spans("picker hints");
        assert_eq!(
            StatusComponent::KeyHints.content(&context, Some(&active)),
            active
        );
        assert_eq!(
            StatusComponent::KeyHints.content(&context, None),
            context.right
        );
        assert!(StatusComponent::Empty
            .content(&context, Some(&active))
            .is_empty());
    }

    #[test]
    fn versions_use_the_reported_server_and_interpreter_metadata() {
        let context = StatusLineContext {
            runtime: StatusRuntime {
                m2_version: Some("engine-build".into()),
                language_server: LanguageServerState::Running(ServerInfo {
                    name: "test-server".into(),
                    version: Some("server-build".into()),
                }),
            },
            ..StatusLineContext::default()
        };
        let version = StatusComponent::Versions.content(&context, None);
        let shown: String = version.iter().map(|span| span.content.as_str()).collect();
        assert!(shown.contains("test-server server-build"), "{shown}");
        assert!(shown.contains("M2 engine-build"), "{shown}");
        let unavailable = StatusLineContext {
            runtime: StatusRuntime {
                language_server: LanguageServerState::Unavailable,
                ..StatusRuntime::default()
            },
            ..StatusLineContext::default()
        };
        assert!(!StatusComponent::LanguageServerVersion
            .content(&unavailable, None)
            .is_empty());
    }

    #[test]
    fn narrow_statuslines_with_wide_characters_stay_inside_their_area() {
        let context = StatusLineContext {
            layout: StatusLineSettings {
                components: std::array::from_fn(|_| StatusSegment::Centre),
                ..StatusLineSettings::default()
            },
            hints: spans("中文 λ"),
            ..StatusLineContext::default()
        };
        for width in 0..40 {
            let mut buffer = Buffer::empty(Rect::new(0, 0, width + 4, 3));
            let before = buffer.clone();
            StatusLine::default().render(&context, Rect::new(2, 1, width, 1), &mut buffer);
            for position in buffer.area.positions() {
                if position.y != 1 || position.x < 2 || position.x >= width + 2 {
                    assert_eq!(buffer[position], before[position]);
                }
            }
        }
    }
}
