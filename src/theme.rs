//! The colour scheme. This is the ONLY module permitted to name
//! [`ratatui::style::Color`]; every other module receives ready-made [`Style`]
//! values from a [`Theme`] and never constructs a colour itself. A clippy
//! `disallowed-types` gate enforces that boundary (see `clippy.toml`).
//!
//! Syntax colours are resolved through [`Theme::style`], an *exhaustive* match
//! over [`HighlightKind`]: a capture the grammar emits cannot be left unstyled
//! without a compile error, which is precisely the bug the old string-indexed
//! `CAPTURE_NAMES` table allowed (four emitted captures had no entry and rendered
//! plain).
#![allow(clippy::disallowed_types)]

use ratatui::style::{Color, Modifier, Style};

use crate::block::highlight::HighlightKind;

/// Named base colours. Every raw RGB triple the theme uses is defined here once
/// with a semantic name, so the styles below read as intent rather than magic
/// numbers and near-duplicate shades (the keyword-accent purples, the warm
/// error/return reds) are visibly related. ANSI colours are already named by
/// [`Color`] and are used directly at the call site.
mod palette {
    use ratatui::style::Color;

    // UI surfaces.
    pub const SURFACE_BASE: Color = Color::Rgb(20, 22, 30);
    pub const SURFACE_RAISED: Color = Color::Rgb(26, 28, 38);
    pub const FG_VALUE: Color = Color::Rgb(214, 217, 224);
    pub const FG_ERROR: Color = Color::Rgb(240, 150, 150);
    pub const FG_WARNING: Color = Color::Rgb(232, 146, 64);
    pub const FG_TYPE: Color = Color::Rgb(196, 167, 110);
    pub const BORDER_DEFAULT: Color = Color::Rgb(72, 78, 96);
    pub const BORDER_ERROR: Color = Color::Rgb(128, 78, 78);
    pub const BORDER_WARNING: Color = Color::Rgb(176, 104, 52);

    // Syntax: the keyword-accent family (purples shading by keyword role).
    pub const KEYWORD: Color = Color::Rgb(210, 140, 255);
    pub const KEYWORD_MODIFIER: Color = Color::Rgb(180, 140, 255);
    pub const KEYWORD_REPEAT: Color = Color::Rgb(200, 140, 255);
    // Syntax: the exception/return family (warm reds).
    pub const KEYWORD_EXCEPTION: Color = Color::Rgb(255, 140, 140);
    pub const KEYWORD_RETURN: Color = Color::Rgb(255, 160, 160);
    // Syntax: warm highlights.
    pub const VARIABLE_BUILTIN: Color = Color::Rgb(255, 200, 100);
    pub const STRING_REGEXP: Color = Color::Rgb(255, 180, 100);
}

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

/// A full colour scheme: the non-syntax UI surfaces plus a per-[`HighlightKind`]
/// syntax palette resolved by [`Theme::style`].
pub struct Theme {
    pub ui: UiTheme,
}

impl Theme {
    /// The foreground style for a syntax [`HighlightKind`]. Exhaustive by
    /// construction: a new capture variant forces a new arm here. Returned styles
    /// are fg + modifiers only, meant to be patched over a surface's background.
    pub fn style(&self, kind: HighlightKind) -> Style {
        let fg = |c: Color| Style::default().fg(c);
        let bold = |c: Color| Style::default().fg(c).add_modifier(Modifier::BOLD);
        match kind {
            HighlightKind::Comment => fg(Color::DarkGray),
            HighlightKind::Error => bold(Color::Red),
            HighlightKind::Number | HighlightKind::NumberFloat => fg(Color::LightGreen),
            HighlightKind::Boolean => fg(Color::LightYellow),
            HighlightKind::String => fg(Color::Green),
            HighlightKind::StringSpecial | HighlightKind::StringSpecialUrl => {
                fg(Color::Cyan).add_modifier(Modifier::UNDERLINED)
            }
            HighlightKind::StringRegexp => fg(palette::STRING_REGEXP),
            HighlightKind::CharacterSpecial => fg(Color::LightMagenta),
            HighlightKind::Variable => fg(Color::White),
            HighlightKind::VariableBuiltin => fg(palette::VARIABLE_BUILTIN),
            HighlightKind::VariableParameter => fg(Color::LightRed),
            HighlightKind::ConstantBuiltin => fg(Color::LightYellow),
            HighlightKind::Function | HighlightKind::FunctionBuiltin => fg(Color::Blue),
            HighlightKind::Constructor => fg(Color::LightCyan),
            HighlightKind::Type => fg(Color::Yellow),
            HighlightKind::Property => fg(Color::Cyan),
            HighlightKind::ModuleBuiltin => fg(Color::LightBlue),
            HighlightKind::Operator => fg(Color::Gray),
            HighlightKind::Keyword | HighlightKind::KeywordConditional => bold(palette::KEYWORD),
            HighlightKind::KeywordRepeat => bold(palette::KEYWORD_REPEAT),
            HighlightKind::KeywordReturn => bold(palette::KEYWORD_RETURN),
            HighlightKind::KeywordException => bold(palette::KEYWORD_EXCEPTION),
            HighlightKind::KeywordModifier => bold(palette::KEYWORD_MODIFIER),
            HighlightKind::KeywordDebug => fg(Color::DarkGray),
            HighlightKind::KeywordOperator => fg(Color::LightMagenta),
            HighlightKind::PunctuationBracket | HighlightKind::PunctuationDelimiter => {
                fg(Color::Gray)
            }
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme { ui: default_ui() }
    }
}

/// The default styles for the non-syntax UI surfaces.
fn default_ui() -> UiTheme {
    UiTheme {
        transcript_bg: Style::default().bg(palette::SURFACE_BASE),
        code_bg: Style::default().bg(palette::SURFACE_RAISED),
        value: Style::default().fg(palette::FG_VALUE),
        error: Style::default().fg(palette::FG_ERROR),
        warning: Style::default().fg(palette::FG_WARNING),
        type_info: Style::default()
            .fg(palette::FG_TYPE)
            .add_modifier(Modifier::ITALIC),
        border: Style::default().fg(palette::BORDER_DEFAULT),
        error_border: Style::default().fg(palette::BORDER_ERROR),
        warning_border: Style::default().fg(palette::BORDER_WARNING),
        prompt: Style::default().fg(Color::Cyan),
        muted: Style::default().fg(Color::DarkGray),
    }
}
