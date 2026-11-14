//! The *editor* tab: how much room the input and returned values take, and how
//! completion offers itself in the input.

use crate::ui::cycle::cyclic;
use serde::{Deserialize, Serialize};
use these_macros_should_be_illegal::discriminated_str;

use super::row::SettingRow;
use super::selection::SelectionSetting;
use super::Settings;
use crate::ui::config::UI;
use crate::ui::controller::picker::{PickerItem, PickerValue};

// @@@settings.layout
// @##issue.magic
const MAX_INPUT_HEIGHT_PERCENT: u8 = 100;
const INPUT_HEIGHT_STEP_PERCENT: i8 = 5;
const MIN_COMPLETE_AFTER: u8 = 1;
const MIN_SCROLL_ROWS: u8 = 1;

/// The values of the *editor* tab.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    /// Show automatic language-server diagnostics and their underlines.
    pub diagnostics: bool,
    /// Maximum editor-height percentage occupied by the input.
    #[serde(alias = "max_height_percent")]
    pub max_input_height_percent: u8,
    /// Rows a returned value shows before its middle folds away.
    pub output_rows: usize,
    /// Context rows retained around the cursor, and kept in sight by a page scroll.
    pub scrolloff: u8,
    /// Rows one notch of the mouse wheel scrolls.
    pub scroll_rows: u8,
    /// Characters of a name typed before it is completed.
    pub complete_after: u8,
    /// Show an inline completion hint only below this many candidates.
    #[serde(rename = "hintCompletionBelow", alias = "hint_completion_below")]
    pub hint_completion_below: u16,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            diagnostics: false,
            max_input_height_percent: 75,
            output_rows: UI.max_output_rows,
            scrolloff: 2,
            scroll_rows: 3,
            complete_after: 1,
            hint_completion_below: 10,
        }
    }
}

cyclic! {
/// One row of the *editor* tab.
#[discriminated_str(label)]
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq)]
pub enum EditorSetting {
    MaxInputHeight = "max input height",
    OutputRows = "max output height",
    Scrolloff = "scrolloff",
    ScrollRows = "scroll speed",
    CompleteAfter = "complete after",
    HintCompletionBelow = "hint below",
}

}

impl SettingRow for EditorSetting {
    fn label(self) -> &'static str {
        EditorSetting::label(&self)
    }

    fn description(self, _: Settings) -> &'static str {
        match self {
            Self::MaxInputHeight => "share of the pane multiline input may grow into",
            Self::OutputRows => "rows a value shows before its middle folds away",
            Self::Scrolloff => "context rows kept around the input cursor and across a page scroll",
            Self::ScrollRows => "rows one notch of the mouse wheel scrolls",
            Self::CompleteAfter => "characters of a name typed before it is completed",
            Self::HintCompletionBelow => {
                "show an inline hint below this many candidates; zero disables hints"
            }
        }
    }

    fn value(self, settings: Settings) -> PickerValue {
        let layout = settings.editor;
        PickerValue::Stepped(match self {
            Self::MaxInputHeight => format!("{}%", layout.max_input_height_percent),
            Self::OutputRows => format!("{} rows", layout.output_rows),
            Self::Scrolloff => format!("{} rows", layout.scrolloff),
            Self::ScrollRows => format!("{} rows", layout.scroll_rows),
            Self::CompleteAfter => format!("{} characters", layout.complete_after),
            Self::HintCompletionBelow => format!("{} candidates", layout.hint_completion_below),
        })
    }

    fn adjust(self, settings: &mut Settings, delta: i8) {
        let layout = &mut settings.editor;
        match self {
            Self::MaxInputHeight => {
                layout.max_input_height_percent = layout
                    .max_input_height_percent
                    .saturating_add_signed(delta.saturating_mul(INPUT_HEIGHT_STEP_PERCENT))
                    .min(MAX_INPUT_HEIGHT_PERCENT);
            }
            // Keeps room for a head, the three fold rows, and a tail.
            Self::OutputRows => {
                layout.output_rows = layout
                    .output_rows
                    .saturating_add_signed(delta.into())
                    .max(UI.min_output_rows);
            }
            Self::Scrolloff => layout.scrolloff = layout.scrolloff.saturating_add_signed(delta),
            Self::ScrollRows => {
                layout.scroll_rows = layout
                    .scroll_rows
                    .saturating_add_signed(delta)
                    .max(MIN_SCROLL_ROWS);
            }
            Self::CompleteAfter => {
                layout.complete_after = layout
                    .complete_after
                    .saturating_add_signed(delta)
                    .max(MIN_COMPLETE_AFTER);
            }
            Self::HintCompletionBelow => {
                layout.hint_completion_below = layout
                    .hint_completion_below
                    .saturating_add_signed(delta.into());
            }
        }
    }
}

/// Editor rows, including mouse selection and copying behavior.
pub fn rows(settings: Settings) -> Vec<Box<dyn PickerItem>> {
    EditorSetting::CHOICES
        .iter()
        .map(|row| row.row(settings))
        .chain(
            SelectionSetting::CHOICES
                .iter()
                .map(|row| row.row(settings)),
        )
        .collect()
}
