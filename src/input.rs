//! Live input model.
//!
//! - [`InputTextField`] — raw code, a char-boundary cursor, submission history, and
//!   undo.
//! - [`InputCode`] — the field and its transient presentation state.
//! - [`Completion`] — what the text before the cursor may be completed to.

mod completion;
mod hints;
mod history;
mod undo;

use std::iter;
use std::ops::Range;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use serde::{Deserialize, Serialize};
use these_macros_should_be_illegal::discriminated_str;
use unicode_width::UnicodeWidthChar;

pub use self::completion::Completion;
use self::hints::Hint;
use self::history::History;
use self::undo::{EditKind, Snapshot, Undo};
use crate::code::highlight::Highlighter;
use crate::ui::block::{HLSpan, Text};
use crate::ui::config::UI;
use crate::ui::graphics::kitty::PLACEHOLDER;
use crate::ui::key::KeyEvent;
use crate::ui::render::{Highlights, Prompt, RenderContext};
use crate::ui::wrap::{HardWrap, TextLayout, WrapPolicy};

/// A text-editing operation owned and interpreted by the input field.
#[discriminated_str(description)]
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InputAction {
    InsertNewline = "insert a line break",
    InsertTab = "insert a tab",
    NextCompletion = "take the completion shown after the cursor, or the next one",
    PreviousCompletion = "take the previous completion",
    DeleteBack = "delete before the cursor",
    DeleteForward = "delete under the cursor",
    DeleteWordBack = "delete back to where a word-wise move left lands",
    DeleteWordForward = "delete on to where a word-wise move right lands",
    Undo = "undo the last edit",
    Redo = "redo the last undone edit",

    MoveLeft = "move left",
    MoveRight = "move right",
    MoveUp = "move up, or recall older input",
    MoveDown = "move down, or recall newer input",
    HistoryPrevious = "recall older matching input",
    HistoryNext = "recall newer matching input",
    MoveWordLeft = "move left by a word",
    MoveWordRight = "move right by a word",
    MoveLineStart = "move to line start",
    MoveLineEnd = "move to line end",
    MoveTextStart = "move to the start of the input",
    MoveTextEnd = "move to the end of the input",

    SelectLeft = "extend selection left",
    SelectRight = "extend selection right",
    SelectWordLeft = "extend selection left by a word",
    SelectWordRight = "extend selection right by a word",
    SelectLineStart = "extend selection to line start",
    SelectLineEnd = "extend selection to line end",
    SelectTextStart = "extend selection to the start of the input",
    SelectTextEnd = "extend selection to the end of the input",

    Clear = "clear input text",
}

/// User-adjustable layout policy owned by the input component.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputSettings {
    /// Maximum editor-height percentage occupied by the input.
    pub max_height_percent: u8,
    /// Context rows retained around the cursor.
    pub scrolloff: u8,
    /// Characters of a name typed before it is completed.
    pub complete_after: u8,
}

impl Default for InputSettings {
    fn default() -> Self {
        Self {
            max_height_percent: 75,
            scrolloff: 2,
            complete_after: 1,
        }
    }
}

/// One input setting exposed through the settings picker.
#[derive(Clone, Copy)]
pub enum InputSetting {
    /// Maximum input height.
    MaxHeight,
    /// Cursor context rows.
    Scrolloff,
    /// Name length that starts completion.
    CompleteAfter,
}

impl InputSetting {
    /// Picker label for this setting.
    pub const fn label(self) -> &'static str {
        match self {
            Self::MaxHeight => "max input height",
            Self::Scrolloff => "scrolloff",
            Self::CompleteAfter => "complete after",
        }
    }

    /// What this setting controls.
    pub const fn description(self) -> &'static str {
        match self {
            Self::MaxHeight => "share of the pane multiline input may grow into",
            Self::Scrolloff => "context rows kept around the input cursor",
            Self::CompleteAfter => "characters of a name typed before it is completed",
        }
    }

    /// Current picker value for this setting.
    pub fn value(self, settings: InputSettings) -> String {
        match self {
            Self::MaxHeight => format!("{}%", settings.max_height_percent),
            Self::Scrolloff => format!("{} rows", settings.scrolloff),
            Self::CompleteAfter => format!("{} characters", settings.complete_after),
        }
    }

    /// Adjust this setting by one picker step.
    pub fn adjust(self, settings: &mut InputSettings, delta: i8) {
        let field = match self {
            Self::MaxHeight => &mut settings.max_height_percent,
            Self::Scrolloff => &mut settings.scrolloff,
            Self::CompleteAfter => &mut settings.complete_after,
        };
        let step = delta.unsigned_abs()
            * if matches!(self, Self::MaxHeight) {
                5
            } else {
                1
            };
        *field = if delta > 0 {
            field.saturating_add(step)
        } else {
            field.saturating_sub(step)
        };
        settings.max_height_percent = settings.max_height_percent.min(100);
        settings.complete_after = settings.complete_after.max(1);
    }
}

/// Editable source, its byte-offset cursor, submission history, and undo.
pub struct InputTextField {
    text: String,
    cursor: usize,
    revision: u64,
    /// Where a selection started, if one is active. The cursor is its other end,
    /// so extending a selection is just moving the cursor.
    anchor: Option<usize>,
    history: History,
    edits: Undo,
}

impl InputTextField {
    fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            revision: 0,
            anchor: None,
            history: History::new(),
            edits: Undo::default(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The cursor's byte offset into the source.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The selected byte range, ordered, when a selection is active and not
    /// empty.
    fn selection(&self) -> Option<Range<usize>> {
        let anchor = self.anchor?;
        let (start, end) = (anchor.min(self.cursor), anchor.max(self.cursor));
        (start < end).then_some(start..end)
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.selection().map(|range| &self.text[range])
    }

    pub fn start_selection(&mut self, cursor: usize) {
        self.history.stop_browsing();
        if self.text.is_char_boundary(cursor) {
            self.cursor = cursor;
            self.anchor = Some(cursor);
        }
    }

    pub fn continue_selection(&mut self, cursor: usize) {
        if self.anchor.is_some() && self.text.is_char_boundary(cursor) {
            self.cursor = cursor;
        }
    }

    /// Drop the selection, keeping the cursor where it is. Every plain movement
    /// does this, so selecting is strictly a shifted gesture.
    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }

    /// Start a selection at the cursor if none is running, then move. The caller
    /// supplies the movement so extending reuses the ordinary cursor motions.
    fn extend_selection(&mut self, move_cursor: impl FnOnce(&mut Self)) {
        self.anchor.get_or_insert(self.cursor);
        move_cursor(self);
    }

    pub fn select_left(&mut self) {
        self.extend_selection(|field| {
            if field.cursor > 0 {
                field.cursor = field.previous_boundary();
            }
        });
    }

    pub fn select_right(&mut self) {
        self.extend_selection(|field| {
            if field.cursor < field.text.len() {
                field.cursor = field.next_boundary(field.cursor);
            }
        });
    }

    /// The cursor moved left over any whitespace, then over the run of word or
    /// punctuation characters before it.
    fn word_start(&self) -> usize {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end();
        let class = trimmed.chars().next_back().map(CharClass::of);
        let run = trimmed
            .chars()
            .rev()
            .take_while(|&character| Some(CharClass::of(character)) == class)
            .map(char::len_utf8)
            .sum::<usize>();
        trimmed.len() - run
    }

    /// The cursor moved right over any whitespace, then over the run of word or
    /// punctuation characters after it.
    fn word_end(&self) -> usize {
        let after = &self.text[self.cursor..];
        let trimmed = after.trim_start();
        let class = trimmed.chars().next().map(CharClass::of);
        let run = trimmed
            .chars()
            .take_while(|&character| Some(CharClass::of(character)) == class)
            .map(char::len_utf8)
            .sum::<usize>();
        self.text.len() - trimmed.len() + run
    }

    pub fn move_word_left(&mut self) {
        self.clear_selection();
        self.cursor = self.word_start();
    }

    pub fn move_word_right(&mut self) {
        self.clear_selection();
        self.cursor = self.word_end();
    }

    pub fn select_word_left(&mut self) {
        self.extend_selection(|field| field.cursor = field.word_start());
    }

    pub fn select_word_right(&mut self) {
        self.extend_selection(|field| field.cursor = field.word_end());
    }

    pub fn move_text_start(&mut self) {
        self.clear_selection();
        self.cursor = 0;
    }

    pub fn move_text_end(&mut self) {
        self.clear_selection();
        self.cursor = self.text.len();
    }

    pub fn select_text_start(&mut self) {
        self.extend_selection(|field| field.cursor = 0);
    }

    pub fn select_text_end(&mut self) {
        self.extend_selection(|field| field.cursor = field.text.len());
    }

    pub fn select_home(&mut self) {
        self.extend_selection(|field| {
            field.cursor = field.text[..field.cursor]
                .rfind('\n')
                .map_or(0, |index| index + 1);
        });
    }

    pub fn select_end(&mut self) {
        self.extend_selection(|field| {
            field.cursor = field.text[field.cursor..]
                .find('\n')
                .map_or(field.text.len(), |index| field.cursor + index);
        });
    }

    /// Remove the selection, if any, leaving the cursor where it stood. Typing
    /// and deleting both begin with this, so a selection behaves as one unit.
    fn delete_selection(&mut self) -> bool {
        let Some(range) = self.selection() else {
            self.clear_selection();
            return false;
        };
        self.delete(range);
        self.clear_selection();
        true
    }

    /// Remove `range`, leaving the cursor at its start.
    fn delete(&mut self, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        self.text.replace_range(range.clone(), "");
        self.cursor = range.start;
        self.changed();
    }

    /// Apply one undoable edit. An edit that changes nothing leaves no step,
    /// and one that replaces a selection never joins the step before it.
    fn edit(&mut self, kind: EditKind, apply: impl FnOnce(&mut Self)) {
        if kind != EditKind::Recalling {
            self.history.stop_browsing();
        }
        if self.selection().is_some() {
            self.edits.break_run();
        }
        let before = self.snapshot();
        let revision = self.revision;
        apply(self);
        if self.revision != revision {
            self.edits.record(kind, before, self.cursor);
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
        }
    }

    pub fn undo(&mut self) {
        self.history.stop_browsing();
        if let Some(previous) = self.edits.undo(self.snapshot()) {
            self.load(previous.text, previous.cursor);
        }
    }

    pub fn redo(&mut self) {
        self.history.stop_browsing();
        if let Some(next) = self.edits.redo(self.snapshot()) {
            self.load(next.text, next.cursor);
        }
    }

    /// Typing undoes a word at a time: a word begun after whitespace starts a
    /// new step.
    pub fn insert_char(&mut self, ch: char) {
        if !ch.is_whitespace() && self.text[..self.cursor].ends_with(char::is_whitespace) {
            self.edits.break_run();
        }
        self.edit(EditKind::Typing, |field| {
            field.delete_selection();
            field.text.insert(field.cursor, ch);
            field.cursor += ch.len_utf8();
            field.changed();
        });
    }

    /// Replace `range` with a completion, leaving the cursor after it.
    fn replace(&mut self, range: Range<usize>, replacement: &str) {
        self.edit(EditKind::Completing, |field| {
            field.clear_selection();
            field.text.replace_range(range.clone(), replacement);
            field.cursor = range.start + replacement.len();
            field.changed();
        });
    }

    pub fn insert_str(&mut self, s: &str) {
        self.edit(EditKind::Replacing, |field| {
            field.delete_selection();
            field.text.insert_str(field.cursor, s);
            field.cursor += s.len();
            field.changed();
        });
    }

    fn clear(&mut self) {
        self.edit(EditKind::Replacing, |field| {
            field.anchor = None;
            field.delete(0..field.text.len());
        });
    }

    pub fn backspace(&mut self) {
        self.edit(EditKind::DeletingBack, |field| {
            if !field.delete_selection() {
                field.delete(field.previous_boundary()..field.cursor);
            }
        });
    }

    pub fn delete_forward(&mut self) {
        self.edit(EditKind::DeletingForward, |field| {
            if !field.delete_selection() {
                field.delete(field.cursor..field.next_boundary(field.cursor));
            }
        });
    }

    /// Delete what a word-wise move left crosses.
    pub fn delete_word_back(&mut self) {
        self.edit(EditKind::DeletingBack, |field| {
            if !field.delete_selection() {
                field.delete(field.word_start()..field.cursor);
            }
        });
    }

    /// Delete what a word-wise move right crosses.
    pub fn delete_word_forward(&mut self) {
        self.edit(EditKind::DeletingForward, |field| {
            if !field.delete_selection() {
                field.delete(field.cursor..field.word_end());
            }
        });
    }

    pub fn move_left(&mut self) {
        self.clear_selection();
        if self.cursor > 0 {
            self.cursor = self.previous_boundary();
        }
    }

    pub fn move_right(&mut self) {
        self.clear_selection();
        if self.cursor < self.text.len() {
            self.cursor = self.next_boundary(self.cursor);
        }
    }

    pub fn move_home(&mut self) {
        self.clear_selection();
        self.cursor = self.text[..self.cursor]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
    }

    pub fn move_end(&mut self) {
        self.clear_selection();
        self.cursor = self.text[self.cursor..]
            .find('\n')
            .map(|i| self.cursor + i)
            .unwrap_or(self.text.len());
    }

    pub fn move_up(&mut self) {
        self.clear_selection();
        let starts = self.line_starts();
        let current = self.current_line(&starts);
        if current == 0 {
            return;
        }
        let column = self.text[starts[current]..self.cursor].chars().count();
        self.cursor = self.cursor_at_column(&starts, current - 1, column);
    }

    pub fn move_down(&mut self) {
        self.clear_selection();
        let starts = self.line_starts();
        let current = self.current_line(&starts);
        if current + 1 == starts.len() {
            return;
        }
        let column = self.text[starts[current]..self.cursor].chars().count();
        self.cursor = self.cursor_at_column(&starts, current + 1, column);
    }

    #[cfg(test)]
    fn cursor_line_col(&self) -> (usize, usize) {
        // `str::lines()` drops a trailing newline, so a cursor sitting just past one
        // would report the previous row. Count newlines directly, and take the column
        // from the text after the last one (the whole prefix when there is none).
        let before = &self.text[..self.cursor];
        let line = before.matches('\n').count();
        let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count());
        (line, col)
    }

    pub fn history_prev(&mut self) {
        if let Some(buffer) = self.history.prev(&self.text, self.cursor) {
            self.edit(EditKind::Recalling, |field| {
                field.load(buffer.text, buffer.cursor)
            });
        }
    }

    pub fn history_next(&mut self) {
        if let Some(buffer) = self.history.next(&self.text) {
            self.edit(EditKind::Recalling, |field| {
                field.load(buffer.text, buffer.cursor)
            });
        }
    }

    /// Replace the whole buffer with source from elsewhere, as one undoable step.
    fn replace_text(&mut self, text: String) {
        self.edit(EditKind::Replacing, |field| {
            let cursor = text.len();
            field.load(text, cursor);
        });
    }

    /// Record the submission and start a fresh input with nothing to undo.
    fn accept_submission(&mut self) {
        self.history.record(&self.text);
        self.load(String::new(), 0);
        self.edits = Undo::default();
    }

    /// Replace the whole buffer, placing the cursor at `cursor` when it lands on
    /// a character boundary and at the end otherwise. The anchor indexes the
    /// text it was set in, so every wholesale replacement has to drop it -- a
    /// stale anchor left pointing past the new end makes the next edit or copy
    /// slice out of bounds.
    fn load(&mut self, text: String, cursor: usize) {
        self.text = text;
        self.cursor = cursor.min(self.text.len());
        if !self.text.is_char_boundary(self.cursor) {
            self.cursor = self.text.len();
        }
        self.clear_selection();
        self.changed();
    }

    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn previous_boundary(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .last()
            .map_or(0, |(index, _)| index)
    }

    fn next_boundary(&self, from: usize) -> usize {
        self.text[from..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| from + i)
            .unwrap_or(self.text.len())
    }

    fn line_starts(&self) -> Vec<usize> {
        iter::once(0)
            .chain(
                self.text
                    .char_indices()
                    .filter(|&(_, ch)| ch == '\n')
                    .map(|(i, _)| i + 1),
            )
            .collect()
    }

    fn current_line(&self, starts: &[usize]) -> usize {
        starts
            .iter()
            .rposition(|&start| start <= self.cursor)
            .unwrap_or(0)
    }

    fn cursor_at_column(&self, starts: &[usize], line: usize, column: usize) -> usize {
        let start = starts[line];
        let end = starts
            .get(line + 1)
            .map_or(self.text.len(), |next| next - 1);
        self.text[start..end]
            .char_indices()
            .nth(column)
            .map_or(end, |(offset, _)| start + offset)
    }

    #[cfg(test)]
    fn new_in_memory() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            revision: 0,
            anchor: None,
            history: History::in_memory(),
            edits: Undo::default(),
        }
    }
}

/// What a word-wise cursor motion treats alike: it crosses a run of one class.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Word,
    Punctuation,
    Space,
}

impl CharClass {
    fn of(character: char) -> Self {
        if character.is_whitespace() {
            Self::Space
        } else if character.is_alphanumeric() || character == '_' || character == '\'' {
            Self::Word
        } else {
            Self::Punctuation
        }
    }
}

/// The live source field and its transient presentation state.
pub struct InputCode {
    field: InputTextField,
    hint: Hint,
    completion: Option<Completion>,
    /// The revision and cursor the completion was offered for; once either
    /// moves it no longer applies.
    completed_for: Option<(u64, usize)>,
    settings: InputSettings,
    diagnostic: Option<String>,
    diagnostic_range: Option<Range<usize>>,
    /// The command typed so far, drawn in the command colour.
    command_range: Option<Range<usize>>,
    selecting: bool,
    highlighted_revision: u64,
    semantic_revision: u64,
    highlighted_pending: Option<String>,
    highlights: Vec<HLSpan>,
    area: Rect,
    text_area: Rect,
    scroll: usize,
    pinned_first_row: bool,
    /// When a keystroke last changed the text, while the typing prompt shows.
    typed_at: Option<Instant>,
}

/// Transient presentation inputs supplied by the containing editor.
pub struct InputContext<'a> {
    /// Width, prefix, and resolved highlight groups.
    pub render: &'a RenderContext,
    /// Parser-backed semantic highlighter.
    pub highlighter: &'a mut dyn Highlighter,
    /// Whether the source currently names a command.
    pub command: bool,
    /// Whether the backend is inside the debugger.
    pub debugging: bool,
    /// Whether this input is the focused UI element.
    pub focused: bool,
    /// Incomplete source whose following line is the live input.
    pub pending: Option<&'a str>,
}

impl InputCode {
    pub fn new() -> Self {
        Self {
            field: InputTextField::new(),
            hint: Hint::default(),
            completion: None,
            completed_for: None,
            settings: InputSettings::default(),
            diagnostic: None,
            diagnostic_range: None,
            command_range: None,
            selecting: false,
            highlighted_revision: u64::MAX,
            semantic_revision: u64::MAX,
            highlighted_pending: None,
            highlights: Vec::new(),
            area: Rect::ZERO,
            text_area: Rect::ZERO,
            scroll: 0,
            pinned_first_row: false,
            typed_at: None,
        }
    }

    pub fn field(&self) -> &InputTextField {
        &self.field
    }

    pub fn field_mut(&mut self) -> &mut InputTextField {
        &mut self.field
    }

    fn edit_state(&self) -> (u64, usize) {
        (self.field.revision, self.field.cursor)
    }

    /// Whether the text or cursor moved since a completion was last offered.
    pub fn needs_completion(&self) -> bool {
        self.completed_for != Some(self.edit_state())
    }

    /// Offer `completion` for the text and cursor as they stand.
    pub fn offer(&mut self, completion: Option<Completion>) {
        self.completion = completion;
        self.completed_for = Some(self.edit_state());
    }

    /// Drop the offered completion, so the next one is computed afresh.
    pub fn forget_completion(&mut self) {
        self.completed_for = None;
    }

    /// The completion offered for the text and cursor as they stand.
    pub fn offered(&self) -> Option<&Completion> {
        self.completion
            .as_ref()
            .filter(|_| !self.needs_completion() && self.field.selection().is_none())
    }

    /// Return the component's adjustable layout policy.
    pub const fn settings(&self) -> InputSettings {
        self.settings
    }

    /// Replace the component's adjustable layout policy.
    pub fn set_settings(&mut self, settings: InputSettings) {
        self.settings = settings;
    }

    pub fn accept_submission(&mut self) {
        self.field.accept_submission();
        self.hint = Hint::default();
        self.diagnostic = None;
        self.diagnostic_range = None;
    }

    pub fn set_diagnostic(&mut self, diagnostic: String, range: Option<Range<usize>>) {
        self.diagnostic = Some(diagnostic);
        self.diagnostic_range = range;
    }

    /// Mark the part of the text that is a command being typed, if any is.
    pub fn set_command_range(&mut self, range: Option<Range<usize>>) {
        self.command_range = range;
    }

    pub fn clear_diagnostic(&mut self) {
        self.diagnostic = None;
        self.diagnostic_range = None;
    }

    pub fn clear(&mut self) {
        self.field.clear();
        self.clear_diagnostic();
    }

    /// Replace the live draft with source transferred back from the feed.
    pub fn restore(&mut self, source: String) {
        self.field.replace_text(source);
        self.clear_diagnostic();
    }

    /// Apply one input-owned editing operation.
    pub fn edit(&mut self, action: InputAction) {
        use InputAction::*;
        if matches!(
            action,
            InsertNewline
                | InsertTab
                | DeleteBack
                | DeleteForward
                | DeleteWordBack
                | DeleteWordForward
                | Undo
                | Redo
                | Clear
        ) {
            self.clear_diagnostic();
        }
        if action == Clear {
            return self.clear();
        }
        if matches!(action, NextCompletion | PreviousCompletion) {
            return self.complete(action);
        }
        let field = &mut self.field;
        if !matches!(action, MoveUp | MoveDown | HistoryPrevious | HistoryNext) {
            field.history.stop_browsing();
        }
        let browse = field.history.browsing(&field.text) || !field.text.contains('\n');
        match action {
            InsertNewline => field.insert_char('\n'),
            InsertTab => field.insert_char('\t'),
            DeleteBack => field.backspace(),
            DeleteForward => field.delete_forward(),
            DeleteWordBack => field.delete_word_back(),
            DeleteWordForward => field.delete_word_forward(),
            Undo => field.undo(),
            Redo => field.redo(),
            MoveLeft => field.move_left(),
            MoveRight => field.move_right(),
            MoveWordLeft => field.move_word_left(),
            MoveWordRight => field.move_word_right(),
            MoveLineStart => field.move_home(),
            MoveLineEnd => field.move_end(),
            MoveTextStart => field.move_text_start(),
            MoveTextEnd => field.move_text_end(),
            HistoryPrevious => field.history_prev(),
            HistoryNext => field.history_next(),
            MoveUp if browse && !field.text[..field.cursor].contains('\n') => field.history_prev(),
            MoveUp => field.move_up(),
            MoveDown if browse && !field.text[field.cursor..].contains('\n') => {
                field.history_next()
            }
            MoveDown => field.move_down(),
            SelectLeft => field.select_left(),
            SelectRight => field.select_right(),
            SelectWordLeft => field.select_word_left(),
            SelectWordRight => field.select_word_right(),
            SelectLineStart => field.select_home(),
            SelectLineEnd => field.select_end(),
            SelectTextStart => field.select_text_start(),
            SelectTextEnd => field.select_text_end(),
            Clear | NextCompletion | PreviousCompletion => unreachable!(),
        }
    }

    /// Take the offered completion in place of the typed text; taking again
    /// cycles through the candidates instead of completing further.
    fn complete(&mut self, action: InputAction) {
        if self.offered().is_none() {
            return;
        }
        let Some(completion) = self.completion.as_mut() else {
            return;
        };
        let (range, chosen) = completion.take(action == InputAction::NextCompletion);
        let chosen = chosen.to_string();
        self.field.replace(range, &chosen);
        self.completed_for = Some(self.edit_state());
    }

    pub fn revision(&self) -> u64 {
        self.field.revision
    }

    /// Show the typing prompt if a keystroke changed the text since `revision`.
    pub fn note_keystroke_since(&mut self, revision: u64) {
        if self.field.revision != revision {
            self.typed_at = Some(Instant::now());
        }
    }

    /// How much longer the typing prompt shows, if a keystroke changed the text
    /// recently enough for it.
    pub fn typing_remaining(&self) -> Option<Duration> {
        Duration::from_millis(UI.typing_prompt_ms).checked_sub(self.typed_at?.elapsed())
    }

    fn typing(&self) -> bool {
        self.typing_remaining().is_some()
    }

    /// Return to the resting prompt once typing has paused; whether it did.
    pub fn settle_typing(&mut self) -> bool {
        let settled = self.typed_at.is_some() && !self.typing();
        if settled {
            self.typed_at = None;
        }
        settled
    }

    /// Interpret one unbound terminal key as editor input.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<char> {
        let revision = self.revision();
        let typed = self.interpret_key(key);
        self.note_keystroke_since(revision);
        typed
    }

    fn interpret_key(&mut self, key: KeyEvent) -> Option<char> {
        use InputAction::*;
        let action = match key.code {
            KeyCode::Char('l') if key.ctrl() => Some(Clear),
            KeyCode::Char('z' | 'Z') if key.ctrl() && key.shift() => Some(Redo),
            KeyCode::Char('z') if key.ctrl() => Some(Undo),
            // The backspace control character, which is what a terminal sends
            // for Ctrl+Backspace when it cannot report the modifier.
            KeyCode::Char('h') if key.ctrl() => Some(DeleteWordBack),
            KeyCode::Backspace if key.ctrl() || key.alt() => Some(DeleteWordBack),
            KeyCode::Delete if key.ctrl() || key.alt() => Some(DeleteWordForward),
            KeyCode::Char(' ') if key.shift() => Some(InsertNewline),
            KeyCode::Enter if key.shift() || key.alt() => Some(InsertNewline),
            // The newline control character itself, which is what a terminal
            // sends for Shift+Enter when it cannot report the modifier.
            KeyCode::Char('j') if key.ctrl() => Some(InsertNewline),
            KeyCode::Tab if key.alt() => Some(InsertTab),
            KeyCode::Tab => Some(NextCompletion),
            KeyCode::BackTab => Some(PreviousCompletion),
            KeyCode::Backspace => Some(DeleteBack),
            KeyCode::Delete => Some(DeleteForward),
            KeyCode::Left if key.ctrl() && key.shift() => Some(SelectWordLeft),
            KeyCode::Right if key.ctrl() && key.shift() => Some(SelectWordRight),
            KeyCode::Home if key.ctrl() && key.shift() => Some(SelectTextStart),
            KeyCode::End if key.ctrl() && key.shift() => Some(SelectTextEnd),
            KeyCode::Left if key.ctrl() => Some(MoveWordLeft),
            KeyCode::Right if key.ctrl() => Some(MoveWordRight),
            KeyCode::Home if key.ctrl() => Some(MoveTextStart),
            KeyCode::End if key.ctrl() => Some(MoveTextEnd),
            KeyCode::Left if key.shift() => Some(SelectLeft),
            KeyCode::Right if key.shift() => Some(SelectRight),
            KeyCode::Home if key.shift() => Some(SelectLineStart),
            KeyCode::End if key.shift() => Some(SelectLineEnd),
            KeyCode::Left => Some(MoveLeft),
            KeyCode::Right => Some(MoveRight),
            KeyCode::Up if key.ctrl() => Some(HistoryPrevious),
            KeyCode::Down if key.ctrl() => Some(HistoryNext),
            KeyCode::Up => Some(MoveUp),
            KeyCode::Down => Some(MoveDown),
            KeyCode::Home => Some(MoveLineStart),
            KeyCode::End => Some(MoveLineEnd),
            KeyCode::Char(character) if key.is_text() => {
                self.clear_diagnostic();
                self.field.insert_char(character);
                return Some(character);
            }
            _ => None,
        };
        if let Some(action) = action {
            self.edit(action);
        }
        None
    }

    /// Insert pasted source as one input-owned edit. Terminals send a pasted line
    /// break as `\r` or `\r\n`; the input breaks lines only at `\n`.
    pub fn insert(&mut self, text: &str) {
        self.clear_diagnostic();
        self.field.insert_str(&typeable(text));
    }

    /// Return whether a pointer event lies over the last rendered input pane.
    pub fn contains(&self, event: MouseEvent) -> bool {
        self.area.contains(Position::new(event.column, event.row))
    }

    /// Handle mouse selection in the last rendered text rectangle.
    pub fn handle_mouse(&mut self, event: MouseEvent) -> (bool, Option<String>) {
        let (area, scroll) = (self.text_area, self.scroll);
        if area.is_empty() {
            self.selecting = false;
            return (false, None);
        }
        let point = Position::new(event.column, event.row);
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) if area.contains(point) => {
                self.selecting = true;
                self.field
                    .start_selection(self.cursor_at_mouse(event, area, scroll));
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
                if self.selecting =>
            {
                self.field
                    .continue_selection(self.cursor_at_mouse(event, area, scroll));
                if matches!(event.kind, MouseEventKind::Up(_)) {
                    self.selecting = false;
                    return (true, self.field.selected_text().map(str::to_string));
                }
            }
            _ => return (false, None),
        }
        (true, None)
    }

    fn cursor_at_mouse(&self, event: MouseEvent, area: Rect, scroll: usize) -> usize {
        let row = event.row.saturating_sub(area.y).min(area.height - 1);
        let column = event.column.saturating_sub(area.x).min(area.width) as usize;
        let source_row = if self.pinned_first_row {
            if row < 2 {
                0
            } else {
                scroll + usize::from(row - 2)
            }
        } else {
            scroll + usize::from(row)
        };
        self.cursor_at_visual(area.width, source_row, column)
    }

    #[cfg(test)]
    pub fn new_in_memory() -> Self {
        Self {
            field: InputTextField::new_in_memory(),
            hint: Hint::default(),
            completion: None,
            completed_for: None,
            settings: InputSettings::default(),
            diagnostic: None,
            diagnostic_range: None,
            command_range: None,
            selecting: false,
            highlighted_revision: u64::MAX,
            semantic_revision: u64::MAX,
            highlighted_pending: None,
            highlights: Vec::new(),
            area: Rect::ZERO,
            text_area: Rect::ZERO,
            scroll: 0,
            pinned_first_row: false,
            typed_at: None,
        }
    }

    /// Return the source currently being edited.
    pub fn text(&self) -> &str {
        self.field.text()
    }

    /// Render the live source with fresh syntax highlighting.
    pub fn highlighted_lines(
        &mut self,
        context: &RenderContext,
        highlighter: &mut dyn Highlighter,
        debugging: bool,
        pending: Option<&str>,
    ) -> Vec<Line<'static>> {
        if self.semantic_revision != highlighter.revision()
            || self.highlighted_revision != self.field.revision
            || self.highlighted_pending.as_deref() != pending
        {
            self.highlights = highlighter.highlight_continuation(pending, self.field.text());
            self.highlighted_revision = self.field.revision;
            self.semantic_revision = highlighter.revision();
            self.highlighted_pending = pending.map(str::to_string);
        }
        let selection = self.field.selection();
        let theme = context.groups();
        let base = theme.code();
        // In the debugger the prompt changes colour, so a nested M2 interpreter is
        // never mistaken for the ordinary one.
        let prompt = if debugging {
            theme.warning().patch(base)
        } else {
            theme.prompt().patch(base)
        };
        let text = self.text();
        let width = context.width().saturating_sub(context.prefix());
        let visual_lines = self.lines(width);
        let ghost = self.offered().map(|completion| {
            let (row, _) = self.visual_cursor(width);
            let offset = self.field.cursor() - visual_lines[row].range.start;
            (row, offset, completion.ghost().to_string())
        });

        let mut lines = Vec::new();
        for (index, visual_line) in visual_lines.iter().enumerate() {
            let marker = if index + 1 != visual_lines.len() {
                ""
            } else if self.typing() {
                UI.glyphs.prompt_typing.current()
            } else {
                UI.glyphs.prompt.current()
            };
            let mut spans = Prompt::new(marker, prompt).spans(&());
            let mut line = self.highlights.paint(
                visual_line.range.start,
                &text[visual_line.range.clone()],
                base,
                theme,
            );
            if let Some(diagnostic) = self.diagnostic_range.as_ref() {
                line = apply_selection(
                    line,
                    visual_line.range.start,
                    diagnostic,
                    theme.diagnostic(),
                );
            }
            if let Some(command) = self.command_range.as_ref() {
                line = apply_selection(line, visual_line.range.start, command, theme.command());
            }
            if let Some(selection) = selection.as_ref() {
                line = apply_selection(line, visual_line.range.start, selection, theme.selection());
            }
            if let Some((_, offset, ghost)) = ghost.as_ref().filter(|(row, ..)| *row == index) {
                line = insert_span(
                    line,
                    *offset,
                    Span::styled(ghost.clone(), theme.completion_hint()),
                );
            }
            spans.extend(line);
            if index == 0 && text.is_empty() {
                spans.push(Span::styled(
                    self.hint.to_string(),
                    theme.placeholder_hint(),
                ));
            }
            lines.push(Line::from(spans));
        }
        lines
    }

    /// Height needed by the input within its configured share of the editor.
    pub fn height(&self, width: u16, prefix: u16, editor_height: u16) -> u16 {
        let rows = self.visual_rows(width.saturating_sub(prefix)) as u16;
        let chrome = 2 + u16::from(self.diagnostic.is_some());
        let cap = editor_height
            .saturating_mul(self.settings.max_height_percent.into())
            .saturating_div(100)
            .max(UI.min_input_height);
        rows.saturating_add(chrome).clamp(UI.min_input_height, cap)
    }

    /// Render the input locally and return its terminal cursor when focused.
    pub fn render(
        &mut self,
        context: InputContext<'_>,
        area: Rect,
        buffer: &mut Buffer,
    ) -> Option<Position> {
        self.area = area;
        let diagnostic = self.diagnostic.clone();
        let groups = context.render.groups();
        let code = groups.code();
        let text_width = area.width.saturating_sub(context.render.prefix());
        let mut rows = self.highlighted_lines(
            context.render,
            context.highlighter,
            context.debugging,
            context.pending,
        );
        if context.command {
            for span in rows
                .iter_mut()
                .flat_map(|line| line.spans.iter_mut().skip(1))
            {
                span.style = span.style.patch(groups.prompt());
            }
        }

        let viewport = area
            .height
            .saturating_sub(2 + u16::from(diagnostic.is_some()))
            .max(1) as usize;
        let (cursor_row, cursor_column) = self.visual_cursor(text_width);
        let clipped = rows.len() > viewport;
        self.pinned_first_row = clipped && viewport >= 3;
        let pinned_rows = if self.pinned_first_row { 2 } else { 0 };
        let body_height = viewport - pinned_rows;
        self.scroll = self.scroll_start(cursor_row, rows.len(), body_height);
        if self.pinned_first_row {
            self.scroll = self.scroll.max(1);
        }
        self.text_area = Rect::new(
            area.x + context.render.prefix(),
            area.y + 1,
            text_width,
            viewport as u16,
        );

        let mut lines = if context.debugging {
            let warning = groups.warning().bold().patch(code);
            vec![Line::from(vec![
                Prompt::new("", warning).spans(&())[0].clone(),
                Span::styled("DEBUGGER", warning),
            ])]
        } else {
            vec![Line::default()]
        };
        if self.pinned_first_row {
            lines.push(rows[0].clone());
            lines.push(Line::from(Span::styled(
                UI.glyphs
                    .pending_separator
                    .current()
                    .repeat(usize::from(area.width)),
                groups.pending_separator().patch(code),
            )));
        }
        lines.extend(rows.into_iter().skip(self.scroll).take(body_height));
        if let Some(message) = diagnostic {
            let error = groups.error().patch(code);
            lines.push(Line::from(vec![
                Prompt::new("", error).spans(&())[0].clone(),
                Span::styled(message, error),
            ]));
        }
        lines.push(Line::default());
        Paragraph::new(lines).style(code).render(area, buffer);
        if clipped && !self.pinned_first_row && area.height > 1 {
            for x in area.x..area.right() {
                buffer[(x, area.bottom() - 1)]
                    .set_symbol(UI.glyphs.pending_separator.current())
                    .set_style(groups.pending_separator().patch(code));
            }
        }
        context.focused.then(|| {
            Position::new(
                area.x + context.render.prefix() + cursor_column as u16,
                area.y
                    + 1
                    + if self.pinned_first_row && cursor_row == 0 {
                        0
                    } else {
                        (pinned_rows + cursor_row.saturating_sub(self.scroll)) as u16
                    },
            )
        })
    }

    /// Forget the pane rectangle while another view is displayed.
    pub fn hide(&mut self) {
        self.area = Rect::ZERO;
        self.text_area = Rect::ZERO;
    }

    fn scroll_start(&self, cursor: usize, rows: usize, viewport: usize) -> usize {
        let last = rows.saturating_sub(viewport);
        let scrolloff = usize::from(self.settings.scrolloff);
        if viewport <= 2 * scrolloff + 1 {
            return cursor.saturating_sub(viewport / 2).min(last);
        }
        self.scroll
            .clamp(
                (cursor + scrolloff + 1).saturating_sub(viewport),
                cursor.saturating_sub(scrolloff),
            )
            .min(last)
    }
}

/// `text` as the input keeps it: line breaks as `\n`, without the control
/// characters that would reach the terminal as commands, or picture placeholders
/// copied from a terminal, which draw pictures in place of text.
fn typeable(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut kept = String::with_capacity(text.len());
    let mut in_placeholder = false;
    for character in text.chars() {
        // The marks after a placeholder number the piece of picture it shows.
        in_placeholder = character == PLACEHOLDER
            || in_placeholder && character.width() == Some(0) && !character.is_control();
        let control = character.is_control() && !matches!(character, '\n' | '\t');
        if !in_placeholder && !control {
            kept.push(character);
        }
    }
    kept
}

/// `inserted` placed `offset` bytes into a row, splitting the span it falls in.
fn insert_span(
    spans: Vec<Span<'static>>,
    offset: usize,
    inserted: Span<'static>,
) -> Vec<Span<'static>> {
    let mut placed = Vec::with_capacity(spans.len() + 2);
    let mut start = 0;
    let mut inserted = Some(inserted);
    for span in spans {
        let end = start + span.content.len();
        if (start..end).contains(&offset) && offset > start {
            let content = span.content.into_owned();
            placed.push(Span::styled(
                content[..offset - start].to_string(),
                span.style,
            ));
            placed.extend(inserted.take());
            placed.push(Span::styled(
                content[offset - start..].to_string(),
                span.style,
            ));
        } else {
            if offset == start {
                placed.extend(inserted.take());
            }
            placed.push(span);
        }
        start = end;
    }
    placed.extend(inserted);
    placed
}

/// Repaint the part of a row covered by `selection`, splitting spans at the
/// boundaries so the highlight follows byte ranges rather than whole spans and
/// syntax colouring survives underneath.
fn apply_selection(
    spans: Vec<Span<'static>>,
    row_start: usize,
    selection: &Range<usize>,
    style: Style,
) -> Vec<Span<'static>> {
    let mut painted = Vec::with_capacity(spans.len());
    let mut offset = row_start;
    for span in spans {
        let content = span.content.into_owned();
        let end = offset + content.len();
        let start = selection.start.max(offset);
        let stop = selection.end.min(end);
        if start >= stop {
            painted.push(Span::styled(content, span.style));
        } else {
            let (head, tail) = (start - offset, stop - offset);
            if head > 0 {
                painted.push(Span::styled(content[..head].to_string(), span.style));
            }
            painted.push(Span::styled(
                content[head..tail].to_string(),
                span.style.patch(style),
            ));
            if tail < content.len() {
                painted.push(Span::styled(content[tail..].to_string(), span.style));
            }
        }
        offset = end;
    }
    painted
}

impl TextLayout for InputCode {
    type Wrapping = HardWrap;

    fn text(&self) -> &str {
        self.field.text()
    }
}

impl InputCode {
    /// The cursor's position in *visual* rows and columns for a text area of
    /// `width`, which is what the terminal cursor needs once lines wrap. A cursor
    /// sitting exactly on a wrap boundary belongs to the row it continues onto,
    /// so typing carries on where the caret is shown; one sitting before a
    /// newline stays at the end of its own row, since the next row begins past
    /// the newline rather than at the cursor.
    pub fn visual_cursor(&self, width: u16) -> (usize, usize) {
        let text = self.text();
        let cursor = self.field.cursor();
        let rows = HardWrap.wrap(text, width);
        let row = rows
            .iter()
            .rposition(|row| row.range.start <= cursor)
            .unwrap_or(0);
        let column = text[rows[row].range.start..cursor].chars().count();
        (row, column)
    }

    /// How many visual rows the live source occupies at `width`.
    pub fn visual_rows(&self, width: u16) -> usize {
        self.lines(width).len()
    }

    pub fn cursor_at_visual(&self, width: u16, row: usize, column: usize) -> usize {
        let rows = HardWrap.wrap(self.text(), width);
        let line = &rows[row.min(rows.len().saturating_sub(1))];
        self.text()[line.range.clone()]
            .char_indices()
            .nth(column)
            .map_or(line.range.end, |(offset, _)| line.range.start + offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::highlight::SyntaxHighlighter;
    use crate::ui::config::UI;
    use crate::ui::controller::settings::theme::groups::Groups;
    use crate::ui::key::KeyMod::{self, Alt, Ctrl, Shift};
    use ratatui::style::Modifier;

    #[test]
    fn editor_owns_modified_newlines_and_tabs() {
        let mut input = InputCode::new_in_memory();
        for key in [
            KeyCode::Char(' ') + Shift,
            KeyCode::Enter + Shift,
            KeyCode::Enter + Alt,
            KeyCode::Tab + Alt,
        ] {
            input.handle_key(key);
        }

        assert_eq!(input.text(), "\n\n\n\t");
    }

    #[test]
    fn pasted_lines_grow_the_input_whatever_line_endings_the_terminal_sends() {
        let rows = |input: &InputCode| input.height(80, UI.prompt_width(), 100);
        let single = rows(&InputCode::new_in_memory());
        for pasted in ["a\nb\nc", "a\rb\rc", "a\r\nb\r\nc"] {
            let mut input = InputCode::new_in_memory();
            input.insert(pasted);

            assert_eq!(input.text(), "a\nb\nc", "{pasted:?}");
            assert!(rows(&input) > single, "{pasted:?}");
        }
    }

    #[test]
    fn a_paste_keeps_text_but_not_terminal_commands_or_picture_placeholders() {
        let placeholder = format!("{PLACEHOLDER}\u{305}\u{30D}");
        let pasted = format!("a\tb{placeholder}{placeholder}\x1b[2Jc\u{301}\r\nd");
        let mut input = InputCode::new_in_memory();
        input.insert(&pasted);

        assert_eq!(input.text(), "a\tb[2Jc\u{301}\nd");
    }

    #[test]
    fn a_long_paste_grows_the_input_to_its_max_height() {
        let mut input = InputCode::new_in_memory();
        let editor_height = 100;
        let cap = editor_height * u16::from(input.settings().max_height_percent) / 100;
        input.insert(&vec!["f(x)"; usize::from(editor_height)].join("\n"));

        assert_eq!(input.height(80, UI.prompt_width(), editor_height), cap);
    }

    #[test]
    fn ctrl_arrows_jump_a_word_and_ctrl_home_end_the_whole_input() {
        let mut input = InputCode::new_in_memory();
        input.insert("R = ZZ[x, y]\nf(a)");
        let key = |code: KeyCode, modifiers: &[KeyMod]| {
            modifiers
                .iter()
                .fold(KeyEvent::from(code), |key, modifier| key + *modifier)
        };
        let word = |input: &mut InputCode, code, modifiers: &[KeyMod]| {
            input.handle_key(key(code, modifiers));
            input.text()[..input.field().cursor()].to_string()
        };

        assert_eq!(
            word(&mut input, KeyCode::Left, &[Ctrl]),
            "R = ZZ[x, y]\nf(a"
        );
        assert_eq!(word(&mut input, KeyCode::Left, &[Ctrl]), "R = ZZ[x, y]\nf(");
        assert_eq!(word(&mut input, KeyCode::Left, &[Ctrl]), "R = ZZ[x, y]\nf");
        assert_eq!(word(&mut input, KeyCode::Left, &[Ctrl]), "R = ZZ[x, y]\n");
        assert_eq!(word(&mut input, KeyCode::Left, &[Ctrl]), "R = ZZ[x, y");
        assert_eq!(word(&mut input, KeyCode::Left, &[Ctrl]), "R = ZZ[x, ");
        assert_eq!(word(&mut input, KeyCode::Home, &[Ctrl]), "");
        assert_eq!(word(&mut input, KeyCode::Right, &[Ctrl]), "R");
        assert_eq!(word(&mut input, KeyCode::Right, &[Ctrl]), "R =");
        assert_eq!(word(&mut input, KeyCode::Right, &[Ctrl]), "R = ZZ");
        assert_eq!(word(&mut input, KeyCode::Right, &[Ctrl]), "R = ZZ[");
        input.handle_key(key(KeyCode::Right, &[Ctrl, Shift]));
        assert_eq!(
            input.field().selection().map(|range| &input.text()[range]),
            Some("x")
        );
        assert_eq!(
            word(&mut input, KeyCode::End, &[Ctrl]),
            "R = ZZ[x, y]\nf(a)"
        );
    }

    #[test]
    fn ctrl_deletion_removes_what_the_ctrl_arrow_crosses() {
        let source = "R = ZZ[x, y]\nf(a)  ";
        let (mut moved, mut deleted) = (InputCode::new_in_memory(), InputCode::new_in_memory());
        moved.insert(source);
        deleted.insert(source);
        while !deleted.text().is_empty() {
            moved.handle_key(KeyCode::Left + Ctrl);
            deleted.handle_key(KeyCode::Backspace + Ctrl);
            assert_eq!(deleted.text(), &moved.text()[..moved.field().cursor()]);
        }

        deleted.insert(source);
        deleted.field_mut().move_text_start();
        moved.field_mut().move_text_start();
        while !deleted.text().is_empty() {
            moved.handle_key(KeyCode::Right + Ctrl);
            deleted.handle_key(KeyCode::Delete + Ctrl);
            assert_eq!(deleted.text(), &moved.text()[moved.field().cursor()..]);
        }
    }

    #[test]
    fn undo_restores_deletions_and_typing_a_word_at_a_time() {
        let mut input = InputCode::new_in_memory();
        for character in "f = x + y".chars() {
            input.handle_key(KeyCode::Char(character).into());
        }
        input.handle_key(KeyCode::Backspace + Ctrl);
        input.handle_key(KeyCode::Backspace + Ctrl);
        input.handle_key(KeyCode::Char('l') + Ctrl);
        assert_eq!(input.text(), "");

        input.handle_key(KeyCode::Char('z') + Ctrl);
        assert_eq!(input.text(), "f = x ");
        input.handle_key(KeyCode::Char('z') + Ctrl);
        assert_eq!(input.text(), "f = x + y", "a run of deletions is one step");
        assert_eq!(input.field().cursor(), input.text().len());
        input.handle_key(KeyCode::Char('z') + Ctrl);
        assert_eq!(input.text(), "f = x + ");
        input.handle_key(KeyCode::Char('z') + Ctrl);
        assert_eq!(input.text(), "f = x ");

        input.handle_key(KeyCode::Char('z') + Ctrl + Shift);
        input.handle_key(KeyCode::Char('z') + Ctrl + Shift);
        assert_eq!(input.text(), "f = x + y");
        input.handle_key(KeyCode::Char('!').into());
        input.handle_key(KeyCode::Char('z') + Ctrl + Shift);
        assert_eq!(
            input.text(),
            "f = x + y!",
            "a new edit drops what was undone"
        );
    }

    #[test]
    fn a_submission_leaves_nothing_to_undo() {
        let mut input = InputCode::new_in_memory();
        input.insert("1 + 1");
        input.accept_submission();

        input.handle_key(KeyCode::Char('z') + Ctrl);

        assert_eq!(input.text(), "");
    }

    #[test]
    fn the_newline_character_a_terminal_sends_for_shift_enter_is_a_newline() {
        let mut input = InputCode::new_in_memory();
        input.insert("1");
        input.handle_key(KeyCode::Char('j') + Ctrl);

        assert_eq!(input.text(), "1\n");
    }

    #[test]
    fn shifted_movement_selects_and_typing_replaces_it() {
        let mut field = InputTextField::new_in_memory();
        field.insert_str("abcdef");
        field.move_home();

        field.select_right();
        field.select_right();
        assert_eq!(field.selected_text(), Some("ab"));

        field.insert_char('X');
        assert_eq!(field.text(), "Xcdef");
        assert!(field.selection().is_none(), "typing consumes the selection");
    }

    // The anchor indexes the buffer it was set in, so replacing the buffer has to
    // drop it -- otherwise it points past the new end and the next edit or copy
    // slices out of bounds.
    #[test]
    fn a_replaced_buffer_carries_no_selection() {
        let mut input = InputCode::new_in_memory();
        input.field_mut().insert_str("abc");
        input.field_mut().select_home();

        input.accept_submission();
        input.field_mut().insert_char('x');

        assert_eq!(input.text(), "x");
    }

    #[test]
    fn recalled_history_carries_no_selection() {
        let mut field = InputTextField::new_in_memory();
        field.history.record("q");
        field.insert_str("abcdef");
        field.select_home();

        field.history_prev();

        assert_eq!(field.text(), "q");
        assert_eq!(field.selected_text(), None);
    }

    #[test]
    fn history_browsing_crosses_multiline_entries_until_cursor_movement() {
        let mut input = InputCode::new_in_memory();
        input.field.history.record("older\nentry");
        input.field.history.record("newer\nentry");
        input.edit(InputAction::MoveUp);
        assert_eq!(input.text(), "newer\nentry");
        input.edit(InputAction::MoveUp);
        assert_eq!(input.field.cursor_line_col().0, 0);
        assert_eq!(input.text(), "newer\nentry");
        input.edit(InputAction::MoveUp);
        assert_eq!(input.text(), "older\nentry");
        input.edit(InputAction::MoveDown);
        assert_eq!(input.text(), "newer\nentry");

        input.edit(InputAction::MoveTextStart);
        input.edit(InputAction::MoveUp);
        assert_eq!(input.text(), "newer\nentry");
        assert_eq!(input.field.cursor, 0);
        input.edit(InputAction::MoveDown);
        assert_eq!(input.field.cursor_line_col(), (1, 0));
    }

    #[test]
    fn editing_recalled_multiline_input_stops_browsing() {
        let mut input = InputCode::new_in_memory();
        input.field.history.record("older\nentry");
        input.field.history.record("newer\nentry");
        input.edit(InputAction::MoveUp);
        input.field.insert_char('!');
        input.edit(InputAction::MoveUp);
        assert_eq!(input.text(), "newer\nentry!");
        assert_eq!(input.field.cursor_line_col().0, 0);
        input.edit(InputAction::MoveUp);
        assert_eq!(input.text(), "newer\nentry!");
    }

    #[test]
    fn clipped_input_pins_its_first_row_above_a_separator() {
        let mut input = InputCode::new_in_memory();
        input
            .field
            .insert_str("first\nsecond\nthird\nfourth\nfifth\nlast");
        let context = RenderContext::new(40, UI.prompt_width());
        let mut highlighter = SyntaxHighlighter::new();
        let area = Rect::new(0, 0, 40, 7);
        let mut buffer = Buffer::empty(area);
        let cursor = input
            .render(
                InputContext {
                    render: &context,
                    highlighter: &mut highlighter,
                    command: false,
                    debugging: false,
                    focused: true,
                    pending: None,
                },
                area,
                &mut buffer,
            )
            .unwrap();
        let row = |y| (0..40).map(|x| buffer[(x, y)].symbol()).collect::<String>();
        assert!(row(1).contains("first"));
        assert!(row(2)
            .chars()
            .all(|symbol| symbol.to_string() == UI.glyphs.pending_separator.current()));
        assert!(row(cursor.y).contains("last"));
        assert!(cursor.y < area.bottom() - 1);
        assert!(input.pinned_first_row);
        assert!(input.scroll > 1);
    }

    #[test]
    fn returning_from_history_restores_the_draft_cursor() {
        let mut field = InputTextField::new_in_memory();
        field.history.record("top\nbottom + 1");
        field.history.record("unrelated input");
        field.insert_str("top\nbottom");
        field.cursor = 0;

        field.history_prev();
        assert_eq!(field.text(), "top\nbottom + 1");
        field.history_next();

        assert_eq!(field.text(), "top\nbottom");
        assert_eq!(field.cursor_line_col(), (0, 0));
    }

    #[test]
    fn multiline_history_without_a_match_preserves_the_draft() {
        let mut field = InputTextField::new_in_memory();
        field.history.record("top\nother");
        field.history.record("unrelated input");
        field.insert_str("top\nbottom");
        field.cursor = 0;

        field.history_prev();

        assert_eq!(field.text(), "top\nbottom");
        assert_eq!(field.cursor_line_col(), (0, 0));
    }

    // Selecting is a shifted gesture: any plain movement abandons it.
    #[test]
    fn plain_movement_clears_the_selection() {
        let mut field = InputTextField::new_in_memory();
        field.insert_str("abc");
        field.select_home();
        assert!(field.selection().is_some());

        field.move_right();
        assert!(field.selection().is_none());
    }

    #[test]
    fn deleting_a_selection_removes_all_of_it() {
        let mut field = InputTextField::new_in_memory();
        field.insert_str("keep this");
        field.select_home();

        field.backspace();
        assert_eq!(field.text(), "");
    }

    #[test]
    fn cursor_after_trailing_newline_is_on_next_line() {
        let mut field = InputTextField::new_in_memory();
        field.insert_str("abc");
        field.insert_char('\n');
        assert_eq!(field.cursor_line_col(), (1, 0));
    }

    #[test]
    fn cursor_within_second_line() {
        let mut field = InputTextField::new_in_memory();
        field.insert_str("ab\ncd");
        assert_eq!(field.cursor_line_col(), (1, 2));
    }

    #[test]
    fn up_moves_to_previous_line_at_same_column() {
        let mut field = InputTextField::new_in_memory();
        field.insert_str("ab\ncd");

        field.move_up();

        assert_eq!(field.cursor_line_col(), (0, 2));
    }

    #[test]
    fn vertical_movement_clamps_to_line_length_and_keeps_utf8_boundaries() {
        let mut field = InputTextField::new_in_memory();
        field.insert_str("αβ\nγδεζ");

        field.move_up();
        assert_eq!(field.cursor_line_col(), (0, 2));
        field.move_down();
        assert_eq!(field.cursor_line_col(), (1, 2));
    }

    #[test]
    fn cursor_on_single_line() {
        let mut field = InputTextField::new_in_memory();
        field.insert_str("abc");
        assert_eq!(field.cursor_line_col(), (0, 3));
    }

    #[test]
    fn multiline_prompt_glyph_is_on_last_line() {
        let mut input = InputCode::new_in_memory();
        input.field_mut().insert_str("first\nsecond\nthird");
        let context = RenderContext::new(40, UI.prompt_width());
        let mut highlighter = SyntaxHighlighter::new();

        let lines = input.highlighted_lines(&context, &mut highlighter, false, None);
        assert_eq!(lines.len(), 3);
        assert!(!lines[0].spans[0]
            .content
            .contains(UI.glyphs.prompt.current()));
        assert!(!lines[1].spans[0]
            .content
            .contains(UI.glyphs.prompt.current()));
        assert!(lines[2].spans[0]
            .content
            .contains(UI.glyphs.prompt.current()));
    }

    #[test]
    fn the_prompt_shows_typing_until_keystrokes_pause() {
        let mut input = InputCode::new_in_memory();
        let context = RenderContext::new(40, UI.prompt_width());
        let mut highlighter = SyntaxHighlighter::new();
        let mut prompt = |input: &mut InputCode| {
            let lines = input.highlighted_lines(&context, &mut highlighter, false, None);
            lines[0].spans[0].content.to_string()
        };
        let typing = UI.glyphs.prompt_typing.current();
        let resting = UI.glyphs.prompt.current();

        input.handle_key(KeyCode::Left.into());
        assert!(prompt(&mut input).contains(resting), "moving is not typing");

        input.handle_key(KeyCode::Char('x').into());
        assert!(prompt(&mut input).contains(typing));
        assert!(!input.settle_typing());

        input.typed_at = input
            .typed_at
            .map(|at| at - Duration::from_millis(UI.typing_prompt_ms));
        assert!(input.settle_typing());
        assert!(prompt(&mut input).contains(resting));
    }

    #[test]
    fn a_typing_prompt_keeps_the_text_in_place() {
        let resting = Prompt::new(UI.glyphs.prompt.current(), Style::default()).spans(&());
        let typing = Prompt::new(UI.glyphs.prompt_typing.current(), Style::default()).spans(&());
        assert_eq!(typing[0].width(), resting[0].width());
    }

    #[test]
    fn the_hint_of_an_empty_input_starts_at_the_cursor() {
        let mut input = InputCode::new_in_memory();
        let context = RenderContext::new(40, UI.prompt_width());
        let mut highlighter = SyntaxHighlighter::new();

        let rendered: String = input.highlighted_lines(&context, &mut highlighter, false, None)[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();

        let hint = rendered.find(&input.hint.to_string()).unwrap();
        assert_eq!(
            rendered[..hint].chars().count(),
            usize::from(UI.prompt_width())
        );
    }

    #[test]
    fn unchanged_input_reuses_its_semantic_highlights() {
        /// A highlighter recording how often parsing is requested.
        struct CountingHighlighter(usize);

        impl Highlighter for CountingHighlighter {
            fn highlight(&mut self, _source: &str) -> Vec<HLSpan> {
                self.0 += 1;
                Vec::new()
            }
        }

        let mut input = InputCode::new_in_memory();
        let context = RenderContext::new(40, UI.prompt_width());
        let mut highlighter = CountingHighlighter(0);

        input.highlighted_lines(&context, &mut highlighter, false, None);
        input.highlighted_lines(&context, &mut highlighter, false, None);
        assert_eq!(highlighter.0, 1);

        input.field_mut().insert_char('x');
        input.highlighted_lines(&context, &mut highlighter, false, None);
        assert_eq!(highlighter.0, 2);

        input.highlighted_lines(&context, &mut highlighter, false, Some("f("));
        assert_eq!(highlighter.0, 3);
    }

    #[test]
    fn diagnostic_special_colour_underlines_only_its_source_range() {
        struct EmptyHighlighter;

        impl Highlighter for EmptyHighlighter {
            fn highlight(&mut self, _source: &str) -> Vec<crate::ui::block::HLSpan> {
                Vec::new()
            }
        }

        let mut input = InputCode::new_in_memory();
        input.field_mut().insert_str("abc");
        input.set_diagnostic("bad b".to_string(), Some(1..2));
        let theme = Groups::default();
        let context = RenderContext::new(40, UI.prompt_width());
        let lines = input.highlighted_lines(&context, &mut EmptyHighlighter, false, None);
        let text = &lines[0].spans[1..];

        assert_eq!(text[0].content, "a");
        assert_eq!(text[1].content, "b");
        assert_eq!(text[2].content, "c");
        assert!(!text[0].style.add_modifier.contains(Modifier::UNDERLINED));
        assert!(text[1].style.add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(text[1].style.underline_color, theme.error().fg);
        assert_eq!(text[1].style.fg, text[0].style.fg);
        assert_eq!(text[1].style.bg, text[0].style.bg);
    }

    #[test]
    fn the_rest_of_a_completion_is_drawn_after_the_cursor_as_a_hint() {
        let mut input = InputCode::new_in_memory();
        input.field_mut().insert_str("f(ide)");
        input.field_mut().move_left();
        let candidates = vec!["ideal".to_string()];
        input.offer(Completion::new(input.text(), 2..5, candidates));
        let context = RenderContext::new(40, UI.prompt_width());

        let lines = input.highlighted_lines(&context, &mut SyntaxHighlighter::new(), false, None);
        let rendered: String = lines[0].spans[1..]
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(rendered, "f(ideal)");
        let ghost = lines[0]
            .spans
            .iter()
            .find(|span| span.content == "al")
            .unwrap();
        assert_eq!(ghost.style, Groups::default().completion_hint());

        input.handle_key(KeyCode::Tab.into());
        assert_eq!(input.text(), "f(ideal)");
        assert!(
            !input.needs_completion(),
            "taking cycles the candidates, it does not complete further"
        );
    }

    #[test]
    fn a_command_is_drawn_in_its_colour_and_its_completion_as_a_hint() {
        let mut input = InputCode::new_in_memory();
        input.field_mut().insert_str("$se");
        input.set_command_range(Some(0..3));
        let candidates = vec!["$settings".to_string()];
        input.offer(Completion::new(input.text(), 0..3, candidates));
        let context = RenderContext::new(40, UI.prompt_width());
        let groups = Groups::default();

        let lines = input.highlighted_lines(&context, &mut SyntaxHighlighter::new(), false, None);
        let style_of = |text: &str| {
            lines[0]
                .spans
                .iter()
                .find(|span| span.content == text)
                .unwrap_or_else(|| panic!("{text} is drawn on its own"))
                .style
        };
        assert_eq!(style_of("$se").fg, groups.command().fg);
        assert_eq!(style_of("ttings"), groups.completion_hint());
    }

    #[test]
    fn long_line_wraps_to_the_available_width() {
        let mut input = InputCode::new_in_memory();
        input.field_mut().insert_str("abcdefghij");

        // Three full rows of four, then the remainder.
        assert_eq!(input.visual_rows(4), 3);
        assert_eq!(input.visual_cursor(4), (2, 2));
    }

    #[test]
    fn cursor_on_a_wrap_boundary_sits_on_the_row_it_continues_onto() {
        let mut input = InputCode::new_in_memory();
        input.field_mut().insert_str("abcd");

        // Exactly one full row: the cursor moves on rather than hanging off the end.
        assert_eq!(input.visual_cursor(4), (1, 0));
    }

    #[test]
    fn cursor_before_a_newline_stays_on_its_own_row() {
        let mut input = InputCode::new_in_memory();
        input.field_mut().insert_str("ab\ncd");
        input.field_mut().move_up();
        input.field_mut().move_end();

        assert_eq!(input.visual_cursor(4), (0, 2));
    }
    #[test]
    fn semantic_revision_recolors_unchanged_input_without_moving_selection() {
        use crate::ui::controller::settings::theme::groups::HLGroup;
        struct ChangingHighlighter {
            revision: u64,
            group: HLGroup,
        }
        impl Highlighter for ChangingHighlighter {
            fn revision(&self) -> u64 {
                self.revision
            }
            fn highlight(&mut self, source: &str) -> Vec<HLSpan> {
                vec![HLSpan::new(0..source.len(), self.group)]
            }
        }
        let mut input = InputCode::new_in_memory();
        input.field_mut().insert_str("alpha");
        let context = RenderContext::new(40, UI.prompt_width());
        let mut highlighter = ChangingHighlighter {
            revision: 0,
            group: HLGroup::Function,
        };
        input.field_mut().start_selection(1);
        input.field_mut().continue_selection(3);
        input.set_diagnostic("example diagnostic".to_string(), Some(3..5));
        let before = input.highlighted_lines(&context, &mut highlighter, false, None);
        let state = (
            input.revision(),
            input.field().cursor(),
            input.field().selection(),
        );
        highlighter.revision = 1;
        highlighter.group = HLGroup::Type;
        let after = input.highlighted_lines(&context, &mut highlighter, false, None);
        let color = |lines: &[Line<'_>]| {
            lines[0]
                .spans
                .iter()
                .find(|span| span.content == "a")
                .unwrap()
                .style
                .fg
        };
        assert_ne!(color(&before), color(&after));
        assert_eq!(
            state,
            (
                input.revision(),
                input.field().cursor(),
                input.field().selection()
            )
        );
        assert_eq!(input.text(), "alpha");
    }
}
