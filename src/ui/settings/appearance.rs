//! The *appearance* tab: a contrast slider, cyclic scheme, mode and background
//! choices, and the colours of returned maths.

use super::row::{SettingRow, SettingsTab};
use super::Settings;
use crate::ui::controller::picker::{PickerItem, PickerValue};
use crate::ui::cycle::{cycled, cyclic};
use crate::ui::render::maths::{MathInk, ScriptColor};
use crate::ui::theme::AppearanceSettings;

cyclic! {
    /// One independently adjustable axis of a colour appearance.
    #[discriminated_str(label)]
    #[derive(Clone, Copy, PartialEq)]
    pub enum AppearanceSetting {
        Theme = "theme",
        Contrast = "contrast",
        Mode = "mode",
        Background = "background",
    }
}

impl AppearanceSetting {
    /// Every appearance this axis steps `appearance` through.
    fn choices(self, appearance: AppearanceSettings) -> Vec<AppearanceSettings> {
        match self {
            Self::Theme => appearance.themes(),
            Self::Contrast => Vec::new(),
            Self::Mode => appearance.modes(),
            Self::Background => appearance.backgrounds(),
        }
    }

    /// How `appearance` reads along this axis.
    fn shown(self, appearance: AppearanceSettings) -> String {
        match self {
            Self::Theme => appearance.scheme().label().to_string(),
            Self::Contrast => format!("{}%", appearance.contrast),
            Self::Mode => appearance.mode().to_string(),
            Self::Background => appearance.background_label().to_string(),
        }
    }
}

impl SettingRow for AppearanceSetting {
    fn label(self) -> &'static str {
        AppearanceSetting::label(&self)
    }

    fn description(self, _: Settings) -> &'static str {
        match self {
            Self::Theme => "colour scheme family",
            Self::Contrast => {
                "blend from the mode's lowest contrast variant at 0% to its highest at 100%"
            }
            Self::Mode => "dark or light palette",
            Self::Background => "paint empty canvas or let the terminal show through",
        }
    }

    fn value(self, settings: Settings) -> PickerValue {
        let appearance = settings.appearance;
        if self == Self::Contrast {
            return PickerValue::Slider(appearance.contrast);
        }
        PickerValue::among(&self.choices(appearance), &appearance, |&appearance| {
            self.shown(appearance).into()
        })
    }

    fn adjust(self, settings: &mut Settings, delta: i8) {
        let appearance = settings.appearance;
        settings.appearance = if self == Self::Contrast {
            appearance.with_contrast(delta)
        } else {
            cycled(&self.choices(appearance), appearance, delta)
        };
    }
}

cyclic! {
    /// One row of the *appearance* tab colouring returned maths, its value kept
    /// among the [`RenderingSettings`](super::rendering::RenderingSettings).
    #[discriminated_str(label)]
    #[derive(Clone, Copy, PartialEq)]
    pub enum MathColorSetting {
        Ink = "maths colours",
        Index = "index colours",
        Exponent = "exponent colours",
    }
}

impl SettingRow for MathColorSetting {
    fn label(self) -> &'static str {
        MathColorSetting::label(&self)
    }

    fn description(self, _: Settings) -> &'static str {
        match self {
            Self::Ink => "colour returned maths by role, or in the value colour with types in the type colour",
            Self::Index => "colour of subscripts: their own, their symbol's, or the operator colour; delimiters and punctuation keep theirs",
            Self::Exponent => "colour of exponents: their own, their symbol's, or the operator colour; delimiters and punctuation keep theirs",
        }
    }

    fn value(self, settings: Settings) -> PickerValue {
        let rendering = settings.rendering;
        let script = |color: ScriptColor| {
            PickerValue::among(ScriptColor::CHOICES, &color, |color| color.label().into())
        };
        match self {
            Self::Ink => PickerValue::among(MathInk::CHOICES, &rendering.math_ink, |ink| {
                ink.label().into()
            }),
            Self::Index => script(rendering.index_color),
            Self::Exponent => script(rendering.exponent_color),
        }
    }

    fn adjust(self, settings: &mut Settings, delta: i8) {
        let rendering = &mut settings.rendering;
        match self {
            Self::Ink => rendering.math_ink = rendering.math_ink.cycle(delta),
            Self::Index => rendering.index_color = rendering.index_color.cycle(delta),
            Self::Exponent => rendering.exponent_color = rendering.exponent_color.cycle(delta),
        }
    }
}

impl SettingsTab for AppearanceSettings {
    const LABEL: &'static str = "appearance";

    fn rows(settings: Settings) -> Vec<Box<dyn PickerItem>> {
        AppearanceSetting::rows(AppearanceSetting::CHOICES, settings)
            .chain(MathColorSetting::rows(MathColorSetting::CHOICES, settings))
            .collect()
    }
}
