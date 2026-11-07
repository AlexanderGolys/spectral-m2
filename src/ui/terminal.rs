//! Crossterm terminal lifecycle and event input.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Stdout, Write};
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
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
use nix::unistd::dup2;
use ratatui::backend::CrosstermBackend;
use ratatui::{Frame, Terminal};

use crate::errors::Result;
use crate::ui::graphics;

/// Where stderr goes while the screen is drawn.
const STDERR_SINK: &str = "/dev/null";

/// Stderr pointed away from the terminal, where whatever a library prints would land
/// at the cursor and stay on the screen; pointed back on drop.
struct SilencedStderr {
    original: OwnedFd,
}

impl SilencedStderr {
    fn new() -> io::Result<Self> {
        let stderr = io::stderr();
        let original = stderr.as_fd().try_clone_to_owned()?;
        let sink: File = OpenOptions::new().write(true).open(STDERR_SINK)?;
        dup2(sink.as_raw_fd(), stderr.as_raw_fd())?;
        Ok(Self { original })
    }
}

impl Drop for SilencedStderr {
    fn drop(&mut self) {
        let _ = dup2(self.original.as_raw_fd(), io::stderr().as_raw_fd());
    }
}

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
    /// Held while the screen is drawn; none when stderr could not be redirected.
    silenced_stderr: Option<SilencedStderr>,
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
            silenced_stderr: SilencedStderr::new().ok(),
        })
    }

    /// Draw a frame, then send the pictures encoded so far that the terminal lacks,
    /// and free those it no longer needs.
    pub fn draw(&mut self, render: impl FnOnce(&mut Frame)) -> Result<()> {
        let frame = self.terminal.draw(render)?;
        let pictures = graphics::synchronise(frame.buffer);
        self.write_pictures(&pictures)
    }

    /// Send the pictures encoded since the last frame, which need no frame to show
    /// them: their placeholders are already drawn.
    pub fn send_pictures(&mut self) -> Result<()> {
        let pictures = graphics::send_encoded();
        self.write_pictures(&pictures)
    }

    fn write_pictures(&mut self, pictures: &str) -> Result<()> {
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
        self.silenced_stderr = None;
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

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use super::*;

    fn stderr_device() -> u64 {
        let stderr = io::stderr().as_fd().try_clone_to_owned().unwrap();
        File::from(stderr).metadata().unwrap().rdev()
    }

    #[test]
    fn stderr_goes_to_the_sink_until_it_is_restored() {
        let sink = File::open(STDERR_SINK).unwrap().metadata().unwrap().rdev();
        let silenced = SilencedStderr::new().unwrap();
        let during = stderr_device();
        drop(silenced);
        assert_eq!(during, sink);
        let original = stderr_device();
        let silenced = SilencedStderr::new().unwrap();
        drop(silenced);
        assert_eq!(stderr_device(), original);
    }
}
