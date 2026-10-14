//! Macaulay2's TeX, parsed into a tree that knows what each piece *is*.
//!
//! M2 emits machine-generated TeX from a small fixed vocabulary — polynomials,
//! fractions, cases and matrices — so this parses that subset rather than LaTeX at
//! large. Anything outside it is refused, and the caller falls back to M2's own Net
//! spelling.
//!
//! The point of parsing at all is colour. A rendered value that arrives as a `String`
//! can only be painted one shade, because by then a numeral, a variable and a bracket
//! are the same thing: characters. Every node here carries the [`HLGroup`] its role
//! implies, so the scheme paints output the way it paints code — from parsed
//! structure, never from matching text.

use std::ops::Range;

use serde::{Deserialize, Serialize};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use these_macros_should_be_illegal::discriminated_str;

use crate::backend::{CollectionKind, M2Value, Role};
use crate::ui::block::TextSpan;
use crate::ui::controller::settings::theme::groups::HLGroup;
use crate::ui::present::{wrap, SpanLine};
use crate::ui::wrap::WordWrap;

/// A piece of parsed maths, tagged with the role that decides its colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Math {
    /// One indivisible run of text: a numeral, a name, an operator, a delimiter.
    Atom { text: String, kind: HLGroup },
    /// The thin space M2 sets between factors: a gap that keeps the product `x y`
    /// apart from the variable `xy`, and a place a long line may break.
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
    /// Rows of aligned entries: a matrix, or the branches of a `cases`.
    Grid {
        rows: Vec<Vec<Math>>,
        kind: GridKind,
    },
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
pub struct Unsupported(pub String);

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

impl FractionStyle {
    pub const fn cycle(self, delta: i8) -> Self {
        match (self, delta > 0) {
            (Self::Stacked, true) | (Self::Inline, false) => Self::Underlined,
            (Self::Underlined, true) | (Self::Stacked, false) => Self::Inline,
            (Self::Inline, true) | (Self::Underlined, false) => Self::Stacked,
        }
    }
}

impl Role {
    /// The group code paints the same thing with, so a value reads as it would
    /// typed: `null` as a constant, `true` as a Boolean, a name as a function.
    const fn group(self) -> HLGroup {
        match self {
            Self::Null => HLGroup::Constant,
            Self::Boolean => HLGroup::Boolean,
            Self::String => HLGroup::String,
            Self::Function => HLGroup::Function,
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
    value: &M2Value,
    width: usize,
    fractions: FractionStyle,
) -> Result<Vec<SpanLine>, Unsupported> {
    match value {
        M2Value::Tex { source, .. } => Ok(Math::from_tex(source)?.wrapped(width, fractions)),
        M2Value::Collection { kind, items } if *kind != CollectionKind::VerticalList => Ok(
            MathBlock::fitted(MathBlock::listed(*kind, items, width, fractions), width),
        ),
        M2Value::Collection { .. } | M2Value::HashTable(_) => Ok(MathBlock::fitted(
            vec![MathBlock::value(value, fractions)],
            width,
        )),
        // A string can run long, so it breaks at words like any prose.
        M2Value::Token { text, role } => Ok(text
            .lines()
            .flat_map(|line| {
                let width = u16::try_from(width).unwrap_or(u16::MAX);
                wrap(&[TextSpan::new(line, role.group())], width, &WordWrap)
            })
            .collect()),
        M2Value::Net(_) | M2Value::Markup { .. } => {
            Err(Unsupported("plain text carries nothing to colour".into()))
        }
    }
}

/// A value's type annotation as rows no wider than `width`, its pieces set side by
/// side from their own structure and painted in the type's colour throughout.
pub fn annotation(parts: &[M2Value], width: usize, fractions: FractionStyle) -> Vec<SpanLine> {
    let line = MathBlock::row(parts.iter().map(|part| MathBlock::value(part, fractions)));
    MathBlock::fitted(vec![line], width)
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|span| TextSpan {
                    kind: HLGroup::Type,
                    ..span
                })
                .collect()
        })
        .collect()
}

/// A cursor over TeX source.
struct Input<'a> {
    source: &'a str,
    at: usize,
}

impl<'a> Input<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source: source.trim(),
            at: 0,
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
            match SPACING
                .iter()
                .find(|space| self.rest().starts_with(**space))
            {
                Some(space) => self.at += space.len(),
                None => return,
            }
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
    fn expression(&mut self, stops: &[&str]) -> Result<Math, Unsupported> {
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
    fn term(&mut self) -> Result<Math, Unsupported> {
        let mut base = self.atom()?;
        loop {
            if self.eat("^") {
                base = Math::Power {
                    base: Box::new(base),
                    exponent: Box::new(self.atom()?),
                };
            } else if self.eat("_") {
                base = Math::Index {
                    base: Box::new(base),
                    subscript: Box::new(self.atom()?),
                };
            } else {
                return Ok(base);
            }
        }
    }

    fn atom(&mut self) -> Result<Math, Unsupported> {
        self.skip_trivia();
        let rest = self.rest();
        let Some(next) = rest.chars().next() else {
            return Err(Unsupported("expected an atom, found nothing".into()));
        };
        match next {
            '{' => {
                self.at += 1;
                let body = self.expression(&["}"])?;
                match self.eat("}") {
                    true => Ok(body),
                    false => Err(Unsupported("unclosed group".into())),
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
            _ if DELIMITERS.contains(next) => {
                self.at += next.len_utf8();
                Ok(Math::atom(next, HLGroup::Delimiter))
            }
            _ => Err(Unsupported(format!("unexpected character {next:?}"))),
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

    fn command(&mut self) -> Result<Math, Unsupported> {
        self.at += 1;
        let name = self.name();
        match name.as_str() {
            "frac" => Ok(Math::Fraction {
                numerator: Box::new(self.atom()?),
                denominator: Box::new(self.atom()?),
            }),
            "begin" => self.environment(),
            "cdot" => Ok(Math::atom(PRODUCT, HLGroup::Operator)),
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
            brace @ ("{" | "}") => Ok(Math::atom(brace, HLGroup::Delimiter)),
            "left" => self.fences(),
            // A font wrapper renames the role of what it wraps but adds no shape.
            font if FONTS.contains(&font) => {
                let body = self.atom()?;
                Ok(match font {
                    "mathbb" => Math::retag(body.doubled(), HLGroup::Type),
                    _ => body,
                })
            }
            other => Err(Unsupported(format!("\\{other}"))),
        }
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
    fn environment(&mut self) -> Result<Math, Unsupported> {
        if !self.eat("{") {
            return Err(Unsupported("\\begin without an environment".into()));
        }
        let name = self.name();
        if !self.eat("}") {
            return Err(Unsupported("unclosed environment name".into()));
        }
        let kind = match name.as_str() {
            "matrix" | "array" => GridKind::Bare,
            "pmatrix" => GridKind::Matrix,
            "cases" => GridKind::Cases,
            other => return Err(Unsupported(format!("environment {other}"))),
        };
        // `array` states a column alignment that the terminal layout does not use.
        if self.eat("{") {
            self.name();
            if !self.eat("}") {
                return Err(Unsupported("unclosed column specification".into()));
            }
        }
        let closing = format!("\\end{{{name}}}");
        let rows = self.rows(&closing)?;
        match self.eat(&closing) {
            true => Ok(Math::Grid { rows, kind }),
            false => Err(Unsupported(format!("missing {closing}"))),
        }
    }

    fn rows(&mut self, closing: &str) -> Result<Vec<Vec<Math>>, Unsupported> {
        let mut rows = vec![Vec::new()];
        loop {
            self.skip_trivia();
            if self.rest().starts_with(closing) {
                return Ok(rows);
            }
            if self.rest().is_empty() {
                return Err(Unsupported(format!("missing {closing}")));
            }
            rows.last_mut()
                .expect("a row is always open")
                .push(self.expression(&["&", "\\\\", closing])?);
            self.skip_trivia();
            if self.eat("\\\\") {
                rows.push(Vec::new());
            } else if !self.eat("&") && !self.rest().starts_with(closing) {
                return Err(Unsupported(format!("unexpected {}", self.rest())));
            }
        }
    }

    /// `\left( … \right)`, whose fences grow to the body.
    fn fences(&mut self) -> Result<Math, Unsupported> {
        self.skip_trivia();
        let left = self.fence()?;
        let body = self.expression(&["\\right"])?;
        if !self.eat("\\right") {
            return Err(Unsupported("\\left without \\right".into()));
        }
        self.skip_trivia();
        let right = self.fence()?;
        Ok(Math::Delimited {
            left,
            right,
            body: Box::new(body),
        })
    }

    fn fence(&mut self) -> Result<char, Unsupported> {
        self.skip_trivia();
        // A brace is escaped in TeX, since a bare one groups.
        self.eat("\\");
        match self.rest().chars().next() {
            Some(fence) if DELIMITERS.contains(fence) => {
                self.at += fence.len_utf8();
                Ok(fence)
            }
            Some(other) => Err(Unsupported(format!("{other:?} is not a fence"))),
            None => Err(Unsupported("expected a fence".into())),
        }
    }
}

/// U+22C5 DOT OPERATOR, whose width is the same under every terminal convention --
/// unlike U+00B7 MIDDLE DOT, which is East Asian Ambiguous and would shift a border.
const PRODUCT: &str = "\u{22C5}";
const THIN_SPACE: &str = "\\,";
/// How many digits M2 sets between the thin spaces of a long numeral.
const DIGIT_GROUP: usize = 3;
const SPACING: [&str; 5] = ["\\qquad", "\\quad", "\\;", "\\!", "\\ "];
const OPERATORS: &str = "+-=<>/*";
const DELIMITERS: &str = "()[]{}|.";
/// Font commands M2 wraps names in. All are transparent to layout: they change how a
/// name is *set*, never what it is. `mathit` marks multi-character names, which is how
/// M2 keeps `x2` from reading as a power; the parser gets that from structure instead.
const FONTS: [&str; 9] = [
    "mathbb", "mathbf", "mathcal", "mathrm", "mathfrak", "mathsf", "mathtt", "mathit", "texttt",
];

impl Math {
    /// Parse one M2 TeX expression.
    pub fn parse(source: &str) -> Result<Self, Unsupported> {
        let mut input = Input::new(source);
        let parsed = input.expression(&[])?;
        match input.rest().is_empty() {
            true => Ok(parsed),
            false => Err(Unsupported(format!("trailing input: {}", input.rest()))),
        }
    }

    fn atom(text: impl Into<String>, kind: HLGroup) -> Self {
        Self::Atom {
            text: text.into(),
            kind,
        }
    }

    /// A blackboard letter as M2 spells the ring it names: `\mathbb Q` is `QQ`, the
    /// name typed in code, rather than a lone `Q` that reads as a variable.
    fn doubled(self) -> Self {
        match self {
            Self::Atom { text, kind } if text.chars().count() == 1 => Self::Atom {
                text: text.repeat(2),
                kind,
            },
            other => other,
        }
    }

    /// Give every atom in a subtree a different role.
    fn retag(self, kind: HLGroup) -> Self {
        match self {
            Self::Atom { text, .. } => Self::Atom { text, kind },
            Self::Row(pieces) => {
                Self::Row(pieces.into_iter().map(|piece| piece.retag(kind)).collect())
            }
            other => other,
        }
    }

    /// Lay out as rows of spans around a baseline, fractions set in `fractions`.
    pub fn layout(&self, fractions: FractionStyle) -> MathBlock {
        match self {
            Self::Atom { text, kind } => MathBlock::atom(text, *kind),
            Self::Space => MathBlock::gap(),
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
                    None => {
                        let base_height = base.height();
                        let baseline = base.baseline;
                        let mut stacked = MathBlock::stack(
                            vec![base, subscript.layout(fractions)],
                            baseline,
                            Align::Left,
                        );
                        stacked.baseline = baseline.min(base_height - 1);
                        stacked
                    }
                }
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
    /// Only the outermost row breaks into lines, since that is where a long polynomial's
    /// terms live. Whatever a break cannot shorten is cut into column slabs instead.
    fn wrapped(&self, width: usize, fractions: FractionStyle) -> Vec<SpanLine> {
        let lines = match self {
            Self::Row(pieces) => MathBlock::pack(Self::chunks(pieces, fractions), width),
            other => vec![other.layout(fractions)],
        };
        MathBlock::fitted(lines, width)
    }

    /// Parse what M2's `tex` produced, when it is maths: the source between `$` signs.
    ///
    /// For a String, a Net or null M2 answers `tex` with the raw text instead, which
    /// is not maths -- read as TeX, `hello world` would set as the name `helloworld`.
    fn from_tex(source: &str) -> Result<Self, Unsupported> {
        let maths = source
            .strip_prefix('$')
            .and_then(|tex| tex.strip_suffix('$'))
            .ok_or_else(|| Unsupported("not maths: no $ delimiters".into()))?;
        Self::parse(maths)
    }

    /// A row's pieces laid out in order, split wherever a long row may break.
    ///
    /// A row breaks before a binary operator, so a continued line opens with the `+` that
    /// says it continues, and after the gap between two factors. Each chunk is everything
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
            if *piece == Self::Space && beside_binary {
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
                && pieces
                    .get(index + 1)
                    .is_some_and(|next| !next.is_binary() && *next != Self::Space);
            let chunk = chunks.last_mut().expect("a chunk is always open");
            chunk.push(piece.layout(fractions));
            if apart || runs_into_factor {
                chunk.push(MathBlock::gap());
            }
            if *piece == Self::Space {
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
        matches!(
            self,
            Self::Atom { text, kind: HLGroup::Operator }
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
        let Self::Atom { text, kind } = self else {
            return None;
        };
        shift.form(text).map(|text| MathBlock::atom(&text, *kind))
    }
}

/// Which way a script moves off its base's row.
#[derive(Clone, Copy)]
enum Shift {
    Raised,
    Lowered,
}

impl Shift {
    /// `text` in its shifted form, when every character of it has one. A script that
    /// does rides its base's row, which keeps polynomials compact.
    fn form(self, text: &str) -> Option<String> {
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

    fn into_rows(self) -> Vec<SpanLine> {
        self.rows
    }

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
    /// that text.
    fn value(value: &M2Value, fractions: FractionStyle) -> Self {
        match value {
            M2Value::Tex { source, fallback } => Math::from_tex(source).map_or_else(
                |_| Self::text(fallback, HLGroup::Value),
                |math| math.layout(fractions),
            ),
            M2Value::Net(text) | M2Value::Markup { fallback: text, .. } => {
                Self::text(text, HLGroup::Value)
            }
            M2Value::Token { text, role } => Self::text(text, role.group()),
            M2Value::Collection {
                kind: CollectionKind::VerticalList,
                items,
            } => Self::braced(
                items
                    .iter()
                    .map(|item| Self::value(item, fractions))
                    .collect(),
            ),
            M2Value::Collection { kind, items } => {
                Self::listed(*kind, items, usize::MAX, fractions)
                    .into_iter()
                    .next()
                    .expect("an unbounded width takes a collection on one line")
            }
            M2Value::HashTable(entries) => Self::braced(
                entries
                    .iter()
                    .map(|[key, value]| {
                        Self::row([
                            Self::value(key, fractions),
                            Self::atom(" \u{21D2} ", HLGroup::Operator),
                            Self::value(value, fractions),
                        ])
                    })
                    .collect(),
            ),
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
        items: &[M2Value],
        width: usize,
        fractions: FractionStyle,
    ) -> Vec<Self> {
        let (open, close) = kind.delimiters();
        let last = items.len().saturating_sub(1);
        let entries = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let entry = Self::value(item, fractions);
                match index == last {
                    true => entry,
                    false => Self::row([entry, Self::atom(",", HLGroup::Punctuation), Self::gap()]),
                }
            })
            .collect();
        let indent = UnicodeWidthStr::width(open);
        let budget = width.saturating_sub(indent + UnicodeWidthStr::width(close));
        let mut lines = Self::pack(entries, budget);
        if lines.is_empty() {
            lines.push(Self::empty());
        }
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

    /// Rows set one under another between braces, as M2 sets a vertical list and a
    /// hash table.
    fn braced(rows: Vec<Self>) -> Self {
        if rows.is_empty() {
            return Self::atom("{}", HLGroup::Delimiter);
        }
        let height: usize = rows.iter().map(Self::height).sum();
        let body = Self::stack(rows, height.saturating_sub(1) / 2, Align::Left);
        Self::row([Self::gap(), body, Self::gap()]).fenced("{", "}")
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
    /// wider than the screen.
    fn slabs(self, width: usize) -> Vec<Self> {
        let total = self.visible_width();
        if total <= width || width == 0 {
            return vec![self];
        }
        (0..total)
            .step_by(width)
            .map(|from| Self {
                rows: self
                    .rows
                    .iter()
                    .map(|row| Self::columns(row, from..from + width))
                    .collect(),
                baseline: self.baseline,
            })
            .collect()
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

    /// Set blocks one under another, nominating which row aligns with neighbours.
    fn stack(blocks: Vec<Self>, baseline: usize, align: Align) -> Self {
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
    /// A bracket grows from its top, extension and bottom pieces, a brace adding the
    /// tip that points at the baseline. Two rows leave no room for a tip, so a brace
    /// that short is drawn from its two halves instead; without either it would be a
    /// parenthesis. These families measure one column under every terminal convention,
    /// unlike the box-drawing set, so a grown matrix keeps its shape wherever it is
    /// read. Any other fence -- M2's `<|` -- stays one row high on the baseline, and
    /// TeX's `.` marks the absence of one.
    fn fence(fence: &str, height: usize, baseline: usize) -> Self {
        let [top, extension, bottom, tip] = match (fence, height) {
            (".", _) => return Self::empty(),
            (_, 1) => return Self::atom(fence, HLGroup::Delimiter),
            ("{", 2) => ["\u{23B0}", "", "\u{23B1}", ""],
            ("}", 2) => ["\u{23B1}", "", "\u{23B0}", ""],
            ("(", _) => ["\u{239B}", "\u{239C}", "\u{239D}", "\u{239C}"],
            (")", _) => ["\u{239E}", "\u{239F}", "\u{23A0}", "\u{239F}"],
            ("[", _) => ["\u{23A1}", "\u{23A2}", "\u{23A3}", "\u{23A2}"],
            ("]", _) => ["\u{23A4}", "\u{23A5}", "\u{23A6}", "\u{23A5}"],
            ("{", _) => ["\u{23A7}", "\u{23AA}", "\u{23A9}", "\u{23A8}"],
            ("}", _) => ["\u{23AB}", "\u{23AA}", "\u{23AD}", "\u{23AC}"],
            ("|", _) => ["\u{23AA}"; 4],
            _ => return Self::atom(fence, HLGroup::Delimiter),
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
                    vec![TextSpan::new(glyph, HLGroup::Delimiter)]
                })
                .collect(),
            baseline,
        }
    }

    /// Entries aligned into columns, fenced by whatever the grid's kind calls for.
    fn grid(rows: &[Vec<Math>], kind: GridKind, fractions: FractionStyle) -> Self {
        let laid: Vec<Vec<Self>> = rows
            .iter()
            .map(|row| row.iter().map(|entry| entry.layout(fractions)).collect())
            .collect();
        let columns = laid.iter().map(Vec::len).max().unwrap_or(0);
        let widths: Vec<usize> = (0..columns)
            .map(|column| {
                laid.iter()
                    .filter_map(|row| row.get(column))
                    .map(Self::width)
                    .max()
                    .unwrap_or(0)
            })
            .collect();

        let column_gap = || Self::atom("  ", HLGroup::Canvas);
        let stacked: Vec<Self> = laid
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .enumerate()
                    .map(|(column, entry)| entry.pad(widths[column], Align::Centre))
                    .reduce(|left, right| left.beside(column_gap()).beside(right))
                    .unwrap_or_else(Self::empty)
            })
            .collect();

        let height: usize = stacked.iter().map(Self::height).sum();
        let body = Self::stack(stacked, height.saturating_sub(1) / 2, Align::Left);
        match kind {
            GridKind::Bare => body,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(text: &str, kind: HLGroup) -> Math {
        Math::atom(text, kind)
    }

    #[test]
    fn a_polynomial_separates_numerals_names_and_operators() {
        let parsed = Math::parse("x^{2} + 3 y").expect("a polynomial parses");

        assert_eq!(
            parsed,
            Math::Row(vec![
                Math::Power {
                    base: Box::new(atom("x", HLGroup::Variable)),
                    exponent: Box::new(atom("2", HLGroup::Number)),
                },
                atom("+", HLGroup::Operator),
                atom("3", HLGroup::Number),
                atom("y", HLGroup::Variable),
            ])
        );
    }

    #[test]
    fn a_fraction_keeps_its_halves_apart() {
        let parsed = Math::parse(r"\frac{1}{3}").expect("a fraction parses");

        assert_eq!(
            parsed,
            Math::Fraction {
                numerator: Box::new(atom("1", HLGroup::Number)),
                denominator: Box::new(atom("3", HLGroup::Number)),
            }
        );
    }

    #[test]
    fn a_matrix_keeps_its_rows_and_columns() {
        let parsed = Math::parse(r"\begin{matrix}1&x\\y&2\end{matrix}").expect("a matrix parses");

        let Math::Grid { rows, kind } = parsed else {
            panic!("expected a grid");
        };
        assert_eq!(kind, GridKind::Bare);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            vec![atom("1", HLGroup::Number), atom("x", HLGroup::Variable)]
        );
        assert_eq!(
            rows[1],
            vec![atom("y", HLGroup::Variable), atom("2", HLGroup::Number)]
        );
    }

    #[test]
    fn cases_are_a_grid_that_knows_it_wants_one_brace() {
        let parsed = Math::parse(r"\begin{cases}1\\2\end{cases}").expect("cases parse");

        let Math::Grid { rows, kind } = parsed else {
            panic!("expected a grid");
        };
        assert_eq!(kind, GridKind::Cases);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn grown_fences_survive_around_their_body() {
        let parsed = Math::parse(r"\left(\!\begin{matrix}1\end{matrix}\!\right)")
            .expect("fenced matrix parses");

        let Math::Delimited { left, right, body } = parsed else {
            panic!("expected delimiters");
        };
        assert_eq!((left, right), ('(', ')'));
        assert!(matches!(*body, Math::Grid { .. }));
    }

    #[test]
    fn blackboard_bold_is_a_type_rather_than_a_variable() {
        let parsed = Math::parse(r"\mathbb{Z}").expect("a ring parses");

        assert_eq!(parsed, atom("ZZ", HLGroup::Type));
    }

    /// The three shapes `backend.rs` currently rewrites with regexes before rendering.
    #[test]
    fn the_forms_macaulay2_emits_need_no_rewriting_first() {
        // `\mathit{…}` around a multi-character name.
        assert_eq!(
            Math::parse(r"\mathit{x2}").expect("mathit parses"),
            atom("x2", HLGroup::Variable)
        );
        // A font command inside braces, the spelling M2 actually uses.
        assert_eq!(
            Math::parse(r"{\mathbb ZZ}").expect("a braced font parses"),
            atom("ZZ", HLGroup::Type)
        );
        // `array` with its column specification, rather than `matrix`.
        let parsed = Math::parse(r"\begin{array}{cc}1&2\\3&4\end{array}").expect("an array parses");
        assert!(matches!(
            parsed,
            Math::Grid {
                kind: GridKind::Bare,
                ..
            }
        ));
    }

    #[test]
    fn an_unknown_command_is_refused_by_name() {
        assert_eq!(
            Math::parse(r"\sqrt{2}").err(),
            Some(Unsupported(r"\sqrt".into()))
        );
    }

    /// The shape M2 actually emits, after the caller's `array`/`mathit` normalisation.
    #[test]
    fn a_real_matrix_from_macaulay2_parses_whole() {
        let source = "\\left(\\!\\begin{matrix}{cc}\n1&x\\\\\nx^{2}+1&\\frac{1}{3}x2\n\\end{matrix}\\!\\right)";

        let Math::Delimited { left, right, body } =
            Math::parse(source).expect("M2's own matrix parses")
        else {
            panic!("expected fences");
        };
        assert_eq!((left, right), ('(', ')'));

        let Math::Grid { rows, kind } = *body else {
            panic!("expected a grid");
        };
        assert_eq!(kind, GridKind::Bare);
        assert_eq!(rows.len(), 2);
        // The entry that the Net spelling cannot distinguish from `x2`.
        assert_eq!(
            rows[1][0],
            Math::Row(vec![
                Math::Power {
                    base: Box::new(atom("x", HLGroup::Variable)),
                    exponent: Box::new(atom("2", HLGroup::Number)),
                },
                atom("+", HLGroup::Operator),
                atom("1", HLGroup::Number),
            ])
        );
        // And the fraction beside it keeps its own halves.
        assert!(matches!(rows[1][1], Math::Row(ref pieces)
            if matches!(pieces[0], Math::Fraction { .. })));
    }

    /// M2 spells a product as a thin space, which renders as the space it is; only an
    /// explicit `\cdot` draws a dot.
    #[test]
    fn a_thin_space_product_keeps_its_factors_apart_without_a_dot() {
        assert_eq!(
            Math::parse(r"xx\,yy").expect("a thin-space product parses"),
            Math::Row(vec![
                atom("xx", HLGroup::Variable),
                Math::Space,
                atom("yy", HLGroup::Variable),
            ])
        );
        assert_eq!(picture(r"3\,x\,y^{2}"), ["3 x y\u{00B2}"]);
        assert_eq!(picture(r"xx \cdot yy"), ["xx\u{22C5}yy"]);
        // And the variable of the same letters stays one atom, not a product.
        assert_eq!(
            Math::parse("xxyy").unwrap(),
            atom("xxyy", HLGroup::Variable)
        );
    }

    /// M2 groups a long integer's digits in threes with thin spaces; the groups are
    /// one numeral, and the space after one still separates it from the next factor.
    #[test]
    fn a_digit_grouped_numeral_is_one_number() {
        assert_eq!(
            Math::parse(r"1\,267\,650\,600").expect("a grouped numeral parses"),
            atom("1267650600", HLGroup::Number)
        );
        assert_eq!(picture(r"12\,345\,678\,x^{2}y"), ["12345678 x\u{00B2}y"]);
        // A group of any other length is a second factor, not more digits.
        assert_eq!(picture(r"12\,5"), ["12 5"]);
    }

    /// `rows` as plain text, one string per terminal row.
    fn text(rows: &[SpanLine]) -> Vec<String> {
        rows.iter()
            .map(|row| row.iter().map(|span| span.content.as_str()).collect())
            .collect()
    }

    fn wrapped_picture(math: &str, width: usize) -> Vec<String> {
        text(
            &Math::parse(math)
                .expect("parses")
                .wrapped(width, FractionStyle::Stacked),
        )
    }

    /// Whatever the breaking policy, a wrapped row fits and loses nothing.
    #[test]
    fn a_long_polynomial_wraps_within_the_width() {
        let source = r"-7\,x\,y+12\,x^{3}y-5\,y^{4}+8\,x\,y\,z-3\,z^{2}+1";
        let whole = picture(source).concat();

        for width in [6, 10, 16, 25] {
            let rows = wrapped_picture(source, width);
            assert!(
                rows.iter()
                    .all(|row| row.trim_end().chars().count() <= width),
                "{rows:?} overflows {width}"
            );
            assert_eq!(
                rows.concat().replace(' ', ""),
                whole.replace(' ', ""),
                "wrapping to {width} must keep every glyph, in order"
            );
        }
        assert_eq!(wrapped_picture(source, 80), [whole]);
    }

    /// Lines break between terms, never inside one: a continued line opens with its
    /// operator and no line ends on one. Slab cutting alone fits the width too, so
    /// this is what tells a packed row from a merely cut one.
    #[test]
    fn a_wrapped_sum_breaks_between_terms() {
        let rows = wrapped_picture("a+b+c+d+e+f+g+h", 7);

        assert!(rows.len() > 1, "{rows:?} did not wrap");
        assert!(rows[1..].iter().all(|row| row.starts_with('+')), "{rows:?}");
        assert!(
            rows.iter().all(|row| !row.trim_end().ends_with('+')),
            "{rows:?}"
        );
    }

    /// A value as M2 sends it for anything with a TeX form.
    fn tex(maths: &str) -> M2Value {
        M2Value::Tex {
            source: format!("${maths}$"),
            fallback: maths.into(),
        }
    }

    fn collection(kind: CollectionKind, items: Vec<M2Value>) -> M2Value {
        M2Value::Collection { kind, items }
    }

    /// Inside a container a number is still a number: entries are rendered one by
    /// one, each in its own roles, rather than as the container's flattened text.
    #[test]
    fn a_collection_colours_each_entry_by_its_own_role() {
        let list = collection(CollectionKind::List, vec![tex("1"), tex("x^{2}")]);
        let rows = spans(&list, 80, FractionStyle::Stacked).expect("a list renders");

        assert_eq!(text(&rows), ["{1, x\u{00B2}}"]);
        let roles: Vec<_> = rows[0]
            .iter()
            .map(|span| (span.content.as_str(), span.kind))
            .collect();
        for role in [
            ("{", HLGroup::Delimiter),
            ("1", HLGroup::Number),
            ("x", HLGroup::Variable),
        ] {
            assert!(roles.contains(&role), "{role:?} missing from {roles:?}");
        }
    }

    #[test]
    fn each_kind_keeps_its_delimiters_and_nests() {
        let sequence = collection(
            CollectionKind::Sequence,
            vec![
                tex("1"),
                collection(CollectionKind::Array, vec![tex("2"), tex("3")]),
                collection(CollectionKind::AngleBarList, Vec::new()),
            ],
        );

        assert_eq!(
            text(&spans(&sequence, 80, FractionStyle::Stacked).expect("renders")),
            ["(1, [2, 3], <||>)"]
        );
    }

    #[test]
    fn a_hash_table_sets_one_entry_per_row_between_braces() {
        let table = M2Value::HashTable(vec![
            [tex("1"), tex("2")],
            [tex("3"), M2Value::Net("text".into())],
        ]);

        assert_eq!(
            text(&spans(&table, 80, FractionStyle::Stacked).expect("renders")),
            [
                "\u{23B0} 1 \u{21D2} 2    \u{23B1}",
                "\u{23B1} 3 \u{21D2} text \u{23B0}"
            ]
        );
    }

    /// M2 answers `tex` for a String with the raw string, which is not maths.
    #[test]
    fn a_string_is_never_read_as_maths() {
        let string = M2Value::Tex {
            source: "hello world".into(),
            fallback: "hello world".into(),
        };

        assert!(
            spans(&string, 80, FractionStyle::Stacked).is_err(),
            "left to the plain text path"
        );
        let rows = spans(
            &collection(CollectionKind::List, vec![string]),
            80,
            FractionStyle::Stacked,
        )
        .expect("renders");
        assert_eq!(text(&rows), ["{hello world}"]);
        assert!(rows[0]
            .iter()
            .any(|span| span.content == "hello world" && span.kind == HLGroup::Value));
    }

    /// A long list breaks between entries, each line ending on its comma.
    #[test]
    fn a_long_list_wraps_between_entries() {
        let items = (1..=12).map(|item| tex(&item.to_string())).collect();
        let rows = text(
            &spans(
                &collection(CollectionKind::List, items),
                10,
                FractionStyle::Stacked,
            )
            .expect("renders"),
        );

        assert!(rows.len() > 1, "{rows:?} did not wrap");
        assert!(
            rows.iter()
                .all(|row| matches!(row.trim_end().chars().last(), Some(',' | '}'))),
            "{rows:?}"
        );
    }

    /// Greedy packing, pinned: each line takes terms until the next would overflow,
    /// and a line's closing gap is never what pushes a term onto the next one.
    #[test]
    fn packing_fills_each_line_before_breaking() {
        assert_eq!(
            wrapped_picture("a+b+c+d+e+f+g+h", 7),
            ["a + b", "+ c + d", "+ e + f", "+ g + h"]
        );
        let items = (1..=12).map(|item| tex(&item.to_string())).collect();
        assert_eq!(
            text(
                &spans(
                    &collection(CollectionKind::List, items),
                    10,
                    FractionStyle::Stacked
                )
                .expect("renders")
            ),
            ["{1, 2, 3,", " 4, 5, 6,", " 7, 8, 9,", " 10, 11,", " 12}"]
        );
    }

    fn token(text: &str, role: Role) -> M2Value {
        M2Value::Token {
            text: text.into(),
            role,
        }
    }

    /// The group each span of a single-row rendering was given, by its text.
    fn roles(rows: &[SpanLine]) -> Vec<(String, HLGroup)> {
        rows.iter()
            .flatten()
            .map(|span| (span.content.clone(), span.kind))
            .collect()
    }

    /// A token reads as it would typed: each role takes the group code gives it.
    #[test]
    fn a_token_is_coloured_as_code_colours_its_role() {
        let list = collection(
            CollectionKind::List,
            vec![
                token("null", Role::Null),
                token("true", Role::Boolean),
                token("hi", Role::String),
                token("print", Role::Function),
            ],
        );
        let roles = roles(&spans(&list, 80, FractionStyle::Stacked).expect("renders"));

        for (text, group) in [
            ("null", HLGroup::Constant),
            ("true", HLGroup::Boolean),
            ("hi", HLGroup::String),
            ("print", HLGroup::Function),
        ] {
            assert!(
                roles.contains(&(text.to_string(), group)),
                "{text} should be {group:?} in {roles:?}"
            );
        }
    }

    /// A name in output is a symbol, coloured as code colours a symbol.
    #[test]
    fn a_name_is_coloured_as_a_symbol_in_code() {
        let rows = spans(&tex("x^{2}+q"), 80, FractionStyle::Stacked).expect("renders");

        assert!(roles(&rows).contains(&("q".to_string(), HLGroup::Variable)));
    }

    fn fraction_picture(math: &str, fractions: FractionStyle) -> Vec<String> {
        text(&spans(&tex(math), 80, fractions).expect("renders"))
    }

    #[test]
    fn a_fraction_sets_in_the_chosen_style() {
        let fraction = r"x+\frac{1}{3}";

        assert_eq!(
            fraction_picture(fraction, FractionStyle::Stacked),
            ["    1", "x + \u{2500}", "    3"]
        );
        assert_eq!(
            fraction_picture(fraction, FractionStyle::Underlined),
            ["x + 1", "    3"]
        );
        assert_eq!(
            fraction_picture(fraction, FractionStyle::Inline),
            ["x + 1/3"]
        );
    }

    /// An inline fraction parenthesises a compound side, and keeps apart from a
    /// factor after it, so neither `x+1/2` nor `1/3x` misreads.
    #[test]
    fn an_inline_fraction_cannot_be_misread() {
        assert_eq!(
            fraction_picture(r"\frac{x+1}{2}", FractionStyle::Inline),
            ["(x + 1)/2"]
        );
        assert_eq!(
            fraction_picture(r"\frac{1}{3}x", FractionStyle::Inline),
            ["1/3 x"]
        );
    }

    /// The rule spans the whole fraction, padding included, so a numerator narrower
    /// than its denominator is still wholly over the rule, and the rule is the only
    /// thing marked.
    #[test]
    fn an_underlined_fraction_rules_its_full_width() {
        let rows = spans(&tex(r"\frac{3}{100}"), 80, FractionStyle::Underlined).expect("renders");

        assert_eq!(text(&rows), [" 3 ", "100"]);
        assert!(rows[0]
            .iter()
            .all(|span| span.mark == Some(HLGroup::FractionRule)));
        assert!(rows[1].iter().all(|span| span.mark.is_none()));
    }

    /// Painted into the terminal, the rule is the numerator's underline in the rule's
    /// special colour, over the numerator's own colours.
    #[test]
    fn an_underlined_rule_paints_in_its_special_colour() {
        use crate::ui::block::Text;
        use crate::ui::controller::settings::theme::groups::Groups;
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::Modifier;
        use ratatui::text::Line;
        use ratatui::widgets::{Paragraph, Widget};

        let rows = spans(&tex(r"\frac{1}{3}"), 80, FractionStyle::Underlined).expect("renders");
        let groups = Groups::default();
        let area = Rect::new(0, 0, 1, 2);
        let mut cells = Buffer::empty(area);
        Paragraph::new(
            rows.iter()
                .map(|row| Line::from(row.spans(&groups)))
                .collect::<Vec<_>>(),
        )
        .render(area, &mut cells);

        let numerator = &cells[(0, 0)];
        assert_eq!(numerator.symbol(), "1");
        assert!(numerator.modifier.contains(Modifier::UNDERLINED));
        assert_eq!(
            Some(numerator.underline_color),
            groups.FractionRule.underline_color
        );
        assert_eq!(Some(numerator.fg), groups.Number.fg);
        assert!(!cells[(0, 1)].modifier.contains(Modifier::UNDERLINED));
    }

    /// A container's brackets grow to the tallest entry on their line.
    #[test]
    fn a_container_grows_its_brackets_around_a_tall_entry() {
        let sequence = collection(
            CollectionKind::Sequence,
            vec![tex(r"\frac{1}{2}"), tex("3")],
        );

        assert_eq!(
            text(&spans(&sequence, 80, FractionStyle::Stacked).expect("renders")),
            [
                "\u{239B}1   \u{239E}",
                "\u{239C}\u{2500}, 3\u{239F}",
                "\u{239D}2   \u{23A0}"
            ]
        );
    }

    /// A numeral has no break inside it, so one wider than the screen is cut.
    #[test]
    fn a_numeral_wider_than_the_screen_is_cut_into_slabs() {
        assert_eq!(
            wrapped_picture(r"1\,267\,650\,600\,228", 5),
            ["12676", "50600", "228"]
        );
    }

    /// The rendered rows as plain text, for reading a layout in a failure message.
    fn picture(math: &str) -> Vec<String> {
        Math::parse(math)
            .expect("parses")
            .layout(FractionStyle::Stacked)
            .rows()
            .iter()
            .map(|row| row.iter().map(|span| span.content.as_str()).collect())
            .collect()
    }

    #[test]
    fn a_simple_power_stays_on_one_row() {
        assert_eq!(picture("x^{2} + 1"), ["x\u{00B2} + 1"]);
        assert_eq!(picture("x^{10}"), ["x\u{00B9}\u{2070}"]);
    }

    #[test]
    fn a_power_without_superscripts_is_written_with_a_caret() {
        assert_eq!(picture("x^{x}"), ["x^x"]);
        assert_eq!(picture("x^{y+1} + 1"), ["x^(y + 1) + 1"]);
        assert_eq!(picture(r"x^{\frac{1}{2}}"), ["x^(1/2)"]);
    }

    #[test]
    fn a_fraction_puts_its_rule_on_the_baseline() {
        let block = Math::parse(r"\frac{1}{3}")
            .expect("parses")
            .layout(FractionStyle::Stacked);

        assert_eq!(picture(r"\frac{1}{3}"), ["1", "\u{2500}", "3"]);
        assert_eq!(block.baseline(), 1, "the rule is what neighbours align to");
    }

    /// The alignment that a naive implementation gets wrong.
    #[test]
    fn a_term_beside_a_fraction_sits_level_with_the_rule() {
        assert_eq!(
            picture(r"x + \frac{1}{3}"),
            ["    1", "x + \u{2500}", "    3"]
        );
    }

    #[test]
    fn a_fraction_centres_the_shorter_half() {
        assert_eq!(
            picture(r"\frac{1}{100}"),
            [" 1 ", "\u{2500}\u{2500}\u{2500}", "100"]
        );
    }

    #[test]
    fn an_environment_that_fences_itself_grows_those_fences() {
        assert_eq!(
            picture(r"\begin{pmatrix}1&x\\y&2\end{pmatrix}"),
            ["\u{239B}1  x\u{239E}", "\u{239D}y  2\u{23A0}"]
        );
    }

    /// The shape M2 sends: its own `\left( … \right)` around a bare grid.
    ///
    /// `matrix` and `array` carry no fences in LaTeX, so supplying some here as well
    /// drew every matrix inside two sets of brackets.
    #[test]
    fn a_fenced_bare_grid_gets_exactly_one_set_of_brackets() {
        assert_eq!(
            picture(r"\left(\!\begin{matrix}1&x\\y&2\end{matrix}\!\right)"),
            ["\u{239B}1  x\u{239E}", "\u{239D}y  2\u{23A0}"]
        );
        // Bare on its own, exactly as LaTeX renders it.
        assert_eq!(
            picture(r"\begin{matrix}1&x\\y&2\end{matrix}"),
            ["1  x", "y  2"]
        );
    }

    #[test]
    fn cases_take_one_brace_on_the_left() {
        let rows = picture(r"\begin{cases}1\\2\end{cases}");

        assert_eq!(rows, ["\u{23B0}1", "\u{23B1}2"]);
    }

    /// A brace is told from a parenthesis by its tip, or at two rows by its halves.
    #[test]
    fn a_brace_never_draws_as_a_parenthesis() {
        assert_eq!(
            picture(r"\begin{cases}1\\2\\3\end{cases}"),
            ["\u{23A7}1", "\u{23A8}2", "\u{23A9}3"]
        );
        assert_ne!(
            picture(r"\left\{\begin{matrix}1\\2\end{matrix}\right\}"),
            picture(r"\left(\begin{matrix}1\\2\end{matrix}\right)")
        );
    }

    #[test]
    fn columns_are_padded_to_their_widest_entry() {
        assert_eq!(
            picture(r"\begin{matrix}1&x\\x^{2}+1&y\end{matrix}"),
            ["  1     x", "x\u{00B2} + 1  y"]
        );
    }

    /// The shape of a map between free modules, as a matrix's type reports it.
    #[test]
    fn arrows_render_long_and_stand_apart_from_their_operands() {
        assert_eq!(
            picture(r"R^{2} \longleftarrow R^{2}"),
            ["R\u{00B2} \u{27F5} R\u{00B2}"]
        );
        // The short spelling renders identically: the long glyph is width-stable.
        assert_eq!(picture(r"R \leftarrow R"), picture(r"R \longleftarrow R"));
    }

    /// M2's apply-to-list operator survives a round trip through TeX.
    ///
    /// This is what makes `contains('\\')` unsafe as a failure test: a value holding
    /// `f \ L` renders a genuine backslash, and sniffing for one would discard a
    /// perfectly good rendering.
    #[test]
    fn a_backslash_operator_renders_rather_than_signalling_failure() {
        assert_eq!(picture(r"f \backslash L"), [r"f \ L"]);
        assert_eq!(picture(r"A \setminus B"), ["A \u{2216} B"]);
    }

    /// Complex scripts render; only *named symbols* fall back, and they say which.
    ///
    /// The space of TeX M2 might emit cannot be enumerated in advance, so the parser
    /// is built to report rather than to cover: every refusal carries the construct
    /// that caused it, which is what lets real use produce the list of what to add.
    #[test]
    fn what_is_not_handled_names_itself() {
        for renders in [r"x^{2}", r"x^{a}", r"x^{n+1}", r"x_{1}", r"\frac{a}{b+c}"] {
            assert!(Math::parse(renders).is_ok(), "{renders} should render");
        }
        for (source, blamed) in [
            (r"x^{\alpha}", r"\alpha"),
            (r"a \otimes b", r"\otimes"),
            (r"\sqrt{2}", r"\sqrt"),
        ] {
            assert_eq!(
                Math::parse(source).err(),
                Some(Unsupported(blamed.into())),
                "{source} should blame {blamed} rather than failing anonymously"
            );
        }
    }
}
