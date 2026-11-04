//! The *layout* tab: how much room the input and returned values take, and how
//! completion offers itself in the input.

use serde::{Deserialize, Serialize};
use these_macros_should_be_illegal::discriminated_str;

use super::tabs::SettingRow;
use super::Settings;
use crate::ui::config::UI;
use crate::ui::controller::picker::PickerValue;
use crate::ui::cycle::cyclic;

const MAX_INPUT_HEIGHT_PERCENT: u8 = 100;
const INPUT_HEIGHT_STEP_PERCENT: i8 = 5;
const MIN_COMPLETE_AFTER: u8 = 1;

/// The values of the *layout* tab.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutSettings {
    /// Maximum editor-height percentage occupied by the input.
    #[serde(alias = "max_height_percent")]
    pub max_input_height_percent: u8,
    /// Rows a returned value shows before its middle folds away.
    pub output_rows: usize,
    /// Context rows retained around the cursor.
    pub scrolloff: u8,
    /// Characters of a name typed before it is completed.
    pub complete_after: u8,
    /// Show an inline completion hint only below this many candidates.
    #[serde(rename = "hintCompletionBelow", alias = "hint_completion_below")]
    pub hint_completion_below: u16,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            max_input_height_percent: 75,
            output_rows: UI.max_output_rows,
            scrolloff: 2,
            complete_after: 1,
            hint_completion_below: 10,
        }
    }
}

cyclic! {
    /// One row of the *layout* tab.
    #[discriminated_str(label)]
    #[derive(Clone, Copy, PartialEq)]
    pub enum LayoutSetting {
        MaxInputHeight = "max input height",
        OutputRows = "max output height",
        Scrolloff = "scrolloff",
        CompleteAfter = "complete after",
        HintCompletionBelow = "hint below",
    }
}

impl SettingRow for LayoutSetting {
    fn label(self) -> &'static str {
        LayoutSetting::label(&self)
    }

    fn description(self, _: Settings) -> &'static str {
        match self {
            Self::MaxInputHeight => "share of the pane multiline input may grow into",
            Self::OutputRows => "rows a value shows before its middle folds away",
            Self::Scrolloff => "context rows kept around the input cursor",
            Self::CompleteAfter => "characters of a name typed before it is completed",
            Self::HintCompletionBelow => {
                "show an inline hint below this many candidates; zero disables hints"
            }
        }
    }

    fn value(self, settings: Settings) -> PickerValue {
        let layout = settings.layout;
        PickerValue::Stepped(match self {
            Self::MaxInputHeight => format!("{}%", layout.max_input_height_percent),
            Self::OutputRows => format!("{} rows", layout.output_rows),
            Self::Scrolloff => format!("{} rows", layout.scrolloff),
            Self::CompleteAfter => format!("{} characters", layout.complete_after),
            Self::HintCompletionBelow => format!("{} candidates", layout.hint_completion_below),
        })
    }

    fn adjust(self, settings: &mut Settings, delta: i8) {
        let layout = &mut settings.layout;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_hint_threshold_is_a_persisted_u16_with_compatible_defaults() {
        let old: LayoutSettings = serde_json::from_str(r#"{"complete_after":2}"#).unwrap();
        assert_eq!(
            old.hint_completion_below,
            LayoutSettings::default().hint_completion_below
        );
        let layout: LayoutSettings =
            serde_json::from_str(r#"{"hintCompletionBelow":65535}"#).unwrap();
        let mut settings = Settings {
            layout,
            ..Settings::default()
        };
        LayoutSetting::HintCompletionBelow.adjust(&mut settings, 1);
        assert_eq!(settings.layout.hint_completion_below, u16::MAX);
        let json = serde_json::to_value(settings.layout).unwrap();
        assert_eq!(json["hintCompletionBelow"], u16::MAX);
        settings.layout.hint_completion_below = 0;
        LayoutSetting::HintCompletionBelow.adjust(&mut settings, -1);
        assert_eq!(settings.layout.hint_completion_below, 0);
    }

    #[test]
    fn stepped_values_stay_within_their_ranges() {
        let mut settings = Settings::default();
        settings.layout.max_input_height_percent = MAX_INPUT_HEIGHT_PERCENT;
        LayoutSetting::MaxInputHeight.adjust(&mut settings, 1);
        assert_eq!(
            settings.layout.max_input_height_percent,
            MAX_INPUT_HEIGHT_PERCENT
        );
        settings.layout.output_rows = UI.min_output_rows;
        LayoutSetting::OutputRows.adjust(&mut settings, -1);
        assert_eq!(settings.layout.output_rows, UI.min_output_rows);
        settings.layout.complete_after = MIN_COMPLETE_AFTER;
        LayoutSetting::CompleteAfter.adjust(&mut settings, -1);
        assert_eq!(settings.layout.complete_after, MIN_COMPLETE_AFTER);
    }
}
