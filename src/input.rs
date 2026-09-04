//! Live input model.
//!
//! - [`InputTextField`] — raw code, a char-boundary cursor, and submission history.
//! - [`InputCode`] — the field and its transient presentation state.

mod history;

use std::iter;
use std::ops::Range;

use crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use these_macros_should_be_illegal::discriminated_str;

use self::history::{History, HistoryBuffer};
use crate::code::highlight::Highlighter;
use crate::ui::block::render::{Highlights, Prompt, RenderContext};
use crate::ui::block::wrap::{HardWrap, TextLayout, WrapPolicy};
use crate::ui::config::UI;
use crate::ui::element::{HLSpan, SpanText, Text};
use crate::ui::key::KeyEvent;

/// The glyph marking the live prompt line.
const PROMPT_GLYPH: &str = "❯";

/// A text-editing operation owned and interpreted by the input field.
#[discriminated_str(description)]
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InputAction {
    InsertNewline = "insert a line break",
    InsertTab = "insert a tab",
    DeleteBack = "delete before the cursor",
    DeleteForward = "delete under the cursor",

    MoveLeft = "move left",
    MoveRight = "move right",
    MoveUp = "move up, or recall older input",
    MoveDown = "move down, or recall newer input",
    MoveLineStart = "move to line start",
    MoveLineEnd = "move to line end",

    SelectLeft = "extend selection left",
    SelectRight = "extend selection right",
    SelectLineStart = "extend selection to line start",
    SelectLineEnd = "extend selection to line end",

    Clear = "clear input text",
}

/// User-adjustable layout policy owned by the input component.
#[derive(Clone, Copy, Debug)]
pub struct InputSettings {
    /// Maximum editor-height percentage occupied by the input.
    pub max_height_percent: u8,
    /// Context rows retained around the cursor.
    pub scrolloff: u8,
}

impl Default for InputSettings {
    fn default() -> Self {
        Self {
            max_height_percent: 75,
            scrolloff: 2,
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
}

impl InputSetting {
    /// Picker label for this setting.
    pub const fn label(self) -> &'static str {
        match self {
            Self::MaxHeight => "max input height",
            Self::Scrolloff => "scrolloff",
        }
    }

    /// Current picker value for this setting.
    pub fn value(self, settings: InputSettings) -> String {
        match self {
            Self::MaxHeight => format!("{}%", settings.max_height_percent),
            Self::Scrolloff => format!("{} rows", settings.scrolloff),
        }
    }

    /// Adjust this setting by one picker step.
    pub fn adjust(self, settings: &mut InputSettings, delta: i8) {
        let field = match self {
            Self::MaxHeight => &mut settings.max_height_percent,
            Self::Scrolloff => &mut settings.scrolloff,
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
    }
}

/// Editable source, its byte-offset cursor, and submission history.
pub struct InputTextField {
    text: String,
    cursor: usize,
    revision: u64,
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
            revision: 0,
            anchor: None,
            history: History::new(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The cursor's byte offset into the source.
    fn cursor(&self) -> usize {
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
        self.changed();
        true
    }

    pub fn insert_char(&mut self, ch: char) {
        self.delete_selection();
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.changed();
    }

    pub fn insert_str(&mut self, s: &str) {
        self.delete_selection();
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
        self.changed();
    }

    fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.anchor = None;
        self.changed();
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
        self.changed();
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
        self.changed();
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
        self.changed();
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
        }
    }
}

/// The live source field and its transient presentation state.
pub struct InputCode {
    field: InputTextField,
    settings: InputSettings,
    diagnostic: Option<String>,
    diagnostic_range: Option<Range<usize>>,
    selecting: bool,
    highlighted_revision: u64,
    highlights: Vec<HLSpan>,
    area: Rect,
    text_area: Rect,
    scroll: usize,
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
}

impl InputCode {
    pub fn new() -> Self {
        Self {
            field: InputTextField::new(),
            settings: InputSettings::default(),
            diagnostic: None,
            diagnostic_range: None,
            selecting: false,
            highlighted_revision: u64::MAX,
            highlights: Vec::new(),
            area: Rect::ZERO,
            text_area: Rect::ZERO,
            scroll: 0,
        }
    }

    pub fn field(&self) -> &InputTextField {
        &self.field
    }

    pub fn field_mut(&mut self) -> &mut InputTextField {
        &mut self.field
    }

    /// Return the component's adjustable layout policy.
    pub const fn settings(&self) -> InputSettings {
        self.settings
    }

    /// Replace the component's adjustable layout policy.
    pub fn set_settings(&mut self, settings: InputSettings) {
        self.settings = settings;
    }

    /// Adjust one component-owned setting.
    pub fn adjust(&mut self, setting: InputSetting, delta: i8) {
        setting.adjust(&mut self.settings, delta);
    }

    pub fn accept_submission(&mut self) {
        self.field.accept_submission();
        self.diagnostic = None;
        self.diagnostic_range = None;
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

    /// Apply one input-owned editing operation.
    pub fn edit(&mut self, action: InputAction) {
        use InputAction::*;
        if matches!(
            action,
            InsertNewline | InsertTab | DeleteBack | DeleteForward | Clear
        ) {
            self.clear_diagnostic();
        }
        if action == Clear {
            return self.clear();
        }
        let field = &mut self.field;
        match action {
            InsertNewline => field.insert_char('\n'),
            InsertTab => field.insert_char('\t'),
            DeleteBack => field.backspace(),
            DeleteForward => field.delete_forward(),
            MoveLeft => field.move_left(),
            MoveRight => field.move_right(),
            MoveLineStart => field.move_home(),
            MoveLineEnd => field.move_end(),
            MoveUp if field.cursor_on_first_line() => field.history_prev(),
            MoveUp => field.move_up(),
            MoveDown if field.cursor_on_last_line() => field.history_next(),
            MoveDown => field.move_down(),
            SelectLeft => field.select_left(),
            SelectRight => field.select_right(),
            SelectLineStart => field.select_home(),
            SelectLineEnd => field.select_end(),
            Clear => unreachable!(),
        }
    }

    /// Interpret one unbound terminal key as editor input.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<char> {
        use InputAction::*;
        let action = match key.code {
            KeyCode::Char('l') if key.ctrl() => Some(Clear),
            KeyCode::Char(' ') if key.shift() => Some(InsertNewline),
            KeyCode::Enter if key.shift() || key.alt() => Some(InsertNewline),
            KeyCode::Tab => Some(InsertTab),
            KeyCode::Backspace => Some(DeleteBack),
            KeyCode::Delete => Some(DeleteForward),
            KeyCode::Left if key.shift() => Some(SelectLeft),
            KeyCode::Right if key.shift() => Some(SelectRight),
            KeyCode::Home if key.shift() => Some(SelectLineStart),
            KeyCode::End if key.shift() => Some(SelectLineEnd),
            KeyCode::Left => Some(MoveLeft),
            KeyCode::Right => Some(MoveRight),
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

    /// Insert pasted source as one input-owned edit.
    pub fn insert(&mut self, text: &str) {
        self.clear_diagnostic();
        self.field.insert_str(text);
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
        self.cursor_at_visual(area.width, scroll + row as usize, column)
    }

    #[cfg(test)]
    pub fn new_in_memory() -> Self {
        Self {
            field: InputTextField::new_in_memory(),
            settings: InputSettings::default(),
            diagnostic: None,
            diagnostic_range: None,
            selecting: false,
            highlighted_revision: u64::MAX,
            highlights: Vec::new(),
            area: Rect::ZERO,
            text_area: Rect::ZERO,
            scroll: 0,
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
    ) -> Vec<Line<'static>> {
        if self.highlighted_revision != self.field.revision {
            self.highlights = highlighter.highlight(self.field.text());
            self.highlighted_revision = self.field.revision;
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
        let painted = self.highlights.paint(0, text, base, theme);
        let visual_lines = self.lines(context.width().saturating_sub(context.prefix()));

        let mut lines = Vec::new();
        for (index, visual_line) in visual_lines.iter().enumerate() {
            let marker = if index + 1 == visual_lines.len() {
                PROMPT_GLYPH
            } else {
                ""
            };
            let mut spans = Prompt::new(marker, prompt).spans(&());
            let mut line = painted.slice(&visual_line.range);
            if let Some(diagnostic) = self.diagnostic_range.as_ref() {
                line = apply_selection(
                    line,
                    visual_line.range.start,
                    diagnostic,
                    theme.diagnostic(),
                );
            }
            if let Some(selection) = selection.as_ref() {
                line = apply_selection(line, visual_line.range.start, selection, theme.selection());
            }
            spans.extend(line);
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
        let mut rows =
            self.highlighted_lines(context.render, context.highlighter, context.debugging);
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
        self.scroll = self.scroll_start(cursor_row, rows.len(), viewport);
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
        lines.extend(rows.into_iter().skip(self.scroll).take(viewport));
        if let Some(message) = diagnostic {
            let error = groups.error().patch(code);
            lines.push(Line::from(vec![
                Prompt::new("", error).spans(&())[0].clone(),
                Span::styled(message, error),
            ]));
        }
        lines.push(Line::default());
        Paragraph::new(lines).style(code).render(area, buffer);
        context.focused.then(|| {
            Position::new(
                area.x + context.render.prefix() + cursor_column as u16,
                area.y + 1 + (cursor_row - self.scroll) as u16,
            )
        })
    }

    /// Forget the pane rectangle while another view is displayed.
    pub fn hide(&mut self) {
        self.area = Rect::ZERO;
        self.text_area = Rect::ZERO;
    }

    /// Return the input pane's last rendered rectangle.
    #[cfg(test)]
    pub fn area(&self) -> Rect {
        self.area
    }

    /// Return the editable text's last rendered rectangle.
    #[cfg(test)]
    pub fn text_area(&self) -> Rect {
        self.text_area
    }

    /// Supply layout rectangles without rendering in mouse-routing tests.
    #[cfg(test)]
    pub fn set_test_area(&mut self, area: Rect, text_area: Rect) {
        self.area = area;
        self.text_area = text_area;
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
    use crate::ui::key::KeyMod::{Alt, Shift};
    use ratatui::style::Modifier;

    #[test]
    fn editor_owns_modified_newlines_and_tabs() {
        let mut input = InputCode::new_in_memory();
        for key in [
            KeyCode::Char(' ') + Shift,
            KeyCode::Enter + Shift,
            KeyCode::Enter + Alt,
            KeyCode::Tab.into(),
        ] {
            input.handle_key(key);
        }

        assert_eq!(input.text(), "\n\n\n\t");
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
        let context = RenderContext::new(40, UI.prompt_width(), &theme);
        let mut highlighter = SyntaxHighlighter::new();

        let lines = input.highlighted_lines(&context, &mut highlighter, false);
        assert_eq!(lines.len(), 3);
        assert!(!lines[0].spans[0].content.contains(PROMPT_GLYPH));
        assert!(!lines[1].spans[0].content.contains(PROMPT_GLYPH));
        assert!(lines[2].spans[0].content.contains(PROMPT_GLYPH));
    }

    #[test]
    fn empty_input_contains_only_the_prompt() {
        let mut input = InputCode::new_in_memory();
        let theme = Groups::default();
        let context = RenderContext::new(40, UI.prompt_width(), &theme);
        let mut highlighter = SyntaxHighlighter::new();

        let rendered: String = input.highlighted_lines(&context, &mut highlighter, false)[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();

        assert_eq!(rendered, "  ❯  ");
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
        let theme = Groups::default();
        let context = RenderContext::new(40, UI.prompt_width(), &theme);
        let mut highlighter = CountingHighlighter(0);

        input.highlighted_lines(&context, &mut highlighter, false);
        input.highlighted_lines(&context, &mut highlighter, false);
        assert_eq!(highlighter.0, 1);

        input.field_mut().insert_char('x');
        input.highlighted_lines(&context, &mut highlighter, false);
        assert_eq!(highlighter.0, 2);
    }

    #[test]
    fn diagnostic_special_colour_underlines_only_its_source_range() {
        struct EmptyHighlighter;

        impl Highlighter for EmptyHighlighter {
            fn highlight(&mut self, _source: &str) -> Vec<crate::ui::element::HLSpan> {
                Vec::new()
            }
        }

        let mut input = InputCode::new_in_memory();
        input.field_mut().insert_str("abc");
        input.set_diagnostic("bad b".to_string(), Some(1..2));
        let theme = Groups::default();
        let context = RenderContext::new(40, UI.prompt_width(), &theme);
        let lines = input.highlighted_lines(&context, &mut EmptyHighlighter, false);
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
