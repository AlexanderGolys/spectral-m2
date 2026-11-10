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

#[cfg(test)]
mod tests {
    use super::*;

    fn folds(ranges: Vec<RangeInclusive<u32>>) -> LineFolds {
        LineFolds::new(Rc::new(RefCell::new(ranges)))
    }

    #[test]
    fn a_folded_stretch_hides_all_but_its_first_line() {
        let mut folds = folds(vec![0..=3, 6..=10, 7..=9]);
        assert!(!folds.hides(1));
        assert!(folds.toggle(6));
        assert!(folds.is_folded(6));
        assert!(!folds.hides(6));
        assert!((7..=10).all(|line| folds.hides(line)));
        assert!(!folds.hides(11));
        assert!(folds.toggle(6));
        assert!(!folds.hides(8));
    }

    #[test]
    fn only_a_stretch_starting_on_the_line_folds() {
        let mut folds = folds(vec![RangeInclusive::new(0, 3)]);
        assert!(!folds.toggle(2));
        assert!(!folds.hides(2));
    }

    #[test]
    fn stretches_learned_later_fold_as_well() {
        let ranges = FoldRanges::default();
        let mut folds = LineFolds::new(ranges.clone());
        assert!(!folds.toggle(0));
        ranges.borrow_mut().push(0..=2);
        assert!(folds.toggle(0));
        assert!(folds.hides(2));
    }
}
