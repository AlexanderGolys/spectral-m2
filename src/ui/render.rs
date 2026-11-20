//! Shared rendering context and helpers for terminal UI elements. Two shapes cover
//! the feed: a full-width *code band* and an inset bordered *box*. Each block's
//! `render` is a thin call into here, so no block re-implements drawing. Measurement
//! and rendering consume the same visual-line ranges produced by the shared layout
//! policies.

use crate::ui::block::{Element, HLSpan, Text};
use crate::ui::config::UI;
use crate::ui::controller::ViewKind;
use crate::ui::glyphs::{Glyphs, Spelled};
use crate::ui::render::maths::MathStyle;
use crate::ui::settings::rendering::RendererPosition;
use crate::ui::settings::Settings;
use crate::ui::theme::groups::{scheme, Groups};
use crate::ui::wrap::WrapMode;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as TuiBlock, Padding, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

pub mod markdown;
pub mod markup;
pub mod maths;
pub mod selection;
#[allow(dead_code)] // groundwork: not yet wired into the running UI
pub mod spin;
pub mod typeset;

/// Horizontal placement of a bordered feed element.
#[derive(Clone, Copy)]
#[non_exhaustive]
enum BorderPlacement {
    Gutter,
    /// Centred over the middle third, widened into the margins as far as `content`
    /// columns of text need.
    MiddleThird {
        content: u16,
    },
}

// @##TODO
// add to // /@@settings
/// How many of the latest outputs M2 also names `oo`, `ooo` and `oooo`.
pub const RECENT_OUTPUTS: usize = 3;

/// Where a block is drawn and with which settings: the view it shows in, the
/// columns it has, and everything the settings decide about its look. Made once
/// per frame by [`RenderContext::new`], then narrowed with
/// [`with_width`](Self::with_width) for each nested element.
///
/// Whatever differs between views follows from the view: in the *reference* view
/// code may take the transcript's background, the *output* view numbers what each
/// cell printed, and both show returned values whole.
#[derive(Clone, Copy)]
pub struct RenderContext {
    settings: Settings,
    view: ViewKind,
    width: u16,
    prefix: u16,
    recent_outputs: [Option<u32>; RECENT_OUTPUTS],
    /// Worker layouts the measured heights already account for; see [`Pending::take`](crate::ui::relayout::Pending::take).
    adopted_relayouts: u64,
    /// Lay values out on the spot rather than on a worker, for drawings kept as
    /// they are first painted, which a placeholder would stay in for good.
    inline_layout: bool,
}

impl RenderContext {
    pub fn new(settings: Settings, view: ViewKind, width: u16) -> Self {
        Self {
            settings,
            view,
            width,
            prefix: 0,
            recent_outputs: [None; RECENT_OUTPUTS],
            adopted_relayouts: u64::MAX,
            inline_layout: false,
        }
    }

    /// Leave `prefix` columns before the text, for a prompt.
    pub fn with_prefix(self, prefix: u16) -> Self {
        Self { prefix, ..self }
    }

    /// A drawing kept as it is first painted: laid out on the spot, and with its
    /// values whole, as it has no fold to open.
    pub fn with_inline_layout(self) -> Self {
        Self {
            inline_layout: true,
            ..self
        }
    }

    pub fn inline_layout(&self) -> bool {
        self.inline_layout
    }

    /// Take up only the first `adopted` worker layouts, the ones whose arrival the
    /// retained heights were already invalidated for.
    pub fn with_adopted_relayouts(self, adopted: u64) -> Self {
        Self {
            adopted_relayouts: adopted,
            ..self
        }
    }

    pub fn adopted_relayouts(&self) -> u64 {
        self.adopted_relayouts
    }

    pub fn code_wrap(&self) -> WrapMode {
        self.settings.editor.code_wrap
    }

    /// Source styling for this view, preserving its syntax foregrounds.
    pub fn code_style(&self) -> Style {
        let mut style = self.groups().code();
        if self.view == ViewKind::Reference && self.settings.rendering.reference_code_background {
            style.bg = self.groups().transcript().bg;
        }
        style
    }

    /// Track the latest outputs, newest first, for highlighting.
    pub fn with_recent_outputs(self, recent_outputs: [Option<u32>; RECENT_OUTPUTS]) -> Self {
        Self {
            recent_outputs,
            ..self
        }
    }

    pub fn recent_outputs(&self) -> [Option<u32>; RECENT_OUTPUTS] {
        self.recent_outputs
    }

    /// Whether output `number` is one of the latest, which M2 recalls as `oo`, `ooo`
    /// and `oooo`.
    pub fn is_recent_output(&self, number: u32) -> bool {
        self.recent_outputs.contains(&Some(number))
    }

    pub fn renderer_position(&self) -> RendererPosition {
        self.settings.rendering.renderer_position
    }

    /// Whether printed text is labelled with the number of the cell that printed
    /// it: only where no code cell stands above it to say so.
    pub fn numbers_printed(&self) -> bool {
        self.view == ViewKind::Output
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    /// Retain rendering services at a nested element's local width.
    pub fn with_width(&self, width: u16) -> Self {
        Self { width, ..*self }
    }

    pub fn math(&self) -> MathStyle {
        MathStyle::from(&self.settings.rendering)
    }

    /// Rows a returned value shows before its middle folds away, if it folds.
    pub fn output_limit(&self) -> Option<usize> {
        let whole =
            self.inline_layout || matches!(self.view, ViewKind::Reference | ViewKind::Output);
        (!whole).then(|| self.settings.editor.output_rows.max(UI::MIN_OUTPUT_ROWS))
    }

    pub fn prefix(&self) -> u16 {
        self.prefix
    }

    /// The scheme in effect, read rather than retained.
    pub fn groups(&self) -> &'static Groups {
        scheme()
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

    /// A label wider than one column takes its extra width out of the gap
    /// after it, so the text never moves when the label changes.
    fn spans(&self, (): &()) -> Vec<Span<'static>> {
        let overhang = UnicodeWidthStr::width(self.label.as_str()).saturating_sub(1);
        vec![Span::styled(
            format!(
                "{}{:1}{}",
                " ".repeat(UI::PROMPT_MARGIN.into()),
                self.label,
                " ".repeat(usize::from(UI::PROMPT_TEXT_GAP).saturating_sub(overhang))
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
pub struct FrameLayout {
    prefix: u16,
    inset: u16,
    placement: BorderPlacement,
}

impl FrameLayout {
    /// Place a frame at an explicit inset with independent content padding.
    pub const fn gutter(prefix: u16, inset: u16) -> Self {
        Self {
            prefix,
            inset,
            placement: BorderPlacement::Gutter,
        }
    }

    /// Centre a box over the middle third of its available width, giving up margin
    /// rather than wrapping text up to `content` columns wide.
    pub const fn middle_third(content: u16) -> Self {
        Self {
            prefix: 0,
            inset: 0,
            placement: BorderPlacement::MiddleThird { content },
        }
    }

    /// Width reserved before bordered content.
    pub const fn prefix(self) -> u16 {
        self.prefix
    }

    /// Horizontal coordinate of the frame's left edge.
    pub const fn inset(self) -> u16 {
        self.inset
    }
}

/// Styled text decorated by the shared rounded feed border.
pub struct Bordered {
    layout: FrameLayout,
    surface: Style,
    border_style: Style,
    title: Option<Line<'static>>,
    body: Vec<Line<'static>>,
}

impl Bordered {
    pub fn area(area: Rect, layout: FrameLayout) -> Rect {
        if let BorderPlacement::MiddleThird { content } = layout.placement {
            let left = area.width / 3;
            let third = (area.width.saturating_mul(2) / 3 - left + 1).min(area.width - left);
            let chrome = third.saturating_sub(Self::content_width(third, layout));
            let width = third.max(content.saturating_add(chrome)).min(area.width);
            let start = match width > third {
                true => (area.width - width) / 2,
                false => left,
            };
            return Rect::new(area.x + start, area.y, width, area.height);
        }
        let width = area
            .width
            .saturating_sub(layout.inset + UI::RIGHT_MARGIN)
            .max(UI::MIN_BOX_WIDTH);
        Rect::new(area.x + layout.inset, area.y, width, area.height)
    }

    pub fn inner(area: Rect, layout: FrameLayout) -> Rect {
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
        Self::frame(layout).inner(Rect::new(0, 0, width, 1)).width
    }

    pub fn text_width(context: &RenderContext, layout: FrameLayout) -> u16 {
        Self::inner(Rect::new(0, 0, context.width(), 0), layout).width
    }

    /// Whether `position` lies on the frame's border line itself.
    pub fn on_border(area: Rect, layout: FrameLayout, position: Position) -> bool {
        let frame = Self::area(area, layout);
        frame.contains(position)
            && (position.x == frame.x
                || position.x == frame.right() - 1
                || position.y == frame.y
                || position.y == frame.bottom() - 1)
    }
}

impl Bordered {
    pub fn new(
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
            .border_set(Glyphs::ROUNDED_BORDER.current())
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
