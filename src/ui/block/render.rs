//! Shared shape renderers behind every [`Block`](super::Block). Two shapes cover
//! the feed: a full-width *code band* and an inset bordered *box*. Each block's
//! `render` is a thin call into here, so no block re-implements drawing. Measurement
//! and rendering consume the same visual-line ranges produced by the shared layout
//! policies.

use crate::ui::config::UI;
use crate::ui::controller::settings::theme::groups::Groups;
use crate::ui::element::{Element, HLSpan, Text};
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
pub struct RenderContext {
    width: u16,
    prefix: u16,
    groups: Groups,
}

impl RenderContext {
    pub fn new(width: u16, prefix: u16, groups: &Groups) -> Self {
        Self {
            width,
            prefix,
            groups: *groups,
        }
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn prefix(&self) -> u16 {
        self.prefix
    }

    pub fn groups(&self) -> &Groups {
        &self.groups
    }
}

/// Styled prefix containing the live input prompt.
pub struct Prompt {
    label: String,
    style: Style,
}

impl Prompt {
    pub fn new(label: impl Into<String>, style: Style) -> Self {
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
pub trait Highlights {
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

/// Geometry shared by result, warning, and error borders.
#[derive(Clone, Copy)]
pub struct BorderLayout {
    prefix: u16,
    inset: u16,
    placement: BorderPlacement,
}

impl BorderLayout {
    /// Align a border and its body to a possible `oN` marker.
    pub const fn numbered(number: u32) -> Self {
        let digits = if number == 0 {
            1
        } else {
            number.ilog10() as u16 + 1
        };
        Self {
            prefix: UI.cell_margin + digits + UI.cell_gap,
            inset: UI.cell_margin + digits,
            placement: BorderPlacement::Gutter,
        }
    }

    /// Cross only the penultimate digit of a multi-digit `oN` marker.
    pub const fn output(number: u32) -> Self {
        let mut layout = Self::numbered(number);
        layout.inset -= if number >= 10 { 1 } else { 0 };
        layout
    }

    /// Align an unnumbered border with an ordinary one-digit result.
    pub const fn plain() -> Self {
        Self::numbered(1)
    }

    /// Place a box between one-third and two-thirds of its available width.
    pub const fn middle_third() -> Self {
        Self {
            prefix: 0,
            inset: 0,
            placement: BorderPlacement::MiddleThird,
        }
    }

    /// Width reserved before bordered content.
    pub const fn prefix(self) -> u16 {
        self.prefix
    }

    /// Draw the line separating a feed marker from its content.
    pub fn render_separator(self, area: Rect, style: Style, buf: &mut Buffer) {
        if matches!(self.placement, BorderPlacement::MiddleThird) {
            return;
        }
        let x = area.x + self.inset;
        for y in area.y..area.bottom() {
            buf[(x, y)].set_symbol("▏").set_style(style);
        }
    }
}

impl Bordered {
    fn area(area: Rect, layout: BorderLayout) -> Rect {
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

    pub fn inner(area: Rect, layout: BorderLayout) -> Rect {
        TuiBlock::bordered()
            .padding(Padding::new(
                layout.prefix.saturating_sub(layout.inset + 1),
                1,
                0,
                0,
            ))
            .inner(Self::area(area, layout))
    }

    pub fn text_width(context: &RenderContext, layout: BorderLayout) -> u16 {
        Self::inner(Rect::new(0, 0, context.width(), 0), layout).width
    }
}

/// Styled text decorated by the shared rounded feed border.
pub struct Bordered {
    layout: BorderLayout,
    surface: Style,
    border_style: Style,
    title: Option<Line<'static>>,
    body: Vec<Line<'static>>,
}

impl Bordered {
    pub fn new(
        layout: BorderLayout,
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
