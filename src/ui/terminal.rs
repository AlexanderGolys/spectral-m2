//! Crossterm terminal lifecycle and event input.

use std::fmt;
use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, Event, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, Command};
use ratatui::backend::CrosstermBackend;
use ratatui::{Frame, Terminal};

use crate::errors::Result;
use crate::ui::graphics;

/// xterm's modifyOtherKeys at the given level. tmux forwards modified keys only to a
/// pane that asks this way; it ignores the kitty keyboard flags, so without this
/// request Shift+Enter reaches the application as a bare Enter.
struct ModifyOtherKeys(u8);

impl ModifyOtherKeys {
    /// Every modified key, Shift+Enter included, arrives with its modifiers.
    const REPORT_ALL: Self = Self(2);
    const RESET: Self = Self(0);
}

impl Command for ModifyOtherKeys {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[>4;{}m", self.0)
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Ok(())
    }
}

/// A raw-mode alternate-screen terminal restored automatically on drop.
pub struct ReplTerminal {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    active: bool,
}

impl ReplTerminal {
    const SCROLL_BUFFER: Duration = Duration::from_millis(8);

    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
            // Focus returning is when tmux needs the pictures it dropped resent.
            EnableFocusChange,
            // Disambiguation deliberately leaves Enter as legacy 0x0d; all-keys
            // reporting is what preserves Shift for Shift+Enter.
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            ),
            ModifyOtherKeys::REPORT_ALL
        )?;
        graphics::detect();
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self {
            terminal,
            active: true,
        })
    }

    /// Draw a frame, then send the pictures its placeholders are the first to show.
    pub fn draw(&mut self, render: impl FnOnce(&mut Frame)) -> Result<()> {
        self.terminal.draw(render)?;
        let pictures = graphics::unsent();
        if !pictures.is_empty() {
            let backend = self.terminal.backend_mut();
            backend.write_all(pictures.as_bytes())?;
            backend.flush()?;
        }
        Ok(())
    }

    /// Read one event, coalescing a burst of wheel reports before the next frame.
    pub fn next_events(&self, timeout: Duration) -> Result<Vec<Event>> {
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

    pub fn leave(&mut self) -> Result<()> {
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
            ModifyOtherKeys::RESET,
            DisableFocusChange,
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
