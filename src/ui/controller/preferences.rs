//! The settings picker: its tabs, the settings each lists, and the button
//! restoring their defaults.

use ratatui::text::Span;
use these_macros_should_be_illegal::discriminated_str;

use super::picker::{PickerItem, PickerOption, PickerTab, PickerValue, Row};
use super::settings::theme::groups::scheme;
use super::settings::theme::Appearance;
use super::settings::{Preferences, SegmentPlacement, StatusLinePosition};
use super::EditorEvent;
use crate::code::semantic::SERVER_EXECUTABLE;
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
            Setting::Input(InputSetting::MaxHeight),
            Setting::OutputRows,
            Setting::Input(InputSetting::Scrolloff),
            Setting::Input(InputSetting::CompleteAfter),
            Setting::Input(InputSetting::HintCompletionBelow),
        ],
    },
    SettingsTab {
        label: "statusline",
        settings: &[
            Setting::StatuslinePosition,
            Setting::StatuslineScheme,
            Setting::StatuslineDirectory,
            Setting::StatuslineHints,
        ],
    },
    SettingsTab {
        label: "rendering",
        settings: &[
            Setting::LanguageServer,
            Setting::SemanticHighlighting,
            Setting::Maths,
            Setting::Fractions,
            Setting::MathFonts,
            Setting::MatrixRowSpacing,
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

/// An off/on value at `chosen`, its on option named `on`.
fn switch(chosen: &bool, on: &str) -> PickerValue {
    PickerValue::among(&[false, true], chosen, |&chosen| match chosen {
        true => on.into(),
        false => "off".into(),
    })
}

/// The side `chosen` names, among those a status line segment may take.
fn placement(chosen: &SegmentPlacement) -> PickerValue {
    PickerValue::among(SegmentPlacement::CHOICES, chosen, |&placement| {
        placement.label().into()
    })
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
    StatuslinePosition,
    StatuslineScheme,
    StatuslineDirectory,
    StatuslineHints,
    LanguageServer,
    SemanticHighlighting,
    Maths,
    Fractions,
    MathFonts,
    MatrixRowSpacing,
    OutputRows,
    ExampleTimeout,
}

impl Setting {
    /// The setting's name, leading its row.
    const fn label(self) -> &'static str {
        match self {
            Self::Appearance(field) => field.label(),
            Self::Input(field) => field.label(),
            Self::StatuslinePosition => "position",
            Self::StatuslineScheme => "colour scheme",
            Self::StatuslineDirectory => "directory",
            Self::StatuslineHints => "key hints",
            Self::LanguageServer => "language server",
            Self::SemanticHighlighting => "semantic highlighting",
            Self::Maths => "maths",
            Self::Fractions => "fractions",
            Self::MathFonts => "math fonts",
            Self::MatrixRowSpacing => "matrix row spacing",
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
            Self::LanguageServer => "analyse the session's source with m2-ls when it is installed",
            Self::SemanticHighlighting => "enrich source colors using the language server",
            Self::StatuslinePosition => "where the status line sits",
            Self::StatuslineScheme => "where the idle line names the colour scheme",
            Self::StatuslineDirectory => "where the idle line names the working directory",
            Self::StatuslineHints => "where the idle line names the commands and settings keys",
            Self::Maths => "draw returned maths as Unicode text, plain ASCII, or typeset pictures",
            Self::Fractions => "how rational values are drawn",
            Self::MathFonts => "use TeX math fonts like mathbb, mathcal",
            Self::MatrixRowSpacing => {
                "distance between the rows of a typeset matrix, relative to TeX's own"
            }
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
            Self::StatuslinePosition => PickerValue::among(
                StatusLinePosition::CHOICES,
                &editor.statusline,
                |&position| position.label().into(),
            ),
            Self::StatuslineScheme => placement(&editor.idle_statusline.scheme),
            Self::StatuslineDirectory => placement(&editor.idle_statusline.directory),
            Self::StatuslineHints => placement(&editor.idle_statusline.hints),
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
            Self::MathFonts => switch(&editor.math_fonts, "on"),
            Self::MatrixRowSpacing => {
                PickerValue::Stepped(format!("{}%", editor.matrix_row_spacing_percent))
            }
            Self::LanguageServer => switch(&editor.language_server, SERVER_EXECUTABLE),
            Self::SemanticHighlighting => switch(&editor.semantic_colors(), "on"),
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
            Self::StatuslinePosition => editor.statusline = editor.statusline.cycle(delta),
            Self::StatuslineScheme => {
                let scheme = &mut editor.idle_statusline.scheme;
                *scheme = scheme.cycle(delta);
            }
            Self::StatuslineDirectory => {
                let directory = &mut editor.idle_statusline.directory;
                *directory = directory.cycle(delta);
            }
            Self::StatuslineHints => {
                let hints = &mut editor.idle_statusline.hints;
                *hints = hints.cycle(delta);
            }
            Self::Maths => editor.maths = editor.maths.cycle(delta),
            Self::Fractions => editor.fractions = editor.fractions.cycle(delta),
            Self::MathFonts => editor.math_fonts = !editor.math_fonts,
            Self::MatrixRowSpacing => editor.adjust_matrix_row_spacing(delta),
            Self::LanguageServer => editor.language_server = !editor.language_server,
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
        .selectable_if(self.applies(preferences))
        .into()
    }

    /// Whether the setting takes effect: semantic colors need the language
    /// server, typeset pictures set fractions and fonts themselves, only they
    /// space matrix rows, and segments need a status line to sit on.
    fn applies(self, preferences: Preferences) -> bool {
        let editor = preferences.editor;
        let pictured = editor.maths == MathDisplay::Typeset && editor.maths.available();
        match self {
            Self::SemanticHighlighting => editor.language_server,
            Self::Fractions | Self::MathFonts => !pictured,
            Self::MatrixRowSpacing => pictured,
            Self::StatuslineScheme | Self::StatuslineDirectory | Self::StatuslineHints => {
                editor.statusline != StatusLinePosition::Off
            }
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_highlighting_reads_off_and_locks_without_the_language_server() {
        let mut preferences = Preferences::default();
        preferences.editor.semantic_highlighting = true;
        preferences.editor.language_server = false;
        let row = Setting::SemanticHighlighting.row(preferences);
        assert!(!row.selectable());
        assert!(matches!(
            row.value(),
            Some(PickerValue::Choice { options, chosen }) if options[*chosen].label == "off"
        ));
        preferences.editor.language_server = true;
        assert!(Setting::SemanticHighlighting.row(preferences).selectable());
    }

    #[test]
    fn typeset_maths_locks_fractions_and_math_fonts_where_it_is_drawn() {
        let mut preferences = Preferences::default();
        preferences.editor.maths = MathDisplay::Typeset;
        let typeset = MathDisplay::Typeset.available();
        for setting in [Setting::Fractions, Setting::MathFonts] {
            assert_eq!(setting.row(preferences).selectable(), !typeset);
        }
        preferences.editor.maths = MathDisplay::Unicode;
        for setting in [Setting::Fractions, Setting::MathFonts] {
            assert!(setting.row(preferences).selectable());
        }
    }

    #[test]
    fn matrix_row_spacing_is_only_adjustable_while_maths_is_drawn_as_pictures() {
        let mut preferences = Preferences::default();
        preferences.editor.maths = MathDisplay::Typeset;
        assert_eq!(
            Setting::MatrixRowSpacing.row(preferences).selectable(),
            MathDisplay::Typeset.available()
        );
        preferences.editor.maths = MathDisplay::Unicode;
        assert!(!Setting::MatrixRowSpacing.row(preferences).selectable());
    }

    #[test]
    fn segment_rows_lock_while_the_status_line_is_off() {
        let segments = [
            Setting::StatuslineScheme,
            Setting::StatuslineDirectory,
            Setting::StatuslineHints,
        ];
        let mut preferences = Preferences::default();
        for segment in segments {
            assert!(segment.row(preferences).selectable());
        }
        preferences.editor.statusline = StatusLinePosition::Off;
        for segment in segments {
            assert!(!segment.row(preferences).selectable());
        }
    }

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
