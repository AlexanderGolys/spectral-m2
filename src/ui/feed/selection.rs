//! A cell or result of the feed taken whole, by a click outside its text or by
//! stepping through them with the keys.

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};

use super::{Feed, FeedContent};
use crate::ui::config::UI;
use crate::ui::item::{copied_together, Item, ItemKind};
use crate::ui::render::RenderContext;
use crate::ui::settings::selection::CopyFormat;

/// How much of a block is taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Taken {
    /// One of the block's cells or results.
    Item(usize),
    /// All of them: the submission with everything it printed and returned.
    Block,
}

/// The part of the feed taken whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockSelection {
    block: usize,
    taken: Taken,
}

impl Feed {
    pub const fn has_selection(&self) -> bool {
        self.selected.is_some()
    }

    pub fn deselect(&mut self) {
        self.selected = None;
    }

    /// What of a block can be taken: the parts holding any text.
    fn items_of(&self, block: usize, context: &RenderContext) -> Vec<Item> {
        let mut items = self
            .blocks
            .get(block)
            .map_or_else(Vec::new, |block| block.content.block().items(context));
        items.retain(|item| !item.text.trim().is_empty());
        items
    }

    /// Take the cell or result drawn on the row of a screen position, or with `whole`
    /// everything of the block it is in; whether the row had anything to take. The
    /// column does not matter: a click in the margin takes what a click on its text does.
    pub fn select_at(&mut self, position: Position, whole: bool) -> bool {
        let Some(surface) = &self.surface else {
            return false;
        };
        let Some(&(block, area, start)) = surface
            .placements
            .iter()
            .find(|(_, area, _)| (area.y..area.bottom()).contains(&position.y))
        else {
            return false;
        };
        let row = start + position.y - area.y;
        let items = self.items_of(block, &surface.context);
        let taken = match items.iter().position(|item| item.rows.contains(&row)) {
            _ if items.is_empty() => return false,
            Some(item) if !whole => Taken::Item(item),
            _ => Taken::Block,
        };
        self.selected = Some(BlockSelection { block, taken });
        true
    }

    /// Take the `index`th code cell of submission `submission` and scroll it into
    /// view; whether the feed still holds it.
    pub fn select_cell(&mut self, submission: u64, index: usize) -> bool {
        let Some(context) = self.surface.as_ref().map(|surface| surface.context) else {
            return false;
        };
        let Some(block) = self.blocks.iter().position(|retained| {
            matches!(&retained.content, FeedContent::Evaluation(evaluation) if evaluation.id() == submission)
        }) else {
            return false;
        };
        let items = self.items_of(block, &context);
        let Some(item) = items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.kind == ItemKind::Input)
            .nth(index)
            .map(|(item, _)| item)
        else {
            return false;
        };
        let rows = items[item].rows.clone();
        self.selected = Some(BlockSelection {
            block,
            taken: Taken::Item(item),
        });
        self.reveal(block, rows.start, rows.end);
        true
    }

    /// Move the selection to the previous (`delta` negative) or next cell or result,
    /// scrolling it into view. With nothing selected it starts from the last one on
    /// screen. Whether anything is selected after it.
    pub fn step_selection(&mut self, delta: i8) -> bool {
        let Some(context) = self.surface.as_ref().map(|surface| surface.context) else {
            return false;
        };
        let target = match self.selected {
            Some(from) => self.neighbour(from, delta, &context),
            None => self.last_item_shown(&context),
        };
        if let Some((block, item)) = target {
            let rows = self.items_of(block, &context)[item].rows.clone();
            self.selected = Some(BlockSelection {
                block,
                taken: Taken::Item(item),
            });
            self.reveal(block, rows.start, rows.end);
        }
        self.selected.is_some()
    }

    /// The item beside everything `from` takes, in the direction of `delta`.
    fn neighbour(
        &self,
        from: BlockSelection,
        delta: i8,
        context: &RenderContext,
    ) -> Option<(usize, usize)> {
        let count = self.items_of(from.block, context).len();
        let taken = match from.taken {
            Taken::Item(item) => item..item + 1,
            Taken::Block => 0..count,
        };
        let forward = delta > 0;
        let beside = match forward {
            true => Some(taken.end).filter(|&item| item < count),
            false => taken.start.checked_sub(1),
        };
        if let Some(item) = beside {
            return Some((from.block, item));
        }
        let others: Vec<usize> = match forward {
            true => (from.block + 1..self.blocks.len()).collect(),
            false => (0..from.block).rev().collect(),
        };
        others.into_iter().find_map(|block| {
            let count = self.items_of(block, context).len();
            (count > 0).then(|| (block, if forward { 0 } else { count - 1 }))
        })
    }

    /// The last item with a row on screen, or failing that the last of the feed.
    fn last_item_shown(&self, context: &RenderContext) -> Option<(usize, usize)> {
        let surface = self.surface.as_ref()?;
        let shown = surface
            .placements
            .iter()
            .rev()
            .find_map(|&(block, area, start)| {
                let visible = start..start + area.height;
                let items = self.items_of(block, context);
                items
                    .iter()
                    .rposition(|item| {
                        item.rows.start < visible.end && item.rows.end > visible.start
                    })
                    .map(|item| (block, item))
            });
        shown.or_else(|| {
            (0..self.blocks.len()).rev().find_map(|block| {
                let count = self.items_of(block, context).len();
                (count > 0).then(|| (block, count - 1))
            })
        })
    }

    /// Scroll so rows `start..end` of `block` are on screen, unless they already are.
    fn reveal(&mut self, block: usize, start: u16, end: u16) {
        let shown = self.surface.as_ref().is_some_and(|surface| {
            surface.placements.iter().any(|&(shown, area, first)| {
                shown == block && first <= start && end <= first + area.height
            })
        });
        if !shown {
            self.anchor = Some((block, start.saturating_sub(1)));
        }
    }

    /// What the selection copies as, if there is one.
    pub fn selected_text(&self, format: CopyFormat) -> Option<String> {
        let selection = self.selected?;
        let items = self.items_of(selection.block, &self.surface.as_ref()?.context);
        match selection.taken {
            Taken::Item(item) => items.get(item).map(|item| item.copied(format)),
            Taken::Block => Some(copied_together(&items, format)),
        }
    }

    /// Tint the rows of the selection that are on screen, margin included.
    pub(super) fn paint_selection(&self, context: &RenderContext, target: &mut Buffer) {
        let (Some(selection), Some(surface), Some(tint)) = (
            self.selected,
            self.surface.as_ref(),
            context.groups().selection().bg,
        ) else {
            return;
        };
        let Some(&(block, area, start)) = surface
            .placements
            .iter()
            .find(|&&(block, ..)| block == selection.block)
        else {
            return;
        };
        let items = self.items_of(block, context);
        let taken = match selection.taken {
            Taken::Item(item) => items.get(item..=item),
            Taken::Block => Some(items.as_slice()),
        };
        let (Some(first), Some(last)) = (
            taken.and_then(<[Item]>::first),
            taken.and_then(<[Item]>::last),
        ) else {
            return;
        };
        let rows = first.rows.start.max(start)..last.rows.end.min(start + area.height);
        let left = area.x.saturating_sub(UI.feed_margin);
        for row in rows {
            let y = area.y + row - start;
            for x in Rect::new(left, y, area.right() - left, 1)
                .positions()
                .map(|p| p.x)
            {
                target[(x, y)].set_bg(tint);
            }
        }
    }
}
