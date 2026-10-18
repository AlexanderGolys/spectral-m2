//! Tree-sitter query captures projected onto renderer highlight groups.

use std::ops::{Add, Sub};

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter as TsHighlighter};

use crate::ui::block::HLSpan;
use crate::ui::controller::settings::theme::groups::HLGroup;

impl Add<u32> for HLSpan {
    type Output = Self;

    fn add(self, offset: u32) -> Self::Output {
        let offset = offset as usize;
        self.with_content(self.content.start + offset..self.content.end + offset)
    }
}

impl Sub<usize> for HLSpan {
    type Output = Self;

    fn sub(self, offset: usize) -> Self::Output {
        self.with_content(
            self.content.start.saturating_sub(offset)..self.content.end.saturating_sub(offset),
        )
    }
}

/// A source-to-highlight projection consumed by the renderer.
pub trait Highlighter {
    /// Return ordered byte ranges carrying semantic theme groups.
    fn highlight(&mut self, source: &str) -> Vec<HLSpan>;

    /// Highlight live input as the continuation of retained incomplete source.
    fn highlight_continuation(&mut self, pending: Option<&str>, input: &str) -> Vec<HLSpan> {
        let Some(pending) = pending else {
            return self.highlight(input);
        };
        let prefix = format!("{pending}\n");
        let offset = prefix.len();
        self.highlight(&(prefix + input))
            .into_iter()
            .filter(|span| span.content.end > offset)
            .map(|span| span - offset)
            .collect()
    }
}

/// The Macaulay2 Tree-sitter highlighter using the grammar crate's own query.
pub struct SyntaxHighlighter {
    config: HighlightConfiguration,
    highlighter: TsHighlighter,
}

impl SyntaxHighlighter {
    const CAPTURES: &[(&str, HLGroup)] = &[
        ("comment", HLGroup::Comment),
        ("number", HLGroup::Number),
        ("number.float", HLGroup::Float),
        ("string", HLGroup::String),
        ("character.special", HLGroup::Quote),
        ("variable", HLGroup::Variable),
        ("operator", HLGroup::Operator),
        ("punctuation.bracket", HLGroup::Punctuation),
        ("punctuation.delimiter", HLGroup::Delimiter),
        ("keyword.conditional", HLGroup::Conditional),
        ("keyword.repeat", HLGroup::Repeat),
        ("keyword.return", HLGroup::Statement),
        ("keyword.exception", HLGroup::Exception),
        ("keyword.modifier", HLGroup::Keyword),
        ("keyword.debug", HLGroup::Debug),
        ("keyword.operator", HLGroup::Operator),
        ("keyword", HLGroup::Keyword),
        ("variable.parameter", HLGroup::Parameter),
        ("function", HLGroup::Function),
        ("property", HLGroup::Property),
        ("type", HLGroup::Type),
        ("function.call", HLGroup::Function),
        ("label", HLGroup::Installation),
        ("type.parameter", HLGroup::Type),
        ("function.builtin", HLGroup::CompiledFunction),
        ("variable.builtin", HLGroup::Builtin),
        ("constant.builtin", HLGroup::Builtin),
        ("boolean", HLGroup::Boolean),
        ("error", HLGroup::Error),
        ("string.special.url", HLGroup::Documentation),
        ("module", HLGroup::Package),
        ("namespace", HLGroup::Namespace),
        ("attribute", HLGroup::Attribute),
        ("function.macro", HLGroup::Macro),
        ("string.regexp", HLGroup::Regex),
    ];

    /// Configure highlighting from the query shipped with the grammar version.
    pub fn new() -> Self {
        let mut config = HighlightConfiguration::new(
            tree_sitter_macaulay2::language(),
            "macaulay2",
            tree_sitter_macaulay2::HIGHLIGHTS_QUERY,
            "",
            "",
        )
        .expect("tree-sitter-macaulay2's highlight query must match its grammar");
        config.configure(
            &Self::CAPTURES
                .iter()
                .map(|(capture, _)| *capture)
                .collect::<Vec<_>>(),
        );
        Self {
            config,
            highlighter: TsHighlighter::new(),
        }
    }
}

impl Default for SyntaxHighlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl Highlighter for SyntaxHighlighter {
    fn highlight(&mut self, source: &str) -> Vec<HLSpan> {
        let Ok(events) = self
            .highlighter
            .highlight(&self.config, source.as_bytes(), None, |_| None)
        else {
            return Vec::new();
        };
        let mut spans = Vec::new();
        let mut stack = Vec::new();
        for event in events {
            match event {
                Ok(HighlightEvent::HighlightStart(highlight)) => stack.push(highlight.0),
                Ok(HighlightEvent::HighlightEnd) => {
                    stack.pop();
                }
                Ok(HighlightEvent::Source { start, end }) => {
                    if let Some((_, kind)) =
                        stack.last().and_then(|index| Self::CAPTURES.get(*index))
                    {
                        spans.push(HLSpan::new(start..end, *kind));
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

    fn kind_at(source: &str, needle: &str) -> Option<HLGroup> {
        let offset = source.find(needle)?;
        SyntaxHighlighter::new()
            .highlight(source)
            .into_iter()
            .find(|span| span.content.contains(&offset))
            .map(|span| span.kind)
    }

    #[test]
    fn grammar_query_highlights_lexical_and_structural_roles() {
        for (source, needle, expected) in [
            ("if x then 1 else 0", "if", HLGroup::Conditional),
            ("1.0", "1.0", HLGroup::Float),
            ("1 + 2", "1", HLGroup::Number),
            ("f 1", "f", HLGroup::Function),
            ("loadPackage \"Foo\"", "\"Foo\"", HLGroup::Package),
            ("f ZZ := x -> x", "f", HLGroup::Installation),
            ("f ZZ := x -> x", "ZZ", HLGroup::Type),
            ("g = x -> x", "g", HLGroup::Function),
            ("g = x -> x", "x ->", HLGroup::Parameter),
            ("x.foo", "foo", HLGroup::Property),
        ] {
            assert_eq!(kind_at(source, needle), Some(expected), "{source:?}");
        }
    }

    #[test]
    fn every_public_query_capture_has_a_theme_group() {
        let highlighter = SyntaxHighlighter::new();

        for capture in highlighter
            .config
            .names()
            .iter()
            .filter(|capture| !capture.starts_with('_'))
        {
            assert!(
                SyntaxHighlighter::CAPTURES
                    .iter()
                    .any(|(mapped, _)| mapped == capture),
                "unmapped Tree-sitter capture: @{capture}"
            );
        }
    }

    #[test]
    fn incomplete_source_keeps_query_highlights() {
        let mut highlighter = SyntaxHighlighter::new();
        let spans = highlighter.highlight("if x then \"unfinished");

        assert!(spans.iter().any(|span| span.kind == HLGroup::Conditional));
        assert!(spans.iter().any(|span| span.kind == HLGroup::String));
    }

    #[test]
    fn continuation_highlights_are_rebased_to_the_live_input() {
        /// A highlighter returning one span over the exact source it receives.
        struct RecordingHighlighter(String);

        impl Highlighter for RecordingHighlighter {
            fn highlight(&mut self, source: &str) -> Vec<HLSpan> {
                self.0 = source.to_string();
                vec![HLSpan::new(0..source.len(), HLGroup::String)]
            }
        }

        let mut highlighter = RecordingHighlighter(String::new());
        let spans = highlighter.highlight_continuation(Some("f("), "x)");

        assert_eq!(highlighter.0, "f(\nx)");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, 0..2);
    }

    #[test]
    fn syntax_highlighting_crosses_the_incomplete_boundary() {
        let input = "continued\"";
        let spans = SyntaxHighlighter::new().highlight_continuation(Some("\"unfinished"), input);

        assert!(spans.iter().any(|span| {
            span.kind == HLGroup::String
                && span.content.start == 0
                && span.content.end <= input.len()
        }));
    }

    #[test]
    fn spans_are_ordered_non_overlapping_utf8_boundaries() {
        let source = "-- λ\nf = x -> x + 1";
        let mut highlighter = SyntaxHighlighter::new();
        let spans = highlighter.highlight(source);

        for pair in spans.windows(2) {
            assert!(pair[0].content.end <= pair[1].content.start);
        }
        for span in spans {
            assert!(source.is_char_boundary(span.content.start));
            assert!(source.is_char_boundary(span.content.end));
        }
    }
}
