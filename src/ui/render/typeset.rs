//! Typesetting from class-tagged expression data, with Unicode fallback.

use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};

use crate::backend::SpecNet;
use crate::ui::block::{Link, Text};
use crate::ui::graphics::{self, ratex::Rgb, Pictured, Typesetting};
use crate::ui::present;
use crate::ui::theme::groups::{Groups, HLGroup};
use crate::ui::wrap::WrappedText;

use super::maths::{self, Math, MathDisplay, MathInk, MathStyle};

/// Rendered rows with their interactive links and chosen representation.
pub struct Drawn {
    /// Terminal rows, including picture placeholders when available.
    pub lines: Vec<Line<'static>>,
    /// Links positioned in the rendered rows.
    pub links: Vec<Link>,
}

type Picture = fn(Typesetting, Rgb, u16) -> Option<Pictured>;

/// M2's class of a value that is nothing but a name, spelled as terminal text.
const NAME_CLASS: &str = "Symbol";

fn paint(text: String, kind: HLGroup, groups: Option<&Groups>) -> String {
    match groups.and_then(|groups| groups.style(kind).fg) {
        Some(Color::Rgb(r, g, b)) => format!(r"{{\color{{#{r:02x}{g:02x}{b:02x}}}{text}}}"),
        _ => text,
    }
}

fn tex(math: &Math, groups: Option<&Groups>) -> String {
    let recurse = |part| tex(part, groups);
    match math {
        Math::Atom { text, kind, color } => {
            let content = match (kind, text.as_str()) {
                (HLGroup::MapArrow | HLGroup::Operator, "⟵") => r"\longleftarrow".into(),
                (HLGroup::Delimiter, "," | ", " | ";") => text.clone(),
                (HLGroup::Operator, "·") => r"\,".into(),
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
            if *kind == maths::GridKind::LeftAligned {
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
                maths::GridKind::LeftAligned => unreachable!(),
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
    let math = Math::from_spec_net(value, style);
    if value.html.is_none()
        && !has_links(value)
        && !matches!(
            math,
            Math::Atom {
                kind: HLGroup::Package,
                ..
            }
        )
    {
        let layout_groups = groups.map_styles(|_, mut style| {
            style.fg = None;
            style
        });
        let source = tex(&math, Some(&layout_groups));
        let painted = if style.ink == MathInk::Monochrome {
            source.clone()
        } else {
            tex(&math, Some(groups))
        };
        if let Some(Color::Rgb(r, g, b)) = groups.style(HLGroup::Value).fg {
            if let Some(Pictured { lines, baseline }) = picture(
                Typesetting {
                    tex: &source,
                    painted: &painted,
                    code: value.code.as_deref().unwrap_or(""),
                    matrix_row_spacing_percent: style.matrix_row_spacing_percent,
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

/// Draw the supplied expression directly, without classifying formatted text.
/// `value_class` is the class of the value itself, which its expression's
/// classes do not show: a ring variable and a bare symbol share one tree.
pub fn drawn(
    value: &SpecNet,
    value_class: Option<&str>,
    width: usize,
    groups: &Groups,
    style: MathStyle,
) -> Drawn {
    drawn_with(
        value,
        value_class,
        width,
        groups,
        style,
        graphics::typeset_alone,
    )
}

fn drawn_with(
    value: &SpecNet,
    value_class: Option<&str>,
    width: usize,
    groups: &Groups,
    style: MathStyle,
    picture: Picture,
) -> Drawn {
    let picture: Picture = if value_class == Some(NAME_CLASS) {
        |_, _, _| None
    } else {
        picture
    };
    draw(value, width, groups, style, picture)
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    #[test]
    fn complex_pictures_compose_labelled_maps_and_under_module_degrees() {
        let mut backend = crate::backend::M2Backend::spawn().unwrap();
        let result = backend
            .evaluate("S = QQ[a_1..a_4]; res coker vars S", None)
            .unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        let value = result.value.unwrap();
        let math = Math::from_spec_net(&value, MathStyle::default());
        let source = tex(&math, None);
        assert_eq!(source.matches(r"\xleftarrow").count(), 4);
        assert_eq!(source.matches(r"\underset").count(), 5);
        assert_eq!(source.matches(r"\left[").count(), 4);
        assert!(graphics::ratex::svg(&source, Rgb([17, 29, 43])).is_ok());
        let rows = maths::spans(&value, 240, MathStyle::default()).unwrap();
        assert!(!rows.is_empty());
    }

    #[test]
    fn package_pictures_follow_the_roles_italic_style() {
        let value = SpecNet::leaf("Package", "ExamplePackage");
        let groups = Groups {
            Package: Style::default().add_modifier(Modifier::ITALIC),
            ..Groups::default()
        };
        let math = Math::from_spec_net(&value, MathStyle::default());
        assert!(tex(&math, Some(&groups)).contains(r"\textit{ExamplePackage}"));
        let groups = Groups {
            Package: Style::default(),
            ..groups
        };
        assert!(tex(&math, Some(&groups)).contains(r"\texttt{ExamplePackage}"));
    }

    #[test]
    fn package_names_use_terminal_text_and_the_package_style() {
        let groups = Groups {
            Package: Style::default()
                .fg(Color::Rgb(83, 127, 173))
                .add_modifier(Modifier::ITALIC),
            ..Groups::default()
        };
        let name = "ExamplePackage";
        let value = SpecNet::leaf("Package", name);
        let drawn = draw(
            &value,
            80,
            &groups,
            MathStyle {
                ink: MathInk::Syntax,
                ..MathStyle::default()
            },
            |_, _, _| panic!("package names should render as terminal text"),
        );
        assert_eq!(
            drawn.lines,
            vec![Line::from(Span::styled(
                name,
                groups.style(HLGroup::Package)
            ))]
        );
    }

    #[test]
    fn numbered_vertical_lists_align_entries_after_the_indices() {
        let value = SpecNet::branch(
            "NumberedVerticalList",
            vec![
                SpecNet::leaf("String", "short"),
                SpecNet::leaf("String", "longer entry"),
            ],
        );
        let math = Math::from_spec_net(&value, MathStyle::default());
        let source = tex(&math, None);
        assert!(source.contains(r"\begin{array}{lll}"));
        assert!(source.contains(r"\mathrm{0}"));
        assert!(source.contains(r"\mathrm{1}"));
        assert!(crate::ui::graphics::ratex::svg(&source, Rgb([17, 29, 43])).is_ok());
        let rows = maths::spans(&value, 80, MathStyle::default()).unwrap();
        let rows: Vec<String> = rows
            .into_iter()
            .map(|row| row.into_iter().map(|span| span.content).collect())
            .collect();
        let short = rows.iter().find(|row| row.contains("short")).unwrap();
        let longer = rows
            .iter()
            .find(|row| row.contains("longer entry"))
            .unwrap();
        assert_eq!(short.find("short"), longer.find("longer entry"));
        assert!(short.contains('0'));
        assert!(longer.contains('1'));
    }

    #[test]
    fn nested_matrix_fences_and_map_arrows_use_their_structural_colour() {
        let structural = Style::default().fg(Color::Rgb(17, 43, 61));
        let groups = Groups {
            Punctuation: structural,
            Delimiter: Style::default().fg(Color::Rgb(101, 113, 127)),
            MapArrow: structural,
            Value: Style::default().fg(Color::Rgb(71, 83, 97)),
            ..Groups::default()
        };
        let matrix = Math::AnnotatedMatrix {
            rows: vec![vec![Math::Atom {
                text: "x".into(),
                kind: HLGroup::Variable,
                color: None,
            }]],
            degrees: Vec::new(),
        };
        let formulas = [
            Math::Delimited {
                left: '{',
                right: '}',
                body: Box::new(matrix.clone()),
            },
            matrix,
            Math::Grid {
                rows: Vec::new(),
                kind: maths::GridKind::Matrix,
            },
            Math::Over {
                base: Box::new(Math::Atom {
                    text: "⟵".into(),
                    kind: HLGroup::MapArrow,
                    color: None,
                }),
                label: Box::new(Math::Row(Vec::new())),
            },
        ];
        for formula in formulas {
            assert!(tex(&formula, Some(&groups)).starts_with(r"{\color{#112b3d}"));
        }
        let comma = Math::Atom {
            text: ",".into(),
            kind: HLGroup::Delimiter,
            color: None,
        };
        assert!(tex(&comma, Some(&groups)).starts_with(r"{\color{#65717f}"));
    }

    #[test]
    fn annotation_pictures_have_copy_text_and_one_type_colour() {
        let parts = vec![
            SpecNet::leaf("Type", "FixtureType"),
            SpecNet::leaf("ZZ", "7"),
        ];
        let groups = Groups {
            Type: Style::default().fg(Color::Rgb(17, 29, 43)),
            ..Groups::default()
        };
        let lines = annotation_with_picture(
            &parts,
            80,
            &groups,
            MathStyle::default(),
            |formula, _, _| {
                assert_eq!(formula.code, "FixtureType7");
                assert!(formula.painted.contains("#111d2b"));
                Some(Pictured {
                    lines: vec![Line::raw("annotation")],
                    baseline: 0,
                })
            },
        );
        assert!(!lines.is_empty());
        let lines =
            annotation_with_picture(&parts, 80, &groups, MathStyle::default(), |_, _, _| None);
        assert!(lines
            .iter()
            .flat_map(|line| &line.spans)
            .all(|span| span.style.fg == groups.Type.fg));
    }

    #[test]
    fn matrix_degrees_and_map_labels_follow_m2_typesetting() {
        let mut backend = crate::backend::M2Backend::spawn().unwrap();
        let response = backend.evaluate("R = QQ[a..d]; syz vars R", None).unwrap();
        let value = response.value.unwrap();
        let math = Math::from_spec_net(&value, MathStyle::default());
        let Math::AnnotatedMatrix { rows, degrees } = &math else {
            panic!("expected matrix with row degrees");
        };
        assert_eq!(rows.len(), degrees.len());
        let source = tex(&math, None);
        assert!(!source.contains("Degrees"));
        assert!(source.contains(r"\vphantom"));
        assert!(crate::ui::graphics::ratex::svg(&source, Rgb([17, 29, 43])).is_ok());
        let map = Math::Over {
            base: Box::new(Math::Atom {
                text: "⟵".into(),
                kind: HLGroup::MapArrow,
                color: None,
            }),
            label: Box::new(math),
        };
        let source = tex(&map, None);
        assert!(source.starts_with(r"\xleftarrow"));
        assert!(crate::ui::graphics::ratex::svg(&source, Rgb([17, 29, 43])).is_ok());
    }

    #[test]
    fn plain_tex_does_not_infer_component_colours() {
        let groups = Groups {
            Value: Style::default().fg(Color::Rgb(11, 22, 33)),
            Variable: Style::default().fg(Color::Rgb(44, 55, 66)),
            Number: Style::default().fg(Color::Rgb(77, 88, 99)),
            ..Groups::default()
        };
        let drawing = tex_preview("$x+7$", 80, &groups, MathStyle::default());
        let spans = drawing
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .collect::<Vec<_>>();
        assert!(!spans.is_empty());
        assert!(spans.iter().all(|span| span.style == groups.value()));
        let text = spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains('x'));
        assert!(text.contains('7'));
    }

    #[test]
    fn symbols_are_typeset_with_or_without_numeric_siblings() {
        let picture: Picture = |formula, _, _| {
            assert!(formula.tex.contains(r"\mathit{sss}"));
            Some(Pictured {
                lines: vec![Line::raw("typeset")],
                baseline: 0,
            })
        };
        let symbol = SpecNet::leaf("Symbol", "sss");
        for sibling in [SpecNet::leaf("String", "1"), SpecNet::leaf("ZZ", "1")] {
            let value = SpecNet::branch("Sequence", vec![symbol.clone(), sibling]);
            let drawing = draw(
                &value,
                80,
                &Groups::default(),
                MathStyle::default(),
                picture,
            );
            assert_eq!(drawing.lines, vec![Line::raw("typeset")]);
        }
        let drawing = draw(
            &symbol,
            80,
            &Groups::default(),
            MathStyle::default(),
            picture,
        );
        assert_eq!(drawing.lines, vec![Line::raw("typeset")]);
    }

    #[test]
    fn bare_symbols_are_names_while_ring_variables_stay_typeset() {
        let symbol = SpecNet::leaf("Symbol", "sss");
        let groups = Groups::default();
        let drawn = drawn_with(
            &symbol,
            Some(NAME_CLASS),
            80,
            &groups,
            MathStyle::default(),
            |_, _, _| panic!("a bare symbol should render as terminal text"),
        );
        let text = drawn
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(text, "sss");
        let typeset = drawn_with(
            &symbol,
            Some("R"),
            80,
            &groups,
            MathStyle::default(),
            |_, _, _| {
                Some(Pictured {
                    lines: vec![Line::raw("typeset")],
                    baseline: 0,
                })
            },
        );
        assert_eq!(typeset.lines, vec![Line::raw("typeset")]);
    }

    #[test]
    fn picture_colours_are_taken_from_component_classes() {
        let value = SpecNet::branch(
            "Power",
            vec![SpecNet::leaf("String", "a"), SpecNet::leaf("Symbol", "a")],
        );
        let picture: Picture = |formula, _, _| {
            assert!(formula.tex.contains(r"\texttt{a}"));
            assert!(formula.tex.contains(r"\mathit{a}"));
            assert!(formula.painted.contains(r"\color"));
            Some(Pictured {
                lines: vec![Line::raw("picture")],
                baseline: 0,
            })
        };
        let drawing = draw(
            &value,
            80,
            &Groups::default(),
            MathStyle::default(),
            picture,
        );
        assert_eq!(drawing.lines, vec![Line::raw("picture")]);
    }

    #[test]
    fn script_colours_preserve_typefaces_and_unpainted_layout() {
        use super::maths::ScriptColor;

        let script = SpecNet::branch(
            "RowExpression",
            vec![
                SpecNet::leaf("Symbol", "b"),
                SpecNet::leaf("Number", "2"),
                SpecNet::leaf("String", "word"),
            ],
        );
        let value = SpecNet::branch(
            "Power",
            vec![
                SpecNet::branch(
                    "Subscript",
                    vec![SpecNet::leaf("Symbol", "x"), script.clone()],
                ),
                script,
            ],
        );
        let groups = Groups {
            Variable: Style::default().fg(Color::Rgb(10, 20, 30)),
            Number: Style::default().fg(Color::Rgb(40, 50, 60)),
            String: Style::default().fg(Color::Rgb(70, 80, 90)),
            Operator: Style::default().fg(Color::Rgb(100, 110, 120)),
            ..Groups::default()
        };
        let original = Math::from_spec_net(&value, MathStyle::default());
        let source = tex(&original, None);
        for color in [ScriptColor::Symbol, ScriptColor::Operator] {
            let recolored = Math::from_spec_net(
                &value,
                MathStyle {
                    index_color: color,
                    exponent_color: color,
                    ..MathStyle::default()
                },
            );
            assert_eq!(tex(&recolored, None), source);
            assert_ne!(
                tex(&recolored, Some(&groups)),
                tex(&original, Some(&groups))
            );
        }
    }

    #[test]
    fn unavailable_pictures_keep_structured_component_colours() {
        let groups = Groups::default();
        let value = SpecNet::branch(
            "RowExpression",
            vec![SpecNet::leaf("String", "a"), SpecNet::leaf("Symbol", "a")],
        );
        let drawing = draw(&value, 80, &groups, MathStyle::default(), |_, _, _| None);
        let styles = drawing
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .map(|span| span.style)
            .collect::<Vec<_>>();
        assert!(styles.contains(&groups.style(HLGroup::String)));
        assert!(styles.contains(&groups.style(HLGroup::Variable)));
        assert!(!drawing.lines.is_empty());
    }

    #[test]
    fn generated_structures_are_accepted_by_the_picture_engine() {
        let groups = Groups::default();
        let style = MathStyle::default();
        let fraction = SpecNet::branch(
            "Divide",
            vec![SpecNet::leaf("ZZ", "13"), SpecNet::leaf("ZZ", "7")],
        );
        let samples = [
            SpecNet::branch(
                "Array",
                vec![SpecNet::leaf("String", "a_b"), fraction.clone()],
            ),
            SpecNet::branch(
                "MatrixExpression",
                vec![SpecNet::branch(
                    "List",
                    vec![fraction, SpecNet::leaf("Symbol", "x")],
                )],
            ),
            SpecNet::branch(
                "Subscript",
                vec![SpecNet::leaf("Symbol", "x"), SpecNet::leaf("String", "a")],
            ),
        ];
        for value in samples {
            let math = Math::from_spec_net(&value, style);
            let source = tex(&math, None);
            let painted = tex(&math, Some(&groups));
            let rendered = graphics::ratex::svg_colored(
                &source,
                &painted,
                Rgb([31, 63, 127]),
                style.matrix_row_spacing_percent,
            );
            assert!(rendered.is_ok(), "{source}: {:?}", rendered.err());
        }
    }
}
