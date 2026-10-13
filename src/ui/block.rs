//! Renderable UI elements and full-width feed blocks.

use std::ops::Range;

use ratatui::buffer::Buffer;
use ratatui::layout::{Offset, Rect};
use ratatui::text::{Line, Span};

use super::controller::settings::theme::groups::{Groups, HLGroup};
use super::render::selection::Exclusions;
use super::render::RenderContext;

/// The available width changed, moving every wrap boundary.
pub(crate) struct Resized {
    pub(crate) width: u16,
}

/// Highlight groups now resolve through a different scheme.
///
/// Carries nothing: a scheme is ambient, read at paint time rather than handed down.
pub(crate) struct Rethemed;

/// Returned values are set differently -- fractions in another style -- so their
/// heights move as well as their cells.
pub(crate) struct Reformatted;

/// A renderable UI component with an explicit rendering context.
pub(crate) trait Element: Send {
    type Context;

    fn render(&self, context: &Self::Context, area: Rect, buf: &mut Buffer);
}

/// A self-contained element rendered locally across a measured number of feed rows.
pub(crate) trait Block {
    /// Number of consecutive feed rows occupied at the context width.
    fn height(&self, context: &RenderContext) -> u16;

    /// Render into a zero-based buffer matching this block's measured extent.
    fn render_local(&self, context: &RenderContext, buf: &mut Buffer);

    /// Add zero-based regions that copied selections must omit.
    fn local_selection_exclusions(&self, _: &RenderContext, _: &mut Exclusions) {}

    /// Restate this block's local exclusions in the coordinates of a placed `area`.
    ///
    /// The one shift applied to selection geometry, mirroring the one [`paste`] applies
    /// to cells, so both leave a child's local frame at the same boundary.
    ///
    /// [`paste`]: super::element::paste
    fn selection_exclusions(
        &self,
        context: &RenderContext,
        area: Rect,
        exclusions: &mut Exclusions,
    ) {
        let mut local = Exclusions::default();
        self.local_selection_exclusions(&context.with_width(area.width), &mut local);
        exclusions.extend(local + Offset::new(i32::from(area.x), i32::from(area.y)));
    }
}

/// Semantic content whose style is resolved from a highlight group at render time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HLSpan<T = Range<usize>> {
    pub(crate) content: T,
    pub(crate) kind: HLGroup,
}

impl<T> HLSpan<T> {
    pub(crate) fn new(content: impl Into<T>, kind: HLGroup) -> Self {
        Self {
            content: content.into(),
            kind,
        }
    }
}

/// Owned semantic text used by UI elements.
pub(crate) type TextSpan = HLSpan<String>;

/// A UI element represented by one line of styled text.
pub(crate) trait Text: Send {
    type Context: ?Sized;

    fn spans(&self, context: &Self::Context) -> Vec<Span<'static>>;

    fn line(&self, context: &Self::Context) -> Line<'static> {
        Line::from(self.spans(context))
    }
}

impl Text for [TextSpan] {
    type Context = Groups;

    fn spans(&self, groups: &Groups) -> Vec<Span<'static>> {
        self.iter()
            .map(|span| Span::styled(span.content.clone(), groups.style(span.kind)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-row block marking its final local column.
    struct Local;

    impl Block for Local {
        fn height(&self, _: &RenderContext) -> u16 {
            1
        }

        fn render_local(&self, context: &RenderContext, buf: &mut Buffer) {
            buf[(context.width() - 1, 0)].set_symbol("x");
        }

        fn local_selection_exclusions(&self, context: &RenderContext, exclusions: &mut Exclusions) {
            exclusions.push(Rect::new(context.width() - 1, 0, 1, 1));
        }
    }

    #[test]
    fn selection_geometry_is_translated_once_at_the_block_boundary() {
        let context = RenderContext::new(8, 0);
        let area = Rect::new(3, 2, 2, 1);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 4));
        buffer.set_style(
            area,
            ratatui::style::Style::new().bg(ratatui::style::Color::Blue),
        );
        let mut exclusions = Exclusions::default();

        Block::selection_exclusions(&Local, &context, area, &mut exclusions);

        assert_eq!(exclusions, [Rect::new(4, 2, 1, 1)].into());
    }
}
