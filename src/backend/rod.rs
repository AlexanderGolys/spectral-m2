//! The drawing M2's Spectral package sends for a value: a tree of shapes laid out
//! as TeX lays out a formula, every atom coloured by a class from the drawing's
//! own table.

use crate::ui::render::maths::{self, GridKind, MathStyle, Place};
use crate::ui::theme::groups::HLGroup;
use serde::{Deserialize, Deserializer};

/// One shaped node of a drawing, with what any node may carry, its atoms coloured
/// by a class of type `C`: the colour itself once read, an index into the class
/// table M2 sends beside the drawing until then.
///
/// `x_1^2` arrives as
/// `{"Shape": "Script", "Base": {"Shape": "Script", "Base": x, "Sub": 1}, "Sup": 2}`,
/// each of `x`, `1` and `2` an atom.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "PascalCase")]
pub struct Node<C = HLGroup> {
    #[serde(flatten)]
    pub shape: Shape<C>,
    /// The class this node and its parts are coloured as, in place of their own.
    pub highlight: Option<C>,
    /// How many levels below this node its highlight reaches; all when absent.
    pub reach: Option<usize>,
    /// Where the node links to.
    pub target: Option<String>,
}

/// The layout constructs a drawing is built from.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "Shape", rename_all_fields = "PascalCase")]
#[non_exhaustive]
pub enum Shape<C = HLGroup> {
    /// Text coloured as `class`, with its own TeX when it has one; `typeset` when
    /// M2 sets it as maths.
    Atom {
        text: String,
        class: C,
        tex: Option<String>,
        #[serde(default)]
        typeset: bool,
    },
    Row {
        items: Vec<Node<C>>,
    },
    /// Items with the separator between each two, as a list's entries are.
    Join {
        items: Vec<Node<C>>,
        separator: Box<Node<C>>,
    },
    Script {
        base: Box<Node<C>>,
        sub: Option<Box<Node<C>>>,
        sup: Option<Box<Node<C>>>,
    },
    Fraction {
        numerator: Box<Node<C>>,
        denominator: Box<Node<C>>,
    },
    Fence {
        left: char,
        right: char,
        body: Box<Node<C>>,
    },
    /// Rows of cells; a matrix may carry its rows' degrees.
    Grid {
        rows: Vec<Vec<Node<C>>>,
        align: GridKind,
        degrees: Option<Vec<Node<C>>>,
    },
    Over {
        base: Box<Node<C>>,
        label: Box<Node<C>>,
    },
    Under {
        base: Box<Node<C>>,
        label: Box<Node<C>>,
    },
    /// Pieces on a grid joined by arrows, each `(from, to, label)`; `straight` is
    /// the same diagram for a layout in characters when this one runs arrows across.
    Diagram {
        pieces: Vec<(Place, Node<C>)>,
        arrows: Vec<(Place, Place, Option<Node<C>>)>,
        straight: Option<Box<Node<C>>>,
    },
    /// A hierarchy, each branch a label over its subtree.
    Tree {
        branches: Vec<(Node<C>, Node<C>)>,
    },
}

impl<C> Node<C> {
    /// The same drawing with every class, of an atom or a highlight, recoloured.
    pub fn recoloured<D>(self, recolour: &impl Fn(C) -> D) -> Node<D> {
        let node = |node: Self| node.recoloured(recolour);
        let boxed = |node: Box<Self>| Box::new(node.recoloured(recolour));
        let nodes = |nodes: Vec<Self>| nodes.into_iter().map(node).collect::<Vec<_>>();
        let shape = match self.shape {
            Shape::Atom {
                text,
                class,
                tex,
                typeset,
            } => Shape::Atom {
                text,
                class: recolour(class),
                tex,
                typeset,
            },
            Shape::Row { items } => Shape::Row {
                items: nodes(items),
            },
            Shape::Join { items, separator } => Shape::Join {
                items: nodes(items),
                separator: boxed(separator),
            },
            Shape::Script { base, sub, sup } => Shape::Script {
                base: boxed(base),
                sub: sub.map(boxed),
                sup: sup.map(boxed),
            },
            Shape::Fraction {
                numerator,
                denominator,
            } => Shape::Fraction {
                numerator: boxed(numerator),
                denominator: boxed(denominator),
            },
            Shape::Fence { left, right, body } => Shape::Fence {
                left,
                right,
                body: boxed(body),
            },
            Shape::Grid {
                rows,
                align,
                degrees,
            } => Shape::Grid {
                rows: rows.into_iter().map(nodes).collect(),
                align,
                degrees: degrees.map(nodes),
            },
            Shape::Over { base, label } => Shape::Over {
                base: boxed(base),
                label: boxed(label),
            },
            Shape::Under { base, label } => Shape::Under {
                base: boxed(base),
                label: boxed(label),
            },
            Shape::Diagram {
                pieces,
                arrows,
                straight,
            } => Shape::Diagram {
                pieces: pieces
                    .into_iter()
                    .map(|(place, piece)| (place, node(piece)))
                    .collect(),
                arrows: arrows
                    .into_iter()
                    .map(|(from, to, label)| (from, to, label.map(node)))
                    .collect(),
                straight: straight.map(boxed),
            },
            Shape::Tree { branches } => Shape::Tree {
                branches: branches
                    .into_iter()
                    .map(|(label, subtree)| (node(label), node(subtree)))
                    .collect(),
            },
        };
        Node {
            shape,
            highlight: self.highlight.map(recolour),
            reach: self.reach,
            target: self.target,
        }
    }
}

/// A drawing as M2 sends it: every atom and highlight points into the table of
/// the classes the drawing is coloured as, each sent once as its ancestry,
/// nearest first: `["ZZ", "Ring", "Type", "MutableHashTable", "HashTable", "Thing"]`.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Transmitted {
    #[serde(flatten)]
    drawing: Node<usize>,
    #[serde(default)]
    classes: Vec<Vec<String>>,
}

impl Transmitted {
    /// The colour of a class: the role of its nearest ancestor that has one.
    fn group(ancestry: &[String]) -> HLGroup {
        ancestry
            .iter()
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

    /// The drawing with each class index replaced by the colour of its class.
    fn coloured<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Node, D::Error> {
        let Self { drawing, classes } = Self::deserialize(deserializer)?;
        let groups: Vec<_> = classes.iter().map(|ancestry| Self::group(ancestry)).collect();
        Ok(drawing.recoloured(&|class| groups.get(class).copied().unwrap_or(HLGroup::Text)))
    }
}

impl Node {
    /// An atom coloured as `class`.
    pub fn atom(text: impl Into<String>, class: HLGroup) -> Self {
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

    /// The colour of an atom standing alone, its highlight's over its class's.
    pub fn group(&self) -> HLGroup {
        match &self.shape {
            Shape::Atom { class, .. } => self.highlight.unwrap_or(*class),
            _ => HLGroup::Text,
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

/// A drawing as M2 sends it: the tree, and what is sent for the value whole.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "PascalCase")]
pub struct Rod {
    #[serde(flatten, deserialize_with = "Transmitted::coloured")]
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
            && !matches!(self.drawing.shape, Shape::Atom { class: HLGroup::Text, .. })
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
