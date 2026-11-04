//! The *rendering* tab: what colours source and how returned maths is drawn.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use these_macros_should_be_illegal::discriminated_str;

use super::tabs::SettingRow;
use super::Settings;
use crate::code::semantic::SERVER_EXECUTABLE;
use crate::ui::config::UI;
use crate::ui::controller::picker::{PickerOption, PickerValue};
use crate::ui::cycle::cyclic;
use crate::ui::graphics;
use crate::ui::render::maths::{FractionStyle, MathDisplay};

/// The values of the *rendering* tab.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderingSettings {
    /// Whether an installed language server analyses the session's source.
    pub language_server: bool,
    /// Whether the language server enriches source colors, which it can only do
    /// while it runs.
    pub semantic_highlighting: bool,
    pub maths: MathDisplay,
    pub fractions: FractionStyle,
    /// Whether rings are drawn as blackboard glyphs (`ℤ`) rather than by name (`ZZ`).
    pub math_fonts: bool,
    /// Distance between the rows of a typeset matrix, as a percentage of TeX's own.
    pub matrix_row_spacing_percent: u16,
    /// Seconds each documentation example may take to be computed again, so
    /// its value is drawn from its structure; none computes nothing.
    pub example_seconds: u16,
}

impl Default for RenderingSettings {
    fn default() -> Self {
        Self {
            language_server: true,
            semantic_highlighting: true,
            maths: MathDisplay::default(),
            fractions: FractionStyle::default(),
            math_fonts: false,
            matrix_row_spacing_percent: UI.matrix_row_spacing_percent,
            example_seconds: 0,
        }
    }
}

impl RenderingSettings {
    /// Whether source takes semantic colors: chosen, and the server providing them runs.
    pub const fn semantic_colors(&self) -> bool {
        self.language_server && self.semantic_highlighting
    }

    /// How long each documentation example may take to be computed again; zero
    /// when they are not.
    pub fn example_timeout(&self) -> Duration {
        Duration::from_secs(self.example_seconds.into())
    }

    /// Whether returned values lay out differently, moving their heights.
    pub(super) fn reformats(&self, other: &Self) -> bool {
        (
            self.maths,
            self.fractions,
            self.math_fonts,
            self.matrix_row_spacing_percent,
        ) != (
            other.maths,
            other.fractions,
            other.math_fonts,
            other.matrix_row_spacing_percent,
        )
    }

    /// Whether maths is drawn as typeset pictures here, not only chosen to be.
    fn pictured(&self) -> bool {
        self.maths == MathDisplay::Typeset && self.maths.available()
    }
}

cyclic! {
    /// One row of the *rendering* tab.
    #[discriminated_str(label)]
    #[derive(Clone, Copy, PartialEq)]
    pub enum RenderingSetting {
        LanguageServer = "language server",
        SemanticHighlighting = "semantic highlighting",
        Maths = "maths",
        Fractions = "fractions",
        MathFonts = "math fonts",
        MatrixRowSpacing = "matrix row spacing",
        ExampleTimeout = "compute examples",
    }
}

/// An off/on value at `chosen`, its on option named `on`.
fn switch(chosen: bool, on: &str) -> PickerValue {
    PickerValue::among(&[false, true], &chosen, |&chosen| match chosen {
        true => on.into(),
        false => "off".into(),
    })
}

impl SettingRow for RenderingSetting {
    fn label(self) -> &'static str {
        RenderingSetting::label(&self)
    }

    fn description(self, settings: Settings) -> &'static str {
        match self {
            Self::Maths if !settings.rendering.maths.available() => {
                "this terminal shows no pictures: kitty and Ghostty do, inside tmux too; drawn as unicode meanwhile"
            }
            Self::LanguageServer => "analyse the session's source with m2-ls when it is installed",
            Self::SemanticHighlighting => "enrich source colors using the language server",
            Self::Maths => "draw returned maths as Unicode text, plain ASCII, or typeset pictures",
            Self::Fractions => "how rational values are drawn",
            Self::MathFonts => "use TeX math fonts like mathbb, mathcal",
            Self::MatrixRowSpacing => {
                "distance between the rows of a typeset matrix, relative to TeX's own"
            }
            Self::ExampleTimeout => {
                "run documentation examples again, each for at most this long, to draw their values"
            }
        }
    }

    fn value(self, settings: Settings) -> PickerValue {
        let rendering = settings.rendering;
        match self {
            Self::LanguageServer => switch(rendering.language_server, SERVER_EXECUTABLE),
            Self::SemanticHighlighting => switch(rendering.semantic_colors(), "on"),
            Self::Maths => PickerValue::among(MathDisplay::CHOICES, &rendering.maths, |&display| {
                PickerOption {
                    label: display.label().to_string(),
                    unavailable: (!display.available()).then(|| match graphics::terminal() {
                        Some(terminal) => format!("unavailable in {terminal}"),
                        None => "unavailable here".to_string(),
                    }),
                }
            }),
            Self::Fractions => {
                PickerValue::among(FractionStyle::CHOICES, &rendering.fractions, |&style| {
                    style.label().into()
                })
            }
            Self::MathFonts => switch(rendering.math_fonts, "on"),
            Self::MatrixRowSpacing => {
                PickerValue::Stepped(format!("{}%", rendering.matrix_row_spacing_percent))
            }
            Self::ExampleTimeout => PickerValue::Stepped(match rendering.example_seconds {
                0 => "off".to_string(),
                seconds => format!("{seconds}s"),
            }),
        }
    }

    fn adjust(self, settings: &mut Settings, delta: i8) {
        let rendering = &mut settings.rendering;
        match self {
            Self::LanguageServer => rendering.language_server = !rendering.language_server,
            Self::SemanticHighlighting => {
                rendering.semantic_highlighting = !rendering.semantic_highlighting
            }
            Self::Maths => rendering.maths = rendering.maths.cycle(delta),
            Self::Fractions => rendering.fractions = rendering.fractions.cycle(delta),
            Self::MathFonts => rendering.math_fonts = !rendering.math_fonts,
            Self::MatrixRowSpacing => {
                let step = u16::from(delta.unsigned_abs()) * UI.matrix_row_spacing_step;
                rendering.matrix_row_spacing_percent = match delta > 0 {
                    true => rendering.matrix_row_spacing_percent.saturating_add(step),
                    false => rendering.matrix_row_spacing_percent.saturating_sub(step),
                }
                .clamp(
                    UI.min_matrix_row_spacing_percent,
                    UI.max_matrix_row_spacing_percent,
                );
            }
            Self::ExampleTimeout => {
                rendering.example_seconds = rendering
                    .example_seconds
                    .saturating_add_signed(delta.into());
            }
        }
    }

    /// Semantic colors need the language server, typeset pictures set fractions
    /// and fonts themselves, and only they space matrix rows.
    fn applies(self, settings: Settings) -> bool {
        let rendering = settings.rendering;
        match self {
            Self::SemanticHighlighting => rendering.language_server,
            Self::Fractions | Self::MathFonts => !rendering.pictured(),
            Self::MatrixRowSpacing => rendering.pictured(),
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_highlighting_reads_off_and_locks_without_the_language_server() {
        let mut settings = Settings::default();
        settings.rendering.semantic_highlighting = true;
        settings.rendering.language_server = false;
        let row = RenderingSetting::SemanticHighlighting.row(settings);
        assert!(!row.selectable());
        assert!(matches!(
            row.value(),
            Some(PickerValue::Choice { options, chosen }) if options[*chosen].label == "off"
        ));
        settings.rendering.language_server = true;
        assert!(RenderingSetting::SemanticHighlighting
            .row(settings)
            .selectable());
    }

    #[test]
    fn typeset_maths_locks_fractions_and_math_fonts_where_it_is_drawn() {
        let mut settings = Settings::default();
        settings.rendering.maths = MathDisplay::Typeset;
        let typeset = MathDisplay::Typeset.available();
        for setting in [RenderingSetting::Fractions, RenderingSetting::MathFonts] {
            assert_eq!(setting.row(settings).selectable(), !typeset);
        }
        settings.rendering.maths = MathDisplay::Unicode;
        for setting in [RenderingSetting::Fractions, RenderingSetting::MathFonts] {
            assert!(setting.row(settings).selectable());
        }
    }

    #[test]
    fn matrix_row_spacing_is_only_adjustable_while_maths_is_drawn_as_pictures() {
        let mut settings = Settings::default();
        settings.rendering.maths = MathDisplay::Typeset;
        assert_eq!(
            RenderingSetting::MatrixRowSpacing
                .row(settings)
                .selectable(),
            MathDisplay::Typeset.available()
        );
        settings.rendering.maths = MathDisplay::Unicode;
        assert!(!RenderingSetting::MatrixRowSpacing
            .row(settings)
            .selectable());
    }

    #[test]
    fn the_examples_timeout_steps_by_seconds_and_stops_at_off() {
        let mut settings = Settings::default();
        RenderingSetting::ExampleTimeout.adjust(&mut settings, 1);
        let on = settings.rendering.example_timeout();
        assert!(!on.is_zero());
        RenderingSetting::ExampleTimeout.adjust(&mut settings, 1);
        assert!(settings.rendering.example_timeout() > on);

        RenderingSetting::ExampleTimeout.adjust(&mut settings, -2);
        RenderingSetting::ExampleTimeout.adjust(&mut settings, -1);
        assert!(
            settings.rendering.example_timeout().is_zero(),
            "off is as low as it goes"
        );
    }

    #[test]
    fn the_matrix_row_spacing_stays_within_its_range() {
        let mut settings = Settings::default();
        RenderingSetting::MatrixRowSpacing.adjust(&mut settings, i8::MIN);
        RenderingSetting::MatrixRowSpacing.adjust(&mut settings, i8::MIN);
        RenderingSetting::MatrixRowSpacing.adjust(&mut settings, -1);
        assert_eq!(
            settings.rendering.matrix_row_spacing_percent,
            UI.min_matrix_row_spacing_percent
        );
        settings.rendering.matrix_row_spacing_percent = UI.max_matrix_row_spacing_percent;
        RenderingSetting::MatrixRowSpacing.adjust(&mut settings, 1);
        assert_eq!(
            settings.rendering.matrix_row_spacing_percent,
            UI.max_matrix_row_spacing_percent
        );
    }

    #[test]
    fn semantic_colors_need_the_language_server_but_keep_their_choice() {
        let mut rendering = RenderingSettings {
            semantic_highlighting: true,
            language_server: false,
            ..RenderingSettings::default()
        };
        assert!(!rendering.semantic_colors());
        rendering.language_server = true;
        assert!(rendering.semantic_colors());
    }
}
