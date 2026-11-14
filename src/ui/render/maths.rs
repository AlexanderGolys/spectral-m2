//! Structured expression layout and explicit TeX rendering for documentation.

mod diagram;
mod expression;
pub use diagram::{Course, Diagram};
pub use expression::{PRODUCT_SIGN, TENSOR_SIGN};

use super::markup;
use crate::backend::{CollectionKind, SpecNet, OPTION_ARROW};
use crate::ui::block::TextSpan;
use crate::ui::config::UI;
use crate::ui::cycle::cyclic;
use crate::ui::graphics;
use crate::ui::present::{wrap, SpanLine};
use crate::ui::theme::groups::HLGroup;
use crate::ui::wrap::WordWrap;
use serde::{Deserialize, Serialize};
use std::ops::Range;
use std::sync::Arc;
use these_macros_should_be_illegal::*;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A piece of parsed maths, tagged with the role that decides its colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Math {
    /// One indivisible run of text: a numeral, a name, an operator, a delimiter.
    Atom {
        text: String,
        kind: HLGroup,
        color: Option<HLGroup>,
    },
    /// An opaque mathematical leaf whose class and native TeX are supplied by M2.
    Native {
        text: String,
        tex: String,
        kind: HLGroup,
        color: Option<HLGroup>,
    },
    /// A component whose rendered cells open a supplied target.
    Linked { body: Box<Math>, target: String },
    /// A label aligned above an expression without changing its baseline.
    Over { base: Box<Math>, label: Box<Math> },
    /// A label centered beneath a base, retaining the base's baseline.
    Under { base: Box<Math>, label: Box<Math> },
    /// The thin space M2 sets between factors. Factors are juxtaposed, so it takes no
    /// column; it only marks where a long line may break.
    Space,
    /// Pieces set side by side, as the terms of a polynomial are.
    Row(Vec<Math>),
    /// A base with something raised above it.
    Power {
        base: Box<Math>,
        exponent: Box<Math>,
    },
    /// A base with something lowered below it.
    Index {
        base: Box<Math>,
        subscript: Box<Math>,
    },
    /// One expression over another, separated by a rule.
    Fraction {
        numerator: Box<Math>,
        denominator: Box<Math>,
    },
    /// A body inside delimiters that grow to its height.
    Delimited {
        left: char,
        right: char,
        body: Box<Math>,
    },
    /// A matrix with degree labels aligned outside its left delimiter.
    AnnotatedMatrix {
        rows: Vec<Vec<Math>>,
        degrees: Vec<Math>,
    },
    /// Rows of aligned entries: a matrix, or the branches of a `cases`.
    Grid {
        rows: Vec<Vec<Math>>,
        kind: GridKind,
    },
    /// Pieces on a grid joined by arrows, as a map of complexes is drawn.
    Diagram(Box<Diagram>),
}

/// What a grid's surrounding delimiters should be.
///
/// Following LaTeX rather than appearance: `matrix` and `array` are *unfenced* grids,
/// and M2 wraps them in an explicit `\left( … \right)` of its own. Supplying fences
/// here as well is what drew every matrix inside two sets of brackets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridKind {
    /// Aligned columns and nothing around them; the source says what fences them.
    Bare,
    /// Unfenced columns with each entry aligned to its left edge.
    LeftAligned,
    /// Left-aligned columns before each row's last entry, which runs on: given too
    /// little width it wraps beneath where it started, as a long value beside its key.
    Aligned,
    /// Entries the environment itself parenthesises, as `pmatrix` does.
    Matrix,
    /// Branches after a single opening brace.
    Cases,
}

/// Why a TeX source could not be parsed.
///
/// Carrying the reason rather than a bare `None` is what lets a fallback be reported
/// rather than silently substituted, so an unhandled construct can be found and added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsupportedTeX(pub String);

cyclic! {
    /// How a fraction is set, as chosen in settings.
    #[discriminated_str(label)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum FractionStyle {
        /// Numerator, rule, denominator: three rows, as M2 sets one itself.
        #[default]
        Stacked = "stacked",
        /// The numerator over its denominator, the rule an underline on the numerator's
        /// bottom row: two rows.
        Underlined = "underlined",
        /// `a/b` along one row, a compound part parenthesised.
        Inline = "inline",
    }
}

cyclic! {
    /// What returned maths is drawn with, as chosen in settings. Every choice is
    /// offered whether or not this terminal can draw it, so one it cannot is still
    /// seen to exist.
    #[discriminated_str(label)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    #[non_exhaustive]
    pub enum MathDisplay {
        /// Characters laid out in two dimensions: stacked fractions, tall brackets.
        #[default]
        Unicode = "unicode",
        /// Macaulay2's own plain-text spelling.
        Ascii = "ascii",
        /// Typeset pictures, on a terminal that shows them.
        #[serde(alias = "ratex", alias = "latex")]
        Typeset = "ratex",
    }
}

impl MathDisplay {
    /// Whether this terminal can draw maths this way.
    pub fn available(self) -> bool {
        self != Self::Typeset || graphics::available()
    }
}

cyclic! {
    /// Whether returned maths takes a colour per role or one colour throughout.
    #[discriminated_str(label)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    #[non_exhaustive]
    pub enum MathInk {
        /// Every number, name and operator in the colour of its role.
        #[default]
        Syntax = "syntax",
        /// The value colour throughout, the type colour for a type.
        Monochrome = "monochrome",
    }
}

cyclic! {
    /// The colour of an index or an exponent, apart from its delimiters and
    /// punctuation, which keep their own.
    #[discriminated_str(label)]
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    #[non_exhaustive]
    pub enum ScriptColor {
        /// Each piece in the colour of its own role.
        #[default]
        Own = "original",
        /// The colour of the symbol the script is attached to.
        Symbol = "symbol",
        /// The operator colour.
        Operator = "operator",
    }
}

impl ScriptColor {
    /// The role `script`'s pieces take on `base`, or `None` when they keep their own.
    fn role(self, base: &Math) -> Option<HLGroup> {
        match self {
            Self::Own => None,
            Self::Symbol => Some(base.leading_role().unwrap_or(HLGroup::Variable)),
            Self::Operator => Some(HLGroup::Operator),
        }
    }
}

/// How returned maths is set, as chosen in settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MathStyle {
    pub ink: MathInk,
    /// Preferred typeset size as a percentage of the terminal text size.
    pub scale_percent: u16,

    /// The colour of subscripts.
    pub index_color: ScriptColor,
    /// The colour of superscripts.
    pub exponent_color: ScriptColor,
    /// Whether a value is drawn as the text Macaulay2 printed for it, not rendered.
    pub original_text: bool,
    pub fractions: FractionStyle,
    /// Whether a ring such as `\mathbb Z` is drawn with its blackboard glyph `ℤ`
    /// rather than the name `ZZ` it has in code.
    pub blackboard: bool,
    pub display: MathDisplay,
    /// Distance between the rows of a typeset matrix, as a percentage of TeX's own.
    pub matrix_row_spacing_percent: u16,
}

impl Default for MathStyle {
    fn default() -> Self {
        Self {
            ink: MathInk::default(),
            scale_percent: UI.math_size_percent,
            index_color: ScriptColor::default(),
            exponent_color: ScriptColor::default(),
            original_text: false,
            fractions: FractionStyle::default(),
            blackboard: false,
            display: MathDisplay::default(),
            matrix_row_spacing_percent: UI.matrix_row_spacing_percent,
        }
    }
}

impl From<FractionStyle> for MathStyle {
    fn from(fractions: FractionStyle) -> Self {
        Self {
            fractions,
            ..Self::default()
        }
    }
}
/// Render an M2 value as coloured rows no wider than `width`, or refuse and name what
/// stopped it.
///
/// TeX is coloured from its parsed structure, a container from its entries, and a
/// token by its role. A value M2 drew as plain text carries nothing to colour, and
/// the caller's flat path renders it unchanged.
pub fn spans(
    value: &SpecNet,
    width: usize,
    style: impl Into<MathStyle>,
) -> Result<Vec<SpanLine>, UnsupportedTeX> {
    let style = style.into();
    let mut rows = role_spans(value, width, style)?;
    if style.ink == MathInk::Monochrome {
        let ink = HLGroup::Value;
        for span in rows.iter_mut().flatten() {
            if span.kind != HLGroup::Canvas {
                span.kind = ink;
            }
        }
    }
    Ok(rows)
}

/// [`spans`], each span in the colour of its role.
fn role_spans(
    value: &SpecNet,
    width: usize,
    style: MathStyle,
) -> Result<Vec<SpanLine>, UnsupportedTeX> {
    if let Some(html) = &value.html {
        return markup::spans(html, width)
            .ok_or_else(|| UnsupportedTeX("markup that does not read as HTML".into()));
    }
    let mut rows = if let Some(text) = &value.text {
        prose(text, value.group(), width)
    } else if let Some(kind) = value.collection_kind() {
        MathBlock::fitted(
            MathBlock::listed(kind, &value.children, width, style),
            width,
        )
    } else {
        Math::from_spec_net(value, style).wrapped(width, style.fractions)
    };
    if let Some(target) = &value.target {
        link(rows.iter_mut().flatten(), target);
    }
    Ok(rows)
}

/// Make every one of `spans` link to `target`.
fn link<'a>(spans: impl IntoIterator<Item = &'a mut TextSpan>, target: &str) {
    let target: Arc<str> = target.into();
    for span in spans {
        span.target = Some(target.clone());
    }
}

/// Text that can run long, so it breaks at words, in `group`'s colour.
fn prose(text: &str, group: HLGroup, width: usize) -> Vec<SpanLine> {
    let width = u16::try_from(width).unwrap_or(u16::MAX);
    text.lines()
        .flat_map(|line| wrap(&[TextSpan::new(line, group)], width, &WordWrap))
        .collect()
}

/// A value's type annotation as rows no wider than `width`, its pieces set side by
/// side in the type colour.
pub fn annotation(parts: &[SpecNet], width: usize, style: impl Into<MathStyle>) -> Vec<SpanLine> {
    let style = style.into();
    let line = MathBlock::row(parts.iter().map(|part| MathBlock::value(part, style)));
    let mut rows = MathBlock::fitted(vec![line], width);
    for span in rows.iter_mut().flatten() {
        if !matches!(
            span.kind,
            HLGroup::Canvas | HLGroup::MapArrow | HLGroup::Punctuation | HLGroup::Delimiter
        ) {
            span.kind = HLGroup::Type;
            span.mark = None;
        }
    }
    rows
}

/// The TeX between a pair of `$` set on a single row, as running text sets maths:
/// fractions along the row. `None` when this renderer does not read it, or it needs
/// more rows than one -- a matrix cannot sit inside a line of prose.
pub fn inline(tex: &str, blackboard: bool) -> Option<SpanLine> {
    let style = MathStyle {
        blackboard,
        ..MathStyle::default()
    };
    let rows = Math::parse_with(tex, style)
        .ok()?
        .layout(FractionStyle::Inline)
        .into_rows();
    let [row] = <[SpanLine; 1]>::try_from(rows).ok()?;
    Some(row)
}

/// `text` as TeX set in a typewriter font, escaped so it reads back as written.
pub fn typewriter(text: &str) -> String {
    let mut tex = String::from(r"\texttt{");
    for character in text.chars() {
        match TEXT_SYMBOLS
            .iter()
            .find(|(_, symbol)| symbol.starts_with(character))
        {
            Some((command, _)) => {
                tex.push('\\');
                tex.push_str(command);
                tex.push_str("{}");
            }
            None if TEXT_ESCAPED.contains(character) => {
                tex.push('\\');
                tex.push(character);
            }
            None => tex.push(character),
        }
    }
    tex.push('}');
    tex
}

/// A cursor over TeX source, retaining the source ranges of semantic atoms.
struct Input<'a> {
    source: &'a str,
    at: usize,
    style: MathStyle,
    highlights: Vec<(Range<usize>, HLGroup)>,
}

impl<'a> Input<'a> {
    fn new(source: &'a str, style: MathStyle) -> Self {
        Self {
            source: source.trim(),
            at: 0,
            style,
            highlights: Vec::new(),
        }
    }

    fn rest(&self) -> &'a str {
        &self.source[self.at..]
    }

    /// Step over anything that occupies no place in the tree.
    ///
    /// TeX's spacing commands adjust presentation and carry no structure, so they are
    /// consumed here rather than parsed. Treating them as nodes would make `\!` before
    /// a closing fence look like an operator with nothing to apply to.
    fn skip_trivia(&mut self) {
        loop {
            match self.rest().chars().next() {
                // M2 breaks matrix rows across lines; whitespace never structures.
                Some(space) if space.is_whitespace() => {
                    self.at += space.len_utf8();
                    continue;
                }
                _ => {}
            }
            if self.eat(PHANTOM) {
                self.skip_group();
                continue;
            }
            match SPACING
                .iter()
                .find(|space| self.rest().starts_with(**space))
            {
                Some(space) => self.at += space.len(),
                None => return,
            }
        }
    }

    /// The text of one braced group in text mode: its spaces kept, inner braces
    /// dropped, and escaped characters and named symbols spelled out.
    fn text_group(&mut self) -> Result<String, UnsupportedTeX> {
        self.at += self.rest().len() - self.rest().trim_start().len();
        if !self.eat("{") {
            return Err(UnsupportedTeX("text without a braced argument".into()));
        }
        let mut text = String::new();
        let mut depth = 0usize;
        while let Some(next) = self.rest().chars().next() {
            self.at += next.len_utf8();
            match next {
                '{' => depth += 1,
                '}' if depth == 0 => return Ok(text),
                '}' => depth -= 1,
                '~' => text.push(' '),
                '\\' => match self.rest().chars().next() {
                    Some(escaped) if !escaped.is_ascii_alphabetic() => {
                        self.at += escaped.len_utf8();
                        text.push(escaped);
                    }
                    _ => {
                        let name = self.name();
                        let (_, symbol) = TEXT_SYMBOLS
                            .iter()
                            .find(|(command, _)| *command == name)
                            .ok_or_else(|| UnsupportedTeX(format!("\\{name} in text")))?;
                        text.push_str(symbol);
                    }
                },
                character => text.push(character),
            }
        }
        Err(UnsupportedTeX("unclosed text".into()))
    }

    /// Step over one braced group, nested braces included, and whatever whitespace
    /// precedes it. M2 pads matrix rows with phantoms that only reserve height in a
    /// typeset page, so the text they hold is never drawn.
    fn skip_group(&mut self) {
        self.at += self.rest().len() - self.rest().trim_start().len();
        if !self.eat("{") {
            return;
        }
        let mut depth = 1;
        while depth > 0 {
            let Some(next) = self.rest().chars().next() else {
                return;
            };
            self.at += next.len_utf8();
            depth += i32::from(next == '{') - i32::from(next == '}');
        }
    }

    /// Consume `token` if it is next.
    fn eat(&mut self, token: &str) -> bool {
        if self.rest().starts_with(token) {
            self.at += token.len();
            return true;
        }
        false
    }

    /// Read pieces until one of `stops` is next, or the input ends.
    fn expression(&mut self, stops: &[&str]) -> Result<Math, UnsupportedTeX> {
        let mut pieces = Vec::new();
        loop {
            self.skip_trivia();
            if self.rest().is_empty() || stops.iter().any(|stop| self.rest().starts_with(stop)) {
                break;
            }
            pieces.push(self.term()?);
        }
        Ok(match pieces.len() {
            1 => pieces.pop().expect("length checked"),
            _ => Math::Row(pieces),
        })
    }

    /// One atom with any scripts attached to it.
    fn term(&mut self) -> Result<Math, UnsupportedTeX> {
        let mut base = self.atom()?;
        loop {
            if self.eat("^") {
                let first_highlight = self.highlights.len();
                let exponent = self.atom()?;
                let exponent =
                    self.tinted(exponent, &base, first_highlight, self.style.exponent_color);
                base = Math::Power {
                    base: Box::new(base),
                    exponent: Box::new(exponent),
                };
            } else if self.eat("_") {
                let start = self.at;
                let first_highlight = self.highlights.len();
                let mut subscript = self.atom()?;
                if base.is_type_name() && matches!(&subscript, Math::Atom { .. }) {
                    subscript = subscript.retag(HLGroup::Type);
                    self.highlights.truncate(first_highlight);
                    self.highlights.push((start..self.at, HLGroup::Type));
                }
                let subscript =
                    self.tinted(subscript, &base, first_highlight, self.style.index_color);
                base = Math::Index {
                    base: Box::new(base),
                    subscript: Box::new(subscript),
                };
            } else {
                return Ok(base);
            }
        }
    }

    /// `script` and the highlights recorded from `first_highlight` on, recoloured as
    /// `color` asks for a script of `base`.
    fn tinted(
        &mut self,
        script: Math,
        base: &Math,
        first_highlight: usize,
        color: ScriptColor,
    ) -> Math {
        let Some(role) = color.role(base) else {
            return script;
        };
        for (_, kind) in &mut self.highlights[first_highlight..] {
            *kind = kind.scripted(role);
        }
        script.tinted(role)
    }

    fn atom(&mut self) -> Result<Math, UnsupportedTeX> {
        self.skip_trivia();
        let start = self.at;
        let first_highlight = self.highlights.len();
        let parsed = self.atom_inner()?;
        let kind = match &parsed {
            Math::Atom { kind, .. } => {
                self.highlights.truncate(first_highlight);
                Some(*kind)
            }
            Math::Fraction { .. } => Some(HLGroup::FractionRule),
            Math::Delimited { .. } | Math::Grid { .. } => Some(HLGroup::Punctuation),
            _ => None,
        };
        if let Some(kind) = kind {
            self.highlights.push((start..self.at, kind));
        }
        Ok(parsed)
    }

    fn atom_inner(&mut self) -> Result<Math, UnsupportedTeX> {
        let rest = self.rest();
        let Some(next) = rest.chars().next() else {
            return Err(UnsupportedTeX("expected an atom, found nothing".into()));
        };
        match next {
            '{' => {
                self.at += 1;
                let body = self.expression(&["}"])?;
                match self.eat("}") {
                    true => Ok(body),
                    false => Err(UnsupportedTeX("unclosed group".into())),
                }
            }
            '\\' => self.command(),
            _ if next.is_ascii_digit() => Ok(self.numeral()),
            // A name in output is a symbol, coloured as code colours a symbol.
            _ if next.is_alphabetic() => Ok(self.run(
                |character| character.is_ascii_alphanumeric(),
                HLGroup::Variable,
            )),
            _ if OPERATORS.contains(next) => {
                self.at += next.len_utf8();
                Ok(Math::atom(next, HLGroup::Operator))
            }
            // A comma is followed by one space, whatever spacing TeX sets after it.
            ',' => {
                self.at += 1;
                self.skip_trivia();
                while self.eat(THIN_SPACE) {
                    self.skip_trivia();
                }
                Ok(Math::atom(", ", HLGroup::Delimiter))
            }
            _ if DELIMITERS.contains(next) => {
                self.at += next.len_utf8();
                Ok(Math::atom(next, HLGroup::Punctuation))
            }
            _ => Err(UnsupportedTeX(format!("unexpected character {next:?}"))),
        }
    }

    /// Take a maximal run of characters passing `accept` as one atom.
    fn run(&mut self, accept: impl Fn(&char) -> bool, kind: HLGroup) -> Math {
        let start = self.at;
        while self.rest().chars().next().is_some_and(|next| accept(&next)) {
            self.at += self.rest().chars().next().expect("just checked").len_utf8();
        }
        Math::atom(&self.source[start..self.at], kind)
    }

    /// A numeral, read whole across the thin spaces M2 groups its digits by.
    ///
    /// M2 sets a long integer as `1\,267\,650`: thin spaces before each group of
    /// exactly three digits. Only that shape continues the numeral, so a thin space
    /// before anything else stays the gap between two factors.
    fn numeral(&mut self) -> Math {
        let mut digits = String::new();
        loop {
            let start = self.at;
            self.at += self.rest().bytes().take_while(u8::is_ascii_digit).count();
            digits.push_str(&self.source[start..self.at]);
            let Some(after) = self.rest().strip_prefix(THIN_SPACE) else {
                break;
            };
            let group = after.bytes().take_while(u8::is_ascii_digit).count();
            if group != DIGIT_GROUP {
                break;
            }
            self.at += THIN_SPACE.len();
        }
        Math::atom(digits, HLGroup::Number)
    }

    fn command(&mut self) -> Result<Math, UnsupportedTeX> {
        self.at += 1;
        let name = self.name();
        match name.as_str() {
            "frac" => Ok(Math::Fraction {
                numerator: Box::new(self.atom()?),
                denominator: Box::new(self.atom()?),
            }),
            "begin" => self.environment(),
            "cdot" => Ok(Math::atom(PRODUCT, HLGroup::Operator)),
            // M2 prefixes interval fields with a square before their blackboard letter.
            // Named as in code, the square is the `i` closing the field's name: RRi.
            "square"
                if !self.style.blackboard && self.rest().trim_start().starts_with(BLACKBOARD) =>
            {
                match self.atom()? {
                    Math::Atom { text, kind, color } => Ok(Math::Atom {
                        text: text + INTERVAL_SUFFIX,
                        kind,
                        color,
                    }),
                    _ => Err(UnsupportedTeX("\\mathbb without a letter".into())),
                }
            }
            "square" => Ok(Math::atom("□", HLGroup::Type)),
            // Reached only between factors: a thin space inside a numeral is consumed
            // by the numeral itself.
            "," => Ok(Math::Space),
            // Arrows appear in the types of maps between modules. Both the short and
            // long spellings render long, because U+2190/U+2192 are East Asian
            // Ambiguous while U+27F5/U+27F6 measure one column everywhere.
            "leftarrow" | "longleftarrow" | "gets" => Ok(Math::atom("\u{27F5}", HLGroup::Operator)),
            "rightarrow" | "longrightarrow" | "to" => Ok(Math::atom("\u{27F6}", HLGroup::Operator)),
            "mapsto" | "longmapsto" => Ok(Math::atom("\u{27FC}", HLGroup::Operator)),
            // M2's own apply-to-list operator, `f \\ L`. It reaches TeX as a command
            // rather than a character, since a bare backslash starts one.
            "backslash" => Ok(Math::atom("\\", HLGroup::Operator)),
            "setminus" => Ok(Math::atom("\u{2216}", HLGroup::Operator)),
            brace @ ("{" | "}") => Ok(Math::atom(brace, HLGroup::Punctuation)),
            "left" => self.fences(),
            // A colour already chosen for what it wraps, which the colours inferred from
            // the words inside must not replace: the typesetter paints it as written.
            "textcolor" => {
                self.text_group()?;
                let inferred = self.highlights.len();
                let body = self.atom()?;
                self.highlights.truncate(inferred);
                Ok(body)
            }
            text if TEXT_COMMANDS.contains(&text) => {
                let words = Math::atom(self.text_group()?, HLGroup::Text);
                Ok(match text {
                    "texttt" => words.named_in_code(),
                    _ => words,
                })
            }
            // A font wrapper renames the role of what it wraps but adds no shape.
            font if FONTS.contains(&font) => {
                let body = self.atom()?;
                let constant = matches!(
                    &body,
                    Math::Atom { text, .. } if CONSTANT_LETTERS.contains(&(font, text.as_str()))
                );
                Ok(match font {
                    _ if constant => body.retag(HLGroup::Builtin),
                    "mathbb" => Math::retag(
                        body.lettered(&DOUBLE_STRUCK, self.style.blackboard),
                        HLGroup::Type,
                    ),
                    "texttt" => body.named_in_code(),
                    "mathfrak" => Math::retag(
                        body.lettered(&FRAKTUR, self.style.blackboard),
                        HLGroup::Type,
                    ),
                    "mathcal" => {
                        Math::retag(body.lettered(&SCRIPT, self.style.blackboard), HLGroup::Type)
                    }
                    _ => body,
                })
            }
            other => CONSTANT_COMMANDS
                .iter()
                .find(|(command, _)| *command == other)
                .map(|(_, glyph)| Math::atom(*glyph, HLGroup::Builtin))
                .or_else(|| self.operator(other))
                .ok_or_else(|| UnsupportedTeX(format!("\\{other}"))),
        }
    }

    /// The operator a command name spells, which may run on into the next word:
    /// M2 writes `x|y` as `x\midy`, so the name read is `midy`. The longest operator
    /// the name starts with is taken and the rest handed back to the input.
    fn operator(&mut self, name: &str) -> Option<Math> {
        let (command, glyph) = OPERATOR_COMMANDS
            .iter()
            .filter(|(command, _)| name.starts_with(command))
            .max_by_key(|(command, _)| command.len())?;
        self.at -= name.len() - command.len();
        Some(Math::atom(*glyph, HLGroup::Operator))
    }

    /// The alphabetic name after a backslash, or the single punctuation character.
    fn name(&mut self) -> String {
        let start = self.at;
        while self
            .rest()
            .chars()
            .next()
            .is_some_and(|next| next.is_ascii_alphabetic())
        {
            self.at += 1;
        }
        if self.at == start {
            self.at += self.rest().chars().next().map_or(0, char::len_utf8);
        }
        self.source[start..self.at].to_string()
    }

    /// `\begin{matrix}` or `\begin{cases}`, through to its matching `\end`.
    fn environment(&mut self) -> Result<Math, UnsupportedTeX> {
        if !self.eat("{") {
            return Err(UnsupportedTeX("\\begin without an environment".into()));
        }
        let name = self.name();
        if !self.eat("}") {
            return Err(UnsupportedTeX("unclosed environment name".into()));
        }
        let kind = match name.as_str() {
            "matrix" | "array" => GridKind::Bare,
            "pmatrix" => GridKind::Matrix,
            "cases" => GridKind::Cases,
            other => return Err(UnsupportedTeX(format!("environment {other}"))),
        };
        // `array` states a column alignment that the terminal layout does not use.
        if self.eat("{") {
            self.name();
            if !self.eat("}") {
                return Err(UnsupportedTeX("unclosed column specification".into()));
            }
        }
        let closing = format!("\\end{{{name}}}");
        let rows = self.rows(&closing)?;
        match self.eat(&closing) {
            true => Ok(Math::Grid { rows, kind }),
            false => Err(UnsupportedTeX(format!("missing {closing}"))),
        }
    }

    fn rows(&mut self, closing: &str) -> Result<Vec<Vec<Math>>, UnsupportedTeX> {
        let mut rows = vec![Vec::new()];
        loop {
            self.skip_trivia();
            if self.rest().starts_with(closing) {
                return Ok(rows);
            }
            if self.rest().is_empty() {
                return Err(UnsupportedTeX(format!("missing {closing}")));
            }
            rows.last_mut()
                .expect("a row is always open")
                .push(self.expression(&["&", "\\\\", closing])?);
            self.skip_trivia();
            if self.eat("\\\\") {
                rows.push(Vec::new());
            } else if !self.eat("&") && !self.rest().starts_with(closing) {
                return Err(UnsupportedTeX(format!("unexpected {}", self.rest())));
            }
        }
    }

    /// `\left( … \right)`, whose fences grow to the body.
    fn fences(&mut self) -> Result<Math, UnsupportedTeX> {
        self.skip_trivia();
        let left = self.fence()?;
        let body = self.expression(&["\\right"])?;
        if !self.eat("\\right") {
            return Err(UnsupportedTeX("\\left without \\right".into()));
        }
        self.skip_trivia();
        let right = self.fence()?;
        Ok(Math::Delimited {
            left,
            right,
            body: Box::new(body),
        })
    }

    fn fence(&mut self) -> Result<char, UnsupportedTeX> {
        self.skip_trivia();
        for (command, fence) in FENCE_COMMANDS {
            if self.eat(command) {
                return Ok(fence);
            }
        }
        // A brace is escaped in TeX, since a bare one groups.
        self.eat("\\");
        match self.rest().chars().next() {
            Some(fence) if DELIMITERS.contains(fence) => {
                self.at += fence.len_utf8();
                Ok(fence)
            }
            Some(other) => Err(UnsupportedTeX(format!("{other:?} is not a fence"))),
            None => Err(UnsupportedTeX("expected a fence".into())),
        }
    }
}

/// U+22C5 DOT OPERATOR, whose width is the same under every terminal convention --
/// unlike U+00B7 MIDDLE DOT, which is East Asian Ambiguous and would shift a border.
const PRODUCT: &str = "\u{22C5}";
/// The font command M2 writes a ring's letter in.
const BLACKBOARD: &str = "\\mathbb";
/// What ends an interval field's name in code, where its TeX has a square: `RRi`.
const INTERVAL_SUFFIX: &str = "i";
const THIN_SPACE: &str = "\\,";
/// Reserves height for its argument without drawing it.
const PHANTOM: &str = "\\vphantom";
/// How many digits M2 sets between the thin spaces of a long numeral.
const DIGIT_GROUP: usize = 3;
/// What an aligned row's last entry is indented by when it cannot fit beside the
/// columns before it and goes beneath them instead.
const BENEATH_INDENT: &str = "  ";
const SPACING: [&str; 8] = [
    "\\qquad",
    "\\quad",
    "\\;",
    "\\:",
    "\\!",
    "\\ ",
    // Empty groups M2 sets around fences to keep their spacing; they carry nothing.
    "\\mathopen{}",
    "\\mathclose{}",
];
const OPERATORS: &str = "+-=<>/*@:~?!%";
/// M2's constants TeX spells as a command, with the glyph a terminal draws for each.
const CONSTANT_COMMANDS: [(&str, &str); 3] = [
    ("pi", "\u{03C0}"),
    ("gamma", "\u{03B3}"),
    ("infty", "\u{221E}"),
];
/// M2's constants TeX spells as one letter in a font: `ii`, as M2 sets it and as it
/// is drawn, and `CatalanConstant`.
const CONSTANT_LETTERS: [(&str, &str); 3] = [("mathbf", "i"), ("mathrm", "i"), ("mathrm", "G")];
/// The words code paints as Booleans.
const BOOLEAN_WORDS: [&str; 2] = ["true", "false"];
/// The word code paints as a builtin among those M2 sets in typewriter face.
const NULL_WORD: &str = "null";

// @##issue non-ascii glyphs set in random place
/// Operators TeX spells as a command, with the glyph a terminal draws for each.
const OPERATOR_COMMANDS: [(&str, &str); 19] = [
    ("wedge", "\u{2227}"),
    ("vee", "\u{2228}"),
    ("otimes", "\u{2297}"),
    ("oplus", "\u{2295}"),
    ("circ", "\u{2218}"),
    ("cap", "\u{2229}"),
    ("cup", "\u{222A}"),
    ("times", "\u{00D7}"),
    ("ast", "\u{2217}"),
    ("mid", "|"),
    ("Rightarrow", OPTION_ARROW),
    ("Leftarrow", "\u{21D0}"),
    ("Leftrightarrow", "\u{21D4}"),
    ("Longrightarrow", "\u{27F9}"),
    ("Longleftarrow", "\u{27F8}"),
    ("leq", "\u{2264}"),
    ("geq", "\u{2265}"),
    ("cdots", "\u{22EF}"),
    // U+2026 is East Asian Ambiguous, so the low ellipsis is three full stops.
    ("ldots", "..."),
];
const DELIMITERS: &str = "()[]{}|.";
/// Fences TeX spells as a command, with the glyph a terminal draws for each.
const FENCE_COMMANDS: [(&str, char); 2] = [("\\langle", '\u{27E8}'), ("\\rangle", '\u{27E9}')];
/// The pieces a fence grows from when taller than a row: top, extension, bottom,
/// and the tip a brace points at its baseline with. These families measure one
/// column under every terminal convention, unlike the box-drawing set.
pub const TALL_FENCES: [(&str, [&str; 4]); 7] = [
    ("(", ["\u{239B}", "\u{239C}", "\u{239D}", "\u{239C}"]),
    (")", ["\u{239E}", "\u{239F}", "\u{23A0}", "\u{239F}"]),
    ("[", ["\u{23A1}", "\u{23A2}", "\u{23A3}", "\u{23A2}"]),
    ("]", ["\u{23A4}", "\u{23A5}", "\u{23A6}", "\u{23A5}"]),
    ("{", ["\u{23A7}", "\u{23AA}", "\u{23A9}", "\u{23A8}"]),
    ("}", ["\u{23AB}", "\u{23AA}", "\u{23AD}", "\u{23AC}"]),
    ("|", ["\u{23AA}"; 4]),
];
/// Braces two rows high, which leave no room for a tip and so are drawn from
/// their two halves; without either they would read as parentheses.
pub const SHORT_BRACES: [(&str, [&str; 4]); 2] = [
    ("{", ["\u{23B0}", "", "\u{23B1}", ""]),
    ("}", ["\u{23B1}", "", "\u{23B0}", ""]),
];
/// A TeX math alphabet as Unicode sets it: capitals in a run of the Mathematical
/// Alphanumeric Symbols block, bar the few Unicode had already placed among the
/// letterlike symbols.
struct Alphabet {
    first_capital: u32,
    letterlike: &'static [(char, char)],
    /// Whether M2 spells a letter in this alphabet twice in code, `\mathbb Q` as `QQ`,
    /// rather than a lone `Q` that reads as a variable.
    doubled_in_code: bool,
}

impl Alphabet {
    fn glyph(&self, letter: char) -> Option<char> {
        letter.is_ascii_uppercase().then(|| {
            self.letterlike
                .iter()
                .find(|(plain, _)| *plain == letter)
                .map_or_else(
                    || {
                        char::from_u32(self.first_capital + u32::from(letter) - u32::from('A'))
                            .expect("a capital of the alphabet")
                    },
                    |(_, glyph)| *glyph,
                )
        })
    }
}

const DOUBLE_STRUCK: Alphabet = Alphabet {
    first_capital: 0x1D538,
    letterlike: &[
        ('C', 'ℂ'),
        ('H', 'ℍ'),
        ('N', 'ℕ'),
        ('P', 'ℙ'),
        ('Q', 'ℚ'),
        ('R', 'ℝ'),
        ('Z', 'ℤ'),
    ],
    doubled_in_code: true,
};
const FRAKTUR: Alphabet = Alphabet {
    first_capital: 0x1D504,
    letterlike: &[('C', 'ℭ'), ('H', 'ℌ'), ('I', 'ℑ'), ('R', 'ℜ'), ('Z', 'ℨ')],
    doubled_in_code: false,
};
const SCRIPT: Alphabet = Alphabet {
    first_capital: 0x1D49C,
    letterlike: &[
        ('B', 'ℬ'),
        ('E', 'ℰ'),
        ('F', 'ℱ'),
        ('H', 'ℋ'),
        ('I', 'ℐ'),
        ('L', 'ℒ'),
        ('M', 'ℳ'),
        ('R', 'ℛ'),
    ],
    doubled_in_code: false,
};
/// Font commands M2 wraps names in. All are transparent to layout: they change how a
/// name is *set*, never what it is. `mathit` marks multi-character names, which is how
/// M2 keeps `x2` from reading as a power; the parser gets that from structure instead.
const FONTS: [&str; 8] = [
    "mathbb", "mathbf", "mathcal", "mathrm", "mathfrak", "mathsf", "mathtt", "mathit",
];
/// Commands whose argument is text rather than maths, set as written, spaces and all.
const TEXT_COMMANDS: [&str; 3] = ["texttt", "textrm", "text"];
/// Named characters TeX text spells as commands, since they would otherwise be markup.
const TEXT_SYMBOLS: [(&str, &str); 3] = [
    ("textbackslash", "\\"),
    ("textasciitilde", "~"),
    ("textasciicircum", "^"),
];
/// Characters TeX text takes as markup unless a backslash escapes them.
const TEXT_ESCAPED: &str = "#$%&_{}";

impl Math {
    /// Parse one M2 TeX expression.
    #[cfg(test)]
    pub fn parse(source: &str) -> Result<Self, UnsupportedTeX> {
        Self::parse_with(source, MathStyle::default())
    }

    /// Parse one M2 TeX expression, drawing rings as blackboard glyphs when asked.
    fn parse_with(source: &str, style: MathStyle) -> Result<Self, UnsupportedTeX> {
        let mut input = Input::new(source, style);
        let parsed = input.expression(&[])?;
        match input.rest().is_empty() {
            true => Ok(parsed),
            false => Err(UnsupportedTeX(format!("trailing input: {}", input.rest()))),
        }
    }

    fn atom(text: impl Into<String>, kind: HLGroup) -> Self {
        Self::Atom {
            text: text.into(),
            kind,
            color: None,
        }
    }

    /// A letter set in `alphabet`: its glyph when `glyphs` are asked for, else as code
    /// spells it.
    fn lettered(self, alphabet: &Alphabet, glyphs: bool) -> Self {
        match self {
            Self::Atom { text, kind, color } if text.chars().count() == 1 => {
                let letter = text.chars().next().expect("one character");
                let text = match alphabet.glyph(letter).filter(|_| glyphs) {
                    Some(glyph) => glyph.to_string(),
                    None if alphabet.doubled_in_code => text.repeat(2),
                    None => text,
                };
                Self::Atom { text, kind, color }
            }
            other => other,
        }
    }

    /// A name M2 sets in typewriter face is one from code: a Boolean or `null` as code
    /// paints them, a type when capitalised, as M2 names its types, else a function.
    fn named_in_code(self) -> Self {
        let kind = match &self {
            Self::Atom { text, .. } if BOOLEAN_WORDS.contains(&text.as_str()) => HLGroup::Boolean,
            Self::Atom { text, .. } if text == NULL_WORD => HLGroup::Builtin,
            Self::Atom { text, .. } if text.starts_with(char::is_uppercase) => HLGroup::Type,
            _ => HLGroup::Function,
        };
        self.retag(kind)
    }

    /// Whether this is a type name, including an interval field's prefix.
    fn is_type_name(&self) -> bool {
        match self {
            Self::Atom { kind, .. } => *kind == HLGroup::Type,
            Self::Row(pieces) => !pieces.is_empty() && pieces.iter().all(Self::is_type_name),
            _ => false,
        }
    }

    /// The role of the first atom, which is the one a script attached to this reads as.
    fn leading_role(&self) -> Option<HLGroup> {
        match self {
            Self::Atom { kind, .. } => Some(*kind),
            Self::Row(pieces) => pieces.iter().find_map(Self::leading_role),
            Self::Power { base, .. } | Self::Index { base, .. } => base.leading_role(),
            _ => None,
        }
    }

    /// Give every atom in a subtree `role`, except the delimiters and punctuation
    /// that structure it.
    fn tinted(self, role: HLGroup) -> Self {
        match self {
            Self::Native {
                text,
                tex,
                kind,
                color: _,
            } => Self::Native {
                text,
                tex,
                kind,
                color: Some(kind.scripted(role)),
            },

            Self::Over { base, label } => Self::Over {
                base: Box::new(base.tinted(role)),
                label: Box::new(label.tinted(role)),
            },
            Self::Under { base, label } => Self::Under {
                base: Box::new(base.tinted(role)),
                label: Box::new(label.tinted(role)),
            },
            Self::Linked { body, target } => Self::Linked {
                body: Box::new(body.tinted(role)),
                target,
            },
            Self::Atom { text, kind, .. } => Self::Atom {
                text,
                kind,
                color: Some(kind.scripted(role)),
            },
            Self::Row(pieces) => {
                Self::Row(pieces.into_iter().map(|piece| piece.tinted(role)).collect())
            }
            Self::Power { base, exponent } => Self::Power {
                base: Box::new(base.tinted(role)),
                exponent: Box::new(exponent.tinted(role)),
            },
            Self::Index { base, subscript } => Self::Index {
                base: Box::new(base.tinted(role)),
                subscript: Box::new(subscript.tinted(role)),
            },
            Self::Fraction {
                numerator,
                denominator,
            } => Self::Fraction {
                numerator: Box::new(numerator.tinted(role)),
                denominator: Box::new(denominator.tinted(role)),
            },
            Self::Delimited { left, right, body } => Self::Delimited {
                left,
                right,
                body: Box::new(body.tinted(role)),
            },
            Self::AnnotatedMatrix { rows, degrees } => Self::AnnotatedMatrix {
                rows: rows
                    .into_iter()
                    .map(|row| row.into_iter().map(|entry| entry.tinted(role)).collect())
                    .collect(),
                degrees: degrees
                    .into_iter()
                    .map(|entry| entry.tinted(role))
                    .collect(),
            },
            Self::Grid { rows, kind } => Self::Grid {
                rows: rows
                    .into_iter()
                    .map(|row| row.into_iter().map(|entry| entry.tinted(role)).collect())
                    .collect(),
                kind,
            },
            Self::Diagram(diagram) => {
                Self::Diagram(Box::new(diagram.map(|piece| piece.tinted(role))))
            }
            Self::Space => Self::Space,
        }
    }

    /// Give every atom in a subtree a different role.
    fn retag(self, kind: HLGroup) -> Self {
        match self {
            Self::Atom { text, color, .. } => Self::Atom { text, kind, color },
            Self::Row(pieces) => {
                Self::Row(pieces.into_iter().map(|piece| piece.retag(kind)).collect())
            }
            other => other,
        }
    }

    /// Lay out as rows of spans around a baseline, fractions set in `fractions`.
    pub fn layout(&self, fractions: FractionStyle) -> MathBlock {
        match self {
            Self::Native {
                text, kind, color, ..
            } => MathBlock::text(text, color.unwrap_or(*kind)),
            Self::Atom { text, kind, color } => MathBlock::text(text, color.unwrap_or(*kind)),
            Self::Over { base, label } => {
                let base = base.layout(fractions);
                let label = label.layout(fractions);
                let baseline = label.height() + base.baseline;
                MathBlock::stack(vec![label, base], baseline, Align::Centre)
            }
            Self::Under { base, label } => {
                let base = base.layout(fractions);
                let baseline = base.baseline;
                MathBlock::stack(vec![base, label.layout(fractions)], baseline, Align::Centre)
            }
            Self::Linked { body, target } => body.layout(fractions).linked(target),
            Self::Diagram(diagram) => diagram.layout(|piece| piece.layout(fractions)),
            Self::Space => MathBlock::empty(),
            Self::Row(pieces) => MathBlock::row(Self::chunks(pieces, fractions)),
            Self::Fraction {
                numerator,
                denominator,
            } => match fractions {
                FractionStyle::Stacked => {
                    MathBlock::fraction(numerator.layout(fractions), denominator.layout(fractions))
                }
                FractionStyle::Underlined => MathBlock::underlined(
                    numerator.layout(fractions),
                    denominator.layout(fractions),
                ),
                FractionStyle::Inline => MathBlock::row([
                    numerator.inline_operand(fractions),
                    MathBlock::atom("/", HLGroup::Operator),
                    denominator.inline_operand(fractions),
                ]),
            },
            Self::Delimited { left, right, body } => body
                .layout(fractions)
                .fenced(&left.to_string(), &right.to_string()),
            Self::Power { base, exponent } => {
                let base = base.layout(fractions);
                // An exponent with no superscript form is written out after a caret,
                // kept to one row so it cannot be read as a row above the base.
                match exponent.inline_script(Shift::Raised) {
                    Some(inline) => base.beside(inline),
                    None => base
                        .beside(MathBlock::atom("^", HLGroup::Operator))
                        .beside(exponent.inline_operand(FractionStyle::Inline)),
                }
            }
            Self::Index { base, subscript } => {
                let base = base.layout(fractions);
                match subscript.inline_script(Shift::Lowered) {
                    Some(inline) => base.beside(inline),
                    // A subscript with no lowered form is written out after an
                    // underscore on the base's row, braced when it is several pieces,
                    // as M2 writes the name `x_{1,1}`.
                    None => {
                        let written = match subscript.as_ref() {
                            Self::Row(_) => {
                                subscript.layout(FractionStyle::Inline).fenced("{", "}")
                            }
                            _ => subscript.inline_operand(FractionStyle::Inline),
                        };
                        let separator = if subscript.is_type_name() {
                            HLGroup::Type
                        } else {
                            HLGroup::Operator
                        };
                        base.beside(MathBlock::atom("_", separator)).beside(written)
                    }
                }
            }
            Self::AnnotatedMatrix { rows, degrees } => {
                MathBlock::grid_with_degrees(rows, GridKind::Matrix, fractions, Some(degrees), None)
            }
            Self::Grid { rows, kind } => MathBlock::grid(rows, *kind, fractions),
        }
    }

    /// One side of an inline fraction: parenthesised when it is more than one piece,
    /// so `(x + 1)/2` cannot read as `x + 1/2`.
    fn inline_operand(&self, fractions: FractionStyle) -> MathBlock {
        let laid = self.layout(fractions);
        match self {
            Self::Row(_) | Self::Fraction { .. } => laid.fenced("(", ")"),
            _ => laid,
        }
    }

    /// Lay out as terminal rows no wider than `width`.
    ///
    /// Whatever no break can shorten is cut into column slabs instead, line by line.
    fn wrapped(&self, width: usize, fractions: FractionStyle) -> Vec<SpanLine> {
        MathBlock::fitted(self.fitted_lines(width, fractions), width)
    }

    /// Lines, to be set one under another, that keep within `width` wherever the
    /// structure lets them: a row too wide breaks between its chunks, and a row that
    /// has no break fits its widest piece into what the others leave.
    fn fitted_lines(&self, width: usize, fractions: FractionStyle) -> Vec<MathBlock> {
        let Self::Row(pieces) = self else {
            return vec![self.fit(width, fractions)];
        };
        let chunks = Self::chunks(pieces, fractions);
        if MathBlock::row(chunks.iter().cloned()).width() <= width {
            return vec![MathBlock::row(chunks)];
        }
        if chunks.len() > 1 {
            return MathBlock::pack(chunks, width);
        }
        let mut laid: Vec<MathBlock> = pieces.iter().map(|piece| piece.layout(fractions)).collect();
        let Some(widest) = (0..laid.len()).max_by_key(|&index| laid[index].width()) else {
            return vec![MathBlock::empty()];
        };
        let others: usize = laid.iter().map(MathBlock::width).sum::<usize>() - laid[widest].width();
        laid[widest] = pieces[widest].fit(width.saturating_sub(others), fractions);
        vec![MathBlock::row(laid)]
    }

    /// Laid out within `width` where the structure lets it, else as it is.
    ///
    /// A row continues beneath where it began; fences around a row that breaks stay
    /// on the lines they open and close, as a wrapped list's do; the last entry of a
    /// vertical or aligned grid's rows fits beside the columns before it.
    fn fit(&self, width: usize, fractions: FractionStyle) -> MathBlock {
        let laid = self.layout(fractions);
        if laid.width() <= width {
            return laid;
        }
        match self {
            Self::Row(_) => MathBlock::lines(self.fitted_lines(width, fractions)),
            Self::Linked { body, target } => body.fit(width, fractions).linked(target),
            Self::Delimited { left, right, body } => {
                let (left, right) = (left.to_string(), right.to_string());
                let fences =
                    UnicodeWidthStr::width(left.as_str()) + UnicodeWidthStr::width(right.as_str());
                let mut lines = body.fitted_lines(width.saturating_sub(fences), fractions);
                match lines.len() {
                    1 => lines.remove(0).fenced(&left, &right),
                    _ => MathBlock::lines(MathBlock::run_on(lines, &left, &right)),
                }
            }
            Self::Grid {
                rows,
                kind: kind @ (GridKind::LeftAligned | GridKind::Aligned),
            } => MathBlock::grid_with_degrees(rows, *kind, fractions, None, Some(width)),
            _ => laid,
        }
    }

    /// Whether this holds rows read one after another, as lines of text are: a
    /// vertical list's entries, an aligned grid's. Cut into strips, a picture of it
    /// would scatter the rows' tails beneath it.
    pub fn reads_in_lines(&self) -> bool {
        match self {
            Self::Grid {
                kind: GridKind::LeftAligned | GridKind::Aligned,
                ..
            }
            | Self::Diagram(_) => true,
            Self::Grid { rows, .. } | Self::AnnotatedMatrix { rows, .. } => {
                rows.iter().flatten().any(Self::reads_in_lines)
            }
            Self::Row(pieces) => pieces.iter().any(Self::reads_in_lines),
            Self::Linked { body, .. } | Self::Delimited { body, .. } => body.reads_in_lines(),
            Self::Over { base, label } | Self::Under { base, label } => {
                base.reads_in_lines() || label.reads_in_lines()
            }
            Self::Power { base, exponent } => base.reads_in_lines() || exponent.reads_in_lines(),
            Self::Index { base, subscript } => base.reads_in_lines() || subscript.reads_in_lines(),
            Self::Fraction {
                numerator,
                denominator,
            } => numerator.reads_in_lines() || denominator.reads_in_lines(),
            Self::Atom { .. } | Self::Native { .. } | Self::Space => false,
        }
    }

    /// Whether a line may end after this, as after a list's comma.
    fn is_separator(&self) -> bool {
        matches!(
            self,
            Self::Atom { text, kind: HLGroup::Delimiter, .. }
                if matches!(text.trim_end(), "," | ";")
        )
    }

    /// Parse what M2's `tex` produced, when it is maths: the source between `$` signs.
    ///
    /// For a String, a Net or null M2 answers `tex` with the raw text instead, which
    /// is not maths -- read as TeX, `hello world` would set as the name `helloworld`.
    pub fn from_tex(source: &str, style: MathStyle) -> Result<Self, UnsupportedTeX> {
        let maths = source
            .strip_prefix('$')
            .and_then(|tex| tex.strip_suffix('$'))
            .ok_or_else(|| UnsupportedTeX("not maths: no $ delimiters".into()))?;
        Self::parse_with(maths, style)
    }

    /// A row's pieces laid out in order, split wherever a long row may break.
    ///
    /// A row breaks before a binary operator, so a continued line opens with the `+` that
    /// says it continues, and between two factors. Each chunk is everything
    /// between two breaks, already set side by side; a row that fits is just its chunks
    /// set side by side again.
    fn chunks(pieces: &[Self], fractions: FractionStyle) -> Vec<MathBlock> {
        let mut chunks = vec![Vec::new()];
        for (index, piece) in pieces.iter().enumerate() {
            // M2 pads an arrow with thin spaces of its own; the gap a binary operator
            // is set apart by already provides them.
            let beside_binary = index
                .checked_sub(1)
                .and_then(|before| pieces.get(before))
                .into_iter()
                .chain(pieces.get(index + 1))
                .any(Self::is_binary);
            if *piece == Self::Space {
                if !beside_binary {
                    // Two numerals juxtaposed would read as one longer numeral.
                    let between_numerals =
                        [index.checked_sub(1), Some(index + 1)]
                            .into_iter()
                            .all(|neighbour| {
                                neighbour
                                    .and_then(|neighbour| pieces.get(neighbour))
                                    .is_some_and(|neighbour| {
                                        matches!(
                                            neighbour,
                                            Self::Atom {
                                                kind: HLGroup::Number,
                                                ..
                                            }
                                        )
                                    })
                            });
                    if between_numerals {
                        chunks
                            .last_mut()
                            .expect("a chunk is always open")
                            .push(MathBlock::gap());
                    }
                    chunks.push(Vec::new());
                }
                continue;
            }
            // A binary operator is set apart from its operands; a leading sign and a
            // product dot are not, so `-x` and `x\u{22C5}y` stay tight.
            let apart = index > 0 && piece.is_binary();
            if apart {
                chunks
                    .last_mut()
                    .expect("a chunk is always open")
                    .push(MathBlock::gap());
                chunks.push(Vec::new());
            }
            // An inline fraction runs straight into a factor after it, and `1/3x`
            // reads as one over 3x; the gap keeps it the factor it is.
            let runs_into_factor = fractions == FractionStyle::Inline
                && matches!(piece, Self::Fraction { .. })
                && pieces.get(index + 1).is_some_and(|next| !next.is_binary());
            let chunk = chunks.last_mut().expect("a chunk is always open");
            chunk.push(piece.layout(fractions));
            if apart || runs_into_factor {
                chunk.push(MathBlock::gap());
            }
            if piece.is_separator() {
                chunks.push(Vec::new());
            }
        }
        chunks
            .into_iter()
            .filter(|chunk| !chunk.is_empty())
            .map(MathBlock::row)
            .collect()
    }

    /// Whether this separates its neighbours rather than binding to one.
    fn is_binary(&self) -> bool {
        if let Self::Over { base, .. } = self {
            return base.is_binary();
        }
        matches!(
            self,
            Self::Atom {
                kind: HLGroup::MapArrow,
                ..
            }
        ) || matches!(
            self,
            Self::Atom { text, kind: HLGroup::Operator, .. }
                if matches!(
                    text.as_str(),
                    "+" | "-"
                        | "="
                        | "<"
                        | ">"
                        | "\\"
                        | "\u{2216}"
                        | "\u{27F5}"
                        | "\u{27F6}"
                        | "\u{27FC}"
                )
        )
    }

    /// This script set on its base's own row, when every character of it can shift.
    fn inline_script(&self, shift: Shift) -> Option<MathBlock> {
        let Self::Atom { text, kind, color } = self else {
            return None;
        };
        shift
            .form(text)
            .map(|text| MathBlock::atom(&text, color.unwrap_or(*kind)))
    }
}

impl HLGroup {
    /// This role in a script recoloured to `role`: delimiters, punctuation and
    /// fraction rules structure a script and keep their own.
    fn scripted(self, role: Self) -> Self {
        match self {
            Self::Delimiter | Self::Punctuation | Self::FractionRule => self,
            _ => role,
        }
    }
}

/// Which way a script moves off its base's row.
#[derive(Clone, Copy)]
pub enum Shift {
    /// Superscript characters above the text baseline.
    Raised,
    /// Subscript characters below the text baseline.
    Lowered,
}

impl Shift {
    /// `text` in its shifted form, when every character of it has one. A script that
    /// does rides its base's row, which keeps polynomials compact.
    pub fn form(self, text: &str) -> Option<String> {
        text.chars()
            .map(|character| {
                Some(match (self, character) {
                    (Self::Raised, '0'..='9') => "\u{2070}\u{00B9}\u{00B2}\u{00B3}\u{2074}\u{2075}\u{2076}\u{2077}\u{2078}\u{2079}"
                        .chars()
                        .nth(character as usize - '0' as usize)?,
                    (Self::Raised, '+') => '\u{207A}',
                    (Self::Raised, '-') => '\u{207B}',
                    (Self::Raised, 'n') => '\u{207F}',
                    (Self::Raised, 'i') => '\u{2071}',
                    (Self::Lowered, '0'..='9') => {
                        char::from_u32(0x2080 + (character as u32 - '0' as u32))?
                    }
                    (Self::Lowered, '+') => '\u{208A}',
                    (Self::Lowered, '-') => '\u{208B}',
                    _ => return None,
                })
            })
            .collect()
    }
}

/// A laid-out block of maths: rows of spans, and the row that aligns with its neighbours.
///
/// The baseline is what makes composition work. Set `a + \frac{b}{c}` without one and
/// the `+` lands level with the numerator; with one it sits on the rule, where a reader
/// expects it. Every operation below preserves it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MathBlock {
    rows: Vec<SpanLine>,
    baseline: usize,
}

impl MathBlock {
    /// The same block, every span of it linking to `target`.
    fn linked(mut self, target: &str) -> Self {
        link(self.rows.iter_mut().flatten(), target);
        self
    }

    fn atom(text: &str, kind: HLGroup) -> Self {
        Self {
            rows: vec![vec![TextSpan::new(text, kind)]],
            baseline: 0,
        }
    }

    fn empty() -> Self {
        Self {
            rows: vec![Vec::new()],
            baseline: 0,
        }
    }

    /// The single column set around a binary operator and between factors.
    fn gap() -> Self {
        Self::atom(" ", HLGroup::Canvas)
    }

    #[cfg(test)]
    pub fn rows(&self) -> &[SpanLine] {
        &self.rows
    }

    pub(super) fn into_rows(self) -> Vec<SpanLine> {
        self.rows
    }

    /// The row that aligns with neighbouring blocks.
    #[cfg(test)]
    pub const fn baseline(&self) -> usize {
        self.baseline
    }

    pub fn width(&self) -> usize {
        self.rows.iter().map(Self::row_width).max().unwrap_or(0)
    }

    pub fn height(&self) -> usize {
        self.rows.len()
    }

    fn row_width(row: &SpanLine) -> usize {
        row.iter()
            .map(|span| UnicodeWidthStr::width(span.content.as_str()))
            .sum()
    }

    /// The part of a row whose columns fall within `range`.
    ///
    /// A mark of no width, such as a combining accent, belongs to the column of the
    /// character it marks.
    fn columns(row: &SpanLine, range: Range<usize>) -> SpanLine {
        let mut column = 0;
        let mut start = 0;
        row.iter()
            .filter_map(|span| {
                let kept: String = span
                    .content
                    .chars()
                    .filter(|character| {
                        let width = UnicodeWidthChar::width(*character).unwrap_or(0);
                        if width > 0 {
                            start = column;
                            column += width;
                        }
                        range.contains(&start)
                    })
                    .collect();
                (!kept.is_empty()).then(|| span.with_content(kept))
            })
            .collect()
    }

    /// Rows above the baseline, and rows below it.
    fn reach(&self) -> (usize, usize) {
        (self.baseline, self.rows.len() - self.baseline - 1)
    }

    /// Pad every row to `width`, so columns line up when stacked.
    fn justify(mut self, width: usize) -> Self {
        for row in &mut self.rows {
            let pad = width - Self::row_width(row);
            if pad > 0 {
                row.push(TextSpan::new(" ".repeat(pad), HLGroup::Canvas));
            }
        }
        self
    }

    /// Set `other` to the right, aligning the two baselines.
    fn beside(self, other: Self) -> Self {
        let (above, below) = self.reach();
        let (other_above, other_below) = other.reach();
        let baseline = above.max(other_above);
        let height = baseline + below.max(other_below) + 1;
        let (left, right) = (self.width(), other.width());
        let mut rows = vec![Vec::new(); height];

        for (block, width) in [(self, left), (other, right)] {
            let offset = baseline - block.baseline;
            let reached = block.rows.len();
            for (index, mut row) in block.justify(width).rows.into_iter().enumerate() {
                rows[offset + index].append(&mut row);
            }
            // Rows this block does not reach still owe their columns.
            for (index, row) in rows.iter_mut().enumerate() {
                if index < offset || index >= offset + reached {
                    row.push(TextSpan::new(" ".repeat(width), HLGroup::Canvas));
                }
            }
        }
        Self { rows, baseline }
    }

    /// Set blocks side by side, left to right.
    fn row(blocks: impl IntoIterator<Item = Self>) -> Self {
        blocks
            .into_iter()
            .reduce(Self::beside)
            .unwrap_or_else(Self::empty)
    }

    /// A value M2 sent, laid out whole: TeX from its structure, a container from its
    /// entries, a token in its role's colour, and whatever M2 drew as plain text as
    /// a string.
    fn value(value: &SpecNet, style: MathStyle) -> Self {
        let block = if let Some(text) = &value.text {
            Self::text(text, value.group())
        } else {
            Math::from_spec_net(value, style).layout(style.fractions)
        };
        match &value.target {
            Some(target) => block.linked(target),
            None => block,
        }
    }

    /// A horizontal collection as lines no wider than `width`.
    ///
    /// The entries are packed between their commas first and fenced after, so each
    /// delimiter grows to the height of the line it closes: the first line opens the
    /// collection, the last closes it, and a line between is indented to sit under
    /// the first entry.
    fn listed(
        kind: CollectionKind,
        items: &[SpecNet],
        width: usize,
        style: MathStyle,
    ) -> Vec<Self> {
        let (open, close) = kind.drawn_delimiters();
        let last = items.len().saturating_sub(1);
        let entries = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let entry = Self::value(item, style);
                match index == last {
                    true => entry,
                    false => Self::row([entry, Self::atom(",", HLGroup::Delimiter), Self::gap()]),
                }
            })
            .collect();
        let budget =
            width.saturating_sub(UnicodeWidthStr::width(open) + UnicodeWidthStr::width(close));
        Self::run_on(Self::pack(entries, budget), open, close)
    }

    /// Lines between fences that stay on the lines they open and close, as text
    /// brackets do: the first line follows the opening fence, the rest are indented
    /// beneath it, and the closing fence ends the last.
    fn run_on(mut lines: Vec<Self>, open: &str, close: &str) -> Vec<Self> {
        if lines.is_empty() {
            lines.push(Self::empty());
        }
        let indent = UnicodeWidthStr::width(open);
        let count = lines.len();
        lines
            .into_iter()
            .enumerate()
            .map(|(index, line)| {
                let (height, baseline) = (line.height(), line.baseline);
                let lead = match index {
                    0 => Self::fence(open, height, baseline),
                    _ => Self::atom(&" ".repeat(indent), HLGroup::Canvas),
                };
                let tail = (index + 1 == count).then(|| Self::fence(close, height, baseline));
                Self::row([lead, line].into_iter().chain(tail))
            })
            .collect()
    }

    /// Lines set one under another, aligned on their left edges, the first line's
    /// baseline aligning with neighbours.
    fn lines(lines: Vec<Self>) -> Self {
        let baseline = lines.first().map_or(0, |line| line.baseline);
        Self::stack(lines, baseline, Align::Left)
    }

    /// Text M2 drew itself, row for row, all in one group.
    fn text(text: &str, kind: HLGroup) -> Self {
        let rows: Vec<SpanLine> = text
            .lines()
            .map(|line| vec![TextSpan::new(line, kind)])
            .collect();
        match rows.is_empty() {
            true => Self::empty(),
            false => Self {
                baseline: (rows.len() - 1) / 2,
                rows,
            },
        }
    }

    /// Lines as the terminal rows they fill, any still wider than `width` cut into
    /// slabs.
    fn fitted(lines: Vec<Self>, width: usize) -> Vec<SpanLine> {
        lines
            .into_iter()
            .flat_map(|line| line.slabs(width))
            .flat_map(Self::into_rows)
            .map(Self::trimmed)
            .collect()
    }

    /// Cut into column ranges at most `width` wide, set one under another.
    ///
    /// The last resort for a piece no line break can shorten: a numeral, or a matrix,
    /// wider than the screen. A slab ends on the last column blank in every row
    /// that it can reach, between a matrix's columns or a list's entries, and the
    /// next starts past the blanks; only a stretch with no such column is cut through.
    fn slabs(self, width: usize) -> Vec<Self> {
        let total = self.visible_width();
        if total <= width || width == 0 {
            return vec![self];
        }
        let blank = self.blank_columns(total);
        let mut ranges = Vec::new();
        let mut from = 0;
        while total - from > width {
            let end = (from + 1..=from + width)
                .rev()
                .find(|&column| blank[column])
                .unwrap_or(from + width);
            ranges.push(from..end);
            from = (end..total).find(|&column| !blank[column]).unwrap_or(total);
        }
        ranges.push(from..total);
        ranges
            .into_iter()
            .map(|range| Self {
                rows: self
                    .rows
                    .iter()
                    .map(|row| Self::columns(row, range.clone()))
                    .collect(),
                baseline: self.baseline,
            })
            .collect()
    }

    /// Which of the first `width` columns show nothing in any row.
    fn blank_columns(&self, width: usize) -> Vec<bool> {
        let mut blank = vec![true; width];
        for row in &self.rows {
            let mut column = 0;
            for span in row {
                for character in span.content.chars() {
                    let cells = UnicodeWidthChar::width(character).unwrap_or(0);
                    if character != ' ' || span.mark.is_some() {
                        let end = (column + cells).min(width);
                        blank[column.min(end)..end].fill(false);
                    }
                    column += cells;
                }
            }
        }
        blank
    }

    /// Pad every row to `width`, placing the content by `align`.
    fn pad(mut self, width: usize, align: Align) -> Self {
        for row in &mut self.rows {
            let slack = width - Self::row_width(row);
            let before = match align {
                Align::Left => 0,
                Align::Centre => slack / 2,
            };
            if before > 0 {
                row.insert(0, TextSpan::new(" ".repeat(before), HLGroup::Canvas));
            }
            if slack > before {
                row.push(TextSpan::new(" ".repeat(slack - before), HLGroup::Canvas));
            }
        }
        self
    }

    /// Set blocks one under another, nominating which row aligns with neighbours;
    /// no blocks stack as an empty block, which still takes a row.
    fn stack(blocks: Vec<Self>, baseline: usize, align: Align) -> Self {
        if blocks.is_empty() {
            return Self::empty();
        }
        let width = blocks.iter().map(Self::width).max().unwrap_or(0);
        Self {
            rows: blocks
                .into_iter()
                .flat_map(|block| block.pad(width, align).rows)
                .collect(),
            baseline,
        }
    }

    /// One expression over another. The rule is the baseline, so a neighbouring `+`
    /// sits level with it rather than beside the numerator.
    fn fraction(numerator: Self, denominator: Self) -> Self {
        let width = numerator.width().max(denominator.width()).max(1);
        let baseline = numerator.height();
        let rule = Self {
            rows: vec![vec![TextSpan::new(
                "\u{2500}".repeat(width),
                HLGroup::Operator,
            )]],
            baseline: 0,
        };
        Self::stack(vec![numerator, rule, denominator], baseline, Align::Centre)
    }

    /// One expression over another in two rows: the rule is an underline across the
    /// numerator's bottom row rather than a row of its own. That row carries the rule,
    /// so it is the baseline, as the rule's row is in a stacked fraction.
    fn underlined(numerator: Self, denominator: Self) -> Self {
        let width = numerator.width().max(denominator.width()).max(1);
        let baseline = numerator.height() - 1;
        let mut numerator = numerator.pad(width, Align::Centre);
        if let Some(bottom) = numerator.rows.last_mut() {
            *bottom = bottom
                .drain(..)
                .map(|span| span.marked(HLGroup::FractionRule))
                .collect();
        }
        Self::stack(vec![numerator, denominator], baseline, Align::Centre)
    }

    /// Surround with fences that grow to the body's height.
    fn fenced(self, left: &str, right: &str) -> Self {
        let (height, baseline) = (self.height(), self.baseline);
        Self::fence(left, height, baseline)
            .beside(self)
            .beside(Self::fence(right, height, baseline))
    }

    /// A fence drawn to `height` rows around `baseline`.
    ///
    /// A bracket grows from its [`TALL_FENCES`] pieces, a brace two rows high from
    /// its [`SHORT_BRACES`] halves, so a grown matrix keeps its shape wherever it
    /// is read. Any other fence -- M2's `<|` -- stays one row high on the
    /// baseline, and TeX's `.` marks the absence of one.
    pub(super) fn fence(fence: &str, height: usize, baseline: usize) -> Self {
        if fence == "." {
            return Self::empty();
        }
        let short = match height {
            2 => &SHORT_BRACES[..],
            _ => &[],
        };
        let Some(&(_, [top, extension, bottom, tip])) = short
            .iter()
            .chain(&TALL_FENCES)
            .find(|(family, _)| *family == fence)
            .filter(|_| height > 1)
        else {
            return Self::atom(fence, HLGroup::Punctuation);
        };
        let pointed = baseline.clamp(1, height.saturating_sub(2).max(1));
        Self {
            rows: (0..height)
                .map(|row| {
                    let glyph = match row {
                        0 => top,
                        _ if row + 1 == height => bottom,
                        _ if row == pointed => tip,
                        _ => extension,
                    };
                    vec![TextSpan::new(glyph, HLGroup::Punctuation)]
                })
                .collect(),
            baseline,
        }
    }

    /// Entries aligned into columns, fenced by whatever the grid's kind calls for.
    fn grid(rows: &[Vec<Math>], kind: GridKind, fractions: FractionStyle) -> Self {
        Self::grid_with_degrees(rows, kind, fractions, None, None)
    }

    /// The grid, with each row's last entry fitted into what is left of `room`
    /// beside the columns before it, when there is a room to keep within.
    fn grid_with_degrees(
        rows: &[Vec<Math>],
        kind: GridKind,
        fractions: FractionStyle,
        degrees: Option<&[Math]>,
        room: Option<usize>,
    ) -> Self {
        let gap = match kind {
            GridKind::Aligned => " ",
            _ => "  ",
        };
        let column_widths = |laid: &[Vec<Self>]| -> Vec<usize> {
            let columns = laid.iter().map(Vec::len).max().unwrap_or(0);
            (0..columns)
                .map(|column| {
                    laid.iter()
                        .filter_map(|row| row.get(column))
                        .map(Self::width)
                        .max()
                        .unwrap_or(0)
                })
                .collect()
        };
        let mut laid: Vec<Vec<Self>> = rows
            .iter()
            .map(|row| row.iter().map(|entry| entry.layout(fractions)).collect())
            .collect();
        // An aligned row's last entry too wide to fit beside the columns before it
        // is set beneath them, leaving the columns to the rows that fit.
        let mut beneath: Vec<Option<Self>> = vec![None; rows.len()];
        if let Some(room) = room {
            let widths = column_widths(&laid);
            for ((row, entries), beneath) in rows.iter().zip(&mut laid).zip(&mut beneath) {
                let (Some(last), Some(cell)) = (row.last(), entries.last_mut()) else {
                    continue;
                };
                let before = row.len() - 1;
                let beside = room
                    .saturating_sub(widths[..before].iter().sum::<usize>() + before * gap.len());
                *cell = last.fit(beside, fractions);
                if kind == GridKind::Aligned && before > 0 && cell.width() > beside {
                    entries.pop();
                    *beneath = Some(
                        Self::atom(BENEATH_INDENT, HLGroup::Canvas)
                            .beside(last.fit(room.saturating_sub(BENEATH_INDENT.len()), fractions)),
                    );
                }
            }
        }
        let widths = column_widths(&laid);

        let column_gap = || Self::atom(gap, HLGroup::Canvas);
        let stacked: Vec<Self> = laid
            .into_iter()
            .zip(beneath)
            .map(|(row, beneath)| {
                let row = row
                    .into_iter()
                    .enumerate()
                    .map(|(column, entry)| {
                        let alignment = if matches!(kind, GridKind::LeftAligned | GridKind::Aligned)
                        {
                            Align::Left
                        } else {
                            Align::Centre
                        };
                        entry.pad(widths[column], alignment)
                    })
                    .reduce(|left, right| left.beside(column_gap()).beside(right))
                    .unwrap_or_else(Self::empty);
                match beneath {
                    Some(beneath) => Self::lines(vec![row, beneath]),
                    None => row,
                }
            })
            .collect();

        if let Some(degrees) = degrees {
            let mut labels = Vec::new();
            let mut entries = Vec::new();
            for (index, row) in stacked.into_iter().enumerate() {
                let degree = degrees
                    .get(index)
                    .map(|degree| degree.layout(fractions))
                    .unwrap_or_else(Self::empty);
                let baseline = degree.baseline.max(row.baseline);
                let below = (degree.height() - degree.baseline).max(row.height() - row.baseline);
                let align = |block: Self| {
                    let mut rows = vec![Vec::new(); baseline - block.baseline];
                    rows.extend(block.rows);
                    rows.resize(baseline + below, Vec::new());
                    Self { rows, baseline }
                };
                labels.push(align(degree));
                entries.push(align(row));
            }
            let height: usize = entries.iter().map(Self::height).sum();
            let baseline = height.saturating_sub(1) / 2;
            let labels = Self::stack(labels, baseline, Align::Left);
            let body = Self::stack(entries, baseline, Align::Left).fenced("(", ")");
            return labels.beside(Self::atom(" ", HLGroup::Canvas)).beside(body);
        }
        let height: usize = stacked.iter().map(Self::height).sum();
        let body = Self::stack(stacked, height.saturating_sub(1) / 2, Align::Left);
        match kind {
            GridKind::Bare | GridKind::LeftAligned | GridKind::Aligned => body,
            GridKind::Matrix => body.fenced("(", ")"),
            GridKind::Cases => Self::fence("{", body.height(), body.baseline).beside(body),
        }
    }

    /// Gather a row's chunks, in order, into lines no wider than `width` where possible.
    ///
    /// Greedy: each line takes chunks until the next would overflow it. A chunk's
    /// trailing gap is not held against the width, since a line may end on it
    /// unseen. A chunk too wide for any line gets one of its own, for `slabs` to cut.
    fn pack(chunks: Vec<Self>, width: usize) -> Vec<Self> {
        let mut lines: Vec<Vec<Self>> = Vec::new();
        let mut filled = 0;
        for chunk in chunks {
            match lines.last_mut() {
                Some(line) if filled + chunk.visible_width() <= width => {
                    filled += chunk.width();
                    line.push(chunk);
                }
                _ => {
                    filled = chunk.width();
                    lines.push(vec![chunk]);
                }
            }
        }
        lines.into_iter().map(Self::row).collect()
    }

    /// Columns up to the last one that shows: how much of a line its content needs,
    /// when the line may end on spaces.
    fn visible_width(&self) -> usize {
        self.rows
            .iter()
            .map(|row| Self::row_width(&Self::trimmed(row.clone())))
            .max()
            .unwrap_or(0)
    }

    /// A row without the blank columns it ends on. A marked blank shows its mark, so
    /// it is not blank.
    fn trimmed(mut row: SpanLine) -> SpanLine {
        while let Some(last) = row.last_mut() {
            if last.mark.is_some() {
                break;
            }
            let kept = last.content.trim_end().len();
            if kept > 0 {
                last.content.truncate(kept);
                break;
            }
            row.pop();
        }
        row
    }
}

/// Where content sits when a row is narrower than its column.
#[derive(Clone, Copy)]
enum Align {
    Left,
    Centre,
}
