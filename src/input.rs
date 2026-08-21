//! The live input model.
//!
//! - [`CodeFragment`] — raw code, not yet split into cells.
//! - [`InputTextField`] — the live field: a fragment, a char-boundary cursor in it,
//!   and the submission history it browses.
//! - [`InputCode`] — the field plus the highlight spans for its whole text; the only
//!   [`Highlighted`] unit. Incomplete input simply stays in the field across Enter
//!   presses (a newline is inserted), so the buffer is one editable region and its
//!   highlight is recomputed wholesale when its render lines are requested. Cell
//!   boundaries are derived only at submit, after the M2 completeness check, never here.

use std::iter;

use ratatui::text::{Line, Span};

use crate::block::highlight::{HighlightSpan, Highlighter};
use crate::block::layout::{PreserveLines, TextLayout};
use crate::block::render::{apply_spans, gutter_span, RenderContext, GUTTER_W};
use crate::block::Highlighted;
use crate::hints::get_hint;
use crate::history::History;

/// The glyph marking the live prompt line.
const PROMPT_GLYPH: &str = "❯";

pub struct CodeFragment {
    text: String,
}

impl CodeFragment {
    pub fn new() -> Self {
        CodeFragment {
            text: String::new(),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn clear(&mut self) {
        self.text.clear();
    }
}

/// The live editing field: a fragment, a byte-offset cursor on a UTF-8 boundary, and the
/// submission history it browses.
pub struct InputTextField {
    fragment: CodeFragment,
    cursor: usize,
    history: History,
}

impl InputTextField {
    pub fn new() -> Self {
        InputTextField {
            fragment: CodeFragment::new(),
            cursor: 0,
            history: History::new(),
        }
    }

    pub fn fragment(&self) -> &CodeFragment {
        &self.fragment
    }

    pub fn insert_char(&mut self, ch: char) {
        self.fragment.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.assert_boundary();
    }

    pub fn insert_str(&mut self, s: &str) {
        self.fragment.text.insert_str(self.cursor, s);
        self.cursor += s.len();
        self.assert_boundary();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.fragment.text[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.fragment.text.replace_range(prev..self.cursor, "");
        self.cursor = prev;
        self.assert_boundary();
    }

    pub fn delete_forward(&mut self) {
        if self.cursor >= self.fragment.text.len() {
            return;
        }
        let next = self.next_boundary(self.cursor);
        self.fragment.text.replace_range(self.cursor..next, "");
        self.assert_boundary();
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.fragment.text[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
        self.assert_boundary();
    }

    pub fn move_right(&mut self) {
        if self.cursor < self.fragment.text.len() {
            self.cursor = self.next_boundary(self.cursor);
        }
        self.assert_boundary();
    }

    pub fn move_home(&mut self) {
        self.cursor = self.fragment.text[..self.cursor]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        self.assert_boundary();
    }

    pub fn move_end(&mut self) {
        self.cursor = self.fragment.text[self.cursor..]
            .find('\n')
            .map(|i| self.cursor + i)
            .unwrap_or(self.fragment.text.len());
        self.assert_boundary();
    }

    pub fn move_up(&mut self) {
        let starts = self.line_starts();
        let current = self.current_line(&starts);
        if current == 0 {
            return;
        }
        let column = self.fragment.text[starts[current]..self.cursor]
            .chars()
            .count();
        self.cursor = self.cursor_at_column(&starts, current - 1, column);
        self.assert_boundary();
    }

    pub fn move_down(&mut self) {
        let starts = self.line_starts();
        let current = self.current_line(&starts);
        if current + 1 == starts.len() {
            return;
        }
        let column = self.fragment.text[starts[current]..self.cursor]
            .chars()
            .count();
        self.cursor = self.cursor_at_column(&starts, current + 1, column);
        self.assert_boundary();
    }

    pub fn cursor_on_first_line(&self) -> bool {
        !self.fragment.text[..self.cursor].contains('\n')
    }

    pub fn cursor_on_last_line(&self) -> bool {
        !self.fragment.text[self.cursor..].contains('\n')
    }

    pub fn cursor_line_col(&self) -> (usize, usize) {
        // `str::lines()` drops a trailing newline, so a cursor sitting just past one
        // would report the previous row. Count newlines directly, and take the column
        // from the text after the last one (the whole prefix when there is none).
        let before = &self.fragment.text[..self.cursor];
        let line = before.matches('\n').count();
        let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count());
        (line, col)
    }

    pub fn history_prev(&mut self) {
        if let Some(text) = self.history.prev(&self.fragment.text) {
            self.load(text);
        }
    }

    pub fn history_next(&mut self) {
        if let Some(text) = self.history.next() {
            self.load(text);
        }
    }

    /// Record the trimmed fragment in history and clear the field, handing back the
    /// submitted fragment. `None` when the field is blank.
    pub fn take_submission(&mut self) -> Option<String> {
        let text = self.fragment.text.trim().to_string();
        if text.is_empty() {
            return None;
        }
        self.history.record(&text);
        self.fragment.clear();
        self.cursor = 0;
        Some(text)
    }

    fn load(&mut self, text: String) {
        self.cursor = text.len();
        self.fragment.text = text;
        self.assert_boundary();
    }

    fn next_boundary(&self, from: usize) -> usize {
        self.fragment.text[from..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| from + i)
            .unwrap_or(self.fragment.text.len())
    }

    fn line_starts(&self) -> Vec<usize> {
        iter::once(0)
            .chain(
                self.fragment
                    .text
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
            .map_or(self.fragment.text.len(), |next| next - 1);
        self.fragment.text[start..end]
            .char_indices()
            .nth(column)
            .map_or(end, |(offset, _)| start + offset)
    }

    fn assert_boundary(&self) {
        debug_assert!(
            self.fragment.text.is_char_boundary(self.cursor),
            "cursor {} off a char boundary in {:?}",
            self.cursor,
            self.fragment.text
        );
    }

    #[cfg(test)]
    fn new_in_memory() -> Self {
        InputTextField {
            fragment: CodeFragment::new(),
            cursor: 0,
            history: History::in_memory(),
        }
    }
}

/// The live field plus the highlight spans over its whole text — the only
/// [`Highlighted`] unit. Incomplete input never leaves the field (Enter inserts a
/// newline and editing continues), so there is one editable region whose spans are
/// recomputed wholesale when rendered; at submit, display blocks are highlighted from
/// their own text. The cursor and history live in the field.
pub struct InputCode {
    field: InputTextField,
    spans: Vec<HighlightSpan>,
    hint: String,
    diagnostic: Option<String>,
}

impl InputCode {
    pub fn new() -> Self {
        InputCode {
            field: InputTextField::new(),
            spans: Vec::new(),
            hint: get_hint(),
            diagnostic: None,
        }
    }

    pub fn field(&self) -> &InputTextField {
        &self.field
    }

    pub fn field_mut(&mut self) -> &mut InputTextField {
        &mut self.field
    }

    /// Take the trimmed buffer as a submission (recording it in history and clearing
    /// the field), or `None` when blank. The caller validates and runs it.
    pub fn take_submission(&mut self) -> Option<String> {
        let submission = self.field.take_submission()?;
        self.spans.clear();
        self.hint = get_hint();
        self.diagnostic = None;
        Some(submission)
    }

    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }

    pub fn set_diagnostic(&mut self, diagnostic: String) {
        self.diagnostic = Some(diagnostic);
    }

    pub fn clear_diagnostic(&mut self) {
        self.diagnostic = None;
    }

    #[cfg(test)]
    fn new_in_memory() -> Self {
        InputCode {
            field: InputTextField::new_in_memory(),
            spans: Vec::new(),
            hint: get_hint(),
            diagnostic: None,
        }
    }
}

impl InputCode {
    pub fn text(&self) -> &str {
        self.field.fragment().as_str()
    }
}

impl TextLayout for InputCode {
    type Wrapping = PreserveLines;

    fn text(&self) -> &str {
        self.field.fragment().as_str()
    }
}

impl Highlighted for InputCode {
    fn highlighted_lines<'a>(
        &'a mut self,
        context: &RenderContext<'_>,
        highlighter: &mut dyn Highlighter,
    ) -> Vec<Line<'a>> {
        self.spans = highlighter.highlight(&self.field.fragment().text);
        let theme = context.theme();
        let base = theme.code();
        let prompt = theme.prompt().patch(base);
        let text = self.text();
        let visual_lines = self.lines(context.width().saturating_sub(GUTTER_W));

        let mut lines = vec![Line::default()];
        for (index, visual_line) in visual_lines.iter().enumerate() {
            let marker = if index + 1 == visual_lines.len() {
                PROMPT_GLYPH
            } else {
                ""
            };
            let mut spans = vec![gutter_span(marker, prompt)];
            let line = &text[visual_line.range.clone()];
            spans.extend(apply_spans(
                visual_line.range.start,
                line,
                &self.spans,
                base,
                theme,
            ));
            if index == 0 && text.is_empty() {
                spans.push(Span::styled(&self.hint, theme.muted()));
            }
            lines.push(Line::from(spans));
        }
        lines
    }
}

/// The column the live prompt's gutter ends and the code begins — where the caller
/// offsets the terminal cursor to.
pub const INPUT_CODE_COL: u16 = GUTTER_W;

#[cfg(test)]
mod tests {
    use super::*;

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
        let theme = crate::theme::Theme::default();
        let context = RenderContext::new(8, &theme);
        let mut highlighter = crate::highlight::SyntaxHighlighter::new();

        let lines = input.highlighted_lines(&context, &mut highlighter);
        assert!(!lines[1].spans[0].content.contains(PROMPT_GLYPH));
        assert!(!lines[2].spans[0].content.contains(PROMPT_GLYPH));
        assert!(lines[3].spans[0].content.contains(PROMPT_GLYPH));
    }
}
