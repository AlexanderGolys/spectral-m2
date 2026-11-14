//! Stretches of lines a view can fold away to their first line.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::ops::RangeInclusive;
use std::rc::Rc;

/// The foldable stretches of a text's zero-based lines, shared with whoever
/// learns them after the view showing the text was made.
pub type FoldRanges = Rc<RefCell<Vec<RangeInclusive<u32>>>>;

/// Which foldable stretches of a text are folded.
#[derive(Debug, Default)]
pub struct LineFolds {
    ranges: FoldRanges,
    /// The first lines of the folded stretches.
    folded: BTreeSet<u32>,
}

impl LineFolds {
    pub fn new(ranges: FoldRanges) -> Self {
        Self {
            ranges,
            folded: BTreeSet::new(),
        }
    }

    /// The widest stretch starting on `line`.
    pub fn starting_at(&self, line: u32) -> Option<RangeInclusive<u32>> {
        self.ranges
            .borrow()
            .iter()
            .filter(|range| *range.start() == line)
            .max_by_key(|range| *range.end())
            .cloned()
    }

    /// Whether the stretch starting on `line` is folded.
    pub fn is_folded(&self, line: u32) -> bool {
        self.folded.contains(&line) && self.starting_at(line).is_some()
    }

    /// Fold or unfold the stretch starting on `line`; whether one starts there.
    pub fn toggle(&mut self, line: u32) -> bool {
        if self.starting_at(line).is_none() {
            return false;
        }
        if !self.folded.remove(&line) {
            self.folded.insert(line);
        }
        true
    }

    /// Whether `line` is folded away inside a stretch kept to its first line.
    pub fn hides(&self, line: u32) -> bool {
        self.folded.iter().any(|&start| {
            self.starting_at(start)
                .is_some_and(|range| start < line && line <= *range.end())
        })
    }
}

