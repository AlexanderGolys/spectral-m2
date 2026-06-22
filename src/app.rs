//! The application: state, the key-driven event loop, terminal setup, and the
//! submit pipeline that turns one input into a sequence of typed feed blocks.

use std::collections::HashSet;
use std::error::Error;
use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyModifiers,
        KeyboardEnhancementFlags, MouseEventKind, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block as TuiBlock, Clear, Paragraph, Widget},
    Frame, Terminal,
};

use crate::block::highlight::Highlighter;
use crate::block::render::gutter_span;
use crate::block::{
    Block, Cell, CodeBlock, Highlighted, OutputBlock, RejectedBlock, RuntimeErrorBlock,
    SyntaxErrorBlock, SystemBlock, TextContainer, WarningBlock,
};
use crate::cells::{has_syntax_error, split_cells};
use crate::commands::{parse_command, ReplCommand, COMMAND_PREFIX};
use crate::highlight::SyntaxHighlighter;
use crate::input::{InputCode, INPUT_CODE_COL};
use crate::m2_backend::{M2Backend, M2Result, ParseVerdict};
use crate::theme::Theme;
use crate::transcript::{clean_m2_output, diagnostic_message, is_diagnostic, strip_m2_location};

const BANNER: &str = "spectral-m2";
const MAX_INPUT_LINES: u16 = 8;
const SCROLL_STEP: usize = 5;

struct App {
    backend: M2Backend,
    highlighter: SyntaxHighlighter,
    theme: Theme,
    feed: Vec<Box<dyn Block>>,
    input: InputCode,
    next_number: u32,
    scroll_offset: usize,
    parse_error: Option<String>,
}

impl App {
    fn new(mut backend: M2Backend) -> Result<Self, Box<dyn Error>> {
        let version = backend.version().unwrap_or_else(|_| "unknown".to_string());
        let banner = format!("{} (M2 {})", BANNER, version);
        Ok(Self {
            backend,
            highlighter: SyntaxHighlighter::new(),
            theme: Theme::default(),
            feed: vec![Box::new(SystemBlock { message: banner })],
            input: InputCode::new(),
            next_number: 1,
            scroll_offset: 0,
            parse_error: None,
        })
    }

    /// Enter: submit when the buffer is syntactically complete, keep editing (insert a
    /// newline) when it is incomplete, otherwise surface the syntax error.
    fn on_enter(&mut self) {
        let text = self.input.text().trim().to_string();
        if text.is_empty() {
            return;
        }
        if !text.contains('\n') && text.starts_with(COMMAND_PREFIX) {
            self.dispatch_command(&text);
            return;
        }
        match self.backend.check(&text) {
            Ok(ParseVerdict::Incomplete) => self.input.field_mut().insert_char('\n'),
            Ok(ParseVerdict::SyntaxError(msg)) => {
                self.parse_error = Some(strip_m2_location(&msg).to_string());
            }
            Ok(ParseVerdict::Complete) if has_syntax_error(&text) => {
                // `parse` accepts it but the strict reader does not (e.g. `;;`): the
                // whole submission is rejected as one block, never sent to M2.
                let code = self.input.take_submission().unwrap_or(text);
                self.feed.push(Box::new(SyntaxErrorBlock {
                    code,
                    message: "syntax error".to_string(),
                }));
                self.scroll_offset = 0;
            }
            Ok(ParseVerdict::Complete) => {
                let source = self.input.take_submission().unwrap_or(text);
                self.run_submission(&source);
            }
            Err(e) => self.parse_error = Some(format!("Communication error: {e}")),
        }
    }

    /// Run a `$`-prefixed REPL command. Recognised commands act on frontend state only.
    fn dispatch_command(&mut self, input: &str) {
        match parse_command(input) {
            Some(ReplCommand::ToggleCode) => {
                self.input.take_submission();
                self.feed.push(Box::new(SystemBlock {
                    message: "the $code pane is temporarily disabled during the restructure"
                        .to_string(),
                }));
                self.scroll_offset = 0;
            }
            Some(ReplCommand::Unknown(name)) => {
                self.parse_error = Some(format!("unknown command: ${name}"));
            }
            None => {}
        }
    }

    /// Split a complete submission into cells, evaluate them in order, and append the
    /// resulting typed blocks. All-success → a [`CodeBlock`] then the per-cell outputs.
    /// A runtime error → the successful prefix and its outputs, a [`RuntimeErrorBlock`]
    /// for the failing cell, and a [`RejectedBlock`] for the never-run tail.
    fn run_submission(&mut self, source: &str) {
        let ranges = split_cells(source);
        let start = self.next_number;
        let mut success: Vec<(String, String)> = Vec::new(); // (code, raw output)
        let mut failure: Option<(usize, String)> = None; // (cell index, raw output)

        for (i, range) in ranges.iter().enumerate() {
            let code = source[range.clone()].to_string();
            match self.backend.evaluate(&code) {
                Ok(M2Result { is_error, output }) if is_error => {
                    failure = Some((i, output));
                    break;
                }
                Ok(M2Result { output, .. }) => success.push((code, output)),
                Err(e) => {
                    failure = Some((i, format!("Communication error: {e}")));
                    break;
                }
            }
        }
        // Only successful cells consume a number; a failing cell's number is reused next.
        self.next_number = start + success.len() as u32;

        if !success.is_empty() {
            let outputs = self.build_outputs(&success, start);
            let codes: Vec<&str> = success.iter().map(|(c, _)| c.as_str()).collect();
            let spans = self.highlighter.highlight(&codes.join("\n"));
            let cells: Vec<Cell> = success
                .iter()
                .map(|(c, _)| Cell { code: c.clone() })
                .collect();
            // INVARIANT: Each CodeBlock with start_number N and K cells produces
            // output blocks (OutputBlock or RuntimeErrorBlock) numbered N..N+K-1.
            // WarningBlocks don't have numbers and precede their OutputBlock.
            self.feed.push(Box::new(CodeBlock {
                cells,
                spans,
                start_number: start,
            }));
            self.feed.extend(outputs);
        }

        if let Some((idx, raw)) = failure {
            let code = source[ranges[idx].clone()].to_string();
            let spans = self.highlighter.highlight(&code);
            let (message, _) = clean_m2_output(&raw, &code, true);
            self.feed.push(Box::new(RuntimeErrorBlock {
                number: self.next_number,
                cell: Cell { code },
                spans,
                message,
            }));

            let tail: Vec<Cell> = ranges[idx + 1..]
                .iter()
                .map(|r| Cell {
                    code: source[r.clone()].to_string(),
                })
                .collect();
            if !tail.is_empty() {
                let codes: Vec<&str> = tail.iter().map(|c| c.code.as_str()).collect();
                let spans = self.highlighter.highlight(&codes.join("\n"));
                self.feed
                    .push(Box::new(RejectedBlock { cells: tail, spans }));
            }
        }
        self.scroll_offset = 0;
    }

    /// Turn each successful cell's raw output into its display blocks: a [`WarningBlock`]
    /// per distinct diagnostic, then an [`OutputBlock`] for the value (suppressed cells —
    /// empty value, no type — yield nothing).
    fn build_outputs(&self, success: &[(String, String)], start: u32) -> Vec<Box<dyn Block>> {
        let mut blocks: Vec<Box<dyn Block>> = Vec::new();
        for (k, (code, raw)) in success.iter().enumerate() {
            let number = start + k as u32;
            let (value, ty) = clean_m2_output(raw, code, false);

            let mut seen = HashSet::new();
            let mut value_lines: Vec<&str> = Vec::new();
            for line in value.lines() {
                if is_diagnostic(line) {
                    let message = diagnostic_message(strip_m2_location(line)).to_string();
                    if seen.insert(message.clone()) {
                        blocks.push(Box::new(WarningBlock { message }));
                    }
                } else {
                    value_lines.push(line);
                }
            }
            while value_lines.first().is_some_and(|l| l.trim().is_empty()) {
                value_lines.remove(0);
            }
            while value_lines.last().is_some_and(|l| l.trim().is_empty()) {
                value_lines.pop();
            }

            let value = value_lines.join("\n");
            if !value.is_empty() || ty.is_some() {
                blocks.push(Box::new(OutputBlock { number, value, ty }));
            }
        }
        blocks
    }

    /// Handle a key. Returns `false` to quit.
    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if matches!(
            key.code,
            KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete | KeyCode::Tab | KeyCode::Enter
        ) {
            self.parse_error = None;
        }
        let field = self.input.field_mut();
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => return false,
            (KeyCode::Enter, m)
                if m.contains(KeyModifiers::SHIFT) || m.contains(KeyModifiers::ALT) =>
            {
                field.insert_char('\n');
            }
            (KeyCode::Enter, _) => self.on_enter(),
            (KeyCode::Char(ch), _) => field.insert_char(ch),
            (KeyCode::Backspace, _) => field.backspace(),
            (KeyCode::Delete, _) => field.delete_forward(),
            (KeyCode::Left, _) => field.move_left(),
            (KeyCode::Right, _) => field.move_right(),
            (KeyCode::Home, _) => field.move_home(),
            (KeyCode::End, _) => field.move_end(),
            (KeyCode::Up, _) => {
                if field.cursor_on_first_line() {
                    field.history_prev();
                } else {
                    field.move_up();
                }
            }
            (KeyCode::Down, _) => {
                if field.cursor_on_last_line() {
                    field.history_next();
                } else {
                    field.move_down();
                }
            }
            (KeyCode::PageUp, _) => {
                self.scroll_offset = self.scroll_offset.saturating_add(SCROLL_STEP)
            }
            (KeyCode::PageDown, _) => {
                self.scroll_offset = self.scroll_offset.saturating_sub(SCROLL_STEP)
            }
            (KeyCode::Tab, _) => field.insert_char('\t'),
            _ => {}
        }
        true
    }

    /// The pinned input area's height: a top and bottom margin around the prompt lines,
    /// plus a hint row when a syntax error is shown, clamped to a sane band.
    fn input_height(&self) -> u16 {
        let lines = if self.input.is_empty() {
            1
        } else {
            self.input.text().split('\n').count() as u16
        };
        let extra = if self.parse_error.is_some() { 1 } else { 0 };
        (lines + 2 + extra).clamp(3, MAX_INPUT_LINES + 2 + extra)
    }

    fn draw(&mut self, frame: &mut Frame) {
        self.input.rehighlight(&mut self.highlighter);

        let size = frame.area();
        frame.render_widget(Clear, size);

        let input_height = self.input_height();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(size.height.saturating_sub(input_height)),
                Constraint::Length(input_height),
            ])
            .split(size);

        self.draw_feed(frame, chunks[0]);
        self.draw_input(frame, chunks[1]);
    }

    /// Composite the whole feed into an off-screen buffer of its full height, then blit
    /// the visible window — so each block draws as a real widget while the feed scrolls
    /// as one region. Defaults to the bottom; PageUp/Down scroll up from there.
    fn draw_feed(&self, frame: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let heights: Vec<u16> = self.feed.iter().map(|b| b.height(area.width)).collect();
        let total: u16 = heights.iter().sum();
        let buf_h = total.max(area.height).max(1);

        let mut comp = Buffer::empty(Rect::new(0, 0, area.width, buf_h));
        fill(&mut comp, self.theme.ui.transcript_bg);
        let mut y = 0;
        for (block, h) in self.feed.iter().zip(&heights) {
            block.render(Rect::new(0, y, area.width, *h), &mut comp, &self.theme);
            y += h;
        }

        let visible = area.height as usize;
        let max_scroll = (total as usize).saturating_sub(visible);
        let top = max_scroll.saturating_sub(self.scroll_offset);
        let dst = frame.buffer_mut();
        for dy in 0..area.height {
            let src_y = top + dy as usize;
            if src_y >= buf_h as usize {
                break;
            }
            for x in 0..area.width {
                if let Some(cell) = comp.cell((x, src_y as u16)) {
                    if let Some(slot) = dst.cell_mut((area.x + x, area.y + dy)) {
                        *slot = cell.clone();
                    }
                }
            }
        }
    }

    fn draw_input(&self, frame: &mut Frame, area: Rect) {
        let code_bg = self.theme.ui.code_bg;
        let mut lines = self.input.highlighted_lines(area.width, &self.theme);
        if let Some(msg) = &self.parse_error {
            let err = self.theme.ui.error.patch(code_bg);
            lines.push(Line::from(vec![
                gutter_span("", err),
                Span::styled(msg.clone(), err),
            ]));
        }
        lines.push(Line::default());
        frame.render_widget(Paragraph::new(lines).style(code_bg), area);

        let (line, col) = self.input.field().cursor_line_col();
        frame.set_cursor_position(Position::new(
            area.x + INPUT_CODE_COL + col as u16,
            area.y + 1 + line as u16,
        ));
    }
}

/// Paint `style` across the whole of `buf` — the feed's base background.
fn fill(buf: &mut Buffer, style: Style) {
    let area = *buf.area();
    TuiBlock::default().style(style).render(area, buf);
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
    let mut app = App::new(backend)?;
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
            Event::Paste(text) => app.input.field_mut().insert_str(&text),
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => {
                    app.scroll_offset = app.scroll_offset.saturating_add(SCROLL_STEP)
                }
                MouseEventKind::ScrollDown => {
                    app.scroll_offset = app.scroll_offset.saturating_sub(SCROLL_STEP)
                }
                _ => {}
            },
            _ => {}
        }
    }
    terminal.leave()?;
    Ok(())
}
