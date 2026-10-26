//! The settings picker: its tabs, the settings each lists, and the button
//! restoring their defaults.

use ratatui::text::Span;
use these_macros_should_be_illegal::discriminated_str;

use super::picker::{PickerItem, PickerOption, PickerTab, PickerValue, Row};
use super::settings::theme::groups::scheme;
use super::settings::theme::Appearance;
use super::settings::{Preferences, StatusLinePosition};
use super::EditorEvent;
use crate::input::InputSetting;
use crate::ui::cycle::cycled;
use crate::ui::graphics;
use crate::ui::render::maths::{FractionStyle, MathDisplay};

/// A page of the settings picker and the settings it lists.
struct SettingsTab {
    label: &'static str,
    settings: &'static [Setting],
}

/// The settings picker's tabs, in the order shown.
const TABS: &[SettingsTab] = &[
    SettingsTab {
        label: "appearance",
        settings: &[
            Setting::Appearance(AppearanceField::Theme),
            Setting::Appearance(AppearanceField::Contrast),
            Setting::Appearance(AppearanceField::Mode),
            Setting::Appearance(AppearanceField::Background),
        ],
    },
    SettingsTab {
        label: "layout",
        settings: &[
            Setting::Statusline,
            Setting::Input(InputSetting::MaxHeight),
            Setting::OutputRows,
            Setting::Input(InputSetting::Scrolloff),
            Setting::Input(InputSetting::CompleteAfter),
        ],
    },
    SettingsTab {
        label: "rendering",
        settings: &[
            Setting::SemanticHighlighting,
            Setting::Maths,
            Setting::Fractions,
            Setting::MathFonts,
            Setting::ExampleTimeout,
        ],
    },
];

/// Every tab, its rows showing `preferences`.
pub fn tabs(preferences: Preferences) -> Vec<PickerTab> {
    TABS.iter()
        .map(|tab| PickerTab {
            label: tab.label,
            items: tab
                .settings
                .iter()
                .map(|setting| setting.row(preferences))
                .collect(),
        })
        .collect()
}

/// The button resetting settings, keymaps and appearance.
pub fn restore_defaults() -> Box<dyn PickerItem> {
    let label = Span::styled("restore defaults", scheme().settings_button());
    Row::enter(vec![label.into()], || EditorEvent::RestoreDefaults)
        .described("reset settings, keymaps and appearance")
        .into()
}

/// One independently cyclic axis of a colour appearance.
#[discriminated_str(label)]
#[derive(Clone, Copy)]
enum AppearanceField {
    Theme = "theme",
    Contrast = "contrast",
    Mode = "mode",
    Background = "background",
}

impl AppearanceField {
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

    const fn description(self) -> &'static str {
        match self {
            Self::Theme => "colour scheme family",
            Self::Contrast => "the theme's variants for the current mode",
            Self::Mode => "dark or light palette",
            Self::Background => "paint empty canvas or let the terminal show through",
        }
    }
}

/// One setting listed in the settings picker.
#[derive(Clone, Copy)]
enum Setting {
    Appearance(AppearanceField),
    Input(InputSetting),
    Statusline,
    SemanticHighlighting,
    Maths,
    Fractions,
    MathFonts,
    OutputRows,
    ExampleTimeout,
}

impl Setting {
    /// The setting's name, leading its row.
    const fn label(self) -> &'static str {
        match self {
            Self::Appearance(field) => field.label(),
            Self::Input(field) => field.label(),
            Self::Statusline => "statusline",
            Self::SemanticHighlighting => "semantic highlighting",
            Self::Maths => "maths",
            Self::Fractions => "fractions",
            Self::MathFonts => "math fonts",
            Self::OutputRows => "max output height",
            Self::ExampleTimeout => "compute examples",
        }
    }

    /// What the setting controls, or why its value does not take effect here,
    /// shown beneath the rows while it is selected.
    fn description(self, preferences: Preferences) -> &'static str {
        match self {
            Self::Maths if !preferences.editor.maths.available() => {
                "this terminal shows no pictures: kitty and Ghostty do, inside tmux too; drawn as unicode meanwhile"
            }
            Self::Appearance(field) => field.description(),
            Self::Input(field) => field.description(),
            Self::SemanticHighlighting => "enrich source colors using m2-ls when it is installed",
            Self::Statusline => "where the status line sits",
            Self::Maths => "draw returned maths as Unicode text, plain ASCII, or typeset pictures",
            Self::Fractions => "how rational values are drawn",
            Self::MathFonts => "use TeX math fonts like mathbb, mathcal",
            Self::OutputRows => "rows a value shows before its middle folds away",
            Self::ExampleTimeout => {
                "run documentation examples again, each for at most this long, to draw their values"
            }
        }
    }

    fn value(self, preferences: Preferences) -> PickerValue {
        let editor = preferences.editor;
        match self {
            Self::Appearance(field) => PickerValue::among(
                &field.choices(preferences.appearance),
                &preferences.appearance,
                |&appearance| field.shown(appearance).into(),
            ),
            Self::Input(field) => PickerValue::Stepped(field.value(preferences.input)),
            Self::Statusline => PickerValue::among(
                StatusLinePosition::CHOICES,
                &editor.statusline,
                |&position| position.label().into(),
            ),
            Self::Maths => PickerValue::among(MathDisplay::CHOICES, &editor.maths, |&display| {
                PickerOption {
                    label: display.label().to_string(),
                    unavailable: (!display.available()).then(|| match graphics::terminal() {
                        Some(terminal) => format!("unavailable in {terminal}"),
                        None => "unavailable here".to_string(),
                    }),
                }
            }),
            Self::Fractions => {
                PickerValue::among(FractionStyle::CHOICES, &editor.fractions, |&style| {
                    style.label().into()
                })
            }
            Self::MathFonts => PickerValue::among(&[false, true], &editor.math_fonts, |&on| {
                match on {
                    true => "on",
                    false => "off",
                }
                .into()
            }),
            Self::SemanticHighlighting => {
                PickerValue::among(&[false, true], &editor.semantic_highlighting, |&on| {
                    if on { "on" } else { "off" }.into()
                })
            }
            Self::OutputRows => PickerValue::Stepped(format!("{} rows", editor.output_rows)),
            Self::ExampleTimeout => PickerValue::Stepped(match editor.example_seconds {
                0 => "off".to_string(),
                seconds => format!("{seconds}s"),
            }),
        }
    }

    /// `preferences` with this setting moved `delta` steps.
    fn adjust(self, mut preferences: Preferences, delta: i8) -> Preferences {
        let editor = &mut preferences.editor;
        match self {
            Self::Appearance(field) => {
                let appearance = preferences.appearance;
                preferences.appearance = cycled(&field.choices(appearance), appearance, delta);
            }
            Self::Input(field) => field.adjust(&mut preferences.input, delta),
            Self::Statusline => editor.statusline = editor.statusline.cycle(delta),
            Self::Maths => editor.maths = editor.maths.cycle(delta),
            Self::Fractions => editor.fractions = editor.fractions.cycle(delta),
            Self::MathFonts => editor.math_fonts = !editor.math_fonts,
            Self::SemanticHighlighting => {
                editor.semantic_highlighting = !editor.semantic_highlighting
            }
            Self::OutputRows => editor.adjust_output_rows(delta),
            Self::ExampleTimeout => editor.adjust_example_seconds(delta),
        }
        preferences
    }

    fn row(self, preferences: Preferences) -> Box<dyn PickerItem> {
        let label = Span::styled(self.label(), scheme().settings_item_label());
        Row::adjust(vec![label.into()], move |delta| {
            EditorEvent::SetPreferences(self.adjust(preferences, delta))
        })
        .with_value(self.value(preferences))
        .described(self.description(preferences))
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepping_any_setting_forward_then_back_restores_it() {
        let preferences = Preferences::default();
        for setting in TABS.iter().flat_map(|tab| tab.settings) {
            let stepped = setting.adjust(preferences, 1);
            assert_eq!(
                setting.adjust(stepped, -1),
                preferences,
                "{}",
                setting.label()
            );
        }
    }
}
