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
    pub(crate) picker_right_margin: usize,
    pub(crate) statusline_margin: u16,
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
    picker_right_margin: 2,
    statusline_margin: 2,
};

impl UiConfig {
    pub(crate) const fn prompt_width(&self) -> u16 {
        self.prompt_margin + 1 + self.prompt_text_gap
    }
}
