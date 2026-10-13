//! Range-based projections from owned text into visual lines.

use std::iter;
use std::ops::Range;

/// A visual row represented as a byte range into its source text.
pub(crate) struct VisualLine {
    pub(crate) range: Range<usize>,
    pub(crate) hard_continuation: bool,
}

impl VisualLine {
    fn shifted(mut self, offset: usize) -> Self {
        self.range = offset + self.range.start..offset + self.range.end;
        self
    }
}
/// A strategy for projecting one logical line into byte ranges for visual rows.
pub(crate) trait WrapPolicy {
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
pub(crate) trait TextLayout {
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
pub(crate) struct WrappedText {
    text: String,
    projections: Vec<Projection>,
}

impl WrappedText {
    /// Wrap ordinary text using word-like boundaries.
    pub(crate) fn word(source: &str, width: u16) -> Self {
        Self::from_lines(source, WordWrap.wrap(source, width))
    }

    /// Wrap code at whitespace and parsed span boundaries.
    pub(crate) fn code(source: &str, width: u16, spans: &[Range<usize>]) -> Self {
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
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Return the number of visual rows in the wrapped text.
    pub(crate) fn height(&self) -> u16 {
        self.projections.len().try_into().unwrap_or(u16::MAX)
    }

    /// Return the source byte range behind each visual row.
    pub(crate) fn source_lines(&self) -> impl Iterator<Item = (&Range<usize>, bool)> {
        self.projections
            .iter()
            .map(|projection| (&projection.source, projection.hard_continuation))
    }
}

/// Break logical lines at exactly the available width, mid-token if necessary.
/// Code has no reliable word boundaries to break on and the caller needs the
/// cursor to map onto a predictable column, so every row but the last is full.
#[derive(Default)]
pub(crate) struct HardWrap;

impl WrapPolicy for HardWrap {
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
        let mut rows = Vec::new();
        let mut start = 0;
        let mut columns = 0;
        let mut hard_continuation = false;
        for (byte, _) in line.char_indices() {
            if columns == width {
                rows.push(VisualLine {
                    range: start..byte,
                    hard_continuation,
                });
                start = byte;
                columns = 0;
                hard_continuation = true;
            }
            columns += 1;
        }
        rows.push(VisualLine {
            range: start..line.len(),
            hard_continuation,
        });
        // A line that ends exactly on a boundary gets an empty row after it, so
        // the caret has somewhere inside the pane to sit and the next character
        // appears where the caret already is.
        if columns == width {
            rows.push(VisualLine {
                range: line.len()..line.len(),
                hard_continuation: true,
            });
        }
        rows.into_iter().map(|line| line.shifted(offset)).collect()
    }
}

/// Soft-wrap logical lines at word-like boundaries.
#[derive(Default)]
pub(crate) struct WordWrap;

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
            if count - start <= width {
                rows.push(VisualLine {
                    range: byte_at(start)..line.len(),
                    hard_continuation,
                });
                break;
            }
            let hard_end = start + width;
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

/// Parsed-span wrapping that never guesses syntax boundaries.
struct SpanWrap<'a>(pub(crate) &'a [Range<usize>]);

impl WrapPolicy for SpanWrap<'_> {
    fn wrap_logical_line(
        &self,
        logical: &str,
        width: u16,
        logical_start: usize,
    ) -> Vec<VisualLine> {
        let width = width as usize;
        let mut lines = Vec::new();
        let characters: Vec<(usize, char)> = logical.char_indices().collect();
        let byte_at = |index: usize| {
            characters
                .get(index)
                .map_or(logical.len(), |&(byte, _)| byte)
        };
        let mut start = 0;
        let mut hard_continuation = false;

        if width == 0 || characters.is_empty() {
            lines.push(VisualLine {
                range: logical_start..logical_start + logical.len(),
                hard_continuation: false,
            });
            return lines;
        }

        while characters.len() - start > width {
            let limit = start + width;
            let whitespace = (start + 1..=limit)
                .rev()
                .find(|&index| characters[index].1.is_whitespace());
            if let Some(end) = whitespace {
                lines.push(VisualLine {
                    range: logical_start + byte_at(start)..logical_start + byte_at(end),
                    hard_continuation,
                });
                start = end + 1;
                hard_continuation = false;
                continue;
            }

            let start_byte = logical_start + byte_at(start);
            let limit_byte = logical_start + byte_at(limit);
            if let Some(end) = self
                .0
                .iter()
                .rev()
                .map(|span| span.end)
                .find(|&end| start_byte < end && end <= limit_byte)
            {
                lines.push(VisualLine {
                    range: start_byte..end,
                    hard_continuation,
                });
                start = characters.partition_point(|&(byte, _)| logical_start + byte < end);
                hard_continuation = false;
                continue;
            }

            lines.push(VisualLine {
                range: start_byte..limit_byte,
                hard_continuation,
            });
            start = limit;
            hard_continuation = true;
        }

        lines.push(VisualLine {
            range: logical_start + byte_at(start)..logical_start + logical.len(),
            hard_continuation,
        });
        lines
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
        for glyph in "ℤℚℝℂ⋯⋅∖⟵⟶⟼⎧⎨⎩⎪⎫⎬⎭⎛⎝⎞⎠".chars() {
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
