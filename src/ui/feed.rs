//! Retained, viewport-sized transcript rendering.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block as TuiBlock, Paragraph, Widget};

use super::block::render::RenderContext;
use super::block::{Block, SubmissionBlock};
use super::config::UI;

/// Retained cells and selection metadata for one feed viewport.
struct ViewportBuffer {
    buffer: Buffer,
    scroll: usize,
    exclusions: Vec<Rect>,
    underfilled: bool,
}

/// One feed element with its lazily computed local layout and cells.
struct RetainedBlock {
    content: Box<dyn Block>,
    layout: Option<(u16, u16)>,
    buffer: Option<BlockBuffer>,
}

/// Rendered cells and selection metadata local to one feed block.
struct BlockBuffer {
    cells: Buffer,
    exclusions: Vec<Rect>,
}

impl RetainedBlock {
    fn new(content: Box<dyn Block>) -> Self {
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
            self.layout = Some((context.width(), self.content.height(context).max(1)));
            self.buffer = None;
        }
        self.layout.unwrap().1
    }

    fn render(&mut self, context: &RenderContext, canvas: Style) -> &BlockBuffer {
        let height = self.height(context);
        self.buffer.get_or_insert_with(|| {
            let mut cells = Feed::buffer(Rect::new(0, 0, context.width(), height), canvas);
            self.content.render(context, *cells.area(), &mut cells);
            let mut exclusions = Vec::new();
            self.content
                .selection_exclusions(context, *cells.area(), &mut exclusions);
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
pub struct Feed {
    blocks: Vec<RetainedBlock>,
    scroll: usize,
    surface: Option<ViewportBuffer>,
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
    ) -> Vec<Rect> {
        if area.is_empty() {
            return Vec::new();
        }
        if self.blocks.is_empty() {
            Paragraph::new(self.empty)
                .style(context.groups().muted())
                .alignment(Alignment::Center)
                .render(area, target);
            return Vec::new();
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
        let mut exclusions = Vec::new();
        for (block, height) in self.blocks.iter().zip(heights) {
            if skip >= height.into() {
                skip -= usize::from(height);
                continue;
            }
            let start = skip as u16;
            skip = 0;
            let rows = (height - start).min(area.height - y);
            let mut rendered = Feed::buffer(Rect::new(0, 0, area.width, height), canvas);
            block.render(context, *rendered.area(), &mut rendered);
            Feed::copy(
                &rendered,
                start,
                target,
                Rect::new(area.x, area.y + y, area.width, rows),
                rows,
            );
            let mut source = Vec::new();
            block.selection_exclusions(context, *rendered.area(), &mut source);
            Feed::project(&source, start, rows, area.y + y, &mut exclusions);
            y += rows;
            if y == area.height {
                break;
            }
        }
        for exclusion in &mut exclusions {
            exclusion.x += area.x;
        }
        exclusions
    }
}

impl Feed {
    /// Create a feed containing one initial block.
    pub fn new(block: Box<dyn Block>) -> Self {
        Self {
            blocks: vec![RetainedBlock::new(block)],
            scroll: 0,
            surface: None,
        }
    }

    /// Append a block and follow the new bottom of the feed.
    pub fn push(&mut self, block: Box<dyn Block>) {
        self.blocks.push(RetainedBlock::new(block));
        self.scroll = 0;
        self.invalidate();
    }

    /// Remove every block and reset scrolling.
    pub fn clear(&mut self) {
        self.blocks.clear();
        self.scroll = 0;
        self.invalidate();
    }

    /// Return whether no blocks are stored.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Return the number of stored blocks.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Visit blocks in transcript order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &dyn Block> {
        self.blocks.iter().map(|block| block.content.as_ref())
    }

    /// Update one submitted cell group and follow its changed output.
    pub fn update_submission(&mut self, id: u64, update: impl FnOnce(&mut SubmissionBlock)) {
        for block in &mut self.blocks {
            let Some(submission) = block.content.submission_mut() else {
                continue;
            };
            if submission.id() == id {
                update(submission);
                block.invalidate_layout();
                self.scroll = 0;
                self.surface = None;
                return;
            }
        }
    }

    /// Move by signed rows, positive toward older content.
    pub fn scroll(&mut self, rows: i8) {
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

    /// Return to the newest transcript row.
    pub fn follow(&mut self) {
        self.scroll = 0;
    }

    /// Discard retained presentation after a theme change.
    pub fn invalidate(&mut self) {
        self.surface = None;
        self.blocks.iter_mut().for_each(RetainedBlock::invalidate);
    }

    /// Return whether the current viewport has retained rendered cells.
    #[cfg(test)]
    pub fn retained(&self) -> bool {
        self.surface.is_some()
    }

    /// Render the current viewport and return whether it is underfilled.
    pub fn render(
        &mut self,
        context: &RenderContext,
        area: Rect,
        target: &mut Buffer,
        canvas: Style,
    ) -> (bool, Vec<Rect>) {
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
        let rebuild = self.surface.as_ref().is_none_or(|surface| {
            surface.buffer.area().width != area.width
                || surface.buffer.area().height != area.height
                || surface.scroll != self.scroll
        });
        if rebuild {
            self.surface = Some(Self::surface(
                &mut self.blocks,
                &context,
                area,
                self.scroll,
                canvas,
            ));
        }
        let surface = self.surface.as_ref().unwrap();
        Self::copy(&surface.buffer, 0, target, area, area.height);
        if area.width > 0 {
            for y in 0..area.height {
                let bg = surface.buffer[(0, y)].bg;
                for x in 1..=UI.feed_margin.min(area.x) {
                    target[(area.x - x, area.y + y)].set_bg(bg);
                }
            }
        }
        let exclusions = surface
            .exclusions
            .iter()
            .map(|rect| Rect::new(area.x + rect.x, area.y + rect.y, rect.width, rect.height))
            .collect();
        self.scroll = surface.scroll;
        (surface.underfilled, exclusions)
    }

    fn surface(
        blocks: &mut [RetainedBlock],
        context: &RenderContext,
        area: Rect,
        scroll: usize,
        canvas: Style,
    ) -> ViewportBuffer {
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
        let mut buffer = Self::buffer(Rect::new(0, 0, area.width, area.height), canvas);
        let underfilled = visible.rows < area.height;
        let mut y = area.height.saturating_sub(visible.rows);
        let mut crop = visible.rows.saturating_sub(area.height);
        let mut exclusions = Vec::new();
        for (index, height, bottom) in visible.blocks.into_iter().rev() {
            let rendered = blocks[index].render(context, canvas);
            let start = crop.min(height - bottom);
            crop -= start;
            let rows = height - bottom - start;
            Self::copy(
                &rendered.cells,
                start,
                &mut buffer,
                Rect::new(0, y, area.width, rows),
                rows,
            );
            Self::project(&rendered.exclusions, start, rows, y, &mut exclusions);
            y += rows;
        }
        ViewportBuffer {
            buffer,
            scroll,
            exclusions,
            underfilled,
        }
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
        for row in 0..rows {
            for column in 0..area.width {
                target[(area.x + column, area.y + row)] = source[(column, source_y + row)].clone();
            }
        }
    }

    fn project(source: &[Rect], start: u16, rows: u16, y: u16, target: &mut Vec<Rect>) {
        let end = start + rows;
        target.extend(source.iter().filter_map(|rect| {
            let top = rect.y.max(start);
            let bottom = rect.bottom().min(end);
            (top < bottom).then(|| Rect::new(rect.x, y + top - start, rect.width, bottom - top))
        }));
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
    use crate::ui::block::TextBlock;
    use crate::ui::controller::settings::theme::groups::Groups;
    use crate::ui::element::Element;

    /// A one-row block counting actual cell rendering.
    struct Counted(Arc<AtomicUsize>);

    impl Element for Counted {
        type Context = RenderContext;

        fn render(&self, _context: &RenderContext, _area: Rect, _buffer: &mut Buffer) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    impl Block for Counted {}

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
    fn feed_margin_keeps_a_full_width_block_background() {
        let groups = Groups::default();
        let context = RenderContext::new(10, 0, &groups);
        let mut feed = Feed::new(Box::new(TextBlock::printed("x")));
        let area = Rect::new(0, 0, 10, 3);
        let mut target = Buffer::empty(area);

        feed.render(&context, area, &mut target, Style::default());

        assert_eq!(target[(0, 1)].symbol(), " ");
        assert_eq!(target[(0, 1)].bg, groups.printed().bg.unwrap());
        assert_eq!(target[(5, 1)].symbol(), "x");
    }
}
