//! Tree-sitter syntax highlighting for the live input and echoed code. Styles
//! are resolved through a [`Theme`], so this module names no colours of its own.

use ratatui::style::Style;
use ratatui::text::Span;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use crate::theme::{Theme, CAPTURE_NAMES};

const M2_HIGHLIGHTS_QUERY: &str = include_str!("../queries/highlights.scm");

pub struct SyntaxHighlighter {
    config: Option<HighlightConfiguration>,
    highlighter: Highlighter,
}

impl SyntaxHighlighter {
    pub fn new() -> Self {
        let lang = tree_sitter_macaulay2::language();
        let config = HighlightConfiguration::new(lang, "macaulay2", M2_HIGHLIGHTS_QUERY, "", "")
            .ok()
            .map(|mut c| {
                c.configure(CAPTURE_NAMES);
                c
            });

        Self {
            config,
            highlighter: Highlighter::new(),
        }
    }

    /// Highlight one line, returning `(style, text)` runs. Falls back to a
    /// single unstyled run when the grammar failed to load or errored.
    pub fn highlight_m2(&mut self, source: &str, theme: &Theme) -> Vec<(Style, String)> {
        let Some(config) = self.config.as_ref() else {
            return vec![(Style::default(), source.to_string())];
        };
        match highlight_spans(&mut self.highlighter, config, source, theme) {
            Ok(spans) => spans,
            Err(_) => vec![(Style::default(), source.to_string())],
        }
    }
}

fn highlight_spans(
    highlighter: &mut Highlighter,
    config: &HighlightConfiguration,
    source: &str,
    theme: &Theme,
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
                    .map_or_else(Style::default, |index| theme.syntax_style(index));
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

/// Flatten `(style, text)` runs into ratatui spans. Any embedded newline is
/// split out (runs should be single-line in practice, but a multi-line span
/// would otherwise corrupt the layout).
pub fn highlight_to_spans(highlighted: &[(Style, String)]) -> Vec<Span<'static>> {
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
