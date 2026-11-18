//! Tree-sitter query captures projected onto the highlight groups that declare them.

use std::ops::{Add, Sub};

use tree_sitter::Language;
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

/// A Tree-sitter grammar with its crate's own queries, known by the names an
/// injection gives it, such as the `m2` of a fenced Markdown code block.
pub struct Grammar {
    names: &'static [&'static str],
    language: fn() -> Language,
    highlights: &'static str,
    injections: &'static str,
}

impl Grammar {
    pub const MACAULAY2: Self = Self {
        names: &["macaulay2", "m2"],
        language: tree_sitter_macaulay2::language,
        highlights: tree_sitter_macaulay2::HIGHLIGHTS_QUERY,
        injections: "",
    };
    /// The block structure of Markdown; its inline content is [`Self::MARKDOWN_INLINE`].
    pub const MARKDOWN: Self = Self {
        names: &["markdown", "md"],
        language: || tree_sitter_md::LANGUAGE.into(),
        highlights: tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
        injections: include_str!("markdown_injections.scm"),
    };
    pub const MARKDOWN_INLINE: Self = Self {
        names: &["markdown_inline"],
        language: || tree_sitter_md::INLINE_LANGUAGE.into(),
        highlights: tree_sitter_md::HIGHLIGHT_QUERY_INLINE,
        injections: tree_sitter_md::INJECTION_QUERY_INLINE,
    };

    fn configuration(&self, captures: &[String]) -> HighlightConfiguration {
        let mut config = HighlightConfiguration::new(
            (self.language)(),
            self.names[0],
            self.highlights,
            self.injections,
            "",
        )
        .expect("a grammar crate's own queries must match its grammar");
        config.configure(captures);
        config
    }
}

/// A Tree-sitter highlighter for one grammar and the grammars it injects.
pub struct SyntaxHighlighter {
    /// The highlighted grammar's configuration first, then each injectable one's.
    configs: Vec<(&'static [&'static str], HighlightConfiguration)>,
    /// The group drawing each configured capture, by its configuration index.
    groups: Vec<HLGroup>,
    highlighter: TsHighlighter,
}

impl SyntaxHighlighter {
    /// Highlight Macaulay2 code.
    pub fn new() -> Self {
        Self::of(&[Grammar::MACAULAY2])
    }

    /// Highlight Markdown prose, with Macaulay2 in its code blocks.
    pub fn markdown() -> Self {
        Self::of(&[
            Grammar::MARKDOWN,
            Grammar::MARKDOWN_INLINE,
            Grammar::MACAULAY2,
        ])
    }

    /// Highlight the first of `grammars`, injecting any of the rest by name, with
    /// the captures the highlight groups declare.
    fn of(grammars: &[Grammar]) -> Self {
        let (captures, groups): (Vec<_>, Vec<_>) = GROUP_DECLARATIONS
            .captures()
            .expect("loaded group declarations have one group per capture")
            .into_iter()
            .unzip();
        Self {
            configs: grammars
                .iter()
                .map(|grammar| (grammar.names, grammar.configuration(&captures)))
                .collect(),
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
        let configs = &self.configs;
        let injected = |name: &str| {
            configs
                .iter()
                .find(|(names, _)| names.contains(&name))
                .map(|(_, config)| config)
        };
        let Ok(events) =
            self.highlighter
                .highlight(&configs[0].1, source.as_bytes(), None, injected)
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
