//! Application-wide services and the terminal event loop.

use std::time::Duration;

use crossterm::event::Event;
use ratatui::Frame;

use crate::editor::{Editor, EditorAction};
use crate::errors::M2Result;
use crate::highlight::SyntaxHighlighter;
use crate::m2_backend::M2Backend;
use crate::session::Session;
use crate::terminal::ReplTerminal;
use crate::theme::Theme;

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(80);

struct App {
    session: Session,
    editor: Editor,
    theme: Theme,
    highlighter: SyntaxHighlighter,
}

impl App {
    fn new(backend: M2Backend) -> Self {
        let session = Session::new(backend);
        let version = session.version().unwrap_or_else(|_| "unknown".to_string());
        Self {
            session,
            editor: Editor::new(welcome_message(&version)),
            theme: Theme::default(),
            highlighter: SyntaxHighlighter::new(),
        }
    }

    fn handle_event(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) => match self.editor.handle_key(key) {
                EditorAction::Continue => true,
                EditorAction::Submit => {
                    let update = self
                        .session
                        .submit(self.editor.submission(), &mut self.highlighter);
                    self.editor.apply(update);
                    true
                }
                EditorAction::Quit => false,
            },
            Event::Paste(text) => {
                self.editor.paste(&text);
                true
            }
            Event::Mouse(mouse) => {
                self.editor.handle_mouse(mouse.kind);
                true
            }
            _ => true,
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        self.editor.draw(
            frame,
            &self.theme,
            &mut self.highlighter,
            self.session.committed(),
        );
    }
}

fn welcome_message(m2_version: &str) -> String {
    format!(
        "spectral-m2 v{}\nMacaulay2 v{m2_version}",
        env!("CARGO_PKG_VERSION")
    )
}

pub fn run(backend: M2Backend) -> M2Result<()> {
    let mut terminal = ReplTerminal::enter()?;
    let mut app = App::new(backend);
    loop {
        terminal.draw(|frame| app.draw(frame))?;
        let Some(event) = terminal.next_event(EVENT_POLL_INTERVAL)? else {
            continue;
        };
        if !app.handle_event(event) {
            break;
        }
    }
    terminal.leave()
}
