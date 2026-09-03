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

impl Bordered {
    fn area(area: Rect) -> Rect {
        let width = area
            .width
            .saturating_sub(UI.border_margin + UI.right_margin)
            .max(UI.min_box_width);
        Rect::new(area.x + UI.border_margin, area.y, width, area.height)
    }

    pub fn inner(area: Rect) -> Rect {
        TuiBlock::bordered()
            .padding(Padding::new(UI.box_left_padding(), 1, 0, 0))
            .inner(Self::area(area))
    }

    pub fn text_width(context: &RenderContext) -> u16 {
        Self::inner(Rect::new(0, 0, context.width(), 0)).width
    }
}

/// Styled text decorated by the shared rounded feed border.
pub struct Bordered {
    surface: Style,
    border_style: Style,
    title: Option<Line<'static>>,
    bottom: Option<Line<'static>>,
    body: Vec<Line<'static>>,
}

impl Bordered {
    pub fn new(
        surface: Style,
        border_style: Style,
        title: Option<Line<'static>>,
        bottom: Option<Line<'static>>,
        body: Vec<Line<'static>>,
    ) -> Self {
        Self {
            surface,
            border_style,
            title,
            bottom,
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
            .padding(Padding::new(UI.box_left_padding(), 1, 0, 0));
        if let Some(title) = &self.title {
            block = block.title(title.clone());
        }
        if let Some(bottom) = &self.bottom {
            block = block.title_bottom(bottom.clone());
        }
        block.render(Self::area(area), buf);
        Paragraph::new(self.body.clone())
            .style(self.surface)
            .render(Self::inner(area), buf);
    }
}
