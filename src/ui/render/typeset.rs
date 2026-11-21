//! Typesetting the drawings M2 sends, with Unicode fallback.

use ratatui::style::{Color, Modifier};
use ratatui::text::Line;

use crate::backend::Shape;
use crate::ui::block::{Link, Text};
use crate::ui::graphics::{self, ratex::Rgb, Pictured, Slicing, Typesetting};
use crate::ui::present;
use crate::ui::theme::groups::{Groups, HLGroup};

use super::maths::{
    self, Course, Diagram, Math, MathInk, MathStyle, Reading, DIRECT_SUM_SIGN, PRODUCT_SIGN,
    TENSOR_SIGN,
};
use crate::ui::graphics::ratex::{
    diagram::{ArrowLengths, Description, MIN_ARROW},
    BREAK_MARK,
};

/// Rendered rows with their interactive links and chosen representation.
pub struct Drawn {
    /// Terminal rows, including picture placeholders when available.
    pub lines: Vec<Line<'static>>,
    /// Links positioned in the rendered rows.
    pub links: Vec<Link>,
}

type Picture = fn(Typesetting, Rgb, u16) -> Option<Pictured>;

/// Where TeX being written lies: in the formula's top-level row, which may
/// break into rows as the Unicode layout's does, or held whole inside a matrix,
/// a script or brackets of its own. The brackets around the whole formula, as
/// of a list, hold their row at the top: a long list breaks between its items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Depth {
    Top,
    Held,
}

impl Depth {
    /// Where `part` of the top-level row lies: a row nested in it runs on at the
    /// top, anything else holds its own parts whole.
    fn of_part(part: &Math) -> Self {
        match part {
            Math::Row(_) => Self::Top,
            Math::Linked { body, .. } => Self::of_part(body),
            _ => Self::Held,
        }
    }
}

/// TeX's script size beside its text size, which an extensible arrow sets its
/// label at.
const SCRIPT_SCALE: f64 = 0.7;
/// Ems an extensible arrow runs past its label, both ends together.
const ARROW_OVERHANG: f64 = 0.7;

/// What writes a value's TeX: the colours its atoms are painted in, none when
/// it is written to be measured alone, and how long its arrows run.
#[derive(Clone, Copy)]
struct Writer<'a> {
    groups: Option<&'a Groups>,
    arrows: ArrowLengths,
}

impl Writer<'_> {
    fn paint(&self, text: String, kind: HLGroup) -> String {
        match self.groups.and_then(|groups| groups.style(kind).fg) {
            Some(Color::Rgb(r, g, b)) => format!(r"{{\color{{#{r:02x}{g:02x}{b:02x}}}{text}}}"),
            _ => text,
        }
    }

    /// What a value is set from: its TeX, or a diagram's description when the value
    /// is a diagram, which TeX alone cannot draw.
    fn formula(&self, math: &Math) -> String {
        let Math::Diagram(diagram) = math else {
            return self.tex_at(math, Depth::Top);
        };
        let ink = self
            .groups
            .and_then(|groups| groups.style(HLGroup::MapArrow).fg)
            .and_then(|colour| match colour {
                Color::Rgb(r, g, b) => Some(Rgb([r, g, b])),
                _ => None,
            });
        let mut description = Description::new(ink, self.arrows);
        for (place, piece) in &diagram.pieces {
            // A degree numbering a column is set small, as one under a module of
            // a complex is.
            let tex = match piece.kind() {
                Some(HLGroup::Degree) => format!(r"{{\scriptsize {}}}", self.tex(piece)),
                _ => self.tex(piece),
            };
            description.piece((place.row, place.column), &tex);
        }
        for arrow in &diagram.arrows {
            let label = arrow.label.as_ref().map(|label| self.tex(label));
            description.arrow(
                (arrow.from.row, arrow.from.column),
                (arrow.to.row, arrow.to.column),
                label.as_deref(),
            );
        }
        description.into()
    }

    /// A diagram inside other maths, as TeX can set it: its pieces in an array, an
    /// arrow between rows a symbol beside the column it leaves.
    fn diagram_array(&self, diagram: &Diagram) -> String {
        let recurse = |part| self.tex(part);
        let rows = 2 * diagram.rows().max(1) - 1;
        let columns = 2 * diagram.columns().max(1) - 1;
        let mut cells = vec![vec![String::new(); columns]; rows];
        for (place, piece) in &diagram.pieces {
            cells[2 * place.row][2 * place.column] = recurse(piece);
        }
        for arrow in &diagram.arrows {
            let label = arrow.label.as_ref().map(recurse).unwrap_or_default();
            let (row, column) = (2 * arrow.from.row, 2 * arrow.from.column);
            match arrow.course() {
                Some(Course::Across) => {
                    let command = match arrow.to.column < arrow.from.column {
                        true => r"\xleftarrow",
                        false => r"\xrightarrow",
                    };
                    let between = 2 * arrow.from.column.min(arrow.to.column) + 1;
                    cells[row][between] = self.arrow(command, &label);
                }
                Some(Course::Down { shift }) => {
                    let (symbol, beside) = match shift.signum() {
                        0 => (r"\Big\downarrow", column),
                        -1 => (r"\swarrow", column.saturating_sub(1)),
                        _ => (r"\searrow", (column + 1).min(columns - 1)),
                    };
                    cells[row + 1][beside] = format!(r"{symbol}{{\scriptstyle {label}}}");
                }
                None => {}
            }
        }
        let body = cells
            .into_iter()
            .map(|row| row.join(" & "))
            .collect::<Vec<_>>()
            .join(r" \\ ");
        format!(
            r"\begin{{array}}{{{}}}{body}\end{{array}}",
            "c".repeat(columns)
        )
    }

    /// A space of `ems` of the text's size inside an extensible arrow's label.
    fn kern(ems: f64) -> String {
        format!(r"\kern{{{:.3}em}}", ems / SCRIPT_SCALE)
    }

    /// An arrow along a row set by `command`, one of TeX's extensible arrows,
    /// over `label`: as long as its label makes it, and longer by what
    /// `self.arrows` adds, half at each end. TeX keeps an arrow shortened no
    /// shorter than its label.
    fn arrow(&self, command: &str, label: &str) -> String {
        let pad = match self.arrows.gain_across() {
            gain if gain > 0.0 => Self::kern(gain / 2.0),
            _ => String::new(),
        };
        format!(r"{command}{{{pad}{{\scriptsize {label}}}{pad}}}")
    }

    /// What scripts are set on: an atom bare, so TeX sets a subscript against
    /// the glyph, under the lean of an italic letter, and anything longer in a
    /// group of its own, whose scripts stand past its whole width.
    fn nucleus(&self, base: &Math) -> String {
        match base {
            Math::Atom { .. } | Math::Native { .. } => self.tex(base),
            _ => format!("{{{}}}", self.tex(base)),
        }
    }

    fn tex(&self, math: &Math) -> String {
        self.tex_at(math, Depth::Held)
    }

    fn tex_at(&self, math: &Math, depth: Depth) -> String {
        let recurse = |part| self.tex(part);
        match math {
            Math::Diagram(diagram) => self.diagram_array(diagram),
            Math::Atom { text, kind, color } => {
                let content = match (kind, text.as_str()) {
                    (HLGroup::MapArrow, "⟵") if self.arrows.gain_across() > 0.0 => {
                        let label = MIN_ARROW + self.arrows.gain_across() - ARROW_OVERHANG;
                        format!(r"\xleftarrow{{{}}}", Self::kern(label))
                    }
                    (HLGroup::MapArrow | HLGroup::Operator, "⟵") => r"\longleftarrow".into(),
                    (HLGroup::Delimiter, "," | ", " | ";") => text.clone(),
                    (HLGroup::Operator, PRODUCT_SIGN) => r"\cdot ".into(),
                    (HLGroup::Operator, TENSOR_SIGN) => r"\otimes ".into(),
                    (HLGroup::Operator, DIRECT_SUM_SIGN) => r"\oplus ".into(),
                    (HLGroup::Operator, "+" | "-" | "=") => text.clone(),
                    // Numerals and names are set in TeX's own maths face, letters italic
                    // and digits upright, whatever a name holds: a ring's, coloured as a
                    // type, is set as x is.
                    (HLGroup::Number | HLGroup::Variable | HLGroup::Type, _)
                        if *kind != HLGroup::Type || text.chars().all(char::is_alphanumeric) =>
                    {
                        maths::in_face("mathnormal", text)
                    }
                    _ => {
                        let content = maths::typewriter(text);
                        if self.groups.is_some_and(|groups| {
                            groups.style(*kind).add_modifier.contains(Modifier::ITALIC)
                        }) {
                            content.replacen("texttt", "textit", 1)
                        } else {
                            content
                        }
                    }
                };
                self.paint(content, color.unwrap_or(*kind))
            }
            Math::Over { base, label } => {
                if matches!(
                    base.as_ref(),
                    Math::Atom {
                        kind: HLGroup::MapArrow,
                        ..
                    }
                ) {
                    return self.paint(
                        self.arrow(r"\xleftarrow", &recurse(label)),
                        HLGroup::MapArrow,
                    );
                }
                format!(r"\overset{{{}}}{{{}}}", recurse(label), recurse(base))
            }
            Math::Linked { body, .. } => self.tex_at(body, depth),
            Math::Under { base, label } => format!(
                r"\underset{{\vphantom{{\Bigg|}}{}}}{{{}}}",
                recurse(label),
                recurse(base)
            ),
            Math::Native {
                tex, kind, color, ..
            } => self.paint(tex.clone(), color.unwrap_or(*kind)),
            Math::Space => r"\,".into(),
            Math::Row(parts) if depth == Depth::Top => parts
                .iter()
                .enumerate()
                .map(|(index, part)| {
                    let mark = |breaks: bool| if breaks { BREAK_MARK } else { "" };
                    format!(
                        "{}{}{}",
                        mark(index > 0 && part.is_binary()),
                        self.tex_at(part, Depth::of_part(part)),
                        mark(part.is_separator())
                    )
                })
                .collect(),
            Math::Row(parts) => parts.iter().map(recurse).collect(),
            // Both scripts of an indexed base sit on its nucleus, stacked as in `x_2^2`:
            // a braced `{x_2}` would be one finished box, its exponent set after the index.
            Math::Power { base, exponent } => match base.as_ref() {
                Math::Index { base, subscript } => format!(
                    "{}_{{{}}}^{{{}}}",
                    self.nucleus(base),
                    recurse(subscript),
                    recurse(exponent)
                ),
                base => format!("{}^{{{}}}", self.nucleus(base), recurse(exponent)),
            },
            Math::Index { base, subscript } => {
                format!("{}_{{{}}}", self.nucleus(base), recurse(subscript))
            }
            Math::Fraction {
                numerator,
                denominator,
            } => self.paint(
                format!(
                    r"\frac{{{}}}{{{}}}",
                    recurse(numerator),
                    recurse(denominator)
                ),
                HLGroup::FractionRule,
            ),
            Math::Delimited { left, right, body } => {
                let fence = |c: &char| match c {
                    '{' => r"\{".to_string(),
                    '}' => r"\}".to_string(),
                    '⟨' => r"\langle".to_string(),
                    '⟩' => r"\rangle".to_string(),
                    _ => c.to_string(),
                };
                self.paint(
                    format!(
                        r"\left{}{}\right{}",
                        fence(left),
                        self.tex_at(body, depth),
                        fence(right)
                    ),
                    HLGroup::Punctuation,
                )
            }
            Math::AnnotatedMatrix { rows, degrees } => {
                let label_rows = rows
                    .iter()
                    .zip(degrees)
                    .map(|(row, degree)| {
                        format!(
                            r"{}\vphantom{{{}}}",
                            recurse(degree),
                            row.iter().map(recurse).collect::<String>()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(r"\\");
                let entries = rows
                    .iter()
                    .enumerate()
                    .map(|(index, row)| {
                        let phantom = degrees.get(index).map(recurse).unwrap_or_default();
                        format!(
                            r"\vphantom{{{phantom}}}{}",
                            row.iter().map(recurse).collect::<Vec<_>>().join("&")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(r"\\");
                self.paint(
                    format!(
                        r"\begin{{array}}{{l}}{label_rows}\end{{array}}\left(\begin{{matrix}}{entries}\end{{matrix}}\right)"
                    ),
                    HLGroup::Punctuation)
            }
            Math::Grid { rows, kind } => {
                let body = rows
                    .iter()
                    .map(|row| row.iter().map(recurse).collect::<Vec<_>>().join("&"))
                    .collect::<Vec<_>>()
                    .join(r"\\");
                if matches!(
                    kind,
                    maths::GridKind::LeftAligned | maths::GridKind::Aligned
                ) {
                    let columns = "l".repeat(rows.iter().map(Vec::len).max().unwrap_or(0));
                    return self.paint(
                        format!(r"\begin{{array}}{{{columns}}}{body}\end{{array}}"),
                        HLGroup::Punctuation,
                    );
                }
                let environment = match kind {
                    maths::GridKind::Matrix => "pmatrix",
                    maths::GridKind::Cases => "cases",
                    maths::GridKind::Bare => "matrix",
                    maths::GridKind::LeftAligned | maths::GridKind::Aligned => unreachable!(),
                };
                self.paint(
                    format!(r"\begin{{{environment}}}{body}\end{{{environment}}}"),
                    HLGroup::Punctuation,
                )
            }
        }
    }
}

fn draw(
    value: &Shape,
    code: &str,
    width: usize,
    groups: &Groups,
    style: MathStyle,
    picture: Picture,
) -> Drawn {
    let width = width.max(1);
    if value.wants_typesetting() {
        let math = Reading::new(value, style).math();
        let layout_groups = groups.map_styles(|_, mut style| {
            style.fg = None;
            style
        });
        let writer = |groups| Writer {
            groups: Some(groups),
            arrows: style.arrows,
        };
        let source = writer(&layout_groups).formula(&math);
        let painted = if style.ink == MathInk::Monochrome {
            source.clone()
        } else {
            writer(groups).formula(&math)
        };
        if let Some(Color::Rgb(r, g, b)) = groups.style(HLGroup::Value).fg {
            if let Some(Pictured { lines, baseline }) = picture(
                Typesetting {
                    tex: &source,
                    painted: &painted,
                    code,
                    matrix_row_spacing_percent: style.matrix_row_spacing_percent,
                    scale_percent: style.scale_percent,
                    slicing: if math.reads_in_lines() {
                        Slicing::Whole
                    } else {
                        Slicing::AtBreaks
                    },
                },
                Rgb([r, g, b]),
                u16::try_from(width).unwrap_or(u16::MAX),
            ) {
                debug_assert!(lines.is_empty() || baseline < lines.len());
                return Drawn {
                    lines,
                    links: Vec::new(),
                };
            }
        }
    }
    let rows = maths::spans(value, width, style).unwrap_or_default();
    let links = present::links(&rows);
    let lines = rows
        .into_iter()
        .map(|row| Line::from(row.spans(groups)))
        .collect();
    Drawn { lines, links }
}

/// Draw the supplied expression directly, without classifying formatted text: as a
/// picture when some part of it is set as maths, else as terminal text; `code`
/// is what copying the picture gives.
pub fn drawn(value: &Shape, code: &str, width: usize, groups: &Groups, style: MathStyle) -> Drawn {
    draw(value, code, width, groups, style, graphics::typeset_alone)
}

/// Render the type method's output with each component's supplied highlight class.
pub fn annotation(
    value: &Shape,
    width: usize,
    groups: &Groups,
    style: MathStyle,
) -> Vec<Line<'static>> {
    let groups = groups.map_styles(|group, mut style| {
        style.bg = None;
        if !matches!(
            group,
            HLGroup::Canvas | HLGroup::MapArrow | HLGroup::Punctuation | HLGroup::Delimiter
        ) {
            style.fg = groups.type_info().fg;
        }
        style
    });
    let style = MathStyle {
        ink: MathInk::Syntax,
        ..style
    };
    draw(
        value,
        &value.render(),
        width,
        &groups,
        style,
        graphics::typeset,
    )
    .lines
}
