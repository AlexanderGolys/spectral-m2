//! An independently editable, automatically saved notebook beside the session.

use std::fs;
use std::path::PathBuf;

use crossterm::event::MouseEvent;
use ratatui::layout::Rect;
use ratatui::Frame;

use super::Editor;
use super::Reaction;
use crate::code::file;
use crate::code::highlight::Highlighter;
use crate::input::{InputAction, InputCode, InputContext};
use crate::ui::action::Action;
use crate::ui::block::HLSpan;
use crate::ui::key::KeyEvent;
use crate::ui::render::RenderContext;
use crate::ui::settings::session::SessionSettings;
use crate::ui::text_object::TextDirection;

/// A notebook such as a list of computations to try, stored in `notes.md`.
/// It uses the input's editing and undo operations without ever submitting code.
pub struct Notes {
    input: InputCode,
    path: Option<PathBuf>,
    saved: String,
}

impl Default for Notes {
    fn default() -> Self {
        Self {
            input: InputCode::new(),
            path: None,
            saved: String::new(),
        }
    }
}

struct PlainText;

impl Highlighter for PlainText {
    fn highlight(&mut self, _: &str) -> Vec<HLSpan> {
        Vec::new()
    }
}

impl Notes {
    fn refresh(&mut self) {
        let path = SessionSettings::directory().map(|root| root.join("notes.md"));
        if self.path != path {
            self.save();
            if self.saved != self.input.text() {
                return;
            }
            self.path = path;
            self.saved = self
                .path
                .as_ref()
                .and_then(|path| fs::read_to_string(path).ok())
                .unwrap_or_default();
            self.input.restore(self.saved.clone());
        }
    }

    fn save(&mut self) {
        if self.input.text() == self.saved {
            return;
        }
        let result = self
            .path
            .as_ref()
            .ok_or_else(|| "no notes storage directory available".to_string())
            .and_then(|path| {
                file::write(path, self.input.text()).map_err(|error| error.to_string())
            });
        match result {
            Ok(()) => self.saved = self.input.text().to_string(),
            Err(error) => self.input.set_diagnostic(error, None),
        }
    }
}

impl Editor {
    /// Edit the notebook using the same remappable editing keys as the input.
    pub fn handle_notes_key(&mut self, key: KeyEvent) -> Reaction {
        if self.picker_open() {
            return self.handle_preview_key(key);
        }
        self.notes.refresh();
        let edit = self.keymaps.edit(key);
        match edit {
            Some(InputAction::MoveUp) => self
                .notes
                .input
                .field_mut()
                .move_vertically(TextDirection::Backward),
            Some(InputAction::MoveDown) => self
                .notes
                .input
                .field_mut()
                .move_vertically(TextDirection::Forward),
            Some(InputAction::HistoryPrevious | InputAction::HistoryNext) => {}
            _ => {
                self.notes.input.handle_edit_key(key, edit);
            }
        }
        self.notes.save();
        Reaction::Continue
    }

    /// Notebook-owned application actions; other actions retain their usual meaning.
    pub fn notes_action(&mut self, action: Action) -> Option<Reaction> {
        if self.picker_open() {
            return None;
        }
        self.notes.refresh();
        let reaction = match action {
            Action::Submit => {
                self.notes.input.edit(InputAction::InsertNewline);
                Reaction::Continue
            }
            Action::CopySelection => self
                .notes
                .input
                .field()
                .selected_text()
                .map_or(Reaction::Continue, |text| Reaction::Copy(text.to_string())),
            Action::CutSelection => self
                .notes
                .input
                .field_mut()
                .cut()
                .map_or(Reaction::Continue, Reaction::Copy),
            _ => return None,
        };
        self.notes.save();
        Some(reaction)
    }

    /// Paste text into the notebook without interpreting mathematical glyphs as code.
    pub fn paste_notes(&mut self, text: &str) {
        self.notes.refresh();
        self.notes
            .input
            .field_mut()
            .insert_str(&text.replace("\r\n", "\n").replace('\r', "\n"));
        self.notes.save();
    }

    /// Mouse selection and cursor placement inside the notebook.
    pub fn notes_mouse(&mut self, event: MouseEvent) -> Reaction {
        let (_, copied) = self.notes.input.handle_mouse(event);
        copied.map_or(Reaction::Continue, Reaction::Copy)
    }

    /// Draw the notebook and keep its caret visible while it has focus.
    pub fn draw_notes(&mut self, frame: &mut Frame, area: Rect, context: &RenderContext) {
        self.notes.refresh();
        self.notes.input.set_settings(self.settings);
        let mut highlighter = PlainText;
        let focused = self.pane_focused && !self.picker_open();
        if let Some(cursor) = self.notes.input.render(
            InputContext {
                render: context,
                highlighter: &mut highlighter,
                command: false,
                debugging: false,
                focused,
                pending: None,
            },
            area,
            frame.buffer_mut(),
        ) {
            frame.set_cursor_position(cursor);
        }
    }
}
