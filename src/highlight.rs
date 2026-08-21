//! The tree-sitter syntax highlighter: the one [`Highlighter`] implementation we
//! ship. It turns M2 source into a flat, ordered list of leaf [`HighlightSpan`]s
//! tagged by [`HighlightKind`] — no colours, no styles. Resolution to a concrete
//! `Style` happens later, at render, through the active theme; this module names
//! nothing visual.

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter as TsHighlighter};

use crate::block::highlight::{HighlightKind, HighlightSpan, Highlighter};

const M2_HIGHLIGHTS_QUERY: &str = include_str!("../queries/highlights.scm");

pub struct SyntaxHighlighter {
    config: Option<HighlightConfiguration>,
    highlighter: TsHighlighter,
}

impl SyntaxHighlighter {
    pub fn new() -> Self {
        let lang = tree_sitter_macaulay2::language();
        // Configure with the capture names in [`HighlightKind::ALL`] order, so the
        // index tree-sitter reports back is exactly a `HighlightKind` ordinal.
        let names: Vec<&str> = HighlightKind::ALL
            .iter()
            .map(|k| k.capture_name())
            .collect();
        let config = HighlightConfiguration::new(lang, "macaulay2", M2_HIGHLIGHTS_QUERY, "", "")
            .ok()
            .map(|mut c| {
                c.configure(&names);
                c
            });
        Self {
            config,
            highlighter: TsHighlighter::new(),
        }
    }
}

impl Highlighter for SyntaxHighlighter {
    /// Highlight `source`, returning leaf spans in source order. The grammar
    /// failing to load, or the highlighter erroring, yields no spans (the text
    /// renders unstyled) rather than panicking.
    fn highlight(&mut self, source: &str) -> Vec<HighlightSpan> {
        let Some(config) = self.config.as_ref() else {
            return Vec::new();
        };
        let Ok(events) = self
            .highlighter
            .highlight(config, source.as_bytes(), None, |_| None)
        else {
            return Vec::new();
        };

        let mut spans = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        for event in events {
            match event {
                Ok(HighlightEvent::HighlightStart(h)) => stack.push(h.0),
                Ok(HighlightEvent::HighlightEnd) => {
                    stack.pop();
                }
                Ok(HighlightEvent::Source { start, end }) => {
                    if let Some(kind) = stack
                        .last()
                        .and_then(|&i| HighlightKind::from_capture_index(i))
                    {
                        spans.push(HighlightSpan {
                            range: start..end,
                            kind,
                        });
                    }
                }
                Err(_) => return spans,
            }
        }
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_query_highlights_current_grammar() {
        let mut highlighter = SyntaxHighlighter::new();
        let spans =
            highlighter.highlight("if x then 1 else 0\nloadPackage \"Foo\"\nf ZZ := x -> x\nf 1");

        assert!(!spans.is_empty());
        for expected in [
            HighlightKind::KeywordConditional,
            HighlightKind::Number,
            HighlightKind::FunctionCall,
            HighlightKind::Module,
            HighlightKind::Label,
            HighlightKind::TypeParameter,
        ] {
            assert!(spans.iter().any(|span| span.kind == expected));
        }
    }
}
