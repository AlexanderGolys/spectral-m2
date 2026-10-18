//! Regions withheld from copied screen selections.
//!
//! Rendered frames mix content with decoration: borders, gutters, markers, and
//! padding all occupy cells that a reader never meant to copy. Elements report those
//! cells here in their own local coordinates; each containing surface translates them
//! once as it places the element, so a selection carries only the text.

use std::iter::FromIterator;
use std::ops::{Add, AddAssign, Range};

use ratatui::layout::{Offset, Position, Rect};

/// Rectangular regions omitted from copied UI selections.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exclusions(Vec<Rect>);

impl Exclusions {
    pub fn push(&mut self, exclusion: Rect) {
        self.0.push(exclusion);
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn contains(&self, position: Position) -> bool {
        self.0.iter().any(|area| area.contains(position))
    }

    /// Clip to source rows and make the first retained row local zero.
    pub fn project(&self, rows: Range<u16>) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusions_project_shift_and_test_points() {
        let exclusions =
            Exclusions::from([Rect::new(2, 3, 2, 3)]).project(4..6) + Offset::new(5, 7);

        assert!(exclusions.contains(Position::new(8, 8)));
        assert!(!exclusions.contains(Position::new(6, 8)));
        assert_eq!(exclusions, [Rect::new(7, 7, 2, 2)].into());
    }
}
