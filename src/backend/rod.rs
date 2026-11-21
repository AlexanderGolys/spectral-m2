//! The drawing M2's Spectral package sends for a value: an expression whose
//! parts are drawings in turn, each of one of the expression classes the shell
//! draws, its atoms in their own colour and font. Nothing else rides along:
//! M2's other spellings of the value come beside it, each from its own
//! formatter.

use crate::ui::render::maths::{self, MathStyle, Place};
use crate::ui::theme::groups::HLGroup;
use serde::Deserialize;

/// The face an atom is set in: one of TeX's maths alphabets, set as maths, or
/// a text face, set as text beside it.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SRodFont {
    /// `\mathbb`: `Q` is ℚ.
    MathBB,
    MathFrak,
    /// `\mathcal`: `O` is 𝒪.
    MathCal,
    /// TeX's own maths: italic letters, upright digits.
    MathNormal,
    MathBf,
    MathIt,
    MathRm,
    /// Typewriter text, as code and strings are spelled.
    TextTT,
    TextNormal,
    TextBf,
    TextIt,
}

impl SRodFont {
    /// Whether text in this face is set as maths.
    pub fn is_maths(self) -> bool {
        !matches!(self, Self::TextTT | Self::TextNormal | Self::TextBf | Self::TextIt)
    }

    /// The TeX command setting text in this face, none for TeX's own maths.
    pub fn command(self) -> Option<&'static str> {
        Some(match self {
            Self::MathBB => "mathbb",
            Self::MathFrak => "mathfrak",
            Self::MathCal => "mathcal",
            Self::MathNormal => return None,
            Self::MathBf => "mathbf",
            Self::MathIt => "mathit",
            Self::MathRm => "mathrm",
            Self::TextTT => "texttt",
            Self::TextNormal => "text",
            Self::TextBf => "textbf",
            Self::TextIt => "textit",
        })
    }

}

/// How an atom is drawn: the highlight group it is coloured as and its face.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "PascalCase")]
pub struct SRodAtomicStyle {
    pub highlight: HLGroup,
    pub font: SRodFont,
}

/// A matrix's degrees as M2 gives them, `[rows, columns]`, a degree, a list of
/// integers, for every row and every column: `[[[1], [1]], [[2]]]` for a map
/// from `R^{-2}` to `R^{-1, -1}`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
pub struct Degrees {
    pub rows: Vec<Shape>,
    pub columns: Vec<Shape>,
}

/// A drawing: an atom, or an expression of one of the classes the shell draws,
/// each named as the M2 class it is, as Spectral's `shapes` lists them.
///
/// `x_1^2` arrives as
/// `{"Shape": "Superscript", "Parts": [{"Shape": "Subscript", "Parts": [x, 1]}, 2]}`,
/// each of `x`, `1` and `2` an atom such as
/// `{"Shape": "Atom", "Parts": {"Text": "x", "Highlight": "Variable", "Font": "MathNormal"}}`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "Shape", content = "Parts", rename_all_fields = "PascalCase")]
#[non_exhaustive]
pub enum Shape {
    /// An atom: text in its colour and face. Not M2's `Holder`, which is
    /// any expression standing for one value, `(x)` too.
    Atom {
        text: String,
        #[serde(flatten)]
        style: SRodAtomicStyle,
    },
    RowExpression(Vec<Shape>),
    Parenthesize((Box<Shape>,)),
    Divide(Box<Shape>, Box<Shape>),
    Subscript(Box<Shape>, Box<Shape>),
    Superscript(Box<Shape>, Box<Shape>),
    List(Vec<Shape>),
    Array(Vec<Shape>),
    Sequence(Vec<Shape>),
    AngleBarList(Vec<Shape>),
    /// Items one under another, between braces.
    VerticalList(Vec<Shape>),
    /// A matrix's entries, row by row, its rows' degrees beside them.
    MatrixExpression {
        rows: Vec<Vec<Shape>>,
        degrees: Option<Degrees>,
    },
    /// A bare grid of cells.
    Table(Vec<Vec<Shape>>),
    /// Rows whose every cell but the last sits in an aligned column, the last
    /// running on.
    AlignedExpression(Vec<Vec<Shape>>),
    UnderExpression(Box<Shape>, Box<Shape>),
    OverExpression(Box<Shape>, Box<Shape>),
    /// Pieces on a grid joined by arrows, each `(from, to, label)`, and the same
    /// diagram for a layout in characters when this one runs arrows across
    /// columns.
    DiagramExpression {
        pieces: Vec<(Place, Shape)>,
        arrows: Vec<(Place, Place, Option<Shape>)>,
        straight: Option<Box<Shape>>,
    },
    /// A hierarchy, each branch a label over its subtree.
    TreeExpression(Vec<(Shape, Shape)>),
    /// How a map is labelled, `(matrix, name)`: by its matrix, or by its name such
    /// as `dd_1` or `f_2`, whichever the *matrix labels* setting shows.
    MapLabel(Box<Shape>, Box<Shape>),
}

/// A documentation page, laid out apart from any drawing: its HTML, the width
/// its examples were archived at, and the names it resolves as types.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct DocPage {
    pub html: String,
    pub example_width: usize,
    #[serde(default)]
    pub type_names: Vec<String>,
}

impl Shape {

    /// A list's items with the delimiters it is fenced by, its entries between
    /// commas.
    pub fn listing(&self) -> Option<((char, char), &[Shape])> {
        match self {
            Self::List(items) => Some((('{', '}'), items)),
            Self::Array(items) => Some((('[', ']'), items)),
            Self::Sequence(items) => Some((('(', ')'), items)),
            _ => None,
        }
    }

    /// The text of an atom set as text, laid out as prose rather than maths.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Atom { text, style } if !style.font.is_maths() => Some(text),
            _ => None,
        }
    }

    /// The colour of an atom standing alone.
    pub fn group(&self) -> HLGroup {
        match self {
            Self::Atom { style, .. } => style.highlight,
            _ => HLGroup::Text,
        }
    }

    /// The drawings this one is built from, in reading order.
    pub fn parts(&self) -> Vec<&Self> {
        match self {
            Self::Atom { .. } => Vec::new(),
            Self::RowExpression(items)
            | Self::List(items)
            | Self::Array(items)
            | Self::Sequence(items)
            | Self::AngleBarList(items)
            | Self::VerticalList(items) => items.iter().collect(),
            Self::Parenthesize((body,)) => vec![&**body],
            Self::Divide(first, second)
            | Self::Subscript(first, second)
            | Self::Superscript(first, second)
            | Self::UnderExpression(first, second)
            | Self::OverExpression(first, second)
            | Self::MapLabel(first, second) => vec![&**first, &**second],
            Self::MatrixExpression { rows, degrees } => rows
                .iter()
                .flatten()
                .chain(degrees.iter().flat_map(|degrees| degrees.rows.iter().chain(&degrees.columns)))
                .collect(),
            Self::Table(rows) | Self::AlignedExpression(rows) => rows.iter().flatten().collect(),
            Self::DiagramExpression {
                pieces,
                arrows,
                straight,
            } => pieces
                .iter()
                .map(|(_, piece)| piece)
                .chain(arrows.iter().filter_map(|(.., label)| label.as_ref()))
                .chain(straight.as_deref())
                .collect(),
            Self::TreeExpression(branches) => branches
                .iter()
                .flat_map(|(label, subtree)| [label, subtree])
                .collect(),
        }
    }

    /// Whether this drawing or any below it satisfies `test`.
    pub fn any(&self, test: &impl Fn(&Self) -> bool) -> bool {
        test(self) || self.parts().into_iter().any(|part| part.any(test))
    }

    /// Whether any atom of this is set as maths.
    pub fn wants_typesetting(&self) -> bool {
        self.any(&|shape| matches!(shape, Self::Atom { style, .. } if style.font.is_maths()))
    }

    /// Whether the drawing tells more than M2's own text: anything but one atom
    /// of plain text.
    pub fn is_structured(&self) -> bool {
        !matches!(self, Self::Atom { style, .. } if style.highlight == HLGroup::Text)
    }

    /// The drawing as Unicode terminal text.
    pub fn render(&self) -> String {
        self.rendered(MathStyle::default())
    }

    /// The drawing as terminal text set in `style`.
    pub fn rendered(&self, style: MathStyle) -> String {
        if let Some(text) = self.text() {
            return text
                .lines()
                .map(str::trim_end)
                .collect::<Vec<_>>()
                .join("\n");
        }
        maths::spans(self, usize::MAX / 4, style)
            .unwrap_or_default()
            .iter()
            .map(|line| {
                line.iter()
                    .map(|span| span.content.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
