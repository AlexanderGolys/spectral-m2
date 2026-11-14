//! Typesetting from class-tagged expression data, with Unicode fallback.

use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};

use crate::backend::SpecNet;
use crate::ui::block::{Link, Text};
use crate::ui::graphics::{self, ratex::Rgb, Pictured, Slicing, Typesetting};
use crate::ui::present;
use crate::ui::theme::groups::{Groups, HLGroup};
use crate::ui::wrap::WrappedText;

use super::maths::{
    self, Course, Diagram, Math, MathDisplay, MathInk, MathStyle, PRODUCT_SIGN, TENSOR_SIGN,
};
use crate::ui::graphics::ratex::diagram::Description;

/// Rendered rows with their interactive links and chosen representation.
pub struct Drawn {
    /// Terminal rows, including picture placeholders when available.
    pub lines: Vec<Line<'static>>,
    /// Links positioned in the rendered rows.
    pub links: Vec<Link>,
}

type Picture = fn(Typesetting, Rgb, u16) -> Option<Pictured>;

fn paint(text: String, kind: HLGroup, groups: Option<&Groups>) -> String {
    match groups.and_then(|groups| groups.style(kind).fg) {
        Some(Color::Rgb(r, g, b)) => format!(r"{{\color{{#{r:02x}{g:02x}{b:02x}}}{text}}}"),
        _ => text,
    }
}

/// What a value is set from: its TeX, or a diagram's description when the value
/// is a diagram, which TeX alone cannot draw.
fn formula(math: &Math, groups: Option<&Groups>) -> String {
    let Math::Diagram(diagram) = math else {
        return tex(math, groups);
    };
    let ink = groups
        .and_then(|groups| groups.style(HLGroup::MapArrow).fg)
        .and_then(|colour| match colour {
            Color::Rgb(r, g, b) => Some(Rgb([r, g, b])),
            _ => None,
        });
    let mut description = Description::new(ink);
    for (place, piece) in &diagram.pieces {
        description.piece((place.row, place.column), &tex(piece, groups));
    }
    for arrow in &diagram.arrows {
        let label = arrow.label.as_ref().map(|label| tex(label, groups));
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
fn diagram_array(diagram: &Diagram, groups: Option<&Groups>) -> String {
    let recurse = |part| tex(part, groups);
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
                cells[row][between] = format!(r"{command}{{\scriptsize {label}}}");
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

fn tex(math: &Math, groups: Option<&Groups>) -> String {
    let recurse = |part| tex(part, groups);
    match math {
        Math::Diagram(diagram) => diagram_array(diagram, groups),
        Math::Atom { text, kind, color } => {
            let content = match (kind, text.as_str()) {
                (HLGroup::MapArrow | HLGroup::Operator, "⟵") => r"\longleftarrow".into(),
                (HLGroup::Delimiter, "," | ", " | ";") => text.clone(),
                (HLGroup::Operator, PRODUCT_SIGN) => r"\,".into(),
                (HLGroup::Operator, TENSOR_SIGN) => r"\otimes ".into(),
                (HLGroup::Operator, "+" | "-" | "=") => text.clone(),
                (HLGroup::Number, _) => maths::typewriter(text).replacen("texttt", "mathrm", 1),
                (HLGroup::Variable, _) => maths::typewriter(text).replacen("texttt", "mathit", 1),
                _ => {
                    let content = maths::typewriter(text);
                    if groups.is_some_and(|groups| {
                        groups.style(*kind).add_modifier.contains(Modifier::ITALIC)
                    }) {
                        content.replacen("texttt", "textit", 1)
                    } else {
                        content
                    }
                }
            };
            paint(content, color.unwrap_or(*kind), groups)
        }
        Math::Over { base, label } => {
            if matches!(
                base.as_ref(),
                Math::Atom {
                    kind: HLGroup::MapArrow,
                    ..
                }
            ) {
                return paint(
                    format!(r"\xleftarrow{{\scriptsize {}}}", recurse(label)),
                    HLGroup::MapArrow,
                    groups,
                );
            }
            format!(r"\overset{{{}}}{{{}}}", recurse(label), recurse(base))
        }
        Math::Linked { body, .. } => recurse(body),
        Math::Under { base, label } => format!(
            r"\underset{{\vphantom{{\Bigg|}}{}}}{{{}}}",
            recurse(label),
            recurse(base)
        ),
        Math::Native {
            tex, kind, color, ..
        } => paint(tex.clone(), color.unwrap_or(*kind), groups),
        Math::Space => r"\,".into(),
        Math::Row(parts) => parts.iter().map(recurse).collect(),
        Math::Power { base, exponent } => {
            format!("{{{}}}^{{{}}}", recurse(base), recurse(exponent))
        }
        Math::Index { base, subscript } => {
            format!("{{{}}}_{{{}}}", recurse(base), recurse(subscript))
        }
        Math::Fraction {
            numerator,
            denominator,
        } => paint(
            format!(
                r"\frac{{{}}}{{{}}}",
                recurse(numerator),
                recurse(denominator)
            ),
            HLGroup::FractionRule,
            groups,
        ),
        Math::Delimited { left, right, body } => {
            let fence = |c: &char| match c {
                '{' => r"\{".to_string(),
                '}' => r"\}".to_string(),
                '⟨' => r"\langle".to_string(),
                '⟩' => r"\rangle".to_string(),
                _ => c.to_string(),
            };
            paint(
                format!(
                    r"\left{}{}\right{}",
                    fence(left),
                    recurse(body),
                    fence(right)
                ),
                HLGroup::Punctuation,
                groups,
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
            paint(
                format!(
                    r"\begin{{array}}{{l}}{label_rows}\end{{array}}\left(\begin{{matrix}}{entries}\end{{matrix}}\right)"
                ),
                HLGroup::Punctuation,
                groups,
            )
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
                return paint(
                    format!(r"\begin{{array}}{{{columns}}}{body}\end{{array}}"),
                    HLGroup::Punctuation,
                    groups,
                );
            }
            let environment = match kind {
                maths::GridKind::Matrix => "pmatrix",
                maths::GridKind::Cases => "cases",
                maths::GridKind::Bare => "matrix",
                maths::GridKind::LeftAligned | maths::GridKind::Aligned => unreachable!(),
            };
            paint(
                format!(r"\begin{{{environment}}}{body}\end{{{environment}}}"),
                HLGroup::Punctuation,
                groups,
            )
        }
    }
}

fn draw(
    value: &SpecNet,
    width: usize,
    groups: &Groups,
    style: MathStyle,
    picture: Picture,
) -> Drawn {
    let width = width.max(1);
    if value.wants_typesetting() && value.html.is_none() && !has_links(value) {
        let math = Math::from_spec_net(&typewritten_where_unmarked(value), style);
        let layout_groups = groups.map_styles(|_, mut style| {
            style.fg = None;
            style
        });
        let source = formula(&math, Some(&layout_groups));
        let painted = if style.ink == MathInk::Monochrome {
            source.clone()
        } else {
            formula(&math, Some(groups))
        };
        if let Some(Color::Rgb(r, g, b)) = groups.style(HLGroup::Value).fg {
            if let Some(Pictured { lines, baseline }) = picture(
                Typesetting {
                    tex: &source,
                    painted: &painted,
                    code: value.code.as_deref().unwrap_or(""),
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

/// `value` with each text M2 did not ask to typeset, which a picture holds only
/// for sitting beside what it did, spelled in a typewriter face as the text it is.
fn typewritten_where_unmarked(value: &SpecNet) -> SpecNet {
    let mut value = value.clone();
    match &value.text {
        Some(text) if !value.typeset && value.tex.is_none() => {
            value.tex = Some(maths::typewriter(text));
        }
        _ => {
            value.children = value
                .children
                .iter()
                .map(typewritten_where_unmarked)
                .collect();
        }
    }
    value
}

fn has_links(value: &SpecNet) -> bool {
    value.target.is_some() || value.children.iter().any(has_links)
}

/// Preview M2's TeX formatter using the display selected in settings.
/// Unsupported TeX remains visible as source under the same preview mode.
pub fn tex_preview(source: &str, width: usize, groups: &Groups, style: MathStyle) -> Drawn {
    let formula = source
        .strip_prefix('$')
        .and_then(|source| source.strip_suffix('$'))
        .unwrap_or(source);
    if style.display == MathDisplay::Typeset {
        if let Some(Color::Rgb(r, g, b)) = groups.style(HLGroup::Value).fg {
            if let Some(picture) = graphics::typeset_alone(
                Typesetting {
                    tex: formula,
                    painted: formula,
                    code: source,
                    matrix_row_spacing_percent: style.matrix_row_spacing_percent,
                    scale_percent: style.scale_percent,
                    slicing: Slicing::AtBreaks,
                },
                Rgb([r, g, b]),
                u16::try_from(width).unwrap_or(u16::MAX),
            ) {
                return Drawn {
                    lines: picture.lines,
                    links: Vec::new(),
                };
            }
        }
    }
    let rows = (style.display != MathDisplay::Ascii)
        .then(|| Math::from_tex(source, style).ok())
        .flatten()
        .map(|math| math.layout(style.fractions).into_rows());
    let lines = rows
        .map(|rows| {
            rows.into_iter()
                .map(|row| {
                    let text = row.into_iter().map(|span| span.content).collect::<String>();
                    Line::from(Span::styled(text, groups.value()))
                })
                .collect()
        })
        .unwrap_or_else(|| {
            WrappedText::word(source, u16::try_from(width.max(1)).unwrap_or(u16::MAX))
                .text()
                .split('\n')
                .map(|row| Line::raw(row.to_string()))
                .collect()
        });
    Drawn {
        lines,
        links: Vec::new(),
    }
}

/// Draw the supplied expression directly, without classifying formatted text: as a
/// picture when M2 asks for some part of it to be typeset, else as terminal text.
pub fn drawn(value: &SpecNet, width: usize, groups: &Groups, style: MathStyle) -> Drawn {
    draw(value, width, groups, style, graphics::typeset_alone)
}

/// Render the type method's output with each component's supplied highlight class.
pub fn annotation(
    parts: &[SpecNet],
    width: usize,
    groups: &Groups,
    style: MathStyle,
) -> Vec<Line<'static>> {
    annotation_with_picture(parts, width, groups, style, graphics::typeset)
}

fn annotation_with_picture(
    parts: &[SpecNet],
    width: usize,
    groups: &Groups,
    style: MathStyle,
    picture: Picture,
) -> Vec<Line<'static>> {
    let mut row = SpecNet::branch("RowExpression", parts.to_vec());
    row.code = Some(parts.iter().map(SpecNet::render).collect::<String>());
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
    draw(&row, width, &groups, style, picture).lines
}
