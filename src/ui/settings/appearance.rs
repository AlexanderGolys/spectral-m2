//! The *appearance* tab: a palette slider and cyclic scheme, mode and background choices.

use super::row::{SettingRow, SettingsTab};
use super::Settings;
use crate::ui::controller::picker::{PickerItem, PickerValue};
use crate::ui::cycle::{cycled, cyclic};
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
            Self::Contrast => "interpolate continuously between the theme's native variants",
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

impl SettingsTab for AppearanceSettings {
    const LABEL: &'static str = "appearance";

    fn rows(settings: Settings) -> Vec<Box<dyn PickerItem>> {
        AppearanceSetting::rows(AppearanceSetting::CHOICES, settings).collect()
    }
}
