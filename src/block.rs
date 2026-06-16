//! The feed-block model and its trait layer.
//!
//! A feed block is one immutable, committed entry in the scrollback — executed
//! code, its output, an error, or a system note. We settled on three *orthogonal*
//! traits rather than one fat one:
//!
//! - **rendering** — `ratatui::widgets::Widget` on `&Block` plus an inherent
//!   `height` (added in the next slice); dispatched through the owning enum.
//! - **`TextOps`** — text content + wrapped height; every text-bearing block.
//! - **`Code: TextOps`** — highlight spans over the text; the M2-source blocks.
//!
//! Storage is the canonical owner; the render shape (`Vec<Line<Span>>`) is built
//! per frame and *borrows back* into these strings. So blocks own plain `String`s
//! and span lists over them — no `Rc` (single owner once submitted), no pre-wrapped
//! lines, no baked `Style` (theme resolves `Kind` at render). That keeps the model
//! orthogonal to the two things that change without the text: width and theme.

use std::ops::Range;

/// A highlight capture id (index into the highlighter's configured capture names),
/// resolved to a concrete `Style` by the theme at render time. Stored as the
/// semantic kind, never a baked `Style`, so re-theming never forces a re-parse.
pub type Kind = u32;

/// One run of source text sharing a single highlight kind. The range indexes into
/// the owning block's [`TextOps::text`]. Runs are flattened and non-overlapping.
pub type HighlightSpan = (Range<usize>, Kind);

/// Read and measurement operations shared by every text-bearing block.
///
/// Read-only by design: committed blocks are frozen and the live views are
/// projections, so the only mutator is `Editor` — which is not a block and does
/// not implement this trait. The scroll layout calls [`TextOps::wrapped_height`]
/// on every block to place it, which is what pulls this trait onto *all* of them.
pub trait TextOps {
    /// The canonical source bytes that every highlight and wrap span indexes into.
    fn text(&self) -> &str;

    /// Number of logical (newline-delimited) lines; at least 1.
    fn line_count(&self) -> usize;

    /// Physical rows this block occupies once word-wrapped to `width` columns.
    fn wrapped_height(&self, width: u16) -> u16;
}

/// Highlighted M2 source: [`TextOps`] plus the flattened highlight runs over
/// [`TextOps::text`]. Implemented by the committed code echo and (later) the live
/// input/pending views.
pub trait Code: TextOps {
    /// The flattened, non-overlapping highlight runs over `self.text()`.
    fn spans(&self) -> &[HighlightSpan];
}

/// A committed, executed input cell echoed into the feed. Owns its text outright —
/// single owner the moment it is submitted — alongside its frozen highlight and the
/// `iN` mirrored from M2 (never recomputed; M2's numbering rules are subtler than
/// they look).
pub struct CodeBlock {
    /// The cell's source. Plain owned `String`: unbounded length forces a heap
    /// handle, and nothing else references it once committed.
    pub text: String,
    /// Frozen highlight runs over `text`, computed once at commit.
    pub spans: Vec<HighlightSpan>,
    /// The `iN` line number M2 assigned this cell.
    pub number: u32,
}

impl TextOps for CodeBlock {
    fn text(&self) -> &str {
        &self.text
    }

    fn line_count(&self) -> usize {
        self.text.lines().count().max(1)
    }

    fn wrapped_height(&self, width: u16) -> u16 {
        wrapped_rows(&self.text, width)
    }
}

impl Code for CodeBlock {
    fn spans(&self) -> &[HighlightSpan] {
        &self.spans
    }
}

/// Total physical rows `text` occupies, word-wrapped to `width` columns — the shared
/// measurement behind every block's [`TextOps::wrapped_height`], and the same row
/// structure the renderer and caret will later ride on. We own the wrapping because
/// ratatui's is sealed render-only and cannot report this back.
fn wrapped_rows(text: &str, width: u16) -> u16 {
    if width == 0 {
        return text.lines().count().max(1) as u16;
    }
    text.lines()
        .map(|line| rows_for_line(line, width))
        .sum::<u16>()
        .max(1)
}

/// How many physical rows one logical line occupies when word-wrapped to `width`
/// columns (`width >= 1`).
fn rows_for_line(line: &str, width: u16) -> u16 {
    // TODO(human)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_line_is_one_row() {
        assert_eq!(rows_for_line("a = 5", 80), 1);
    }

    #[test]
    fn empty_line_still_occupies_one_row() {
        assert_eq!(rows_for_line("", 80), 1);
    }

    #[test]
    fn wraps_on_word_boundary_without_splitting_a_word() {
        // width 8: "hello" (5) fits; "hello world" (11) does not, so "world"
        // moves to row 2 — and "hello" is never split.
        assert_eq!(rows_for_line("hello world", 8), 2);
    }

    #[test]
    fn word_longer_than_width_hard_breaks() {
        // A single 10-char token cannot fit width 4, so it must break across
        // ceil(10 / 4) = 3 rows — the one case mid-token splitting is allowed.
        assert_eq!(rows_for_line("abcdefghij", 4), 3);
    }

    #[test]
    fn wrapped_rows_sums_logical_lines() {
        // Two logical lines, each one row at width 80.
        assert_eq!(wrapped_rows("a = 5\nb = 6", 80), 2);
    }
}
