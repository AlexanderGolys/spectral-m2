//! Shared rendering context and helpers for terminal UI elements. Two shapes cover
//! the feed: a full-width *code band* and an inset bordered *box*. Each block's
//! `render` is a thin call into here, so no block re-implements drawing. Measurement
//! and rendering consume the same visual-line ranges produced by the shared layout
//! policies.

use crate::ui::block::{Element, HLSpan, Text};
use crate::ui::config::UI;
use crate::ui::controller::settings::theme::groups::Groups;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as TuiBlock, BorderType, Padding, Paragraph, Widget};

/// Horizontal placement of a bordered feed element.
#[derive(Clone, Copy)]
enum BorderPlacement {
    Gutter,
    MiddleThird,
}

/// Runtime rendering inputs shared by every block without owning application state.
pub(crate) struct RenderContext {
    width: u16,
    prefix: u16,
    groups: Groups,
    output_limit: Option<usize>,
}

impl RenderContext {
    pub(crate) fn new(width: u16, prefix: u16, groups: &Groups) -> Self {
        Self {
            width,
            prefix,
            groups: *groups,
            output_limit: Some(UI.max_output_rows),
        }
    }

    pub(crate) fn width(&self) -> u16 {
        self.width
    }

    /// Retain rendering services at a nested element's local width.
    pub(crate) fn with_width(&self, width: u16) -> Self {
        Self {
            width,
            prefix: self.prefix,
            groups: self.groups,
            output_limit: self.output_limit,
        }
    }

    /// Render returned values without the shell's row limit.
    pub(crate) fn full_output(&self) -> Self {
        Self {
            output_limit: None,
            ..self.with_width(self.width)
        }
    }

    pub(crate) fn output_limit(&self) -> Option<usize> {
        self.output_limit
    }

    pub(crate) fn prefix(&self) -> u16 {
        self.prefix
    }

    pub(crate) fn groups(&self) -> &Groups {
        &self.groups
    }
}

/// Styled prefix containing the live input prompt.
pub(crate) struct Prompt {
    label: String,
    style: Style,
}

impl Prompt {
    pub(crate) fn new(label: impl Into<String>, style: Style) -> Self {
        Self {
            label: label.into(),
            style,
        }
    }
}

impl Text for Prompt {
    type Context = ();

    fn spans(&self, (): &()) -> Vec<Span<'static>> {
        vec![Span::styled(
            format!(
                "{}{:1}{}",
                " ".repeat(UI.prompt_margin.into()),
                self.label,
                " ".repeat(UI.prompt_text_gap.into())
            ),
            self.style,
        )]
    }
}

/// Semantic highlight ranges that paint source text into styled spans.
pub(crate) trait Highlights {
    fn paint(
        &self,
        abs_start: usize,
        seg: &str,
        base: Style,
        groups: &Groups,
    ) -> Vec<Span<'static>>;
}

impl Highlights for [HLSpan] {
    fn paint(
        &self,
        abs_start: usize,
        seg: &str,
        base: Style,
        groups: &Groups,
    ) -> Vec<Span<'static>> {
        let seg_end = abs_start + seg.len();
        let mut out = Vec::new();
        let mut cur = abs_start;
        for s in self
            .iter()
            .filter(|s| s.content.start < seg_end && s.content.end > abs_start)
        {
            let st = s.content.start.max(abs_start);
            let en = s.content.end.min(seg_end);
            if st > cur {
                out.push(Span::styled(
                    seg[cur - abs_start..st - abs_start].to_string(),
                    base,
                ));
            }
            out.push(Span::styled(
                seg[st - abs_start..en - abs_start].to_string(),
                base.patch(groups.style(s.kind)),
            ));
            cur = en;
        }
        if cur < seg_end {
            out.push(Span::styled(seg[cur - abs_start..].to_string(), base));
        }
        if out.is_empty() {
            out.push(Span::styled(seg.to_string(), base));
        }
        out
    }
}

/// Content geometry for a generic bordered element.
#[derive(Clone, Copy)]
pub(crate) struct FrameLayout {
    prefix: u16,
    inset: u16,
    placement: BorderPlacement,
}

impl FrameLayout {
    /// Place a frame at an explicit inset with independent content padding.
    pub(crate) const fn gutter(prefix: u16, inset: u16) -> Self {
        Self {
            prefix,
            inset,
            placement: BorderPlacement::Gutter,
        }
    }

    /// Place a box between one-third and two-thirds of its available width.
    pub(crate) const fn middle_third() -> Self {
        Self {
            prefix: 0,
            inset: 0,
            placement: BorderPlacement::MiddleThird,
        }
    }

    /// Width reserved before bordered content.
    pub(crate) const fn prefix(self) -> u16 {
        self.prefix
    }

    /// Horizontal coordinate of the frame's left edge.
    pub(crate) const fn inset(self) -> u16 {
        self.inset
    }
}

impl Bordered {
    fn area(area: Rect, layout: FrameLayout) -> Rect {
        if matches!(layout.placement, BorderPlacement::MiddleThird) {
            let left = area.width / 3;
            let right = area.width.saturating_mul(2) / 3;
            return Rect::new(
                area.x + left,
                area.y,
                (right - left + 1).min(area.width - left),
                area.height,
            );
        }
        let width = area
            .width
            .saturating_sub(layout.inset + UI.right_margin)
            .max(UI.min_box_width);
        Rect::new(area.x + layout.inset, area.y, width, area.height)
    }

    pub(crate) fn inner(area: Rect, layout: FrameLayout) -> Rect {
        TuiBlock::bordered()
            .padding(Padding::new(
                layout.prefix.saturating_sub(layout.inset + 1),
                1,
                0,
                0,
            ))
            .inner(Self::area(area, layout))
    }

    pub(crate) fn text_width(context: &RenderContext, layout: FrameLayout) -> u16 {
        Self::inner(Rect::new(0, 0, context.width(), 0), layout).width
    }
}

/// Styled text decorated by the shared rounded feed border.
pub(crate) struct Bordered {
    layout: FrameLayout,
    surface: Style,
    border_style: Style,
    title: Option<Line<'static>>,
    body: Vec<Line<'static>>,
}

impl Bordered {
    pub(crate) fn new(
        layout: FrameLayout,
        surface: Style,
        border_style: Style,
        title: Option<Line<'static>>,
        body: Vec<Line<'static>>,
    ) -> Self {
        Self {
            layout,
            surface,
            border_style,
            title,
            body,
        }
    }
}

impl Element for Bordered {
    type Context = ();

    fn render(&self, (): &(), area: Rect, buf: &mut Buffer) {
        let mut block = TuiBlock::bordered()
            .style(self.surface)
            .border_type(BorderType::Rounded)
            .border_style(self.border_style)
            .padding(Padding::new(
                self.layout.prefix.saturating_sub(self.layout.inset + 1),
                1,
                0,
                0,
            ));
        if let Some(title) = &self.title {
            block = block.title(title.clone());
        }
        block.render(Self::area(area, self.layout), buf);
        Paragraph::new(self.body.clone())
            .style(self.surface)
            .render(Self::inner(area, self.layout), buf);
    }
}
