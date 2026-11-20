//! The drawing M2's Spectral package sends for a value: an expression whose
//! parts are drawings in turn, each node of one of the expression classes the
//! shell draws, every atom coloured by the highlight group its M2 class names.

use crate::ui::render::maths::{self, MathStyle, Place};
use crate::ui::theme::groups::HLGroup;
use serde::Deserialize;

/// One node of a drawing, an expression of one of the [`Shape`]s, with what any
/// node may carry.
///
/// `x_1^2` arrives as
/// `{"Shape": "Superscript", "Parts": [{"Shape": "Subscript", "Parts": [x, 1]}, 2]}`,
/// each of `x`, `1` and `2` a `Holder` atom.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "PascalCase")]
pub struct Node {
    #[serde(flatten)]
    pub shape: Shape,
    /// The colour of an atom's class.
    pub group: Option<HLGroup>,
    /// The group this node and its parts are coloured as, in place of their own.
    pub highlight: Option<HLGroup>,
    /// How many levels below this node its highlight reaches; all when absent.
    pub reach: Option<usize>,
    /// Where the node links to.
    pub target: Option<String>,
    /// An atom's own TeX.
    pub tex: Option<String>,
    /// Whether M2 sets an atom as maths.
    #[serde(default)]
    pub typeset: bool,
    /// A matrix's degrees, its rows' standing beside them.
    pub degrees: Option<Degrees>,
    /// A diagram for a layout in characters, when this one runs arrows across
    /// columns.
    pub straight: Option<Box<Node>>,
}

/// A matrix's degrees as M2 gives them, `[rows, columns]`, each a degree, a
/// list of integers, for every row or column: `[[[1], [1]], [[2]]]` for a map
/// from `R^{-2}` to `R^{-1, -1}`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
pub struct Degrees {
    pub rows: Vec<Node>,
    pub columns: Vec<Node>,
}

/// The expression classes a drawing is built from, each named as the M2 class
/// it is; Spectral's `shapes` lists the same.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "Shape", content = "Parts")]
#[non_exhaustive]
pub enum Shape {
    /// An atom: text in the colour of its class.
    Holder((String,)),
    RowExpression(Vec<Node>),
    Parenthesize((Box<Node>,)),
    Divide(Box<Node>, Box<Node>),
    Subscript(Box<Node>, Box<Node>),
    Superscript(Box<Node>, Box<Node>),
    List(Vec<Node>),
    Array(Vec<Node>),
    Sequence(Vec<Node>),
    AngleBarList(Vec<Node>),
    /// Items one under another, between braces.
    VerticalList(Vec<Node>),
    MatrixExpression(Vec<Vec<Node>>),
    /// A bare grid of cells.
    Table(Vec<Vec<Node>>),
    /// Rows whose every cell but the last sits in an aligned column, the last
    /// running on.
    AlignedExpression(Vec<Vec<Node>>),
    UnderExpression(Box<Node>, Box<Node>),
    OverExpression(Box<Node>, Box<Node>),
    /// Pieces on a grid joined by arrows, each `(from, to, label)`.
    DiagramExpression(Vec<(Place, Node)>, Vec<(Place, Place, Option<Node>)>),
    /// A hierarchy, each branch a label over its subtree.
    TreeExpression(Vec<(Node, Node)>),
}

impl Shape {
    /// A list's items with the delimiters it is fenced by, its entries between
    /// commas.
    pub fn listing(&self) -> Option<((char, char), &[Node])> {
        match self {
            Self::List(items) => Some((('{', '}'), items)),
            Self::Array(items) => Some((('[', ']'), items)),
            Self::Sequence(items) => Some((('(', ')'), items)),
            _ => None,
        }
    }
}

impl Node {
    /// An atom coloured as `group`.
    pub fn atom(text: impl Into<String>, group: HLGroup) -> Self {
        Self {
            shape: Shape::Holder((text.into(),)),
            group: Some(group),
            highlight: None,
            reach: None,
            target: None,
            tex: None,
            typeset: false,
            degrees: None,
            straight: None,
        }
    }

    /// The text of an atom.
    pub fn text(&self) -> Option<&str> {
        match &self.shape {
            Shape::Holder((text,)) => Some(text),
            _ => None,
        }
    }

    /// The colour of an atom standing alone, its highlight's over its class's.
    pub fn group(&self) -> HLGroup {
        match &self.shape {
            Shape::Holder(_) => self.highlight.or(self.group).unwrap_or(HLGroup::Text),
            _ => HLGroup::Text,
        }
    }

    /// The nodes this one is built from, in reading order.
    pub fn parts(&self) -> Vec<&Self> {
        let own = match &self.shape {
            Shape::Holder(_) => Vec::new(),
            Shape::RowExpression(items)
            | Shape::List(items)
            | Shape::Array(items)
            | Shape::Sequence(items)
            | Shape::AngleBarList(items)
            | Shape::VerticalList(items) => items.iter().collect(),
            Shape::Parenthesize((body,)) => vec![&**body],
            Shape::Divide(first, second)
            | Shape::Subscript(first, second)
            | Shape::Superscript(first, second)
            | Shape::UnderExpression(first, second)
            | Shape::OverExpression(first, second) => vec![&**first, &**second],
            Shape::MatrixExpression(rows) | Shape::Table(rows) | Shape::AlignedExpression(rows) => {
                rows.iter().flatten().collect()
            }
            Shape::DiagramExpression(pieces, arrows) => pieces
                .iter()
                .map(|(_, piece)| piece)
                .chain(arrows.iter().filter_map(|(.., label)| label.as_ref()))
                .collect(),
            Shape::TreeExpression(branches) => branches
                .iter()
                .flat_map(|(label, subtree)| [label, subtree])
                .collect(),
        };
        own.into_iter()
            .chain(
                self.degrees
                    .iter()
                    .flat_map(|degrees| degrees.rows.iter().chain(&degrees.columns)),
            )
            .chain(self.straight.as_deref())
            .collect()
    }

    /// Whether this node or any below it satisfies `test`.
    pub fn any(&self, test: &impl Fn(&Self) -> bool) -> bool {
        test(self) || self.parts().into_iter().any(|part| part.any(test))
    }
}

/// A drawing as M2 sends it: the tree, and what is sent for the value whole.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "PascalCase")]
pub struct Rod {
    #[serde(flatten)]
    pub drawing: Node,
    /// M2's own two-dimensional spelling.
    pub fallback: Option<String>,
    /// M2's copyable input spelling.
    pub code: Option<String>,
    /// A documentation page, laid out apart from any drawing.
    pub html: Option<String>,
    /// The width the page's archived examples were produced at.
    #[serde(default)]
    pub example_width: usize,
    /// Names M2 resolves as types where the page was written.
    #[serde(default)]
    pub type_names: Vec<String>,
}

impl Rod {
    /// Plain text, a single atom of M2's `Net`.
    pub fn plain(text: String) -> Self {
        Self {
            drawing: Node::atom(text, HLGroup::Text),
            fallback: None,
            code: None,
            html: None,
            example_width: 0,
            type_names: Vec::new(),
        }
    }

    /// A documentation page with its original text and archived example width.
    pub fn document(html: String, fallback: String, example_width: usize) -> Self {
        Self {
            html: Some(html),
            example_width,
            ..Self::plain(fallback)
        }
    }

    /// The text of a value drawn as a single atom.
    pub fn text(&self) -> Option<&str> {
        self.drawing.text()
    }

    /// Whether M2 asks for any part of this to be set as typeset maths.
    pub fn wants_typesetting(&self) -> bool {
        self.drawing
            .any(&|node| matches!(node.shape, Shape::Holder(_)) && node.typeset)
    }

    /// Whether any part of this links somewhere.
    pub fn has_links(&self) -> bool {
        self.drawing.any(&|node| node.target.is_some())
    }

    /// Whether the drawing tells more than M2's own text: anything but a page or
    /// one atom of no particular class.
    pub fn is_structured(&self) -> bool {
        self.html.is_none()
            && !(matches!(self.drawing.shape, Shape::Holder(_))
                && self
                    .drawing
                    .group
                    .is_none_or(|group| group == HLGroup::Text))
    }

    /// Documentation content and the width of its recorded examples.
    pub fn markup(&self) -> Option<(&str, &usize)> {
        self.html.as_deref().map(|html| (html, &self.example_width))
    }

    /// The drawing as Unicode terminal text.
    pub fn render(&self) -> String {
        if let Some(text) = self.text() {
            return text
                .lines()
                .map(str::trim_end)
                .collect::<Vec<_>>()
                .join("\n");
        }
        self.render_with(MathStyle::default())
    }

    fn render_with(&self, style: MathStyle) -> String {
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

    /// M2's own spelling, for when ASCII output is asked for.
    pub fn ascii(&self) -> String {
        self.fallback
            .clone()
            .or_else(|| self.text().map(str::to_owned))
            .unwrap_or_else(|| {
                self.render_with(MathStyle {
                    display: maths::MathDisplay::Ascii,
                    ..MathStyle::default()
                })
            })
    }
}
