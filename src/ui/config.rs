//! Compile-time presentation configuration for the complete interface.

/// Geometry values shared by UI elements.
pub struct UiConfig {
    /// Columns kept clear down the whole screen's left edge, around every pane.
    pub screen_margin: u16,
    pub feed_margin: u16,
    /// The frames the marker of a cell still evaluating cycles through.
    pub busy_frames: &'static [&'static str],
    /// Milliseconds each of those frames is shown.
    pub busy_frame_ms: u64,
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
    /// The fewest rows a folded value may keep: a head row, the fold row, and a tail.
    pub min_output_rows: usize,
    /// The glyph drawn down the column between a split's panes.
    pub split_divider: &'static str,
    /// Columns kept between a syntax tree's deepest row and its source ranges.
    pub tree_location_gap: usize,
    /// Blank rows inside the band of printed output, above and below its text.
    pub printed_margin: u16,
}

/// Presentation configuration used by every UI element.
pub const UI: UiConfig = UiConfig {
    screen_margin: 1,
    feed_margin: 1,
    busy_frames: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
    busy_frame_ms: 80,
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
    min_output_rows: 3,
    split_divider: "\u{2502}",
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
