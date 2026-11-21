//! The *side panel* tab: what the views beside the session show, and how.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::row::{SettingRow, SettingsTab};
use super::Settings;
use crate::ui::controller::picker::{PickerItem, PickerValue};
use crate::ui::cycle::cyclic;
use crate::ui::wrap::CodeOverflow;

cyclic! {
    /// What the output view beside the session shows of the feed.
    #[non_exhaustive]
    #[discriminated_str(label)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum OutputView {
        /// Everything in the feed except the code: printed text, values and messages.
        #[default]
        Everything = "everything",
        Values = "values",
        Printed = "printed",
    }
}

/// The values of the *side panel* tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelSettings {
    pub output_view: OutputView,
    /// Seconds each documentation example may take to be computed again, so
    /// its value is drawn from its structure; none computes nothing.
    pub example_seconds: u16,
    /// Whether code in the views wraps, as the editor's code wrapping does, or
    /// is cut at the right edge.
    pub code_overflow: CodeOverflow,
}

impl PanelSettings {
    /// How long each documentation example may take to be computed again; zero
    /// when they are not.
    pub fn example_timeout(&self) -> Duration {
        Duration::from_secs(self.example_seconds.into())
    }
}

impl SettingsTab for PanelSettings {
    const LABEL: &'static str = "side panel";

    fn rows(settings: Settings) -> Vec<Box<dyn PickerItem>> {
        PanelSetting::rows(PanelSetting::CHOICES, settings).collect()
    }
}

cyclic! {
    /// One row of the *side panel* tab.
    #[non_exhaustive]
    #[discriminated_str(label)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum PanelSetting {
        OutputView = "output view",
        CodeOverflow = "code overflow",
        ExampleTimeout = "compute examples",
    }
}

impl SettingRow for PanelSetting {
    fn label(self) -> &'static str {
        Self::label(&self)
    }

    fn description(self, _: Settings) -> &'static str {
        match self {
            Self::OutputView => {
                "what the output view shows: everything but the code, values, or printed text"
            }
            Self::CodeOverflow => {
                "wrap code wider than the panel as the editor's code wrapping does, or cut it at the right edge"
            }
            Self::ExampleTimeout => {
                "run documentation examples again, each for at most this long, to draw their values"
            }
        }
    }

    fn value(self, settings: Settings) -> PickerValue {
        let panel = settings.panel;
        match self {
            Self::OutputView => {
                PickerValue::among(OutputView::CHOICES, &panel.output_view, |view| {
                    view.label().into()
                })
            }
            Self::CodeOverflow => {
                PickerValue::among(CodeOverflow::CHOICES, &panel.code_overflow, |overflow| {
                    overflow.label().into()
                })
            }
            Self::ExampleTimeout => PickerValue::Stepped(match panel.example_seconds {
                0 => "off".to_string(),
                seconds => format!("{seconds}s"),
            }),
        }
    }

    fn adjust(self, settings: &mut Settings, delta: i8) {
        let panel = &mut settings.panel;
        match self {
            Self::OutputView => panel.output_view = panel.output_view.cycle(delta),
            Self::CodeOverflow => panel.code_overflow = panel.code_overflow.cycle(delta),
            Self::ExampleTimeout => {
                panel.example_seconds = panel.example_seconds.saturating_add_signed(delta.into())
            }
        }
    }
}
