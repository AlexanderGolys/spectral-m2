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

mod caret;

pub use self::caret::{CaretPane, ScreenCaret};
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
    fn edge_scroll(&self, line: u16) -> Option<isize> {
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
