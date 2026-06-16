//! The live input line editor: the text buffer, a char-boundary cursor,
//! multi-line navigation, and submitted-input history.

use std::mem;

/// The interactive input buffer. `cursor` is a byte offset that is always kept
/// on a UTF-8 char boundary.
pub struct Editor {
    text: String,
    cursor: usize,
    history: Vec<String>,
    history_pos: Option<usize>,
    draft: String,
}

impl Editor {
    pub fn new() -> Self {
        Editor {
            text: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_pos: None,
            draft: String::new(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn insert_char(&mut self, ch: char) {
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.assert_boundary();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.text[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.text.replace_range(prev..self.cursor, "");
        self.cursor = prev;
        self.assert_boundary();
    }

    pub fn delete_forward(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let next = self.next_boundary(self.cursor);
        self.text.replace_range(self.cursor..next, "");
        self.assert_boundary();
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.text[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
        self.assert_boundary();
    }

    pub fn move_right(&mut self) {
        if self.cursor < self.text.len() {
            self.cursor = self.next_boundary(self.cursor);
        }
        self.assert_boundary();
    }

    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor = self.text.len();
    }

    /// Move to the start of the previous line, if any.
    pub fn move_up(&mut self) {
        let starts = self.line_starts();
        let current = self.current_line(&starts);
        if current > 0 {
            self.cursor = starts[current - 1];
        } else {
            self.cursor = 0;
        }
        self.assert_boundary();
    }

    /// Move to the start of the next line, if any, else to the end of the text.
    pub fn move_down(&mut self) {
        let starts = self.line_starts();
        let current = self.current_line(&starts);
        if current < starts.len() {
            self.cursor = starts[current];
        } else {
            self.cursor = self.text.len();
        }
        self.assert_boundary();
    }

    /// True when no newline precedes the cursor — Up should recall history.
    pub fn cursor_on_first_line(&self) -> bool {
        !self.text[..self.cursor].contains('\n')
    }

    /// True when no newline follows the cursor — Down should advance history.
    pub fn cursor_on_last_line(&self) -> bool {
        !self.text[self.cursor..].contains('\n')
    }

    /// The cursor's `(line, column)` in characters, for placing the terminal
    /// cursor. Column counts characters on the cursor's line.
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let before = &self.text[..self.cursor];
        let line = before.lines().count().max(1) - 1;
        let col = before
            .lines()
            .last()
            .map(|l| l.chars().count())
            .unwrap_or(0);
        (line, col)
    }

    /// Step to an older history entry, stashing the draft on first recall.
    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_pos {
            None => {
                self.draft = self.text.clone();
                self.history.len() - 1
            }
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.history_pos = Some(idx);
        self.load(self.history[idx].clone());
    }

    /// Step to a newer history entry, restoring the draft past the newest.
    pub fn history_next(&mut self) {
        let idx = match self.history_pos {
            None => return,
            Some(i) if i + 1 < self.history.len() => i + 1,
            Some(_) => {
                self.history_pos = None;
                let draft = mem::take(&mut self.draft);
                self.load(draft);
                return;
            }
        };
        self.history_pos = Some(idx);
        self.load(self.history[idx].clone());
    }

    /// Take the trimmed buffer as a submission: records it in history (skipping
    /// a consecutive duplicate), resets history navigation, and clears the
    /// buffer. Returns `None` when the buffer is blank.
    pub fn take_submission(&mut self) -> Option<String> {
        let code = self.text.trim().to_string();
        if code.is_empty() {
            return None;
        }
        if self.history.last() != Some(&code) {
            self.history.push(code.clone());
        }
        self.history_pos = None;
        self.draft.clear();
        self.text.clear();
        self.cursor = 0;
        Some(code)
    }

    /// Replace the buffer and park the cursor at its end.
    fn load(&mut self, text: String) {
        self.cursor = text.len();
        self.text = text;
        self.assert_boundary();
    }

    fn next_boundary(&self, from: usize) -> usize {
        self.text[from..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| from + i)
            .unwrap_or(self.text.len())
    }

    /// Byte offsets just after each newline — the start of every line but the
    /// first.
    fn line_starts(&self) -> Vec<usize> {
        self.text
            .char_indices()
            .filter(|&(_, ch)| ch == '\n')
            .map(|(i, _)| i + 1)
            .collect()
    }

    /// Index of the line the cursor sits on, given the line starts.
    fn current_line(&self, starts: &[usize]) -> usize {
        starts
            .iter()
            .position(|&start| start > self.cursor)
            .unwrap_or(starts.len())
    }

    fn assert_boundary(&self) {
        debug_assert!(
            self.text.is_char_boundary(self.cursor),
            "cursor {} off a char boundary in {:?}",
            self.cursor,
            self.text
        );
    }
}
