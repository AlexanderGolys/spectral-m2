//! Regions withheld from copied screen selections.
//!
//! Rendered frames mix content with decoration: borders, gutters, markers, and
//! padding all occupy cells that a reader never meant to copy. Elements report those
//! cells here in their own local coordinates; each containing surface translates them
//! once as it places the element, so a selection carries only the text.

use std::iter::FromIterator;
use std::ops::{Add, AddAssign, Range};

use ratatui::layout::{Offset, Position, Rect};

/// How a wrapped row joins the row it continues when copied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seam {
    /// The break fell between two characters.
    Direct,
    /// The break took the place of a space.
    Space,
}

impl Seam {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Direct => "",
            Self::Space => " ",
        }
    }
}

/// `area` cut to `rows` and made local to the first of them.
fn clipped(area: Rect, rows: &Range<u16>) -> Option<Rect> {
    let top = area.y.max(rows.start);
    let bottom = area.bottom().min(rows.end);
    (top < bottom).then(|| Rect::new(area.x, top - rows.start, area.width, bottom - top))
}

/// Rectangular regions omitted from copied UI selections, and the rows a copy
/// reads as carrying on the row above them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exclusions {
    areas: Vec<Rect>,
    seams: Vec<(Rect, Seam)>,
}

impl Exclusions {
    pub fn push(&mut self, exclusion: Rect) {
        self.areas.push(exclusion);
    }

    pub fn clear(&mut self) {
        self.areas.clear();
        self.seams.clear();
    }

    pub fn contains(&self, position: Position) -> bool {
        self.areas.iter().any(|area| area.contains(position))
    }

    /// How the row at `position` joins the one above it, when it only wraps it.
    pub fn seam_at(&self, position: Position) -> Option<Seam> {
        self.seams
            .iter()
            .find(|(area, _)| area.contains(position))
            .map(|(_, seam)| *seam)
    }

    /// Mark which of `rows`, drawn from local row `top` and `width` wide, only wrap
    /// the one above them, by reading them against the `source` they show. Rows
    /// that do not read back as `source` -- folded, drawn as pictures -- stay lines
    /// of their own, so a copy never joins what was not one line.
    pub fn mark_wrapped_rows<'a>(
        &mut self,
        top: u16,
        width: u16,
        rows: impl IntoIterator<Item = &'a str>,
        source: &str,
    ) {
        let mut rest = source;
        let mut seams = Vec::new();
        let mut after_blank = true;
        for (index, row) in rows.into_iter().enumerate() {
            let row = row.trim();
            let text = rest.trim_start();
            let gap = &rest[..rest.len() - text.len()];
            let Some(after) = text.strip_prefix(row) else {
                return;
            };
            let seam = match gap {
                "" => Some(Seam::Direct),
                " " => Some(Seam::Space),
                _ => None,
            };
            if let Some(seam) = seam.filter(|_| !after_blank && !row.is_empty()) {
                let index = u16::try_from(index).unwrap_or(u16::MAX);
                seams.push((Rect::new(0, top.saturating_add(index), width, 1), seam));
            }
            after_blank = row.is_empty();
            rest = after;
        }
        if rest.trim().is_empty() {
            self.seams.extend(seams);
        }
    }

    /// Clip to source rows and make the first retained row local zero.
    pub fn project(&self, rows: Range<u16>) -> Self {
        Self {
            areas: self
                .areas
                .iter()
                .filter_map(|area| clipped(*area, &rows))
                .collect(),
            seams: self
                .seams
                .iter()
                .filter_map(|(area, seam)| Some((clipped(*area, &rows)?, *seam)))
                .collect(),
        }
    }

    /// Take on everything `other` holds.
    pub fn merge(&mut self, other: Self) {
        self.areas.extend(other.areas);
        self.seams.extend(other.seams);
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
        let mut moved = self.clone();
        moved += offset;
        moved
    }
}

impl AddAssign<Offset> for Exclusions {
    fn add_assign(&mut self, offset: Offset) {
        self.areas.iter_mut().for_each(|area| *area += offset);
        self.seams.iter_mut().for_each(|(area, _)| *area += offset);
    }
}

impl Extend<Rect> for Exclusions {
    fn extend<T: IntoIterator<Item = Rect>>(&mut self, iter: T) {
        self.areas.extend(iter);
    }
}

impl FromIterator<Rect> for Exclusions {
    fn from_iter<T: IntoIterator<Item = Rect>>(iter: T) -> Self {
        Self {
            areas: iter.into_iter().collect(),
            seams: Vec::new(),
        }
    }
}

impl<const N: usize> From<[Rect; N]> for Exclusions {
    fn from(areas: [Rect; N]) -> Self {
        areas.into_iter().collect()
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

    /// The seam of each row of `rows` read against `source`.
    fn seams(rows: &[&str], source: &str) -> Vec<Option<Seam>> {
        let mut exclusions = Exclusions::default();
        exclusions.mark_wrapped_rows(0, 10, rows.iter().copied(), source);
        (0..rows.len() as u16)
            .map(|row| exclusions.seam_at(Position::new(0, row)))
            .collect()
    }

    #[test]
    fn a_wrapped_row_is_told_from_a_line_of_its_own() {
        assert_eq!(
            seams(&["ab cd", "ef", "gh", "", "ij"], "ab cd ef\ngh\n\nij"),
            [None, Some(Seam::Space), None, None, None]
        );
        assert_eq!(seams(&["abc", "def"], "abcdef"), [None, Some(Seam::Direct)]);
    }

    #[test]
    fn rows_that_do_not_read_as_their_source_are_not_joined() {
        assert_eq!(seams(&["ab", "cd"], "ab xy"), [None, None]);
        assert_eq!(seams(&["ab", "cd"], "ab cd ef"), [None, None]);
        assert_eq!(seams(&["ab", "cd"], "ab  cd"), [None, None]);
    }

    #[test]
    fn seams_move_and_clip_with_their_block() {
        let mut exclusions = Exclusions::default();
        exclusions.mark_wrapped_rows(2, 10, ["ab", "cd"], "ab cd");
        let placed = exclusions.project(1..5) + Offset::new(3, 4);
        assert_eq!(placed.seam_at(Position::new(3, 6)), Some(Seam::Space));
        assert_eq!(placed.seam_at(Position::new(3, 5)), None);
    }
}
