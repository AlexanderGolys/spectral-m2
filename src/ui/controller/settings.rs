//! User-adjustable presentation settings.
//!
//! Values the user is expected to tune live here rather than as constants, so a
//! settings picker has one place to read and write and the defaults stay visible.

pub mod keymap;
pub mod theme;

/// Presentation choices shared by the editor and its renderers.
#[derive(Clone, Copy, Debug, Default)]
pub struct Settings {
    pub statusline: StatusLinePosition,
}

/// Placement of the editor status line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusLinePosition {
    #[default]
    BelowInput,
    AboveInput,
    Off,
}

impl StatusLinePosition {
    pub const fn cycle(self, delta: i8) -> Self {
        match (self, delta > 0) {
            (Self::BelowInput, true) | (Self::Off, false) => Self::AboveInput,
            (Self::AboveInput, true) => Self::Off,
            (Self::AboveInput, false) => Self::BelowInput,
            (Self::Off, true) => Self::BelowInput,
            (Self::BelowInput, false) => Self::Off,
        }
    }
}
