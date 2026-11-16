//! Retained transcript rendering and interaction limited to the viewport.

use std::collections::HashMap;
use std::ops::Range;

use crate::code::highlight::Highlighter;
use crate::code::semantic::SourceId;

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Offset, Position, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block as TuiBlock, Paragraph, Widget};

use super::block::{Block, CellPoint, Link, Reformatted, Resized, Rethemed, Ticked};
use super::cell::{EvaluationBlock, IncompleteBlock};
use super::config::UI;
use super::element::{paste, EventListener};
use super::graphics::{self, kitty::ImageId};
use super::render::selection::Inclusions;
use super::render::{RenderContext, RECENT_OUTPUTS};

mod selection;

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
    fn block_mut(&mut self) -> &mut dyn Block {
        match self {
            Self::Block(block) => block.as_mut(),
            Self::Evaluation(block) => block,
            Self::Incomplete(block) => block,
        }
    }

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
    inclusions: Option<Inclusions>,
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
            inclusions: None,
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

    fn render(&mut self, context: &RenderContext, canvas: Style) -> (&Buffer, &Inclusions) {
        let height = self.height(context);
        if self.cells.is_none() {
            let mut cells = Feed::buffer(Rect::new(0, 0, context.width(), height), canvas);
            self.content.block().render_local(context, &mut cells);
            self.cells = Some(cells);
        }
        if self.inclusions.is_none() {
            let mut inclusions = Inclusions::default();
            self.content
                .block()
                .local_selection_inclusions(context, &mut inclusions);
            self.inclusions = Some(inclusions);
        }
        (
            self.cells.as_ref().expect("cells were just settled"),
            self.inclusions
                .as_ref()
                .expect("inclusions were just settled"),
        )
    }

    fn invalidate(&mut self) {
        self.cells = None;
        self.inclusions = None;
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

/// Rows between two row ranges: none when they overlap.
fn rows_between(block: &Range<usize>, viewport: &Range<usize>) -> usize {
    let above = viewport.start.saturating_sub(block.end);
    let below = block.start.saturating_sub(viewport.end);
    above.max(below)
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
    ) -> Inclusions {
        if area.is_empty() {
            return Inclusions::default();
        }
        if self.blocks.is_empty() {
            Paragraph::new(self.empty)
                .style(context.groups().muted())
                .alignment(Alignment::Center)
                .render(area, target);
            return Inclusions::default();
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
        let viewport = *scroll..*scroll + usize::from(area.height);
        let mut top = 0;
        graphics::report_distances(self.blocks.iter().zip(&heights).flat_map(
            |(block, &height)| {
                let rows = top..top + usize::from(height);
                top = rows.end;
                let distance = rows_between(&rows, &viewport);
                block
                    .pictures()
                    .into_iter()
                    .map(move |image| (image, distance))
            },
        ));
        let mut skip = *scroll;
        let mut y = 0;
        let mut inclusions = Inclusions::default();
        for (block, height) in self.blocks.iter().zip(heights) {
            if skip >= height.into() {
                skip -= usize::from(height);
                continue;
            }
            let start = skip as u16;
            skip = 0;
            let rows = (height - start).min(area.height - y);
            let mut rendered = Feed::buffer(Rect::new(0, 0, context.width(), height), canvas);
            block.render_local(context, &mut rendered);
            paste(
                &rendered,
                start..start + rows,
                Rect::new(area.x, area.y + y, area.width, rows),
                target,
            );
            let mut source = Inclusions::default();
            block.local_selection_inclusions(context, &mut source);
            inclusions.merge(
                source.project(start..start + rows)
                    + Offset::new(i32::from(area.x), i32::from(area.y + y)),
            );
            y += rows;
            if y == area.height {
                break;
            }
        }
        inclusions
    }
}

/// Transcript blocks with their scrolling and retained presentation state.
pub struct Feed {
    focus_pending: bool,
    blocks: Vec<RetainedBlock>,
    scroll: usize,
    anchor: Option<(usize, u16)>,
    surface: Option<ViewportState>,
    /// The latest outputs the retained cells were labelled for.
    labelled: [Option<u32>; RECENT_OUTPUTS],
    /// Whether the block the feed was created with still leads it.
    opened: bool,
    /// The cell or result taken whole, if one is.
    selected: Option<selection::BlockSelection>,
}

impl Feed {
    /// Create a feed containing one initial block.
    pub fn new(block: Box<dyn Block>) -> Self {
        Self {
            focus_pending: false,
            blocks: vec![RetainedBlock::new(FeedContent::Block(block))],
            scroll: 0,
            anchor: None,
            surface: None,
            labelled: [None; RECENT_OUTPUTS],
            opened: true,
            selected: None,
        }
    }

    /// Replace the block the feed was created with, unless it was cleared away.
    pub fn replace_opening(&mut self, block: Box<dyn Block>) {
        if self.opened {
            self.blocks[0] = RetainedBlock::new(FeedContent::Block(block));
            self.surface = None;
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
        self.selected = None;
        self.opened = false;
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

    /// Repaint source colors without following the bottom or replacing output blocks.
    pub fn rehighlight(&mut self, sources: &[SourceId], highlighter: &mut dyn Highlighter) {
        let cells: HashMap<_, _> = sources
            .iter()
            .filter_map(|&source| match source {
                SourceId::Cell {
                    submission, index, ..
                } => Some(((submission, index), source)),
                _ => None,
            })
            .collect();
        let pending = sources
            .iter()
            .find(|source| matches!(source, SourceId::Pending(_)))
            .copied();
        let anchor = self.anchor.or_else(|| self.scrolled_anchor());
        let mut changed = false;
        for block in &mut self.blocks {
            let repainted = match &mut block.content {
                FeedContent::Evaluation(evaluation) => evaluation.rehighlight(&cells, highlighter),
                FeedContent::Incomplete(incomplete) => {
                    pending.is_some_and(|id| incomplete.rehighlight(id, highlighter))
                }
                FeedContent::Block(_) => false,
            };
            if repainted {
                block.cells = None;
                changed = true;
            }
        }
        if changed {
            self.anchor = anchor;
            self.surface = None;
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
        let toggled = block.content.block_mut().toggle_fold(
            Position::new(position.x - area.x, start + position.y - area.y),
            &context,
        );
        if toggled {
            block.forget_layout();
            self.anchor = anchor;
            self.surface = None;
        }
        toggled
    }

    /// Draw the value whose renderer icon is at a screen position by its next
    /// renderer, keeping the top row in place.
    pub fn cycle_renderer(&mut self, position: Position) -> bool {
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
        let cycled = block.content.block_mut().cycle_renderer(
            Position::new(position.x - area.x, start + position.y - area.y),
            &context,
        );
        if cycled {
            block.forget_layout();
            self.anchor = anchor;
            self.surface = None;
        }
        cycled
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

    /// The submitted code drawn at a screen position.
    pub fn code_at(&self, position: Position) -> Option<CellPoint> {
        let surface = self.surface.as_ref()?;
        let &(index, area, start) = surface
            .placements
            .iter()
            .find(|(_, area, _)| area.contains(position))?;
        self.blocks[index].content.block().code_at(
            &surface.context,
            Position::new(position.x - area.x, start + position.y - area.y),
        )
    }

    /// Move by signed rows, positive toward older content.
    pub fn scroll(&mut self, rows: isize) {
        self.anchor = None;
        self.scroll = self.scroll.saturating_add_signed(rows);
    }

    /// The rows of the view, as last drawn.
    pub fn height(&self) -> u16 {
        self.surface
            .as_ref()
            .map_or(0, |surface| surface.area.height)
    }

    /// Bring the start of the block at the top of the view to the top, or the
    /// start of the one before it when it is already there.
    pub fn scroll_to_block_start(&mut self) {
        let Some(&(index, _, start)) = self
            .surface
            .as_ref()
            .and_then(|surface| surface.placements.first())
        else {
            return;
        };
        let index = match start {
            0 => index.saturating_sub(1),
            _ => index,
        };
        self.anchor = Some((index, 0));
    }

    /// Bring the end of the block at the bottom of the view to the bottom, or the
    /// end of the one after it when it is already there.
    pub fn scroll_to_block_end(&mut self) {
        let Some(&(index, area, start)) = self
            .surface
            .as_ref()
            .and_then(|surface| surface.placements.last())
        else {
            return;
        };
        let height =
            |block: &RetainedBlock| block.layout.map_or(1, |layout| usize::from(layout.height));
        let hidden = height(&self.blocks[index]).saturating_sub(usize::from(start + area.height));
        let rows = match (hidden, self.blocks.get(index + 1)) {
            (0, Some(next)) => height(next),
            (hidden, _) => hidden,
        };
        self.anchor = None;
        self.scroll = self.scroll.saturating_sub(rows);
    }

    /// Open at the beginning of retained content.
    pub fn scroll_to_top(&mut self) {
        self.anchor = None;
        self.scroll = usize::MAX;
    }

    /// Honour a source block's requested initial location on the next layout.
    pub fn focus_target(&mut self) {
        self.focus_pending = true;
    }

    /// Return the current distance from the bottom.
    pub fn offset(&self) -> usize {
        self.scroll
    }

    /// Render the current viewport and return whether it is underfilled.
    pub fn render(
        &mut self,
        context: &RenderContext,
        area: Rect,
        target: &mut Buffer,
        canvas: Style,
    ) -> (bool, Inclusions) {
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
            return (true, Inclusions::default());
        }
        // Content keeps the width it was given, settled behind a dragged border; the
        // viewport crops or pads it to the area meanwhile.
        let context = context.with_width(context.width().saturating_sub(UI.feed_margin));
        if std::mem::take(&mut self.focus_pending) {
            self.anchor = self.blocks.iter().enumerate().find_map(|(index, block)| {
                block
                    .content
                    .block()
                    .focus_row(&context)
                    .map(|row| (index, row))
            });
        }
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
        let (surface, underfilled, inclusions) = Self::surface(
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
        self.paint_selection(&context, target);
        (underfilled, inclusions)
    }

    /// Every picture any block holds, wherever the viewport stands.
    pub fn pictures(&self) -> Vec<ImageId> {
        self.blocks
            .iter()
            .flat_map(|block| block.content.block().pictures())
            .collect()
    }

    /// How far each picture stands from the viewport, in rows: none for a block on
    /// screen, else the rows of the blocks between it and the nearest one that is. A
    /// feed not drawn yet holds none.
    pub fn picture_distances(&self) -> Vec<(ImageId, usize)> {
        let Some(surface) = &self.surface else {
            return Vec::new();
        };
        let on_screen = surface.placements.iter().map(|(index, ..)| *index);
        let (Some(first), Some(last)) = (on_screen.clone().min(), on_screen.max()) else {
            return Vec::new();
        };
        let (first, after) = (
            first.min(self.blocks.len()),
            (last + 1).min(self.blocks.len()),
        );
        let holding = |block: &RetainedBlock, rows: usize| {
            block
                .content
                .block()
                .pictures()
                .into_iter()
                .map(move |image| (image, rows))
        };
        let mut distances = Vec::new();
        let mut between = 0;
        for block in self.blocks[..first].iter().rev() {
            distances.extend(holding(block, between));
            between += block.layout.map_or(1, |layout| usize::from(layout.height));
        }
        for block in &self.blocks[first..after] {
            distances.extend(holding(block, 0));
        }
        let mut between = 0;
        for block in &self.blocks[after..] {
            distances.extend(holding(block, between));
            between += block.layout.map_or(1, |layout| usize::from(layout.height));
        }
        distances
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
    ) -> (ViewportState, bool, Inclusions) {
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
        let mut inclusions = Inclusions::default();
        let mut placements = Vec::new();
        for (index, height, bottom) in visible.blocks.into_iter().rev() {
            let (cells, block_inclusions) = blocks[index].render(context, canvas);
            let start = crop.min(height - bottom);
            crop -= start;
            let rows = height - bottom - start;
            paste(
                cells,
                start..start + rows,
                Rect::new(area.x, y, area.width, rows),
                target,
            );
            inclusions.merge(
                block_inclusions.project(start..start + rows)
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
            inclusions,
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
