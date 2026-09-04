//! Compile-time presentation configuration for the complete interface.

/// Geometry values shared by UI elements.
pub struct UiConfig {
    pub feed_margin: u16,
    pub prompt_margin: u16,
    pub cell_margin: u16,
    pub cell_gap: u16,
    pub prompt_text_gap: u16,
    pub right_margin: u16,
    pub min_box_width: u16,
    pub min_input_height: u16,
    pub picker_min_rows: u16,
    pub picker_tab_width: usize,
    pub picker_right_margin: usize,
    pub statusline_margin: u16,
}

/// Presentation configuration used by every UI element.
pub const UI: UiConfig = UiConfig {
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
    picker_right_margin: 2,
    statusline_margin: 2,
};

impl UiConfig {
    pub const fn prompt_width(&self) -> u16 {
        self.prompt_margin + 1 + self.prompt_text_gap
    }
}
