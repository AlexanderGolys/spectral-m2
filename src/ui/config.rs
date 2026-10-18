//! Compile-time presentation configuration for the complete interface.

use std::sync::LazyLock;

/// A preferred symbol and a plain Unicode fallback for limited terminals.
#[derive(Clone, Copy)]
pub struct Glyph {
    preferred: &'static str,
    basic: &'static str,
}

impl Glyph {
    /// Select the symbol for a terminal's glyph capability.
    pub const fn for_basic(self, basic: bool) -> &'static str {
        if basic {
            self.basic
        } else {
            self.preferred
        }
    }

    /// Select the symbol for this process's terminal.
    pub fn current(self) -> &'static str {
        self.for_basic(Glyphs::basic_terminal())
    }
}

/// UI symbols that have a simpler spelling on limited terminals.
#[derive(Clone, Copy)]
pub struct Glyphs {
    pub prompt: Glyph,
    pub split_divider: Glyph,
    pub cell_separator: Glyph,
    pub pending_separator: Glyph,
    pub fold_rule: Glyph,
    pub input_edge: Glyph,
    pub unevaluated_cell: Glyph,
    pub rejected_cell: Glyph,
}

impl Glyphs {
    /// `TERM=dumb` and an explicit override use plain Unicode characters.
    pub fn basic_terminal() -> bool {
        static BASIC: LazyLock<bool> = LazyLock::new(|| {
            std::env::var("TERM").is_ok_and(|term| term == "dumb")
                || std::env::var("SPECTRAL_M2_BASIC_GLYPHS").is_ok_and(|value| value == "1")
        });
        *BASIC
    }
}

/// Frames and cadence of the pending-cell spinner.
#[derive(Clone, Copy)]
pub struct SpinnerConfig {
    pub frames: &'static [&'static str],
    pub basic_frames: &'static [&'static str],
    pub frame_ms: u64,
}

impl SpinnerConfig {
    /// Spinner symbols chosen for the terminal.
    pub fn active_frames(self) -> &'static [&'static str] {
        if Glyphs::basic_terminal() {
            self.basic_frames
        } else {
            self.frames
        }
    }
}

/// Geometry values shared by UI elements.
pub struct UiConfig {
    /// Columns kept clear down the whole screen's left edge, around every pane.
    pub screen_margin: u16,
    pub feed_margin: u16,
    /// Symbols used in shell chrome and their compatibility fallbacks.
    pub glyphs: Glyphs,
    /// Animation of a pending cell marker.
    pub spinner: SpinnerConfig,
    pub prompt_margin: u16,
    pub cell_margin: u16,
    pub cell_gap: u16,
    /// Digits every cell number is laid out for, however few it has, so a number
    /// growing a digit never moves a block.
    pub min_number_digits: u16,
    pub prompt_text_gap: u16,
    pub right_margin: u16,
    pub min_box_width: u16,
    pub min_input_height: u16,
    pub picker_tab_width: usize,
    pub picker_column_gap: usize,
    pub picker_right_margin: usize,
    pub statusline_margin: u16,
    pub max_output_rows: usize,
    /// Bytes of flat output admitted to synchronous preview formatting.
    pub max_output_preview_bytes: usize,
    /// Cells admitted to a value's expanded preview buffer.
    pub max_output_preview_cells: usize,
    /// The fewest rows a folded value may keep: a head row, three fold rows, and a tail.
    pub min_output_rows: usize,
    /// Columns kept between a syntax tree's deepest row and its source ranges.
    pub tree_location_gap: usize,
    /// Blank rows inside the band of printed output, above and below its text.
    pub printed_margin: u16,
}

/// Presentation configuration used by every UI element.
pub const UI: UiConfig = UiConfig {
    screen_margin: 1,
    feed_margin: 1,
    glyphs: Glyphs {
        prompt: Glyph {
            preferred: "❯",
            basic: ">",
        },
        split_divider: Glyph {
            preferred: "│",
            basic: "|",
        },
        cell_separator: Glyph {
            preferred: "│",
            basic: "|",
        },
        pending_separator: Glyph {
            preferred: "┈",
            basic: "-",
        },
        fold_rule: Glyph {
            preferred: "─",
            basic: "-",
        },
        input_edge: Glyph {
            preferred: "▀",
            basic: "-",
        },
        unevaluated_cell: Glyph {
            preferred: "…",
            basic: ".",
        },
        rejected_cell: Glyph {
            preferred: "×",
            basic: "x",
        },
    },
    spinner: SpinnerConfig {
        frames: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
        basic_frames: &["|", "/", "-", "\\"],
        frame_ms: 80,
    },
    prompt_margin: 2,
    cell_margin: 0,
    cell_gap: 3,
    min_number_digits: 3,
    prompt_text_gap: 2,
    right_margin: 3,
    min_box_width: 8,
    min_input_height: 3,
    picker_tab_width: 4,
    picker_column_gap: 2,
    picker_right_margin: 2,
    statusline_margin: 2,
    max_output_rows: 40,
    max_output_preview_bytes: 256 * 1024,
    max_output_preview_cells: 256 * 1024,
    min_output_rows: 5,
    tree_location_gap: 2,
    printed_margin: 1,
};

impl UiConfig {
    /// The first picker tab stop leaving at least a column gap after `width` cells.
    pub const fn picker_tab_stop(&self, width: usize) -> usize {
        (width + self.picker_column_gap).next_multiple_of(self.picker_tab_width)
    }

    pub const fn prompt_width(&self) -> u16 {
        self.prompt_margin + 1 + self.prompt_text_gap
    }
}

#[cfg(test)]
mod tests {
    use super::UI;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn basic_glyphs_and_spinner_frames_fit_one_cell() {
        let glyphs = UI.glyphs;
        for glyph in [
            glyphs.prompt,
            glyphs.split_divider,
            glyphs.cell_separator,
            glyphs.pending_separator,
            glyphs.fold_rule,
            glyphs.input_edge,
            glyphs.unevaluated_cell,
            glyphs.rejected_cell,
        ] {
            assert_eq!(UnicodeWidthStr::width(glyph.for_basic(true)), 1);
            assert_eq!(UnicodeWidthStr::width_cjk(glyph.for_basic(true)), 1);
        }
        assert!(!UI.spinner.frames.is_empty());
        assert!(!UI.spinner.basic_frames.is_empty());
        const { assert!(UI.spinner.frame_ms > 0) };
        assert!(UI.spinner.basic_frames.iter().all(|frame| {
            UnicodeWidthStr::width(*frame) == 1 && UnicodeWidthStr::width_cjk(*frame) == 1
        }));
    }

    #[test]
    fn prompt_and_divider_keep_their_preferred_symbols() {
        assert_eq!(UI.glyphs.prompt.for_basic(false), "❯");
        assert_eq!(UI.glyphs.split_divider.for_basic(false), "│");
        assert_eq!(UI.glyphs.prompt.for_basic(true), ">");
        assert_eq!(UI.glyphs.split_divider.for_basic(true), "|");
    }
}
