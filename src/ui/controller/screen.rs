//! Mouse selection over the drawn screen: what each pane drew, which of its
//! cells may be copied, and the text a drag or a double click selects. A
//! selection belongs to the pane it starts in and never leaves it, but reaches
//! past what the pane shows: dragging against its edge scrolls it, and every row
//! drawn while the selection lasts stays captured. It stays marked until the next
//! press, keystroke or change to the transcript, as the input's own does.

use std::collections::{BTreeMap, HashSet};
use std::ops::RangeInclusive;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::Frame;

use super::{Editor, Reaction};
use crate::ui::graphics::{
    self,
    kitty::{ImageId, Piece},
};
use crate::ui::render::selection::{Layer, Seam};
use crate::ui::text_object::{Bracket, CharClass};
use crate::ui::theme::groups::scheme;

/// A selectable cell position: a content row and a screen column, so a
/// selection stays on its text while the pane scrolls. Positions order as text
/// reads.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct ScreenPosition {
    row: i64,
    column: u16,
}

impl ScreenPosition {
    fn new(column: u16, row: i64) -> Self {
        Self { row, column }
    }
}

/// A mouse selection spanning captured cells of one pane.
pub struct ScreenSelection {
    /// The area of the pane the selection started in.
    area: Rect,
    extent: Extent,
    reach: Reach,
}

/// How far a selection has got.
#[derive(Clone, Copy, Debug)]
enum Extent {
    /// The button went down here and has not moved: nothing is selected yet.
    Pressed(ScreenPosition),
    Dragged {
        anchor: ScreenPosition,
        focus: ScreenPosition,
    },
    /// What a double click picked out.
    Picked(Region),
}

/// The cells a selection covers, between two positions in reading order.
#[derive(Clone, Copy, Debug)]
enum Region {
    /// Everything read from `start` through `end`, wrapping at the pane's edges.
    Flow {
        start: ScreenPosition,
        end: ScreenPosition,
    },
    /// The rectangle with these corners: a picture, or a block between tall fences.
    Block {
        start: ScreenPosition,
        end: ScreenPosition,
    },
}

impl Region {
    fn rows(self) -> RangeInclusive<i64> {
        let (Self::Flow { start, end } | Self::Block { start, end }) = self;
        start.row..=end.row
    }

    /// The columns of `area` the region covers on `row`.
    fn columns(self, row: i64, area: Rect) -> RangeInclusive<u16> {
        match self {
            Self::Flow { start, end } => {
                let from = match row == start.row {
                    true => start.column,
                    false => area.x,
                };
                let through = match row == end.row {
                    true => end.column,
                    false => area.right().saturating_sub(1),
                };
                from..=through
            }
            Self::Block { start, end } => start.column..=end.column,
        }
    }
}

/// Which captured cells a region reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Reach {
    /// Only the text of one layer, as a selection starting on it reads it.
    Layer(Layer),
    /// Every cell drawn, as a double click on decoration reads it.
    Drawn,
    /// Every cell drawn and every row apart, as the screen shows them.
    Verbatim,
}

impl Reach {
    /// Whether a row holding `cell` takes part in the copy at all.
    fn admits(self, cell: &ScreenCell) -> bool {
        match self {
            Self::Layer(layer) => cell.layer == Some(layer),
            Self::Drawn | Self::Verbatim => true,
        }
    }

    /// Whether `cell` contributes its text.
    fn reads(self, cell: &ScreenCell) -> bool {
        self.admits(cell) && (cell.selectable || !matches!(self, Self::Layer(_)))
    }
}

/// One place a picture is shown: the picture and the content cell holding its first
/// piece, so the same formula shown twice copies twice while the pieces of one
/// showing copy once.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Showing {
    image: ImageId,
    column: i32,
    row: i64,
}

/// Captured terminal content and whether it may be copied.
pub struct ScreenCell {
    /// The text the cell copies as: what it shows, or for a piece of a picture
    /// the M2 code of the whole picture.
    symbol: String,
    /// The showing of a picture the cell is a piece of, copied once however many
    /// cells it covers.
    picture: Option<Showing>,
    /// Whether the cell holds text of its layer, rather than the blanks after it.
    selectable: bool,
    /// The layer the cell belongs to, which a row of blanks can as well, to copy as
    /// an empty line; none for decoration such as a frame.
    layer: Option<Layer>,
    /// How the cell's row joins the row above when it only wraps it.
    seam: Option<Seam>,
}

impl ScreenCell {
    /// The one character the cell shows as text, if that is what it shows.
    fn character(&self) -> Option<char> {
        let mut characters = self.symbol.chars();
        let character = characters.next()?;
        (self.picture.is_none() && characters.next().is_none()).then_some(character)
    }

    /// Whether the cell shows a piece of a fence grown taller than a row.
    fn is_fence_piece(&self) -> bool {
        self.picture.is_none() && Bracket::is_fence_piece(&self.symbol)
    }
}

/// What one pane drew, kept to select and copy from: its last frame, and while a
/// selection lasts every row drawn since it began.
pub struct ScreenCapture {
    area: Rect,
    /// The content row shown on the area's first line.
    top: i64,
    rows: BTreeMap<i64, Vec<ScreenCell>>,
}

impl ScreenCapture {
    /// The position under `event`, held inside this pane's area.
    fn position(&self, event: MouseEvent) -> ScreenPosition {
        let area = self.area;
        let column = event.column.clamp(area.x, area.right().saturating_sub(1));
        let line = event.row.clamp(area.y, area.bottom().saturating_sub(1));
        ScreenPosition::new(column, self.top + i64::from(line - area.y))
    }

    /// The screen line showing `content_row`, if it is on screen.
    fn line(&self, content_row: i64) -> Option<u16> {
        let offset = u16::try_from(content_row - self.top).ok()?;
        (offset < self.area.height).then(|| self.area.y + offset)
    }

    fn cell(&self, at: ScreenPosition) -> Option<&ScreenCell> {
        self.rows
            .get(&at.row)?
            .get(usize::from(at.column.checked_sub(self.area.x)?))
    }

    /// The rows the pane scrolls toward when a drag reaches `line`: older ones at
    /// its first line, newer ones at its last.
    fn edge_scroll(&self, line: u16) -> Option<i8> {
        match line {
            _ if line <= self.area.y => Some(1),
            _ if line >= self.area.bottom().saturating_sub(1) => Some(-1),
            _ => None,
        }
    }

    /// Whether a picture shown on `upper` carries on to the row below, so the two
    /// read as one line.
    fn joined_below(&self, upper: i64) -> bool {
        let (Some(above), Some(below)) = (self.rows.get(&upper), self.rows.get(&(upper + 1)))
        else {
            return false;
        };
        let shown: HashSet<Showing> = above.iter().filter_map(|cell| cell.picture).collect();
        below
            .iter()
            .filter_map(|cell| cell.picture)
            .any(|showing| shown.contains(&showing))
    }

    /// The text `region` covers. Rows a picture spans read as one line, column by
    /// column, a tall fence as the bracket it draws; every picture copies once.
    fn text(&self, region: Region, reach: Reach) -> Option<String> {
        let mut copied = HashSet::new();
        let mut lines: Vec<String> = Vec::new();
        let mut band: Vec<Vec<(u16, &ScreenCell)>> = Vec::new();
        for row in region.rows() {
            let Some(cells) = self.rows.get(&row) else {
                continue;
            };
            let selected: Vec<(u16, &ScreenCell)> = region
                .columns(row, self.area)
                .filter_map(|x| Some((x, cells.get(usize::from(x.checked_sub(self.area.x)?))?)))
                .filter(|(_, cell)| reach.admits(cell))
                .collect();
            if !selected.is_empty() {
                band.push(selected);
            }
            if !band.is_empty() && (!self.joined_below(row) || row == *region.rows().end()) {
                let seam = band[0].first().and_then(|(_, cell)| cell.seam);
                match (
                    Self::band_line(&band, reach, &mut copied),
                    seam,
                    lines.last_mut(),
                ) {
                    (Some(line), Some(seam), Some(above)) if reach != Reach::Verbatim => {
                        above.push_str(seam.text());
                        above.push_str(line.trim_start());
                    }
                    (line, ..) => lines.extend(line),
                }
                band.clear();
            }
        }
        let text = lines.join("\n");
        let text = text.trim_matches('\n');
        (!text.is_empty()).then(|| text.to_string())
    }

    /// One line read from rows joined by the pictures they share; none when they
    /// hold only pictures already copied.
    fn band_line(
        band: &[Vec<(u16, &ScreenCell)>],
        reach: Reach,
        copied: &mut HashSet<Showing>,
    ) -> Option<String> {
        let mut columns: BTreeMap<u16, Vec<&ScreenCell>> = BTreeMap::new();
        for row in band {
            for &(x, cell) in row.iter().filter(|(_, cell)| reach.reads(cell)) {
                columns.entry(x).or_default().push(cell);
            }
        }
        let tall = band.len() > 1;
        let mut line = String::new();
        let mut pictured = false;
        for cells in columns.values() {
            let mut read = String::new();
            let mut covered = false;
            let mut above_fence = false;
            for cell in cells {
                if let Some(showing) = cell.picture {
                    pictured = true;
                    covered = true;
                    if copied.insert(showing) {
                        read.push_str(&cell.symbol);
                    }
                } else if tall && cell.is_fence_piece() {
                    covered = true;
                    if !above_fence {
                        read.extend(Bracket::of_fence_top(&cell.symbol).map(Bracket::character));
                    }
                } else if !tall || !cell.symbol.trim().is_empty() {
                    read.push_str(&cell.symbol);
                }
                above_fence = cell.is_fence_piece();
            }
            if tall && read.is_empty() && !covered {
                read.push(' ');
            }
            line.push_str(&read);
        }
        let line = line.trim_end();
        (!(pictured && line.trim().is_empty())).then(|| line.to_string())
    }

    /// What a double click at `at` picks out, and how to read it: a whole picture,
    /// the block from a bracket to its partner, or a word.
    fn pick(&self, at: ScreenPosition) -> Option<(Region, Reach)> {
        let cell = self.cell(at)?;
        let reach = cell.layer.map_or(Reach::Drawn, Reach::Layer);
        if let Some(showing) = cell.picture {
            return Some((self.showing_extent(showing), Reach::Drawn));
        }
        if cell.is_fence_piece() {
            return self.fenced_block(at).map(|region| (region, reach));
        }
        let character = cell.character()?;
        if let Some(bracket) = Bracket::of(character) {
            return self
                .bracketed_flow(at, bracket)
                .map(|region| (region, reach));
        }
        (CharClass::of(character) == CharClass::Word).then(|| (self.word(at), reach))
    }

    /// Whether `at` is on text a selection can read, rather than on the decoration
    /// or the blanks around it.
    fn holds_text(&self, at: ScreenPosition) -> bool {
        self.cell(at)
            .is_some_and(|cell| cell.layer.is_some() && cell.selectable)
    }

    /// The rows of text around `at`, up to the rows holding none either side, and
    /// how to read them.
    fn paragraph(&self, at: ScreenPosition) -> Option<(Region, Reach)> {
        let layer = self.cell(at)?.layer?;
        let holds_text = |row: i64| {
            self.rows.get(&row).is_some_and(|cells| {
                cells
                    .iter()
                    .any(|cell| cell.layer == Some(layer) && cell.selectable)
            })
        };
        if !holds_text(at.row) {
            return None;
        }
        let first = (i64::MIN..at.row)
            .rev()
            .take_while(|&row| holds_text(row))
            .last()
            .unwrap_or(at.row);
        let last = (at.row + 1..)
            .take_while(|&row| holds_text(row))
            .last()
            .unwrap_or(at.row);
        let region = Region::Flow {
            start: ScreenPosition::new(self.area.x, first),
            end: ScreenPosition::new(self.area.right().saturating_sub(1), last),
        };
        Some((region, Reach::Layer(layer)))
    }

    /// The cells a picture's showing covers.
    fn showing_extent(&self, showing: Showing) -> Region {
        let covered = self.rows.iter().flat_map(|(&row, cells)| {
            cells
                .iter()
                .zip(self.area.x..)
                .filter(move |(cell, _)| cell.picture == Some(showing))
                .map(move |(_, column)| ScreenPosition::new(column, row))
        });
        let (start, end) = covered.fold(
            (
                ScreenPosition::new(u16::MAX, i64::MAX),
                ScreenPosition::new(0, i64::MIN),
            ),
            |(start, end), at| {
                (
                    ScreenPosition::new(start.column.min(at.column), start.row.min(at.row)),
                    ScreenPosition::new(end.column.max(at.column), end.row.max(at.row)),
                )
            },
        );
        Region::Block { start, end }
    }

    /// The run of word characters on `at`'s row around it.
    fn word(&self, at: ScreenPosition) -> Region {
        let is_word = |column: u16| {
            self.cell(ScreenPosition::new(column, at.row))
                .and_then(ScreenCell::character)
                .is_some_and(|character| CharClass::of(character) == CharClass::Word)
        };
        let first = (self.area.x..at.column)
            .rev()
            .take_while(|&column| is_word(column))
            .last()
            .unwrap_or(at.column);
        let last = (at.column + 1..self.area.right())
            .take_while(|&column| is_word(column))
            .last()
            .unwrap_or(at.column);
        Region::Flow {
            start: ScreenPosition::new(first, at.row),
            end: ScreenPosition::new(last, at.row),
        }
    }

    /// From a one-row bracket at `at` to its partner, in reading order.
    fn bracketed_flow(&self, at: ScreenPosition, bracket: Bracket) -> Option<Region> {
        let layer = self.cell(at)?.layer;
        let characters: Vec<(ScreenPosition, char)> = self
            .rows
            .iter()
            .flat_map(|(&row, cells)| {
                cells
                    .iter()
                    .zip(self.area.x..)
                    .filter(|(cell, _)| cell.layer == layer)
                    .filter_map(move |(cell, column)| {
                        Some((ScreenPosition::new(column, row), cell.character()?))
                    })
            })
            .collect();
        let index = characters
            .iter()
            .position(|&(position, _)| position == at)?;
        let partner = match bracket.opening {
            true => bracket.partner(characters[index + 1..].iter().copied()),
            false => bracket.partner(characters[..index].iter().rev().copied()),
        }?;
        Some(Region::Flow {
            start: at.min(partner),
            end: at.max(partner),
        })
    }

    /// From the tall fence a piece at `at` belongs to, to its partner of the same
    /// height: the rows the fence spans, across the columns between them.
    fn fenced_block(&self, at: ScreenPosition) -> Option<Region> {
        let is_piece = |column: u16, row: i64| {
            self.cell(ScreenPosition::new(column, row))
                .is_some_and(ScreenCell::is_fence_piece)
        };
        let top = (i64::MIN..=at.row)
            .rev()
            .take_while(|&row| is_piece(at.column, row))
            .last()?;
        let bottom = (at.row..)
            .take_while(|&row| is_piece(at.column, row))
            .last()?;
        let fence_top = |column: u16| {
            (is_piece(column, top) && !is_piece(column, top - 1))
                .then(|| self.cell(ScreenPosition::new(column, top)))
                .flatten()
                .and_then(|cell| Bracket::of_fence_top(&cell.symbol))
        };
        let bracket = fence_top(at.column)?;
        let tops = |columns: Vec<u16>| {
            columns
                .into_iter()
                .filter_map(move |column| Some((column, fence_top(column)?.character())))
        };
        let partner = match bracket.opening {
            true => bracket.partner(tops((at.column + 1..self.area.right()).collect())),
            false => bracket.partner(tops((self.area.x..at.column).rev().collect())),
        }?;
        Some(Region::Block {
            start: ScreenPosition::new(at.column.min(partner), top),
            end: ScreenPosition::new(at.column.max(partner), bottom),
        })
    }
}

impl Editor {
    pub fn handle_screen_mouse(&mut self, event: MouseEvent) -> Option<Reaction> {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) if !self.input.contains(event) => {
                self.input.field_mut().clear_selection();
                self.screen_selection = None;
                let raw = self.settings.selection.raw;
                let pointer = Position::new(event.column, event.row);
                let capture = self
                    .captures
                    .iter()
                    .find(|capture| capture.area.contains(pointer))?;
                let position = capture.position(event);
                let layer = capture.cell(position).and_then(|cell| cell.layer);
                self.screen_selection = Some(ScreenSelection {
                    area: capture.area,
                    extent: Extent::Pressed(position),
                    reach: match raw {
                        true => Reach::Verbatim,
                        false => Reach::Layer(layer.unwrap_or(Layer::Text)),
                    },
                });
                self.screen_selecting = true;
            }
            MouseEventKind::Drag(MouseButton::Left) if self.screen_selecting => {
                let selection = self.screen_selection.as_mut()?;
                let capture = self
                    .captures
                    .iter()
                    .find(|capture| capture.area == selection.area)?;
                selection.drag_to(capture.position(event));
                if let Some(rows) = capture.edge_scroll(event.row) {
                    return Some(Reaction::ScrollView(rows));
                }
            }
            MouseEventKind::Up(MouseButton::Left) if self.screen_selecting => {
                if let Some(selection) = self.screen_selection.as_mut() {
                    if let Some(capture) = self
                        .captures
                        .iter()
                        .find(|capture| capture.area == selection.area)
                    {
                        selection.release_at(capture.position(event));
                    }
                }
                let text = self.selected_screen_text();
                self.screen_selecting = false;
                match text {
                    Some(text) => return Some(Reaction::Copy(text)),
                    None => self.screen_selection = None,
                }
            }
            _ => return None,
        }
        Some(Reaction::Continue)
    }

    /// Whether a click is on the decoration or blanks of the pane under it, rather
    /// than on its text.
    pub fn outside_text(&self, event: MouseEvent) -> bool {
        let pointer = Position::new(event.column, event.row);
        self.captures
            .iter()
            .find(|capture| capture.area.contains(pointer))
            .is_some_and(|capture| !capture.holds_text(capture.position(event)))
    }

    /// Select and copy what the `clicks`th click of a run picks out of the pane
    /// under it: a word, a bracketed block or a picture, typed annotations
    /// included, for the second, and the paragraph for the third.
    pub fn pick_screen_text(&mut self, event: MouseEvent, clicks: u8) -> Option<Reaction> {
        let raw = self.settings.selection.raw;
        let pointer = Position::new(event.column, event.row);
        let capture = self
            .captures
            .iter()
            .find(|capture| capture.area.contains(pointer))?;
        let area = capture.area;
        let at = capture.position(event);
        let (region, reach) = match clicks {
            2 => capture.pick(at)?,
            _ => capture.paragraph(at)?,
        };
        let reach = if raw { Reach::Verbatim } else { reach };
        let text = capture.text(region, reach)?;
        self.input.field_mut().clear_selection();
        self.screen_selecting = false;
        self.screen_selection = Some(ScreenSelection {
            area,
            extent: Extent::Picked(region),
            reach,
        });
        Some(Reaction::Copy(text))
    }

    /// The capture of the pane `selection` belongs to.
    fn capture_of(&self, selection: &ScreenSelection) -> Option<&ScreenCapture> {
        self.captures
            .iter()
            .find(|capture| capture.area == selection.area)
    }

    pub fn selected_screen_text(&self) -> Option<String> {
        let selection = self.screen_selection.as_ref()?;
        let capture = self.capture_of(selection)?;
        capture.text(selection.region()?, selection.reach)
    }

    /// Keep what the pane just drawn into the feed area shows, replacing whatever
    /// was captured where it now lies; a selection in that pane keeps the rows it
    /// captured before as well.
    pub fn capture_screen(&mut self, frame: &mut Frame) {
        let area = self.feed_area;
        let top = self.feed_top;
        let buffer = frame.buffer_mut();
        let rows = (area.y..area.bottom())
            .map(|y| {
                let row = top + i64::from(y - area.y);
                let pictures: Vec<Option<Showing>> = (area.x..area.right())
                    .map(|x| {
                        let piece = Piece::placed(buffer[(x, y)].symbol(), buffer[(x, y)].fg)?;
                        Some(Showing {
                            image: piece.image,
                            column: i32::from(x) - i32::from(piece.column),
                            row: row - i64::from(piece.row),
                        })
                    })
                    .collect();
                let symbols: Vec<String> = (area.x..area.right())
                    .zip(&pictures)
                    .map(|(x, picture)| {
                        picture
                            .and_then(|showing| graphics::source(showing.image))
                            .unwrap_or_else(|| buffer[(x, y)].symbol().to_string())
                    })
                    .collect();
                let layers: Vec<Option<Layer>> = (area.x..area.right())
                    .map(|x| self.selection_inclusions.layer_at(Position::new(x, y)))
                    .collect();
                let selectable = selectable_columns(&symbols, &layers);
                let seams: Vec<Option<Seam>> = (area.x..area.right())
                    .map(|x| self.selection_inclusions.seam_at(Position::new(x, y)))
                    .collect();
                let cells = symbols
                    .into_iter()
                    .zip(pictures)
                    .zip(selectable)
                    .zip(layers)
                    .zip(seams)
                    .map(
                        |((((symbol, picture), selectable), layer), seam)| ScreenCell {
                            symbol,
                            picture,
                            selectable,
                            layer,
                            seam,
                        },
                    )
                    .collect();
                (row, cells)
            })
            .collect::<BTreeMap<_, _>>();
        // The pane was laid out afresh, so the rows the selection names are gone.
        if self
            .screen_selection
            .as_ref()
            .is_some_and(|selection| selection.area != area && selection.area.intersects(area))
        {
            self.screen_selection = None;
            self.screen_selecting = false;
        }
        let selecting = self
            .screen_selection
            .as_ref()
            .is_some_and(|selection| selection.area == area);
        if let Some(kept) = self
            .captures
            .iter_mut()
            .find(|capture| selecting && capture.area == area)
        {
            kept.top = top;
            kept.rows.extend(rows);
            return;
        }
        self.captures
            .retain(|capture| !capture.area.intersects(area));
        self.captures.push(ScreenCapture { area, top, rows });
    }

    /// Paint the selection over the pane just drawn, if the selection is its own.
    pub fn paint_screen_selection(&self, frame: &mut Frame) {
        let Some(selection) = self
            .screen_selection
            .as_ref()
            .filter(|selection| selection.area == self.feed_area)
        else {
            return;
        };
        let (Some(capture), Some(region)) = (self.capture_of(selection), selection.region()) else {
            return;
        };
        let buffer = frame.buffer_mut();
        for content_row in region.rows() {
            let Some(y) = capture.line(content_row) else {
                continue;
            };
            for x in region.columns(content_row, capture.area) {
                let at = ScreenPosition::new(x, content_row);
                if capture
                    .cell(at)
                    .is_some_and(|cell| selection.reach.reads(cell))
                {
                    buffer[(x, y)].set_style(scheme().selection());
                }
            }
        }
    }
}

impl ScreenSelection {
    /// The cells selected so far; none before the pointer moves.
    fn region(&self) -> Option<Region> {
        match self.extent {
            Extent::Pressed(_) => None,
            Extent::Dragged { anchor, focus } => Some(Region::Flow {
                start: anchor.min(focus),
                end: anchor.max(focus),
            }),
            Extent::Picked(region) => Some(region),
        }
    }

    fn drag_to(&mut self, focus: ScreenPosition) {
        if let Extent::Pressed(anchor) | Extent::Dragged { anchor, .. } = self.extent {
            self.extent = Extent::Dragged { anchor, focus };
        }
    }

    /// Finish a drag at `focus`; a press that never moved stays one.
    fn release_at(&mut self, focus: ScreenPosition) {
        if let Extent::Dragged { anchor, .. } = self.extent {
            self.extent = Extent::Dragged { anchor, focus };
        }
    }
}

/// Which columns of a row hold text: each run of one layer up to its last glyph, so
/// the blanks after it copy as nothing.
fn selectable_columns(symbols: &[String], layers: &[Option<Layer>]) -> Vec<bool> {
    let mut selectable = vec![false; symbols.len()];
    let mut start = 0;
    for run in layers.chunk_by(|left, right| left == right) {
        let end = start + run.len();
        if run[0].is_some() {
            let last_glyph = symbols[start..end]
                .iter()
                .rposition(|symbol| !symbol.trim().is_empty());
            if let Some(last_glyph) = last_glyph {
                selectable[start..=start + last_glyph].fill(true);
            }
        }
        start = end;
    }
    selectable
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyModifiers;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;
    use crate::backend::M2Value;
    use crate::code::highlight::SyntaxHighlighter;
    use crate::ui::action::Action;
    use crate::ui::block::Block;
    use crate::ui::cell::OutputBlock;
    use crate::ui::controller::EditorUpdate;
    use crate::ui::render::maths::SHORT_BRACES;
    use crate::ui::settings::selection::CopyFormat;

    /// A pane at `area` that drew `text` on every line, every cell selectable.
    fn pane(area: Rect, text: &str) -> ScreenCapture {
        let line = || {
            text.chars()
                .cycle()
                .take(usize::from(area.width))
                .map(|character| ScreenCell {
                    symbol: character.to_string(),
                    picture: None,
                    selectable: true,
                    layer: Some(Layer::Text),
                    seam: None,
                })
                .collect()
        };
        ScreenCapture {
            area,
            top: 0,
            rows: (0..i64::from(area.height))
                .map(|row| (row, line()))
                .collect(),
        }
    }

    #[test]
    fn a_wrapped_row_copies_as_part_of_the_line_it_continues() {
        let area = Rect::new(0, 0, 2, 3);
        let mut capture = pane(area, "ab");
        for cell in capture.rows.get_mut(&1).unwrap() {
            cell.seam = Some(Seam::Space);
        }
        for cell in capture.rows.get_mut(&2).unwrap() {
            cell.seam = Some(Seam::Direct);
        }
        let region = Region::Flow {
            start: ScreenPosition::new(0, 0),
            end: ScreenPosition::new(1, 2),
        };
        assert_eq!(
            capture.text(region, Reach::Layer(Layer::Text)).as_deref(),
            Some("ab abab")
        );
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// A pane drawing `lines` from its first column: blank cells copy as
    /// nothing, and a `None` line is a frame no selection copies.
    fn printed(area: Rect, lines: &[Option<&str>]) -> ScreenCapture {
        let row = |line: Option<&str>| {
            (0..area.width)
                .map(|column| {
                    let symbol = line
                        .and_then(|text| text.chars().nth(usize::from(column)))
                        .unwrap_or(' ');
                    ScreenCell {
                        symbol: symbol.to_string(),
                        picture: None,
                        selectable: symbol != ' ',
                        layer: line.map(|_| Layer::Text),
                        seam: None,
                    }
                })
                .collect()
        };
        ScreenCapture {
            area,
            top: 0,
            rows: (0..).zip(lines.iter().map(|&line| row(line))).collect(),
        }
    }

    #[test]
    fn blank_lines_inside_a_selection_are_copied_and_frames_are_not() {
        let area = Rect::new(0, 0, 4, 6);
        let lines = [Some(""), Some("1"), Some(""), None, Some("1"), Some("")];
        let mut editor = Editor::new(String::new());
        editor.captures = vec![printed(area, &lines)];

        let left = MouseButton::Left;
        editor.handle_screen_mouse(mouse(MouseEventKind::Down(left), 0, 0));
        editor.handle_screen_mouse(mouse(MouseEventKind::Drag(left), 3, 5));
        let copied = editor.handle_screen_mouse(mouse(MouseEventKind::Up(left), 3, 5));

        let Some(Reaction::Copy(text)) = copied else {
            panic!("a drag copies its text");
        };
        assert_eq!(text, "1\n\n1");
    }

    #[test]
    fn each_showing_of_a_picture_copies_its_code_once_whatever_it_covers() {
        let area = Rect::new(0, 0, 5, 3);
        let image = ImageId::from(7);
        let piece = |column: i32| ScreenCell {
            symbol: "A_1".to_string(),
            picture: Some(Showing {
                image,
                column,
                row: 0,
            }),
            selectable: true,
            layer: Some(Layer::Text),
            seam: None,
        };
        let text = |symbol: &str| ScreenCell {
            symbol: symbol.to_string(),
            picture: None,
            selectable: true,
            layer: Some(Layer::Text),
            seam: None,
        };
        // The same two-by-two picture shown twice, at columns 0 and 3.
        let pictured = |between: &str| vec![piece(0), piece(0), text(between), piece(3), piece(3)];
        let mut editor = Editor::new(String::new());
        editor.captures = vec![ScreenCapture {
            area,
            top: 0,
            rows: (0..)
                .zip([
                    pictured(","),
                    pictured(" "),
                    (0..5).map(|_| text("o")).collect(),
                ])
                .collect(),
        }];

        let left = MouseButton::Left;
        editor.handle_screen_mouse(mouse(MouseEventKind::Down(left), 0, 0));
        editor.handle_screen_mouse(mouse(MouseEventKind::Drag(left), 4, 2));
        let copied = editor.handle_screen_mouse(mouse(MouseEventKind::Up(left), 4, 2));

        let Some(Reaction::Copy(text)) = copied else {
            panic!("a drag copies its text");
        };
        assert_eq!(text, "A_1,A_1\nooooo");
    }

    #[test]
    fn a_selection_dragged_out_of_its_pane_stays_inside_it() {
        let shell = Rect::new(0, 0, 4, 2);
        let view = Rect::new(5, 0, 4, 2);
        let mut editor = Editor::new(String::new());
        editor.captures = vec![pane(shell, "s"), pane(view, "v")];

        let left = MouseButton::Left;
        editor.handle_screen_mouse(mouse(MouseEventKind::Down(left), view.x + 1, 0));
        editor.handle_screen_mouse(mouse(MouseEventKind::Drag(left), shell.x, 1));
        let copied = editor.handle_screen_mouse(mouse(MouseEventKind::Up(left), shell.x, 1));

        let Some(Reaction::Copy(text)) = copied else {
            panic!("a drag copies its text");
        };
        assert!(!text.contains('s'), "{text:?} reached into the shell");
        assert_eq!(text, "vvv\nv");
    }

    fn text_cell(symbol: &str) -> ScreenCell {
        ScreenCell {
            symbol: symbol.to_string(),
            picture: None,
            selectable: true,
            layer: Some(Layer::Text),
            seam: None,
        }
    }

    /// A pane drawing `lines` as plain text, every line copyable.
    fn drawn(lines: &[&str]) -> ScreenCapture {
        let width = lines
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        ScreenCapture {
            area: Rect::new(0, 0, width as u16, lines.len() as u16),
            top: 0,
            rows: (0..)
                .zip(lines.iter().map(|line| {
                    let mut symbols: Vec<String> = line.chars().map(String::from).collect();
                    symbols.resize(width, " ".to_string());
                    let selectable = selectable_columns(&symbols, &vec![Some(Layer::Text); width]);
                    symbols
                        .iter()
                        .zip(selectable)
                        .map(|(symbol, selectable)| ScreenCell {
                            selectable,
                            ..text_cell(symbol)
                        })
                        .collect()
                }))
                .collect(),
        }
    }

    /// The text a double click at `column`, `line` copies from `capture`.
    fn double_click(capture: ScreenCapture, column: u16, line: u16) -> Option<String> {
        let mut editor = Editor::new(String::new());
        editor.captures = vec![capture];
        match editor.pick_screen_text(
            mouse(MouseEventKind::Down(MouseButton::Left), column, line),
            2,
        ) {
            Some(Reaction::Copy(text)) => Some(text),
            _ => None,
        }
    }

    #[test]
    fn a_double_click_copies_the_word_under_it() {
        let capture = drawn(&["x = foo_bar2 + 1"]);
        assert_eq!(double_click(capture, 7, 0).as_deref(), Some("foo_bar2"));
    }

    #[test]
    fn a_double_click_on_a_bracket_copies_through_its_partner() {
        let lines = ["{a, (b, [c],", " d), e}"];
        assert_eq!(
            double_click(drawn(&lines), 4, 0).as_deref(),
            Some("(b, [c],\n d)")
        );
        assert_eq!(
            double_click(drawn(&lines), 2, 1).as_deref(),
            Some("(b, [c],\n d)")
        );
        assert_eq!(
            double_click(drawn(&lines), 6, 1).as_deref(),
            Some("{a, (b, [c],\n d), e}")
        );
    }

    #[test]
    fn a_double_click_on_a_tall_fence_copies_its_block_as_one_line() {
        let piece = |image: u32, column: i32, code: &str| ScreenCell {
            symbol: code.to_string(),
            picture: Some(Showing {
                image: ImageId::from(image),
                column,
                row: 0,
            }),
            selectable: true,
            layer: Some(Layer::Text),
            seam: None,
        };
        // `{a/b, c/d}` with each fraction a picture two rows high, between braces
        // drawn from their halves.
        let row = |open: &str, between: &str, close: &str| {
            vec![
                text_cell(open),
                piece(1, 1, "a/b"),
                piece(1, 1, "a/b"),
                text_cell(between),
                text_cell(" "),
                piece(2, 5, "c/d"),
                piece(2, 5, "c/d"),
                text_cell(close),
            ]
        };
        let (short_open, short_close) = (SHORT_BRACES[0].1, SHORT_BRACES[1].1);
        let capture = ScreenCapture {
            area: Rect::new(0, 0, 8, 2),
            top: 0,
            rows: (0..)
                .zip([
                    row(short_open[0], ",", short_close[0]),
                    row(short_open[2], " ", short_close[2]),
                ])
                .collect(),
        };
        assert_eq!(double_click(capture, 0, 1).as_deref(), Some("{a/b, c/d}"));
    }

    #[test]
    fn a_selection_keeps_rows_scrolled_out_of_view() {
        let mut editor = Editor::new(String::new());
        let mut capture = drawn(&["one", "two", "six", "ten"]);
        capture.area.height = 2;
        capture.top = 2;
        capture.rows = capture.rows.into_iter().collect();
        editor.captures = vec![capture];
        let left = MouseButton::Left;

        editor.handle_screen_mouse(mouse(MouseEventKind::Down(left), 2, 1));
        // The pane scrolled back to the top while the button was held.
        editor.captures[0].top = 0;
        let edge = editor.handle_screen_mouse(mouse(MouseEventKind::Drag(left), 0, 0));
        let copied = editor.handle_screen_mouse(mouse(MouseEventKind::Up(left), 0, 0));

        assert!(matches!(edge, Some(Reaction::ScrollView(rows)) if rows > 0));
        let Some(Reaction::Copy(text)) = copied else {
            panic!("a drag copies its text");
        };
        assert_eq!(text, "one\ntwo\nsix\nten");
        assert_eq!(
            editor.selected_screen_text().as_deref(),
            Some("one\ntwo\nsix\nten"),
            "copying keeps the selection"
        );
    }

    /// `editor` with a value and its type on one row, then dragged across all of it
    /// from `from` to `to`, returning what it copied.
    fn dragged(editor: &mut Editor, from: u16, to: u16) -> String {
        let left = MouseButton::Left;
        editor.handle_mouse(mouse(MouseEventKind::Down(left), from, 0));
        editor.handle_mouse(mouse(MouseEventKind::Drag(left), to, 0));
        match editor.handle_mouse(mouse(MouseEventKind::Up(left), to, 0)) {
            Reaction::Copy(text) => text,
            _ => panic!("a drag copies its text"),
        }
    }

    /// A pane showing `1 + x   ZZ[x]`, the type on a layer of its own, behind a
    /// gutter of two decorated columns.
    fn value_and_type() -> ScreenCapture {
        let text = "| 1 + x   ZZ[x]";
        let symbols: Vec<String> = text.chars().map(String::from).collect();
        let layers: Vec<Option<Layer>> = (0..text.len())
            .map(|column| match column {
                0..2 => None,
                2..10 => Some(Layer::Text),
                _ => Some(Layer::Type),
            })
            .collect();
        let selectable = selectable_columns(&symbols, &layers);
        let cells = symbols
            .into_iter()
            .zip(layers)
            .zip(selectable)
            .map(|((symbol, layer), selectable)| ScreenCell {
                symbol,
                picture: None,
                selectable,
                layer,
                seam: None,
            })
            .collect();
        ScreenCapture {
            area: Rect::new(0, 0, text.len() as u16, 1),
            top: 0,
            rows: BTreeMap::from([(0, cells)]),
        }
    }

    #[test]
    fn a_drag_stays_on_the_layer_it_starts_on() {
        let mut editor = Editor::new(String::new());
        editor.captures = vec![value_and_type()];

        assert_eq!(dragged(&mut editor, 2, 14), "1 + x");
        assert_eq!(dragged(&mut editor, 14, 2), "ZZ[x]");
    }

    #[test]
    fn a_double_click_on_a_type_reads_the_type() {
        assert_eq!(double_click(value_and_type(), 11, 0).as_deref(), Some("ZZ"));
    }

    #[test]
    fn raw_selection_reads_every_character_as_drawn() {
        let mut editor = Editor::new(String::new());
        editor.settings.selection.raw = true;
        editor.captures = vec![value_and_type()];

        assert_eq!(dragged(&mut editor, 0, 14), "| 1 + x   ZZ[x]");
    }

    #[test]
    fn a_double_click_marks_what_it_copies() {
        let mut editor = Editor::new(String::new());
        editor.captures = vec![drawn(&["x = foo + 1"])];
        let press = mouse(MouseEventKind::Down(MouseButton::Left), 5, 0);

        assert!(matches!(
            editor.pick_screen_text(press, 2),
            Some(Reaction::Copy(_))
        ));
        assert_eq!(editor.selected_screen_text().as_deref(), Some("foo"));

        let elsewhere = mouse(MouseEventKind::Down(MouseButton::Left), 0, 0);
        editor.handle_screen_mouse(elsewhere);
        assert_eq!(
            editor.selected_screen_text(),
            None,
            "a new press lets it go"
        );
    }

    #[test]
    fn with_copy_on_select_off_a_selection_waits_for_the_copy_key() {
        let mut editor = Editor::new(String::new());
        editor.settings.selection.copy_on_select = false;
        editor.captures = vec![drawn(&["x = foo + 1"])];
        let left = MouseButton::Left;

        editor.handle_mouse(mouse(MouseEventKind::Down(left), 4, 0));
        editor.handle_mouse(mouse(MouseEventKind::Drag(left), 6, 0));
        let released = editor.handle_mouse(mouse(MouseEventKind::Up(left), 6, 0));

        assert!(matches!(released, Reaction::Continue));
        let copied = editor.handle_action(Action::CopySelection);
        assert!(matches!(copied, Reaction::Copy(text) if text == "foo"));
    }

    /// An editor whose feed shows `block`, drawn on a screen of `size` and ready to
    /// select from, with that screen's rows as text.
    fn showing(block: Box<dyn Block>, size: (u16, u16)) -> (Editor, Vec<String>) {
        let mut editor = Editor::new(String::new());
        editor.apply(EditorUpdate::Message(block));
        let mut terminal = Terminal::new(TestBackend::new(size.0, size.1)).expect("test terminal");
        terminal
            .draw(|frame| {
                editor.draw(frame, frame.area(), &mut SyntaxHighlighter::new(), false);
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let rows = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect()
            })
            .collect();
        (editor, rows)
    }

    /// What dragging across the whole feed copies.
    fn dragged_across_the_feed(editor: &mut Editor) -> String {
        let area = editor.feed_area;
        let left = MouseButton::Left;
        let (right, bottom) = (area.right() - 1, area.bottom() - 1);
        editor.handle_screen_mouse(mouse(MouseEventKind::Down(left), area.x, area.y));
        editor.handle_screen_mouse(mouse(MouseEventKind::Drag(left), right, bottom));
        match editor.handle_screen_mouse(mouse(MouseEventKind::Up(left), right, bottom)) {
            Some(Reaction::Copy(text)) => text,
            _ => panic!("a drag copies its text"),
        }
    }

    fn page(html: &str) -> Box<dyn Block> {
        Box::new(OutputBlock::rendered(
            M2Value::Markup {
                html: html.into(),
                fallback: String::new(),
                example_width: 77,
            },
            None,
        ))
    }

    #[test]
    fn code_embedded_in_a_page_copies_without_its_gutter() {
        let code = "f = x -> (\n    x + 1\n)";
        for html in [
            format!("<p>Before</p><pre><code class='language-macaulay2'>{code}</code></pre>"),
            format!("<ul><li><pre><code class='language-macaulay2'>{code}</code></pre></li></ul>"),
        ] {
            let (mut editor, rows) = showing(page(&html), (60, 30));
            assert!(
                rows.iter().any(|row| row.contains("x + 1")),
                "the code is drawn: {rows:#?}"
            );
            let copied = dragged_across_the_feed(&mut editor);
            assert!(
                copied.contains(&format!("\n{code}")) || copied.starts_with(code),
                "its own indentation stays and no other does: {copied:?}"
            );
        }
    }

    #[test]
    fn an_example_in_a_page_copies_only_its_code_and_value() {
        let html = "<pre><code>i1 : 2+2\n\no1 = 4\n\no1 : ZZ</code></pre>";
        let (mut editor, rows) = showing(page(html), (60, 30));
        assert!(
            rows.iter().any(|row| row.contains("o1")),
            "the example keeps its label: {rows:#?}"
        );
        let copied = dragged_across_the_feed(&mut editor);
        let lines: Vec<&str> = copied.lines().filter(|line| !line.is_empty()).collect();
        assert_eq!(lines, ["2+2", "4"], "{copied:?}");
    }

    #[test]
    fn a_drawn_value_and_its_type_select_apart() {
        let block = OutputBlock::value("value42".to_string(), Some("Type7".to_string()));
        let (mut editor, rows) = showing(Box::new(block), (60, 30));
        let (line, row) = rows
            .iter()
            .enumerate()
            .find_map(|(line, row)| row.contains("value42").then_some((line as u16, row)))
            .expect("the value is drawn");
        let column = |text: &str| {
            let bytes = row.find(text).expect("drawn on one row");
            row[..bytes].chars().count() as u16
        };
        let (value, ty) = (column("value42"), column("Type7"));
        let left = MouseButton::Left;
        let mut drag = |from: u16, to: u16| {
            editor.handle_screen_mouse(mouse(MouseEventKind::Down(left), from, line));
            editor.handle_screen_mouse(mouse(MouseEventKind::Drag(left), to, line));
            match editor.handle_screen_mouse(mouse(MouseEventKind::Up(left), to, line)) {
                Some(Reaction::Copy(text)) => text,
                _ => panic!("a drag copies its text"),
            }
        };

        assert_eq!(drag(value, ty + 4), "value42");
        assert_eq!(drag(ty + 4, value), "Type7");
    }

    /// A feed of one submission: `1+1` and `2+2` with their values, each shown with
    /// the number M2 gave it.
    fn session() -> (Editor, Vec<String>) {
        use crate::ui::cell::{CellState, EvaluationBlock};
        let mut block = EvaluationBlock::new(
            1,
            [
                ("1+1".to_string(), Vec::new()),
                ("2+2".to_string(), Vec::new()),
            ],
        );
        for (index, (number, value)) in [(1, "2"), (2, "4")].into_iter().enumerate() {
            let output = OutputBlock::value(value.to_string(), Some("ZZ".into())).numbered(number);
            block.succeed(
                index,
                number,
                CellState::Value,
                Vec::new(),
                None,
                Some(output),
            );
        }
        showing(Box::new(block), (60, 30))
    }

    fn pressed(key: crossterm::event::KeyCode) -> crate::ui::key::KeyEvent {
        crate::ui::key::KeyEvent::new(key)
    }

    /// What `y` copies after the selection has stepped `steps` times.
    fn yanked_after(editor: &mut Editor, steps: &[Action]) -> Option<String> {
        for &step in steps {
            editor.handle_action(step);
        }
        match editor.handle_input(pressed(crossterm::event::KeyCode::Char('y'))) {
            Reaction::Copy(text) => Some(text),
            _ => None,
        }
    }

    #[test]
    fn the_keys_step_through_cells_and_results_from_the_last_up() {
        let (mut editor, _) = session();
        let up = Action::SelectPreviousBlock;
        assert_eq!(yanked_after(&mut editor, &[up]).as_deref(), Some("4"));
        assert_eq!(yanked_after(&mut editor, &[up]).as_deref(), Some("2"));
        assert_eq!(yanked_after(&mut editor, &[up]).as_deref(), Some("2+2"));
        assert_eq!(yanked_after(&mut editor, &[up]).as_deref(), Some("1+1"));
        assert_eq!(
            yanked_after(&mut editor, &[up]).as_deref(),
            Some("1+1"),
            "the first cell is the end"
        );
        let down = Action::SelectNextBlock;
        assert_eq!(yanked_after(&mut editor, &[down]).as_deref(), Some("2+2"));
    }

    #[test]
    fn a_session_copy_carries_the_prompts() {
        let (mut editor, _) = session();
        editor.settings.selection.copy_format = CopyFormat::Session;
        let up = Action::SelectPreviousBlock;
        assert_eq!(
            yanked_after(&mut editor, &[up]).as_deref(),
            Some("o2 = 4\n\no2 : ZZ")
        );
        assert_eq!(
            yanked_after(&mut editor, &[up, up, up]).as_deref(),
            Some("i1 : 1+1")
        );
    }

    #[test]
    fn a_key_other_than_the_selecting_ones_leaves_the_selection_and_types() {
        let (mut editor, _) = session();
        editor.handle_action(Action::SelectPreviousBlock);
        assert!(editor.feed.has_selection());
        editor.handle_input(pressed(crossterm::event::KeyCode::Char('k')));
        assert!(!editor.feed.has_selection());
        assert_eq!(editor.analysis_input().0, "k");

        editor.handle_action(Action::SelectPreviousBlock);
        editor.handle_input(pressed(crossterm::event::KeyCode::Esc));
        assert!(!editor.feed.has_selection());
        assert_eq!(editor.analysis_input().0, "k", "Esc only lets go");
    }

    /// Press the left button twice or three times on one cell, as a click run does,
    /// and return what the last press reacted with.
    fn clicked(editor: &mut Editor, times: usize, column: u16, row: u16) -> Reaction {
        let press = mouse(MouseEventKind::Down(MouseButton::Left), column, row);
        let mut reaction = Reaction::Continue;
        for _ in 0..times {
            reaction = editor.handle_mouse(press);
            editor.handle_mouse(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                ..press
            });
        }
        reaction
    }

    #[test]
    fn a_double_click_outside_the_text_takes_the_cell_or_result_whole() {
        let (mut editor, rows) = session();
        let row = rows
            .iter()
            .position(|row| row.contains("2+2"))
            .expect("the cell is drawn") as u16;
        let Reaction::Copy(text) = clicked(&mut editor, 2, 0, row) else {
            panic!("clicking beside a cell copies it");
        };
        assert_eq!(text, "2+2");
        assert!(editor.feed.has_selection());
    }

    #[test]
    fn a_triple_click_outside_the_text_takes_the_whole_submission() {
        let (mut editor, rows) = session();
        let row = rows
            .iter()
            .position(|row| row.contains("2+2"))
            .expect("the cell is drawn") as u16;
        editor.settings.selection.copy_format = CopyFormat::Session;
        let Reaction::Copy(text) = clicked(&mut editor, 3, 0, row) else {
            panic!("clicking beside a cell copies it");
        };
        assert_eq!(
            text,
            "i1 : 1+1\n\no1 = 2\n\no1 : ZZ\n\ni2 : 2+2\n\no2 = 4\n\no2 : ZZ"
        );
    }

    #[test]
    fn a_press_elsewhere_lets_go_of_a_selected_block() {
        let (mut editor, rows) = session();
        let row = rows.iter().position(|row| row.contains("2+2")).unwrap() as u16;
        clicked(&mut editor, 2, 0, row);
        assert!(editor.feed.has_selection());
        clicked(&mut editor, 1, 20, row + 8);
        assert!(!editor.feed.has_selection());
    }

    #[test]
    fn a_selection_made_by_hand_is_copied_only_when_asked_to() {
        let (mut editor, rows) = session();
        editor.settings.selection.copy_on_select = false;
        let row = rows.iter().position(|row| row.contains("2+2")).unwrap() as u16;
        assert!(matches!(
            clicked(&mut editor, 2, 0, row),
            Reaction::Continue
        ));
        let Reaction::Copy(text) = editor.handle_action(Action::CopySelection) else {
            panic!("the copy key copies what was selected");
        };
        assert_eq!(text, "2+2");
    }

    #[test]
    fn a_run_of_clicks_counts_to_three_and_starts_over() {
        let mut editor = Editor::new(String::new());
        let press = mouse(MouseEventKind::Down(MouseButton::Left), 4, 4);
        let counts: Vec<u8> = (0..4).map(|_| editor.clicks(press)).collect();
        assert_eq!(counts, [1, 2, 3, 1]);
        let elsewhere = mouse(MouseEventKind::Down(MouseButton::Left), 5, 4);
        assert_eq!(editor.clicks(elsewhere), 1);
        let release = mouse(MouseEventKind::Up(MouseButton::Left), 5, 4);
        assert_eq!(editor.clicks(release), 0);
    }

    #[test]
    fn a_paragraph_is_the_rows_of_text_between_the_empty_ones() {
        let area = Rect::new(0, 0, 4, 5);
        let mut capture = pane(area, "ab");
        for cell in capture.rows.get_mut(&2).unwrap() {
            cell.selectable = false;
        }
        let rows = |at: i64| {
            capture
                .paragraph(ScreenPosition::new(1, at))
                .map(|(region, _)| region.rows())
        };
        assert_eq!(rows(0), Some(0..=1));
        assert_eq!(rows(1), Some(0..=1));
        assert_eq!(rows(2), None, "an empty row is no paragraph");
        assert_eq!(rows(4), Some(3..=4));
    }

    #[test]
    fn a_double_click_in_the_feed_is_painted_as_a_selection() {
        let block = OutputBlock::value("value42 next".to_string(), None);
        let mut editor = Editor::new(String::new());
        editor.apply(EditorUpdate::Message(Box::new(block)));
        let mut terminal = Terminal::new(TestBackend::new(60, 30)).expect("test terminal");
        let mut draw = |editor: &mut Editor| {
            terminal
                .draw(|frame| {
                    editor.draw(frame, frame.area(), &mut SyntaxHighlighter::new(), false);
                })
                .expect("draw");
            terminal.backend().buffer().clone()
        };
        let before = draw(&mut editor);
        let (x, y) = (0..before.area.height)
            .find_map(|y| {
                let row: Vec<&str> = (0..before.area.width)
                    .map(|x| before[(x, y)].symbol())
                    .collect();
                let x = row.windows(2).position(|pair| pair == ["v", "a"])?;
                Some((x as u16, y))
            })
            .expect("the value is drawn");

        let press = mouse(MouseEventKind::Down(MouseButton::Left), x, y);
        assert!(editor.pick_screen_text(press, 2).is_some());
        let after = draw(&mut editor);

        let selected = scheme().selection().bg;
        assert_eq!(
            after[(x, y)].bg,
            selected.expect("selections have a colour")
        );
        assert_eq!(after[(x + 6, y)].bg, selected.unwrap());
        assert_ne!(
            after[(x + 7, y)].bg,
            selected.unwrap(),
            "the word ends there"
        );
    }

    #[test]
    fn a_change_to_the_transcript_lets_a_finished_selection_go() {
        let mut editor = Editor::new(String::new());
        editor.captures = vec![drawn(&["x = foo + 1"])];
        let press = mouse(MouseEventKind::Down(MouseButton::Left), 5, 0);
        editor.pick_screen_text(press, 2);

        editor.apply(EditorUpdate::Ignore);
        assert!(editor.selected_screen_text().is_some());
        editor.apply(EditorUpdate::ExamplesComputed);
        assert!(editor.selected_screen_text().is_none());
    }
}
