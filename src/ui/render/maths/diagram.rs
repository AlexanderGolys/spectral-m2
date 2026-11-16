//! Pieces on a grid joined by arrows, as a map of complexes is drawn: its two
//! complexes one above the other, each degree in a column of its own, and the
//! components running down between them, as many columns across as the map's
//! degree.

// Keep mathematical notation and interface symbols legible in their native spelling.
#![allow(clippy::non_ascii_literal)]
use unicode_width::UnicodeWidthChar;

use super::{Math, MathBlock};
use serde::Deserialize;
use crate::ui::block::TextSpan;
use crate::ui::present::SpanLine;
use crate::ui::theme::groups::HLGroup;

/// Columns kept between neighbouring columns that no arrow joins.
const COLUMN_GAP: usize = 2;
/// The shortest horizontal arrow, head included.
const MIN_ARROW: usize = 3;
/// The fewest rows an arrow between two rows runs down.
const MIN_BAND: usize = 3;

const LEFT_ARROW: &str = "←";
const RIGHT_ARROW: &str = "→";
const SHAFT: &str = "─";
const DOWN_SHAFT: &str = "│";
const DOWN_HEAD: &str = "↓";

/// A cell of a diagram's grid, sent as `[row, column]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(from = "(usize, usize)")]
pub struct Place {
    pub row: usize,
    pub column: usize,
}

/// An arrow between two pieces of a diagram, with what it is labelled by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arrow {
    pub from: Place,
    pub to: Place,
    pub label: Option<Math>,
}

/// How an arrow runs across the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Course {
    /// Along one row, between neighbouring columns.
    Across,
    /// From one row to the next, `shift` columns to the right; to the left when
    /// negative.
    Down { shift: isize },
}

impl From<(usize, usize)> for Place {
    fn from((row, column): (usize, usize)) -> Self {
        Self { row, column }
    }
}

impl Arrow {
    /// How the arrow runs, when it joins places a diagram's arrows may.
    pub fn course(&self) -> Option<Course> {
        let (from, to) = (self.from, self.to);
        let shift = to.column as isize - from.column as isize;
        match () {
            _ if from.row == to.row && shift.abs() == 1 => Some(Course::Across),
            _ if to.row == from.row + 1 => Some(Course::Down { shift }),
            _ => None,
        }
    }
}

/// Pieces placed on a grid of rows and columns, joined by arrows: along a row
/// between neighbouring columns, or down to the next row in any column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagram {
    pub pieces: Vec<(Place, Math)>,
    pub arrows: Vec<Arrow>,
    /// The same diagram for a layout in characters, which can only run an arrow
    /// straight down, when this one runs some across.
    pub straight: Option<Box<Diagram>>,
}

impl Diagram {
    pub fn rows(&self) -> usize {
        self.places().map(|place| place.row + 1).max().unwrap_or(0)
    }

    pub fn columns(&self) -> usize {
        self.places()
            .map(|place| place.column + 1)
            .max()
            .unwrap_or(0)
    }

    fn places(&self) -> impl Iterator<Item = Place> + '_ {
        self.pieces
            .iter()
            .map(|(place, _)| *place)
            .chain(self.arrows.iter().flat_map(|arrow| [arrow.from, arrow.to]))
    }

    /// The same diagram with every piece and label passed through `change`.
    pub fn map(self, change: impl Fn(Math) -> Math + Copy) -> Self {
        Self {
            pieces: self
                .pieces
                .into_iter()
                .map(|(place, piece)| (place, change(piece)))
                .collect(),
            arrows: self
                .arrows
                .into_iter()
                .map(|arrow| Arrow {
                    label: arrow.label.map(change),
                    ..arrow
                })
                .collect(),
            straight: self.straight.map(|straight| Box::new(straight.map(change))),
        }
    }

    /// The diagram in characters: each column as wide as its widest piece, the
    /// gaps between them wide enough for the arrows and labels they hold. Arrows
    /// between rows run straight down from their source, as characters can only
    /// draw them; [`Diagram::straight`] arranges a diagram for that.
    pub(super) fn layout(&self, lay: impl Fn(&Math) -> MathBlock) -> MathBlock {
        if let Some(straight) = &self.straight {
            return straight.layout(lay);
        }
        let pieces = self
            .pieces
            .iter()
            .map(|(place, piece)| (*place, lay(piece)))
            .collect::<Vec<_>>();
        let labels = self
            .arrows
            .iter()
            .map(|arrow| arrow.label.as_ref().map(&lay))
            .collect::<Vec<_>>();
        let (rows, columns) = (self.rows(), self.columns());
        let piece_at = |place: Place| {
            pieces
                .iter()
                .find(|(at, _)| *at == place)
                .map(|(_, block)| block)
        };
        let label_size = |index: usize| {
            labels[index]
                .as_ref()
                .map_or((0, 0), |label| (label.width(), label.height()))
        };

        let mut widths = vec![0; columns];
        let mut above = vec![0; rows];
        let mut below = vec![0; rows];
        for (place, block) in &pieces {
            widths[place.column] = widths[place.column].max(block.width());
            above[place.row] = above[place.row].max(block.baseline);
            below[place.row] = below[place.row].max(block.height() - block.baseline - 1);
        }
        let mut bands = vec![0; rows];
        for (index, arrow) in self.arrows.iter().enumerate() {
            let (_, height) = label_size(index);
            match arrow.course() {
                Some(Course::Across) => above[arrow.from.row] = above[arrow.from.row].max(height),
                Some(_) => {
                    bands[arrow.from.row] = bands[arrow.from.row].max(MIN_BAND.max(height + 2))
                }
                None => {}
            }
        }
        let mut gaps = vec![COLUMN_GAP; columns];
        for (index, arrow) in self.arrows.iter().enumerate() {
            let (width, _) = label_size(index);
            let left = arrow.from.column.min(arrow.to.column);
            match arrow.course() {
                Some(Course::Across) => gaps[left] = gaps[left].max(width.max(MIN_ARROW) + 2),
                Some(Course::Down { .. }) => {
                    // Straight down from the source, labelled on its right.
                    let column = arrow.from.column;
                    let line = widths[column].saturating_sub(1) / 2;
                    let reach = (line + 2 + width).saturating_sub(widths[column]);
                    gaps[column] = gaps[column].max(reach + 1);
                }
                None => {}
            }
        }

        let mut lefts = Vec::with_capacity(columns);
        let mut x = 0;
        for column in 0..columns {
            lefts.push(x);
            x += widths[column] + gaps[column];
        }
        let width = lefts.last().map_or(0, |left| left + widths[columns - 1]);
        let mut baselines = Vec::with_capacity(rows);
        let mut y = 0;
        for row in 0..rows {
            baselines.push(y + above[row]);
            y += above[row] + below[row] + 1 + bands[row];
        }
        let height = y - bands.last().copied().unwrap_or(0);
        let mut canvas = Canvas::new(width, height);

        // Where each piece was set, as its left column and top row.
        let corner = |place: Place, block: &MathBlock| {
            (
                lefts[place.column] + (widths[place.column] - block.width()) / 2,
                baselines[place.row] - block.baseline,
            )
        };
        for (place, block) in &pieces {
            let (x, y) = corner(*place, block);
            canvas.paint(block, x, y);
        }
        for (index, arrow) in self.arrows.iter().enumerate() {
            let label = labels[index].as_ref();
            let (label_width, label_height) = label_size(index);
            match arrow.course() {
                Some(Course::Across) => {
                    let row = baselines[arrow.from.row];
                    let left = arrow.from.column.min(arrow.to.column);
                    let start = piece_at(Place {
                        row: arrow.from.row,
                        column: left,
                    })
                    .map_or(lefts[left] + widths[left], |block| {
                        corner(
                            Place {
                                row: arrow.from.row,
                                column: left,
                            },
                            block,
                        )
                        .0 + block.width()
                    }) + 1;
                    let right = Place {
                        row: arrow.from.row,
                        column: left + 1,
                    };
                    let end = piece_at(right)
                        .map_or(lefts[left + 1], |block| corner(right, block).0)
                        .saturating_sub(1);
                    let length = end.saturating_sub(start).max(1);
                    let line = match arrow.to.column < arrow.from.column {
                        true => format!("{LEFT_ARROW}{}", SHAFT.repeat(length - 1)),
                        false => format!("{}{RIGHT_ARROW}", SHAFT.repeat(length - 1)),
                    };
                    canvas.text(&line, start, row, HLGroup::MapArrow);
                    if let Some(label) = label {
                        let x = start + length.saturating_sub(label_width) / 2;
                        canvas.paint(label, x, row - label_height);
                    }
                }
                Some(Course::Down { .. }) => {
                    let band = bands[arrow.from.row];
                    let top = baselines[arrow.from.row] + below[arrow.from.row] + 1;
                    let column = arrow.from.column;
                    let x = lefts[column] + widths[column].saturating_sub(1) / 2;
                    for step in 0..band {
                        let glyph = if step + 1 == band {
                            DOWN_HEAD
                        } else {
                            DOWN_SHAFT
                        };
                        canvas.text(glyph, x, top + step, HLGroup::MapArrow);
                    }
                    if let Some(label) = label {
                        canvas.paint(label, x + 2, top + (band - label_height) / 2);
                    }
                }
                None => {}
            }
        }
        canvas.into_block(height / 2)
    }
}

/// Character cells pieces are painted onto at any place, read back as rows of
/// spans; a wide character's second cell is kept empty.
struct Canvas {
    cells: Vec<Vec<Option<TextSpan>>>,
}

impl Canvas {
    fn new(width: usize, height: usize) -> Self {
        Self {
            cells: vec![vec![None; width]; height],
        }
    }

    fn put(&mut self, x: usize, y: usize, span: TextSpan) {
        if let Some(cell) = self.cells.get_mut(y).and_then(|row| row.get_mut(x)) {
            *cell = Some(span);
        }
    }

    fn spans(&mut self, spans: &[TextSpan], mut x: usize, y: usize) {
        for span in spans {
            for character in span.content.chars() {
                let columns = character.width().unwrap_or(0);
                if columns == 0 {
                    // A combining mark joins the character before it.
                    if let Some(Some(before)) = self
                        .cells
                        .get_mut(y)
                        .and_then(|row| row.get_mut(x.wrapping_sub(1)))
                    {
                        before.content.push(character);
                    }
                    continue;
                }
                self.put(
                    x,
                    y,
                    TextSpan {
                        content: character.to_string(),
                        ..span.clone()
                    },
                );
                for column in 1..columns {
                    self.put(
                        x + column,
                        y,
                        TextSpan {
                            content: String::new(),
                            ..span.clone()
                        },
                    );
                }
                x += columns;
            }
        }
    }

    fn paint(&mut self, block: &MathBlock, x: usize, y: usize) {
        for (offset, row) in block.rows.iter().enumerate() {
            self.spans(row, x, y + offset);
        }
    }

    fn text(&mut self, text: &str, x: usize, y: usize, kind: HLGroup) {
        self.spans(&[TextSpan::new(text, kind)], x, y);
    }

    fn into_block(self, baseline: usize) -> MathBlock {
        let rows = self
            .cells
            .into_iter()
            .map(|row| {
                let mut line: SpanLine = Vec::new();
                for cell in row {
                    let cell = cell.unwrap_or_else(|| TextSpan::new(" ", HLGroup::Canvas));
                    match line.last_mut() {
                        Some(last)
                            if last.kind == cell.kind
                                && last.mark == cell.mark
                                && last.target == cell.target =>
                        {
                            last.content.push_str(&cell.content)
                        }
                        _ => line.push(cell),
                    }
                }
                line
            })
            .collect::<Vec<_>>();
        MathBlock {
            baseline: baseline.min(rows.len().saturating_sub(1)),
            rows,
        }
    }
}
