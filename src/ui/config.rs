//! Compile-time presentation configuration for the complete interface.

/// Geometry values shared by UI elements.
pub(crate) struct UiConfig {
    pub(crate) feed_margin: u16,
    pub(crate) prompt_margin: u16,
    pub(crate) cell_margin: u16,
    pub(crate) cell_gap: u16,
    pub(crate) prompt_text_gap: u16,
    pub(crate) right_margin: u16,
    pub(crate) min_box_width: u16,
    pub(crate) min_input_height: u16,
    pub(crate) picker_min_rows: u16,
    pub(crate) picker_tab_width: usize,
    pub picker_column_gap: usize,
    pub(crate) picker_right_margin: usize,
    pub(crate) statusline_margin: u16,
    pub(crate) max_output_rows: usize,
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
pub(crate) const UI: UiConfig = UiConfig {
    feed_margin: 1,
    prompt_margin: 2,
    cell_margin: 1,
    cell_gap: 2,
    prompt_text_gap: 2,
    right_margin: 3,
    min_box_width: 8,
    min_input_height: 3,
    picker_min_rows: 5,
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

    pub(crate) const fn prompt_width(&self) -> u16 {
        self.prompt_margin + 1 + self.prompt_text_gap
    }
}
