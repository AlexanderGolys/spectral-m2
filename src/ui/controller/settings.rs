//! User-adjustable presentation settings.
//!
//! Values the user is expected to tune live here rather than as constants, so a
//! settings picker has one place to read and write and the defaults stay visible.

pub mod keymap;
pub mod theme;

/// Presentation choices shared by the editor and its renderers.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// The share of the editor's height the input may grow to before it scrolls
    /// internally instead of pushing the feed further off screen.
    pub max_input_height_percent: u8,
    /// Rows of context kept between the cursor and the edges of the input
    /// viewport, so the line being typed is never flush against either edge --
    /// the same idea as vim's `scrolloff`.
    pub input_scrolloff: u8,
    pub statusline: StatusLinePosition,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_input_height_percent: 75,
            input_scrolloff: 2,
            statusline: StatusLinePosition::default(),
        }
    }
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
    const fn cycle(self, delta: i8) -> Self {
        match (self, delta > 0) {
            (Self::BelowInput, true) | (Self::Off, false) => Self::AboveInput,
            (Self::AboveInput, true) => Self::Off,
            (Self::AboveInput, false) => Self::BelowInput,
            (Self::Off, true) => Self::BelowInput,
            (Self::BelowInput, false) => Self::Off,
        }
    }
}

/// One adjustable entry, so the picker can walk the settings without knowing
/// which fields exist.
#[derive(Clone, Copy, PartialEq)]
pub enum SettingField {
    MaxInputHeight,
    InputScrolloff,
    StatusLine,
}

impl SettingField {
    pub fn label(self) -> &'static str {
        match self {
            Self::MaxInputHeight => "max input height",
            Self::InputScrolloff => "scrolloff",
            Self::StatusLine => "statusline",
        }
    }

    pub fn value(self, settings: &Settings) -> String {
        match self {
            Self::MaxInputHeight => format!("{}%", settings.max_input_height_percent),
            Self::InputScrolloff => format!("{} rows", settings.input_scrolloff),
            Self::StatusLine => match settings.statusline {
                StatusLinePosition::BelowInput => "below input",
                StatusLinePosition::AboveInput => "above input",
                StatusLinePosition::Off => "off",
            }
            .to_string(),
        }
    }

    /// Nudge this setting, clamping to what the editor can actually honour.
    pub fn adjust(self, settings: &mut Settings, delta: i8) {
        if self == Self::StatusLine {
            settings.statusline = settings.statusline.cycle(delta);
            return;
        }
        let step = delta
            .unsigned_abs()
            .saturating_mul(if self == Self::MaxInputHeight { 5 } else { 1 });
        let field = match self {
            Self::MaxInputHeight => &mut settings.max_input_height_percent,
            Self::InputScrolloff => &mut settings.input_scrolloff,
            Self::StatusLine => unreachable!("statusline is adjusted above"),
        };
        *field = if delta > 0 {
            field.saturating_add(step)
        } else {
            field.saturating_sub(step)
        };
        settings.max_input_height_percent = settings.max_input_height_percent.min(100);
    }
}
