//! Rows of the highlight debug panel.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use super::settings::theme::groups::scheme;

/// A titled run of entries, drawn together in columns.
pub struct DebugSection {
    pub title: &'static str,
    pub entries: Vec<DebugEntry>,
}

impl DebugSection {
    pub fn new(title: &'static str, entries: Vec<DebugEntry>) -> Self {
        Self { title, entries }
    }
}

/// A name drawn in its own style, or beside a swatch of the palette colour it names.
pub struct DebugEntry {
    swatch: Option<Color>,
    label: String,
    style: Style,
}

impl DebugEntry {
    const SWATCH: &'static str = "██ ";
    const GAP: usize = 2;

    pub fn group(name: &str, style: Style) -> Self {
        Self {
            swatch: None,
            label: name.into(),
            style,
        }
    }

    pub fn swatch(name: &str, color: Color) -> Self {
        Self {
            swatch: Some(color),
            label: name.into(),
            style: scheme().value(),
        }
    }

    /// The colour's `#rrggbb` code, drawn in the colour itself.
    fn hex(&self) -> Option<String> {
        match self.swatch? {
            Color::Rgb(r, g, b) => Some(format!(" #{r:02x}{g:02x}{b:02x}")),
            _ => None,
        }
    }

    fn swatch_width(&self) -> usize {
        self.swatch.map_or(0, |_| Self::SWATCH.chars().count())
    }

    pub fn width(&self) -> usize {
        let hex = self.hex().map_or(0, |hex| hex.chars().count());
        self.swatch_width() + self.label.chars().count() + hex + Self::GAP
    }

    /// The entry padded to `column_width`, swatch included.
    pub fn spans(&self, column_width: usize) -> Vec<Span<'static>> {
        let hex = self.hex().unwrap_or_default();
        let padding = column_width
            .saturating_sub(self.swatch_width() + self.label.chars().count() + hex.chars().count());
        let mut spans = Vec::from_iter(
            self.swatch
                .map(|color| Span::styled(Self::SWATCH, Style::new().fg(color))),
        );
        spans.push(Span::styled(self.label.clone(), self.style));
        if let Some(color) = self.swatch {
            spans.push(Span::styled(hex, Style::new().fg(color)));
        }
        spans.push(Span::raw(" ".repeat(padding)));
        spans
    }
}
