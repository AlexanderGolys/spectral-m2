//! User-adjustable presentation settings.
//!
//! Values the user is expected to tune live here rather than as constants, so a
//! settings picker has one place to read and write and the defaults stay visible.

pub(crate) mod keymap;
pub(crate) mod theme;

use crate::ui::config::UI;
use crate::ui::render::maths::FractionStyle;

/// Presentation choices shared by the editor and its renderers.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Settings {
    pub(crate) statusline: StatusLinePosition,
    pub(crate) fractions: FractionStyle,
    /// Rows a returned value shows before its middle folds away.
    pub(crate) output_rows: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            statusline: StatusLinePosition::default(),
            fractions: FractionStyle::default(),
            output_rows: UI.max_output_rows,
        }
    }
}

impl Settings {
    /// Rows one picker step moves the output fold threshold.
    const OUTPUT_ROWS_STEP: usize = 4;

    /// Move the output fold threshold by picker steps, keeping room for a head, the
    /// fold row, and a tail.
    pub(crate) fn adjust_output_rows(&mut self, delta: i8) {
        let step = Self::OUTPUT_ROWS_STEP * usize::from(delta.unsigned_abs());
        self.output_rows = match delta > 0 {
            true => self.output_rows.saturating_add(step),
            false => self.output_rows.saturating_sub(step),
        }
        .max(Self::OUTPUT_ROWS_STEP);
    }

    /// Whether returned values lay out differently, moving their heights.
    pub(crate) fn reformats(&self, other: &Self) -> bool {
        (self.fractions, self.output_rows) != (other.fractions, other.output_rows)
    }
}

/// Placement of the editor status line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum StatusLinePosition {
    #[default]
    BelowInput,
    AboveInput,
    Off,
}

impl StatusLinePosition {
    pub(crate) const fn cycle(self, delta: i8) -> Self {
        match (self, delta > 0) {
            (Self::BelowInput, true) | (Self::Off, false) => Self::AboveInput,
            (Self::AboveInput, true) => Self::Off,
            (Self::AboveInput, false) => Self::BelowInput,
            (Self::Off, true) => Self::BelowInput,
            (Self::BelowInput, false) => Self::Off,
        }
    }
}
