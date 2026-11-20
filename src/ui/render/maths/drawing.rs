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
        let body = match &node.shape {
            Shape::Atom {
                text,
                class,
                tex,
                typeset,
            } => {
                let kind = reaching.map_or(*class, |over| over.group);
                let typewriter = (self.typewritten && !typeset).then(|| super::typewriter(text));
                match tex.clone().or(typewriter) {
                    Some(tex) => Math::Native {
                        text: text.clone(),
                        tex,
                        kind,
                        color: None,
                    },
                    None => Math::atom(text, kind),
                }
            }
            Shape::Row { items } => Math::Row(items.iter().map(part).collect()),
            Shape::Join { items, separator } => {
                let separator = part(separator);
                Math::Row(
                    items
                        .iter()
                        .enumerate()
                        .flat_map(|(index, item)| {
                            (index > 0)
                                .then(|| separator.clone())
                                .into_iter()
                                .chain([part(item)])
                        })
                        .collect(),
                )
            }
            Shape::Script { base, sub, sup } => {
                let tinted = |script: &Node, color: super::ScriptColor, base: &Math| {
                    let script = part(script);
                    match color.role(base) {
                        Some(role) => script.tinted(role),
                        None => script,
                    }
                };
                let base = part(base);
                let base = match sub {
                    Some(sub) => Math::Index {
                        subscript: Box::new(tinted(sub, self.style.index_color, &base)),
                        base: Box::new(base),
                    },
                    None => base,
                };
                match sup {
                    Some(sup) => Math::Power {
                        exponent: Box::new(tinted(sup, self.style.exponent_color, &base)),
                        base: Box::new(base),
                    },
                    None => base,
                }
            }
            Shape::Fraction {
                numerator,
                denominator,
            } => Math::Fraction {
                numerator: boxed(numerator),
                denominator: boxed(denominator),
            },
            Shape::Fence { left, right, body } => Math::Delimited {
                left: *left,
                right: *right,
                body: boxed(body),
            },
            Shape::Grid {
                rows,
                align,
                degrees,
            } => {
                let rows = rows
                    .iter()
                    .map(|row| row.iter().map(part).collect())
                    .collect();
                match degrees {
                    Some(degrees) => Math::AnnotatedMatrix {
                        rows,
                        degrees: degrees.iter().map(part).collect(),
                    },
                    None => Math::Grid { rows, kind: *align },
                }
            }
            Shape::Over { base, label } => Math::Over {
                base: boxed(base),
                label: boxed(label),
            },
            Shape::Under { base, label } => Math::Under {
                base: boxed(base),
                label: boxed(label),
            },
            Shape::Diagram { .. } => Math::Diagram(Box::new(self.diagram(node, reaching))),
            Shape::Tree { branches } => Math::Grid {
                rows: branches
                    .iter()
                    .map(|(label, subtree)| vec![part(label), part(subtree)])
                    .collect(),
                kind: GridKind::LeftAligned,
            },
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
        let Shape::Diagram {
            pieces,
            arrows,
            straight,
        } = &node.shape
        else {
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
            straight: straight
                .as_deref()
                .map(|straight| Box::new(self.diagram(straight, reaching))),
        }
    }
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
