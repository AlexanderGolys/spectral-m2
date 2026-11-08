//! Compile-time presentation configuration for the complete interface.

use crate::ui::glyphs::{Glyphs, GLYPHS};

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

/// How typeset maths is sized against the terminal's text.
pub struct TypesetConfig {
    /// A formula's em as a share of one cell's height, so its letters stand as tall
    /// as the text around them.
    pub em_per_cell_height: f32,
    /// How far down its row the text baseline runs, as a share of the cell's height.
    /// A one-row formula is set on it, so neighbouring formulas share a baseline
    /// with the terminal text beside them rather than each centring its own ink.
    pub baseline_per_cell_height: f32,
    /// The smallest share of its natural size a formula may shrink to fit a width;
    /// below it the formula is drawn as text instead.
    pub min_scale: f32,
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
    /// How long after the last keystroke that changed the input it keeps the
    /// typing prompt.
    pub typing_prompt_ms: u64,
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
    /// The fewest columns, step arrows included, a value keeps for rows to line
    /// their values up; with less room each value follows its own row's cells.
    pub picker_min_value_width: usize,
    pub statusline_margin: u16,
    pub max_output_rows: usize,
    /// Bytes of flat output admitted to synchronous preview formatting.
    pub max_output_preview_bytes: usize,
    /// Cells admitted to a value's expanded preview buffer.
    pub max_output_preview_cells: usize,
    /// The fewest rows a folded value may keep: a head row, three fold rows, and a tail.
    pub min_output_rows: usize,
    /// Distance between the rows of a typeset matrix, as a percentage of TeX's own,
    /// which leaves stacked fractions in neighbouring rows almost touching.
    pub matrix_row_spacing_percent: u16,
    pub min_matrix_row_spacing_percent: u16,
    pub max_matrix_row_spacing_percent: u16,
    /// Percentage points one picker step moves the matrix row spacing.
    pub matrix_row_spacing_step: u16,
    /// Columns kept between a syntax tree's deepest row and its source ranges.
    pub tree_location_gap: usize,
    /// Blank rows inside the band of printed output, above and below its text.
    pub printed_margin: u16,
    /// Columns kept between a value and the type set beside it.
    pub type_gap: usize,
    /// The largest share of an output's width its type may keep beside the value,
    /// gap included; a wider type goes below the value instead.
    pub max_type_share: f32,
    pub typeset: TypesetConfig,
}

/// Presentation configuration used by every UI element.
pub const UI: UiConfig = UiConfig {
    screen_margin: 1,
    feed_margin: 1,
    glyphs: GLYPHS,
    spinner: SpinnerConfig {
        frames: &["󰋙 ", "󰫃 ", "󰫄 ", "󰫅 ", "󰫆 ", "󰫇 ", "󰫈 ", "󰋘 ", " "],
        // frames: &["", "", "", "", "", ""],
        basic_frames: &["<", "^", ">", "v"],
        frame_ms: 80,
    },

    typing_prompt_ms: 200,
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
    picker_min_value_width: 14,

    statusline_margin: 2,

    max_output_rows: 40,
    max_output_preview_bytes: 256 * 1024,
    max_output_preview_cells: 256 * 1024,
    min_output_rows: 5,
    matrix_row_spacing_percent: 125,
    min_matrix_row_spacing_percent: 100,
    max_matrix_row_spacing_percent: 300,
    matrix_row_spacing_step: 5,

    tree_location_gap: 2,
    printed_margin: 1,
    type_gap: 2,
    max_type_share: 0.3,
    typeset: TypesetConfig {
        em_per_cell_height: 0.65,
        baseline_per_cell_height: 0.77,
        min_scale: 0.6,
    },
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
    fn basic_spinner_frames_fit_one_cell() {
        assert!(!UI.spinner.frames.is_empty());
        assert!(!UI.spinner.basic_frames.is_empty());
        assert!(UI.spinner.basic_frames.iter().all(|frame| {
            UnicodeWidthStr::width(*frame) == 1 && UnicodeWidthStr::width_cjk(*frame) == 1
        }));
    }
}
