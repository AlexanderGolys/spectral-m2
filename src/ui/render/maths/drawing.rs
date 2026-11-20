//! Reading a drawing M2 sent into maths: each shape into its construct, each atom
//! in the colour of its class, or of the highlight over it.

// Keep mathematical notation and interface symbols legible in their native spelling.
#![allow(clippy::non_ascii_literal)]
use super::diagram::Arrow;
use super::{Diagram, GridKind, Math, MathDisplay, MathStyle};
use crate::backend::rod::{Node, Rod, Shape};
use crate::ui::theme::groups::HLGroup;

/// The sign between the factors of a product.
pub const PRODUCT_SIGN: &str = "·";
/// The sign between the factors of a tensor product.
pub const TENSOR_SIGN: &str = "⊗";
/// The sign between the summands of a direct sum.
pub const DIRECT_SUM_SIGN: &str = "⊕";
/// What stands between a list's entries, as M2 spells it.
const LIST_SEPARATOR: &str = ", ";
/// What stands inside an angle-bar list's brackets, as `<|a, b|>` spells it.
const ANGLE_BAR: &str = "|";
/// The signs a drawing sets, each with its spelling in M2's own syntax, which a
/// layout in ASCII shows instead.
const ASCII_SPELLINGS: [(&str, &str); 4] = [
    ("⟵", "<--"),
    (PRODUCT_SIGN, "*"),
    (TENSOR_SIGN, "**"),
    (DIRECT_SUM_SIGN, "++"),
];

/// A highlight reaching down from the node that set it: its colour, and how many
/// levels further it reaches, without end when `None`.
#[derive(Clone, Copy)]
struct Reaching {
    group: HLGroup,
    levels: Option<usize>,
}

impl Reaching {
    fn below(self) -> Option<Self> {
        match self.levels {
            Some(0) => None,
            levels => Some(Self {
                levels: levels.map(|levels| levels - 1),
                ..self
            }),
        }
    }
}

/// How a drawing is read into maths.
pub struct Reading<'a> {
    value: &'a Rod,
    style: MathStyle,
    /// Whether an atom M2 did not mark typeset is spelled in a typewriter face, as
    /// a picture sets what only sits beside its maths.
    typewritten: bool,
}

impl<'a> Reading<'a> {
    pub fn new(value: &'a Rod, style: MathStyle) -> Self {
        Self {
            value,
            style,
            typewritten: false,
        }
    }

    pub fn typewritten(self) -> Self {
        Self {
            typewritten: true,
            ..self
        }
    }

    /// The whole drawing.
    pub fn math(&self) -> Math {
        self.part(&self.value.drawing)
    }

    /// One part of the drawing, as if it stood alone.
    pub fn part(&self, node: &Node) -> Math {
        let math = self.node(node, None);
        match self.style.display {
            MathDisplay::Ascii => ascii(math),
            _ => math,
        }
    }

    fn node(&self, node: &Node, reaching: Option<Reaching>) -> Math {
        let reaching = node
            .highlight
            .map(|group| Reaching {
                group,
                levels: node.reach,
            })
            .or(reaching);
        let part = |part: &Node| self.node(part, reaching.and_then(Reaching::below));
        let boxed = |node: &Node| Box::new(part(node));
        let rows = |rows: &[Vec<Node>]| -> Vec<Vec<Math>> {
            rows.iter()
                .map(|row| row.iter().map(part).collect())
                .collect()
        };
        let tinted = |script: &Node, color: super::ScriptColor, base: &Math| {
            let script = part(script);
            match color.role(base) {
                Some(role) => script.tinted(role),
                None => script,
            }
        };
        let body = match (&node.shape, node.shape.listing()) {
            (_, Some(((left, right), items))) => Math::Delimited {
                left,
                right,
                body: Box::new(Math::Row(listed(items.iter().map(part)))),
            },
            (Shape::Holder((text,)), _) => {
                let kind = reaching.map_or(node.group(), |over| over.group);
                let typewriter =
                    (self.typewritten && !node.typeset).then(|| super::typewriter(text));
                match node.tex.clone().or(typewriter) {
                    Some(tex) => Math::Native {
                        text: text.clone(),
                        tex,
                        kind,
                        color: None,
                    },
                    None => Math::atom(text, kind),
                }
            }
            (Shape::RowExpression(items), _) => Math::Row(items.iter().map(part).collect()),
            (Shape::Parenthesize((body,)), _) => parenthesized(part(body)),
            (Shape::Subscript(base, sub), _) => {
                let base = part(base);
                Math::Index {
                    subscript: Box::new(tinted(sub, self.style.index_color, &base)),
                    base: Box::new(base),
                }
            }
            (Shape::Superscript(base, sup), _) => {
                let base = part(base);
                Math::Power {
                    exponent: Box::new(tinted(sup, self.style.exponent_color, &base)),
                    base: Box::new(base),
                }
            }
            (Shape::Divide(numerator, denominator), _) => Math::Fraction {
                numerator: boxed(numerator),
                denominator: boxed(denominator),
            },
            (Shape::AngleBarList(items), _) => {
                let bar = || Math::atom(ANGLE_BAR, HLGroup::Punctuation);
                Math::Delimited {
                    left: '⟨',
                    right: '⟩',
                    body: Box::new(Math::Row(
                        [bar()]
                            .into_iter()
                            .chain(listed(items.iter().map(part)))
                            .chain([bar()])
                            .collect(),
                    )),
                }
            }
            (Shape::VerticalList(items), _) => {
                let margin = || Math::atom(" ", HLGroup::Canvas);
                Math::Delimited {
                    left: '{',
                    right: '}',
                    body: Box::new(Math::Row(vec![
                        margin(),
                        Math::Grid {
                            rows: items.iter().map(|item| vec![part(item)]).collect(),
                            kind: GridKind::LeftAligned,
                        },
                        margin(),
                    ])),
                }
            }
            (Shape::MatrixExpression(cells), _) => match &node.degrees {
                Some(degrees) => Math::AnnotatedMatrix {
                    rows: rows(cells),
                    degrees: degrees.iter().map(part).collect(),
                },
                None => Math::Grid {
                    rows: rows(cells),
                    kind: GridKind::Matrix,
                },
            },
            (Shape::Table(cells), _) => Math::Grid {
                rows: rows(cells),
                kind: GridKind::Bare,
            },
            (Shape::AlignedExpression(cells), _) => Math::Grid {
                rows: rows(cells),
                kind: GridKind::Aligned,
            },
            (Shape::OverExpression(base, label), _) => Math::Over {
                base: boxed(base),
                label: boxed(label),
            },
            (Shape::UnderExpression(base, label), _) => Math::Under {
                base: boxed(base),
                label: boxed(label),
            },
            (Shape::DiagramExpression(..), _) => {
                Math::Diagram(Box::new(self.diagram(node, reaching)))
            }
            (Shape::TreeExpression(branches), _) => Math::Grid {
                rows: branches
                    .iter()
                    .map(|(label, subtree)| vec![part(label), part(subtree)])
                    .collect(),
                kind: GridKind::LeftAligned,
            },
            (Shape::List(_) | Shape::Array(_) | Shape::Sequence(_), None) => {
                unreachable!("every list has a listing")
            }
        };
        match &node.target {
            Some(target) => Math::Linked {
                body: Box::new(body),
                target: target.clone(),
            },
            None => body,
        }
    }

    fn diagram(&self, node: &Node, reaching: Option<Reaching>) -> Diagram {
        let part = |part: &Node| self.node(part, reaching.and_then(Reaching::below));
        let Shape::DiagramExpression(pieces, arrows) = &node.shape else {
            unreachable!("only a diagram node is read as a diagram")
        };
        Diagram {
            pieces: pieces
                .iter()
                .map(|(place, piece)| (*place, part(piece)))
                .collect(),
            arrows: arrows
                .iter()
                .map(|(from, to, label)| Arrow {
                    from: *from,
                    to: *to,
                    label: label.as_ref().map(part),
                })
                .collect(),
            straight: node
                .straight
                .as_deref()
                .map(|straight| Box::new(self.diagram(straight, reaching))),
        }
    }
}

/// A list's entries with a comma between each two.
fn listed(entries: impl Iterator<Item = Math>) -> Vec<Math> {
    let comma = Math::atom(LIST_SEPARATOR, HLGroup::Delimiter);
    entries
        .enumerate()
        .flat_map(|(index, entry)| {
            (index > 0)
                .then(|| comma.clone())
                .into_iter()
                .chain([entry])
        })
        .collect()
}

fn parenthesized(body: Math) -> Math {
    Math::Delimited {
        left: '(',
        right: ')',
        body: Box::new(body),
    }
}

/// Maths spelled as M2's own syntax spells it: signs by their ASCII spellings,
/// scripts and fractions on the line with their operators.
fn ascii(math: Math) -> Math {
    let operator = |text: &str| Math::atom(text, HLGroup::Operator);
    let infix = |left: Math, sign: &str, right: Math| Math::Row(vec![left, operator(sign), right]);
    match math {
        Math::Atom { text, kind, color } => Math::atom(
            match kind {
                HLGroup::Operator | HLGroup::MapArrow => ASCII_SPELLINGS
                    .iter()
                    .find(|(sign, _)| *sign == text)
                    .map_or(text, |(_, spelling)| (*spelling).into()),
                _ => text,
            },
            color.unwrap_or(kind),
        ),
        Math::Over { base, label } => infix(ascii(*base), "^", parenthesized(ascii(*label))),
        Math::Under { base, label } => infix(ascii(*base), "_", parenthesized(ascii(*label))),
        Math::Power { base, exponent } => infix(ascii(*base), "^", parenthesized(ascii(*exponent))),
        Math::Index { base, subscript } => {
            infix(ascii(*base), "_", parenthesized(ascii(*subscript)))
        }
        Math::Fraction {
            numerator,
            denominator,
        } => infix(
            parenthesized(ascii(*numerator)),
            "/",
            parenthesized(ascii(*denominator)),
        ),
        Math::Linked { body, target } => Math::Linked {
            body: Box::new(ascii(*body)),
            target,
        },
        Math::Diagram(diagram) => Math::Diagram(Box::new(diagram.map(ascii))),
        Math::Row(parts) => Math::Row(parts.into_iter().map(ascii).collect()),
        Math::Delimited { left, right, body } => Math::Delimited {
            left,
            right,
            body: Box::new(ascii(*body)),
        },
        Math::AnnotatedMatrix { rows, degrees } => Math::AnnotatedMatrix {
            rows: rows
                .into_iter()
                .map(|row| row.into_iter().map(ascii).collect())
                .collect(),
            degrees: degrees.into_iter().map(ascii).collect(),
        },
        Math::Grid { rows, kind } => Math::Grid {
            rows: rows
                .into_iter()
                .map(|row| row.into_iter().map(ascii).collect())
                .collect(),
            kind,
        },
        Math::Native {
            text, kind, color, ..
        } => Math::atom(text, color.unwrap_or(kind)),
        Math::Space => Math::Space,
    }
}
