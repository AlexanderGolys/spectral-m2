//! The drawing M2's Spectral package sends for a value: a tree of shapes laid out
//! as TeX lays out a formula, every atom coloured by a class from the drawing's
//! own table.

use crate::ui::render::maths::{self, GridKind, MathStyle, Place};
use crate::ui::theme::groups::HLGroup;
use serde::Deserialize;

/// One shaped node of a drawing, with what any node may carry.
///
/// `x_1^2` arrives as
/// `{"Shape": "Script", "Base": {"Shape": "Script", "Base": x, "Sub": 1}, "Sup": 2}`,
/// each of `x`, `1` and `2` an atom.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "PascalCase")]
pub struct Node {
    #[serde(flatten)]
    pub shape: Shape,
    /// The class this node and its parts are coloured as, in place of their own.
    pub highlight: Option<usize>,
    /// How many levels below this node its highlight reaches; all when absent.
    pub reach: Option<usize>,
    /// Where the node links to.
    pub target: Option<String>,
}

/// The layout constructs a drawing is built from.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "Shape", rename_all_fields = "PascalCase")]
#[non_exhaustive]
pub enum Shape {
    /// Text of the class at `class` in the drawing's table, with its own TeX when
    /// it has one; `typeset` when M2 sets it as maths.
    Atom {
        text: String,
        class: usize,
        tex: Option<String>,
        #[serde(default)]
        typeset: bool,
    },
    Row {
        items: Vec<Node>,
    },
    /// Items with the separator between each two, as a list's entries are.
    Join {
        items: Vec<Node>,
        separator: Box<Node>,
    },
    Script {
        base: Box<Node>,
        sub: Option<Box<Node>>,
        sup: Option<Box<Node>>,
    },
    Fraction {
        numerator: Box<Node>,
        denominator: Box<Node>,
    },
    Fence {
        left: char,
        right: char,
        body: Box<Node>,
    },
    /// Rows of cells; a matrix may carry its rows' degrees.
    Grid {
        rows: Vec<Vec<Node>>,
        align: GridKind,
        degrees: Option<Vec<Node>>,
    },
    Over {
        base: Box<Node>,
        label: Box<Node>,
    },
    Under {
        base: Box<Node>,
        label: Box<Node>,
    },
    /// Pieces on a grid joined by arrows, each `(from, to, label)`; `straight` is
    /// the same diagram for a layout in characters when this one runs arrows across.
    Diagram {
        pieces: Vec<(Place, Node)>,
        arrows: Vec<(Place, Place, Option<Node>)>,
        straight: Option<Box<Node>>,
    },
    /// A hierarchy, each branch a label over its subtree.
    Tree {
        branches: Vec<(Node, Node)>,
    },
}

impl Node {
    /// An atom of the class at `class`.
    pub fn atom(text: impl Into<String>, class: usize) -> Self {
        Self {
            shape: Shape::Atom {
                text: text.into(),
                class,
                tex: None,
                typeset: false,
            },
            highlight: None,
            reach: None,
            target: None,
        }
    }

    /// The text of an atom.
    pub fn text(&self) -> Option<&str> {
        match &self.shape {
            Shape::Atom { text, .. } => Some(text),
            _ => None,
        }
    }

    /// The nodes this one is built from, in reading order.
    pub fn parts(&self) -> Vec<&Self> {
        match &self.shape {
            Shape::Atom { .. } => Vec::new(),
            Shape::Row { items } => items.iter().collect(),
            Shape::Join { items, separator } => items.iter().chain([&**separator]).collect(),
            Shape::Script { base, sub, sup } => [Some(base), sub.as_ref(), sup.as_ref()]
                .into_iter()
                .flatten()
                .map(|part| &**part)
                .collect(),
            Shape::Fraction {
                numerator,
                denominator,
            } => vec![numerator, denominator],
            Shape::Fence { body, .. } => vec![body],
            Shape::Grid { rows, degrees, .. } => rows
                .iter()
                .flatten()
                .chain(degrees.iter().flatten())
                .collect(),
            Shape::Over { base, label } | Shape::Under { base, label } => vec![base, label],
            Shape::Diagram {
                pieces,
                arrows,
                straight,
            } => pieces
                .iter()
                .map(|(_, piece)| piece)
                .chain(arrows.iter().filter_map(|(.., label)| label.as_ref()))
                .chain(straight.as_deref())
                .collect(),
            Shape::Tree { branches } => branches
                .iter()
                .flat_map(|(label, subtree)| [label, subtree])
                .collect(),
        }
    }

    /// Whether this node or any below it satisfies `test`.
    pub fn any(&self, test: &impl Fn(&Self) -> bool) -> bool {
        test(self) || self.parts().into_iter().any(|part| part.any(test))
    }
}

/// A drawing as M2 sends it: the tree, the classes its atoms and highlights point
/// into, each as its ancestry nearest first, and what is sent for the value whole.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "PascalCase")]
pub struct Rod {
    #[serde(flatten)]
    pub drawing: Node,
    #[serde(default)]
    pub classes: Vec<Vec<String>>,
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
            drawing: Node::atom(text, 0),
            classes: vec![vec!["Net".into()]],
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

    /// The colour of the class at `class` in the table: the role of the nearest
    /// ancestor that has one.
    pub fn group(&self, class: usize) -> HLGroup {
        self.classes
            .get(class)
            .into_iter()
            .flatten()
            .find_map(|name| {
                Some(match name.as_str() {
                    "Delimiter" => HLGroup::Delimiter,
                    "Punctuation" => HLGroup::Punctuation,
                    "Operator" => HLGroup::Operator,
                    "MapArrow" => HLGroup::MapArrow,
                    "Canvas" => HLGroup::Canvas,
                    "Nothing" => HLGroup::Builtin,
                    "Boolean" => HLGroup::Boolean,
                    "String" => HLGroup::String,
                    "Number" => HLGroup::Number,
                    "Symbol" | "RingElement" | "RawRingElement" | "Variety" => HLGroup::Variable,
                    "Function" | "ScriptedFunctor" => HLGroup::Function,
                    "Type" | "Ring" | "RingFamily" => HLGroup::Type,
                    "Package" => HLGroup::Package,
                    "Option" => HLGroup::Option,
                    "Keyword" => HLGroup::Keyword,
                    "Command" => HLGroup::Macro,
                    "FilePosition" => HLGroup::MarkupLink,
                    _ => return None,
                })
            })
            .unwrap_or(HLGroup::Text)
    }

    /// The text of a value drawn as a single atom.
    pub fn text(&self) -> Option<&str> {
        self.drawing.text()
    }

    /// Whether M2 asks for any part of this to be set as typeset maths.
    pub fn wants_typesetting(&self) -> bool {
        self.drawing
            .any(&|node| matches!(node.shape, Shape::Atom { typeset: true, .. }))
    }

    /// Whether any part of this links somewhere.
    pub fn has_links(&self) -> bool {
        self.drawing.any(&|node| node.target.is_some())
    }

    /// Whether the drawing tells more than M2's own text: anything but a page or
    /// one atom of no particular class.
    pub fn is_structured(&self) -> bool {
        self.html.is_none()
            && !matches!(&self.drawing.shape, Shape::Atom { class, .. } if self.group(*class) == HLGroup::Text)
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
