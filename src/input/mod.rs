//! The live input model.
//!
//! - [`InputTextField`] — raw code, a char-boundary cursor, and submission history.
//! - [`InputCode`] — the field and its transient presentation state.

mod hints;
mod history;

use std::iter;
use std::ops::Range;

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use self::hints::get_hint;
use self::history::{History, HistoryBuffer};
use crate::code::highlight::Highlighter;
use crate::ui::block::layout::{wrap, HardWrap, TextLayout};
use crate::ui::block::render::{apply_spans, gutter_span, RenderContext};

/// The glyph marking the live prompt line.
const PROMPT_GLYPH: &str = "❯";

/// Editable source, its byte-offset cursor, and submission history.
pub struct InputTextField {
    text: String,
    cursor: usize,
    /// Where a selection started, if one is active. The cursor is its other end,
    /// so extending a selection is just moving the cursor.
    anchor: Option<usize>,
    history: History,
}

impl InputTextField {
    fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            anchor: None,
            history: History::new(),
        }
    }

    fn text(&self) -> &str {
        &self.text
    }

    /// The cursor's byte offset into the source.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The selected byte range, ordered, when a selection is active and not
    /// empty.
    pub fn selection(&self) -> Option<Range<usize>> {
        let anchor = self.anchor?;
        let (start, end) = (anchor.min(self.cursor), anchor.max(self.cursor));
        (start < end).then_some(start..end)
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.selection().map(|range| &self.text[range])
    }

    pub fn start_selection(&mut self, cursor: usize) {
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
        self.text.replace_range(range.clone(), "");
        self.cursor = range.start;
        self.clear_selection();
        true
    }

    pub fn insert_char(&mut self, ch: char) {
        self.delete_selection();
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    pub fn insert_str(&mut self, s: &str) {
        self.delete_selection();
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.anchor = None;
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let prev = self.previous_boundary();
        self.text.replace_range(prev..self.cursor, "");
        self.cursor = prev;
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor >= self.text.len() {
            return;
        }
        let next = self.next_boundary(self.cursor);
        self.text.replace_range(self.cursor..next, "");
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

    pub fn cursor_on_first_line(&self) -> bool {
        !self.text[..self.cursor].contains('\n')
    }

    pub fn cursor_on_last_line(&self) -> bool {
        !self.text[self.cursor..].contains('\n')
    }

    #[cfg(test)]
    pub fn cursor_line_col(&self) -> (usize, usize) {
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
            self.load(buffer);
        }
    }

    pub fn history_next(&mut self) {
        if let Some(buffer) = self.history.next() {
            self.load(buffer);
        }
    }

    /// Replace the whole buffer. The anchor indexes the text it was set in, so
    /// every wholesale replacement has to drop it -- a stale anchor left pointing
    /// past the new end makes the next edit or copy slice out of bounds.
    fn replace_text(&mut self, text: String) {
        self.cursor = text.len();
        self.text = text;
        self.clear_selection();
    }

    fn accept_submission(&mut self) {
        self.history.record(&self.text);
        self.replace_text(String::new());
    }

    fn load(&mut self, buffer: HistoryBuffer) {
        self.text = buffer.text;
        self.cursor = buffer.cursor.min(self.text.len());
        if !self.text.is_char_boundary(self.cursor) {
            self.cursor = self.text.len();
        }
        self.clear_selection();
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
            anchor: None,
            history: History::in_memory(),
        }
    }
}

/// The live source field and its transient presentation state.
pub struct InputCode {
    field: InputTextField,
    hint: &'static str,
    diagnostic: Option<String>,
    diagnostic_range: Option<Range<usize>>,
}

impl InputCode {
    pub fn new() -> Self {
        Self {
            field: InputTextField::new(),
            hint: get_hint(),
            diagnostic: None,
            diagnostic_range: None,
        }
    }

    pub fn field(&self) -> &InputTextField {
        &self.field
    }

    pub fn field_mut(&mut self) -> &mut InputTextField {
        &mut self.field
    }

    pub fn accept_submission(&mut self) {
        self.field.accept_submission();
        self.hint = get_hint();
        self.diagnostic = None;
        self.diagnostic_range = None;
    }

    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }

    pub fn set_diagnostic(&mut self, diagnostic: String, range: Option<Range<usize>>) {
        self.diagnostic = Some(diagnostic);
        self.diagnostic_range = range;
    }

    pub fn clear_diagnostic(&mut self) {
        self.diagnostic = None;
        self.diagnostic_range = None;
    }

    pub fn clear(&mut self) {
        self.field.clear();
        self.clear_diagnostic();
    }

    #[cfg(test)]
    pub(crate) fn new_in_memory() -> Self {
        Self {
            field: InputTextField::new_in_memory(),
            hint: get_hint(),
            diagnostic: None,
            diagnostic_range: None,
        }
    }

    /// Return the source currently being edited.
    pub fn text(&self) -> &str {
        self.field.text()
    }

    /// Render the live source with fresh syntax highlighting.
    pub fn highlighted_lines(
        &self,
        context: &RenderContext<'_>,
        highlighter: &mut dyn Highlighter,
    ) -> Vec<Line<'static>> {
        let highlights = highlighter.highlight(self.text());
        let selection = self.field.selection();
        let theme = context.groups();
        let base = theme.code();
        // In the debugger the prompt changes colour, so a nested M2 interpreter is
        // never mistaken for the ordinary one.
        let prompt = if context.is_debugging() {
            theme.warning().patch(base)
        } else {
            theme.prompt().patch(base)
        };
        let text = self.text();
        let visual_lines = self.lines(context.width().saturating_sub(context.gutter()));

        let mut lines = Vec::new();
        for (index, visual_line) in visual_lines.iter().enumerate() {
            let marker = if index + 1 == visual_lines.len() {
                PROMPT_GLYPH
            } else {
                ""
            };
            let mut spans = vec![gutter_span(marker, context.gutter(), prompt)];
            let line = &text[visual_line.range.clone()];
            let mut painted = apply_spans(visual_line.range.start, line, &highlights, base, theme);
            if let Some(diagnostic) = self.diagnostic_range.as_ref() {
                painted = apply_selection(
                    painted,
                    visual_line.range.start,
                    diagnostic,
                    theme.diagnostic(),
                );
            }
            if let Some(selection) = selection.as_ref() {
                painted = apply_selection(
                    painted,
                    visual_line.range.start,
                    selection,
                    theme.selection(),
                );
            }
            spans.extend(painted);
            if index == 0 && text.is_empty() {
                spans.push(Span::styled(format!(" {}", self.hint), theme.muted()));
            }
            lines.push(Line::from(spans));
        }
        lines
    }
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
        let rows = wrap::<HardWrap>(text, width);
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
        let rows = wrap::<HardWrap>(self.text(), width);
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
    use crate::ui::block::render::{gutter_width, prompt_gutter_width};
    use crate::ui::Groups;
    use ratatui::style::Modifier;

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
    fn returning_from_history_restores_the_draft_cursor() {
        let mut field = InputTextField::new_in_memory();
        field.history.record("old input");
        field.insert_str("top\nbottom");
        field.cursor = 0;

        field.history_prev();
        field.history_next();

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
        let theme = Groups::default();
        let context = RenderContext::new(40, gutter_width(9), &theme);
        let mut highlighter = SyntaxHighlighter::new();

        let lines = input.highlighted_lines(&context, &mut highlighter);
        assert_eq!(lines.len(), 3);
        assert!(!lines[0].spans[0].content.contains(PROMPT_GLYPH));
        assert!(!lines[1].spans[0].content.contains(PROMPT_GLYPH));
        assert!(lines[2].spans[0].content.contains(PROMPT_GLYPH));
    }

    #[test]
    fn empty_input_leaves_two_cells_after_the_prompt_and_one_before_the_hint() {
        let input = InputCode::new_in_memory();
        let theme = Groups::default();
        let context = RenderContext::new(40, prompt_gutter_width(), &theme);
        let mut highlighter = SyntaxHighlighter::new();

        let rendered: String = input.highlighted_lines(&context, &mut highlighter)[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();

        assert!(rendered.starts_with("  ❯   "));
        let hint_column = rendered[..rendered.find(input.hint).expect("hint")]
            .chars()
            .count();
        assert_eq!(hint_column, 6);
    }

    #[test]
    fn diagnostic_special_colour_underlines_only_its_source_range() {
        struct EmptyHighlighter;

        impl Highlighter for EmptyHighlighter {
            fn highlight(&mut self, _source: &str) -> Vec<crate::code::highlight::HLSpan> {
                Vec::new()
            }
        }

        let mut input = InputCode::new_in_memory();
        input.field_mut().insert_str("abc");
        input.set_diagnostic("bad b".to_string(), Some(1..2));
        let theme = Groups::default();
        let context = RenderContext::new(40, prompt_gutter_width(), &theme);
        let lines = input.highlighted_lines(&context, &mut EmptyHighlighter);
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
}
