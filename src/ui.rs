//! All drawing. Every surface is styled from a [`Theme`]; this module names no
//! colours. Output is composited into an off-screen [`Buffer`] of full height so
//! each entry can be a real ratatui widget while the transcript still scrolls as
//! one region.

use ratatui::buffer::Buffer;
use ratatui::layout::{self, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph, Widget};
use ratatui::Frame;

use std::collections::HashSet;

use crate::editor::Editor;
use crate::highlight::{highlight_to_spans, SyntaxHighlighter};
use crate::session::Session;
use crate::theme::Theme;
use crate::transcript::{
    diagnostic_message, is_diagnostic, strip_m2_location, trim_leading_blank, trim_trailing_blank,
    wrap_chars, TranscriptEntry,
};

const MAX_INPUT_HEIGHT: u16 = 8;
const INPUT_PLACEHOLDER: &str = "Type M2 code...";
const GUTTER_W: u16 = 5;
const MIN_BOX_W: u16 = 8;
const WARN_TITLE: &str = "Warning";
const CODE_PANE_TITLE: &str = "$code";

pub fn calculate_input_height(editor: &Editor, has_error: bool) -> u16 {
    let lines = if editor.is_empty() {
        1
    } else {
        editor.text().lines().count().max(1) as u16
    };
    // +2 for the breathing-room rows above and below the prompt, +1 for a hint row.
    let extra = if has_error { 1 } else { 0 };
    (lines + 2 + extra).clamp(3, MAX_INPUT_HEIGHT + 2 + extra)
}

/// The left status column: a right-aligned marker (`oN`, prompt glyph, …) padded
/// to [`GUTTER_W`] in the given style, so it reads like an editor's gutter.
fn gutter_span(label: &str, style: Style) -> Span<'static> {
    let w = GUTTER_W as usize;
    let label_w = label.chars().count();
    let pad = w.saturating_sub(label_w + 1);
    let trailing = w.saturating_sub(pad + label_w);
    let text = format!("{}{}{}", " ".repeat(pad), label, " ".repeat(trailing));
    Span::styled(text, style.bold())
}

/// One composited unit of the transcript. `Code`/`System` are full-width bands;
/// `Output` is a bordered box inset by one gutter on each side.
enum Panel {
    Band {
        lines: Vec<Line<'static>>,
        base: Style,
    },
    Output {
        border_style: Style,
        title: Option<Line<'static>>,
        bottom: Option<Line<'static>>,
        body: Vec<Line<'static>>,
    },
}

impl Panel {
    fn height(&self) -> u16 {
        match self {
            Panel::Band { lines, .. } => lines.len() as u16,
            Panel::Output { body, .. } => body.len() as u16 + 2,
        }
    }
}

impl Widget for Panel {
    /// Render into the full-width row `area` (`area.height` must be [`Panel::height`]).
    /// A `Band` fills the row; an `Output` box insets itself by one [`GUTTER_W`] gutter
    /// on each side, matching the input bands' code column.
    fn render(self, area: Rect, buf: &mut Buffer) {
        match self {
            Panel::Band { lines, base } => {
                Paragraph::new(lines).style(base).render(area, buf);
            }
            Panel::Output {
                border_style,
                title,
                bottom,
                body,
            } => {
                let box_w = area.width.saturating_sub(2 * GUTTER_W).max(MIN_BOX_W);
                let box_area = Rect::new(area.x + GUTTER_W, area.y, box_w, area.height);
                let mut block = Block::bordered()
                    .border_type(BorderType::Rounded)
                    .border_style(border_style)
                    .padding(Padding::horizontal(1));
                if let Some(t) = title {
                    block = block.title(t);
                }
                if let Some(b) = bottom {
                    block = block.title_bottom(b);
                }
                let inner = block.inner(box_area);
                block.render(box_area, buf);
                Paragraph::new(body).render(inner, buf);
            }
        }
    }
}

/// Build the panel for an echoed input block: a full-width tinted band with the
/// `iN` marker, coloured by the status of the result that follows it.
fn input_panel(
    number: usize,
    code: &str,
    follows: Option<&TranscriptEntry>,
    highlighter: &mut SyntaxHighlighter,
    theme: &Theme,
) -> Panel {
    let code_bg = theme.ui.code_bg;
    let marker_style = match follows {
        Some(TranscriptEntry::Output { is_error: true, .. }) => theme.ui.error,
        Some(TranscriptEntry::Output { result, .. }) if result.lines().any(is_diagnostic) => {
            theme.ui.warning
        }
        _ => theme.ui.prompt,
    }
    .patch(code_bg);

    let mut lines = vec![Line::default()]; // margin above the code
    for (i, line) in code.lines().enumerate() {
        let marker = if i == 0 {
            format!("i{number}")
        } else {
            String::new()
        };
        let mut spans = vec![gutter_span(&marker, marker_style)];
        for mut s in highlight_to_spans(&highlighter.highlight_m2(line, theme)) {
            s.style = s.style.patch(code_bg);
            spans.push(s);
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::default()); // margin below the code
    Panel::Band {
        lines,
        base: code_bg,
    }
}

/// Build the bordered box for a result. Returns `None` for a fully suppressed
/// output (e.g. `z = 7;`), which renders nothing.
fn output_panel(
    result: &str,
    type_info: &Option<String>,
    is_error: bool,
    width: u16,
    theme: &Theme,
) -> Option<Panel> {
    let box_w = width.saturating_sub(2 * GUTTER_W).max(MIN_BOX_W) as usize;
    let text_w = box_w.saturating_sub(4); // two borders, two padding columns

    let raw: Vec<String> = if result.is_empty() {
        Vec::new()
    } else {
        result.lines().map(|s| s.to_string()).collect()
    };

    let type_str = type_info.as_deref().filter(|t| !t.is_empty());
    let bottom = type_str
        .map(|t| Line::from(Span::styled(format!(" {t} "), theme.ui.type_info)).right_aligned());

    if is_error {
        let value_style = theme.ui.error;
        let body: Vec<Line<'static>> = raw
            .iter()
            .flat_map(|l| wrap_chars(strip_m2_location(l), text_w))
            .map(|l| Line::from(Span::styled(l, value_style)))
            .collect();
        return Some(Panel::Output {
            border_style: theme.ui.error_border,
            title: None,
            bottom,
            body,
        });
    }

    // Split warnings out of the value; both go in one box, whose border turns
    // orange and gains a `Warning` title when a warning is present. Messages
    // (keyword dropped) are de-duplicated so repeats don't spam.
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

    if value.is_empty() && type_str.is_none() && warnings.is_empty() {
        return None;
    }

    let value_style = theme.ui.value;
    let warn_style = theme.ui.warning;
    let mut body: Vec<Line<'static>> = warnings
        .iter()
        .flat_map(|w| wrap_chars(w, text_w))
        .map(|l| Line::from(Span::styled(l, warn_style)))
        .collect();
    if !warnings.is_empty() && !value.is_empty() {
        body.push(Line::default());
    }
    body.extend(
        value
            .iter()
            .flat_map(|c| wrap_chars(c, text_w))
            .map(|l| Line::from(Span::styled(l, value_style))),
    );

    let title = (!warnings.is_empty())
        .then(|| Line::from(Span::styled(format!(" {WARN_TITLE} "), warn_style)));
    let border_style = if warnings.is_empty() {
        theme.ui.border
    } else {
        theme.ui.warning_border
    };
    Some(Panel::Output {
        border_style,
        title,
        bottom,
        body,
    })
}

fn system_panel(msg: &str, theme: &Theme) -> Panel {
    let muted = theme.ui.muted;
    Panel::Band {
        lines: vec![
            Line::from(vec![
                gutter_span("", muted.patch(theme.ui.transcript_bg)),
                Span::styled(msg.to_string(), muted),
            ]),
            Line::default(),
        ],
        base: theme.ui.transcript_bg,
    }
}

fn build_panels(
    transcript: &[TranscriptEntry],
    highlighter: &mut SyntaxHighlighter,
    theme: &Theme,
    width: u16,
) -> Vec<Panel> {
    let mut panels = Vec::new();
    for (idx, entry) in transcript.iter().enumerate() {
        match entry {
            TranscriptEntry::Input { number, code } => {
                let follows = transcript.get(idx + 1);
                panels.push(input_panel(*number, code, follows, highlighter, theme));
            }
            TranscriptEntry::Output {
                result,
                type_info,
                is_error,
            } => {
                if let Some(p) = output_panel(result, type_info, *is_error, width, theme) {
                    panels.push(p);
                }
            }
            TranscriptEntry::System(msg) => panels.push(system_panel(msg, theme)),
        }
    }
    panels
}

pub fn draw_transcript(
    frame: &mut Frame,
    area: Rect,
    transcript: &[TranscriptEntry],
    highlighter: &mut SyntaxHighlighter,
    theme: &Theme,
    scroll_offset: usize,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let panels = build_panels(transcript, highlighter, theme, area.width);
    let total: u16 = panels.iter().map(|p| p.height()).sum();
    let buf_h = total.max(area.height).max(1);

    let mut comp = Buffer::empty(Rect::new(0, 0, area.width, buf_h));
    Block::default()
        .style(theme.ui.transcript_bg)
        .render(*comp.area(), &mut comp);

    let mut y = 0;
    for panel in panels {
        let h = panel.height();
        panel.render(Rect::new(0, y, area.width, h), &mut comp);
        y += h;
    }

    // Show the bottom by default; PageUp/Down adjust the offset within bounds.
    let visible = area.height as usize;
    let scroll = scroll_offset.max((total as usize).saturating_sub(visible));
    let dst = frame.buffer_mut();
    for dy in 0..area.height {
        let src_y = scroll + dy as usize;
        if src_y >= buf_h as usize {
            break;
        }
        for x in 0..area.width {
            if let Some(cell) = comp.cell((x, src_y as u16)) {
                let cell = cell.clone();
                if let Some(slot) = dst.cell_mut((area.x + x, area.y + dy)) {
                    *slot = cell;
                }
            }
        }
    }
}

pub fn draw_input_area(
    frame: &mut Frame,
    area: Rect,
    editor: &Editor,
    highlighter: &mut SyntaxHighlighter,
    theme: &Theme,
    parse_error: Option<&str>,
) {
    let code_bg = theme.ui.code_bg;
    let mut lines: Vec<Line<'static>> = vec![Line::default()]; // margin above the prompt

    let input_lines: Vec<&str> = if editor.is_empty() {
        vec![""]
    } else {
        editor.text().lines().collect()
    };

    for (i, line) in input_lines.iter().enumerate() {
        // The live prompt keeps the `❯` glyph; the echoed code gets the `iN`
        // label instead.
        let marker = if i == 0 { "❯" } else { "" };
        let mut spans = vec![gutter_span(marker, theme.ui.prompt.patch(code_bg))];
        for mut s in highlight_to_spans(&highlighter.highlight_m2(line, theme)) {
            s.style = s.style.patch(code_bg);
            spans.push(s);
        }
        if i == 0 && editor.is_empty() {
            spans.push(Span::styled(INPUT_PLACEHOLDER.to_string(), theme.ui.muted));
        }
        lines.push(Line::from(spans));
    }
    // A syntax-error hint sits just under the input; cleared on the next edit.
    if let Some(msg) = parse_error {
        lines.push(Line::from(vec![
            gutter_span("", theme.ui.error.patch(code_bg)),
            Span::styled(msg.to_string(), theme.ui.error.patch(code_bg)),
        ]));
    }
    lines.push(Line::default()); // breathing room below the prompt

    let paragraph = Paragraph::new(lines).style(code_bg);
    frame.render_widget(paragraph, area);

    let (cursor_line, cursor_col) = editor.cursor_line_col();
    frame.set_cursor_position(Position::new(
        area.x + GUTTER_W + cursor_col as u16,
        area.y + 1 + cursor_line as u16, // +1 for the top margin row
    ));
}

/// Draw the `$code` pane: this session's committed cells, each under its `iN`
/// marker, above the live (not-yet-committed) input. This is a *view* of the
/// session code — the same committed sequence `Session::committed_text` joins
/// for the language server — so the `iN` markers double as line-anchors into it.
pub fn draw_code_pane(
    frame: &mut Frame,
    area: Rect,
    session: &Session,
    editor: &Editor,
    highlighter: &mut SyntaxHighlighter,
    theme: &Theme,
) {
    let bg = theme.ui.code_bg;
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.ui.border)
        .title(Span::styled(
            format!(" {CODE_PANE_TITLE} "),
            theme.ui.prompt,
        ))
        .style(bg);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line<'static>> = Vec::new();
    for cell in session.cells() {
        for (i, line) in cell.code.lines().enumerate() {
            // Only the first source line of a cell carries the `iN` marker.
            let marker = if i == 0 {
                format!("i{}", cell.number.0)
            } else {
                String::new()
            };
            let mut spans = vec![gutter_span(&marker, theme.ui.prompt.patch(bg))];
            for mut s in highlight_to_spans(&highlighter.highlight_m2(line, theme)) {
                s.style = s.style.patch(bg);
                spans.push(s);
            }
            lines.push(Line::from(spans));
        }
    }

    // The live, not-yet-committed input shown below the committed cells. It has
    // no number yet — a cell earns its `iN` only when it commits (a successful
    // run), which is exactly why errors never appear in this pane.
    let live = editor.text();
    if !live.trim().is_empty() {
        // NOTE (superseded by the block.rs rewrite): append the live input to `lines` as a dimmed, "pending"
        // block. Design decisions, all tied to the iN-as-anchor model: do you
        // prefix it with the number it *would* take on success
        // (`session.next_number().0`, mirroring the markers above) or leave it
        // unnumbered? What separates it from the committed cells (a blank
        // `Line::default()`?), and what dim style reads as "not committed yet"
        // (e.g. `theme.ui.muted.patch(bg)`)? Build one or more `Line` per source
        // line of `live` and push them onto `lines`.
    }

    frame.render_widget(Paragraph::new(lines).style(bg), inner);
}
