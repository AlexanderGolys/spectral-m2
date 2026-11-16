//! Range-based projections from owned text into visual lines.

use std::iter;
use std::ops::Range;

use crate::ui::cycle::cyclic;
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthChar;

cyclic! {
    /// Where a long line breaks: at word/token boundaries or at terminal columns.
    /// For example, character wrapping can split `polynomial` across two rows.
    #[discriminated_str(label)]
    #[non_exhaustive]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum WrapMode {
        #[default]
        Word = "word",
        Character = "character",
    }
}

/// Wrap at terminal columns, preserving every source character.
#[derive(Default)]
pub struct CharacterWrap;

impl WrapPolicy for CharacterWrap {
    fn wrap_logical_line(&self, line: &str, width: u16, offset: usize) -> Vec<VisualLine> {
        let mut rows = Vec::new();
        let mut start = 0;
        let mut columns = 0;
        for (byte, character) in line.char_indices() {
            let cells = character.width().unwrap_or(0);
            if byte > start && columns + cells > usize::from(width.max(1)) {
                rows.push(VisualLine {
                    range: offset + start..offset + byte,
                    hard_continuation: start > 0,
                });
                start = byte;
                columns = 0;
            }
            columns += cells;
        }
        rows.push(VisualLine {
            range: offset + start..offset + line.len(),
            hard_continuation: start > 0,
        });
        rows
    }
}

impl WrapMode {
    /// Wrap editable input and retain an empty row for a caret at a full edge.
    pub fn caret_lines(self, text: &str, width: u16) -> Vec<VisualLine> {
        match self {
            Self::Word => CaretWrap(WordWrap).wrap(text, width),
            Self::Character => CaretWrap(CharacterWrap).wrap(text, width),
        }
    }
}

/// A visual row represented as a byte range into its source text.
pub struct VisualLine {
    pub range: Range<usize>,
    pub hard_continuation: bool,
}

impl VisualLine {
    fn shifted(mut self, offset: usize) -> Self {
        self.range = offset + self.range.start..offset + self.range.end;
        self
    }
}
/// The byte offset in `text` of the character drawn at terminal `column`, or the
/// end of `text` when it is narrower; a wide character spans every column it
/// covers.
pub fn offset_at_column(text: &str, column: usize) -> usize {
    let mut columns = 0;
    for (offset, character) in text.char_indices() {
        columns += character.width().unwrap_or(0);
        if columns > column {
            return offset;
        }
    }
    text.len()
}

/// A strategy for projecting one logical line into byte ranges for visual rows.
pub trait WrapPolicy {
    fn wrap_logical_line(&self, line: &str, width: u16, offset: usize) -> Vec<VisualLine>;

    fn wrap(&self, text: &str, width: u16) -> Vec<VisualLine> {
        let mut lines = Vec::new();
        let mut offset = 0;
        for logical in text.split('\n') {
            lines.extend(self.wrap_logical_line(logical, width, offset));
            offset += logical.len() + 1;
        }
        lines
    }
}

/// Text that can be projected into visual rows by an associated wrapping policy.
pub trait TextLayout {
    type Wrapping: Default + WrapPolicy;

    fn text(&self) -> &str;

    fn lines(&self, width: u16) -> Vec<VisualLine> {
        Self::Wrapping::default().wrap(self.text(), width)
    }
}

/// Source range and continuation state behind one wrapped row.
struct Projection {
    source: Range<usize>,
    hard_continuation: bool,
}

/// Contiguous display text produced by wrapping, together with enough mapping
/// information to relocate byte ranges from the source.
pub struct WrappedText {
    text: String,
    projections: Vec<Projection>,
}

impl WrappedText {
    /// Wrap ordinary text using word-like boundaries.
    pub fn word(source: &str, width: u16) -> Self {
        Self::from_lines(source, WordWrap.wrap(source, width))
    }

    /// Wrap code at whitespace and parsed span boundaries.
    pub fn code(source: &str, width: u16, spans: &[Range<usize>], mode: WrapMode) -> Self {
        let lines = match mode {
            WrapMode::Word => SpanWrap(spans).wrap(source, width),
            WrapMode::Character => CharacterWrap.wrap(source, width),
        };
        Self::from_lines(source, lines)
    }

    fn from_lines(source: &str, lines: Vec<VisualLine>) -> Self {
        let mut text = String::new();
        let mut projections = Vec::with_capacity(lines.len());
        for (index, line) in lines.into_iter().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            text.push_str(&source[line.range.clone()]);
            projections.push(Projection {
                source: line.range,
                hard_continuation: line.hard_continuation,
            });
        }
        Self { text, projections }
    }

    /// Return the complete wrapped text, with visual rows separated by newlines.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Return the number of visual rows in the wrapped text.
    pub fn height(&self) -> u16 {
        self.projections.len().try_into().unwrap_or(u16::MAX)
    }

    /// Return the source byte range behind each visual row.
    pub fn source_lines(&self) -> impl Iterator<Item = (&Range<usize>, bool)> {
        self.projections
            .iter()
            .map(|projection| (&projection.source, projection.hard_continuation))
    }
}

/// A wrapping policy for text being edited. A line ending exactly on the width gets
/// an empty row after it, so the caret has somewhere inside the pane to sit and
/// the next character appears where the caret already is.
#[derive(Default)]
pub struct CaretWrap<P = WordWrap>(pub P);

impl<P: WrapPolicy> WrapPolicy for CaretWrap<P> {
    fn wrap_logical_line(&self, line: &str, width: u16, offset: usize) -> Vec<VisualLine> {
        let mut rows = self.0.wrap_logical_line(line, width, offset);
        let ends_on_width = rows.last().is_some_and(|row| {
            let columns: usize = line[row.range.start - offset..row.range.end - offset]
                .chars()
                .map(|character| character.width().unwrap_or(0))
                .sum();
            width > 0 && columns == usize::from(width)
        });
        if ends_on_width {
            let end = offset + line.len();
            rows.push(VisualLine {
                range: end..end,
                hard_continuation: true,
            });
        }
        rows
    }
}

/// Soft-wrap logical lines at word-like boundaries.
#[derive(Default)]
pub struct WordWrap;

impl WrapPolicy for WordWrap {
    fn wrap_logical_line(&self, line: &str, width: u16, offset: usize) -> Vec<VisualLine> {
        let width = width as usize;
        if width == 0 || line.is_empty() {
            return iter::once(VisualLine {
                range: 0..line.len(),
                hard_continuation: false,
            })
            .map(|line| line.shifted(offset))
            .collect();
        }

        let characters: Vec<(usize, char)> = line.char_indices().collect();
        let count = characters.len();
        let byte_at = |index: usize| characters.get(index).map_or(line.len(), |&(byte, _)| byte);
        let mut rows = Vec::new();
        let mut start = 0;
        let mut hard_continuation = false;

        while start < count {
            // The first character past the row, measured in terminal columns.
            let mut hard_end = start;
            let mut columns = 0;
            while hard_end < count {
                let cells = characters[hard_end].1.width().unwrap_or(0);
                if hard_end > start && columns + cells > width {
                    break;
                }
                columns += cells;
                hard_end += 1;
            }
            if hard_end == count {
                rows.push(VisualLine {
                    range: byte_at(start)..line.len(),
                    hard_continuation,
                });
                break;
            }
            let (end, next, hard_break) = if let Some(index) = (start + 1..hard_end)
                .rev()
                .find(|&index| characters[index].1 == ' ')
            {
                (index, index + 1, false)
            } else if let Some(index) = (start + 1..hard_end)
                .rev()
                .find(|&index| !is_word_character(characters[index].1))
            {
                (index, index, false)
            } else {
                (hard_end, hard_end, true)
            };
            rows.push(VisualLine {
                range: byte_at(start)..byte_at(end),
                hard_continuation,
            });
            start = next;
            hard_continuation = hard_break;
        }
        rows.into_iter().map(|line| line.shifted(offset)).collect()
    }
}

/// Code wrapping that never splits a word or a token: each row ends at the last
/// place that fits where whitespace stands or one token meets the next, and only
/// a word wider than the whole row is cut.
pub struct SpanWrap<'a>(pub &'a [Range<usize>]);

impl SpanWrap<'_> {
    /// Whether a row may end before byte `at`, between `before` and `after`.
    fn parts(&self, at: usize, before: char, after: char) -> bool {
        let within_word = is_word_character(before) && is_word_character(after);
        let within_token = self.0.iter().any(|span| span.start < at && at < span.end);
        !within_word && !within_token
    }
}

impl WrapPolicy for SpanWrap<'_> {
    fn wrap_logical_line(
        &self,
        logical: &str,
        width: u16,
        logical_start: usize,
    ) -> Vec<VisualLine> {
        let width = usize::from(width);
        let characters: Vec<(usize, char)> = logical.char_indices().collect();
        let count = characters.len();
        let byte_at = |index: usize| {
            logical_start
                + characters
                    .get(index)
                    .map_or(logical.len(), |&(byte, _)| byte)
        };
        let mut lines = Vec::new();
        let mut start = 0;
        let mut hard_continuation = false;
        if width == 0 || characters.is_empty() {
            lines.push(VisualLine {
                range: logical_start..logical_start + logical.len(),
                hard_continuation: false,
            });
            return lines;
        }
        loop {
            // The first character past the row, measured in terminal columns.
            let mut past = start;
            let mut columns = 0;
            while past < count {
                let cells = characters[past].1.width().unwrap_or(0);
                if past > start && columns + cells > width {
                    break;
                }
                columns += cells;
                past += 1;
            }
            if past == count {
                lines.push(VisualLine {
                    range: byte_at(start)..byte_at(count),
                    hard_continuation,
                });
                return lines;
            }
            let whitespace = |index: usize| characters[index].1.is_whitespace();
            // A row ending at whitespace drops it; one ending between tokens keeps all.
            let parting = (start + 1..=past).rev().find_map(|index| {
                if index - 1 > start && whitespace(index - 1) {
                    Some((index - 1, index))
                } else if index < count && whitespace(index) {
                    Some((index, index + 1))
                } else if self.parts(byte_at(index), characters[index - 1].1, characters[index].1) {
                    Some((index, index))
                } else {
                    None
                }
            });
            let word_end = || {
                (start + 1..past)
                    .rev()
                    .find(|&index| {
                        !(is_word_character(characters[index - 1].1)
                            && is_word_character(characters[index].1))
                    })
                    .map(|index| (index, index))
            };
            let (end, next, cut) = match parting.or_else(word_end) {
                Some((end, next)) => (end, next, false),
                None => (past, past, true),
            };
            lines.push(VisualLine {
                range: byte_at(start)..byte_at(end),
                hard_continuation,
            });
            start = next;
            hard_continuation = cut;
            if start >= count {
                lines.push(VisualLine {
                    range: byte_at(count)..byte_at(count),
                    hard_continuation: false,
                });
                return lines;
            }
        }
    }
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '\'' || character == '$'
}
