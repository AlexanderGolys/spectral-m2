//! Typesetting the drawings M2 sends, with Unicode fallback.

use ratatui::style::{Color, Modifier};
use ratatui::text::Line;

use crate::backend::Shape;
use crate::ui::block::{Link, Text};
use crate::ui::graphics::{self, Pictured, Typesetting};
use crate::ui::present;
use crate::ui::theme::groups::{Groups, HLGroup};

use super::maths::{
    self, Course, Diagram, GridKind, Math, MathInk, MathStyle, Reading, DIRECT_SUM_SIGN,
    PRODUCT_SIGN, TENSOR_SIGN,
};
use crate::ui::graphics::ratex::{
    self, break_mark, delimiter,
    diagram::{ArrowLengths, Description, MIN_ARROW},
    Rgb, Source, PERCENT_PER_ARRAYSTRETCH,
};

/// Rendered rows with their interactive links and chosen representation.
pub struct Drawn {
    /// Terminal rows, including picture placeholders when available.
    pub lines: Vec<Line<'static>>,
    /// Links positioned in the rendered rows.
    pub links: Vec<Link>,
}

type Picture = fn(Typesetting, Rgb, u16) -> Option<Pictured>;

/// Where TeX being written lies: in a row that may break into lines, at the
/// rank its breaks have, or held whole inside a script, a fraction or a label.
/// A formula's own row has rank 0, and each row, bracketed part or matrix
/// nested in a row one more than it, so a formula breaks between a list's
/// items before it breaks inside one of them. The brackets around a row hold
/// it at their own depth: a long list breaks between its items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Depth {
    Top(usize),
    Held,
}

impl Depth {
    /// Where `part` of a row lying here lies: a row, a bracketed part or a
    /// matrix one rank deeper, anything else holding its own parts whole.
    fn of_part(self, part: &Math) -> Self {
        match (self, part) {
            (_, Math::Linked { body, .. }) => self.of_part(body),
            (
                Self::Top(rank),
                Math::Row(_)
                | Math::Delimited { .. }
                | Math::Grid { .. }
                | Math::AnnotatedMatrix { .. },
            ) => Self::Top(rank + 1),
            _ => Self::Held,
        }
    }

    /// The mark of a place a row lying here may break, when it may.
    fn mark(self, breaks: bool) -> String {
        match self {
            Self::Top(rank) if breaks => break_mark(rank),
            _ => String::new(),
        }
    }
}

/// TeX's script size beside its text size, which an extensible arrow sets its
/// label at.
const SCRIPT_SCALE: f64 = 0.7;
/// Ems an extensible arrow runs past its label, both ends together.
const ARROW_OVERHANG: f64 = 0.7;

/// What writes a value's TeX: the colours its atoms are painted in, none when
/// it is written to be measured alone, how long its arrows run and how far
/// apart, in percent of TeX's own spacing, a matrix's rows stand.
#[derive(Clone, Copy)]
struct Writer<'a> {
    groups: Option<&'a Groups>,
    arrows: ArrowLengths,
    matrix_rows: u16,
}

impl Writer<'_> {
    /// The colour `kind` is painted in, when this writer paints.
    fn ink(&self, kind: HLGroup) -> Option<Rgb> {
        match self.groups?.style(kind).fg? {
            Color::Rgb(r, g, b) => Some(Rgb([r, g, b])),
            _ => None,
        }
    }

    /// `array` with its rows `percent` of TeX's own spacing apart. Every array
    /// states its own spacing as it begins, so a column of lines inside a
    /// matrix keeps the spacing of text and a matrix inside it that of matrices.
    fn spaced(percent: u16, array: String) -> String {
        let stretch = f64::from(percent) / f64::from(PERCENT_PER_ARRAYSTRETCH);
        format!(r"\def\arraystretch{{{stretch}}}{array}")
    }

    fn paint(&self, text: String, kind: HLGroup) -> String {
        match self.ink(kind) {
            Some(ink) => ink.scope(&text),
            None => text,
        }
    }

    /// What one row of ink is set from: its TeX, or a diagram's description when
    /// it is a diagram, which TeX alone cannot draw.
    fn formula(&self, math: &Math) -> String {
        let Math::Diagram(diagram) = math else {
            return self.tex_at(math, Depth::Top(0));
        };
        let mut description = Description::new(self.ink(HLGroup::MapArrow), self.arrows);
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

    /// The rows of a grid lying at `depth`, each its entries between `&`. A grid
    /// in a row that breaks may break before any column but its first: a matrix
    /// too wide for its room is cut between columns, the columns after the cut
    /// set beneath. A column's mark stands before its widest entry alone, the
    /// one a centred column begins with; before a narrower one it would stand
    /// inside the column.
    fn grid_rows(&self, rows: &[Vec<Math>], depth: Depth) -> Vec<String> {
        // Measured unpainted, so the plain and the painted TeX mark the same rows.
        let plain = Writer {
            groups: None,
            ..*self
        };
        let width = |entry: &Math| ratex::width(&plain.tex(entry)).unwrap_or(0.0);
        let columns = match depth {
            Depth::Top(_) => rows.iter().map(Vec::len).max().unwrap_or(0),
            Depth::Held => 0,
        };
        let widest = (0..columns)
            .map(|column| {
                (0..rows.len())
                    .filter_map(|row| Some((row, width(rows[row].get(column)?))))
                    .max_by(|(_, a), (_, b)| a.total_cmp(b))
                    .map(|(row, _)| row)
            })
            .collect::<Vec<_>>();
        rows.iter()
            .enumerate()
            .map(|(index, row)| {
                row.iter()
                    .enumerate()
                    .map(|(column, entry)| {
                        let leads = column > 0 && widest.get(column) == Some(&Some(index));
                        format!("{}{}", depth.mark(leads), self.tex(entry))
                    })
                    .collect::<Vec<_>>()
                    .join("&")
            })
            .collect()
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
        Self::spaced(
            self.matrix_rows,
            format!(
                r"\begin{{array}}{{{}}}{body}\end{{array}}",
                "c".repeat(columns)
            ),
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
            Math::Row(parts) => parts
                .iter()
                .enumerate()
                .map(|(index, part)| {
                    format!(
                        "{}{}{}",
                        depth.mark(index > 0 && part.is_binary()),
                        self.tex_at(part, depth.of_part(part)),
                        depth.mark(part.is_separator())
                    )
                })
                .collect(),
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
            // Lines between fences stand off them by the space an array keeps by
            // its sides, not by the margins a terminal sets them between.
            Math::Delimited { left, right, body } => self.paint(
                format!(
                    r"\left{}{}\right{}",
                    delimiter(*left),
                    self.tex_at(body.lines_alone().unwrap_or(body), depth),
                    delimiter(*right)
                ),
                HLGroup::Punctuation,
            ),
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
                let entries = self
                    .grid_rows(rows, depth)
                    .into_iter()
                    .enumerate()
                    .map(|(index, row)| {
                        let phantom = degrees.get(index).map(recurse).unwrap_or_default();
                        format!(r"\vphantom{{{phantom}}}{row}")
                    })
                    .collect::<Vec<_>>()
                    .join(r"\\");
                self.paint(
                    Self::spaced(self.matrix_rows, format!(
                        r"\begin{{array}}{{l}}{label_rows}\end{{array}}\left(\begin{{matrix}}{entries}\end{{matrix}}\right)"
                    )),
                    HLGroup::Punctuation)
            }
            Math::Grid { rows, kind } => {
                // Columns break apart; rows read as lines are held whole here,
                // where they are part of a row of ink and not rows of their own.
                let depth = match kind {
                    GridKind::Matrix | GridKind::Bare => depth,
                    GridKind::LeftAligned | GridKind::Aligned | GridKind::Cases => Depth::Held,
                };
                let body = self.grid_rows(rows, depth).join(r"\\");
                if kind.reads_in_lines() {
                    let columns = "l".repeat(rows.iter().map(Vec::len).max().unwrap_or(0));
                    return self.paint(
                        Self::spaced(
                            PERCENT_PER_ARRAYSTRETCH,
                            format!(r"\begin{{array}}{{{columns}}}{body}\end{{array}}"),
                        ),
                        HLGroup::Punctuation,
                    );
                }
                let (environment, rows) = match kind {
                    GridKind::Matrix => ("pmatrix", self.matrix_rows),
                    GridKind::Bare => ("matrix", self.matrix_rows),
                    GridKind::Cases => ("cases", PERCENT_PER_ARRAYSTRETCH),
                    GridKind::LeftAligned | GridKind::Aligned => unreachable!(),
                };
                self.paint(
                    Self::spaced(
                        rows,
                        format!(r"\begin{{{environment}}}{body}\end{{{environment}}}"),
                    ),
                    HLGroup::Punctuation,
                )
            }
        }
    }
}

/// What writes a value's picture: its TeX as it is laid out, uncoloured, so no
/// colour scope moves a glyph, and the same TeX painted.
#[derive(Clone, Copy)]
struct Setting<'a> {
    plain: Writer<'a>,
    painted: Writer<'a>,
}

impl Setting<'_> {
    /// What `math` is set from: rows read as lines, each breaking on its own,
    /// where they are all there is of it or of what fences hold; anything else
    /// one row of ink.
    fn source(&self, math: &Math) -> Source {
        match math.alone() {
            Math::Grid { rows, kind } if kind.reads_in_lines() => Source::Rows(
                rows.iter()
                    .map(|row| row.iter().map(|cell| self.source(cell)).collect())
                    .collect(),
            ),
            Math::Delimited { left, right, body } if body.lines_alone().is_some() => {
                Source::Fenced {
                    left: *left,
                    right: *right,
                    ink: self.painted.ink(HLGroup::Punctuation),
                    body: Box::new(self.source(body)),
                }
            }
            _ => Source::Formula {
                tex: self.plain.formula(math),
                painted: self.painted.formula(math),
            },
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
            matrix_rows: style.matrix_row_spacing_percent,
        };
        let plain = writer(&layout_groups);
        let painted = match style.ink {
            MathInk::Monochrome => plain,
            _ => writer(groups),
        };
        let source = Setting { plain, painted }.source(&math);
        if let Some(Color::Rgb(r, g, b)) = groups.style(HLGroup::Value).fg {
            if let Some(Pictured { lines, baseline }) = picture(
                Typesetting {
                    source: &source,
                    code,
                    scale_percent: style.scale_percent,
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
