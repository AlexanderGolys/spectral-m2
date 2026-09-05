//! Crossterm terminal lifecycle and event input.

use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::{Frame, Terminal};

use crate::errors::Result;

/// A raw-mode alternate-screen terminal restored automatically on drop.
pub(crate) struct ReplTerminal {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    active: bool,
}

impl ReplTerminal {
    const SCROLL_BUFFER: Duration = Duration::from_millis(8);

    pub(crate) fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
            // Disambiguation deliberately leaves Enter as legacy 0x0d; all-keys
            // reporting is what preserves Shift for Shift+Enter.
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            )
        )?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self {
            terminal,
            active: true,
        })
    }

    pub(crate) fn draw(&mut self, render: impl FnOnce(&mut Frame)) -> Result<()> {
        self.terminal.draw(render)?;
        Ok(())
    }

    /// Read one event, coalescing a burst of wheel reports before the next frame.
    pub(crate) fn next_events(&self, timeout: Duration) -> Result<Vec<Event>> {
        if !event::poll(timeout)? {
            return Ok(Vec::new());
        }
        let first = event::read()?;
        let scroll = matches!(
            first,
            Event::Mouse(mouse)
                if matches!(mouse.kind, event::MouseEventKind::ScrollUp | event::MouseEventKind::ScrollDown)
        );
        let mut events = vec![first];
        if scroll {
            let deadline = Instant::now() + Self::SCROLL_BUFFER;
            while let Some(timeout) = deadline.checked_duration_since(Instant::now()) {
                if !event::poll(timeout)? {
                    break;
                }
                events.push(event::read()?);
            }
        }
        Ok(events)
    }

    pub(crate) fn leave(&mut self) -> Result<()> {
        self.restore()
    }

    fn restore(&mut self) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        disable_raw_mode()?;
        execute!(
            self.terminal.backend_mut(),
            PopKeyboardEnhancementFlags,
            DisableMouseCapture,
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
        let _ = self.restore();
    }
}
