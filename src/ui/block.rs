//! Renderable UI elements and full-width feed blocks.

use std::ops::Range;
use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::layout::{Offset, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::graphics::kitty::ImageId;
use super::item::Item;
use super::render::selection::Inclusions;
use super::render::RenderContext;
use super::theme::groups::{Groups, HLGroup};
use crate::code::highlight::Highlighter;
use crate::code::semantic::SourceId;

/// The available width changed, moving every wrap boundary.
pub struct Resized {
    pub width: u16,
}

/// Highlight groups now resolve through a different scheme.
///
/// Carries nothing: a scheme is ambient, read at paint time rather than handed down.
pub struct Rethemed;

/// Time passed, so whatever animates must be drawn again.
pub struct Ticked;

/// Returned values are set differently -- fractions in another style -- so their
/// heights move as well as their cells.
pub struct Reformatted;

/// A renderable UI component with an explicit rendering context.
pub trait Element: Send {
    type Context;

    fn render(&self, context: &Self::Context, area: Rect, buf: &mut Buffer);
}

/// A self-contained element rendered locally across a measured number of feed rows.
pub trait Block {
    /// Preferred initial row for a source reference, before the user scrolls.
    fn focus_row(&self, _: &RenderContext) -> Option<u16> {
        None
    }
    /// Number of consecutive feed rows occupied at the context width.
    fn height(&self, context: &RenderContext) -> u16;

    /// Render into a zero-based buffer matching this block's measured extent.
    fn render_local(&self, context: &RenderContext, buf: &mut Buffer);

    /// Whether this block's cells change with the clock, and so go stale on every tick.
    fn animated(&self) -> bool {
        false
    }

    /// The pictures this block last laid out, which the terminal may hold for it.
    fn pictures(&self) -> Vec<ImageId> {
        Vec::new()
    }

    /// Add zero-based regions that copied selections must omit.
    fn local_selection_inclusions(&self, _: &RenderContext, _: &mut Inclusions) {}

    /// The cells and results a selection can take whole, with the rows they cover.
    fn items(&self, _: &RenderContext) -> Vec<Item> {
        Vec::new()
    }

    /// Expand or fold content at a local position, returning whether its layout changed.
    fn toggle_fold(&mut self, _: Position, _: &RenderContext) -> bool {
        false
    }

    /// Draw the value whose renderer icon is at local `position` by its next
    /// renderer, returning whether its layout changed.
    fn cycle_renderer(&mut self, _: Position, _: &RenderContext) -> bool {
        false
    }

    /// The link drawn at local `position`, if any, with every cell it covers.
    fn link_at(&self, _: &RenderContext, _: Position) -> Option<Link> {
        None
    }

    /// The submitted code drawn at local `position`, if any.
    fn code_at(&self, _: &RenderContext, _: Position) -> Option<CellPoint> {
        None
    }

    /// The cells, local to this block, that draw the submitted code `code`.
    fn code_cells(&self, _: &RenderContext, _: &CodeRange) -> Vec<Rect> {
        Vec::new()
    }

    /// Pick out what is drawn at local `position`, returning the submitted code it
    /// stands for; the block then shows it picked.
    fn pick(&mut self, _: Position, _: &RenderContext) -> Option<CodeRange> {
        None
    }

    /// Drop whatever was picked, returning whether anything was.
    fn unpick(&mut self) -> bool {
        false
    }

    /// Repaint code of `sources` this block shows as the highlighter now colours
    /// it, returning whether its colours changed.
    fn rehighlight(&mut self, _: &[SourceId], _: &mut dyn Highlighter) -> bool {
        false
    }

    /// Retained link geometry for composing this block inside another document.
    fn links(&self, _: &RenderContext) -> Vec<Link> {
        Vec::new()
    }

    /// Restate this block's local inclusions in the coordinates of a placed `area`.
    ///
    /// The one shift applied to selection geometry, mirroring the one [`paste`] applies
    /// to cells, so both leave a child's local frame at the same boundary.
    ///
    /// [`paste`]: super::element::paste
    fn selection_inclusions(
        &self,
        context: &RenderContext,
        area: Rect,
        inclusions: &mut Inclusions,
    ) {
        let mut local = Inclusions::default();
        self.local_selection_inclusions(&context.with_width(area.width), &mut local);
        inclusions.merge(local + Offset::new(i32::from(area.x), i32::from(area.y)));
    }
}

/// A place in submitted code: a cell of a submission and bytes into its source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellPoint {
    pub submission: u64,
    pub index: usize,
    pub offset: usize,
}

/// A stretch of submitted code: bytes of one cell of a submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeRange {
    pub submission: u64,
    pub index: usize,
    pub bytes: Range<usize>,
}

/// A drawn link: what it points to and the cells it covers, which may run over
/// several rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub target: Arc<str>,
    pub cells: Vec<Rect>,
}

impl Link {
    /// The same link drawn `offset` away.
    pub fn moved(self, offset: Offset) -> Self {
        Self {
            cells: self.cells.into_iter().map(|cells| cells + offset).collect(),
            ..self
        }
    }
}

/// Semantic content whose style is resolved from a highlight group at render time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HLSpan<T = Range<usize>> {
    pub content: T,
    pub kind: HLGroup,
    /// A group laid over `kind` without replacing it, such as the underline that
    /// rules a fraction's numerator.
    pub mark: Option<HLGroup>,
    /// What the span links to. Every piece of one link shares the one target, so
    /// the pieces a wrap leaves are still known as one link.
    pub target: Option<Arc<str>>,
}

impl<T> HLSpan<T> {
    pub fn new(content: impl Into<T>, kind: HLGroup) -> Self {
        Self {
            content: content.into(),
            kind,
            mark: None,
            target: None,
        }
    }

    pub fn marked(self, mark: HLGroup) -> Self {
        Self {
            mark: Some(mark),
            ..self
        }
    }

    /// The one group that stands for both of this span's, where text can only be set in
    /// a single one: the semantic group, unless it leaves the syntax colour as it is.
    pub fn flattened_kind(&self) -> HLGroup {
        match self.mark {
            Some(HLGroup::LspVariable) | None => self.kind,
            Some(mark) => mark,
        }
    }

    /// The same groups over different content: a piece of this span, or this span moved.
    pub fn with_content<U>(&self, content: impl Into<U>) -> HLSpan<U> {
        HLSpan {
            content: content.into(),
            kind: self.kind,
            mark: self.mark,
            target: self.target.clone(),
        }
    }

    pub fn style(&self, groups: &Groups) -> Style {
        let style = groups.style(self.kind);
        match self.mark {
            Some(mark) => style.patch(groups.style(mark)),
            None => style,
        }
    }
}

/// Owned semantic text used by UI elements.
pub type TextSpan = HLSpan<String>;

/// A UI element represented by one line of styled text.
pub trait Text: Send {
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
            .map(|span| Span::styled(span.content.clone(), span.style(groups)))
            .collect()
    }
}
