//! Mouse selection over the drawn screen: what each pane drew, which of its
//! cells may be copied, and the text a drag or a double click selects. A
//! selection belongs to the pane it starts in and never leaves it, but reaches
//! past what the pane shows: dragging against its edge scrolls it, and every row
//! drawn while the selection lasts stays captured.

use std::collections::{BTreeMap, HashSet};
use std::ops::RangeInclusive;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::Frame;

use super::settings::theme::groups::scheme;
use super::{Editor, Reaction};
use crate::ui::graphics::{
    self,
    kitty::{ImageId, Piece},
};
use crate::ui::text_object::{Bracket, CharClass};

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

/// A mouse selection spanning captured cells of one pane, alive while the button
/// is held: releasing it copies what it covers and lets it go.
pub struct ScreenSelection {
    /// The area of the pane the selection started in.
    area: Rect,
    anchor: ScreenPosition,
    focus: ScreenPosition,
    dragged: bool,
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
    /// Only text outside every selection exclusion, as a drag reads it.
    Copyable,
    /// Every cell drawn, as a double click on an excluded type reads it.
    Drawn,
}

impl Reach {
    /// Whether a row holding `cell` takes part in the copy at all.
    fn admits(self, cell: &ScreenCell) -> bool {
        self == Self::Drawn || cell.copyable
    }

    /// Whether `cell` contributes its text.
    fn reads(self, cell: &ScreenCell) -> bool {
        self == Self::Drawn || cell.selectable
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
    /// Whether the cell holds text a selection copies.
    selectable: bool,
    /// Whether the cell lies outside every selection exclusion, such as a frame,
    /// so a row of such cells with no text still copies as an empty line.
    copyable: bool,
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
        let mut lines = Vec::new();
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
                lines.extend(Self::band_line(&band, reach, &mut copied));
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
        let reach = match cell.copyable {
            true => Reach::Copyable,
            false => Reach::Drawn,
        };
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
        let characters: Vec<(ScreenPosition, char)> = self
            .rows
            .iter()
            .flat_map(|(&row, cells)| {
                cells
                    .iter()
                    .zip(self.area.x..)
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
                let pointer = Position::new(event.column, event.row);
                let capture = self
                    .captures
                    .iter()
                    .find(|capture| capture.area.contains(pointer))?;
                let position = capture.position(event);
                self.screen_selection = Some(ScreenSelection {
                    area: capture.area,
                    anchor: position,
                    focus: position,
                    dragged: false,
                });
                self.screen_selecting = true;
            }
            MouseEventKind::Drag(MouseButton::Left) if self.screen_selecting => {
                let selection = self.screen_selection.as_mut()?;
                let capture = self
                    .captures
                    .iter()
                    .find(|capture| capture.area == selection.area)?;
                selection.focus = capture.position(event);
                selection.dragged = true;
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
                        selection.focus = capture.position(event);
                    }
                }
                let text = self.selected_screen_text();
                self.screen_selecting = false;
                self.screen_selection = None;
                if let Some(text) = text {
                    return Some(Reaction::Copy(text));
                }
            }
            _ => return None,
        }
        Some(Reaction::Continue)
    }

    /// Copy what a double click picks out of the pane under it: a word, a
    /// bracketed block or a picture, typed annotations included.
    pub fn pick_screen_text(&mut self, event: MouseEvent) -> Option<Reaction> {
        let pointer = Position::new(event.column, event.row);
        let capture = self
            .captures
            .iter()
            .find(|capture| capture.area.contains(pointer))?;
        let (region, reach) = capture.pick(capture.position(event))?;
        let text = capture.text(region, reach)?;
        self.screen_selecting = false;
        self.screen_selection = None;
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
        if !selection.dragged {
            return None;
        }
        let capture = self.capture_of(selection)?;
        capture.text(selection.region(), Reach::Copyable)
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
                let allowed: Vec<bool> = (area.x..area.right())
                    .map(|x| {
                        let position = Position::new(x, y);
                        !self.selection_exclusions.contains(position)
                    })
                    .collect();
                let selectable = selectable_columns(&symbols, &allowed);
                let cells = symbols
                    .into_iter()
                    .zip(pictures)
                    .zip(selectable)
                    .zip(allowed)
                    .map(|(((symbol, picture), selectable), copyable)| ScreenCell {
                        symbol,
                        picture,
                        selectable,
                        copyable,
                    })
                    .collect();
                (row, cells)
            })
            .collect::<BTreeMap<_, _>>();
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
            .filter(|selection| selection.dragged && selection.area == self.feed_area)
        else {
            return;
        };
        let Some(capture) = self.capture_of(selection) else {
            return;
        };
        let region = selection.region();
        let buffer = frame.buffer_mut();
        for content_row in region.rows() {
            let Some(y) = capture.line(content_row) else {
                continue;
            };
            for x in region.columns(content_row, capture.area) {
                let at = ScreenPosition::new(x, content_row);
                if capture.cell(at).is_some_and(|cell| cell.selectable) {
                    buffer[(x, y)].set_style(scheme().selection());
                }
            }
        }
    }
}

impl ScreenSelection {
    fn region(&self) -> Region {
        Region::Flow {
            start: self.anchor.min(self.focus),
            end: self.anchor.max(self.focus),
        }
    }
}

fn selectable_columns(symbols: &[String], allowed: &[bool]) -> Vec<bool> {
    let mut selectable = vec![false; symbols.len()];
    let mut segment_start = 0;
    while segment_start < symbols.len() {
        while segment_start < symbols.len() && !allowed[segment_start] {
            segment_start += 1;
        }
        let mut segment_end = segment_start;
        while segment_end < symbols.len() && allowed[segment_end] {
            segment_end += 1;
        }
        mark_selectable_segment(symbols, segment_start, segment_end, &mut selectable);
        segment_start = segment_end.saturating_add(1);
    }
    selectable
}

fn mark_selectable_segment(
    symbols: &[String],
    segment_start: usize,
    segment_end: usize,
    selectable: &mut [bool],
) {
    if segment_start >= segment_end {
        return;
    }
    let Some(last_content) = symbols[segment_start..segment_end]
        .iter()
        .rposition(|symbol| !symbol.trim().is_empty())
        .map(|index| segment_start + index)
    else {
        return;
    };
    selectable[segment_start..=last_content].fill(true);
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyModifiers;

    use super::*;
    use crate::ui::render::maths::SHORT_BRACES;

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
                    copyable: true,
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
                        copyable: line.is_some(),
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
            copyable: true,
        };
        let text = |symbol: &str| ScreenCell {
            symbol: symbol.to_string(),
            picture: None,
            selectable: true,
            copyable: true,
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
            copyable: true,
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
                    let selectable = selectable_columns(&symbols, &vec![true; width]);
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
        match editor.pick_screen_text(mouse(MouseEventKind::Down(MouseButton::Left), column, line))
        {
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
            copyable: true,
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
        assert!(
            editor.screen_selection.is_none(),
            "copying lets the selection go"
        );
    }
}
