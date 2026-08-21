//! Crossterm terminal lifecycle and event input.

use std::io::{self, Stdout};
use std::time::Duration;

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

use crate::errors::M2Result;

pub struct ReplTerminal {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    active: bool,
}

impl ReplTerminal {
    pub fn enter() -> M2Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self {
            terminal,
            active: true,
        })
    }

    pub fn draw(&mut self, render: impl FnOnce(&mut Frame)) -> M2Result<()> {
        self.terminal.draw(render)?;
        Ok(())
    }

    pub fn next_event(&self, timeout: Duration) -> M2Result<Option<Event>> {
        if event::poll(timeout)? {
            Ok(Some(event::read()?))
        } else {
            Ok(None)
        }
    }

    pub fn leave(&mut self) -> M2Result<()> {
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
        if self.active {
            let _ = disable_raw_mode();
            let _ = execute!(
                self.terminal.backend_mut(),
                PopKeyboardEnhancementFlags,
                DisableMouseCapture,
                DisableBracketedPaste,
                LeaveAlternateScreen
            );
            let _ = self.terminal.show_cursor();
        }
    }
}
