//! The colour scheme. This is the ONLY module permitted to name
//! [`ratatui::style::Color`]; every other module receives ready-made [`Style`]
//! values from a [`Theme`] and never constructs a colour itself. A clippy
//! `disallowed-types` gate enforces that boundary (see `clippy.toml`).
#![allow(clippy::disallowed_types)]

use ratatui::style::{Color, Modifier, Style};

/// The tree-sitter capture names the highlighter is configured with, and the
/// keys of a theme's syntax palette. Single source of truth: the highlighter is
/// configured with this slice, and [`Theme::syntax`] is indexed in the same
/// order, so a capture's style is `syntax[index]`.
pub const CAPTURE_NAMES: &[&str] = &[
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

/// The styles for every non-syntax surface of the TUI. Background entries
/// (`transcript_bg`, `code_bg`) carry only a `bg` and are meant to be merged
/// onto a foreground style with [`Style::patch`].
pub struct UiTheme {
    pub transcript_bg: Style,
    pub code_bg: Style,
    pub value: Style,
    pub error: Style,
    pub warning: Style,
    pub type_info: Style,
    pub border: Style,
    pub error_border: Style,
    pub warning_border: Style,
    pub prompt: Style,
    pub muted: Style,
}

/// A full colour scheme: UI surfaces plus the per-capture syntax palette.
pub struct Theme {
    pub ui: UiTheme,
    syntax: Vec<Style>,
}

impl Theme {
    /// The style for the capture at `index` into [`CAPTURE_NAMES`]; unstyled
    /// when the index is out of range.
    pub fn syntax_style(&self, index: usize) -> Style {
        self.syntax.get(index).copied().unwrap_or_default()
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            ui: default_ui(),
            syntax: CAPTURE_NAMES
                .iter()
                .map(|n| default_syntax_style(n))
                .collect(),
        }
    }
}

fn default_ui() -> UiTheme {
    UiTheme {
        transcript_bg: Style::default().bg(Color::Rgb(20, 22, 30)),
        code_bg: Style::default().bg(Color::Rgb(26, 28, 38)),
        value: Style::default().fg(Color::Rgb(214, 217, 224)),
        error: Style::default().fg(Color::Rgb(240, 150, 150)),
        warning: Style::default().fg(Color::Rgb(232, 146, 64)),
        type_info: Style::default()
            .fg(Color::Rgb(196, 167, 110))
            .add_modifier(Modifier::ITALIC),
        border: Style::default().fg(Color::Rgb(72, 78, 96)),
        error_border: Style::default().fg(Color::Rgb(128, 78, 78)),
        warning_border: Style::default().fg(Color::Rgb(176, 104, 52)),
        prompt: Style::default().fg(Color::Cyan),
        muted: Style::default().fg(Color::DarkGray),
    }
}

fn default_syntax_style(name: &str) -> Style {
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
