//! A keyboard cursor over what a pane drew. It moves over the transcript or a
//! view as the input's cursor moves over source, selects and copies, follows
//! links, folds values, switches their formatter, and asks the language server
//! about the code under it, so none of that waits on the mouse.

use crossterm::event::KeyCode;
use ratatui::layout::Position;
use ratatui::Frame;

use super::{Extent, Reach, ScreenCapture, ScreenPosition, ScreenSelection};
use crate::input::{InputAction, Traversal};
use crate::ui::controller::{CodePoint, CodeSource, Editor, Lookup, Reaction};
use crate::ui::floating::Anchor;
use crate::ui::key::KeyEvent;
use crate::ui::render::selection::Layer;
use crate::ui::text_object::{TextDirection, TextMotion};

use TextDirection::{Backward, Forward};

/// Stands in for a cell that shows something other than one character, such as
/// a piece of a picture, so motions over a row still count it.
const OBJECT: char = '\u{FFFC}';
/// What a lookup on the caret says when there is no session code under it.
const NO_CODE_UNDER_CARET: &str = "no session code under the cursor";

/// The pane a caret is in: the session's shell, or the view beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaretPane {
    Shell,
    View,
}

/// The keyboard cursor in one pane, kept as a content row so it stays on its text
/// while the pane scrolls.
#[derive(Clone, Copy, Debug)]
pub struct ScreenCaret {
    pane: CaretPane,
    /// Where it stands; none until the pane is drawn with it, which places it on
    /// the last text shown.
    at: Option<ScreenPosition>,
    /// Whether moving drags a selection along.
    selecting: bool,
}

/// One move of the caret.
#[derive(Clone, Copy, Debug)]
enum CaretStep {
    /// Over the text of its row, as the input moves over a line.
    Text(TextMotion, TextDirection),
    /// To the row above or below.
    Row(TextDirection),
    /// To the partner of the bracket under it.
    MatchingBracket,
}

/// One row's characters, each with the column it is drawn at.
pub struct RowText {
    pub text: String,
    /// The byte each character starts at and its column.
    characters: Vec<(usize, u16)>,
}

impl RowText {
    /// The byte of the character drawn at or before `column`.
    fn offset(&self, column: u16) -> usize {
        self.characters
            .iter()
            .rev()
            .find(|(_, at)| *at <= column)
            .map_or(0, |(offset, _)| *offset)
    }

    /// The column the character at byte `offset` is drawn at; the last one's for
    /// the end of the row.
    fn column(&self, offset: usize) -> Option<u16> {
        self.characters
            .iter()
            .find(|(start, _)| *start >= offset)
            .or(self.characters.last())
            .map(|(_, column)| *column)
    }

    /// The columns of its first and last characters other than whitespace.
    fn ends(&self) -> Option<(u16, u16)> {
        let shows = |(offset, _): &&(usize, u16)| {
            self.text[*offset..]
                .chars()
                .next()
                .is_some_and(|character| !character.is_whitespace())
        };
        let first = self.characters.iter().find(shows)?.1;
        let last = self.characters.iter().rev().find(shows)?.1;
        Some((first, last))
    }
}

impl ScreenCapture {
    pub fn row_text(&self, row: i64) -> RowText {
        let mut text = String::new();
        let mut characters = Vec::new();
        for (cell, column) in self.rows.get(&row).into_iter().flatten().zip(self.area.x..) {
            if cell.symbol.is_empty() {
                continue;
            }
            characters.push((text.len(), column));
            text.push(cell.character().unwrap_or(OBJECT));
        }
        RowText { text, characters }
    }

    /// Whether `row` shows any text a selection could read.
    fn shows_text(&self, row: i64) -> bool {
        self.rows.get(&row).is_some_and(|cells| {
            cells
                .iter()
                .any(|cell| cell.layer.is_some() && cell.selectable)
        })
    }

    /// Where a caret entering the pane stands: the start of the last row of text.
    fn resting_place(&self) -> ScreenPosition {
        let row = self
            .rows
            .keys()
            .rev()
            .copied()
            .find(|&row| self.shows_text(row))
            .unwrap_or(self.top);
        let column = self
            .row_text(row)
            .ends()
            .map_or(self.area.x, |(first, _)| first);
        ScreenPosition::new(column, row)
    }

    fn last_row(&self) -> i64 {
        self.top + i64::from(self.area.height) - 1
    }

    /// The position `step` takes the caret to from `at`, and the rows the pane
    /// scrolls to show it, positive toward older content.
    fn step(&self, at: ScreenPosition, step: CaretStep) -> (ScreenPosition, isize) {
        let rightmost = self.area.right().saturating_sub(1);
        let to_row = |row: i64, column: u16| {
            let scroll = match row {
                _ if row < self.top => 1,
                _ if row > self.last_row() => -1,
                _ => 0,
            };
            (ScreenPosition::new(column, row), scroll)
        };
        match step {
            CaretStep::Row(Backward) => to_row(at.row - 1, at.column),
            CaretStep::Row(Forward) => to_row(at.row + 1, at.column),
            CaretStep::MatchingBracket => {
                let partner = self.pick(at).and_then(|(region, _)| match region {
                    super::Region::Flow { start, end } if start == at => Some(end),
                    super::Region::Flow { start, end } if end == at => Some(start),
                    _ => None,
                });
                (partner.unwrap_or(at), 0)
            }
            CaretStep::Text(TextMotion::Character, Backward) => (
                ScreenPosition::new(at.column.saturating_sub(1).max(self.area.x), at.row),
                0,
            ),
            CaretStep::Text(TextMotion::Character, Forward) => (
                ScreenPosition::new((at.column + 1).min(rightmost), at.row),
                0,
            ),
            CaretStep::Text(TextMotion::Line, direction) => {
                let ends = self.row_text(at.row).ends();
                let column = match direction {
                    Backward => ends.map_or(self.area.x, |(first, _)| first),
                    Forward => ends.map_or(self.area.x, |(_, last)| last),
                };
                (ScreenPosition::new(column, at.row), 0)
            }
            CaretStep::Text(TextMotion::Paragraph, direction) => {
                let edge = |row: i64, beside: i64| !self.shows_text(row) && self.shows_text(beside);
                let row = match direction {
                    Backward => (self.top..at.row)
                        .rev()
                        .find(|&row| edge(row, row + 1))
                        .unwrap_or(self.top - 1),
                    Forward => (at.row + 1..=self.last_row())
                        .find(|&row| edge(row, row - 1))
                        .unwrap_or(self.last_row() + 1),
                };
                to_row(row, self.area.x)
            }
            // Past either end of what is held, which drawing brings into view.
            CaretStep::Text(TextMotion::Full, Backward) => (
                ScreenPosition::new(self.area.x, i64::MIN / 2),
                isize::MAX / 2,
            ),
            CaretStep::Text(TextMotion::Full, Forward) => (
                ScreenPosition::new(self.area.x, i64::MAX / 2),
                -(isize::MAX / 2),
            ),
            CaretStep::Text(motion, direction) => {
                let row = self.row_text(at.row);
                let from = row.offset(at.column);
                let reached = motion.reach(&row.text, from, direction);
                match row.column(reached).filter(|&column| column != at.column) {
                    Some(column) => (ScreenPosition::new(column, at.row), 0),
                    None => {
                        let (row, end) = match direction {
                            Backward => (at.row - 1, Forward),
                            Forward => (at.row + 1, Backward),
                        };
                        let ends = self.row_text(row).ends();
                        let column = match end {
                            Forward => ends.map_or(self.area.x, |(_, last)| last),
                            Backward => ends.map_or(self.area.x, |(first, _)| first),
                        };
                        to_row(row, column)
                    }
                }
            }
        }
    }
}

impl CaretStep {
    /// The step a key the input binds takes, and whether it selects.
    fn of_edit(edit: InputAction) -> Option<(Self, bool)> {
        if let Some((traversal, motion, direction)) = edit.traversal() {
            return match traversal {
                Traversal::Move => Some((Self::Text(motion, direction), false)),
                Traversal::Select => Some((Self::Text(motion, direction), true)),
                Traversal::Delete => None,
            };
        }
        Some(match edit {
            InputAction::MoveUp => (Self::Row(Backward), false),
            InputAction::MoveDown => (Self::Row(Forward), false),
            InputAction::SelectUp => (Self::Row(Backward), true),
            InputAction::SelectDown => (Self::Row(Forward), true),
            InputAction::MoveMatchingBracket => (Self::MatchingBracket, false),
            _ => return None,
        })
    }

    /// The step a plain letter takes, as a pager's or a copy mode's would.
    fn of_letter(letter: char) -> Option<Self> {
        Some(match letter {
            'h' => Self::Text(TextMotion::Character, Backward),
            'l' => Self::Text(TextMotion::Character, Forward),
            'k' => Self::Row(Backward),
            'j' => Self::Row(Forward),
            'b' => Self::Text(TextMotion::WordAlpha, Backward),
            'w' => Self::Text(TextMotion::WordAlpha, Forward),
            'B' => Self::Text(TextMotion::WordAny, Backward),
            'W' => Self::Text(TextMotion::WordAny, Forward),
            '0' | '^' => Self::Text(TextMotion::Line, Backward),
            '$' => Self::Text(TextMotion::Line, Forward),
            '{' => Self::Text(TextMotion::Paragraph, Backward),
            '}' => Self::Text(TextMotion::Paragraph, Forward),
            'g' => Self::Text(TextMotion::Full, Backward),
            'G' => Self::Text(TextMotion::Full, Forward),
            '%' => Self::MatchingBracket,
            _ => return None,
        })
    }
}

impl Editor {
    fn focused_pane(&self) -> CaretPane {
        match self.previewing {
            true => CaretPane::View,
            false => CaretPane::Shell,
        }
    }

    /// Whether the focused pane holds the caret, so its keys move it.
    pub fn caret_active(&self) -> bool {
        self.caret
            .is_some_and(|caret| caret.pane == self.focused_pane())
    }

    /// Put the caret into the focused pane, or take it out again.
    pub fn toggle_caret(&mut self) {
        self.caret = match self.caret_active() {
            true => None,
            false => Some(ScreenCaret {
                pane: self.focused_pane(),
                at: None,
                selecting: false,
            }),
        };
        self.screen_selection = None;
        self.input.field_mut().clear_selection();
        self.refresh_status_line();
    }

    fn leave_caret(&mut self) -> Reaction {
        self.caret = None;
        self.screen_selection = None;
        self.refresh_status_line();
        Reaction::Continue
    }

    /// The capture of the pane the caret is in, and where in it the caret stands.
    pub fn caret_capture(&self) -> Option<(&ScreenCapture, ScreenPosition)> {
        let at = self.caret?.at?;
        let area = self.caret_area?;
        let capture = self.captures.iter().find(|capture| capture.area == area)?;
        Some((capture, at))
    }

    /// Where on screen the caret stands, when it is shown.
    fn caret_point(&self) -> Option<Position> {
        let (capture, at) = self.caret_capture()?;
        Some(Position::new(at.column, capture.line(at.row)?))
    }

    /// What a key does while the caret is in the focused pane; none when it is not.
    pub fn handle_caret_key(&mut self, key: KeyEvent) -> Option<Reaction> {
        if !self.caret_active() {
            return None;
        }
        self.floating = None;
        let plain = !key.ctrl() && !key.alt();
        let selecting = self.caret?.selecting;
        let step = match key.code {
            KeyCode::Char(letter) if plain => {
                CaretStep::of_letter(letter).map(|step| (step, selecting))
            }
            _ => self
                .keymaps
                .edit(key)
                .and_then(CaretStep::of_edit)
                .map(|(step, select)| (step, select || selecting)),
        };
        if let Some((step, selecting)) = step {
            return Some(self.step_caret(step, selecting));
        }
        Some(match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'i') if plain => self.leave_caret(),
            KeyCode::Char('v') if plain => {
                let caret = self.caret.as_mut()?;
                caret.selecting = !caret.selecting;
                if !caret.selecting {
                    self.screen_selection = None;
                }
                Reaction::Continue
            }
            KeyCode::Char('y') if plain => {
                let copied = self.selected_screen_text();
                if let Some(caret) = self.caret.as_mut() {
                    caret.selecting = false;
                }
                copied.map_or(Reaction::Continue, Reaction::Copy)
            }
            KeyCode::Char('K') if plain => self.look_up_at_caret(Lookup::Brief),
            _ if self.keymaps.edit(key) == Some(InputAction::GrowSelection) => {
                self.pick_at_caret();
                Reaction::Continue
            }
            _ => Reaction::Continue,
        })
    }

    /// Move the caret one `step`, dragging the selection along when `selecting`
    /// and letting it go otherwise.
    fn step_caret(&mut self, step: CaretStep, selecting: bool) -> Reaction {
        let Some((capture, at)) = self.caret_capture() else {
            return Reaction::Continue;
        };
        let (to, scroll) = capture.step(at, step);
        let area = capture.area;
        let layer = capture.cell(at).and_then(|cell| cell.layer);
        let raw = self.settings.selection.raw;
        if let Some(caret) = self.caret.as_mut() {
            caret.at = Some(to);
        }
        match selecting {
            true => {
                let selection = self
                    .screen_selection
                    .get_or_insert_with(|| ScreenSelection {
                        area,
                        extent: Extent::Pressed(at),
                        reach: match raw {
                            true => Reach::Verbatim,
                            false => Reach::Layer(layer.unwrap_or(Layer::Text)),
                        },
                    });
                selection.drag_to(to);
            }
            false => self.screen_selection = None,
        }
        match scroll {
            0 => Reaction::Continue,
            rows => Reaction::ScrollView(rows),
        }
    }

    /// Select what a double click on the caret would.
    fn pick_at_caret(&mut self) {
        let Some((capture, at)) = self.caret_capture() else {
            return;
        };
        let area = capture.area;
        let Some((region, reach)) = capture.pick(at).or_else(|| capture.paragraph(at)) else {
            return;
        };
        let reach = match self.settings.selection.raw {
            true => Reach::Verbatim,
            false => reach,
        };
        self.screen_selection = Some(ScreenSelection {
            area,
            extent: Extent::Picked(region),
            reach,
        });
    }

    /// What Enter does on the caret: open the link under it, else fold or unfold
    /// the value there, else switch to the formatter whose icon it is on.
    pub fn activate_caret(&mut self) -> Reaction {
        let Some(point) = self.caret_point() else {
            return Reaction::Continue;
        };
        let feed = match self.focused_pane() {
            CaretPane::Shell => &mut self.feed,
            CaretPane::View => &mut self.reference,
        };
        if let Some(link) = feed.link_at(point) {
            return Self::open_link(link.target.to_string(), false);
        }
        if !feed.toggle_fold(point) {
            feed.cycle_renderer(point);
        }
        Reaction::Continue
    }

    /// Ask the language server `lookup` about the code under the caret.
    pub fn look_up_at_caret(&mut self, lookup: Lookup) -> Reaction {
        let point = self.caret_point();
        let code = point
            .filter(|_| self.focused_pane() == CaretPane::Shell)
            .and_then(|point| self.feed.code_at(point));
        match (point, code) {
            (Some(point), Some(code)) => Reaction::Lookup(
                lookup,
                CodePoint {
                    source: CodeSource::Cell {
                        submission: code.submission,
                        index: code.index,
                    },
                    offset: code.offset,
                    anchor: Anchor::At(point),
                },
            ),
            _ => {
                self.input
                    .set_diagnostic(NO_CODE_UNDER_CARET.to_string(), None);
                Reaction::Continue
            }
        }
    }

    /// Show the caret in the pane just drawn, if it is there: placed on the last
    /// text when it has just entered, and kept inside what the pane shows.
    pub fn place_caret(&mut self, frame: &mut Frame, pane: CaretPane) {
        let Some(caret) = self.caret.filter(|caret| caret.pane == pane) else {
            return;
        };
        let area = self.feed_area;
        self.caret_area = Some(area);
        let Some(capture) = self.captures.iter().find(|capture| capture.area == area) else {
            return;
        };
        let at = caret.at.unwrap_or_else(|| capture.resting_place());
        let at = ScreenPosition::new(
            at.column.clamp(area.x, area.right().saturating_sub(1)),
            at.row.clamp(capture.top, capture.last_row()),
        );
        if let Some(line) = capture.line(at.row) {
            frame.set_cursor_position(Position::new(at.column, line));
        }
        if let Some(caret) = self.caret.as_mut() {
            caret.at = Some(at);
        }
    }
}
