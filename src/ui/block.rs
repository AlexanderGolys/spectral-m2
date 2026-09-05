//! Renderable UI elements and full-width feed blocks.

use std::iter::FromIterator;
use std::ops::{Add, AddAssign, Range};

use ratatui::buffer::Buffer;
use ratatui::layout::{Offset, Position, Rect};
use ratatui::text::{Line, Span};

use super::controller::settings::theme::groups::{Groups, HLGroup};
use super::render::RenderContext;

/// Rectangular regions omitted from copied UI selections.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Exclusions(Vec<Rect>);

impl Exclusions {
    pub(crate) fn push(&mut self, exclusion: Rect) {
        self.0.push(exclusion);
    }

    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }

    pub(crate) fn contains(&self, position: Position) -> bool {
        self.0.iter().any(|area| area.contains(position))
    }

    /// Clip to source rows and make the first retained row local zero.
    pub(crate) fn project(&self, rows: Range<u16>) -> Self {
        self.0
            .iter()
            .filter_map(|area| {
                let top = area.y.max(rows.start);
                let bottom = area.bottom().min(rows.end);
                (top < bottom)
                    .then(|| Rect::new(area.x, top - rows.start, area.width, bottom - top))
            })
            .collect()
    }
}

impl Add<Offset> for Exclusions {
    type Output = Self;

    fn add(mut self, offset: Offset) -> Self {
        self += offset;
        self
    }
}

impl Add<Offset> for &Exclusions {
    type Output = Exclusions;

    fn add(self, offset: Offset) -> Exclusions {
        self.0.iter().map(|area| *area + offset).collect()
    }
}

impl AddAssign<Offset> for Exclusions {
    fn add_assign(&mut self, offset: Offset) {
        self.0.iter_mut().for_each(|area| *area += offset);
    }
}

impl Extend<Rect> for Exclusions {
    fn extend<T: IntoIterator<Item = Rect>>(&mut self, iter: T) {
        self.0.extend(iter);
    }
}

impl FromIterator<Rect> for Exclusions {
    fn from_iter<T: IntoIterator<Item = Rect>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for Exclusions {
    type Item = Rect;
    type IntoIter = std::vec::IntoIter<Rect>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<const N: usize> From<[Rect; N]> for Exclusions {
    fn from(areas: [Rect; N]) -> Self {
        Self(Vec::from(areas))
    }
}

/// A renderable UI component with an explicit rendering context.
pub(crate) trait Element: Send {
    type Context: ?Sized;

    fn render(&self, context: &Self::Context, area: Rect, buf: &mut Buffer);

    /// Add visual regions that copied selections must omit.
    fn selection_exclusions(&self, _: &Self::Context, _: Rect, _: &mut Exclusions) {}
}

/// A self-contained element rendered locally across a measured number of feed rows.
pub(crate) trait Block: Element<Context = RenderContext> {
    /// Number of consecutive feed rows occupied at the context width.
    fn height(&self, context: &RenderContext) -> u16;

    /// Render into a zero-based buffer matching this block's measured extent.
    fn render_local(&self, context: &RenderContext, buf: &mut Buffer);

    /// Add zero-based regions that copied selections must omit.
    fn local_selection_exclusions(&self, _: &RenderContext, _: &mut Exclusions) {}
}

impl<T: Block + ?Sized> Element for T {
    type Context = RenderContext;

    fn render(&self, context: &RenderContext, area: Rect, buf: &mut Buffer) {
        let mut local = Buffer::empty(Rect::new(0, 0, area.width, area.height));
        for y in 0..area.height {
            for x in 0..area.width {
                local[(x, y)] = buf[(area.x + x, area.y + y)].clone();
            }
        }
        self.render_local(&context.with_width(area.width), &mut local);
        local.resize(area);
        buf.merge(&local);
    }

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
#[derive(Clone)]
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
    fn element_translates_block_coordinates_once_at_its_boundary() {
        let context = RenderContext::new(8, 0, &Groups::default());
        let area = Rect::new(3, 2, 2, 1);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 4));
        buffer.set_style(
            area,
            ratatui::style::Style::new().bg(ratatui::style::Color::Blue),
        );
        let mut exclusions = Exclusions::default();

        Element::render(&Local, &context, area, &mut buffer);
        Element::selection_exclusions(&Local, &context, area, &mut exclusions);

        assert_eq!(buffer[(4, 2)].symbol(), "x");
        assert_eq!(buffer[(3, 2)].bg, ratatui::style::Color::Blue);
        assert_eq!(buffer[(4, 2)].bg, ratatui::style::Color::Blue);
        assert_eq!(exclusions, [Rect::new(4, 2, 1, 1)].into());
    }

    #[test]
    fn exclusions_project_shift_and_test_points() {
        let exclusions =
            Exclusions::from([Rect::new(2, 3, 2, 3)]).project(4..6) + Offset::new(5, 7);

        assert!(exclusions.contains(Position::new(8, 8)));
        assert!(!exclusions.contains(Position::new(6, 8)));
        assert_eq!(exclusions, [Rect::new(7, 7, 2, 2)].into());
    }
}
