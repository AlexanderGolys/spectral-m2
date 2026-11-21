//! The *rendering* tab: how returned values and their maths are drawn.

use ratatui::layout::Rect;

use serde::{Deserialize, Serialize};

use super::row::{SettingRow, SettingsTab};
use super::Settings;
use crate::ui::config::UI;
use crate::ui::controller::picker::{PickerItem, PickerOption, PickerValue};
use crate::ui::cycle::cyclic;
use crate::ui::glyphs::Charset;
use crate::ui::graphics::{self, ratex::diagram::ArrowLengths};
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

/// The values of the *rendering* tab, and the maths colours the *appearance*
/// tab shows: they change how values lay out, as every rendering value does.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderingSettings {
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
    /// Whether the maps of a complex or a diagram are labelled by their matrices
    /// rather than their names.
    pub matrix_labels: bool,
    /// How long the arrows of typeset complexes and diagrams are drawn.
    pub arrows: ArrowLengths,
}

impl Default for RenderingSettings {
    fn default() -> Self {
        Self {
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
            matrix_labels: false,
            arrows: ArrowLengths::default(),
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
    /// Whether returned values lay out differently, moving their heights: any
    /// setting differs but italics.
    pub(super) fn reformats(&self, other: &Self) -> bool {
        Self {
            italics: other.italics,
            ..*self
        } != *other
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
        Italics = "italics",
        Charset = "symbols",
        Fractions = "fractions",
        MathFonts = "math fonts",
        MatrixRowSpacing = "matrix row spacing",
        MatrixDegrees = "matrix degrees",
        MatrixLabels = "matrix labels",
        ArrowLength = "arrow length",
        ArrowHeight = "arrow height",
    }
}

/// `percent` moved `delta` picker steps of `step` points each, kept between
/// `least` and `most`.
fn stepped(percent: u16, delta: i8, step: u16, (least, most): (u16, u16)) -> u16 {
    let moved = u16::from(delta.unsigned_abs()) * step;
    match delta > 0 {
        true => percent.saturating_add(moved),
        false => percent.saturating_sub(moved),
    }
    .clamp(least, most)
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
            Self::MathSize => "preferred formula size; formulas wrap at the available width",
            Self::Maths => "draw returned maths as Unicode text, plain ASCII, or typeset pictures",
            Self::RendererPosition => "place the available output modes and current mode name in the value border",
            Self::Italics => "draw emphasised and commented text in italics",
            Self::Charset => "draw prompts, dividers, icons and the spinner with Nerd Font icons, Unicode symbols, or plain ASCII for fonts with neither",
            Self::Fractions => "how rational values are drawn",
            Self::MathFonts => "use TeX math fonts like mathbb, mathcal",
            Self::MatrixRowSpacing => {
                "distance between the rows of a typeset matrix, relative to TeX's own"
            }
            Self::MatrixDegrees => "show the degrees of a matrix's rows beside them, greyed out",
            Self::MatrixLabels => "label the maps of complexes and diagrams by their matrices rather than their names, dd_i or f_i",
            Self::ArrowLength => "how long the arrows along a row of a typeset complex or diagram run, relative to the shortest; never shorter than their labels",
            Self::ArrowHeight => "how far the arrows between the rows of a typeset diagram run down, relative to the shortest; never shorter than their labels",
        }
    }

    fn value(self, settings: Settings) -> PickerValue {
        let rendering = settings.rendering;
        match self {
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
            Self::MatrixLabels => switch(rendering.matrix_labels, "on"),
            Self::ArrowLength => {
                PickerValue::Stepped(format!("{}%", rendering.arrows.across_percent))
            }
            Self::ArrowHeight => {
                PickerValue::Stepped(format!("{}%", rendering.arrows.down_percent))
            }
        }
    }

    fn adjust(self, settings: &mut Settings, delta: i8) {
        let rendering = &mut settings.rendering;
        let arrow = |percent: u16| {
            stepped(
                percent,
                delta,
                UI::ARROW_LENGTH_STEP,
                (UI::MIN_ARROW_LENGTH_PERCENT, UI::MAX_ARROW_LENGTH_PERCENT),
            )
        };
        match self {
            Self::Maths => rendering.maths = rendering.maths.cycle(delta),
            Self::RendererPosition => {
                rendering.renderer_position = rendering.renderer_position.cycle(delta)
            }
            Self::MathSize => {
                rendering.math_size_percent = stepped(
                    rendering.math_size_percent,
                    delta,
                    UI::MATH_SIZE_STEP,
                    (UI::MIN_MATH_SIZE_PERCENT, UI::MAX_MATH_SIZE_PERCENT),
                );
            }
            Self::Italics => rendering.italics = !rendering.italics,
            Self::Charset => rendering.charset = rendering.charset.cycle(delta),
            Self::Fractions => rendering.fractions = rendering.fractions.cycle(delta),
            Self::MathFonts => rendering.math_fonts = !rendering.math_fonts,
            Self::MatrixRowSpacing => {
                rendering.matrix_row_spacing_percent = stepped(
                    rendering.matrix_row_spacing_percent,
                    delta,
                    UI::MATRIX_ROW_SPACING_STEP,
                    (
                        UI::MIN_MATRIX_ROW_SPACING_PERCENT,
                        UI::MAX_MATRIX_ROW_SPACING_PERCENT,
                    ),
                );
            }
            Self::MatrixDegrees => rendering.matrix_degrees = !rendering.matrix_degrees,
            Self::MatrixLabels => rendering.matrix_labels = !rendering.matrix_labels,
            Self::ArrowLength => {
                rendering.arrows.across_percent = arrow(rendering.arrows.across_percent)
            }
            Self::ArrowHeight => {
                rendering.arrows.down_percent = arrow(rendering.arrows.down_percent)
            }
        }
    }

    /// Semantic colors need the language server, typeset pictures set fractions
    /// and fonts themselves, and only they space matrix rows and scale arrows.
    fn applies(self, settings: Settings) -> bool {
        let rendering = settings.rendering;
        match self {
            Self::Fractions | Self::MathFonts => !rendering.pictured(),
            Self::MathSize | Self::MatrixRowSpacing | Self::ArrowLength | Self::ArrowHeight => {
                rendering.pictured()
            }
            _ => true,
        }
    }
}
