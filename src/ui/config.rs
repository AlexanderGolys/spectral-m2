//! Compile-time presentation constants of the whole interface, gathered under
//! [`UI`] and, for the parts with several of their own, a namespace each.

// Keep interface symbols legible in their native spelling.
#![allow(clippy::non_ascii_literal)]
use std::time::Duration;

use crate::spinner_frames;
use crate::ui::glyphs::Spellings;
use crate::ui::render::spin::Spinner;

/// Geometry, timing and limits shared by UI elements.
pub struct UI;

impl UI {
    /// Columns kept clear down the whole screen's left edge, around every pane.
    pub const SCREEN_MARGIN: u16 = 1;
    pub const FEED_MARGIN: u16 = 1;
    /// Number of cells in a settings slider track.
    pub const SLIDER_WIDTH: usize = 12;

    /// The marker of a cell still evaluating. A Nerd Font icon is drawn wider
    /// than its one cell, so its frames carry a space to spill into.
    pub const PENDING_CELL_SPINNER: Spellings<Spinner> = Spellings::new(
        Spinner::new(
            spinner_frames![PendingCellMarker: "◐", "◓", "◑", "◒"],
            Duration::from_millis(80),
        ),
        Spinner::new(
            spinner_frames![PendingCellMarker: "<", "^", ">", "v"],
            Duration::from_millis(80),
        ),
    )
    .nerd(Spinner::new(
        spinner_frames![PendingCellMarker: "󰋙 ", "󰫃 ", "󰫄 ", "󰫅 ", "󰫆 ", "󰫇 ", "󰫈 ", "󰋘 ", " "],
        Duration::from_millis(80),
    ));

    /// How long after the last keystroke that changed the input it keeps the
    /// typing prompt.
    pub const TYPING_PROMPT: Duration = Duration::from_millis(200);
    /// How long the status line says what a copy took.
    pub const COPY_NOTICE: Duration = Duration::from_secs(2);
    /// How long the input's cursor must rest before the language server is asked
    /// about where it stands, so a run of moves asks once, about where it stops.
    pub const CURSOR_QUESTION_SETTLE: Duration = Duration::from_millis(200);
    /// The time between frames while anything moves: 60 a second. Whatever moves
    /// changes its own state at its own rate; a frame draws the state it finds.
    pub const FRAME_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 60);
    /// How long a run goes before the status line offers Ctrl+C to cancel it.
    pub const CANCEL_HINT_DELAY: Duration = Duration::from_secs(3);
    /// How long after a run ends a cancel still counts as meant for it, and is
    /// ignored rather than clearing the input or quitting.
    pub const CANCEL_GRACE: Duration = Duration::from_millis(500);
    /// How often sp2 asks the installed sp2 and m2-ls for their versions.
    pub const INSTALLED_CHECK_INTERVAL: Duration = Duration::from_secs(60);
    /// How often sp2 asks crates.io for the published sp2 and m2-ls versions.
    pub const PUBLISHED_CHECK_INTERVAL: Duration = Duration::from_secs(30 * 60);

    pub const PROMPT_MARGIN: u16 = 2;
    pub const PROMPT_TEXT_GAP: u16 = 2;
    /// The prompt's margin, its one-cell glyph and the gap after it.
    pub const PROMPT_WIDTH: u16 = Self::PROMPT_MARGIN + 1 + Self::PROMPT_TEXT_GAP;
    pub const CELL_MARGIN: u16 = 0;
    pub const CELL_GAP: u16 = 3;
    /// Digits every cell number is laid out for, however few it has, so a number
    /// growing a digit never moves a block.
    pub const MIN_NUMBER_DIGITS: u16 = 3;
    pub const RIGHT_MARGIN: u16 = 3;
    pub const MIN_BOX_WIDTH: u16 = 8;
    pub const MIN_INPUT_HEIGHT: u16 = 3;

    pub const PICKER_TAB_WIDTH: usize = 4;
    pub const PICKER_COLUMN_GAP: usize = 2;
    pub const PICKER_RIGHT_MARGIN: usize = 2;
    /// The fewest columns, step arrows included, a value keeps for rows to line
    /// their values up; with less room each value follows its own row's cells.
    pub const PICKER_MIN_VALUE_WIDTH: usize = 14;

    pub const STATUSLINE_MARGIN: u16 = 2;
    /// Screens of the feed above the viewport laid out before they are scrolled to,
    /// so that scrolling up reaches only blocks the workers have laid out.
    pub const LAID_OUT_AHEAD_SCREENS: usize = 4;

    pub const MAX_OUTPUT_ROWS: usize = 40;
    /// Bytes of flat output admitted to synchronous preview formatting.
    pub const MAX_OUTPUT_PREVIEW_BYTES: usize = 256 * 1024;
    /// Cells admitted to a value's expanded preview buffer.
    pub const MAX_OUTPUT_PREVIEW_CELLS: usize = 256 * 1024;
    /// The fewest rows a folded value may keep: a head row, three fold rows, and a tail.
    pub const MIN_OUTPUT_ROWS: usize = 5;

    /// Visits the history of the view beside the session remembers.
    pub const TAB_HISTORY: usize = 256;
    /// Pages kept for the visits nearest in that history; one further off is
    /// fetched again when it is visited.
    pub const TAB_PAGES_KEPT: usize = 16;

    pub const MATH_SIZE_PERCENT: u16 = 100;
    pub const MIN_MATH_SIZE_PERCENT: u16 = 50;
    pub const MAX_MATH_SIZE_PERCENT: u16 = 200;
    pub const MATH_SIZE_STEP: u16 = 10;
    /// Distance between the rows of a typeset matrix, as a percentage of TeX's own,
    /// which leaves stacked fractions in neighbouring rows almost touching.
    pub const MATRIX_ROW_SPACING_PERCENT: u16 = 125;
    pub const MIN_MATRIX_ROW_SPACING_PERCENT: u16 = 100;
    pub const MAX_MATRIX_ROW_SPACING_PERCENT: u16 = 300;
    /// Percentage points one picker step moves the matrix row spacing.
    pub const MATRIX_ROW_SPACING_STEP: u16 = 5;
    /// The length of a typeset arrow, as a percentage of its shortest: at this one
    /// it is as short as its label lets it be.
    pub const ARROW_LENGTH_PERCENT: u16 = 100;
    pub const MIN_ARROW_LENGTH_PERCENT: u16 = 50;
    pub const MAX_ARROW_LENGTH_PERCENT: u16 = 400;
    /// Percentage points one picker step moves an arrow length.
    pub const ARROW_LENGTH_STEP: u16 = 10;

    /// Columns kept between a syntax tree's deepest row and its source ranges.
    pub const TREE_LOCATION_GAP: usize = 2;
    /// Blank rows inside the band of printed output, above and below its text.
    pub const PRINTED_MARGIN: u16 = 1;
    /// Columns kept between a value and the type set beside it.
    pub const TYPE_GAP: usize = 2;
    /// The largest share of an output's width its type may keep beside the value,
    /// gap included; a wider type goes below the value instead.
    pub const MAX_TYPE_SHARE: f32 = 0.3;

    /// The first picker tab stop leaving at least a column gap after `width` cells.
    pub const fn picker_tab_stop(width: usize) -> usize {
        (width + Self::PICKER_COLUMN_GAP).next_multiple_of(Self::PICKER_TAB_WIDTH)
    }
}

/// How typeset maths is sized against the terminal's text.
pub struct TypesetSizing;

impl TypesetSizing {
    /// A formula's em as a share of one cell's height, so its letters stand as tall
    /// as the text around them.
    pub const EM_PER_CELL_HEIGHT: f32 = 0.65;
    /// How far down its row the text baseline runs, as a share of the cell's height.
    /// A one-row formula is set on it, so neighbouring formulas share a baseline
    /// with the terminal text beside them rather than each centring its own ink.
    pub const BASELINE_PER_CELL_HEIGHT: f32 = 0.77;
}

/// The size limits of a panel floating beside a point, such as hover or
/// signature help.
pub struct FloatingPanelLimits;

impl FloatingPanelLimits {
    /// Columns, border included.
    pub const MAX_WIDTH: u16 = 72;
    /// Rows of content, border excluded.
    pub const MAX_ROWS: u16 = 8;
}

/// When content is laid out again for a pane being resized; see
/// [`crate::ui::settling`].
pub struct ResizeRelayout;

impl ResizeRelayout {
    /// How long a pane's width must hold before content is laid out again.
    pub const SETTLE: Duration = Duration::from_millis(120);
    /// The longest content keeps an old layout while the pane keeps moving.
    pub const MAX_STALE: Duration = Duration::from_millis(350);
}
