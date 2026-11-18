//! The semantic-token vocabulary m2-ls speaks, and the highlight groups each word
//! of it is drawn with.
//!
//! A token is drawn as Neovim draws one: the group of its type, then the group of
//! each modifier over it, then the group of a modifier on that particular type,
//! as `@lsp.type.<type>`, `@lsp.mod.<modifier>` and `@lsp.typemod.<type>.<modifier>`.
//! Every type and modifier has a group of its own, so a scheme or the user's
//! `groups.toml` can style any of them; most modifier groups are empty by default.

use std::iter::once;
use std::sync::LazyLock;

use lsp_types::{SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokensLegend};
use ratatui::style::Style;
use strum::{EnumIter, EnumString, IntoEnumIterator, IntoStaticStr};
use these_macros_should_be_illegal::enum_fn;

use crate::ui::theme::groups::{Groups, HLGroup};

use TokenModifier::{
    Algebraic, Complex, Declaration, DefaultLibrary, Module, Morphism, OptionalParameter, Space,
    Structure,
};
use TokenType::{Function, Method, Property, Variable};

/// A token type, named by its variant as m2-ls sends it: `EnumMember` is
/// `enumMember`. Each draws with the group given here.
#[enum_fn(pub const group -> HLGroup)]
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, EnumString, IntoStaticStr)]
#[strum(serialize_all = "camelCase")]
pub enum TokenType {
    Type = HLGroup::Type,
    Function = HLGroup::Function,
    /// Left empty, so a plain name keeps the colour tree-sitter gives it.
    Variable = HLGroup::LspVariable,
    Parameter = HLGroup::Parameter,
    Property = HLGroup::Property,
    /// A package, or a file a `load` runs.
    Namespace = HLGroup::Namespace,
    /// A table of the interpreter's own data, such as `typicalValues`.
    Struct = HLGroup::Table,
    /// A name used as a `Symbol`: `Linear`, the `g` of `symbol g`.
    EnumMember = HLGroup::Symbol,
    Keyword = HLGroup::Keyword,
    String = HLGroup::String,
    Number = HLGroup::Number,
    Operator = HLGroup::Operator,
    Comment = HLGroup::Comment,
    /// A method function, made by `method()`.
    Method = HLGroup::Method,
    /// A quoting keyword: `symbol`, `local`, `global`.
    Modifier = HLGroup::StorageClass,
    /// A domain of an installation: the `ZZ` of `f ZZ := …`.
    TypeParameter = HLGroup::TypeParameter,
    /// Markup around documentation text: `TT`, `{\tt`.
    Macro = HLGroup::Macro,
    /// A codomain of an installation, or a documentation link constructor such as `TO`.
    Decorator = HLGroup::Attribute,
    Regexp = HLGroup::Regex,
    EscapeSequence = HLGroup::EscapeSequence,
    /// A bracket of code embedded in documentation.
    Delimiter = HLGroup::Punctuation,
    /// A comma or semicolon of code embedded in documentation.
    Punctuation = HLGroup::Delimiter,
    /// Documentation prose.
    Docs = HLGroup::Docs,
}

/// A token modifier, named by its variant as m2-ls sends it: `MemberAccess` is
/// `memberAccess`. Each draws with the group named `Lsp` before its variant, as
/// `LspMemberAccess`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, EnumString, IntoStaticStr)]
#[strum(serialize_all = "camelCase")]
pub enum TokenModifier {
    /// An option key: `Strategy` in `Strategy => Linear`.
    OptionalParameter,
    /// Where a name is bound, or what an installation installs on.
    Declaration,
    DefaultLibrary,
    /// A binding protected from reassignment.
    Readonly,
    Documentation,
    /// A ring generator, or a value living over a ring that is of no kind below.
    Algebraic,
    Ring,
    /// A module, an ideal, a coherent sheaf.
    Module,
    /// A matrix, a ring map.
    Morphism,
    /// A chain complex, a resolution.
    Complex,
    /// `Ext`, `Tor`, `HH`.
    Functor,
    /// A monoid, a poset, a Lie algebra.
    Structure,
    /// A variety, a polyhedron, a fan.
    Space,
    /// An operator selecting a part of its left operand: the `_` of `S_1`, `.`, `#`.
    MemberAccess,
    Strong,
    Emphasis,
    Monospace,
    Url,
    File,
    Directory,
    /// The target of a `[[wikilink]]`.
    Wikilink,
    /// TeX mathematics, `$…$`.
    Tex,
    /// Code written inside a string, such as a `TEST` body.
    InjectedCode,
    /// A string naming an object, such as the package of `needsPackage "Foo"`.
    InjectedName,
    /// A key a documentation page documents or a link names.
    DocumentTag,
    Example,
    Test,
    Abstract,
    Index,
    Nonsense,
    /// An indexed library binding exported as protected.
    LibraryConst,
    /// An indexed library binding exported as mutable.
    LibraryConfigParam,
}

/// Each modifier's group, read once by its name.
static MODIFIER_GROUPS: LazyLock<Vec<HLGroup>> = LazyLock::new(|| {
    TokenModifier::iter()
        .map(|modifier| {
            HLGroup::named(&format!("Lsp{modifier:?}"))
                .expect("groups.toml declares a group for every token modifier")
        })
        .collect()
});

impl TokenModifier {
    pub fn group(self) -> HLGroup {
        MODIFIER_GROUPS[self as usize]
    }

    const fn bit(self) -> u64 {
        1 << self as u64
    }
}

/// Groups for modifiers on particular types, the most specific first; only the
/// first that applies is drawn, over everything else.
const REFINEMENTS: [(&[TokenType], &[TokenModifier], HLGroup); 4] = [
    (
        &[Variable],
        &[Algebraic, Module, Morphism, Complex, Structure, Space],
        HLGroup::RingGenerator,
    ),
    (&[Property], &[OptionalParameter], HLGroup::Option),
    (&[Function, Method], &[Declaration], HLGroup::Installation),
    (&[Function, Method], &[DefaultLibrary], HLGroup::CompiledFunction),
];

/// What m2-ls says one token is: its type and modifiers, as
/// `variable` with `declaration` and `algebraic` for the `x` of `QQ[x]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SemanticClass {
    kind: TokenType,
    modifiers: u64,
}

impl SemanticClass {
    pub fn has(self, modifier: TokenModifier) -> bool {
        self.modifiers & modifier.bit() != 0
    }

    pub fn modifiers(self) -> impl Iterator<Item = TokenModifier> {
        TokenModifier::iter().filter(move |&modifier| self.has(modifier))
    }

    /// The group of a modifier on this particular type, when one applies.
    fn refinement(self) -> Option<HLGroup> {
        REFINEMENTS.iter().find_map(|&(kinds, modifiers, group)| {
            (kinds.contains(&self.kind) && modifiers.iter().any(|&modifier| self.has(modifier)))
                .then_some(group)
        })
    }

    /// The one group standing for the whole class, where text takes a single group.
    pub fn group(self) -> HLGroup {
        self.refinement().unwrap_or(self.kind.group())
    }

    /// Every group the class is drawn with, least specific first.
    pub fn groups(self) -> impl Iterator<Item = HLGroup> {
        once(self.kind.group())
            .chain(self.modifiers().map(TokenModifier::group))
            .chain(self.refinement())
    }

    pub fn style(self, groups: &Groups) -> Style {
        self.groups()
            .fold(Style::new(), |style, group| style.patch(groups.style(group)))
    }
}

/// A server's legend read into this vocabulary: words sp2 does not know are left out.
pub struct Legend {
    kinds: Vec<Option<TokenType>>,
    modifiers: Vec<Option<TokenModifier>>,
}

impl Legend {
    /// The legend sp2 announces it understands: every word above.
    pub fn understood() -> SemanticTokensLegend {
        SemanticTokensLegend {
            token_types: TokenType::iter()
                .map(|kind| SemanticTokenType::new(kind.into()))
                .collect(),
            token_modifiers: TokenModifier::iter()
                .map(|modifier| SemanticTokenModifier::new(modifier.into()))
                .collect(),
        }
    }

    pub fn class(&self, token: &SemanticToken) -> Option<SemanticClass> {
        let kind = (*self.kinds.get(token.token_type as usize)?)?;
        let modifiers = self
            .modifiers
            .iter()
            .take(u32::BITS as usize)
            .enumerate()
            .filter(|&(bit, _)| (token.token_modifiers_bitset >> bit) & 1 == 1)
            .filter_map(|(_, modifier)| *modifier)
            .fold(0, |bits, modifier| bits | modifier.bit());
        Some(SemanticClass { kind, modifiers })
    }
}

impl From<&SemanticTokensLegend> for Legend {
    fn from(legend: &SemanticTokensLegend) -> Self {
        Self {
            kinds: legend
                .token_types
                .iter()
                .map(|kind| kind.as_str().parse().ok())
                .collect(),
            modifiers: legend
                .token_modifiers
                .iter()
                .map(|modifier| modifier.as_str().parse().ok())
                .collect(),
        }
    }
}

/// Every group a semantic token can be drawn with.
pub static SEMANTIC_GROUPS: LazyLock<Vec<HLGroup>> = LazyLock::new(|| {
    TokenType::iter()
        .map(TokenType::group)
        .chain(TokenModifier::iter().map(TokenModifier::group))
        .chain(REFINEMENTS.iter().map(|&(_, _, group)| group))
        .collect()
});
