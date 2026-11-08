//! Tree-sitter query captures projected onto the highlight groups that declare them.

use std::ops::{Add, Sub};

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter as TsHighlighter};

use crate::ui::block::HLSpan;
use crate::ui::theme::groups::{HLGroup, GROUP_DECLARATIONS};

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
    /// Revision of cached semantic spans, independent of source edits.
    fn revision(&self) -> u64 {
        0
    }

    /// Highlight a source segment by its identity within the session.
    fn highlight_source(&mut self, _: super::semantic::SourceId, source: &str) -> Vec<HLSpan> {
        self.highlight(source)
    }

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
    /// The group drawing each configured capture, by its configuration index.
    groups: Vec<HLGroup>,
    highlighter: TsHighlighter,
}

impl SyntaxHighlighter {
    /// Configure the grammar's own query with the captures the highlight groups declare.
    pub fn new() -> Self {
        let (captures, groups): (Vec<_>, Vec<_>) = GROUP_DECLARATIONS
            .captures()
            .expect("loaded group declarations have one group per capture")
            .into_iter()
            .unzip();
        let mut config = HighlightConfiguration::new(
            tree_sitter_macaulay2::language(),
            "macaulay2",
            tree_sitter_macaulay2::HIGHLIGHTS_QUERY,
            "",
            "",
        )
        .expect("tree-sitter-macaulay2's highlight query must match its grammar");
        config.configure(&captures);
        Self {
            config,
            groups,
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
                    if let Some(kind) = stack.last().and_then(|index| self.groups.get(*index)) {
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

    #[test]
    fn every_public_query_capture_has_a_theme_group() {
        let highlighter = SyntaxHighlighter::new();
        let captures = GROUP_DECLARATIONS.captures().unwrap();

        for capture in highlighter
            .config
            .names()
            .iter()
            .filter(|capture| !capture.starts_with('_'))
        {
            assert!(
                captures.iter().any(|(mapped, _)| mapped == capture),
                "unmapped Tree-sitter capture: @{capture}"
            );
        }
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
