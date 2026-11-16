//! Layout implementations for M2 expression constructors, independent of domain types.

// Keep mathematical notation and interface symbols legible in their native spelling.
#![allow(clippy::non_ascii_literal)]
use super::{Diagram, GridKind, Math, MathStyle};
use crate::backend::SpecNet;
use crate::ui::theme::groups::HLGroup;

/// The sign between the factors of a product.
pub const PRODUCT_SIGN: &str = "·";
/// The sign between the factors of a tensor product.
pub const TENSOR_SIGN: &str = "⊗";
/// M2's tensor product operator.
const TENSOR_OPERATOR: &str = "**";
/// M2's tensor power operator, `M ^** n`.
const TENSOR_POWER_OPERATOR: &str = "^**";

trait ExpressionLayout: Sync {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math;
}

struct Row;
struct Sum;
struct Product(&'static str);
struct Power;
struct Subscript;
struct Divide;
struct Matrix(bool);
struct Vector;
struct Map;
struct Call;
struct Binary;
struct Minus;
struct Parenthesize;
struct Collection(char, char);
struct OptionLayout;
struct Vertical {
    numbered: bool,
}
struct Angle;
struct Aligned;
struct Under;
struct ComplexMap;
impl ExpressionLayout for ComplexMap {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        Diagram::complex_map(value, style)
            .map(|diagram| Math::Diagram(Box::new(diagram)))
            .unwrap_or_else(|| unknown(value, style))
    }
}
impl ExpressionLayout for Under {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        let [base, label] = value.children.as_slice() else {
            return unknown(value, style);
        };
        Math::Under {
            base: Box::new(Math::from_spec_net(base, style)),
            label: Box::new(Math::from_spec_net(label, style)),
        }
    }
}
impl ExpressionLayout for Vertical {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        Math::Delimited {
            left: '{',
            right: '}',
            body: Box::new(Math::Row(vec![
                Math::atom(" ", HLGroup::Canvas),
                Math::Grid {
                    rows: children(value, style)
                        .into_iter()
                        .enumerate()
                        .map(|(index, entry)| {
                            if self.numbered {
                                vec![Math::atom(format!("{index}."), HLGroup::Number), entry]
                            } else {
                                vec![entry]
                            }
                        })
                        .collect(),
                    kind: GridKind::LeftAligned,
                },
                Math::atom(" ", HLGroup::Canvas),
            ])),
        }
    }
}
impl ExpressionLayout for Aligned {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        Math::Grid {
            rows: value
                .children
                .iter()
                .map(|row| children(row, style))
                .collect(),
            kind: GridKind::Aligned,
        }
    }
}
impl ExpressionLayout for Angle {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        Math::Delimited {
            left: '⟨',
            right: '⟩',
            body: Box::new(Math::Row(vec![
                Math::atom("|", HLGroup::Punctuation),
                joined(children(value, style), ", ", HLGroup::Delimiter),
                Math::atom("|", HLGroup::Punctuation),
            ])),
        }
    }
}
/// A pair whose key is a plain name or string draws it as a key, as in
/// `Strategy => x` or a table's `"a" => 1`; a compound key keeps its drawing.
impl ExpressionLayout for OptionLayout {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        let mut sides = children(value, style);
        if let (Some(key), Some(Math::Atom { kind, .. })) =
            (value.children.first(), sides.first_mut())
        {
            if key.children.is_empty() && key.tex.is_none() {
                *kind = HLGroup::Option;
            }
        }
        joined(sides, " => ", HLGroup::Operator)
    }
}

fn children(value: &SpecNet, style: MathStyle) -> Vec<Math> {
    value
        .children
        .iter()
        .map(|child| Math::from_spec_net(child, style))
        .collect()
}
fn joined(items: Vec<Math>, separator: &str, kind: HLGroup) -> Math {
    let mut row = Vec::new();
    for item in items {
        if !row.is_empty() {
            row.push(Math::atom(separator, kind));
        }
        row.push(item);
    }
    Math::Row(row)
}
fn parenthesized(body: Math) -> Math {
    Math::Delimited {
        left: '(',
        right: ')',
        body: Box::new(body),
    }
}
/// `base` raised to a power, parenthesized unless it reads as one term.
fn power_base(base: &SpecNet, style: MathStyle) -> Math {
    let body = Math::from_spec_net(base, style);
    let compound = [
        "Sum",
        "Product",
        "TensorProduct",
        "Divide",
        "Minus",
        "Power",
    ]
    .iter()
    .any(|class| base.is(class))
        || base.is("BinaryOperation");
    if compound {
        parenthesized(body)
    } else {
        body
    }
}
fn operand(value: &SpecNet, style: MathStyle) -> Math {
    let body = Math::from_spec_net(value, style);
    if value.is("Sum") || value.is("Equation") {
        parenthesized(body)
    } else {
        body
    }
}
impl ExpressionLayout for Row {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        Math::Row(children(value, style))
    }
}
impl ExpressionLayout for Sum {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        let mut row = Vec::new();
        for (index, child) in value.children.iter().enumerate() {
            if index > 0 && !child.is("Minus") {
                row.push(Math::atom("+", HLGroup::Operator));
            }
            row.push(Math::from_spec_net(child, style));
        }
        if row.is_empty() {
            Math::atom("0", HLGroup::Number)
        } else {
            Math::Row(row)
        }
    }
}
impl ExpressionLayout for Product {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        joined(
            value.children.iter().map(|x| operand(x, style)).collect(),
            self.0,
            HLGroup::Operator,
        )
    }
}
impl ExpressionLayout for Power {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        let [base, exponent] = value.children.as_slice() else {
            return unknown(value, style);
        };
        let base_math = power_base(base, style);
        let exponent = Math::from_spec_net(exponent, style);
        let role = style.exponent_color.role(&base_math);
        Math::Power {
            base: Box::new(base_math),
            exponent: Box::new(match role {
                Some(role) => exponent.tinted(role),
                None => exponent,
            }),
        }
    }
}
impl ExpressionLayout for Subscript {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        let [base, subscript] = value.children.as_slice() else {
            return unknown(value, style);
        };
        let base = operand(base, style);
        let subscript = if subscript.is("Sequence") {
            joined(children(subscript, style), ",", HLGroup::Delimiter)
        } else {
            Math::from_spec_net(subscript, style)
        };
        let role = style.index_color.role(&base);
        Math::Index {
            base: Box::new(base),
            subscript: Box::new(match role {
                Some(role) => subscript.tinted(role),
                None => subscript,
            }),
        }
    }
}
impl ExpressionLayout for Divide {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        let [numerator, denominator] = value.children.as_slice() else {
            return unknown(value, style);
        };
        Math::Fraction {
            numerator: Box::new(Math::from_spec_net(numerator, style)),
            denominator: Box::new(Math::from_spec_net(denominator, style)),
        }
    }
}
impl ExpressionLayout for Vector {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        Math::Grid {
            rows: children(value, style)
                .into_iter()
                .map(|entry| vec![entry])
                .collect(),
            kind: GridKind::Matrix,
        }
    }
}
impl ExpressionLayout for Matrix {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        let matrix = Math::Grid {
            rows: value
                .children
                .iter()
                .filter(|row| !row.is("Option"))
                .map(|row| children(row, style))
                .collect(),
            kind: if self.0 {
                GridKind::Matrix
            } else {
                GridKind::Bare
            },
        };
        if let Some(degrees) = value
            .children
            .iter()
            .find(|child| {
                child.is("Option")
                    && child
                        .children
                        .first()
                        .is_some_and(|key| key.text.as_deref() == Some("Degrees"))
            })
            .and_then(|option| option.children.get(1))
            .and_then(|pair| pair.children.first())
        {
            if let Math::Grid { rows, .. } = matrix {
                return Math::AnnotatedMatrix {
                    rows,
                    degrees: children(degrees, style),
                };
            }
        }
        let annotations: Vec<_> = value
            .children
            .iter()
            .filter(|child| child.is("Option"))
            .map(|child| vec![Math::from_spec_net(child, style)])
            .collect();
        if annotations.is_empty() {
            return matrix;
        }
        Math::Grid {
            rows: std::iter::once(vec![matrix]).chain(annotations).collect(),
            kind: GridKind::Bare,
        }
    }
}
impl ExpressionLayout for Map {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        let [target, source, rest @ ..] = value.children.as_slice() else {
            return unknown(value, style);
        };
        let arrow = Math::atom("⟵", HLGroup::MapArrow);
        let arrow = match rest.first() {
            Some(label) => Math::Over {
                base: Box::new(arrow),
                label: Box::new(Math::from_spec_net(label, style)),
            },
            None => arrow,
        };
        // A chain of maps is one row, so a chain too wide breaks between its maps.
        let mut chain = match Math::from_spec_net(target, style) {
            Math::Row(pieces) if target.is("MapExpression") => pieces,
            composed => vec![composed],
        };
        chain.extend([arrow, Math::from_spec_net(source, style)]);
        Math::Row(chain)
    }
}
impl ExpressionLayout for Call {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        let [function, arguments] = value.children.as_slice() else {
            return unknown(value, style);
        };
        let arguments = if arguments.is("Sequence") {
            joined(children(arguments, style), ", ", HLGroup::Delimiter)
        } else {
            Math::from_spec_net(arguments, style)
        };
        Math::Row(vec![
            Math::from_spec_net(function, style),
            parenthesized(arguments),
        ])
    }
}
impl ExpressionLayout for Binary {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        let [operator, left, right] = value.children.as_slice() else {
            return unknown(value, style);
        };
        match operator.text.as_deref() {
            Some(TENSOR_POWER_OPERATOR) => {
                return Math::Power {
                    base: Box::new(power_base(left, style)),
                    exponent: Box::new(Math::Row(vec![
                        Math::atom(TENSOR_SIGN, HLGroup::Operator),
                        Math::from_spec_net(right, style),
                    ])),
                }
            }
            Some(TENSOR_OPERATOR) => {
                return joined(
                    vec![operand(left, style), operand(right, style)],
                    TENSOR_SIGN,
                    HLGroup::Operator,
                )
            }
            _ => {}
        }
        Math::Row(vec![
            operand(left, style),
            Math::atom(" ", HLGroup::Canvas),
            Math::from_spec_net(operator, style),
            Math::atom(" ", HLGroup::Canvas),
            operand(right, style),
        ])
    }
}
impl ExpressionLayout for Minus {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        Math::Row(
            std::iter::once(Math::atom("-", HLGroup::Operator))
                .chain(value.children.iter().map(|x| operand(x, style)))
                .collect(),
        )
    }
}
impl ExpressionLayout for Parenthesize {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        parenthesized(Math::Row(children(value, style)))
    }
}
impl ExpressionLayout for Collection {
    fn compose(&self, value: &SpecNet, style: MathStyle) -> Math {
        Math::Delimited {
            left: self.0,
            right: self.1,
            body: Box::new(joined(children(value, style), ", ", HLGroup::Delimiter)),
        }
    }
}

fn layout(class: &str) -> Option<&'static dyn ExpressionLayout> {
    Some(match class {
        "Sum" | "DirectSum" => &Sum,
        "Product" | "NonAssociativeProduct" => &Product(PRODUCT_SIGN),
        "TensorProduct" => &Product(TENSOR_SIGN),
        "Power" | "Superscript" => &Power,
        "Subscript" => &Subscript,
        "Divide" => &Divide,
        "MatrixExpression" => &Matrix(true),
        "VectorExpression" => &Vector,
        "MapExpression" => &Map,
        "UnderExpression" => &Under,
        "ComplexMapExpression" => &ComplexMap,
        "FunctionApplication" => &Call,
        "BinaryOperation" => &Binary,
        "Minus" => &Minus,
        "Parenthesize" => &Parenthesize,
        "RowExpression" | "Adjacent" | "Holder" | "SheafExpression" | "OneExpression"
        | "ZeroExpression" | "RingElement" => &Row,
        "List" | "VisibleList" | "MutableList" => &Collection('{', '}'),
        "Table" => &Matrix(false),
        "Option" => &OptionLayout,
        "Array" => &Collection('[', ']'),
        "Sequence" => &Collection('(', ')'),
        "AngleBarList" => &Angle,
        "AlignedExpression" => &Aligned,
        "NumberedVerticalList" => &Vertical { numbered: true },
        "HashTable" | "MutableHashTable" | "VerticalList" => &Vertical { numbered: false },
        _ => return None,
    })
}
fn unknown(value: &SpecNet, style: MathStyle) -> Math {
    Math::Row(vec![
        Math::atom(value.class(), HLGroup::Type),
        Collection('{', '}').compose(value, style),
    ])
}

fn ascii(math: Math) -> Math {
    match math {
        Math::Atom { text, kind, color } => Math::atom(
            if matches!(kind, HLGroup::Operator | HLGroup::MapArrow) {
                match text.as_str() {
                    "⟵" => "<--".into(),
                    PRODUCT_SIGN => "*".into(),
                    TENSOR_SIGN => TENSOR_OPERATOR.into(),
                    _ => text,
                }
            } else {
                text
            },
            color.unwrap_or(kind),
        ),
        Math::Over { base, label } => Math::Row(vec![
            ascii(*base),
            Math::atom("^", HLGroup::Operator),
            parenthesized(ascii(*label)),
        ]),
        Math::Under { base, label } => Math::Row(vec![
            ascii(*base),
            Math::atom("_", HLGroup::Operator),
            parenthesized(ascii(*label)),
        ]),
        Math::Linked { body, target } => Math::Linked {
            body: Box::new(ascii(*body)),
            target,
        },
        Math::Diagram(diagram) => Math::Diagram(Box::new(diagram.map(ascii))),
        Math::Row(parts) => Math::Row(parts.into_iter().map(ascii).collect()),
        Math::Power { base, exponent } => Math::Row(vec![
            ascii(*base),
            Math::atom("^", HLGroup::Operator),
            parenthesized(ascii(*exponent)),
        ]),
        Math::Index { base, subscript } => Math::Row(vec![
            ascii(*base),
            Math::atom("_", HLGroup::Operator),
            parenthesized(ascii(*subscript)),
        ]),
        Math::Fraction {
            numerator,
            denominator,
        } => Math::Row(vec![
            parenthesized(ascii(*numerator)),
            Math::atom("/", HLGroup::Operator),
            parenthesized(ascii(*denominator)),
        ]),
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

impl Math {
    /// Whether all nodes have an installed structured layout or a classified leaf.
    pub fn supports_spec_net(value: &SpecNet) -> bool {
        if value.html.is_some() {
            return false;
        }
        if value.text.is_some() {
            return value.group() != HLGroup::Text;
        }
        value.classes.iter().any(|class| layout(class).is_some())
            && value
                .children
                .iter()
                .all(|child| child.text.is_some() || Self::supports_spec_net(child))
    }

    /// Read explicit M2 expression structure and classes without parsing formatted text.
    pub fn from_spec_net(value: &SpecNet, style: MathStyle) -> Self {
        let body = if let Some(text) = &value.text {
            match &value.tex {
                Some(tex) => Self::Native {
                    text: text.clone(),
                    tex: tex.clone(),
                    kind: value.group(),
                    color: None,
                },
                None => Self::atom(text, value.group()),
            }
        } else {
            value
                .classes
                .iter()
                .find_map(|class| layout(class))
                .map_or_else(
                    || unknown(value, style),
                    |layout| layout.compose(value, style),
                )
        };
        let body = if style.display == super::MathDisplay::Ascii {
            ascii(body)
        } else {
            body
        };
        match &value.target {
            Some(target) => Self::Linked {
                body: Box::new(body),
                target: target.clone(),
            },
            None => body,
        }
    }
}
