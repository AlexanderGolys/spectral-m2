//! Typeset mathematical leaves composed with terminal text around a shared baseline.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::backend::{CollectionKind, M2Value, SPACED_OPTION_ARROW};
use crate::ui::block::Text;
use crate::ui::controller::settings::theme::groups::{Groups, HLGroup};
use crate::ui::graphics::{self, typeset::Rgb, Pictured, Typesetting};
use crate::ui::wrap::WrappedText;

use super::maths::{self, MathBlock, MathStyle};

/// What M2 sets between the items of a collection.
const COLLECTION_TEX_SEPARATOR: &str = r",\,";
/// What M2 prints between the items of a collection.
const COLLECTION_SEPARATOR: &str = ", ";

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

/// The pieces of a type annotation, which carry their own spacing, set together in
/// the type colour: as one formula with its words in a typewriter font when every piece has a TeX
/// form, so cell boundaries never widen the gaps between them; else side by side.
pub fn annotation(
    parts: &[M2Value],
    width: usize,
    groups: &Groups,
    style: MathStyle,
) -> Vec<Line<'static>> {
    let ink = Some(groups.style(HLGroup::Type));
    let block =
        |part: &M2Value| Block::value(part, width.max(1), groups, style, ink, graphics::typeset);
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

/// A collection of TeX values as one formula, so cell boundaries never widen the gaps
/// between its items and its fences grow around them; none when an item has no TeX form.
fn collection_formula(kind: CollectionKind, items: &[M2Value]) -> Option<M2Value> {
    if kind == CollectionKind::VerticalList || items.is_empty() {
        return None;
    }
    let (tex_open, tex_close) = kind.tex_delimiters();
    let (open, close) = kind.delimiters();
    let (mut tex, mut fallback, mut code) = (Vec::new(), Vec::new(), Vec::new());
    for item in items {
        let nested = match item {
            M2Value::Collection { kind, items } => Some(collection_formula(*kind, items)?),
            _ => None,
        };
        let M2Value::Tex {
            source,
            fallback: net,
            code: spelling,
        } = nested.as_ref().unwrap_or(item)
        else {
            return None;
        };
        tex.push(source.strip_prefix('$')?.strip_suffix('$')?.to_string());
        fallback.push(net.clone());
        code.push(spelling.clone());
    }
    Some(M2Value::Tex {
        source: format!(
            "${tex_open}{}{tex_close}$",
            tex.join(COLLECTION_TEX_SEPARATOR)
        ),
        fallback: format!("{open}{}{close}", fallback.join(COLLECTION_SEPARATOR)),
        code: format!("{open}{}{close}", code.join(COLLECTION_SEPARATOR)),
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

    /// The `$`-delimited TeX `source` as a picture, when it can be set in `width`.
    fn pictured(
        source: &str,
        code: &str,
        width: usize,
        groups: &Groups,
        style: MathStyle,
        ink: Option<Style>,
        picture: Picture,
    ) -> Option<Self> {
        let tex = maths::pictured_tex(source.strip_prefix('$')?.strip_suffix('$')?);
        let Some(Color::Rgb(r, g, b)) = ink.unwrap_or_else(|| groups.style(HLGroup::Value)).fg
        else {
            return None;
        };
        let painted = match ink {
            Some(_) => tex.clone(),
            None => maths::colored_tex(&tex, groups),
        };
        let formula = Typesetting {
            tex: &tex,
            painted: &painted,
            code,
            matrix_row_spacing_percent: style.matrix_row_spacing_percent,
        };
        let Pictured { lines, baseline } = picture(
            formula,
            Rgb([r, g, b]),
            u16::try_from(width).unwrap_or(u16::MAX),
        )?;
        Some(Self { lines, baseline })
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
                if let Some(block) =
                    Self::pictured(source, code, width, groups, style, ink, picture)
                {
                    return block;
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
            M2Value::Net(text) => Self::wrapped(text, text_ink(HLGroup::String), width),
            M2Value::Markup { fallback, .. } => {
                Self::wrapped(fallback, text_ink(HLGroup::Value), width)
            }
            M2Value::Collection { kind, items } if *kind != CollectionKind::VerticalList => {
                let whole = collection_formula(*kind, items).and_then(|formula| match &formula {
                    M2Value::Tex { source, code, .. } => {
                        Self::pictured(source, code, width, groups, style, ink, picture)
                    }
                    _ => None,
                });
                if let Some(block) = whole {
                    return block;
                }
                let (open, close) = kind.drawn_delimiters();
                let budget = width.saturating_sub(open.width() + close.width()).max(1);
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
                        Self::text(&" ".repeat(open.width()), Style::default())
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
            M2Value::Option { key, value } => {
                let part = |part| Self::value(part, width, groups, style, ink, picture);
                part(key)
                    .beside(Self::text(SPACED_OPTION_ARROW, text_ink(HLGroup::Operator)))
                    .beside(part(value))
            }
            M2Value::HashTable(entries) => {
                let part = |part| {
                    Self::value(
                        part,
                        width.saturating_sub(4).max(1),
                        groups,
                        style,
                        ink,
                        picture,
                    )
                };
                Self::vertical(
                    entries
                        .iter()
                        .map(|[key, value]| {
                            part(key)
                                .beside(Self::text(
                                    SPACED_OPTION_ARROW,
                                    text_ink(HLGroup::Operator),
                                ))
                                .beside(part(value))
                        })
                        .collect(),
                    groups,
                    ink,
                )
            }
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
    fn an_annotation_is_wholly_in_the_type_colour() {
        let groups = Groups::default();
        let word = |text: &str, role| M2Value::Token {
            text: text.into(),
            role,
        };
        for parts in [
            vec![
                word("Expression", Role::Type),
                word(" of class ", Role::String),
                word("Holder", Role::Type),
            ],
            vec![word("free module", Role::String)],
            // No pictures in this test's terminal, so the maths falls back to rows.
            vec![
                word("Matrix", Role::Type),
                M2Value::Tex {
                    source: "$R^{2}$".into(),
                    fallback: "R^2".into(),
                    code: "R^2".into(),
                },
            ],
        ] {
            let lines = annotation(&parts, 80, &groups, MathStyle::default());
            for span in lines.iter().flat_map(|line| &line.spans) {
                assert!(
                    span.content.trim().is_empty() || span.style == groups.style(HLGroup::Type),
                    "{span:?}"
                );
            }
        }
    }

    #[test]
    fn a_net_is_coloured_as_a_string() {
        let groups = Groups::default();
        let value = M2Value::Net("LOUD".into());
        let block = Block::value(
            &value,
            80,
            &groups,
            MathStyle::default(),
            None,
            |_, _, _| None,
        );
        assert!(block.lines[0]
            .spans
            .iter()
            .any(|span| span.content == "LOUD" && span.style == groups.style(HLGroup::String)));
    }

    #[test]
    fn a_list_of_options_is_text_around_any_maths_in_it() {
        let groups = Groups::default();
        let name = |text: &str| {
            Box::new(M2Value::Token {
                text: text.into(),
                role: Role::Symbol,
            })
        };
        let names_only = M2Value::Collection {
            kind: CollectionKind::List,
            items: vec![M2Value::Option {
                key: name("Strategy"),
                value: Box::new(M2Value::Token {
                    text: "false".into(),
                    role: Role::Boolean,
                }),
            }],
        };
        let block = Block::value(
            &names_only,
            80,
            &groups,
            MathStyle::default(),
            None,
            |_, _, _| panic!("an option of names has no maths to picture"),
        );
        assert_eq!(
            text(&block.lines),
            [format!("{{Strategy{SPACED_OPTION_ARROW}false}}")]
        );

        let with_maths = M2Value::Collection {
            kind: CollectionKind::List,
            items: vec![M2Value::Option {
                key: name("half"),
                value: Box::new(M2Value::Tex {
                    source: r"$\frac{1}{2}$".into(),
                    fallback: "1/2".into(),
                    code: "1/2".into(),
                }),
            }],
        };
        let block = Block::value(
            &with_maths,
            80,
            &groups,
            MathStyle::default(),
            None,
            spelled,
        );
        let drawn = text(&block.lines).join("");
        assert!(drawn.contains("half"), "{drawn}");
        assert!(drawn.contains(SPACED_OPTION_ARROW), "{drawn}");
        assert!(!drawn.contains(r"\Rightarrow"), "{drawn}");
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

    fn tex(source: &str, code: &str) -> M2Value {
        M2Value::Tex {
            source: source.into(),
            fallback: code.into(),
            code: code.into(),
        }
    }

    /// A picture of one row spelling out the formula and the code it copies.
    fn spelled(formula: Typesetting, _: Rgb, _: u16) -> Option<Pictured> {
        Some(Pictured {
            lines: vec![Line::raw(format!("{} = {}", formula.tex, formula.code))],
            baseline: 0,
        })
    }

    #[test]
    fn a_collection_of_formulas_is_one_picture() {
        let groups = Groups::default();
        let value = M2Value::Collection {
            kind: CollectionKind::Array,
            items: vec![
                M2Value::Collection {
                    kind: CollectionKind::AngleBarList,
                    items: vec![tex("$1$", "1"), tex("$2$", "2")],
                },
                M2Value::Collection {
                    kind: CollectionKind::Sequence,
                    items: vec![tex("$3$", "3")],
                },
            ],
        };
        let block = Block::value(&value, 80, &groups, MathStyle::default(), None, spelled);
        assert_eq!(
            text(&block.lines),
            [concat!(
                r"\left[\left\langle\!\left|1,\,2\right|\!\right\rangle,\,\left(3\right)\right]",
                " = [<|1, 2|>, (3)]"
            )]
        );
    }

    #[test]
    fn the_fences_of_a_collection_formula_are_coloured_with_its_items() {
        let groups = Groups::default();
        for kind in [
            CollectionKind::List,
            CollectionKind::Array,
            CollectionKind::Sequence,
            CollectionKind::AngleBarList,
        ] {
            let Some(M2Value::Tex { source, .. }) =
                collection_formula(kind, &[tex("$1$", "1"), tex("$2$", "2")])
            else {
                panic!("a formula for {kind:?}");
            };
            let tex = source.trim_matches('$');
            assert_ne!(maths::colored_tex(tex, &groups), tex, "{kind:?}");
        }
    }

    #[test]
    fn a_collection_with_text_is_drawn_item_by_item() {
        let groups = Groups::default();
        let value = M2Value::Collection {
            kind: CollectionKind::AngleBarList,
            items: vec![
                tex(r"$\frac{2}{3}$", "2/3"),
                M2Value::Token {
                    text: "null".into(),
                    role: Role::Null,
                },
            ],
        };
        let block = Block::value(&value, 80, &groups, MathStyle::default(), None, picture);
        let drawn = text(&block.lines);
        assert_eq!(block.baseline, 1);
        assert!(
            drawn[1].starts_with("\u{27E8}|─, null|\u{27E9}"),
            "{drawn:?}"
        );
    }

    #[test]
    fn a_collection_too_wide_to_picture_is_drawn_item_by_item() {
        let groups = Groups::default();
        let value = M2Value::Collection {
            kind: CollectionKind::Sequence,
            items: vec![tex("$1$", "1"), tex("$2$", "2")],
        };
        let wholes_refused: Picture = |formula, _, _| {
            (!formula.tex.contains(r"\left")).then(|| Pictured {
                lines: vec![Line::raw(formula.tex.to_string())],
                baseline: 0,
            })
        };
        let block = Block::value(
            &value,
            80,
            &groups,
            MathStyle::default(),
            None,
            wholes_refused,
        );
        assert_eq!(text(&block.lines), ["(1, 2)"]);
    }

    #[test]
    fn a_line_of_angle_bar_items_wraps_under_its_first_item() {
        let groups = Groups::default();
        let value = M2Value::Collection {
            kind: CollectionKind::AngleBarList,
            items: ["aaaa", "bbbb"]
                .map(|text| M2Value::Token {
                    text: text.into(),
                    role: Role::String,
                })
                .to_vec(),
        };
        let block = Block::value(&value, 10, &groups, MathStyle::default(), None, picture);
        let drawn = text(&block.lines);
        assert_eq!(drawn.len(), 2, "{drawn:?}");
        let (open, _) = CollectionKind::AngleBarList.drawn_delimiters();
        assert!(drawn[1].starts_with(&" ".repeat(open.width())), "{drawn:?}");
        assert!(
            block.lines.iter().all(|line| line.width() <= 10),
            "{drawn:?}"
        );
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
    fn annotation_formulas_are_wholly_in_the_type_colour() {
        let groups = Groups::default();
        let parts = [M2Value::Tex {
            source: r"$\texttt{Proj}\left({\mathbb R}_{53}\left[x_1\right]\right)$".into(),
            fallback: "Proj(RR_53[x_1])".into(),
            code: "Proj(RR_53[x_1])".into(),
        }];
        let rows = maths::annotation(&parts, 80, MathStyle::default());
        assert!(rows
            .iter()
            .flatten()
            .all(|span| span.content.trim().is_empty() || span.kind == HLGroup::Type));
        let drawn = annotation(&parts, 80, &groups, MathStyle::default());
        assert!(drawn.iter().flat_map(|line| &line.spans).all(|span| span
            .content
            .trim()
            .is_empty()
            || span.style == groups.style(HLGroup::Type)));
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
