//! Highlight spans and their classification.
//!
//! [`HighlightKind`] is the hub: one variant per tree-sitter capture the M2 grammar
//! emits, so the capture set is a closed, exhaustively-matched enum rather than a loose
//! list of strings — a capture that the query produces but the theme forgot to colour
//! becomes a compile error, not a silently-unstyled span. [`HighlightSpan`] pairs a byte
//! range with its kind: a plain-value "what to colour where" cached next to the source
//! and resolved to a concrete `Style` only at render. [`Highlighter`] is the producer
//! side — anything that turns source text into spans — decoupled from the render
//! consumer by the span type alone.

use std::ops::Range;

/// A tree-sitter capture the M2 grammar emits, as a closed enum. [`HighlightKind::ALL`]
/// fixes the capture-index order the highlighter is configured with, so a capture's
/// index into that slice is its identity on the wire to tree-sitter; the exhaustive
/// [`HighlightKind::capture_name`] match means a capture cannot be added without also
/// giving it a name (and, downstream, a theme colour).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HighlightKind {
    Comment,
    Error,
    Number,
    NumberFloat,
    Boolean,
    String,
    StringSpecialUrl,
    CharacterSpecial,
    Variable,
    VariableBuiltin,
    VariableParameter,
    ConstantBuiltin,
    Function,
    FunctionCall,
    FunctionBuiltin,
    Label,
    Type,
    TypeParameter,
    Property,
    Module,
    Namespace,
    Operator,
    Keyword,
    KeywordConditional,
    KeywordRepeat,
    KeywordReturn,
    KeywordException,
    KeywordModifier,
    KeywordDebug,
    KeywordOperator,
    PunctuationBracket,
    PunctuationDelimiter,
}

impl HighlightKind {
    pub const ALL: &'static [HighlightKind] = &[
        HighlightKind::Comment,
        HighlightKind::Error,
        HighlightKind::Number,
        HighlightKind::NumberFloat,
        HighlightKind::Boolean,
        HighlightKind::String,
        HighlightKind::StringSpecialUrl,
        HighlightKind::CharacterSpecial,
        HighlightKind::Variable,
        HighlightKind::VariableBuiltin,
        HighlightKind::VariableParameter,
        HighlightKind::ConstantBuiltin,
        HighlightKind::Function,
        HighlightKind::FunctionCall,
        HighlightKind::FunctionBuiltin,
        HighlightKind::Label,
        HighlightKind::Type,
        HighlightKind::TypeParameter,
        HighlightKind::Property,
        HighlightKind::Module,
        HighlightKind::Namespace,
        HighlightKind::Operator,
        HighlightKind::Keyword,
        HighlightKind::KeywordConditional,
        HighlightKind::KeywordRepeat,
        HighlightKind::KeywordReturn,
        HighlightKind::KeywordException,
        HighlightKind::KeywordModifier,
        HighlightKind::KeywordDebug,
        HighlightKind::KeywordOperator,
        HighlightKind::PunctuationBracket,
        HighlightKind::PunctuationDelimiter,
    ];

    pub fn capture_name(self) -> &'static str {
        match self {
            HighlightKind::Comment => "comment",
            HighlightKind::Error => "error",
            HighlightKind::Number => "number",
            HighlightKind::NumberFloat => "number.float",
            HighlightKind::Boolean => "boolean",
            HighlightKind::String => "string",
            HighlightKind::StringSpecialUrl => "string.special.url",
            HighlightKind::CharacterSpecial => "character.special",
            HighlightKind::Variable => "variable",
            HighlightKind::VariableBuiltin => "variable.builtin",
            HighlightKind::VariableParameter => "variable.parameter",
            HighlightKind::ConstantBuiltin => "constant.builtin",
            HighlightKind::Function => "function",
            HighlightKind::FunctionCall => "function.call",
            HighlightKind::FunctionBuiltin => "function.builtin",
            HighlightKind::Label => "label",
            HighlightKind::Type => "type",
            HighlightKind::TypeParameter => "type.parameter",
            HighlightKind::Property => "property",
            HighlightKind::Module => "module",
            HighlightKind::Namespace => "namespace",
            HighlightKind::Operator => "operator",
            HighlightKind::Keyword => "keyword",
            HighlightKind::KeywordConditional => "keyword.conditional",
            HighlightKind::KeywordRepeat => "keyword.repeat",
            HighlightKind::KeywordReturn => "keyword.return",
            HighlightKind::KeywordException => "keyword.exception",
            HighlightKind::KeywordModifier => "keyword.modifier",
            HighlightKind::KeywordDebug => "keyword.debug",
            HighlightKind::KeywordOperator => "keyword.operator",
            HighlightKind::PunctuationBracket => "punctuation.bracket",
            HighlightKind::PunctuationDelimiter => "punctuation.delimiter",
        }
    }

    pub fn from_capture_index(index: usize) -> Option<HighlightKind> {
        HighlightKind::ALL.get(index).copied()
    }
}

/// A byte range of the source and the [`HighlightKind`] to colour it — a plain value
/// stored next to its buffer (no self-reference) and resolved to a `Style` only at
/// render, when the active theme is known.
#[derive(Clone, Debug)]
pub struct HighlightSpan {
    pub range: Range<usize>,
    pub kind: HighlightKind,
}

/// The producer of [`HighlightSpan`]s: turns source text into spans. The render side
/// depends only on the spans, never on which highlighter made them — so an output- or
/// diagnostic-flavoured highlighter can slot in beside the M2 `SyntaxHighlighter`.
pub trait Highlighter {
    fn highlight(&mut self, source: &str) -> Vec<HighlightSpan>;
}
