//! Compile-time presentation configuration for the complete interface.

// Keep mathematical notation and interface symbols legible in their native spelling.
#![allow(clippy::non_ascii_literal)]
use std::time::Duration;

use crate::spinner_frames;
use crate::ui::glyphs::Glyphs;
use crate::ui::render::spin::Spinner;

/// The pending-cell spinner, with its fallback for terminals without rich glyphs.
#[derive(Clone, Copy, Debug)]
pub struct SpinnerConfig {
    pub rich: Spinner,
    pub basic: Spinner,
}

impl SpinnerConfig {
    /// The spinner chosen for the terminal.
    pub fn active(self) -> Spinner {
        if Glyphs::basic() {
            self.basic
        } else {
            self.rich
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
    /// Number of cells in a settings slider track.
    pub slider_width: usize,
    /// Animation of a pending cell marker.
    pub spinner: SpinnerConfig,
    /// How long after the last keystroke that changed the input it keeps the
    /// typing prompt.
    pub typing_prompt_ms: u64,
    /// How long the status line says what a copy took.
    pub copy_notice_ms: u64,
    /// How long the input's cursor must rest before the language server is asked
    /// about where it stands, so a run of moves asks once, about where it stops.
    pub cursor_question_settle: Duration,
    /// The time between frames while anything moves: 60 a second. Whatever moves
    /// changes its own state at its own rate; a frame draws the state it finds.
    pub frame_interval: Duration,
    /// How long a run goes before the status line offers Ctrl+C to cancel it.
    pub cancel_hint_delay: Duration,
    /// How long after a run ends a cancel still counts as meant for it, and is
    /// ignored rather than clearing the input or quitting.
    pub cancel_grace: Duration,
    /// How often sp2 asks the installed sp2 and m2-ls for their versions.
    pub installed_check_interval: Duration,
    /// How often sp2 asks crates.io for the published sp2 and m2-ls versions.
    pub published_check_interval: Duration,
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
    pub math_size_percent: u16,
    pub min_math_size_percent: u16,
    pub max_math_size_percent: u16,
    pub math_size_step: u16,
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
    pub floating: FloatingConfig,
    pub resize: ResizeConfig,
}

/// How content follows a pane being resized; see [`crate::ui::settling`].
#[derive(Clone, Copy, Debug)]
pub struct ResizeConfig {
    /// How long a pane's width must hold before content is laid out again.
    pub settle: Duration,
    /// The longest content keeps an old layout while the pane keeps moving.
    pub max_stale: Duration,
}

/// Limits of a panel floating beside a point.
#[derive(Clone, Copy, Debug)]
pub struct FloatingConfig {
    /// Columns, border included.
    pub max_width: u16,
    /// Rows of content, border excluded.
    pub max_rows: u16,
}

/// Presentation configuration used by every UI element.
pub const UI: UiConfig = UiConfig {
    slider_width: 12,
    screen_margin: 1,
    feed_margin: 1,

    spinner: SpinnerConfig {
        rich: Spinner::new(
            spinner_frames![PendingCellMarker: "󰋙 ", "󰫃 ", "󰫄 ", "󰫅 ", "󰫆 ", "󰫇 ", "󰫈 ", "󰋘 ", " "],
            // spinner_frames![PendingCellMarker: "", "", "", "", "", ""],
            Duration::from_millis(80),
        ),
        basic: Spinner::new(
            spinner_frames![PendingCellMarker: "<", "^", ">", "v"],
            Duration::from_millis(80),
        ),
    },

    typing_prompt_ms: 200,
    copy_notice_ms: 2000,
    cursor_question_settle: Duration::from_millis(200),
    frame_interval: Duration::from_nanos(1_000_000_000 / 60),
    cancel_hint_delay: Duration::from_secs(3),
    cancel_grace: Duration::from_millis(500),
    installed_check_interval: Duration::from_secs(60),
    published_check_interval: Duration::from_secs(30 * 60),
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
    math_size_percent: 100,
    min_math_size_percent: 50,
    max_math_size_percent: 200,
    math_size_step: 10,
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
    floating: FloatingConfig {
        max_width: 72,
        max_rows: 8,
    },
    resize: ResizeConfig {
        settle: Duration::from_millis(120),
        max_stale: Duration::from_millis(350),
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
