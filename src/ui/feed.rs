//! Retained, viewport-sized transcript rendering.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Offset, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block as TuiBlock, Paragraph, Widget};

use super::block::{Block, Exclusions};
use super::cell::{EvaluationBlock, IncompleteBlock};
use super::config::UI;
use super::render::RenderContext;

/// Retained position metadata for one feed viewport.
struct ViewportState {
    area: Rect,
    scroll: usize,
    anchor: Option<(usize, u16)>,
}

/// One feed element with its lazily computed local layout and cells.
struct RetainedBlock {
    content: FeedContent,
    layout: Option<(u16, u16)>,
    buffer: Option<BlockBuffer>,
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

/// Rendered cells and selection metadata local to one feed block.
struct BlockBuffer {
    cells: Buffer,
    exclusions: Exclusions,
}

impl RetainedBlock {
    fn new(content: FeedContent) -> Self {
        Self {
            content,
            layout: None,
            buffer: None,
        }
    }

    fn height(&mut self, context: &RenderContext) -> u16 {
        if self
            .layout
            .is_none_or(|(width, _)| width != context.width())
        {
            self.layout = Some((context.width(), self.content.block().height(context).max(1)));
            self.buffer = None;
        }
        self.layout.unwrap().1
    }

    fn render(&mut self, context: &RenderContext, canvas: Style) -> &BlockBuffer {
        let height = self.height(context);
        self.buffer.get_or_insert_with(|| {
            let mut cells = Feed::buffer(Rect::new(0, 0, context.width(), height), canvas);
            self.content.block().render_local(context, &mut cells);
            let mut exclusions = Exclusions::default();
            self.content
                .block()
                .local_selection_exclusions(context, &mut exclusions);
            BlockBuffer { cells, exclusions }
        })
    }

    fn invalidate(&mut self) {
        self.buffer = None;
    }

    fn invalidate_layout(&mut self) {
        self.layout = None;
        self.invalidate();
    }
}

/// Transcript blocks with their scrolling and retained presentation state.
pub(crate) struct Feed {
    blocks: Vec<RetainedBlock>,
    scroll: usize,
    anchor: Option<(usize, u16)>,
    surface: Option<ViewportState>,
}

/// A borrowed block list rendered through a scrollable rectangular viewport.
pub(crate) struct BlockView<'a> {
    blocks: &'a [&'a dyn Block],
    empty: &'a str,
}

impl<'a> BlockView<'a> {
    /// Borrow blocks and the message shown when that list is empty.
    pub(crate) const fn new(blocks: &'a [&'a dyn Block], empty: &'a str) -> Self {
        Self { blocks, empty }
    }

    /// Render only blocks intersecting the selected rows and clamp `scroll`.
    pub(crate) fn render(
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
            Feed::copy(
                &rendered,
                start,
                target,
                Rect::new(area.x, area.y + y, area.width, rows),
                rows,
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

impl Feed {
    /// Create a feed containing one initial block.
    pub(crate) fn new(block: Box<dyn Block>) -> Self {
        Self {
            blocks: vec![RetainedBlock::new(FeedContent::Block(block))],
            scroll: 0,
            anchor: None,
            surface: None,
        }
    }

    /// Append a block and follow the new bottom of the feed.
    pub(crate) fn push(&mut self, block: Box<dyn Block>) {
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
    pub(crate) fn push_evaluation(&mut self, block: EvaluationBlock) {
        self.take_incomplete();
        self.blocks
            .push(RetainedBlock::new(FeedContent::Evaluation(block)));
        self.scroll = 0;
        self.anchor = None;
        self.surface = None;
    }

    /// Replace the optional unfinished tail and keep it last.
    pub(crate) fn set_incomplete(&mut self, block: IncompleteBlock) {
        self.take_incomplete();
        self.blocks
            .push(RetainedBlock::new(FeedContent::Incomplete(block)));
        self.scroll = 0;
        self.anchor = None;
        self.surface = None;
    }

    /// Remove and return the unfinished tail, when present.
    pub(crate) fn take_incomplete(&mut self) -> Option<IncompleteBlock> {
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
    pub(crate) fn incomplete_source(&self) -> Option<&str> {
        match self.blocks.last().map(|block| &block.content) {
            Some(FeedContent::Incomplete(block)) => Some(block.source()),
            _ => None,
        }
    }

    /// Remove every block and reset scrolling.
    pub(crate) fn clear(&mut self) {
        self.blocks.clear();
        self.scroll = 0;
        self.anchor = None;
        self.surface = None;
    }

    /// Return whether no blocks are stored.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Return the number of stored blocks.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Collect printed text, warnings, and errors produced by submissions.
    pub(crate) fn evaluation_outputs(&self) -> Vec<&dyn Block> {
        self.blocks
            .iter()
            .filter_map(|block| match &block.content {
                FeedContent::Evaluation(evaluation) => Some(evaluation.evaluation_outputs()),
                FeedContent::Block(_) | FeedContent::Incomplete(_) => None,
            })
            .flatten()
            .collect()
    }

    /// Collect returned values produced by submissions.
    pub(crate) fn values(&self) -> Vec<&dyn Block> {
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
    pub(crate) fn outputs(&self) -> Vec<&dyn Block> {
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
    pub(crate) fn update_evaluation(&mut self, id: u64, update: impl FnOnce(&mut EvaluationBlock)) {
        let anchor = self.anchor.or_else(|| self.scrolled_anchor());
        for block in &mut self.blocks {
            let FeedContent::Evaluation(evaluation) = &mut block.content else {
                continue;
            };
            if evaluation.id() == id {
                update(evaluation);
                block.invalidate_layout();
                self.anchor = anchor;
                self.surface = None;
                return;
            }
        }
    }

    /// Move by signed rows, positive toward older content.
    pub(crate) fn scroll(&mut self, rows: i8) {
        self.anchor = None;
        self.scroll = if rows < 0 {
            self.scroll.saturating_sub(rows.unsigned_abs().into())
        } else {
            self.scroll.saturating_add(rows as usize)
        };
    }

    /// Return the current distance from the bottom.
    pub(crate) fn offset(&self) -> usize {
        self.scroll
    }

    /// Return whether unscrolled content ends in submitted source.
    pub(crate) fn ends_in_code(&self) -> bool {
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

    /// Return to the newest transcript row.
    pub(crate) fn follow(&mut self) {
        self.scroll = 0;
        self.anchor = None;
    }

    /// Discard retained presentation after a theme change.
    pub(crate) fn invalidate(&mut self) {
        self.surface = None;
        self.blocks.iter_mut().for_each(RetainedBlock::invalidate);
    }

    /// Return whether the current viewport has retained rendered cells.
    #[cfg(test)]
    pub(crate) fn retained(&self) -> bool {
        self.surface.is_some()
    }

    /// Render the current viewport and return whether it is underfilled.
    pub(crate) fn render(
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
        let context = RenderContext::new(area.width, context.prefix(), context.groups());
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
        for (index, height, bottom) in visible.blocks.into_iter().rev() {
            let rendered = blocks[index].render(context, canvas);
            let start = crop.min(height - bottom);
            crop -= start;
            let rows = height - bottom - start;
            Self::copy(
                &rendered.cells,
                start,
                target,
                Rect::new(area.x, y, area.width, rows),
                rows,
            );
            exclusions.extend(
                rendered.exclusions.project(start..start + rows)
                    + Offset::new(i32::from(area.x), i32::from(y)),
            );
            anchor = anchor.or((rows > 0).then_some((index, start)));
            y += rows;
        }
        (
            ViewportState {
                area,
                scroll,
                anchor,
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
        let row = row.min(block.height(context).saturating_sub(1));
        blocks[index..]
            .iter_mut()
            .map(|block| usize::from(block.height(context)))
            .sum::<usize>()
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
            let block_height = block.height(context);
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

    fn copy(source: &Buffer, source_y: u16, target: &mut Buffer, area: Rect, rows: u16) {
        let width = usize::from(area.width);
        for row in 0..rows {
            let source_index = source.index_of(source.area().x, source_y + row);
            let target_index = target.index_of(area.x, area.y + row);
            target.content[target_index..target_index + width]
                .clone_from_slice(&source.content[source_index..source_index + width]);
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
    use crate::ui::cell::{CellState, OutputBlock, TextBlock};
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
    fn scrolling_renders_only_newly_visible_blocks() {
        let counts = (0..3)
            .map(|_| Arc::new(AtomicUsize::new(0)))
            .collect::<Vec<_>>();
        let mut feed = Feed::new(Box::new(Counted(counts[0].clone())));
        feed.push(Box::new(Counted(counts[1].clone())));
        feed.push(Box::new(Counted(counts[2].clone())));
        let groups = Groups::default();
        let context = RenderContext::new(10, 0, &groups);
        let area = Rect::new(0, 0, 10, 1);
        let mut target = Buffer::empty(area);

        feed.render(&context, area, &mut target, Style::default());
        feed.render(&context, area, &mut target, Style::default());
        assert_eq!(
            counts
                .iter()
                .map(|count| count.load(Ordering::Relaxed))
                .sum::<usize>(),
            1
        );

        feed.scroll(1);
        feed.render(&context, area, &mut target, Style::default());
        feed.scroll(-1);
        feed.render(&context, area, &mut target, Style::default());
        assert_eq!(
            counts
                .iter()
                .map(|count| count.load(Ordering::Relaxed))
                .sum::<usize>(),
            2
        );
    }

    #[test]
    fn scrolling_inside_a_tall_block_copies_only_its_visible_slice() {
        let renders = Arc::new(AtomicUsize::new(0));
        let mut feed = Feed::new(Box::new(Tall(renders.clone())));
        let groups = Groups::default();
        let context = RenderContext::new(10, 0, &groups);
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

    #[test]
    fn appending_keeps_rendered_block_buffers() {
        let counts = [Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))];
        let mut feed = Feed::new(Box::new(Counted(counts[0].clone())));
        let groups = Groups::default();
        let context = RenderContext::new(10, 0, &groups);
        let area = Rect::new(0, 0, 10, 2);
        let mut target = Buffer::empty(area);

        feed.render(&context, area, &mut target, Style::default());
        feed.push(Box::new(Counted(counts[1].clone())));
        feed.render(&context, area, &mut target, Style::default());

        assert_eq!(counts.map(|count| count.load(Ordering::Relaxed)), [1, 1]);
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
        let groups = Groups::default();
        let context = RenderContext::new(12, 0, &groups);
        let area = Rect::new(0, 0, 12, 3);
        let mut target = Buffer::empty(area);
        feed.scroll(1);

        feed.render(&context, area, &mut target, Style::default());
        assert_eq!(target[(UI.feed_margin, 0)].symbol(), "C");
        feed.update_evaluation(7, |evaluation| {
            evaluation.succeed(
                0,
                1,
                CellState::Null,
                vec![Box::new(TextBlock::printed("result"))],
                None,
            );
        });
        feed.update_evaluation(7, |evaluation| {
            evaluation.succeed(
                0,
                1,
                CellState::Null,
                vec![Box::new(TextBlock::printed("another result"))],
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
        let groups = Groups::default();
        let context = RenderContext::new(12, 0, &groups);
        let wide = Rect::new(0, 0, 12, 3);
        let mut target = Buffer::empty(wide);
        feed.scroll(2);

        feed.render(&context, wide, &mut target, Style::default());
        assert_eq!(target[(UI.feed_margin, 0)].symbol(), "B");

        let narrow = Rect::new(0, 0, 8, 3);
        let mut target = Buffer::empty(narrow);
        feed.render(&context, narrow, &mut target, Style::default());

        assert_eq!(target[(UI.feed_margin, 0)].symbol(), "B");
        assert_eq!(feed.offset(), 7);
    }

    #[test]
    fn resizing_preserves_the_top_block_row() {
        let mut feed = Feed::new(Box::new(Responsive('A')));
        for symbol in ['B', 'C', 'D', 'E', 'F'] {
            feed.push(Box::new(Responsive(symbol)));
        }
        let groups = Groups::default();
        let context = RenderContext::new(8, 0, &groups);
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
        let context = RenderContext::new(10, 0, &groups);
        let mut feed = Feed::new(Box::new(TextBlock::printed("x")));
        let area = Rect::new(0, 0, 10, 3);
        let mut target = Buffer::empty(area);

        feed.render(&context, area, &mut target, Style::default());

        assert_eq!(target[(0, 2)].symbol(), " ");
        assert_eq!(target[(0, 2)].bg, groups.printed().bg.unwrap());
        assert_eq!(target[(5, 2)].symbol(), "x");
    }

    #[test]
    fn nested_output_inherits_the_feed_surface() {
        let mut evaluation = EvaluationBlock::new(1, [("x".into(), Vec::new())]);
        evaluation.succeed(
            0,
            1,
            CellState::Value,
            Vec::new(),
            Some(OutputBlock::value("x".into(), None).numbered(1)),
        );
        let groups = Groups::default();
        let context = RenderContext::new(20, 0, &groups);
        let area = Rect::new(0, 0, 20, 6);
        let mut target = Buffer::empty(area);
        let mut feed = Feed::new(Box::new(evaluation));

        feed.render(&context, area, &mut target, groups.transcript());

        assert_eq!(
            target[(UI.feed_margin + 5, 4)].bg,
            groups.transcript().bg.unwrap()
        );
    }
}
