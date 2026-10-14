//! Shared rendering context and helpers for terminal UI elements. Two shapes cover
//! the feed: a full-width *code band* and an inset bordered *box*. Each block's
//! `render` is a thin call into here, so no block re-implements drawing. Measurement
//! and rendering consume the same visual-line ranges produced by the shared layout
//! policies.

use crate::ui::block::{Element, HLSpan, Text};
use crate::ui::config::UI;
use crate::ui::controller::settings::theme::groups::{scheme, Groups};
use crate::ui::render::maths::FractionStyle;
use crate::ui::render::selection::Exclusions;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as TuiBlock, BorderType, Padding, Paragraph, Widget};

pub(crate) mod maths;
pub(crate) mod selection;
#[allow(dead_code)] // groundwork: not yet wired into the running UI
mod spin;

/// Horizontal placement of a bordered feed element.
#[derive(Clone, Copy)]
enum BorderPlacement {
    Gutter,
    /// Centred over the middle third, widened into the margins as far as `content`
    /// columns of text need.
    MiddleThird { content: u16 },
}

/// Runtime rendering inputs shared by every block without owning application state.
#[derive(Clone, Copy)]
pub(crate) struct RenderContext {
    width: u16,
    prefix: u16,
    output_limit: Option<usize>,
    fractions: FractionStyle,
}

impl RenderContext {
    pub(crate) fn new(width: u16, prefix: u16) -> Self {
        Self {
            width,
            prefix,
            output_limit: Some(UI.max_output_rows),
            fractions: FractionStyle::default(),
        }
    }

    /// Fold returned values taller than `rows`.
    pub(crate) fn with_output_limit(self, rows: usize) -> Self {
        Self {
            output_limit: Some(rows),
            ..self
        }
    }

    /// Set returned maths with fractions in `fractions`.
    pub(crate) fn with_fractions(self, fractions: FractionStyle) -> Self {
        Self { fractions, ..self }
    }

    pub(crate) fn width(&self) -> u16 {
        self.width
    }

    /// Retain rendering services at a nested element's local width.
    pub(crate) fn with_width(&self, width: u16) -> Self {
        Self { width, ..*self }
    }

    pub(crate) fn fractions(&self) -> FractionStyle {
        self.fractions
    }

    /// Render returned values without the shell's row limit.
    pub(crate) fn full_output(&self) -> Self {
        Self {
            output_limit: None,
            ..*self
        }
    }

    pub(crate) fn output_limit(&self) -> Option<usize> {
        self.output_limit
    }

    pub(crate) fn prefix(&self) -> u16 {
        self.prefix
    }

    /// The scheme in effect, read rather than retained.
    pub(crate) fn groups(&self) -> &'static Groups {
        scheme()
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
                base.patch(s.style(groups)),
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

    /// Centre a box over the middle third of its available width, giving up margin
    /// rather than wrapping text up to `content` columns wide.
    pub(crate) const fn middle_third(content: u16) -> Self {
        Self {
            prefix: 0,
            inset: 0,
            placement: BorderPlacement::MiddleThird { content },
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
        if let BorderPlacement::MiddleThird { content } = layout.placement {
            let left = area.width / 3;
            let third = (area.width.saturating_mul(2) / 3 - left + 1).min(area.width - left);
            let chrome = third.saturating_sub(Self::content_width(third, layout));
            let width = third
                .max(content.saturating_add(chrome))
                .min(area.width);
            let start = match width > third {
                true => (area.width - width) / 2,
                false => left,
            };
            return Rect::new(area.x + start, area.y, width, area.height);
        }
        let width = area
            .width
            .saturating_sub(layout.inset + UI.right_margin)
            .max(UI.min_box_width);
        Rect::new(area.x + layout.inset, area.y, width, area.height)
    }

    pub(crate) fn inner(area: Rect, layout: FrameLayout) -> Rect {
        Self::frame(layout).inner(Self::area(area, layout))
    }

    /// The border and padding drawn around content, without a title or colours.
    fn frame(layout: FrameLayout) -> TuiBlock<'static> {
        TuiBlock::bordered().padding(Padding::new(
            layout.prefix.saturating_sub(layout.inset + 1),
            1,
            0,
            0,
        ))
    }

    /// Columns left for content inside a frame `width` wide.
    fn content_width(width: u16, layout: FrameLayout) -> u16 {
        Self::frame(layout)
            .inner(Rect::new(0, 0, width, 1))
            .width
    }

    pub(crate) fn text_width(context: &RenderContext, layout: FrameLayout) -> u16 {
        Self::inner(Rect::new(0, 0, context.width(), 0), layout).width
    }

    /// Exclude the frame and its padding while retaining rendered body glyphs.
    pub(crate) fn exclude_frame(area: Rect, layout: FrameLayout, exclusions: &mut Exclusions) {
        let frame = Self::area(area, layout);
        let inner = Self::inner(area, layout);
        match (frame.width, frame.height) {
            (0, _) | (_, 0) => {}
            _ => exclusions.extend([
                Rect::new(frame.x, frame.y, frame.width, 1),
                Rect::new(frame.x, frame.bottom() - 1, frame.width, 1),
                Rect::new(frame.x, frame.y, inner.x - frame.x, frame.height),
                Rect::new(
                    inner.right(),
                    frame.y,
                    frame.right() - inner.right(),
                    frame.height,
                ),
            ]),
        }
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
        let mut block = Self::frame(self.layout)
            .style(self.surface)
            .border_type(BorderType::Rounded)
            .border_style(self.border_style);
        if let Some(title) = &self.title {
            block = block.title(title.clone());
        }
        block.render(Self::area(area, self.layout), buf);
        Paragraph::new(self.body.clone())
            .style(self.surface)
            .render(Self::inner(area, self.layout), buf);
    }
}
