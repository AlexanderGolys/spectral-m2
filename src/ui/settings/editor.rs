//! The *editor* tab: how much room the input and returned values take, how
//! completion offers itself in the input, and what a selection copies.

use crate::ui::cycle::cyclic;
use serde::{Deserialize, Serialize};

use super::rendering::switch;
use super::row::{SettingRow, SettingsTab};
use super::selection::{SelectionSetting, SelectionSettings};
use super::Settings;
use crate::ui::config::UI;
use crate::ui::controller::picker::{PickerItem, PickerValue};
use crate::ui::wrap::WrapMode;

// @@@settings.layout
// @##issue.magic
const MAX_INPUT_HEIGHT_PERCENT: u8 = 100;
const INPUT_HEIGHT_STEP_PERCENT: i8 = 5;
const MIN_COMPLETE_AFTER: u8 = 1;
const MIN_SCROLL_ROWS: u8 = 1;

cyclic! {
    /// Candidate order; for example, alphabetical order ignores recorded usage.
    #[discriminated_str(label)]
    #[non_exhaustive]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum CompletionSort {
        #[default]
        Usage = "usage",
        Alphabetical = "alphabetical",
        Length = "length",
    }
}

/// The values of the *editor* tab.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
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
    /// Offer name and path completions while typing.
    pub completion: bool,
    /// Ordering of symbol candidates.
    pub completion_sort: CompletionSort,
    /// Open a candidate list when completing with Tab.
    pub completion_list: bool,
    /// Maximum number of candidates visible at once.
    pub completion_rows: u8,
    /// How the live input wraps across terminal rows.
    pub input_wrap: WrapMode,
    /// How submitted and previewed source code wraps.
    pub code_wrap: WrapMode,
    /// What a mouse selection reads and copies.
    pub selection: SelectionSettings,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            max_input_height_percent: 75,
            output_rows: UI::MAX_OUTPUT_ROWS,
            scrolloff: 2,
            scroll_rows: 3,
            complete_after: 1,
            hint_completion_below: 10,
            completion: true,
            completion_sort: CompletionSort::default(),
            completion_list: true,
            completion_rows: 8,
            input_wrap: WrapMode::default(),
            code_wrap: WrapMode::default(),
            selection: SelectionSettings::default(),
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
    InputWrap = "input wrapping",
    CodeWrap = "code wrapping",
    Scrolloff = "scrolloff",
    ScrollRows = "scroll speed",
    Completion = "completion",
    CompletionSort = "completion sorting",
    CompletionList = "completion list",
    CompletionRows = "completion rows",
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
            Self::InputWrap => "wrap live input at word boundaries or terminal columns",
            Self::CodeWrap => {
                "wrap submitted and previewed code at token boundaries or terminal columns"
            }
            Self::Scrolloff => "context rows kept around the input cursor and across a page scroll",
            Self::ScrollRows => "rows one notch of the mouse wheel scrolls",
            Self::Completion => "offer name and path completions while typing",
            Self::CompletionSort => "rank symbol candidates by usage, alphabetically, or by length",
            Self::CompletionList => "show candidates when Tab completes; off cycles inline",
            Self::CompletionRows => "maximum number of candidates visible in the list",
            Self::CompleteAfter => "characters of a name typed before it is completed",
            Self::HintCompletionBelow => {
                "show an inline hint below this many candidates; zero disables hints"
            }
        }
    }

    fn value(self, settings: Settings) -> PickerValue {
        let layout = settings.editor;
        match self {
            Self::InputWrap | Self::CodeWrap => {
                return PickerValue::among(
                    WrapMode::CHOICES,
                    &match self {
                        Self::InputWrap => layout.input_wrap,
                        _ => layout.code_wrap,
                    },
                    |mode| mode.label().into(),
                )
            }
            Self::Completion => return switch(layout.completion, "on"),
            Self::CompletionList => return switch(layout.completion_list, "on"),
            Self::CompletionSort => {
                return PickerValue::among(
                    CompletionSort::CHOICES,
                    &layout.completion_sort,
                    |sort| sort.label().into(),
                )
            }
            _ => {}
        }
        PickerValue::Stepped(match self {
            Self::MaxInputHeight => format!("{}%", layout.max_input_height_percent),
            Self::OutputRows => format!("{} rows", layout.output_rows),
            Self::Scrolloff => format!("{} rows", layout.scrolloff),
            Self::ScrollRows => format!("{} rows", layout.scroll_rows),
            Self::CompletionRows => format!("{} candidates", layout.completion_rows),
            Self::CompleteAfter => format!("{} characters", layout.complete_after),
            Self::HintCompletionBelow => format!("{} candidates", layout.hint_completion_below),
            _ => unreachable!("choices are displayed above"),
        })
    }

    fn adjust(self, settings: &mut Settings, delta: i8) {
        let layout = &mut settings.editor;
        match self {
            Self::InputWrap => layout.input_wrap = layout.input_wrap.cycle(delta),
            Self::CodeWrap => layout.code_wrap = layout.code_wrap.cycle(delta),
            Self::Completion => layout.completion = !layout.completion,
            Self::CompletionList => layout.completion_list = !layout.completion_list,
            Self::CompletionSort => layout.completion_sort = layout.completion_sort.cycle(delta),
            Self::CompletionRows => {
                layout.completion_rows = layout
                    .completion_rows
                    .saturating_add_signed(delta)
                    .max(MIN_COMPLETE_AFTER)
            }
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
                    .max(UI::MIN_OUTPUT_ROWS);
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

impl SettingsTab for EditorSettings {
    const LABEL: &'static str = "editor";

    fn rows(settings: Settings) -> Vec<Box<dyn PickerItem>> {
        EditorSetting::rows(EditorSetting::CHOICES, settings)
            .chain(SelectionSetting::rows(SelectionSetting::CHOICES, settings))
            .collect()
    }
}
