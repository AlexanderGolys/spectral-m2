//! The cells of a rendered frame a selection reads.
//!
//! Rendered frames mix content with decoration: borders, gutters, markers, and
//! padding all occupy cells that a reader never meant to copy. Elements report the
//! cells holding their text here in their own local coordinates; each containing
//! surface translates them once as it places the element. Whatever nothing reports
//! is decoration, so a selection carries only the text.

use std::ops::{Add, AddAssign, Range};

use ratatui::layout::{Offset, Position, Rect};

/// What kind of text a cell holds. A selection reads the layer it starts on and
/// passes over the others, so a value and the type set beside it never mix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Text,
    Type,
}

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

/// Rectangular regions a selection reads, and the rows a copy reads as carrying on
/// the row above them. Where regions overlap, the one reported last decides.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inclusions {
    areas: Vec<(Rect, Option<Layer>)>,
    seams: Vec<(Rect, Seam)>,
}

impl Inclusions {
    pub fn push(&mut self, text: Rect) {
        self.areas.push((text, Some(Layer::Text)));
    }

    pub fn push_type(&mut self, annotation: Rect) {
        self.areas.push((annotation, Some(Layer::Type)));
    }

    /// Withdraw whatever was reported under an overlay drawn over `area`.
    pub fn cover(&mut self, area: Rect) {
        self.areas.push((area, None));
    }

    pub fn clear(&mut self) {
        self.areas.clear();
        self.seams.clear();
    }

    pub fn layer_at(&self, position: Position) -> Option<Layer> {
        self.areas
            .iter()
            .rev()
            .find(|(area, _)| area.contains(position))
            .and_then(|(_, layer)| *layer)
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

    /// The same regions with each row moved to where `shown` puts it, or dropped
    /// where it is not shown, as when rows are folded away.
    pub fn folded(&self, shown: impl Fn(u16) -> Option<u16>) -> Self {
        let rows = |area: Rect| {
            (area.y..area.bottom())
                .filter_map(|row| Some(Rect::new(area.x, shown(row)?, area.width, 1)))
                .collect::<Vec<_>>()
        };
        Self {
            areas: self
                .areas
                .iter()
                .flat_map(|&(area, layer)| rows(area).into_iter().map(move |row| (row, layer)))
                .collect(),
            seams: self
                .seams
                .iter()
                .flat_map(|&(area, seam)| rows(area).into_iter().map(move |row| (row, seam)))
                .collect(),
        }
    }

    /// Clip to source rows and make the first retained row local zero.
    pub fn project(&self, rows: Range<u16>) -> Self {
        Self {
            areas: self
                .areas
                .iter()
                .filter_map(|(area, layer)| Some((clipped(*area, &rows)?, *layer)))
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

impl Add<Offset> for Inclusions {
    type Output = Self;

    fn add(mut self, offset: Offset) -> Self {
        self += offset;
        self
    }
}

impl Add<Offset> for &Inclusions {
    type Output = Inclusions;

    fn add(self, offset: Offset) -> Inclusions {
        let mut moved = self.clone();
        moved += offset;
        moved
    }
}

impl AddAssign<Offset> for Inclusions {
    fn add_assign(&mut self, offset: Offset) {
        self.areas.iter_mut().for_each(|(area, _)| *area += offset);
        self.seams.iter_mut().for_each(|(area, _)| *area += offset);
    }
}

impl AddAssign for Inclusions {
    fn add_assign(&mut self, other: Self) {
        self.areas.extend(other.areas);
        self.seams.extend(other.seams);
    }
}

impl<const N: usize> From<[Rect; N]> for Inclusions {
    fn from(areas: [Rect; N]) -> Self {
        Self {
            areas: areas.map(|area| (area, Some(Layer::Text))).into(),
            seams: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inclusions_project_shift_and_test_points() {
        let inclusions =
            Inclusions::from([Rect::new(2, 3, 2, 3)]).project(4..6) + Offset::new(5, 7);

        assert_eq!(inclusions.layer_at(Position::new(8, 8)), Some(Layer::Text));
        assert_eq!(inclusions.layer_at(Position::new(6, 8)), None);
        assert_eq!(inclusions, [Rect::new(7, 7, 2, 2)].into());
    }

    #[test]
    fn the_region_reported_last_decides_a_cell() {
        let mut inclusions = Inclusions::from([Rect::new(0, 0, 6, 1)]);
        inclusions.push_type(Rect::new(4, 0, 2, 1));
        assert_eq!(inclusions.layer_at(Position::new(3, 0)), Some(Layer::Text));
        assert_eq!(inclusions.layer_at(Position::new(4, 0)), Some(Layer::Type));

        inclusions.cover(Rect::new(3, 0, 2, 1));
        assert_eq!(inclusions.layer_at(Position::new(3, 0)), None);
        assert_eq!(inclusions.layer_at(Position::new(5, 0)), Some(Layer::Type));
    }

    /// The seam of each row of `rows` read against `source`.
    fn seams(rows: &[&str], source: &str) -> Vec<Option<Seam>> {
        let mut inclusions = Inclusions::default();
        inclusions.mark_wrapped_rows(0, 10, rows.iter().copied(), source);
        (0..rows.len() as u16)
            .map(|row| inclusions.seam_at(Position::new(0, row)))
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
        let mut inclusions = Inclusions::default();
        inclusions.mark_wrapped_rows(2, 10, ["ab", "cd"], "ab cd");
        let placed = inclusions.project(1..5) + Offset::new(3, 4);
        assert_eq!(placed.seam_at(Position::new(3, 6)), Some(Seam::Space));
        assert_eq!(placed.seam_at(Position::new(3, 5)), None);
    }
}
