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
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Terminal;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use std::collections::HashSet;
use std::error::Error;

use crate::m2_backend::{M2Backend, M2Result};

const USER_BG: Color = Color::Rgb(20, 22, 30);
const INPUT_BG: Color = Color::Rgb(26, 28, 38);
const VALUE_FG: Color = Color::Rgb(214, 217, 224);
const ERROR_FG: Color = Color::Rgb(240, 150, 150);
const TYPE_FG: Color = Color::Rgb(196, 167, 110);
const WARN_FG: Color = Color::Rgb(232, 146, 64);
const BORDER_FG: Color = Color::Rgb(72, 78, 96);
const ERROR_BORDER_FG: Color = Color::Rgb(128, 78, 78);
const WARN_BORDER_FG: Color = Color::Rgb(176, 104, 52);

const MAX_INPUT_HEIGHT: u16 = 8;
const INPUT_PLACEHOLDER: &str = "Type M2 code...";
const GUTTER_W: u16 = 5;
const MIN_BOX_W: u16 = 8;

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
    history: Vec<String>,
    history_pos: Option<usize>,
    draft: String,
}

impl App {
    fn new(backend: M2Backend) -> Self {
        let input_number = backend.next_input_number();
        Self {
            backend,
            input: String::new(),
            input_cursor: 0,
            input_number,
            transcript: vec![TranscriptEntry::System(
                "spectral-m2 — Macaulay2 highlighted REPL".to_string(),
            )],
            highlighter: SyntaxHighlighter::new(),
            scroll_offset: 0,
            history: Vec::new(),
            history_pos: None,
            draft: String::new(),
        }
    }

    fn submit_input(&mut self) {
        let code = self.input.trim().to_string();
        if code.is_empty() {
            return;
        }
        let number = self.input_number;

        // Record in history (skip consecutive duplicates), reset navigation.
        if self.history.last() != Some(&code) {
            self.history.push(code);
        }
        self.history_pos = None;
        self.draft.clear();

        self.transcript.push(TranscriptEntry::Input {
            number,
            code: self.input.clone(),
        });

        match self.backend.evaluate(self.input.trim()) {
            Ok(M2Result { is_error, output }) => {
                let (result_text, type_info) =
                    clean_m2_output(&output, self.input.trim(), is_error);
                self.transcript.push(TranscriptEntry::Output {
                    result: result_text,
                    type_info,
                    is_error,
                });
            }
            Err(e) => {
                self.transcript.push(TranscriptEntry::Output {
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
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => {
                return false;
            }
            (KeyCode::Enter, modifiers)
                if modifiers.contains(KeyModifiers::SHIFT)
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
                if self.cursor_on_first_line() {
                    self.history_prev();
                } else {
                    self.move_cursor_up();
                }
            }
            (KeyCode::Down, _) => {
                if self.cursor_on_last_line() {
                    self.history_next();
                } else {
                    self.move_cursor_down();
                }
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
            let target_line_start = if current_line == 0 {
                0
            } else {
                line_starts[current_line - 1]
            };
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

    /// True when no newline precedes the cursor — Up should recall history.
    fn cursor_on_first_line(&self) -> bool {
        !self.input[..self.input_cursor].contains('\n')
    }

    /// True when no newline follows the cursor — Down should advance history.
    fn cursor_on_last_line(&self) -> bool {
        !self.input[self.input_cursor..].contains('\n')
    }

    /// Replace the live input and park the cursor at its end.
    fn load_input(&mut self, text: String) {
        self.input_cursor = text.len();
        self.input = text;
    }

    /// Step to an older history entry, stashing the draft on first recall.
    fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_pos {
            None => {
                self.draft = self.input.clone();
                self.history.len() - 1
            }
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.history_pos = Some(idx);
        self.load_input(self.history[idx].clone());
    }

    /// Step to a newer history entry, restoring the draft past the newest.
    fn history_next(&mut self) {
        let idx = match self.history_pos {
            None => return,
            Some(i) if i + 1 < self.history.len() => i + 1,
            Some(_) => {
                self.history_pos = None;
                let draft = self.draft.clone();
                self.draft.clear();
                self.load_input(draft);
                return;
            }
        };
        self.history_pos = Some(idx);
        self.load_input(self.history[idx].clone());
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

        // Reset every cell first: `Paragraph::style` recolors backgrounds but
        // leaves stale glyphs from prior frames in place, which show through as
        // ghost rectangles. Clearing wipes the symbols so the fills are clean.
        frame.render_widget(Clear, size);
        self.draw_transcript(frame, chunks[0]);
        self.draw_input_area(frame, chunks[1]);
    }

    fn calculate_input_height(&self) -> u16 {
        let lines = if self.input.is_empty() {
            1
        } else {
            self.input.lines().count().max(1) as u16
        };
        // +2 for the breathing-room rows above and below the prompt.
        (lines + 2).clamp(3, MAX_INPUT_HEIGHT + 2)
    }

    fn draw_transcript(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let mut lines: Vec<Line<'static>> = Vec::new();
        // A full-width blank row carrying the code tint, so the margins around an
        // echoed code block share its background rather than the transcript's.
        let tint_row = || {
            Line::from(Span::styled(
                " ".repeat(area.width as usize),
                Style::default().bg(INPUT_BG),
            ))
        };

        for (idx, entry) in self.transcript.iter().enumerate() {
            match entry {
                TranscriptEntry::Input { number, code } => {
                    // The marker echoes the result's status: red if the run
                    // errored, orange if it warned, else cyan.
                    let marker_color = match self.transcript.get(idx + 1) {
                        Some(TranscriptEntry::Output { is_error: true, .. }) => ERROR_FG,
                        Some(TranscriptEntry::Output { result, .. })
                            if result.lines().any(is_diagnostic) =>
                        {
                            WARN_FG
                        }
                        _ => Color::Cyan,
                    };
                    lines.push(tint_row()); // margin above the code
                    for (i, line) in code.lines().enumerate() {
                        let highlighted = self.highlighter.highlight_m2(line);
                        let marker = if i == 0 {
                            format!("i{number}")
                        } else {
                            String::new()
                        };
                        // The code line is a borderless tinted band: gutter + the
                        // highlighted code (re-backed onto the tint) + a fill to
                        // the right edge.
                        let mut spans = vec![gutter_span(&marker, marker_color, INPUT_BG)];
                        for mut s in highlight_to_spans(&highlighted) {
                            s.style = s.style.bg(INPUT_BG);
                            spans.push(s);
                        }
                        let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
                        if (used as u16) < area.width {
                            spans.push(Span::styled(
                                " ".repeat(area.width as usize - used),
                                Style::default().bg(INPUT_BG),
                            ));
                        }
                        lines.push(Line::from(spans));
                    }
                    lines.push(tint_row()); // margin below the code
                }
                TranscriptEntry::Output {
                    result,
                    type_info,
                    is_error,
                } => {
                    let raw: Vec<String> = if result.is_empty() {
                        Vec::new()
                    } else {
                        result.lines().map(|s| s.to_string()).collect()
                    };

                    if *is_error {
                        // The whole payload is the error; shed the location
                        // prefixes and frame it in the red box.
                        let content: Vec<String> = raw
                            .iter()
                            .map(|l| strip_m2_location(l).to_string())
                            .collect();
                        push_output_block(&mut lines, &[], &content, type_info, true, area.width);
                        continue;
                    }

                    // Split warnings out of the value; both go in one box, whose
                    // border turns orange and gains a `Warning` title when a
                    // warning is present. Messages (keyword dropped) are the
                    // interior rows, de-duplicated so repeats don't spam.
                    let (warn_raw, mut value): (Vec<String>, Vec<String>) =
                        raw.into_iter().partition(|l| is_diagnostic(l));
                    let mut seen = HashSet::new();
                    let warnings: Vec<String> = warn_raw
                        .iter()
                        .map(|l| diagnostic_message(strip_m2_location(l)).to_string())
                        .filter(|m| seen.insert(m.clone()))
                        .collect();
                    trim_leading_blank(&mut value);
                    trim_trailing_blank(&mut value);

                    // Suppressed output (e.g. `z = 7;`) — nothing to show.
                    if value.is_empty() && type_info.is_none() && warnings.is_empty() {
                        continue;
                    }
                    push_output_block(&mut lines, &warnings, &value, type_info, false, area.width);
                }
                TranscriptEntry::System(msg) => {
                    lines.push(Line::from(vec![
                        gutter_span("", Color::DarkGray, USER_BG),
                        Span::styled(msg.to_string(), Style::default().fg(Color::DarkGray)),
                    ]));
                    lines.push(Line::default());
                }
            }
        }

        let total_lines = lines.len();
        let visible_lines = area.height as usize;
        let scroll = self
            .scroll_offset
            .max(total_lines.saturating_sub(visible_lines));

        let visible: Vec<Line<'static>> =
            lines.into_iter().skip(scroll).take(visible_lines).collect();

        let paragraph = Paragraph::new(visible).style(Style::default().bg(USER_BG));
        frame.render_widget(paragraph, area);
    }

    fn draw_input_area(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        // A borderless full-width tinted band; the distinct background separates
        // it from the transcript above.
        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(Line::default()); // margin above the prompt

        let input_lines: Vec<&str> = if self.input.is_empty() {
            vec![""]
        } else {
            self.input.lines().collect()
        };

        for (i, line) in input_lines.iter().enumerate() {
            let highlighted = self.highlighter.highlight_m2(line);
            // The live prompt keeps the `❯` glyph; only the released/echoed code
            // in the transcript gets the `iN` label.
            let marker = if i == 0 { "❯" } else { "" };
            let mut line_spans = vec![gutter_span(marker, Color::Cyan, INPUT_BG)];
            line_spans.extend(highlight_to_spans(&highlighted));
            if i == 0 && self.input.is_empty() {
                line_spans.push(Span::styled(
                    INPUT_PLACEHOLDER.to_string(),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            lines.push(Line::from(line_spans));
        }
        // Breathing room below the prompt (keeps it off the very bottom edge).
        lines.push(Line::default());

        let paragraph = Paragraph::new(lines).style(Style::default().bg(INPUT_BG));
        frame.render_widget(paragraph, area);

        let cursor_line = self.input[..self.input_cursor].lines().count().max(1) - 1;
        let cursor_col = self.input[..self.input_cursor]
            .lines()
            .last()
            .map(|l| l.chars().count())
            .unwrap_or(0);

        frame.set_cursor_position(Position::new(
            area.x + GUTTER_W + cursor_col as u16,
            area.y + 1 + cursor_line as u16, // +1 for the top margin row
        ));
    }
}

/// The left status column: a right-aligned marker (`oN`, prompt glyph, …) padded
/// to [`GUTTER_W`] on its own background, so it reads like an editor's gutter.
fn gutter_span(label: &str, fg: Color, bg: Color) -> Span<'static> {
    let w = GUTTER_W as usize;
    let label_w = label.chars().count();
    // Right-align the marker, always leaving one trailing space before content.
    let pad = w.saturating_sub(label_w + 1);
    let trailing = w.saturating_sub(pad + label_w);
    let text = format!("{}{}{}", " ".repeat(pad), label, " ".repeat(trailing));
    Span::styled(
        text,
        Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
    )
}

/// Match a Macaulay2 session prefix like `i12 = ` or `o3 : `, returning the
/// byte length of the prefix (tag, digits, space, separator, space) if present.
fn io_prefix(s: &str, tag: u8, sep: u8) -> Option<usize> {
    let b = s.as_bytes();
    if b.is_empty() || b[0] != tag {
        return None;
    }
    let mut i = 1;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == 1 {
        return None; // no digits after the tag
    }
    if i + 3 <= b.len() && b[i] == b' ' && b[i + 1] == sep && b[i + 2] == b' ' {
        Some(i + 3)
    } else {
        None
    }
}

/// A trailing `iN : ` prompt with nothing meaningful after it.
fn is_trailing_prompt(s: &str) -> bool {
    io_prefix(s, b'i', b':').is_some_and(|p| s[p..].trim().is_empty())
}

/// An M2 diagnostic line — `<loc>:<l>:<c>:(n):[m]: warning: …` (or `error:`).
/// Used to lift warnings out of a successful result's value body.
fn is_diagnostic(s: &str) -> bool {
    s.contains(": warning:") || s.contains(": error:")
}

/// Strip M2's cryptic source-location prefix (`currentString:1:6:(3):[8]: `),
/// keeping only the message (`warning: …` / `error: …`). The prefix is slated
/// for removal upstream, so we drop it rather than render it. Returns the input
/// unchanged when no such prefix is present (e.g. an ordinary value line).
fn strip_m2_location(s: &str) -> &str {
    match s.find("]: ") {
        Some(i) if s[..i].contains(":(") && s[..i].contains(":[") => &s[i + 3..],
        _ => s,
    }
}

/// Wrap a string to `width` characters, breaking at the rightmost space that
/// fits when possible and hard-splitting only words too long to fit. Never
/// returns an empty vec (an empty input yields one empty line).
fn wrap_chars(s: &str, width: usize) -> Vec<String> {
    if width == 0 || s.chars().count() <= width {
        return vec![s.to_string()];
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        if chars.len() - start <= width {
            out.push(chars[start..].iter().collect());
            break;
        }
        let hard = start + width;
        // Prefer the rightmost space inside the window; the space is consumed.
        let brk = (start..hard).rev().find(|&i| chars[i] == ' ');
        let (end, next) = match brk {
            Some(i) if i > start => (i, i + 1),
            _ => (hard, hard),
        };
        out.push(chars[start..end].iter().collect());
        start = next;
    }
    out
}

/// Drop a diagnostic's leading keyword (`warning: character '.' …` → `character
/// '.' …`); the keyword becomes the box's border title instead. Returns the
/// whole string when there's no `keyword: ` lead.
fn diagnostic_message(s: &str) -> &str {
    match s.split_once(": ") {
        Some((_keyword, message)) => message,
        None => s,
    }
}

fn trim_leading_blank(lines: &mut Vec<String>) {
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
}

fn trim_trailing_blank(lines: &mut Vec<String>) {
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
}

/// Strip the session scaffolding that `capture(.., UserMode => true)` wraps
/// around a result: the echoed input prompt, the trailing next prompt, and the
/// `oN = ` / `oN : Type` labels. Returns `(value, type)`.
fn clean_m2_output(raw: &str, code: &str, is_error: bool) -> (String, Option<String>) {
    let mut lines: Vec<String> = raw.lines().map(|s| s.to_string()).collect();

    trim_leading_blank(&mut lines);

    // Drop the echoed input: one transcript line per submitted input line.
    if lines
        .first()
        .is_some_and(|l| io_prefix(l, b'i', b':').is_some())
    {
        let echo_n = code.lines().count().max(1);
        for _ in 0..echo_n {
            if lines.is_empty() {
                break;
            }
            lines.remove(0);
        }
    }
    trim_leading_blank(&mut lines);

    if !is_error {
        trim_trailing_blank(&mut lines);
        if lines.last().is_some_and(|l| is_trailing_prompt(l)) {
            lines.pop();
        }
        trim_trailing_blank(&mut lines);
    }

    // Pull off a trailing `oN : Type` annotation.
    let mut type_info = None;
    if !is_error {
        if let Some(p) = lines.last().and_then(|l| io_prefix(l, b'o', b':')) {
            type_info = Some(lines.last().unwrap()[p..].trim().to_string());
            lines.pop();
            trim_trailing_blank(&mut lines);
        }
    }

    // Drop the `oN = ` value prefix, preserving 2-D net alignment by removing
    // the same number of leading columns from every value line.
    if !is_error {
        if let Some(plen) = lines.iter().find_map(|l| io_prefix(l, b'o', b'=')) {
            for l in lines.iter_mut() {
                *l = l.chars().skip(plen).collect();
            }
        }
    }
    trim_trailing_blank(&mut lines);

    (lines.join("\n"), type_info)
}

/// Render a result as a framed box inset by one gutter on each side (so its
/// right margin matches the left): a muted rounded border, the result type
/// tucked into the top-right of the frame, and the value lines inside, wrapped
/// to the interior width. The interior shares the transcript background, so the
/// border is the box's only boundary — no offset fill box. The `iN` label lives
/// on the code line above, not here (input and output share the counter).
fn push_output_block(
    out: &mut Vec<Line<'static>>,
    warnings: &[String],
    content: &[String],
    type_info: &Option<String>,
    is_error: bool,
    area_width: u16,
) {
    let value_fg = if is_error { ERROR_FG } else { VALUE_FG };
    let border_fg = if is_error {
        ERROR_BORDER_FG
    } else if warnings.is_empty() {
        BORDER_FG
    } else {
        WARN_BORDER_FG
    };

    // The box is inset by one gutter on each side, so its right margin matches
    // the left. `inner` is the dash span between the corner glyphs; `text_w` is
    // the value area, inset one padding space inside each vertical border.
    let box_w = area_width.saturating_sub(2 * GUTTER_W).max(MIN_BOX_W) as usize;
    let inner = box_w - 2;
    let text_w = box_w - 4;

    let border_style = Style::default().fg(border_fg);
    let value_style = Style::default().fg(value_fg);
    let warn_style = Style::default().fg(WARN_FG);
    let type_style = Style::default().fg(TYPE_FG).add_modifier(Modifier::ITALIC);
    let gutter = || gutter_span("", border_fg, USER_BG);

    let type_str = type_info.as_deref().filter(|t| !t.is_empty());
    let type_len = type_str.map(|t| t.chars().count()).unwrap_or(0);

    // Top border: `╭─ Warning ────╮` when the result carries warnings, tucked
    // left; otherwise a plain frame. The type lives on the bottom border.
    const WARN_TITLE: &str = "Warning";
    let mut top = vec![gutter()];
    match Some(WARN_TITLE).filter(|t| !warnings.is_empty() && t.len() + 4 <= inner) {
        Some(t) => {
            let lead = inner - (t.len() + 2) - 1;
            top.push(Span::styled("╭─ ".to_string(), border_style));
            top.push(Span::styled(t.to_string(), warn_style));
            top.push(Span::styled(
                format!(" {}╮", "─".repeat(lead)),
                border_style,
            ));
        }
        None => top.push(Span::styled(
            format!("╭{}╮", "─".repeat(inner)),
            border_style,
        )),
    }
    out.push(Line::from(top));

    // Rows: warnings (amber) above the value, each `│ text… │`, wrapped to the
    // interior width so nothing is lost off the right edge. A blank line sets
    // the value off from any warnings above it.
    let mut rows: Vec<(&str, Style)> = warnings.iter().map(|w| (w.as_str(), warn_style)).collect();
    if !warnings.is_empty() && !content.is_empty() {
        rows.push(("", value_style));
    }
    rows.extend(content.iter().map(|c| (c.as_str(), value_style)));
    for (line, style) in rows {
        for val in wrap_chars(line, text_w) {
            let pad = text_w - val.chars().count();
            out.push(Line::from(vec![
                gutter(),
                Span::styled("│ ".to_string(), border_style),
                Span::styled(val, style),
                Span::styled(" ".repeat(pad), style),
                Span::styled(" │".to_string(), border_style),
            ]));
        }
    }

    // Bottom border: `╰──── Type ─╯`, the type tucked right (dropped if it won't fit).
    let mut bottom = vec![gutter()];
    match type_str.filter(|_| type_len + 4 <= inner) {
        Some(t) => {
            let lead = inner - (type_len + 2) - 1;
            bottom.push(Span::styled(
                format!("╰{} ", "─".repeat(lead)),
                border_style,
            ));
            bottom.push(Span::styled(t.to_string(), type_style));
            bottom.push(Span::styled(" ─╯".to_string(), border_style));
        }
        None => bottom.push(Span::styled(
            format!("╰{}╯", "─".repeat(inner)),
            border_style,
        )),
    }
    out.push(Line::from(bottom));
}

struct SyntaxHighlighter {
    config: Option<HighlightConfiguration>,
    highlighter: Highlighter,
}

impl SyntaxHighlighter {
    fn new() -> Self {
        let lang = tree_sitter_macaulay2::language();
        let config = HighlightConfiguration::new(lang, "macaulay3", M2_HIGHLIGHTS_QUERY, "", "")
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
                    app.insert_input_char(ch);
                }
            }
            _ => {}
        }
    }
    terminal.leave()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::{clean_m2_output, push_output_block, strip_m2_location, GUTTER_W};

    #[test]
    fn strips_m2_location_prefix() {
        assert_eq!(
            strip_m2_location("ntString:1:12:(3):[8]: warning: character '.' follows"),
            "warning: character '.' follows"
        );
        assert_eq!(
            strip_m2_location("currentString:1:6:(3):[8]: error: exponent missing"),
            "error: exponent missing"
        );
        // An ordinary value line (no location prefix) is returned unchanged.
        assert_eq!(
            strip_m2_location("{{Binary, {Token, 2}}}"),
            "{{Binary, {Token, 2}}}"
        );
    }

    #[test]
    fn output_box_lines_are_inset_by_one_gutter() {
        // The long line exercises wrapping; every rendered box line must reach
        // the right margin exactly — the box is inset by one gutter on each side,
        // so each line spans the full width minus that single trailing gutter.
        let content = ["x + y".to_string(), "z".to_string(), "q".repeat(400)];
        let warns_none: Vec<String> = Vec::new();
        let warns_some: Vec<String> = vec!["warning: ".to_string() + &"w".repeat(300)];
        for width in [40u16, 60, 96, 120] {
            for ty in [None, Some("PolynomialRing".to_string())] {
                for is_error in [false, true] {
                    for warnings in [&warns_none, &warns_some] {
                        let mut lines = Vec::new();
                        push_output_block(&mut lines, warnings, &content, &ty, is_error, width);
                        for line in &lines {
                            assert_eq!(
                                line.width() as u16,
                                width - GUTTER_W,
                                "width={width} ty={ty:?} is_error={is_error} warns={}",
                                warnings.len()
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn strips_echo_and_prompt_simple_value() {
        let raw = "\ni1 : 2 + 2\n\no1 = 4\n\ni2 : ";
        let (val, ty) = clean_m2_output(raw, "2 + 2", false);
        assert_eq!(val, "4");
        assert_eq!(ty, None);
    }

    #[test]
    fn extracts_type_annotation() {
        let raw = "\ni1 : parse \"2...2\"\n\no1 = {{Binary, {Token, 2}}}\n\no1 : List\n\ni2 : ";
        let (val, ty) = clean_m2_output(raw, "parse \"2...2\"", false);
        assert_eq!(val, "{{Binary, {Token, 2}}}");
        assert_eq!(ty.as_deref(), Some("List"));
    }

    #[test]
    fn preserves_2d_net_alignment() {
        let raw = "\ni1 : R = QQ[x,y]; x^2 + y\n\n      2\no2 = x  + y\n\no2 : R\n\ni3 : ";
        let (val, ty) = clean_m2_output(raw, "R = QQ[x,y]; x^2 + y", false);
        assert_eq!(val, " 2\nx  + y");
        assert_eq!(ty.as_deref(), Some("R"));
    }

    #[test]
    fn multiline_input_echo_fully_stripped() {
        let raw = "\ni1 : a = 1 +\n     2 + 3\n\no1 = 6\n\ni2 : ";
        let (val, ty) = clean_m2_output(raw, "a = 1 +\n2 + 3", false);
        assert_eq!(val, "6");
        assert_eq!(ty, None);
    }

    #[test]
    fn suppressed_assignment_is_empty() {
        let raw = "\ni1 : z = 7;\n\ni2 : ";
        let (val, ty) = clean_m2_output(raw, "z = 7;", false);
        assert_eq!(val, "");
        assert_eq!(ty, None);
    }

    #[test]
    fn error_keeps_message_drops_echo() {
        let raw = "\ni1 : 1/0\ncurrentString:1:1:(3):[9]: error: division by zero";
        let (val, ty) = clean_m2_output(raw, "1/0", true);
        assert_eq!(val, "currentString:1:1:(3):[9]: error: division by zero");
        assert_eq!(ty, None);
    }
}
