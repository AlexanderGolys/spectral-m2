//! The *selection* tab: what a mouse selection reads and whether it copies itself.

use serde::{Deserialize, Serialize};
use these_macros_should_be_illegal::discriminated_str;

use super::rendering::switch;
use super::tabs::SettingRow;
use super::Settings;
use crate::ui::controller::picker::PickerValue;
use crate::ui::cycle::cyclic;

cyclic! {
    /// How a whole cell or output copies: as its own text, or as the session shows it.
    #[discriminated_str(label)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    #[non_exhaustive]
    pub enum CopyFormat {
        /// Only the text: code as typed, a value as printed.
        #[default]
        Raw = "raw text",
        /// With the prompts Macaulay2 gives it: `i1 : code`, `o1 = value`, `o1 : Type`.
        Session = "session",
    }
}

/// The values of the *selection* tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SelectionSettings {
    /// Whether finishing a mouse selection copies it, rather than waiting for the
    /// copy key.
    pub copy_on_select: bool,
    /// Whether a mouse selection reads every character as drawn, gutters, frames
    /// and rows on screen included, rather than only the text of what it starts on.
    pub raw: bool,
    /// What a whole cell or output copies as.
    pub copy_format: CopyFormat,
}

impl Default for SelectionSettings {
    fn default() -> Self {
        Self {
            copy_on_select: true,
            raw: false,
            copy_format: CopyFormat::default(),
        }
    }
}

cyclic! {
    /// One row of the *selection* tab.
    #[discriminated_str(label)]
    #[derive(Clone, Copy, PartialEq)]
    pub enum SelectionSetting {
        CopyOnSelect = "copy on select",
        Raw = "raw selection",
        CopyFormat = "copy cells as",
    }
}

impl SettingRow for SelectionSetting {
    fn label(self) -> &'static str {
        SelectionSetting::label(&self)
    }

    fn description(self, _: Settings) -> &'static str {
        match self {
            Self::CopyOnSelect => {
                "copy a mouse selection when it is made; off leaves it to the copy key"
            }
            Self::Raw => {
                "select any character as drawn, numbers, frames and rows as on screen included"
            }
            Self::CopyFormat => {
                "copy a whole cell or output as its text alone, or with its i1 : and o1 = prompts"
            }
        }
    }

    fn value(self, settings: Settings) -> PickerValue {
        let selection = settings.selection;
        match self {
            Self::CopyOnSelect => switch(selection.copy_on_select, "on"),
            Self::Raw => switch(selection.raw, "on"),
            Self::CopyFormat => {
                PickerValue::among(CopyFormat::CHOICES, &selection.copy_format, |&format| {
                    format.label().into()
                })
            }
        }
    }

    fn adjust(self, settings: &mut Settings, delta: i8) {
        let selection = &mut settings.selection;
        match self {
            Self::CopyOnSelect => selection.copy_on_select = !selection.copy_on_select,
            Self::Raw => selection.raw = !selection.raw,
            Self::CopyFormat => selection.copy_format = selection.copy_format.cycle(delta),
        }
    }
}
