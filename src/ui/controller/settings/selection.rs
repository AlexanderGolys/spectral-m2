//! The *selection* tab: what a mouse selection reads and whether it copies itself.

use serde::{Deserialize, Serialize};
use these_macros_should_be_illegal::discriminated_str;

use super::rendering::switch;
use super::tabs::SettingRow;
use super::Settings;
use crate::ui::controller::picker::PickerValue;
use crate::ui::cycle::cyclic;

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
}

impl Default for SelectionSettings {
    fn default() -> Self {
        Self {
            copy_on_select: true,
            raw: false,
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
        }
    }

    fn value(self, settings: Settings) -> PickerValue {
        let selection = settings.selection;
        match self {
            Self::CopyOnSelect => switch(selection.copy_on_select, "on"),
            Self::Raw => switch(selection.raw, "on"),
        }
    }

    fn adjust(self, settings: &mut Settings, _: i8) {
        let selection = &mut settings.selection;
        match self {
            Self::CopyOnSelect => selection.copy_on_select = !selection.copy_on_select,
            Self::Raw => selection.raw = !selection.raw,
        }
    }
}
