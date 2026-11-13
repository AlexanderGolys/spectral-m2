//! Recursive formatting data produced by M2's value and type methods.

use crate::ui::render::maths::{self, MathStyle};
use crate::ui::theme::groups::HLGroup;
use serde::{Deserialize, Serialize};

/// A class-tagged expression node. Classes are transmitted nearest ancestor first.
/// Unknown classes remain data and never prevent a result from being decoded.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase", try_from = "NodeData")]
pub struct SpecNet {
    /// The concrete M2 class followed by its ancestors.
    pub classes: Vec<String>,
    /// An explicit semantic highlight class when it differs from the runtime class.
    pub highlight: Option<Vec<String>>,
    /// Ordered components; matrix rows remain separate child nodes.
    pub children: Vec<Self>,
    /// Text of an opaque leaf, absent on structural nodes.
    pub text: Option<String>,
    /// Native TeX spelling of an opaque mathematical leaf.
    pub tex: Option<String>,
    /// M2's original two-dimensional spelling.
    pub fallback: Option<String>,
    /// M2's copyable input spelling.
    pub code: Option<String>,
    /// An optional source or documentation link.
    pub target: Option<String>,
    /// Semantic documentation HTML, independent of expression layout.
    pub html: Option<String>,
    /// Width used to produce archived documentation examples.
    pub example_width: usize,
    /// Names resolved by M2 as types in this documentation page's environment.
    pub type_names: Vec<String>,
}

impl Default for SpecNet {
    fn default() -> Self {
        Self {
            classes: vec!["Thing".into()],
            highlight: None,
            children: Vec::new(),
            text: None,
            tex: None,
            fallback: None,
            code: None,
            target: None,
            html: None,
            example_width: 0,
            type_names: Vec::new(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeData {
    classes: Vec<String>,
    highlight: Option<Vec<String>>,
    children: Vec<SpecNet>,
    text: Option<String>,
    tex: Option<String>,
    fallback: Option<String>,
    code: Option<String>,
    target: Option<String>,
    html: Option<String>,
    #[serde(default)]
    example_width: usize,
    #[serde(default)]
    type_names: Vec<String>,
}

impl TryFrom<NodeData> for SpecNet {
    type Error = &'static str;

    fn try_from(node: NodeData) -> Result<Self, Self::Error> {
        if node.classes.is_empty() {
            return Err("an expression node needs its M2 class");
        }
        if node.text.is_some() && !node.children.is_empty() {
            return Err("an opaque leaf cannot also have children");
        }
        Ok(Self {
            classes: node.classes,
            highlight: node.highlight,
            children: node.children,
            text: node.text,
            tex: node.tex,
            fallback: node.fallback,
            code: node.code,
            target: node.target,
            html: node.html,
            example_width: node.example_width,
            type_names: node.type_names,
        })
    }
}

impl SpecNet {
    /// Construct an opaque component in an explicitly supplied M2 class.
    pub fn leaf(class: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            classes: vec![class.into()],
            text: Some(text.into()),
            ..Self::default()
        }
    }

    /// Compose ordered children under an M2 expression or container class.
    pub fn branch(class: impl Into<String>, children: Vec<Self>) -> Self {
        Self {
            classes: vec![class.into()],
            children,
            ..Self::default()
        }
    }

    /// Whether the node belongs to an M2 class, including through inheritance.
    pub fn is(&self, class: &str) -> bool {
        self.classes.iter().any(|ancestor| ancestor == class)
    }

    /// The concrete class name, retained even for unsupported expressions.
    pub fn class(&self) -> &str {
        self.classes.first().map_or("Thing", String::as_str)
    }

    /// The theme role supplied by the component's class ancestry.
    pub fn group(&self) -> HLGroup {
        self.highlight
            .as_ref()
            .unwrap_or(&self.classes)
            .iter()
            .find_map(|class| match class.as_str() {
                "Delimiter" => Some(HLGroup::Delimiter),
                "Punctuation" => Some(HLGroup::Punctuation),
                "Nothing" => Some(HLGroup::Builtin),
                "Boolean" => Some(HLGroup::Boolean),
                "String" => Some(HLGroup::String),
                "ZZ" | "QQ" | "RR" | "CC" | "Number" => Some(HLGroup::Number),
                "Symbol" | "RingElement" | "RawRingElement" => Some(HLGroup::Variable),
                "Function" | "ScriptedFunctor" => Some(HLGroup::Function),
                "Type" | "Ring" | "RingFamily" => Some(HLGroup::Type),
                "Package" => Some(HLGroup::Package),
                "Operator" => Some(HLGroup::Operator),
                "Keyword" => Some(HLGroup::Keyword),
                "Command" => Some(HLGroup::Macro),
                "FilePosition" => Some(HLGroup::MarkupLink),
                _ => None,
            })
            .unwrap_or(HLGroup::Text)
    }

    /// Collection layout inherited from the nearest visible container class.
    pub fn collection_kind(&self) -> Option<CollectionKind> {
        self.classes
            .iter()
            .find_map(|class| match class.as_str() {
                "List" | "VisibleList" | "MutableList" => Some(CollectionKind::List),
                "Array" => Some(CollectionKind::Array),
                "Sequence" => Some(CollectionKind::Sequence),
                "AngleBarList" => Some(CollectionKind::AngleBarList),
                _ => None,
            })
            .filter(|_| {
                self.text.is_none()
                    && !self.is("Expression")
                    && !self.is("RingElement")
                    && !self.is("VerticalList")
            })
    }

    /// Documentation content and the width of its recorded examples.
    pub fn markup(&self) -> Option<(&str, &usize)> {
        self.html.as_deref().map(|html| (html, &self.example_width))
    }

    /// Borrow an opaque value's text without building a character grid.
    pub fn plain_text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// Render the structure as Unicode terminal text without inferring semantics.
    pub fn render(&self) -> String {
        if let Some(text) = self.plain_text() {
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

    /// Preserve M2's original spelling when ASCII output is requested.
    pub fn ascii(&self) -> String {
        self.fallback
            .clone()
            .or_else(|| self.text.clone())
            .unwrap_or_else(|| {
                self.render_with(MathStyle {
                    display: maths::MathDisplay::Ascii,
                    ..MathStyle::default()
                })
            })
    }

    /// Plain text without a domain-specific drawing case.
    pub fn plain(text: String) -> Self {
        Self::leaf("Net", text)
    }

    /// A documentation page with its original text and archived example width.
    pub fn document(html: String, fallback: String, example_width: usize) -> Self {
        Self {
            html: Some(html),
            example_width,
            ..Self::plain(fallback)
        }
    }
}

/// Delimiters of M2's built-in visible collections.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub enum CollectionKind {
    /// Braces.
    List,
    /// Square brackets.
    Array,
    /// Parentheses.
    Sequence,
    /// Angle bars.
    AngleBarList,
    /// Vertically stacked entries.
    VerticalList,
}

impl CollectionKind {
    /// The plain delimiters of this collection.
    pub const fn delimiters(self) -> (&'static str, &'static str) {
        match self {
            Self::List | Self::VerticalList => ("{", "}"),
            Self::Array => ("[", "]"),
            Self::Sequence => ("(", ")"),
            Self::AngleBarList => ("<|", "|>"),
        }
    }
    /// Delimiters suitable for terminal character layout.
    pub const fn drawn_delimiters(self) -> (&'static str, &'static str) {
        match self {
            Self::AngleBarList => ("⟨|", "|⟩"),
            _ => self.delimiters(),
        }
    }
}

#[cfg(test)]
mod fixtures {
    use super::*;
    use crate::ui::render::maths::{GridKind, Math};

    fn from_math(math: Math) -> SpecNet {
        match math {
            Math::Atom { text, kind, .. } => SpecNet::leaf(
                match kind {
                    HLGroup::Number => "Number",
                    HLGroup::Variable => "Symbol",
                    HLGroup::Operator => "Operator",
                    HLGroup::Type => "Type",
                    HLGroup::Delimiter => "Delimiter",
                    HLGroup::Punctuation => "Punctuation",
                    HLGroup::Function => "Function",
                    HLGroup::Builtin => "Nothing",
                    HLGroup::Boolean => "Boolean",
                    _ => "Thing",
                },
                text,
            ),
            Math::Over { base, label } => SpecNet::branch(
                "Table",
                vec![
                    SpecNet::branch("List", vec![from_math(*label)]),
                    SpecNet::branch("List", vec![from_math(*base)]),
                ],
            ),
            Math::Under { base, label } => {
                SpecNet::branch("UnderExpression", vec![from_math(*base), from_math(*label)])
            }
            Math::Linked { body, target } => {
                let mut node = from_math(*body);
                node.target = Some(target);
                node
            }
            Math::Native {
                text,
                tex,
                kind,
                color,
            } => {
                let mut node = from_math(Math::Atom { text, kind, color });
                node.tex = Some(tex);
                node
            }
            Math::Space => SpecNet::leaf("Thing", ""),
            Math::Row(parts) => {
                SpecNet::branch("RowExpression", parts.into_iter().map(from_math).collect())
            }
            Math::Power { base, exponent } => {
                SpecNet::branch("Power", vec![from_math(*base), from_math(*exponent)])
            }
            Math::Index { base, subscript } => {
                SpecNet::branch("Subscript", vec![from_math(*base), from_math(*subscript)])
            }
            Math::Fraction {
                numerator,
                denominator,
            } => SpecNet::branch(
                "Divide",
                vec![from_math(*numerator), from_math(*denominator)],
            ),
            Math::Delimited { left, right, body } => SpecNet::branch(
                "RowExpression",
                vec![
                    SpecNet::leaf("Delimiter", left.to_string()),
                    from_math(*body),
                    SpecNet::leaf("Delimiter", right.to_string()),
                ],
            ),
            Math::AnnotatedMatrix { .. } => {
                unreachable!("TeX fixtures do not produce structured matrix metadata")
            }
            Math::Grid { rows, kind } => {
                let mut node = SpecNet::branch(
                    "MatrixExpression",
                    rows.into_iter()
                        .map(|row| {
                            SpecNet::branch("List", row.into_iter().map(from_math).collect())
                        })
                        .collect(),
                );
                if kind == GridKind::Bare {
                    node.classes = vec!["Table".into()];
                }
                node
            }
        }
    }
    impl SpecNet {
        pub fn tex_fixture(source: String, fallback: String, code: String) -> Self {
            let mut node = source
                .strip_prefix('$')
                .and_then(|x| x.strip_suffix('$'))
                .and_then(|x| Math::parse(x).ok())
                .map(from_math)
                .unwrap_or_else(|| Self::plain(fallback.clone()));
            node.fallback = Some(fallback);
            node.code = Some(code);
            node
        }
        pub fn token_fixture(text: String, role: &str) -> Self {
            Self::leaf(role, text)
        }
        pub fn collection_fixture(kind: CollectionKind, items: Vec<Self>) -> Self {
            Self::branch(format!("{kind:?}"), items)
        }
        pub fn table_fixture(rows: Vec<[Self; 2]>) -> Self {
            Self::branch(
                "HashTable",
                rows.into_iter()
                    .map(|[key, value]| Self::branch("Option", vec![key, value]))
                    .collect(),
            )
        }
        pub fn linked_fixture(mut value: Box<Self>, target: String) -> Self {
            value.target = Some(target);
            *value
        }
        pub fn location_fixture(text: String, target: String) -> Self {
            Self {
                target: Some(target),
                ..Self::leaf("FilePosition", text)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_classes_roundtrip_without_losing_structure_or_highlights() {
        let input = r#"{"classes":["Unfamiliar","Expression","BasicList","Thing"],"children":[{"classes":["String","Thing"],"children":[],"text":"a"},{"classes":["Symbol","Thing"],"children":[],"text":"a"}],"text":null}"#;
        let node: SpecNet = serde_json::from_str(input).unwrap();
        assert_eq!(node.class(), "Unfamiliar");
        assert_eq!(node.children[0].text, node.children[1].text);
        assert_eq!(node.children[0].group(), HLGroup::String);
        assert_eq!(node.children[1].group(), HLGroup::Variable);
        let decoded: SpecNet =
            serde_json::from_str(&serde_json::to_string(&node).unwrap()).unwrap();
        assert_eq!(decoded, node);
    }

    #[test]
    fn malformed_nodes_are_rejected_at_the_transport_boundary() {
        for input in [
            r#"{"classes":[],"children":[],"text":"x"}"#,
            r#"{"classes":["String"],"children":[{"classes":["Symbol"],"children":[],"text":"x"}],"text":"x"}"#,
            r#"{"classes":["Expression"],"text":null}"#,
        ] {
            assert!(serde_json::from_str::<SpecNet>(input).is_err());
        }
    }
}
