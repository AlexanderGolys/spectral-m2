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
use super::debug::mebibytes;
use super::graphics::Usage;
use super::theme::groups::{scheme, Groups, HLGroup};
use crate::ui::settings::statusline::{StatusComponent, StatusLineSettings, STATUS_SLOT_COUNT};

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
    pub m2_process: Option<u32>,
    pub language_server: LanguageServerState,
}

/// What the debug component shows, sampled only while it is placed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusDiagnostics {
    pub sp2_memory: Option<u64>,
    pub m2_memory: Option<u64>,
    pub pictures: Usage,
    pub frames_per_second: u32,
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
    pub diagnostics: StatusDiagnostics,
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
            Self::Debug => {
                let diagnostics = &context.diagnostics;
                let memory = |bytes: Option<u64>| bytes.map_or_else(|| "n/a".into(), mebibytes);
                let separator = TextSpan::new(" / ", HLGroup::StatuslineSeparator);
                [
                    format!(
                        "sp2 {} {}",
                        env!("CARGO_PKG_VERSION"),
                        memory(diagnostics.sp2_memory)
                    ),
                    format!("M2 {}", memory(diagnostics.m2_memory)),
                    format!(
                        "pictures {} ({})",
                        diagnostics.pictures.resident,
                        mebibytes(diagnostics.pictures.resident_bytes as u64)
                    ),
                    format!("{} fps", diagnostics.frames_per_second),
                ]
                .map(|part| text(part, HLGroup::Statusline1))
                .join(&separator)
            }
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

