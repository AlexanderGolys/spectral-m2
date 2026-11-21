//! The *rendering* tab: how returned values and their maths are drawn.

use ratatui::layout::Rect;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::row::{SettingRow, SettingsTab};
use super::Settings;
use crate::ui::config::UI;
use crate::ui::controller::picker::{PickerItem, PickerOption, PickerValue};
use crate::ui::cycle::cyclic;
use crate::ui::glyphs::Charset;
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

/// The values of the *rendering* tab.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderingSettings {
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
    /// The symbols the shell's chrome is drawn with; colours stay as they are.
    pub charset: Charset,
    pub fractions: FractionStyle,
    /// Whether rings are drawn as blackboard glyphs (`ℤ`) rather than by name (`ZZ`).
    pub math_fonts: bool,
    /// Distance between the rows of a typeset matrix, as a percentage of TeX's own.
    pub matrix_row_spacing_percent: u16,
    /// Whether a matrix shows the degrees of its rows beside them.
    pub matrix_degrees: bool,
    /// Seconds each documentation example may take to be computed again, so
    /// its value is drawn from its structure; none computes nothing.
    pub example_seconds: u16,
    pub output_view: OutputView,
}

impl Default for RenderingSettings {
    fn default() -> Self {
        Self {
            reference_code_background: false,
            maths: MathDisplay::default(),
            math_size_percent: UI::MATH_SIZE_PERCENT,
            math_ink: MathInk::default(),
            index_color: ScriptColor::default(),
            exponent_color: ScriptColor::default(),
            renderer_position: RendererPosition::default(),
            italics: true,
            charset: Charset::default(),
            fractions: FractionStyle::default(),
            math_fonts: false,
            matrix_row_spacing_percent: UI::MATRIX_ROW_SPACING_PERCENT,
            matrix_degrees: false,
            example_seconds: 0,
            output_view: OutputView::default(),
        }
    }
}

impl SettingsTab for RenderingSettings {
    const LABEL: &'static str = "rendering";

    fn rows(settings: Settings) -> Vec<Box<dyn PickerItem>> {
        RenderingSetting::rows(RenderingSetting::CHOICES, settings).collect()
    }
}

impl RenderingSettings {
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
            self.matrix_degrees,
            self.charset,
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
            other.matrix_degrees,
            other.charset,
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
        Charset = "symbols",
        Fractions = "fractions",
        MathFonts = "math fonts",
        MatrixRowSpacing = "matrix row spacing",
        MatrixDegrees = "matrix degrees",
        ExampleTimeout = "compute examples",
        ReferenceCodeBackground = "swap reference backgrounds",
        OutputView = "output view",
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
            Self::Charset => "draw prompts, dividers, icons and the spinner with Nerd Font icons, Unicode symbols, or plain ASCII for fonts with neither",
            Self::Fractions => "how rational values are drawn",
            Self::MathFonts => "use TeX math fonts like mathbb, mathcal",
            Self::MatrixRowSpacing => {
                "distance between the rows of a typeset matrix, relative to TeX's own"
            }
            Self::MatrixDegrees => "show the degrees of a matrix's rows beside them, greyed out",
            Self::ExampleTimeout => {
                "run documentation examples again, each for at most this long, to draw their values"
            }
            Self::OutputView => {
                "what the output view beside the session shows: everything but the code, values, or printed text"
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
            Self::Charset => PickerValue::among(Charset::CHOICES, &rendering.charset, |charset| {
                charset.label().into()
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
            Self::MatrixDegrees => switch(rendering.matrix_degrees, "on"),
            Self::OutputView => {
                PickerValue::among(OutputView::CHOICES, &rendering.output_view, |view| {
                    view.label().into()
                })
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
                let step = u16::from(delta.unsigned_abs()) * UI::MATH_SIZE_STEP;
                rendering.math_size_percent = if delta > 0 {
                    rendering.math_size_percent.saturating_add(step)
                } else {
                    rendering.math_size_percent.saturating_sub(step)
                }
                .clamp(UI::MIN_MATH_SIZE_PERCENT, UI::MAX_MATH_SIZE_PERCENT);
            }
            Self::MathInk => rendering.math_ink = rendering.math_ink.cycle(delta),
            Self::IndexColor => rendering.index_color = rendering.index_color.cycle(delta),
            Self::ExponentColor => rendering.exponent_color = rendering.exponent_color.cycle(delta),
            Self::Italics => rendering.italics = !rendering.italics,
            Self::Charset => rendering.charset = rendering.charset.cycle(delta),
            Self::Fractions => rendering.fractions = rendering.fractions.cycle(delta),
            Self::MathFonts => rendering.math_fonts = !rendering.math_fonts,
            Self::MatrixRowSpacing => {
                let step = u16::from(delta.unsigned_abs()) * UI::MATRIX_ROW_SPACING_STEP;
                rendering.matrix_row_spacing_percent = match delta > 0 {
                    true => rendering.matrix_row_spacing_percent.saturating_add(step),
                    false => rendering.matrix_row_spacing_percent.saturating_sub(step),
                }
                .clamp(
                    UI::MIN_MATRIX_ROW_SPACING_PERCENT,
                    UI::MAX_MATRIX_ROW_SPACING_PERCENT,
                );
            }
            Self::MatrixDegrees => rendering.matrix_degrees = !rendering.matrix_degrees,
            Self::OutputView => rendering.output_view = rendering.output_view.cycle(delta),
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
