use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyModifiers,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Terminal;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use crate::m2_backend::{M2Backend, M2Result};

const USER_BG: Color = Color::Rgb(20, 22, 30);
const OUTPUT_BG: Color = Color::Rgb(34, 36, 40);
const ERROR_BG: Color = Color::Rgb(36, 18, 18);
const LINE_NUMBER_FG: Color = Color::DarkGray;
const INPUT_BOTTOM_GAP: u16 = 1;
const MAX_INPUT_HEIGHT: u16 = 8;
const INPUT_PLACEHOLDER: &str = "Type M2 code...";

const HIGHLIGHT_NAMES: &[&str] = &[
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "function",
    "function.builtin",
    "keyword",
    "keyword.conditional",
    "keyword.debug",
    "keyword.exception",
    "keyword.modifier",
    "keyword.operator",
    "keyword.repeat",
    "keyword.return",
    "number",
    "operator",
    "property",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "string",
    "string.special",
    "string.special.url",
    "string.regexp",
    "type",
    "variable",
    "variable.builtin",
    "variable.parameter",
];

const M2_HIGHLIGHTS_QUERY: &str = include_str!("../queries/highlights.scm");

enum TranscriptEntry {
    Input {
        number: usize,
        code: String,
    },
    Output {
        number: usize,
        result: String,
        type_info: Option<String>,
        is_error: bool,
    },
    System(String),
}

struct App {
    backend: M2Backend,
    input: String,
    input_cursor: usize,
    input_number: usize,
    transcript: Vec<TranscriptEntry>,
    highlighter: SyntaxHighlighter,
    scroll_offset: usize,
}

impl App {
    fn new(backend: M2Backend) -> Self {
        let input_number = backend.next_input_number();
        Self {
            backend,
            input: String::new(),
            input_cursor: 0,
            input_number,
            transcript: vec![TranscriptEntry::System(format!(
                "pretty-shell — Macaulay2 highlighted REPL"
            ))],
            highlighter: SyntaxHighlighter::new(),
            scroll_offset: 0,
        }
    }

    fn submit_input(&mut self) -> Option<()> {
        let code = self.input.trim().to_string();
        if code.is_empty() {
            return Some(());
        }
        let number = self.input_number;

        self.transcript.push(TranscriptEntry::Input { number, code: self.input.clone() });

        match self.backend.evaluate(self.input.trim()) {
            Ok(M2Result { is_error, output }) => {
                let (result_text, type_info) = parse_m2_output(&output);
                self.transcript.push(TranscriptEntry::Output {
                    number,
                    result: result_text,
                    type_info,
                    is_error,
                });
            }
            Err(e) => {
                self.transcript.push(TranscriptEntry::Output {
                    number,
                    result: format!("Communication error: {}", e),
                    type_info: None,
                    is_error: true,
                });
            }
        }

        self.input_number = self.backend.next_input_number();
        self.input.clear();
        self.input_cursor = 0;
        self.scroll_offset = 0;
        Some(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => {
                return false;
            }
            (KeyCode::Enter, modifiers) if modifiers.contains(KeyModifiers::SHIFT)
                || modifiers.contains(KeyModifiers::ALT) =>
            {
                self.insert_input_char('\n');
            }
            (KeyCode::Enter, _) => {
                self.submit_input();
            }
            (KeyCode::Char(ch), _) => {
                self.insert_input_char(ch);
            }
            (KeyCode::Backspace, _) => {
                self.delete_input_char_back();
            }
            (KeyCode::Delete, _) => {
                self.delete_input_char_forward();
            }
            (KeyCode::Left, _) => {
                if self.input_cursor > 0 {
                    self.input_cursor -= 1;
                }
            }
            (KeyCode::Right, _) => {
                if self.input_cursor < self.input.len() {
                    self.input_cursor += 1;
                }
            }
            (KeyCode::Home, _) => {
                self.input_cursor = 0;
            }
            (KeyCode::End, _) => {
                self.input_cursor = self.input.len();
            }
            (KeyCode::Up, _) => {
                self.move_cursor_up();
            }
            (KeyCode::Down, _) => {
                self.move_cursor_down();
            }
            (KeyCode::PageUp, _) => {
                self.scroll_offset = self.scroll_offset.saturating_add(5);
            }
            (KeyCode::PageDown, _) => {
                self.scroll_offset = self.scroll_offset.saturating_sub(5);
            }
            (KeyCode::Tab, _) => {
                self.insert_input_char('\t');
            }
            _ => {}
        }
        true
    }

    fn insert_input_char(&mut self, ch: char) {
        self.input.insert(self.input_cursor, ch);
        self.input_cursor += ch.len_utf8();
    }

    fn delete_input_char_back(&mut self) {
        if self.input_cursor > 0 {
            let prev = self.input[..self.input_cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.input.replace_range(prev..self.input_cursor, "");
            self.input_cursor = prev;
        }
    }

    fn delete_input_char_forward(&mut self) {
        if self.input_cursor < self.input.len() {
            let next = self.input[self.input_cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.input_cursor + i)
                .unwrap_or(self.input.len());
            self.input.replace_range(self.input_cursor..next, "");
        }
    }

    fn move_cursor_up(&mut self) {
        let line_starts: Vec<usize> = self
            .input
            .char_indices()
            .filter(|&(_, ch)| ch == '\n')
            .map(|(i, _)| i + 1)
            .collect();

        if line_starts.is_empty() {
            return;
        }

        let current_line = line_starts
            .iter()
            .position(|&start| start > self.input_cursor)
            .unwrap_or(line_starts.len());

        if current_line > 0 {
            let target_line_start = if current_line == 0 { 0 } else { line_starts[current_line - 1] };
            self.input_cursor = target_line_start;
        } else {
            self.input_cursor = 0;
        }
    }

    fn move_cursor_down(&mut self) {
        let line_starts: Vec<usize> = self
            .input
            .char_indices()
            .filter(|&(_, ch)| ch == '\n')
            .map(|(i, _)| i + 1)
            .collect();

        let current_line = line_starts
            .iter()
            .position(|&start| start > self.input_cursor)
            .unwrap_or(line_starts.len());

        if current_line < line_starts.len() {
            let target_line_start = line_starts[current_line];
            self.input_cursor = target_line_start;
        } else {
            self.input_cursor = self.input.len();
        }
    }

    fn draw(&mut self, frame: &mut ratatui::Frame) {
        let size = frame.area();
        let input_height = self.calculate_input_height();
        let transcript_height = size.height.saturating_sub(input_height);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(transcript_height),
                Constraint::Length(input_height),
            ])
            .split(size);

        self.draw_transcript(frame, chunks[0]);
        self.draw_input_area(frame, chunks[1]);
    }

    fn calculate_input_height(&self) -> u16 {
        if self.input.is_empty() {
            return 2 + INPUT_BOTTOM_GAP;
        }
        let lines = self.input.lines().count().max(1) as u16;
        (lines + INPUT_BOTTOM_GAP + 1).min(MAX_INPUT_HEIGHT + 2).max(2)
    }

    fn draw_transcript(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let mut lines: Vec<Line<'static>> = Vec::new();

        for entry in &self.transcript {
            match entry {
                TranscriptEntry::Input { number, code } => {
                    let prompt = format!("i{} : ", number);
                    for (i, line) in code.lines().enumerate() {
                        let highlighted = self.highlighter.highlight_m2(line);
                        let spans = highlight_to_spans(&highlighted);
                        if i == 0 {
                            let mut line_spans = vec![Span::styled(
                                prompt.clone(),
                                Style::default().fg(Color::Cyan),
                            )];
                            line_spans.extend(spans);
                            lines.push(Line::from(line_spans));
                        } else {
                            let mut line_spans = vec![Span::styled(
                                "     ".to_string(),
                                Style::default().fg(LINE_NUMBER_FG),
                            )];
                            line_spans.extend(spans);
                            lines.push(Line::from(line_spans));
                        }
                    }
                }
                TranscriptEntry::Output {
                    number,
                    result,
                    type_info,
                    is_error,
                } => {
                    let bg = if *is_error { ERROR_BG } else { OUTPUT_BG };
                    let result_fg = if *is_error {
                        Color::LightRed
                    } else {
                        Color::White
                    };

                    for (i, line) in result.lines().enumerate() {
                        if i == 0 {
                            let mut line_spans = vec![
                                Span::styled(
                                    format!("o{} : ", number),
                                    Style::default().fg(Color::Green),
                                ),
                                Span::styled(line.to_string(), Style::default().fg(result_fg).bg(bg)),
                            ];
                            if let Some(ti) = type_info {
                                line_spans.push(Span::styled(
                                    format!("  {}", ti),
                                    Style::default()
                                        .fg(Color::Yellow)
                                        .bg(bg)
                                        .add_modifier(Modifier::ITALIC),
                                ));
                            }
                            lines.push(Line::from(line_spans));
                        } else {
                            lines.push(Line::from(Span::styled(
                                format!("     {}", line),
                                Style::default().fg(result_fg).bg(bg),
                            )));
                        }
                    }
                    lines.push(Line::default());
                }
                TranscriptEntry::System(msg) => {
                    lines.push(Line::from(Span::styled(
                        msg.to_string(),
                        Style::default().fg(Color::DarkGray),
                    )));
                    lines.push(Line::default());
                }
            }
        }

        let total_lines = lines.len();
        let visible_lines = area.height as usize;
        let scroll = self.scroll_offset.max(
            total_lines.saturating_sub(visible_lines),
        );

        let visible: Vec<Line<'static>> = lines.into_iter().skip(scroll).take(visible_lines).collect();

        let paragraph = Paragraph::new(visible).style(Style::default().bg(USER_BG));
        frame.render_widget(paragraph, area);
    }

    fn draw_input_area(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let prompt = format!("i{} : ", self.input_number);
        let mut lines: Vec<Line<'static>> = Vec::new();

        let input_lines: Vec<&str> = if self.input.is_empty() {
            vec![""]
        } else {
            self.input.lines().collect()
        };

        for (i, line) in input_lines.iter().enumerate() {
            let highlighted = self.highlighter.highlight_m2(line);
            let spans = highlight_to_spans(&highlighted);
            if i == 0 {
                let mut line_spans = vec![Span::styled(
                    prompt.clone(),
                    Style::default().fg(Color::Cyan),
                )];
                line_spans.extend(spans);
                if self.input.is_empty() {
                    line_spans.push(Span::styled(
                        INPUT_PLACEHOLDER.to_string(),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                lines.push(Line::from(line_spans));
            } else {
                let mut line_spans = vec![Span::styled(
                    "     ".to_string(),
                    Style::default().fg(LINE_NUMBER_FG),
                )];
                line_spans.extend(spans);
                lines.push(Line::from(line_spans));
            }
        }

        let paragraph = Paragraph::new(lines).style(Style::default().bg(USER_BG));
        frame.render_widget(paragraph, area);

        let cursor_line = self.input[..self.input_cursor]
            .lines()
            .count()
            .max(1)
            - 1;
        let cursor_col = self.input[..self.input_cursor]
            .lines()
            .last()
            .map(|l| l.chars().count())
            .unwrap_or(0);

        let prompt_width = prompt.chars().count() as u16;
        frame.set_cursor_position(Position::new(
            area.x + prompt_width + cursor_col as u16,
            area.y + cursor_line as u16,
        ));
    }
}

fn parse_m2_output(output: &str) -> (String, Option<String>) {
    let lines: Vec<&str> = output.lines().collect();
    if lines.is_empty() {
        return (String::new(), None);
    }

    let last_line = lines.last().unwrap();
    let type_pattern = " : ";
    if let Some(pos) = last_line.rfind(type_pattern) {
        let type_info = last_line[pos + 3..].to_string();
        if !type_info.is_empty() && lines.len() > 1 {
            let result_lines: Vec<&str> = lines[..lines.len() - 1].to_vec();
            return (result_lines.join("\n"), Some(type_info));
        }
    }

    (output.to_string(), None)
}

struct SyntaxHighlighter {
    config: Option<HighlightConfiguration>,
    highlighter: Highlighter,
}

impl SyntaxHighlighter {
    fn new() -> Self {
        let lang = tree_sitter_macaulay2::language();
        let config = HighlightConfiguration::new(
            lang,
            "macaulay2",
            M2_HIGHLIGHTS_QUERY,
            "",
            "",
        )
        .ok()
        .map(|mut c| {
            c.configure(HIGHLIGHT_NAMES);
            c
        });

        Self {
            config,
            highlighter: Highlighter::new(),
        }
    }

    fn highlight_m2(&mut self, source: &str) -> Vec<(Style, String)> {
        let Some(config) = self.config.as_ref() else {
            return vec![(Style::default(), source.to_string())];
        };
        match highlight_spans(&mut self.highlighter, config, source) {
            Ok(spans) => spans,
            Err(_) => vec![(Style::default(), source.to_string())],
        }
    }
}

fn highlight_spans(
    highlighter: &mut Highlighter,
    config: &HighlightConfiguration,
    source: &str,
) -> Result<Vec<(Style, String)>, tree_sitter_highlight::Error> {
    let mut spans = Vec::new();
    let mut stack = Vec::new();
    let events = highlighter.highlight(config, source.as_bytes(), None, |_| None)?;
    for event in events {
        match event? {
            HighlightEvent::Source { start, end } => {
                let style = stack
                    .last()
                    .copied()
                    .map_or_else(Style::default, highlight_style);
                spans.push((style, source[start..end].to_string()));
            }
            HighlightEvent::HighlightStart(tree_sitter_highlight::Highlight(index)) => {
                stack.push(index);
            }
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
        }
    }
    Ok(spans)
}

fn highlight_to_spans(highlighted: &[(Style, String)]) -> Vec<Span<'static>> {
    highlighted
        .iter()
        .flat_map(|(style, text)| {
            if !text.contains('\n') {
                return vec![Span::styled(text.clone(), *style)];
            }
            let mut result = Vec::new();
            for piece in text.split('\n') {
                if !piece.is_empty() {
                    result.push(Span::styled(piece.to_string(), *style));
                }
            }
            result
        })
        .collect()
}

fn highlight_style(index: usize) -> Style {
    let name = HIGHLIGHT_NAMES.get(index).copied().unwrap_or("");
    match name {
        "comment" => Style::default().fg(Color::DarkGray),
        "constant" | "constant.builtin" => Style::default().fg(Color::LightYellow),
        "constructor" => Style::default().fg(Color::LightCyan),
        "function" | "function.builtin" => Style::default().fg(Color::Blue),
        "keyword" | "keyword.conditional" => Style::default()
            .fg(Color::Rgb(210, 140, 255))
            .add_modifier(Modifier::BOLD),
        "keyword.debug" => Style::default().fg(Color::DarkGray),
        "keyword.exception" => Style::default()
            .fg(Color::Rgb(255, 140, 140))
            .add_modifier(Modifier::BOLD),
        "keyword.modifier" => Style::default()
            .fg(Color::Rgb(180, 140, 255))
            .add_modifier(Modifier::BOLD),
        "keyword.operator" => Style::default().fg(Color::LightMagenta),
        "keyword.repeat" => Style::default()
            .fg(Color::Rgb(200, 140, 255))
            .add_modifier(Modifier::BOLD),
        "keyword.return" => Style::default()
            .fg(Color::Rgb(255, 160, 160))
            .add_modifier(Modifier::BOLD),
        "number" => Style::default().fg(Color::LightGreen),
        "operator" => Style::default().fg(Color::Gray),
        "property" => Style::default().fg(Color::Cyan),
        "punctuation.bracket" | "punctuation.delimiter" | "punctuation.special" => {
            Style::default().fg(Color::Gray)
        }
        "string" => Style::default().fg(Color::Green),
        "string.special" | "string.special.url" => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::UNDERLINED),
        "string.regexp" => Style::default().fg(Color::Rgb(255, 180, 100)),
        "type" => Style::default().fg(Color::Yellow),
        "variable" => Style::default().fg(Color::White),
        "variable.builtin" => Style::default().fg(Color::Rgb(255, 200, 100)),
        "variable.parameter" => Style::default().fg(Color::LightRed),
        _ => Style::default(),
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

pub fn run(backend: M2Backend) -> Result<(), Box<dyn std::error::Error>> {
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
                    app.insert_input_char(ch);
                }
            }
            _ => {}
        }
    }

    terminal.leave()?;
    Ok(())
}