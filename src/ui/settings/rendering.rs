//! The *rendering* tab: what colours source and how returned maths is drawn.

use ratatui::layout::Rect;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use these_macros_should_be_illegal::discriminated_str;

use super::row::SettingRow;
use super::Settings;
use crate::ui::config::UI;
use crate::ui::controller::picker::{PickerOption, PickerValue};
use crate::ui::cycle::cyclic;
use crate::ui::graphics;
use crate::ui::render::maths::{FractionStyle, MathDisplay, MathInk, ScriptColor};

cyclic! {
    /// Position of output mode controls on the value border.
    #[non_exhaustive]
    #[discriminated_str(label)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum RendererPosition {
        Off = "off",
        UpperLeft = "upper left",
        UpperCenter = "upper center",
        #[default]
        UpperRight = "upper right",
        LowerLeft = "lower left",
        LowerCenter = "lower center",
        LowerRight = "lower right",
    }
}

impl RendererPosition {
    /// Place a title between the corners of its output frame.
    pub fn area(self, frame: Rect, width: u16) -> Rect {
        if self == Self::Off {
            return Rect::default();
        }
        let available = frame.width.saturating_sub(2);
        let width = width.min(available);
        let offset = match self {
            Self::Off | Self::UpperLeft | Self::LowerLeft => 0,
            Self::UpperCenter | Self::LowerCenter => (available - width) / 2,
            Self::UpperRight | Self::LowerRight => available - width,
        };
        let row = match self {
            Self::Off | Self::UpperLeft | Self::UpperCenter | Self::UpperRight => frame.y,
            Self::LowerLeft | Self::LowerCenter | Self::LowerRight => {
                frame.bottom().saturating_sub(1)
            }
        };
        Rect::new(
            frame.x.saturating_add(1) + offset,
            row,
            width,
            u16::from(frame.height > 0),
        )
    }
}

/// The values of the *rendering* tab.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderingSettings {
    /// Whether an installed language server analyses the session's source.
    pub language_server: bool,
    /// Show automatic signature-help popups while typing.
    pub signature_hints: bool,
    /// Whether the language server enriches source colors, which it can only do
    /// while it runs.
    pub semantic_highlighting: bool,
    /// Swap code and canvas backgrounds in the reference pane.
    pub reference_code_background: bool,
    pub maths: MathDisplay,
    /// Preferred typeset formula size relative to terminal text.
    pub math_size_percent: u16,
    /// Whether returned maths takes a colour per role or one colour throughout.
    pub math_ink: MathInk,
    /// The colour of subscripts in returned maths.
    pub index_color: ScriptColor,
    /// The colour of superscripts in returned maths.
    pub exponent_color: ScriptColor,
    /// Position of per-output renderer controls within the border.
    pub renderer_position: RendererPosition,
    /// Whether text may be drawn in italics.
    pub italics: bool,
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
            signature_hints: false,
            semantic_highlighting: true,
            reference_code_background: false,
            maths: MathDisplay::default(),
            math_size_percent: UI.math_size_percent,
            math_ink: MathInk::default(),
            index_color: ScriptColor::default(),
            exponent_color: ScriptColor::default(),
            renderer_position: RendererPosition::default(),
            italics: true,
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
            self.reference_code_background,
            self.maths,
            self.math_size_percent,
            self.math_ink,
            self.index_color,
            self.exponent_color,
            self.renderer_position,
            self.fractions,
            self.math_fonts,
            self.matrix_row_spacing_percent,
        ) != (
            other.reference_code_background,
            other.maths,
            other.math_size_percent,
            other.math_ink,
            other.index_color,
            other.exponent_color,
            other.renderer_position,
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
        Maths = "maths",
        MathSize = "maths size",
        RendererPosition = "output mode position",
        MathInk = "maths colours",
        IndexColor = "index colours",
        ExponentColor = "exponent colours",
        Italics = "italics",
        Fractions = "fractions",
        MathFonts = "math fonts",
        MatrixRowSpacing = "matrix row spacing",
        ExampleTimeout = "compute examples",
        ReferenceCodeBackground = "swap reference backgrounds",
    }
}

/// An off/on value at `chosen`, its on option named `on`.
pub(super) fn switch(chosen: bool, on: &str) -> PickerValue {
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
            Self::ReferenceCodeBackground => "exchange the reference pane canvas and code backgrounds",
            Self::MathSize => "preferred formula size; formulas wrap at the available width",
            Self::Maths => "draw returned maths as Unicode text, plain ASCII, or typeset pictures",
            Self::RendererPosition => "place the available output modes and current mode name in the value border",
            Self::MathInk => "colour returned maths by role, or in the value colour with types in the type colour",
            Self::IndexColor => "colour of subscripts: their own, their symbol's, or the operator colour; delimiters and punctuation keep theirs",
            Self::ExponentColor => "colour of exponents: their own, their symbol's, or the operator colour; delimiters and punctuation keep theirs",
            Self::Italics => "draw emphasised and commented text in italics",
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
            Self::ReferenceCodeBackground => switch(rendering.reference_code_background, "on"),
            Self::Maths => PickerValue::among(MathDisplay::CHOICES, &rendering.maths, |&display| {
                PickerOption {
                    label: display.label().to_string(),
                    unavailable: (!display.available()).then(|| match graphics::terminal() {
                        Some(terminal) => format!("unavailable in {terminal}"),
                        None => "unavailable here".to_string(),
                    }),
                }
            }),
            Self::RendererPosition => PickerValue::among(
                RendererPosition::CHOICES,
                &rendering.renderer_position,
                |position| position.label().into(),
            ),
            Self::MathInk => PickerValue::among(MathInk::CHOICES, &rendering.math_ink, |&ink| {
                ink.label().into()
            }),
            Self::IndexColor => {
                PickerValue::among(ScriptColor::CHOICES, &rendering.index_color, |&color| {
                    color.label().into()
                })
            }
            Self::ExponentColor => {
                PickerValue::among(ScriptColor::CHOICES, &rendering.exponent_color, |&color| {
                    color.label().into()
                })
            }
            Self::MathSize => PickerValue::Stepped(format!("{}%", rendering.math_size_percent)),
            Self::Italics => switch(rendering.italics, "on"),
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
            Self::ReferenceCodeBackground => {
                rendering.reference_code_background = !rendering.reference_code_background
            }
            Self::Maths => rendering.maths = rendering.maths.cycle(delta),
            Self::RendererPosition => {
                rendering.renderer_position = rendering.renderer_position.cycle(delta)
            }
            Self::MathSize => {
                let step = u16::from(delta.unsigned_abs()) * UI.math_size_step;
                rendering.math_size_percent = if delta > 0 {
                    rendering.math_size_percent.saturating_add(step)
                } else {
                    rendering.math_size_percent.saturating_sub(step)
                }
                .clamp(UI.min_math_size_percent, UI.max_math_size_percent);
            }
            Self::MathInk => rendering.math_ink = rendering.math_ink.cycle(delta),
            Self::IndexColor => rendering.index_color = rendering.index_color.cycle(delta),
            Self::ExponentColor => rendering.exponent_color = rendering.exponent_color.cycle(delta),
            Self::Italics => rendering.italics = !rendering.italics,
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
            Self::Fractions | Self::MathFonts => !rendering.pictured(),
            Self::MathSize | Self::MatrixRowSpacing => rendering.pictured(),
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn every_new_rendering_choice_cycles_and_asks_for_a_reformat() {
        let mut settings = Settings::default();
        let before = settings.rendering;
        for setting in [
            RenderingSetting::MathInk,
            RenderingSetting::IndexColor,
            RenderingSetting::ExponentColor,
            RenderingSetting::RendererPosition,
        ] {
            let mut changed = settings;
            setting.adjust(&mut changed, 1);
            assert!(changed.rendering.reformats(&before), "{}", setting.label());
        }
        RenderingSetting::Italics.adjust(&mut settings, 1);
        assert!(!settings.rendering.italics);
    }
    #[test]
    fn mode_positions_use_the_selected_border_and_alignment() {
        let frame = Rect::new(4, 3, 40, 10);
        for (position, x, y) in [
            (RendererPosition::UpperLeft, 5, 3),
            (RendererPosition::UpperCenter, 18, 3),
            (RendererPosition::UpperRight, 31, 3),
            (RendererPosition::LowerLeft, 5, 12),
            (RendererPosition::LowerCenter, 18, 12),
            (RendererPosition::LowerRight, 31, 12),
        ] {
            assert_eq!(position.area(frame, 12), Rect::new(x, y, 12, 1));
            let clipped = position.area(frame, 100);
            assert!(clipped.x > frame.x && clipped.right() < frame.right());
        }
    }
}
