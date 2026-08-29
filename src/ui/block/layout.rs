//! Range-based projections from owned text into visual lines.

use std::iter;
use std::ops::Range;

/// A strategy for projecting one logical line into byte ranges for visual rows.
pub trait WrapPolicy {
    fn wrap_logical_line(line: &str, width: u16) -> Vec<VisualLine>;
}

/// Text that can be projected into visual rows by an associated wrapping policy.
pub trait TextLayout {
    type Wrapping: WrapPolicy;

    fn text(&self) -> &str;

    fn lines(&self, width: u16) -> Vec<VisualLine> {
        wrap::<Self::Wrapping>(self.text(), width)
    }
}

/// A visual row represented as a byte range into its source text.
pub struct VisualLine {
    pub range: Range<usize>,
    pub hard_continuation: bool,
}

/// Corresponding byte ranges in the source and wrapped text.
struct Projection {
    source: Range<usize>,
    wrapped: Range<usize>,
}

/// Contiguous display text produced by wrapping, together with enough mapping
/// information to relocate byte ranges from the source.
pub struct WrappedText {
    text: String,
    projections: Vec<Projection>,
    hard_continuations: Vec<usize>,
}

impl WrappedText {
    /// Wrap ordinary text using word-like boundaries.
    pub fn word(source: &str, width: u16) -> Self {
        Self::from_lines(source, wrap::<WordWrap>(source, width))
    }

    /// Wrap code at whitespace and syntax-token boundaries.
    pub fn code(source: &str, width: u16, token_ends: &[usize]) -> Self {
        Self::from_lines(source, wrap_at_tokens(source, width, token_ends))
    }

    fn from_lines(source: &str, lines: Vec<VisualLine>) -> Self {
        let mut text = String::new();
        let mut projections = Vec::with_capacity(lines.len());
        let mut hard_continuations = Vec::new();
        for (index, line) in lines.into_iter().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            let start = text.len();
            if line.hard_continuation {
                hard_continuations.push(start);
            }
            text.push_str(&source[line.range.clone()]);
            projections.push(Projection {
                source: line.range,
                wrapped: start..text.len(),
            });
        }
        Self {
            text,
            projections,
            hard_continuations,
        }
    }

    /// Return the complete wrapped text, with visual rows separated by newlines.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Return the number of visual rows in the wrapped text.
    pub fn height(&self) -> u16 {
        self.text.matches('\n').count() as u16 + 1
    }

    /// Return wrapped byte offsets whose rows continue a hard-split token.
    pub fn hard_continuations(&self) -> impl Iterator<Item = usize> + '_ {
        self.hard_continuations.iter().copied()
    }

    /// Relocate one source byte range, splitting it when wrapping split its text.
    pub fn relocate(&self, range: &Range<usize>) -> Vec<Range<usize>> {
        self.projections
            .iter()
            .filter_map(|projection| {
                let start = range.start.max(projection.source.start);
                let end = range.end.min(projection.source.end);
                (start < end).then(|| {
                    projection.wrapped.start + start - projection.source.start
                        ..projection.wrapped.start + end - projection.source.start
                })
            })
            .collect()
    }
}

/// Break logical lines at exactly the available width, mid-token if necessary.
/// Code has no reliable word boundaries to break on and the caller needs the
/// cursor to map onto a predictable column, so every row but the last is full.
pub struct HardWrap;

impl WrapPolicy for HardWrap {
    fn wrap_logical_line(line: &str, width: u16) -> Vec<VisualLine> {
        let width = width as usize;
        if width == 0 || line.is_empty() {
            return iter::once(VisualLine {
                range: 0..line.len(),
                hard_continuation: false,
            })
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
        rows
    }
}

/// Soft-wrap logical lines at word-like boundaries.
pub struct WordWrap;

impl WrapPolicy for WordWrap {
    fn wrap_logical_line(line: &str, width: u16) -> Vec<VisualLine> {
        let width = width as usize;
        if width == 0 || line.is_empty() {
            return iter::once(VisualLine {
                range: 0..line.len(),
                hard_continuation: false,
            })
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
        rows
    }
}

/// Project a source string into visual rows with the selected policy.
pub fn wrap<Policy: WrapPolicy>(text: &str, width: u16) -> Vec<VisualLine> {
    let mut lines = Vec::new();
    let mut logical_start = 0;
    for logical in text.split('\n') {
        lines.extend(
            Policy::wrap_logical_line(logical, width)
                .into_iter()
                .map(|line| VisualLine {
                    range: logical_start + line.range.start..logical_start + line.range.end,
                    hard_continuation: line.hard_continuation,
                }),
        );
        logical_start += logical.len() + 1;
    }
    lines
}

/// Wrap code at whitespace or highlighted token boundaries, falling back to a
/// character boundary when one token is wider than the available row.
pub fn wrap_at_tokens(text: &str, width: u16, token_ends: &[usize]) -> Vec<VisualLine> {
    let width = width as usize;
    let mut lines = Vec::new();
    let mut logical_start = 0;

    for logical in text.split('\n') {
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
            logical_start += logical.len() + 1;
            continue;
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
            if let Some(&end) = token_ends
                .iter()
                .rev()
                .find(|&&end| start_byte < end && end <= limit_byte)
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
        logical_start += logical.len() + 1;
    }

    lines
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '\'' || character == '$'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_wrapping_prefers_whitespace_then_token_boundaries() {
        let whitespace = wrap_at_tokens("aa bbbb", 4, &[2, 7]);
        assert_eq!(whitespace[0].range, 0..2);
        assert_eq!(whitespace[1].range, 3..7);
        assert!(!whitespace[1].hard_continuation);

        let tokens = wrap_at_tokens("abc+def", 5, &[3, 4, 7]);
        assert_eq!(tokens[0].range, 0..4);
        assert_eq!(tokens[1].range, 4..7);
        assert!(!tokens[1].hard_continuation);
    }

    #[test]
    fn a_single_long_token_marks_its_continuation() {
        let lines = wrap_at_tokens("abcdefgh", 4, &[8]);
        assert_eq!(lines[0].range, 0..4);
        assert!(!lines[0].hard_continuation);
        assert_eq!(lines[1].range, 4..8);
        assert!(lines[1].hard_continuation);
    }

    #[test]
    fn shared_wrapper_reports_hard_word_continuations() {
        let wrapped = WrappedText::word("abcdefgh", 4);

        assert_eq!(wrapped.text(), "abcd\nefgh");
        assert_eq!(wrapped.hard_continuations().collect::<Vec<_>>(), [5]);
    }
}
