//! Typeset mathematical leaves composed with terminal text around a shared baseline.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::backend::{CollectionKind, M2Value, SPACED_OPTION_ARROW};
use crate::ui::block::Text;
use crate::ui::controller::settings::theme::groups::{Groups, HLGroup};
use crate::ui::graphics::{self, typeset::Rgb, Pictured, Typesetting};
use crate::ui::wrap::WrappedText;

use super::maths::{self, MathBlock, MathStyle};

/// The image backend, returning terminal-cell rows for one formula.
type Picture = fn(Typesetting, Rgb, u16) -> Option<Pictured>;

/// Render each value in its own representation, without converting text into TeX.
pub fn lines(
    value: &M2Value,
    width: usize,
    groups: &Groups,
    style: MathStyle,
) -> Vec<Line<'static>> {
    let picture: Picture = match value {
        M2Value::Tex { .. } => graphics::typeset_alone,
        _ => graphics::typeset,
    };
    Block::value(value, width.max(1), groups, style, None, picture).lines
}

/// The pieces of a type annotation, which carry their own spacing, set together:
/// as one formula with its words in a typewriter font when every piece has a TeX
/// form, so cell boundaries never widen the gaps between them; else side by side.
pub fn annotation(
    parts: &[M2Value],
    width: usize,
    groups: &Groups,
    style: MathStyle,
) -> Vec<Line<'static>> {
    let block =
        |part: &M2Value| Block::value(part, width.max(1), groups, style, None, graphics::typeset);
    let blocks = match formula(parts) {
        Some(formula) => vec![block(&formula)],
        None => parts.iter().map(block).collect(),
    };
    Block::pack(blocks, width.max(1))
        .into_iter()
        .flat_map(|block| block.lines)
        .collect()
}

/// `parts` joined into one TeX value, words as typewriter text; none when a part
/// has no TeX form.
fn formula(parts: &[M2Value]) -> Option<M2Value> {
    let (mut tex, mut fallback, mut code) = (String::new(), String::new(), String::new());
    for part in parts {
        match part {
            M2Value::Tex {
                source,
                fallback: net,
                code: spelling,
            } => {
                tex.push_str(source.strip_prefix('$')?.strip_suffix('$')?);
                fallback.push_str(net);
                code.push_str(spelling);
            }
            M2Value::Token { text, role } if *role != crate::backend::Role::Type => {
                tex.push_str(&maths::typewriter(text));
                fallback.push_str(text);
                code.push_str(text);
            }
            _ => return None,
        }
    }
    Some(M2Value::Tex {
        source: format!("${tex}$"),
        fallback,
        code,
    })
}

struct Block {
    lines: Vec<Line<'static>>,
    baseline: usize,
}

impl Block {
    fn text(text: &str, ink: Style) -> Self {
        let lines: Vec<_> = text
            .split('\n')
            .map(|line| Line::from(Span::styled(line.to_string(), ink)))
            .collect();
        Self {
            baseline: lines.len().saturating_sub(1) / 2,
            lines,
        }
    }

    fn wrapped(text: &str, ink: Style, width: usize) -> Self {
        let width = u16::try_from(width).unwrap_or(u16::MAX);
        Self::text(WrappedText::word(text, width).text(), ink)
    }

    fn width(&self) -> usize {
        self.lines.iter().map(Line::width).max().unwrap_or(0)
    }

    fn beside(self, other: Self) -> Self {
        let baseline = self.baseline.max(other.baseline);
        let below = (self.lines.len() - self.baseline).max(other.lines.len() - other.baseline);
        let mut lines = vec![Line::default(); baseline + below];
        for block in [self, other] {
            let width = block.width();
            let offset = baseline - block.baseline;
            let height = block.lines.len();
            for (row, mut line) in block.lines.into_iter().enumerate() {
                let padding = width - line.width();
                line.spans.push(Span::raw(" ".repeat(padding)));
                lines[row + offset].spans.extend(line.spans);
            }
            for (row, line) in lines.iter_mut().enumerate() {
                if row < offset || row >= offset + height {
                    line.spans.push(Span::raw(" ".repeat(width)));
                }
            }
        }
        Self { lines, baseline }
    }

    /// Wrap between complete values, retaining each value's baseline and image rows.
    fn pack(blocks: impl IntoIterator<Item = Self>, width: usize) -> Vec<Self> {
        let mut rows = Vec::new();
        let mut row = Self::text("", Style::default());
        for block in blocks {
            if row.width() > 0 && row.width() + block.width() > width {
                rows.push(row);
                row = Self::text("", Style::default());
            }
            row = row.beside(block);
        }
        rows.push(row);
        rows
    }

    fn fence(&self, text: &str, groups: &Groups, ink: Option<Style>) -> Self {
        let fence = MathBlock::fence(text, self.lines.len(), self.baseline);
        let baseline = fence.baseline();
        let lines = fence
            .into_rows()
            .into_iter()
            .map(|row| {
                let mut spans = row.spans(groups);
                if let Some(ink) = ink {
                    for span in &mut spans {
                        span.style = ink;
                    }
                }
                Line::from(spans)
            })
            .collect();
        Self { lines, baseline }
    }

    fn fenced(self, open: &str, close: &str, groups: &Groups, ink: Option<Style>) -> Self {
        let left = self.fence(open, groups, ink);
        let right = self.fence(close, groups, ink);
        left.beside(self).beside(right)
    }

    fn value(
        value: &M2Value,
        width: usize,
        groups: &Groups,
        style: MathStyle,
        ink: Option<Style>,
        picture: Picture,
    ) -> Self {
        let text_ink = |kind| ink.unwrap_or_else(|| groups.style(kind));
        match value {
            M2Value::Type(value) => Self::value(
                value,
                width,
                groups,
                style,
                Some(text_ink(HLGroup::Type)),
                picture,
            ),
            M2Value::Tex {
                source,
                fallback,
                code,
            } => {
                if let Some(tex) = source
                    .strip_prefix('$')
                    .and_then(|tex| tex.strip_suffix('$'))
                    .map(maths::pictured_tex)
                {
                    let fg = text_ink(HLGroup::Value).fg;
                    if let Some(Color::Rgb(r, g, b)) = fg {
                        let painted = match ink {
                            Some(_) => tex.clone(),
                            None => maths::colored_tex(&tex, groups),
                        };
                        let formula = Typesetting {
                            tex: &tex,
                            painted: &painted,
                            code,
                        };
                        if let Some(Pictured { lines, baseline }) = picture(
                            formula,
                            Rgb([r, g, b]),
                            u16::try_from(width).unwrap_or(u16::MAX),
                        ) {
                            return Self { lines, baseline };
                        }
                    }
                }
                let lines = maths::spans(value, width, style).ok().map(|rows| {
                    rows.into_iter()
                        .map(|row| {
                            let mut spans = row.spans(groups);
                            if let Some(ink) = ink {
                                for span in &mut spans {
                                    span.style = ink;
                                }
                            }
                            Line::from(spans)
                        })
                        .collect::<Vec<_>>()
                });
                match lines.filter(|lines| !lines.is_empty()) {
                    Some(lines) => Self {
                        baseline: lines.len().saturating_sub(1) / 2,
                        lines,
                    },
                    None => Self::wrapped(fallback, text_ink(HLGroup::Value), width),
                }
            }
            M2Value::Token { text, role } => Self::wrapped(text, text_ink(role.group()), width),
            M2Value::Net(text) | M2Value::Markup { fallback: text, .. } => {
                Self::wrapped(text, text_ink(HLGroup::Value), width)
            }
            M2Value::Collection { kind, items } if *kind != CollectionKind::VerticalList => {
                let (open, close) = kind.delimiters();
                let budget = width.saturating_sub(open.len() + close.len()).max(1);
                let entries = items.iter().enumerate().map(|(index, item)| {
                    let entry = Self::value(item, budget, groups, style, ink, picture);
                    if index + 1 < items.len() {
                        entry.beside(Self::text(", ", text_ink(HLGroup::Punctuation)))
                    } else {
                        entry
                    }
                });
                let rows = Self::pack(entries, budget);
                let count = rows.len();
                let mut lines = Vec::new();
                let mut baseline = None;
                for (index, row) in rows.into_iter().enumerate() {
                    let left = if index == 0 {
                        row.fence(open, groups, ink)
                    } else {
                        Self::text(&" ".repeat(open.len()), Style::default())
                    };
                    let right = (index + 1 == count).then(|| row.fence(close, groups, ink));
                    let mut row = left.beside(row);
                    if let Some(right) = right {
                        row = row.beside(right);
                    }
                    baseline = (count == 1).then_some(row.baseline);
                    lines.extend(row.lines);
                }
                Self {
                    baseline: baseline.unwrap_or(lines.len().saturating_sub(1) / 2),
                    lines,
                }
            }
            M2Value::Collection { items, .. } => Self::vertical(
                items
                    .iter()
                    .map(|item| {
                        Self::value(
                            item,
                            width.saturating_sub(4).max(1),
                            groups,
                            style,
                            ink,
                            picture,
                        )
                    })
                    .collect(),
                groups,
                ink,
            ),
            M2Value::HashTable(entries) => Self::vertical(
                entries
                    .iter()
                    .map(|[key, value]| {
                        Self::value(
                            key,
                            width.saturating_sub(4).max(1),
                            groups,
                            style,
                            ink,
                            picture,
                        )
                        .beside(Self::text(SPACED_OPTION_ARROW, text_ink(HLGroup::Operator)))
                        .beside(Self::value(
                            value,
                            width.saturating_sub(4).max(1),
                            groups,
                            style,
                            ink,
                            picture,
                        ))
                    })
                    .collect(),
                groups,
                ink,
            ),
        }
    }

    fn vertical(rows: Vec<Self>, groups: &Groups, ink: Option<Style>) -> Self {
        if rows.is_empty() {
            return Self::text(
                "{}",
                ink.unwrap_or_else(|| groups.style(HLGroup::Delimiter)),
            );
        }
        let lines: Vec<_> = rows.into_iter().flat_map(|row| row.lines).collect();
        let body = Self {
            baseline: lines.len().saturating_sub(1) / 2,
            lines,
        };
        Self::text(" ", Style::default())
            .beside(body)
            .beside(Self::text(" ", Style::default()))
            .fenced("{", "}", groups, ink)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Role;

    #[test]
    fn mathematical_types_keep_their_colour_in_pictures_and_text_fallbacks() {
        let groups = Groups::default();
        let style = MathStyle::default();
        let value = M2Value::Type(Box::new(M2Value::Tex {
            source: "$R$".into(),
            fallback: "R".into(),
            code: "R".into(),
        }));
        let picture: Picture = |formula, ink, _| {
            let groups = Groups::default();
            let Some(Color::Rgb(r, g, b)) = groups.style(HLGroup::Type).fg else {
                panic!("an RGB type colour");
            };
            assert_eq!(ink, Rgb([r, g, b]));
            assert_eq!(formula.tex, "R");
            assert_eq!(formula.painted, "R");
            assert_eq!(formula.code, "R");
            Some(Pictured {
                lines: vec![Line::raw("R")],
                baseline: 0,
            })
        };
        let block = Block::value(&value, 80, &groups, style, None, picture);
        assert_eq!(text(&block.lines), ["R"]);
        let fallback = Block::value(&value, 80, &groups, style, None, |_, _, _| None);
        assert_eq!(text(&fallback.lines), ["R"]);
        assert!(fallback
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .all(|span| span.style == groups.style(HLGroup::Type)));
    }

    #[test]
    fn an_option_arrow_is_pictured_long() {
        let groups = Groups::default();
        let value = M2Value::Tex {
            source: r"$a\ \Rightarrow \ 1$".into(),
            fallback: "a => 1".into(),
            code: "a => 1".into(),
        };
        let picture: Picture = |formula, _, _| {
            Some(Pictured {
                lines: vec![Line::raw(formula.tex.to_string())],
                baseline: 0,
            })
        };
        let block = Block::value(&value, 80, &groups, MathStyle::default(), None, picture);
        assert_eq!(text(&block.lines), [r"a\ \Longrightarrow \ 1"]);
    }

    #[test]
    fn the_imaginary_unit_is_pictured_upright() {
        let groups = Groups::default();
        let value = M2Value::Tex {
            source: r"${2}\,\mathbf{i}$".into(),
            fallback: "2*ii".into(),
            code: "2*ii".into(),
        };
        let picture: Picture = |formula, _, _| {
            Some(Pictured {
                lines: vec![Line::raw(formula.tex.to_string())],
                baseline: 0,
            })
        };
        let block = Block::value(&value, 80, &groups, MathStyle::default(), None, picture);
        assert_eq!(text(&block.lines), [r"{2}\,\mathrm{i}"]);
    }

    fn picture(formula: Typesetting, _: Rgb, _: u16) -> Option<Pictured> {
        assert_eq!(formula.tex, r"\frac{2}{3}");
        Some(Pictured {
            lines: vec![Line::raw("2"), Line::raw("─"), Line::raw("3")],
            baseline: 1,
        })
    }

    #[test]
    fn a_table_key_lines_up_with_the_baseline_of_a_tall_value() {
        let groups = Groups::default();
        let value = M2Value::HashTable(vec![[
            M2Value::Net("key".into()),
            M2Value::Tex {
                source: r"$\frac{2}{3}$".into(),
                fallback: "2/3".into(),
                code: "2/3".into(),
            },
        ]]);
        let block = Block::value(&value, 80, &groups, MathStyle::default(), None, picture);
        let drawn = text(&block.lines);
        assert_eq!(drawn.len(), 3);
        assert!(
            drawn[1].contains("key") && drawn[1].contains("─"),
            "{drawn:?}"
        );
    }

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn angle_bar_delimiters_align_with_a_tall_formula_baseline() {
        let groups = Groups::default();
        let value = M2Value::Collection {
            kind: CollectionKind::AngleBarList,
            items: vec![M2Value::Tex {
                source: r"$\frac{2}{3}$".into(),
                fallback: "2/3".into(),
                code: "2/3".into(),
            }],
        };
        let block = Block::value(&value, 80, &groups, MathStyle::default(), None, picture);
        assert_eq!(text(&block.lines), ["  2  ", "<|─|>", "  3  "]);
        assert_eq!(block.baseline, 1);
    }

    #[test]
    fn a_net_does_not_change_its_mathematical_siblings() {
        let groups = Groups::default();
        let fraction = M2Value::Tex {
            source: r"$\frac{2}{3}$".into(),
            fallback: "2/3".into(),
            code: String::new(),
        };
        let string = M2Value::Token {
            text: "2/3".into(),
            role: Role::String,
        };
        let null = M2Value::Token {
            text: "null".into(),
            role: Role::Null,
        };
        for net in [false, true] {
            let mut items = vec![fraction.clone(), string.clone(), null.clone()];
            if net {
                items.push(M2Value::Net("2/3".into()));
            }
            let value = M2Value::Collection {
                kind: CollectionKind::Sequence,
                items,
            };
            let block = Block::value(&value, 80, &groups, MathStyle::default(), None, picture);
            let drawn = text(&block.lines);
            assert_eq!(block.lines.len(), 3);
            assert!(drawn[1].contains("─, 2/3, null"), "{drawn:?}");
            assert!(block.lines[1]
                .spans
                .iter()
                .any(|span| span.content == "2/3" && span.style == groups.style(HLGroup::String)));
            assert!(
                block.lines[1]
                    .spans
                    .iter()
                    .any(|span| span.content == "null"
                        && span.style == groups.style(HLGroup::Builtin))
            );
        }
    }

    #[test]
    fn annotation_formulas_keep_component_colours() {
        let groups = Groups::default();
        let parts = [M2Value::Tex {
            source: r"$\texttt{Proj}\left({\mathbb R}_{53}\left[x_1\right]\right)$".into(),
            fallback: "Proj(RR_53[x_1])".into(),
            code: "Proj(RR_53[x_1])".into(),
        }];
        let rows = maths::annotation(&parts, 80, MathStyle::default());
        let drawn = annotation(&parts, 80, &groups, MathStyle::default());
        for kind in [
            HLGroup::Type,
            HLGroup::Variable,
            HLGroup::Number,
            HLGroup::Delimiter,
        ] {
            assert!(rows.iter().flatten().any(|span| span.kind == kind));
            assert!(drawn
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| span.style == groups.style(kind)));
        }
        let joined = formula(&parts).expect("a combined annotation formula");
        let M2Value::Tex { source, .. } = joined else {
            panic!("an annotation formula has TeX");
        };
        let painted = maths::colored_tex(source.trim_matches('$'), &groups);
        for kind in [HLGroup::Variable, HLGroup::Number, HLGroup::Delimiter] {
            let Some(Color::Rgb(r, g, b)) = groups.style(kind).fg else {
                panic!("an RGB component colour");
            };
            assert!(painted.contains(&format!(r"\color{{#{r:02x}{g:02x}{b:02x}}}")));
        }
    }

    #[test]
    fn annotation_pieces_keep_their_own_spacing() {
        let parts = [
            M2Value::Tex {
                source: r"${\mathbb R}$".into(),
                fallback: "RR".into(),
                code: String::new(),
            },
            M2Value::Token {
                text: " (of precision ".into(),
                role: Role::String,
            },
            M2Value::Tex {
                source: "$53$".into(),
                fallback: "53".into(),
                code: String::new(),
            },
            M2Value::Token {
                text: ")".into(),
                role: Role::String,
            },
        ];
        let groups = Groups::default();
        let drawn = text(&annotation(&parts, 80, &groups, MathStyle::default()));
        assert_eq!(drawn.len(), 1);
        assert!(drawn[0].ends_with(" (of precision 53)"), "{drawn:?}");
        assert!(!drawn[0].contains("  "), "{drawn:?}");
        let unicode: Vec<String> = maths::annotation(&parts, 80, MathStyle::default())
            .iter()
            .map(|row| row.iter().map(|span| span.content.as_str()).collect())
            .collect();
        assert_eq!(unicode, drawn);
    }

    #[test]
    fn ordinary_values_never_enter_the_picture_backend() {
        let groups = Groups::default();
        let value = M2Value::Collection {
            kind: CollectionKind::List,
            items: vec![
                M2Value::Token {
                    text: r"\frac{2}{3}".into(),
                    role: Role::String,
                },
                M2Value::Token {
                    text: "null".into(),
                    role: Role::Null,
                },
                M2Value::Net("literal".into()),
            ],
        };
        let block = Block::value(
            &value,
            80,
            &groups,
            MathStyle::default(),
            None,
            |_, _, _| panic!("ordinary text must not be typeset"),
        );
        assert_eq!(text(&block.lines), [r"{\frac{2}{3}, null, literal}"]);
    }

    #[test]
    fn text_wraps_within_the_space_reserved_for_collection_delimiters() {
        let groups = Groups::default();
        let value = M2Value::Collection {
            kind: CollectionKind::Sequence,
            items: vec![M2Value::Token {
                text: "abcdefghij".into(),
                role: Role::String,
            }],
        };
        let block = Block::value(&value, 6, &groups, MathStyle::default(), None, picture);
        assert!(block.lines.iter().all(|line| line.width() <= 6));
        assert_eq!(block.lines.len(), 3);
    }
}
