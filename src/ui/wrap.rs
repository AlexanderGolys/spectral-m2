//! Range-based projections from owned text into visual lines.

use std::iter;
use std::ops::Range;

use unicode_width::UnicodeWidthChar;

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
    pub fn code(source: &str, width: u16, spans: &[Range<usize>]) -> Self {
        Self::from_lines(source, SpanWrap(spans).wrap(source, width))
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

/// Word wrapping for text being edited. A line ending exactly on the width gets
/// an empty row after it, so the caret has somewhere inside the pane to sit and
/// the next character appears where the caret already is.
#[derive(Default)]
pub struct CaretWrap;

impl WrapPolicy for CaretWrap {
    fn wrap_logical_line(&self, line: &str, width: u16, offset: usize) -> Vec<VisualLine> {
        let mut rows = WordWrap.wrap_logical_line(line, width, offset);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Glyphs whose rendered width is the same under every terminal convention.
    ///
    /// `unicode-width` reports East Asian Ambiguous characters as one column, while a
    /// terminal configured for CJK renders them as two. Neither answer is wrong, and
    /// nothing portable can ask which applies, so a glyph that differs between the two
    /// conventions will misalign a border on somebody's machine no matter how carefully
    /// this crate measures. Choosing from the stable set is the only real defence.
    fn width_is_stable(glyph: char) -> bool {
        use unicode_width::UnicodeWidthChar;
        glyph.width() == glyph.width_cjk()
    }

    /// Report each candidate as (glyph, western columns, CJK columns).
    fn screen(candidates: &str) -> Vec<(char, Option<usize>, Option<usize>)> {
        use unicode_width::UnicodeWidthChar;
        candidates
            .chars()
            .map(|glyph| (glyph, glyph.width(), glyph.width_cjk()))
            .collect()
    }

    #[test]
    fn rendered_value_glyphs_keep_one_width_everywhere() {
        // Safe to emit from the value renderer: matrix brackets, blackboard bold,
        // and the elision mark all measure one column under both conventions.
        for glyph in "ℤℚℝℂ⋯⋅∖⟵⟶⟼⎧⎨⎩⎰⎱⎪⎫⎬⎭⎛⎝⎞⎠".chars()
        {
            assert!(width_is_stable(glyph), "{glyph} is ambiguous");
        }
    }

    #[test]
    fn the_ambiguous_glyphs_this_shell_already_draws_are_known() {
        // Recorded rather than asserted away: these are in use today, and any terminal
        // treating ambiguous as double columns will draw them two cells wide.
        let ambiguous: String = screen("─⇒²³│╭╮╰╯┈×…")
            .into_iter()
            .filter(|(glyph, ..)| !width_is_stable(*glyph))
            .map(|(glyph, ..)| glyph)
            .collect();

        assert_eq!(ambiguous, "─⇒²³│╭╮╰╯┈×…");
    }

    #[test]
    fn token_wrapping_prefers_whitespace_then_token_boundaries() {
        let whitespace = SpanWrap(&[0..2, 3..7]).wrap("aa bbbb", 4);
        assert_eq!(whitespace[0].range, 0..2);
        assert_eq!(whitespace[1].range, 3..7);
        assert!(!whitespace[1].hard_continuation);

        let tokens = SpanWrap(&[0..3, 3..4, 4..7]).wrap("abc+def", 5);
        assert_eq!(tokens[0].range, 0..4);
        assert_eq!(tokens[1].range, 4..7);
        assert!(!tokens[1].hard_continuation);
    }

    /// However narrow the row, a name or keyword is never cut while a place
    /// between tokens fits.
    #[test]
    fn code_never_splits_a_word_that_could_move_whole() {
        let source = "result = matrix{{alpha, beta}} ** identity";
        let words: Vec<Range<usize>> = source.match_indices(|ch: char| ch.is_alphanumeric()).fold(
            Vec::<Range<usize>>::new(),
            |mut words, (at, ch)| {
                match words.last_mut().filter(|word| word.end == at) {
                    Some(word) => word.end = at + ch.len(),
                    None => words.push(at..at + ch.len()),
                }
                words
            },
        );
        for width in 9..=source.len() as u16 {
            for row in SpanWrap(&words).wrap(source, width) {
                for word in &words {
                    let cut = word.start < row.range.end && row.range.end < word.end
                        || word.start < row.range.start && row.range.start < word.end;
                    assert!(
                        !cut,
                        "{width}: {:?} cuts {:?}",
                        row.range,
                        &source[word.clone()]
                    );
                }
            }
        }
    }

    #[test]
    fn code_is_measured_in_terminal_columns() {
        let rows = SpanWrap(&[]).wrap("漢漢漢", 4);
        assert_eq!(rows[0].range, 0.."漢漢".len());
    }

    #[test]
    fn a_single_long_token_marks_its_continuation() {
        let span = 0..8;
        let lines = SpanWrap(std::slice::from_ref(&span)).wrap("abcdefgh", 4);
        assert_eq!(lines[0].range, 0..4);
        assert!(!lines[0].hard_continuation);
        assert_eq!(lines[1].range, 4..8);
        assert!(lines[1].hard_continuation);
    }

    #[test]
    fn edited_text_breaks_between_words_and_leaves_the_caret_a_row() {
        let text = "abc defg";
        let rows: Vec<_> = CaretWrap
            .wrap(text, 4)
            .into_iter()
            .map(|row| &text[row.range])
            .collect();
        assert_eq!(rows, ["abc", "defg", ""]);
    }

    #[test]
    fn word_wrap_measures_wide_characters_by_their_columns() {
        let text = "ααα 中中中";
        for row in WordWrap.wrap(text, 4) {
            let columns: usize = text[row.range]
                .chars()
                .map(|character| character.width().unwrap_or(0))
                .sum();
            assert!(columns <= 4);
        }
    }

    #[test]
    fn shared_wrapper_reports_hard_word_continuations() {
        let wrapped = WrappedText::word("abcdefgh", 4);

        assert_eq!(wrapped.text(), "abcd\nefgh");
        assert_eq!(
            wrapped
                .source_lines()
                .filter_map(|(range, hard)| hard.then_some(range.start))
                .collect::<Vec<_>>(),
            [4]
        );
    }
}
