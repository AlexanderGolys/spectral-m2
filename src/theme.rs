//! Colour-scheme data and its mapping onto editor roles.
#![allow(clippy::disallowed_types)]

use ratatui::style::{Color, Modifier, Style};

use crate::block::highlight::HighlightKind;

macro_rules! rgb {
    ($hex:literal) => {{
        const VALUE: u32 = $hex;
        Color::Rgb(
            ((VALUE >> 16) & 0xff) as u8,
            ((VALUE >> 8) & 0xff) as u8,
            (VALUE & 0xff) as u8,
        )
    }};
}

/// A conventional terminal colour scheme. The names follow the Catppuccin
/// palette, which makes schemes data rather than collections of pre-baked UI
/// styles. Editor and syntax roles are derived from these colours below.
#[expect(
    dead_code,
    reason = "a complete colour-scheme palette supports future role mappings and scheme switching"
)]
pub struct Theme {
    pub name: &'static str,
    pub rosewater: Color,
    pub flamingo: Color,
    pub pink: Color,
    pub mauve: Color,
    pub red: Color,
    pub maroon: Color,
    pub peach: Color,
    pub yellow: Color,
    pub green: Color,
    pub teal: Color,
    pub sky: Color,
    pub sapphire: Color,
    pub blue: Color,
    pub lavender: Color,
    pub text: Color,
    pub subtext1: Color,
    pub subtext0: Color,
    pub overlay2: Color,
    pub overlay1: Color,
    pub overlay0: Color,
    pub surface2: Color,
    pub surface1: Color,
    pub surface0: Color,
    pub base: Color,
    pub mantle: Color,
    pub crust: Color,
}

impl Theme {
    /// Catppuccin Mocha, the default spectral-m2 colour scheme.
    pub fn catppuccin_mocha() -> Self {
        Self {
            name: "Catppuccin Mocha",
            rosewater: rgb!(0xf5e0dc),
            flamingo: rgb!(0xf2cdcd),
            pink: rgb!(0xf5c2e7),
            mauve: rgb!(0xcba6f7),
            red: rgb!(0xf38ba8),
            maroon: rgb!(0xeba0ac),
            peach: rgb!(0xfab387),
            yellow: rgb!(0xf9e2af),
            green: rgb!(0xa6e3a1),
            teal: rgb!(0x94e2d5),
            sky: rgb!(0x89dceb),
            sapphire: rgb!(0x74c7ec),
            blue: rgb!(0x89b4fa),
            lavender: rgb!(0xb4befe),
            text: rgb!(0xcdd6f4),
            subtext1: rgb!(0xbac2de),
            subtext0: rgb!(0xa6adc8),
            overlay2: rgb!(0x9399b2),
            overlay1: rgb!(0x7f849c),
            overlay0: rgb!(0x6c7086),
            surface2: rgb!(0x585b70),
            surface1: rgb!(0x45475a),
            surface0: rgb!(0x313244),
            base: rgb!(0x1e1e2e),
            mantle: rgb!(0x181825),
            crust: rgb!(0x11111b),
        }
    }

    fn foreground(color: Color) -> Style {
        Style::default().fg(color)
    }

    pub fn transcript(&self) -> Style {
        Style::default().bg(self.base)
    }

    pub fn code(&self) -> Style {
        Style::default().bg(self.mantle)
    }

    pub fn value(&self) -> Style {
        Self::foreground(self.text)
    }

    pub fn error(&self) -> Style {
        Self::foreground(self.red)
    }

    pub fn warning(&self) -> Style {
        Self::foreground(self.peach)
    }

    pub fn type_info(&self) -> Style {
        Self::foreground(self.yellow).add_modifier(Modifier::ITALIC)
    }

    pub fn border(&self) -> Style {
        Self::foreground(self.surface2)
    }

    pub fn error_border(&self) -> Style {
        Self::foreground(self.maroon)
    }

    pub fn warning_border(&self) -> Style {
        Self::foreground(self.peach)
    }

    pub fn prompt(&self) -> Style {
        Self::foreground(self.teal)
    }

    pub fn muted(&self) -> Style {
        Self::foreground(self.overlay0)
    }

    /// Resolve a tree-sitter capture against the active colour scheme.
    pub fn syntax(&self, kind: HighlightKind) -> Style {
        let fg = Self::foreground;
        let bold = |color| fg(color).add_modifier(Modifier::BOLD);
        match kind {
            HighlightKind::Comment | HighlightKind::KeywordDebug => fg(self.overlay0),
            HighlightKind::Error => bold(self.red),
            HighlightKind::Number | HighlightKind::NumberFloat => fg(self.peach),
            HighlightKind::Boolean | HighlightKind::ConstantBuiltin => fg(self.peach),
            HighlightKind::String => fg(self.green),
            HighlightKind::StringSpecialUrl => fg(self.sky).add_modifier(Modifier::UNDERLINED),
            HighlightKind::CharacterSpecial | HighlightKind::KeywordOperator => fg(self.pink),
            HighlightKind::Variable => fg(self.text),
            HighlightKind::VariableBuiltin => fg(self.red),
            HighlightKind::VariableParameter => fg(self.maroon),
            HighlightKind::Function
            | HighlightKind::FunctionCall
            | HighlightKind::FunctionBuiltin => fg(self.blue),
            HighlightKind::Label => fg(self.sapphire),
            HighlightKind::Type | HighlightKind::TypeParameter => fg(self.yellow),
            HighlightKind::Property => fg(self.teal),
            HighlightKind::Module | HighlightKind::Namespace => fg(self.lavender),
            HighlightKind::Operator => fg(self.sky),
            HighlightKind::Keyword
            | HighlightKind::KeywordConditional
            | HighlightKind::KeywordRepeat
            | HighlightKind::KeywordModifier => bold(self.mauve),
            HighlightKind::KeywordReturn | HighlightKind::KeywordException => bold(self.red),
            HighlightKind::PunctuationBracket | HighlightKind::PunctuationDelimiter => {
                fg(self.overlay2)
            }
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::catppuccin_mocha()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_scheme_is_catppuccin_mocha() {
        assert_eq!(Theme::default().name, "Catppuccin Mocha");
    }
}
