//! Visual line wrapping for the feed.
//!
//! Soft wrapping is *visual only* — it never inserts a `\n`, because the stored text is
//! real M2 source (corrupting it would break the clipboard and the oracle). The wrapper
//! produces byte *ranges* of each visual row, not owned strings: cheap plain values that
//! can be cached next to the buffer, that the renderer slices into `Span`s, and that the
//! cursor maps an offset against to find its row. The word class is M2's, not the
//! default: `'` and `$` are part of an identifier, so a name like `x'` or `R$f` never
//! breaks across them.

use std::ops::Range;

/// True for the bytes M2 treats as part of one identifier — the wrap word-class.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '$'
}

/// Byte ranges of each visual row when `line` is wrapped to `width` columns: break only
/// before a char where [`is_word_char`] is false, hard-break a word longer than `width`.
/// The renderer slices `line` by these, the cursor maps an offset to its range.
pub fn wrap_line(line: &str, width: u16) -> Vec<Range<usize>> {
    let width = width as usize;
    if width == 0 || line.is_empty() {
        return std::iter::once(0..line.len()).collect();
    }
    // Char index → byte offset, with a sentinel past the end so a row can end on the
    // final char. Wrapping reasons in chars (display columns) but yields byte ranges.
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let n = chars.len();
    let byte_at = |i: usize| chars.get(i).map_or(line.len(), |&(b, _)| b);

    let mut rows = Vec::new();
    let mut start = 0; // first char index of the current row
    while start < n {
        if n - start <= width {
            rows.push(byte_at(start)..line.len());
            break;
        }
        let hard = start + width; // exclusive char index the row may not pass
                                  // Prefer breaking at the rightmost space in the window — consumed, not rendered.
                                  // Failing that, break just before the rightmost non-word char, keeping it. With
                                  // neither, the word itself is longer than `width`, so hard-break mid-word.
        let (end, next) = if let Some(i) = (start + 1..hard).rev().find(|&i| chars[i].1 == ' ') {
            (i, i + 1)
        } else if let Some(i) = (start + 1..hard).rev().find(|&i| !is_word_char(chars[i].1)) {
            (i, i)
        } else {
            (hard, hard)
        };
        rows.push(byte_at(start)..byte_at(end));
        start = next;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_line_is_one_row() {
        assert_eq!(wrap_line("a = 5", 80).len(), 1);
    }

    #[test]
    fn empty_line_still_occupies_one_row() {
        assert_eq!(wrap_line("", 80).len(), 1);
    }

    #[test]
    fn wraps_on_word_boundary_without_splitting_a_word() {
        assert_eq!(wrap_line("hello world", 8).len(), 2);
    }

    #[test]
    fn word_longer_than_width_hard_breaks() {
        assert_eq!(wrap_line("abcdefghij", 4).len(), 3);
    }

    #[test]
    fn m2_identifier_never_splits_on_prime_or_dollar() {
        // `x'` and `y$z` are single words — the wrap must keep them whole, never
        // breaking on the `'` or `$` the way a default word-class would.
        let rows = wrap_line("x' y$z", 3);
        assert_eq!(rows.len(), 2);
    }
}
