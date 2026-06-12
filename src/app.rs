//! The application: state, the key-driven event loop, and terminal setup.

use std::error::Error;
use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyModifiers,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::widgets::Clear;
use ratatui::{Frame, Terminal};

use crate::cells::{parse_cells, CellParse};
use crate::editor::Editor;
use crate::highlight::SyntaxHighlighter;
use crate::m2_backend::{M2Backend, M2Result};
use crate::session::Session;
use crate::theme::Theme;
use crate::transcript::{clean_m2_output, TranscriptEntry};
use crate::ui;

struct App {
    backend: M2Backend,
    editor: Editor,
    theme: Theme,
    transcript: Vec<TranscriptEntry>,
    highlighter: SyntaxHighlighter,
    session: Session,
    scroll_offset: usize,
}

impl App {
    fn new(backend: M2Backend) -> Self {
        Self {
            backend,
            editor: Editor::new(),
            theme: Theme::default(),
            transcript: vec![TranscriptEntry::System(
                "spectral-m2 — Macaulay2 highlighted REPL".to_string(),
            )],
            highlighter: SyntaxHighlighter::new(),
            session: Session::new(),
            scroll_offset: 0,
        }
    }

    /// Enter: submit when the buffer is syntactically complete, otherwise keep
    /// editing on a fresh line.
    fn on_enter(&mut self) {
        if self.editor.text().trim().is_empty() {
            return;
        }
        match parse_cells(self.editor.text()) {
            CellParse::Incomplete => self.editor.insert_char('\n'),
            CellParse::Complete(cells) => self.submit_cells(cells),
        }
    }

    /// Evaluate each cell in order. An erroring cell is shown but not committed,
    /// so it consumes no number and the next cell reuses it.
    fn submit_cells(&mut self, cells: Vec<String>) {
        self.editor.take_submission(); // record the buffer in history and clear
        for code in cells {
            let number = self.session.next_number();
            self.transcript.push(TranscriptEntry::Input {
                number: number.0,
                code: code.clone(),
            });
            match self.backend.evaluate(&code) {
                Ok(M2Result { is_error, output }) => {
                    let (result, type_info) = clean_m2_output(&output, &code, is_error);
                    self.transcript.push(TranscriptEntry::Output {
                        result,
                        type_info,
                        is_error,
                    });
                    if !is_error {
                        self.session.commit(code);
                    }
                }
                Err(e) => {
                    self.transcript.push(TranscriptEntry::Output {
                        result: format!("Communication error: {e}"),
                        type_info: None,
                        is_error: true,
                    });
                }
            }
        }
        self.scroll_offset = 0;
    }

    /// Handle a key. Returns `false` to quit.
    fn handle_key(&mut self, key: KeyEvent) -> bool {
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => return false,
            (KeyCode::Enter, m)
                if m.contains(KeyModifiers::SHIFT) || m.contains(KeyModifiers::ALT) =>
            {
                self.editor.insert_char('\n');
            }
            (KeyCode::Enter, _) => self.on_enter(),
            (KeyCode::Char(ch), _) => self.editor.insert_char(ch),
            (KeyCode::Backspace, _) => self.editor.backspace(),
            (KeyCode::Delete, _) => self.editor.delete_forward(),
            (KeyCode::Left, _) => self.editor.move_left(),
            (KeyCode::Right, _) => self.editor.move_right(),
            (KeyCode::Home, _) => self.editor.move_home(),
            (KeyCode::End, _) => self.editor.move_end(),
            (KeyCode::Up, _) => {
                if self.editor.cursor_on_first_line() {
                    self.editor.history_prev();
                } else {
                    self.editor.move_up();
                }
            }
            (KeyCode::Down, _) => {
                if self.editor.cursor_on_last_line() {
                    self.editor.history_next();
                } else {
                    self.editor.move_down();
                }
            }
            (KeyCode::PageUp, _) => self.scroll_offset = self.scroll_offset.saturating_add(5),
            (KeyCode::PageDown, _) => self.scroll_offset = self.scroll_offset.saturating_sub(5),
            (KeyCode::Tab, _) => self.editor.insert_char('\t'),
            _ => {}
        }
        true
    }

    fn draw(&mut self, frame: &mut Frame) {
        let size = frame.area();
        let input_height = ui::calculate_input_height(&self.editor);
        let transcript_height = size.height.saturating_sub(input_height);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(transcript_height),
                Constraint::Length(input_height),
            ])
            .split(size);

        // Reset every cell first: stale glyphs from prior frames would otherwise
        // show through the background fills as ghost rectangles.
        frame.render_widget(Clear, size);
        ui::draw_transcript(
            frame,
            chunks[0],
            &self.transcript,
            &mut self.highlighter,
            &self.theme,
            self.scroll_offset,
        );
        ui::draw_input_area(
            frame,
            chunks[1],
            &self.editor,
            &mut self.highlighter,
            &self.theme,
        );
    }
}

struct ReplTerminal {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    active: bool,
}

impl ReplTerminal {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self {
            terminal,
            active: true,
        })
    }

    fn leave(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        disable_raw_mode()?;
        execute!(
            self.terminal.backend_mut(),
            PopKeyboardEnhancementFlags,
            DisableBracketedPaste,
            LeaveAlternateScreen
        )?;
        self.terminal.show_cursor()?;
        self.active = false;
        Ok(())
    }
}

impl Drop for ReplTerminal {
    fn drop(&mut self) {
        if self.active {
            let _ = disable_raw_mode();
            let _ = execute!(
                self.terminal.backend_mut(),
                PopKeyboardEnhancementFlags,
                DisableBracketedPaste,
                LeaveAlternateScreen
            );
            let _ = self.terminal.show_cursor();
        }
    }
}

pub fn run(backend: M2Backend) -> Result<(), Box<dyn Error>> {
    let mut app = App::new(backend);
    let mut terminal = ReplTerminal::enter()?;
    loop {
        terminal.terminal.draw(|frame| app.draw(frame))?;
        if !event::poll(Duration::from_millis(80))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) => {
                if !app.handle_key(key) {
                    break;
                }
            }
            Event::Paste(text) => {
                for ch in text.chars() {
                    app.editor.insert_char(ch);
                }
            }
            _ => {}
        }
    }
    terminal.leave()?;
    Ok(())
}
