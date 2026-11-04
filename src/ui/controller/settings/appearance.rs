//! The *appearance* tab: the axes along which [`Appearance`] steps.

use these_macros_should_be_illegal::discriminated_str;

use super::tabs::SettingRow;
use super::theme::Appearance;
use super::Settings;
use crate::ui::controller::picker::PickerValue;
use crate::ui::cycle::{cycled, cyclic};

cyclic! {
    /// One independently cyclic axis of a colour appearance.
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
    fn choices(self, appearance: Appearance) -> Vec<Appearance> {
        match self {
            Self::Theme => appearance.themes(),
            Self::Contrast => appearance.contrasts(),
            Self::Mode => appearance.modes(),
            Self::Background => appearance.backgrounds(),
        }
    }

    /// How `appearance` reads along this axis.
    fn shown(self, appearance: Appearance) -> String {
        match self {
            Self::Theme => appearance.scheme().label().to_string(),
            Self::Contrast => appearance.variant().label().to_string(),
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
            Self::Contrast => "the theme's variants for the current mode",
            Self::Mode => "dark or light palette",
            Self::Background => "paint empty canvas or let the terminal show through",
        }
    }

    fn value(self, settings: Settings) -> PickerValue {
        let appearance = settings.appearance;
        PickerValue::among(&self.choices(appearance), &appearance, |&appearance| {
            self.shown(appearance).into()
        })
    }

    fn adjust(self, settings: &mut Settings, delta: i8) {
        let appearance = settings.appearance;
        settings.appearance = cycled(&self.choices(appearance), appearance, delta);
    }
}
