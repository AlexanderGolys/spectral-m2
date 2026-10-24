//! Retained transcript rendering and interaction limited to the viewport.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Offset, Position, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block as TuiBlock, Paragraph, Widget};

use super::block::{Block, Link, Reformatted, Resized, Rethemed, Ticked};
use super::cell::{EvaluationBlock, IncompleteBlock};
use super::config::UI;
use super::element::{paste, EventListener};
use super::render::selection::Exclusions;
use super::render::{RenderContext, RECENT_OUTPUTS};

/// Retained position metadata for one feed viewport.
struct ViewportState {
    area: Rect,
    scroll: usize,
    anchor: Option<(usize, u16)>,
    context: RenderContext,
    /// Each visible block's index, screen rows, and first local row shown there.
    placements: Vec<(usize, Rect, u16)>,
}

/// Storage for an open-ended block or the one block the feed must mutate in place.
///
/// The generic variant accepts every [`Block`] implementation; this distinction
/// exists only so submission updates remain typed rather than downcast from a
/// trait object.
enum FeedContent {
    /// Any block requiring only rendering and selection behavior.
    Block(Box<dyn Block>),
    /// A live submission addressed by evaluation updates.
    Evaluation(EvaluationBlock),
    /// The optional unfinished submission, which is always the last block.
    Incomplete(IncompleteBlock),
}

impl FeedContent {
    fn block(&self) -> &dyn Block {
        match self {
            Self::Block(block) => block.as_ref(),
            Self::Evaluation(block) => block,
            Self::Incomplete(block) => block,
        }
    }
}

/// One feed element with its retained local layout, cells, and selection geometry.
///
/// The two retentions go stale on different events and so are held apart: a width
/// changes every wrap boundary and invalidates both, while a scheme only changes which
/// style a highlight group resolves to, leaving the geometry exactly where it was.
struct RetainedBlock {
    content: FeedContent,
    layout: Option<Layout>,
    cells: Option<Buffer>,
    exclusions: Option<Exclusions>,
}

/// A block's measured height, kept as an estimate once it goes stale so a block off
/// screen is never laid out again just to be scrolled past.
#[derive(Clone, Copy)]
struct Layout {
    width: u16,
    height: u16,
    settled: bool,
}

impl RetainedBlock {
    fn new(content: FeedContent) -> Self {
        Self {
            content,
            layout: None,
            cells: None,
            exclusions: None,
        }
    }

    /// The height at the context width, laid out again if it went stale.
    fn height(&mut self, context: &RenderContext) -> u16 {
        self.on_event(&Resized {
            width: context.width(),
        });
        match self.layout {
            Some(layout) if layout.settled => layout.height,
            _ => {
                let height = self.content.block().height(context).max(1);
                self.layout = Some(Layout {
                    width: context.width(),
                    height,
                    settled: true,
                });
                height
            }
        }
    }

    /// The last height measured, stale or not; only a block never measured is laid out.
    fn estimated_height(&mut self, context: &RenderContext) -> u16 {
        self.on_event(&Resized {
            width: context.width(),
        });
        match self.layout {
            Some(layout) => layout.height,
            None => self.height(context),
        }
    }

    fn render(&mut self, context: &RenderContext, canvas: Style) -> (&Buffer, &Exclusions) {
        let height = self.height(context);
        if self.cells.is_none() {
            let mut cells = Feed::buffer(Rect::new(0, 0, context.width(), height), canvas);
            self.content.block().render_local(context, &mut cells);
            self.cells = Some(cells);
        }
        if self.exclusions.is_none() {
            let mut exclusions = Exclusions::default();
            self.content
                .block()
                .local_selection_exclusions(context, &mut exclusions);
            self.exclusions = Some(exclusions);
        }
        (
            self.cells.as_ref().expect("cells were just settled"),
            self.exclusions
                .as_ref()
                .expect("exclusions were just settled"),
        )
    }

    fn invalidate(&mut self) {
        self.cells = None;
        self.exclusions = None;
    }

    fn invalidate_layout(&mut self) {
        if let Some(layout) = &mut self.layout {
            layout.settled = false;
        }
        self.invalidate();
    }

    /// The block's own content changed, so even its old height says nothing.
    fn forget_layout(&mut self) {
        self.layout = None;
        self.invalidate();
    }
}

/// A width moves every wrap boundary, so both the geometry and the cells go with it.
impl EventListener<Resized> for RetainedBlock {
    fn on_event(&mut self, event: &Resized) {
        if let Some(layout) = &mut self.layout {
            if layout.width != event.width {
                layout.width = event.width;
                self.invalidate_layout();
            }
        }
    }
}

/// A scheme only changes which style a group resolves to, so the geometry survives.
impl EventListener<Rethemed> for RetainedBlock {
    fn on_event(&mut self, _: &Rethemed) {
        self.cells = None;
    }
}

/// A tick only repaints what moves with the clock, so the rest keeps its cells.
impl EventListener<Ticked> for RetainedBlock {
    fn on_event(&mut self, _: &Ticked) {
        if self.content.block().animated() {
            self.invalidate();
        }
    }
}

/// Setting values differently moves their heights, so the geometry goes with the cells.
impl EventListener<Reformatted> for RetainedBlock {
    fn on_event(&mut self, _: &Reformatted) {
        self.invalidate_layout();
    }
}

/// A borrowed block list rendered through a scrollable rectangular viewport.
pub struct BlockView<'a> {
    blocks: &'a [&'a dyn Block],
    empty: &'a str,
}

impl<'a> BlockView<'a> {
    /// Borrow blocks and the message shown when that list is empty.
    pub const fn new(blocks: &'a [&'a dyn Block], empty: &'a str) -> Self {
        Self { blocks, empty }
    }

    /// Render only blocks intersecting the selected rows and clamp `scroll`.
    pub fn render(
        &self,
        context: &RenderContext,
        area: Rect,
        target: &mut Buffer,
        scroll: &mut usize,
        canvas: Style,
    ) -> Exclusions {
        if area.is_empty() {
            return Exclusions::default();
        }
        if self.blocks.is_empty() {
            Paragraph::new(self.empty)
                .style(context.groups().muted())
                .alignment(Alignment::Center)
                .render(area, target);
            return Exclusions::default();
        }
        let heights = self
            .blocks
            .iter()
            .map(|block| block.height(context).max(1))
            .collect::<Vec<_>>();
        let total = heights
            .iter()
            .map(|height| usize::from(*height))
            .sum::<usize>();
        *scroll = (*scroll).min(total.saturating_sub(area.height.into()));
        let mut skip = *scroll;
        let mut y = 0;
        let mut exclusions = Exclusions::default();
        for (block, height) in self.blocks.iter().zip(heights) {
            if skip >= height.into() {
                skip -= usize::from(height);
                continue;
            }
            let start = skip as u16;
            skip = 0;
            let rows = (height - start).min(area.height - y);
            let mut rendered = Feed::buffer(Rect::new(0, 0, area.width, height), canvas);
            block.render_local(context, &mut rendered);
            paste(
                &rendered,
                start..start + rows,
                Rect::new(area.x, area.y + y, area.width, rows),
                target,
            );
            let mut source = Exclusions::default();
            block.local_selection_exclusions(context, &mut source);
            exclusions.extend(
                source.project(start..start + rows)
                    + Offset::new(i32::from(area.x), i32::from(area.y + y)),
            );
            y += rows;
            if y == area.height {
                break;
            }
        }
        exclusions
    }
}

/// Transcript blocks with their scrolling and retained presentation state.
pub struct Feed {
    blocks: Vec<RetainedBlock>,
    scroll: usize,
    anchor: Option<(usize, u16)>,
    surface: Option<ViewportState>,
    /// The latest outputs the retained cells were labelled for.
    labelled: [Option<u32>; RECENT_OUTPUTS],
}

impl Feed {
    /// Create a feed containing one initial block.
    pub fn new(block: Box<dyn Block>) -> Self {
        Self {
            blocks: vec![RetainedBlock::new(FeedContent::Block(block))],
            scroll: 0,
            anchor: None,
            surface: None,
            labelled: [None; RECENT_OUTPUTS],
        }
    }

    /// Append a block and follow the new bottom of the feed.
    pub fn push(&mut self, block: Box<dyn Block>) {
        let index = self.blocks.len().saturating_sub(usize::from(matches!(
            self.blocks.last().map(|block| &block.content),
            Some(FeedContent::Incomplete(_))
        )));
        self.blocks
            .insert(index, RetainedBlock::new(FeedContent::Block(block)));
        self.scroll = 0;
        self.anchor = None;
        self.surface = None;
    }

    /// Append a mutable submitted-source block and follow it.
    pub fn push_evaluation(&mut self, block: EvaluationBlock) {
        self.take_incomplete();
        self.blocks
            .push(RetainedBlock::new(FeedContent::Evaluation(block)));
        self.scroll = 0;
        self.anchor = None;
        self.surface = None;
    }

    /// Replace the optional unfinished tail and keep it last.
    pub fn set_incomplete(&mut self, block: IncompleteBlock) {
        self.take_incomplete();
        self.blocks
            .push(RetainedBlock::new(FeedContent::Incomplete(block)));
        self.scroll = 0;
        self.anchor = None;
        self.surface = None;
    }

    /// Remove and return the unfinished tail, when present.
    pub fn take_incomplete(&mut self) -> Option<IncompleteBlock> {
        match self.blocks.last().map(|block| &block.content) {
            Some(FeedContent::Incomplete(_)) => {}
            _ => return None,
        }
        self.surface = None;
        match self.blocks.pop().map(|block| block.content) {
            Some(FeedContent::Incomplete(block)) => {
                self.scroll = 0;
                self.anchor = None;
                Some(block)
            }
            _ => unreachable!("the checked tail changed before removal"),
        }
    }

    /// Borrow the unfinished source preceding the live editor line.
    pub fn incomplete_source(&self) -> Option<&str> {
        match self.blocks.last().map(|block| &block.content) {
            Some(FeedContent::Incomplete(block)) => Some(block.source()),
            _ => None,
        }
    }

    /// Remove every block and reset scrolling.
    pub fn clear(&mut self) {
        self.blocks.clear();
        self.scroll = 0;
        self.anchor = None;
        self.surface = None;
    }

    /// Return the number of stored blocks.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Collect only the text submissions printed, each numbered by its cell.
    pub fn printed(&self) -> Vec<&dyn Block> {
        self.blocks
            .iter()
            .filter_map(|block| match &block.content {
                FeedContent::Evaluation(evaluation) => Some(evaluation.printed()),
                FeedContent::Block(_) | FeedContent::Incomplete(_) => None,
            })
            .flatten()
            .collect()
    }

    /// The lines recalled as `oo`, `ooo` and `oooo`, newest first: the latest that
    /// evaluated to something, whether or not they showed it. A null line is not
    /// counted, so it never takes a place in the rotation.
    pub fn recent_outputs(&self) -> [Option<u32>; RECENT_OUTPUTS] {
        let mut valued = self
            .blocks
            .iter()
            .flat_map(|block| match &block.content {
                FeedContent::Evaluation(evaluation) => {
                    Some(evaluation.valued_numbers()).into_iter().flatten()
                }
                FeedContent::Block(_) | FeedContent::Incomplete(_) => None.into_iter().flatten(),
            })
            .collect::<Vec<_>>();
        valued.sort_unstable_by(|first, second| second.cmp(first));
        std::array::from_fn(|age| valued.get(age).copied())
    }

    /// Collect returned values produced by submissions.
    pub fn values(&self) -> Vec<&dyn Block> {
        self.blocks
            .iter()
            .filter_map(|block| match &block.content {
                FeedContent::Evaluation(evaluation) => Some(evaluation.values()),
                FeedContent::Block(_) | FeedContent::Incomplete(_) => None,
            })
            .flatten()
            .collect()
    }

    /// Collect top-level messages and all submitted results in transcript order.
    pub fn outputs(&self) -> Vec<&dyn Block> {
        let mut outputs = Vec::new();
        for block in &self.blocks {
            match &block.content {
                FeedContent::Block(block) => outputs.push(block.as_ref()),
                FeedContent::Evaluation(evaluation) => outputs.extend(evaluation.outputs()),
                FeedContent::Incomplete(_) => {}
            }
        }
        outputs
    }

    /// Update one evaluation, following only when the viewport was already at bottom.
    pub fn update_evaluation(&mut self, id: u64, update: impl FnOnce(&mut EvaluationBlock)) {
        let anchor = self.anchor.or_else(|| self.scrolled_anchor());
        for block in &mut self.blocks {
            let FeedContent::Evaluation(evaluation) = &mut block.content else {
                continue;
            };
            if evaluation.id() == id {
                update(evaluation);
                block.forget_layout();
                self.anchor = anchor;
                self.surface = None;
                return;
            }
        }
    }

    /// Expand or fold the value under a screen position, keeping the top row in place.
    pub fn toggle_fold(&mut self, position: Position) -> bool {
        let Some(surface) = &self.surface else {
            return false;
        };
        let Some(&(index, area, start)) = surface
            .placements
            .iter()
            .find(|(_, area, _)| area.contains(position))
        else {
            return false;
        };
        let (context, anchor) = (surface.context, surface.anchor);
        let block = &mut self.blocks[index];
        let toggled = match &mut block.content {
            FeedContent::Evaluation(evaluation) => evaluation.toggle_fold(
                Position::new(position.x - area.x, start + position.y - area.y),
                &context,
            ),
            FeedContent::Block(_) | FeedContent::Incomplete(_) => false,
        };
        if toggled {
            block.forget_layout();
            self.anchor = anchor;
            self.surface = None;
        }
        toggled
    }

    /// The link drawn at a screen position, with the cells of it on screen.
    pub fn link_at(&self, position: Position) -> Option<Link> {
        let surface = self.surface.as_ref()?;
        let &(index, area, start) = surface
            .placements
            .iter()
            .find(|(_, area, _)| area.contains(position))?;
        let link = self.blocks[index].content.block().link_at(
            &surface.context,
            Position::new(position.x - area.x, start + position.y - area.y),
        )?;
        let visible = start..start + area.height;
        let cells = link
            .cells
            .iter()
            .filter(|cells| visible.contains(&cells.y))
            .map(|cells| Rect {
                x: cells.x + area.x,
                y: cells.y - start + area.y,
                ..*cells
            })
            .collect();
        Some(Link { cells, ..link })
    }

    /// Move by signed rows, positive toward older content.
    pub fn scroll(&mut self, rows: i8) {
        self.anchor = None;
        self.scroll = if rows < 0 {
            self.scroll.saturating_sub(rows.unsigned_abs().into())
        } else {
            self.scroll.saturating_add(rows as usize)
        };
    }

    /// Return the current distance from the bottom.
    pub fn offset(&self) -> usize {
        self.scroll
    }

    /// Return whether unscrolled content ends in submitted source.
    pub fn ends_in_code(&self) -> bool {
        self.scroll == 0
            && self
                .blocks
                .last()
                .is_some_and(|block| match &block.content {
                    FeedContent::Evaluation(evaluation) => evaluation.outputs().next().is_none(),
                    FeedContent::Incomplete(_) => true,
                    FeedContent::Block(_) => false,
                })
    }

    /// Render the current viewport and return whether it is underfilled.
    pub fn render(
        &mut self,
        context: &RenderContext,
        area: Rect,
        target: &mut Buffer,
        canvas: Style,
    ) -> (bool, Exclusions) {
        TuiBlock::default().style(canvas).render(
            Rect::new(area.x, area.y, UI.feed_margin.min(area.width), area.height),
            target,
        );
        let area = Rect::new(
            area.x + UI.feed_margin,
            area.y,
            area.width.saturating_sub(UI.feed_margin),
            area.height,
        );
        if area.is_empty() {
            self.surface = None;
            return (true, Exclusions::default());
        }
        let context = context.with_width(area.width);
        // A new output relabels the ones before it, which moves no row but repaints.
        if self.labelled != context.recent_outputs() {
            self.labelled = context.recent_outputs();
            self.blocks.iter_mut().for_each(|block| block.cells = None);
        }
        let anchor = self.anchor.take().or_else(|| {
            self.surface.as_ref().and_then(|surface| {
                (self.scroll > 0
                    && surface.scroll == self.scroll
                    && (surface.area.width != area.width || surface.area.height != area.height))
                    .then_some(surface.anchor)
                    .flatten()
            })
        });
        if let Some(anchor) = anchor {
            self.scroll = Self::anchored_scroll(&mut self.blocks, &context, area.height, anchor);
        }
        let (surface, underfilled, exclusions) = Self::surface(
            &mut self.blocks,
            &context,
            area,
            target,
            self.scroll,
            canvas,
        );
        if area.width > 0 {
            for y in 0..area.height {
                let bg = target[(area.x, area.y + y)].bg;
                for x in 1..=UI.feed_margin.min(area.x) {
                    target[(area.x - x, area.y + y)].set_bg(bg);
                }
            }
        }
        self.scroll = surface.scroll;
        self.surface = Some(surface);
        (underfilled, exclusions)
    }

    fn scrolled_anchor(&self) -> Option<(usize, u16)> {
        (self.scroll > 0)
            .then(|| {
                self.surface
                    .as_ref()
                    .filter(|surface| surface.scroll == self.scroll)
                    .and_then(|surface| surface.anchor)
            })
            .flatten()
    }

    fn surface(
        blocks: &mut [RetainedBlock],
        context: &RenderContext,
        area: Rect,
        target: &mut Buffer,
        scroll: usize,
        canvas: Style,
    ) -> (ViewportState, bool, Exclusions) {
        let (mut visible, total) = Self::visible(blocks, context, area.height, scroll);
        let scroll = if visible.rows < area.height && total > area.height.into() {
            total - usize::from(area.height)
        } else if total <= area.height.into() {
            0
        } else {
            scroll
        };
        if scroll != visible.scroll {
            visible = Self::visible(blocks, context, area.height, scroll).0;
        }
        TuiBlock::default().style(canvas).render(area, target);
        let underfilled = visible.rows < area.height;
        let mut y = area.y + area.height.saturating_sub(visible.rows);
        let mut crop = visible.rows.saturating_sub(area.height);
        let mut anchor = None;
        let mut exclusions = Exclusions::default();
        let mut placements = Vec::new();
        for (index, height, bottom) in visible.blocks.into_iter().rev() {
            let (cells, block_exclusions) = blocks[index].render(context, canvas);
            let start = crop.min(height - bottom);
            crop -= start;
            let rows = height - bottom - start;
            paste(
                cells,
                start..start + rows,
                Rect::new(area.x, y, area.width, rows),
                target,
            );
            exclusions.extend(
                block_exclusions.project(start..start + rows)
                    + Offset::new(i32::from(area.x), i32::from(y)),
            );
            anchor = anchor.or((rows > 0).then_some((index, start)));
            placements.push((index, Rect::new(area.x, y, area.width, rows), start));
            y += rows;
        }
        (
            ViewportState {
                area,
                scroll,
                anchor,
                context: *context,
                placements,
            },
            underfilled,
            exclusions,
        )
    }

    fn anchored_scroll(
        blocks: &mut [RetainedBlock],
        context: &RenderContext,
        height: u16,
        (index, row): (usize, u16),
    ) -> usize {
        let Some(block) = blocks.get_mut(index) else {
            return 0;
        };
        let anchored = block.height(context);
        let row = row.min(anchored.saturating_sub(1));
        // The blocks the viewport reaches are laid out, as the walk that draws them
        // will lay them out; those past its bottom count by their estimates on both
        // sides, so the anchor lands where it was.
        let mut shown = anchored - row;
        let below = blocks[index + 1..]
            .iter_mut()
            .map(|block| {
                let block_height = match shown < height {
                    true => block.height(context),
                    false => block.estimated_height(context),
                };
                shown = shown.saturating_add(block_height);
                usize::from(block_height)
            })
            .sum::<usize>();
        (below + usize::from(anchored))
            .saturating_sub(row.into())
            .saturating_sub(height.into())
    }

    fn visible(
        blocks: &mut [RetainedBlock],
        context: &RenderContext,
        height: u16,
        scroll: usize,
    ) -> (Visible, usize) {
        let mut below = scroll;
        let mut rows = 0u16;
        let mut total = 0;
        let mut visible = Vec::new();
        for (index, block) in blocks.iter_mut().enumerate().rev() {
            // Blocks scrolled past keep their estimates; only one reaching the
            // viewport is laid out, and its settled height decides the rest.
            let mut block_height = block.estimated_height(context);
            if below < usize::from(block_height) {
                block_height = block.height(context);
            }
            total += usize::from(block_height);
            if below >= usize::from(block_height) {
                below -= usize::from(block_height);
                continue;
            }
            let bottom = below as u16;
            below = 0;
            rows = rows.saturating_add(block_height - bottom);
            visible.push((index, block_height, bottom));
            if rows >= height {
                break;
            }
        }
        (
            Visible {
                blocks: visible,
                rows,
                scroll,
            },
            total,
        )
    }

    fn buffer(area: Rect, style: Style) -> Buffer {
        let mut buffer = Buffer::empty(area);
        TuiBlock::default().style(style).render(area, &mut buffer);
        buffer
    }
}

/// Fan any change the blocks understand across every one of them.
///
/// The container declares no event vocabulary of its own; it forwards whatever its
/// children can receive, which is what a generic event parameter buys over an
/// associated type — one impl covers every kind, present and future.
impl<E> EventListener<E> for Feed
where
    RetainedBlock: EventListener<E>,
{
    fn on_event(&mut self, event: &E) {
        self.surface = None;
        for block in &mut self.blocks {
            block.on_event(event);
        }
    }
}

/// Bottom-up block selection covering one requested viewport.
struct Visible {
    blocks: Vec<(usize, u16, u16)>,
    rows: u16,
    scroll: usize,
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::ui::cell::{CellState, EvaluationBlock, OutputBlock, TextBlock};
    use crate::ui::controller::settings::theme::groups::Groups;

    /// A one-row block counting actual cell rendering.
    struct Counted(Arc<AtomicUsize>);

    impl Block for Counted {
        fn height(&self, _: &RenderContext) -> u16 {
            1
        }

        fn render_local(&self, _: &RenderContext, _: &mut Buffer) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// A block counting hover queries independently of rendering and layout.
    struct Hovered(Arc<AtomicUsize>);

    impl Block for Hovered {
        fn height(&self, _: &RenderContext) -> u16 {
            10
        }

        fn render_local(&self, _: &RenderContext, _: &mut Buffer) {}

        fn link_at(&self, _: &RenderContext, position: Position) -> Option<Link> {
            self.0.fetch_add(1, Ordering::Relaxed);
            assert!(position.y >= 7, "only the bottom three rows are visible");
            None
        }
    }

    #[test]
    fn hover_queries_only_the_visible_block_under_the_pointer() {
        let counts = [Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))];
        let mut feed = Feed::new(Box::new(Hovered(counts[0].clone())));
        feed.push(Box::new(Hovered(counts[1].clone())));
        let context = RenderContext::new(30, 0);
        let area = Rect::new(4, 5, 30, 3);
        feed.render(
            &context,
            area,
            &mut Buffer::empty(Rect::new(0, 0, 40, 12)),
            Style::default(),
        );
        for pointer in [
            Position::new(5, 4),
            Position::new(5, 8),
            Position::new(0, 5),
            Position::new(39, 5),
        ] {
            assert!(feed.link_at(pointer).is_none());
        }
        assert_eq!(
            counts.each_ref().map(|count| count.load(Ordering::Relaxed)),
            [0, 0]
        );
        let _ = feed.link_at(Position::new(10, 6));
        assert_eq!(
            counts.each_ref().map(|count| count.load(Ordering::Relaxed)),
            [0, 1]
        );
    }

    /// A block whose height doubles in narrow viewports.
    struct Responsive(char);

    impl Block for Responsive {
        fn height(&self, context: &RenderContext) -> u16 {
            match context.width() {
                0..=7 => 2,
                _ => 1,
            }
        }

        fn render_local(&self, context: &RenderContext, buffer: &mut Buffer) {
            for row in 0..self.height(context) {
                buffer[(0, row)].set_char(match row {
                    0 => self.0,
                    _ => self.0.to_ascii_lowercase(),
                });
            }
        }
    }

    /// A tall cached block exposing which local rows reach the viewport.
    struct Tall(Arc<AtomicUsize>);

    impl Block for Tall {
        fn height(&self, _: &RenderContext) -> u16 {
            100
        }

        fn render_local(&self, _: &RenderContext, buffer: &mut Buffer) {
            self.0.fetch_add(1, Ordering::Relaxed);
            for row in 0..100 {
                buffer[(0, row)].set_char(char::from_digit(u32::from(row % 10), 10).unwrap());
            }
        }
    }

    #[test]
    fn toggling_a_fold_keeps_the_top_row_in_place() {
        let rows = (0..30)
            .map(|row| format!("row {row:02}"))
            .collect::<Vec<_>>();
        let mut evaluation = EvaluationBlock::new(1, [("x".to_string(), Vec::new())]);
        evaluation.succeed(
            0,
            1,
            CellState::Value,
            Vec::new(),
            None,
            Some(OutputBlock::value(rows.join("\n"), None)),
        );
        let mut feed = Feed::new(Box::new(TextBlock::printed("")));
        feed.push_evaluation(evaluation);
        let context = RenderContext::new(40, 0).with_output_limit(8);
        let area = Rect::new(0, 0, 40, 10);
        let screen = |feed: &mut Feed| {
            let mut target = Buffer::empty(area);
            feed.render(&context, area, &mut target, Style::default());
            (0..area.height)
                .map(|y| (0..area.width).map(|x| target[(x, y)].symbol()).collect())
                .collect::<Vec<String>>()
        };

        let folded = screen(&mut feed);
        assert!(folded.iter().any(|row| row.contains("rows hidden")));
        assert!(!feed.toggle_fold(Position::new(0, 20)), "outside the feed");
        assert!(feed.toggle_fold(Position::new(10, 5)));
        let expanded = screen(&mut feed);
        assert_eq!(expanded[0], folded[0]);
        assert!(!expanded.iter().any(|row| row.contains("rows hidden")));
        assert!(expanded[8].contains("row 07"), "{expanded:#?}");

        assert!(!feed.toggle_fold(Position::new(10, 5)), "inside the value");
        let border = expanded[5]
            .chars()
            .position(|symbol| symbol == '│')
            .expect("the value's border crosses its rows");
        assert!(feed.toggle_fold(Position::new(border as u16, 5)));
        assert!(screen(&mut feed)
            .iter()
            .any(|row| row.contains("rows hidden")));
    }

    #[test]
    fn rendering_prepares_only_visible_blocks_and_keeps_them_when_scrolling() {
        let counts = (0..3)
            .map(|_| Arc::new(AtomicUsize::new(0)))
            .collect::<Vec<_>>();
        let mut feed = Feed::new(Box::new(Counted(counts[0].clone())));
        feed.push(Box::new(Counted(counts[1].clone())));
        feed.push(Box::new(Counted(counts[2].clone())));
        let context = RenderContext::new(10, 0);
        let area = Rect::new(0, 0, 10, 1);
        let mut target = Buffer::empty(area);

        feed.render(&context, area, &mut target, Style::default());
        feed.render(&context, area, &mut target, Style::default());
        assert_eq!(
            counts
                .iter()
                .map(|count| count.load(Ordering::Relaxed))
                .collect::<Vec<_>>(),
            vec![0, 0, 1]
        );

        feed.scroll(1);
        feed.render(&context, area, &mut target, Style::default());
        feed.scroll(-1);
        feed.render(&context, area, &mut target, Style::default());
        assert_eq!(
            counts
                .iter()
                .map(|count| count.load(Ordering::Relaxed))
                .collect::<Vec<_>>(),
            vec![0, 1, 1]
        );
    }

    /// A one-row block counting how often it is laid out.
    struct Measured(Arc<AtomicUsize>);

    impl Block for Measured {
        fn height(&self, _: &RenderContext) -> u16 {
            self.0.fetch_add(1, Ordering::Relaxed);
            1
        }

        fn render_local(&self, _: &RenderContext, _: &mut Buffer) {}
    }

    #[test]
    fn a_change_lays_out_again_only_the_blocks_on_screen() {
        let counts = (0..3)
            .map(|_| Arc::new(AtomicUsize::new(0)))
            .collect::<Vec<_>>();
        let mut feed = Feed::new(Box::new(Measured(counts[0].clone())));
        feed.push(Box::new(Measured(counts[1].clone())));
        feed.push(Box::new(Measured(counts[2].clone())));
        let context = RenderContext::new(10, 0);
        let area = Rect::new(0, 0, 10, 1);
        let mut target = Buffer::empty(area);
        feed.scroll(1);
        feed.render(&context, area, &mut target, Style::default());
        counts
            .iter()
            .for_each(|count| count.store(0, Ordering::Relaxed));

        feed.on_event(&Reformatted);
        feed.render(&context, area, &mut target, Style::default());

        assert_eq!(
            counts
                .iter()
                .map(|count| count.load(Ordering::Relaxed))
                .collect::<Vec<_>>(),
            vec![0, 1, 0]
        );
    }

    #[test]
    fn scrolling_inside_a_tall_block_copies_only_its_visible_slice() {
        let renders = Arc::new(AtomicUsize::new(0));
        let mut feed = Feed::new(Box::new(Tall(renders.clone())));
        let context = RenderContext::new(10, 0);
        let area = Rect::new(0, 0, 10, 3);
        let mut target = Buffer::empty(area);

        feed.render(&context, area, &mut target, Style::default());
        assert_eq!(target[(UI.feed_margin, 0)].symbol(), "7");
        assert_eq!(target[(UI.feed_margin, 2)].symbol(), "9");
        feed.scroll(1);
        feed.render(&context, area, &mut target, Style::default());
        assert_eq!(target[(UI.feed_margin, 0)].symbol(), "6");
        assert_eq!(target[(UI.feed_margin, 2)].symbol(), "8");
        assert_eq!(renders.load(Ordering::Relaxed), 1);
    }

    /// A block whose selection geometry counts how often it is derived.
    struct Excluding(Arc<AtomicUsize>);

    impl Block for Excluding {
        fn height(&self, _: &RenderContext) -> u16 {
            1
        }

        fn render_local(&self, _: &RenderContext, _: &mut Buffer) {}

        fn local_selection_exclusions(&self, _: &RenderContext, exclusions: &mut Exclusions) {
            self.0.fetch_add(1, Ordering::Relaxed);
            exclusions.push(Rect::new(0, 0, 1, 1));
        }
    }

    #[test]
    fn a_retheme_repaints_without_rederiving_selection_geometry() {
        let derivations = Arc::new(AtomicUsize::new(0));
        let mut feed = Feed::new(Box::new(Excluding(derivations.clone())));
        let context = RenderContext::new(10, 0);
        let area = Rect::new(0, 0, 10, 1);
        let mut target = Buffer::empty(area);

        feed.render(&context, area, &mut target, Style::default());
        assert_eq!(derivations.load(Ordering::Relaxed), 1);

        feed.on_event(&Rethemed);
        feed.render(&context, area, &mut target, Style::default());
        assert_eq!(
            derivations.load(Ordering::Relaxed),
            1,
            "a scheme cannot move a rectangle"
        );

        feed.on_event(&Resized { width: 4 });
        feed.render(&context, area, &mut target, Style::default());
        assert_eq!(
            derivations.load(Ordering::Relaxed),
            2,
            "a width moves every wrap boundary"
        );
    }

    #[test]
    fn appending_keeps_rendered_block_buffers() {
        let counts = [Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))];
        let mut feed = Feed::new(Box::new(Counted(counts[0].clone())));
        let context = RenderContext::new(10, 0);
        let area = Rect::new(0, 0, 10, 2);
        let mut target = Buffer::empty(area);

        feed.render(&context, area, &mut target, Style::default());
        feed.push(Box::new(Counted(counts[1].clone())));
        feed.render(&context, area, &mut target, Style::default());

        assert_eq!(counts.map(|count| count.load(Ordering::Relaxed)), [1, 1]);
    }

    #[test]
    fn a_tick_redraws_what_is_evaluating_and_nothing_else() {
        let counts = [Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))];
        let mut feed = Feed::new(Box::new(Counted(counts[0].clone())));
        feed.push_evaluation(EvaluationBlock::new(1, [("x".to_string(), Vec::new())]));
        feed.push(Box::new(Counted(counts[1].clone())));
        let context = RenderContext::new(12, 0);
        let area = Rect::new(0, 0, 12, 8);
        let mut target = Buffer::empty(area);
        feed.render(&context, area, &mut target, Style::default());
        let marker = |target: &Buffer| {
            (0..area.height)
                .map(|row| {
                    target[(UI.feed_margin + UI.cell_margin, row)]
                        .symbol()
                        .to_string()
                })
                .collect::<Vec<_>>()
        };
        let before = marker(&target);

        std::thread::sleep(std::time::Duration::from_millis(
            UI.spinner.frame_ms * 3 / 2,
        ));
        feed.on_event(&Ticked);
        feed.render(&context, area, &mut target, Style::default());

        assert_eq!(counts.map(|count| count.load(Ordering::Relaxed)), [1, 1]);
        assert_ne!(marker(&target), before, "the spinner should have turned");
    }

    #[test]
    fn incomplete_block_stays_last_and_returns_its_source() {
        let mut feed = Feed::new(Box::new(TextBlock::system("welcome")));
        feed.set_incomplete(IncompleteBlock::new("f(\n".into(), Vec::new()));
        feed.push(Box::new(TextBlock::system("message")));

        assert!(matches!(
            feed.blocks.last().map(|block| &block.content),
            Some(FeedContent::Incomplete(_))
        ));
        assert_eq!(
            feed.take_incomplete().map(IncompleteBlock::into_source),
            Some("f(\n".into())
        );
        assert_eq!(feed.len(), 2);
    }

    #[test]
    fn evaluation_growth_preserves_a_scrolled_viewport() {
        let mut feed = Feed::new(Box::new(Responsive('A')));
        for symbol in ['B', 'C', 'D'] {
            feed.push(Box::new(Responsive(symbol)));
        }
        feed.push_evaluation(EvaluationBlock::new(7, [("x".to_string(), Vec::new())]));
        let context = RenderContext::new(12, 0);
        let area = Rect::new(0, 0, 12, 3);
        let mut target = Buffer::empty(area);
        feed.scroll(2);

        feed.render(&context, area, &mut target, Style::default());
        assert_eq!(target[(UI.feed_margin, 0)].symbol(), "C");
        feed.update_evaluation(7, |evaluation| {
            evaluation.succeed(
                0,
                1,
                CellState::Null,
                Vec::new(),
                Some(TextBlock::printed("result")),
                None,
            );
        });
        feed.update_evaluation(7, |evaluation| {
            evaluation.succeed(
                0,
                1,
                CellState::Null,
                Vec::new(),
                Some(TextBlock::printed("another result")),
                None,
            );
        });
        feed.render(&context, area, &mut target, Style::default());

        assert_eq!(target[(UI.feed_margin, 0)].symbol(), "C");
        assert!(feed.offset() > 1);
    }

    #[test]
    fn resizing_preserves_the_top_visible_block() {
        let mut feed = Feed::new(Box::new(Responsive('A')));
        for symbol in ['B', 'C', 'D', 'E', 'F'] {
            feed.push(Box::new(Responsive(symbol)));
        }
        let context = RenderContext::new(12, 0);
        let wide = Rect::new(0, 0, 12, 3);
        let mut target = Buffer::empty(wide);
        feed.scroll(2);

        feed.render(&context, wide, &mut target, Style::default());
        assert_eq!(target[(UI.feed_margin, 0)].symbol(), "B");

        let narrow = Rect::new(0, 0, 8, 3);
        let mut target = Buffer::empty(narrow);
        feed.render(&context, narrow, &mut target, Style::default());

        assert_eq!(target[(UI.feed_margin, 0)].symbol(), "B");

        feed.scroll(-i8::MAX);
        feed.render(&context, narrow, &mut target, Style::default());
        assert_eq!(target[(UI.feed_margin, 1)].symbol(), "F");
        assert_eq!(target[(UI.feed_margin, 2)].symbol(), "f");
    }

    #[test]
    fn resizing_preserves_the_top_block_row() {
        let mut feed = Feed::new(Box::new(Responsive('A')));
        for symbol in ['B', 'C', 'D', 'E', 'F'] {
            feed.push(Box::new(Responsive(symbol)));
        }
        let context = RenderContext::new(8, 0);
        let tall = Rect::new(0, 0, 8, 3);
        let mut target = Buffer::empty(tall);
        feed.scroll(2);

        feed.render(&context, tall, &mut target, Style::default());
        assert_eq!(target[(UI.feed_margin, 0)].symbol(), "d");

        let short = Rect::new(0, 0, 8, 2);
        let mut target = Buffer::empty(short);
        feed.render(&context, short, &mut target, Style::default());

        assert_eq!(target[(UI.feed_margin, 0)].symbol(), "d");
        assert_eq!(feed.offset(), 3);
    }

    #[test]
    fn feed_margin_keeps_a_full_width_block_background() {
        let groups = Groups::default();
        let context = RenderContext::new(12, 0);
        let mut feed = Feed::new(Box::new(TextBlock::printed("x")));
        let area = Rect::new(0, 0, 12, 3);
        let mut target = Buffer::empty(area);

        feed.render(&context, area, &mut target, Style::default());

        // The bottom row is the block's inner margin, still on the printed band.
        assert_eq!(target[(0, 2)].symbol(), " ");
        assert_eq!(target[(0, 2)].bg, groups.printed().bg.unwrap());
        assert_eq!(target[(5, 2)].bg, groups.printed().bg.unwrap());
        let row: String = (0..area.width).map(|x| target[(x, 1)].symbol()).collect();
        assert!(row.contains('x'));
    }

    #[test]
    fn nested_output_inherits_the_feed_surface() {
        let mut evaluation = EvaluationBlock::new(1, [("x".into(), Vec::new())]);
        evaluation.succeed(
            0,
            1,
            CellState::Value,
            Vec::new(),
            None,
            Some(OutputBlock::value("x".into(), None).numbered(1)),
        );
        let groups = Groups::default();
        let context = RenderContext::new(20, 0);
        let area = Rect::new(0, 0, 20, 6);
        let mut target = Buffer::empty(area);
        let mut feed = Feed::new(Box::new(evaluation));

        feed.render(&context, area, &mut target, groups.transcript());

        assert_eq!(
            target[(UI.feed_margin + 5, 4)].bg,
            groups.transcript().bg.unwrap()
        );
    }

    #[test]
    fn retained_output_keeps_its_label_and_updates_its_bold_style() {
        let mut evaluation = EvaluationBlock::new(1, [("x".into(), Vec::new())]);
        evaluation.succeed(
            0,
            1,
            CellState::Value,
            Vec::new(),
            None,
            Some(OutputBlock::value("x".into(), None).numbered(1)),
        );
        let mut feed = Feed::new(Box::new(TextBlock::system("")));
        feed.push_evaluation(evaluation);
        let area = Rect::new(0, 0, 30, 12);
        let drawn = |feed: &mut Feed, recent: [Option<u32>; RECENT_OUTPUTS]| {
            let mut target = Buffer::empty(area);
            let context = RenderContext::new(30, 0).with_recent_outputs(recent);
            feed.render(&context, area, &mut target, Style::default());
            let text = (0..area.height)
                .flat_map(|y| (0..area.width).map(move |x| (x, y)))
                .map(|position| target[position].symbol().to_string())
                .collect::<String>();
            let bold_label = target.content.iter().any(|cell| {
                cell.symbol() == "o" && cell.modifier.contains(ratatui::style::Modifier::BOLD)
            });
            (text, bold_label)
        };

        for recent in [[Some(1), None, None], [Some(2), Some(1), None]] {
            let (text, bold) = drawn(&mut feed, recent);
            assert!(text.contains("o1"));
            assert!(bold);
        }
        let (text, bold) = drawn(&mut feed, [Some(4), Some(3), Some(2)]);
        assert!(text.contains("o1"));
        assert!(!bold);
    }

    #[test]
    fn muted_lines_count_towards_what_m2_recalls_as_oo_ooo_oooo() {
        let mut evaluation = EvaluationBlock::new(
            1,
            ["1;", "2;", "3"].map(|code| (code.to_string(), Vec::new())),
        );
        for (index, number) in [(0, 54), (1, 55)] {
            evaluation.succeed(index, number, CellState::Muted, Vec::new(), None, None);
        }
        evaluation.succeed(
            2,
            56,
            CellState::Value,
            Vec::new(),
            None,
            Some(OutputBlock::value("3".into(), None).numbered(56)),
        );
        let mut feed = Feed::new(Box::new(TextBlock::system("")));
        feed.push_evaluation(evaluation);

        assert_eq!(feed.recent_outputs(), [Some(56), Some(55), Some(54)]);
    }

    #[test]
    fn null_lines_take_no_place_in_the_rotation() {
        let codes = ["1;", "null;", "1;", "null"];
        let mut evaluation =
            EvaluationBlock::new(1, codes.map(|code| (code.to_string(), Vec::new())));
        for (index, (number, state)) in [
            (239, CellState::Muted),
            (240, CellState::Null),
            (241, CellState::Muted),
            (242, CellState::Null),
        ]
        .into_iter()
        .enumerate()
        {
            evaluation.succeed(index, number, state, Vec::new(), None, None);
        }
        let mut feed = Feed::new(Box::new(TextBlock::system("")));
        feed.push_evaluation(evaluation);

        assert_eq!(feed.recent_outputs(), [Some(241), Some(239), None]);
    }
}
