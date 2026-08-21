//! The interactive editor: submitted feed, live input and history, navigation,
//! scrolling, and rendering.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as TuiBlock, BorderType, Clear, Paragraph, Widget};
use ratatui::Frame;

use crate::block::highlight::Highlighter;
use crate::block::render::{gutter_span, RenderContext};
use crate::block::{Block, CodeBlock, Highlighted, SystemBlock};
use crate::input::{InputCode, INPUT_CODE_COL};
use crate::theme::Theme;

const MAX_INPUT_LINES: u16 = 8;
const SCROLL_STEP: usize = 1;

pub enum EditorAction {
    Continue,
    Submit,
    Quit,
}

pub enum EditorUpdate {
    Ignore,
    ContinueSubmission,
    Diagnostic(String),
    Commit(Vec<Box<dyn Block>>),
    ToggleCode,
}

pub struct Editor {
    feed: Vec<Box<dyn Block>>,
    input: InputCode,
    scroll_offset: usize,
    show_code_pane: bool,
}

impl Editor {
    pub fn new(welcome: String) -> Self {
        Self {
            feed: vec![Box::new(SystemBlock { text: welcome })],
            input: InputCode::new(),
            scroll_offset: 0,
            show_code_pane: false,
        }
    }

    pub fn submission(&self) -> &str {
        self.input.text()
    }

    pub fn apply(&mut self, update: EditorUpdate) {
        match update {
            EditorUpdate::Ignore => {}
            EditorUpdate::ContinueSubmission => self.input.field_mut().insert_char('\n'),
            EditorUpdate::Diagnostic(diagnostic) => self.input.set_diagnostic(diagnostic),
            EditorUpdate::Commit(blocks) => {
                self.input.take_submission();
                self.feed.extend(blocks);
                self.scroll_offset = 0;
            }
            EditorUpdate::ToggleCode => {
                self.input.take_submission();
                self.show_code_pane = !self.show_code_pane;
            }
        }
    }

    pub fn paste(&mut self, text: &str) {
        self.input.clear_diagnostic();
        self.input.field_mut().insert_str(text);
    }

    pub fn handle_mouse(&mut self, kind: MouseEventKind) {
        match kind {
            MouseEventKind::ScrollUp => {
                self.scroll_offset = self.scroll_offset.saturating_add(SCROLL_STEP);
            }
            MouseEventKind::ScrollDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(SCROLL_STEP);
            }
            _ => {}
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> EditorAction {
        if matches!(
            key.code,
            KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete | KeyCode::Tab | KeyCode::Enter
        ) {
            self.input.clear_diagnostic();
        }

        let field = self.input.field_mut();
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
                EditorAction::Quit
            }
            (KeyCode::Enter, modifiers)
                if modifiers.contains(KeyModifiers::SHIFT)
                    || modifiers.contains(KeyModifiers::ALT) =>
            {
                field.insert_char('\n');
                EditorAction::Continue
            }
            (KeyCode::Enter, _) => EditorAction::Submit,
            (KeyCode::Char(character), _) => {
                field.insert_char(character);
                EditorAction::Continue
            }
            (KeyCode::Backspace, _) => {
                field.backspace();
                EditorAction::Continue
            }
            (KeyCode::Delete, _) => {
                field.delete_forward();
                EditorAction::Continue
            }
            (KeyCode::Left, _) => {
                field.move_left();
                EditorAction::Continue
            }
            (KeyCode::Right, _) => {
                field.move_right();
                EditorAction::Continue
            }
            (KeyCode::Home, _) => {
                field.move_home();
                EditorAction::Continue
            }
            (KeyCode::End, _) => {
                field.move_end();
                EditorAction::Continue
            }
            (KeyCode::Up, _) if field.cursor_on_first_line() => {
                field.history_prev();
                EditorAction::Continue
            }
            (KeyCode::Up, _) => {
                field.move_up();
                EditorAction::Continue
            }
            (KeyCode::Down, _) if field.cursor_on_last_line() => {
                field.history_next();
                EditorAction::Continue
            }
            (KeyCode::Down, _) => {
                field.move_down();
                EditorAction::Continue
            }
            (KeyCode::PageUp, _) => {
                self.scroll_offset = self.scroll_offset.saturating_add(SCROLL_STEP);
                EditorAction::Continue
            }
            (KeyCode::PageDown, _) => {
                self.scroll_offset = self.scroll_offset.saturating_sub(SCROLL_STEP);
                EditorAction::Continue
            }
            (KeyCode::Tab, _) => {
                field.insert_char('\t');
                EditorAction::Continue
            }
            _ => EditorAction::Continue,
        }
    }

    pub fn draw(
        &mut self,
        frame: &mut Frame,
        theme: &Theme,
        highlighter: &mut dyn Highlighter,
        committed: &[String],
    ) {
        let area = frame.area();
        frame.render_widget(Clear, area);

        let (editor_area, code_area) = if self.show_code_pane {
            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);
            (columns[0], Some(columns[1]))
        } else {
            (area, None)
        };

        let input_height = self.input_height();
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(editor_area.height.saturating_sub(input_height)),
                Constraint::Length(input_height),
            ])
            .split(editor_area);
        let context = RenderContext::new(editor_area.width, theme);

        self.draw_feed(frame, sections[0], &context);
        self.draw_input(frame, sections[1], &context, highlighter);
        if let Some(code_area) = code_area {
            self.draw_code_pane(frame, code_area, theme, highlighter, committed);
        }
    }

    fn input_height(&self) -> u16 {
        let lines = self.input.text().split('\n').count() as u16;
        let diagnostic = u16::from(self.input.diagnostic().is_some());
        (lines + 2 + diagnostic).clamp(3, MAX_INPUT_LINES + 2 + diagnostic)
    }

    fn draw_feed(&self, frame: &mut Frame, area: Rect, context: &RenderContext<'_>) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let heights: Vec<u16> = self
            .feed
            .iter()
            .map(|block| block.height(context))
            .collect();
        let total: u16 = heights.iter().sum();
        let buffer_height = total.max(area.height).max(1);
        let mut composition = Buffer::empty(Rect::new(0, 0, area.width, buffer_height));
        fill(&mut composition, context.theme().transcript());

        let mut y = 0;
        for (block, height) in self.feed.iter().zip(&heights) {
            block.render(
                context,
                Rect::new(0, y, area.width, *height),
                &mut composition,
            );
            y += height;
        }

        let visible = area.height as usize;
        let max_scroll = (total as usize).saturating_sub(visible);
        let top = max_scroll.saturating_sub(self.scroll_offset);
        let destination = frame.buffer_mut();
        for row in 0..area.height {
            let source_y = top + row as usize;
            if source_y >= buffer_height as usize {
                break;
            }
            for column in 0..area.width {
                if let Some(cell) = composition.cell((column, source_y as u16)) {
                    if let Some(slot) = destination.cell_mut((area.x + column, area.y + row)) {
                        *slot = cell.clone();
                    }
                }
            }
        }
    }

    fn draw_input(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        context: &RenderContext<'_>,
        highlighter: &mut dyn Highlighter,
    ) {
        let diagnostic = self.input.diagnostic().map(str::to_string);
        let mut lines = self.input.highlighted_lines(context, highlighter);
        let code = context.theme().code();
        if let Some(message) = diagnostic {
            let error = context.theme().error().patch(code);
            lines.push(Line::from(vec![
                gutter_span("", error),
                Span::styled(message, error),
            ]));
        }
        lines.push(Line::default());
        frame.render_widget(Paragraph::new(lines).style(code), area);

        let (line, column) = self.input.field().cursor_line_col();
        frame.set_cursor_position(Position::new(
            area.x + INPUT_CODE_COL + column as u16,
            area.y + 1 + line as u16,
        ));
    }

    fn draw_code_pane(
        &self,
        frame: &mut Frame,
        area: Rect,
        theme: &Theme,
        highlighter: &mut dyn Highlighter,
        committed: &[String],
    ) {
        let pane = TuiBlock::bordered()
            .border_type(BorderType::Rounded)
            .border_style(theme.border())
            .title(Span::styled(" $code ", theme.prompt()))
            .style(theme.code());
        let inner = pane.inner(area);
        frame.render_widget(pane, area);

        if committed.is_empty() || inner.width == 0 || inner.height == 0 {
            return;
        }

        let spans = highlighter.highlight(&committed.join("\n"));
        let code = CodeBlock::new(committed.to_vec(), spans, 1);
        let context = RenderContext::new(inner.width, theme);
        code.render(&context, inner, frame.buffer_mut());
    }
}

fn fill(buffer: &mut Buffer, style: Style) {
    let area = *buffer.area();
    TuiBlock::default().style(style).render(area, buffer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_wheel_changes_feed_scroll_offset() {
        let mut editor = Editor::new(String::new());
        editor.handle_mouse(MouseEventKind::ScrollUp);
        assert_eq!(editor.scroll_offset, SCROLL_STEP);
        editor.handle_mouse(MouseEventKind::ScrollDown);
        assert_eq!(editor.scroll_offset, 0);
    }

    #[test]
    fn code_command_toggles_pane_and_clears_input() {
        let mut editor = Editor::new(String::new());
        editor.input.field_mut().insert_str("$code");

        editor.apply(EditorUpdate::ToggleCode);

        assert!(editor.show_code_pane);
        assert!(editor.submission().is_empty());
    }
}
