//! Typeset mathematical leaves composed with terminal text around a shared baseline.

use ratatui::layout::{Offset, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::backend::{CollectionKind, M2Value, Role, SPACED_OPTION_ARROW};
use crate::ui::block::{Link, Text};
use crate::ui::graphics::{self, ratex::Rgb, Pictured, Typesetting};
use crate::ui::theme::groups::{Groups, HLGroup};
use crate::ui::wrap::WrappedText;

use super::maths::{self, MathBlock, MathInk, MathStyle};

/// What M2 sets between the items of a collection.
const COLLECTION_TEX_SEPARATOR: &str = r",\,";
/// What M2 prints between the items of a collection.
const COLLECTION_SEPARATOR: &str = ", ";

/// The image backend, returning terminal-cell rows for one formula.
type Picture = fn(Typesetting, Rgb, u16) -> Option<Pictured>;

/// How a piece of TeX was drawn, the least faithful first so that `min` finds the
/// weakest piece of a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Form {
    /// M2's own plain-text spelling, because nothing else could draw it.
    Net,
    Unicode,
    Picture,
}

impl Form {
    /// The weaker of two forms, where a part that is no TeX has none.
    fn weakest(first: Option<Self>, second: Option<Self>) -> Option<Self> {
        match (first, second) {
            (Some(first), Some(second)) => Some(first.min(second)),
            (first, second) => first.or(second),
        }
    }
}

/// A value's rows as drawn, the links among them, and the weakest form that drew
/// any TeX in them.
pub struct Drawn {
    pub lines: Vec<Line<'static>>,
    pub links: Vec<Link>,
    pub form: Option<Form>,
}

/// Render each value in its own representation, without converting text into TeX.
pub fn drawn(value: &M2Value, width: usize, groups: &Groups, style: MathStyle) -> Drawn {
    let picture: Picture = match value {
        M2Value::Tex { .. } => graphics::typeset_alone,
        _ => graphics::typeset,
    };
    let ink = (style.ink == MathInk::Monochrome).then(|| groups.style(HLGroup::Value));
    let Block {
        lines, links, form, ..
    } = Block::value(value, width.max(1), groups, style, ink, picture);
    Drawn { lines, links, form }
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

/// A piece of a formula: its TeX, the spelling M2 prints it with, and the one-line
/// code M2 reads back.
struct FormulaPart {
    tex: String,
    fallback: String,
    code: String,
    /// Whether any of it is maths, as against words only, which are better left as
    /// the terminal's own text.
    maths: bool,
}

/// `text` as typewriter text in the colour `group` has in code. The colour is
/// explicit, because the formula would otherwise guess it from how the word is spelt.
fn typewritten(text: &str, group: HLGroup, groups: &Groups) -> String {
    let tex = maths::typewriter(text);
    match groups.style(group).fg {
        Some(Color::Rgb(r, g, b)) => format!(r"\textcolor{{#{r:02x}{g:02x}{b:02x}}}{{{tex}}}"),
        _ => tex,
    }
}

/// One item of a collection as part of a formula; none when it has no TeX form. What
/// is not maths, such as a symbol or a string, is set in typewriter face.
fn formula_part(item: &M2Value, groups: &Groups) -> Option<FormulaPart> {
    let word = |text: &String, group, code: String| FormulaPart {
        tex: typewritten(text, group, groups),
        fallback: text.clone(),
        code,
        maths: false,
    };
    match item {
        M2Value::Tex {
            source,
            fallback,
            code,
        } => Some(FormulaPart {
            tex: source.strip_prefix('$')?.strip_suffix('$')?.to_string(),
            fallback: fallback.clone(),
            code: code.clone(),
            maths: true,
        }),
        M2Value::Type(item) => formula_part(item, groups),
        M2Value::Collection { kind, items } => collection_part(*kind, items, groups),
        M2Value::Token {
            text,
            role: Role::String,
        } => Some(word(
            text,
            HLGroup::String,
            format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"")),
        )),
        M2Value::Token { text, role } => Some(word(text, role.group(), text.clone())),
        M2Value::Net(text) if !text.contains('\n') => {
            Some(word(text, HLGroup::String, text.clone()))
        }
        M2Value::Location { text, .. } if !text.contains('\n') => {
            Some(word(text, HLGroup::MarkupLink, text.clone()))
        }
        _ => None,
    }
}

/// A collection as one formula, so cell boundaries never widen the gaps between its
/// items and its fences grow around them; none when an item has no TeX form, or when
/// nothing in it is maths.
fn collection_formula(
    kind: CollectionKind,
    items: &[M2Value],
    groups: &Groups,
) -> Option<FormulaPart> {
    collection_part(kind, items, groups).filter(|part| part.maths)
}

fn collection_part(
    kind: CollectionKind,
    items: &[M2Value],
    groups: &Groups,
) -> Option<FormulaPart> {
    if kind == CollectionKind::VerticalList || items.is_empty() {
        return None;
    }
    let (tex_open, tex_close) = kind.tex_delimiters();
    let (open, close) = kind.delimiters();
    let parts = items
        .iter()
        .map(|item| formula_part(item, groups))
        .collect::<Option<Vec<_>>>()?;
    let joined = |spelling: fn(&FormulaPart) -> &String, separator: &str| {
        parts
            .iter()
            .map(spelling)
            .cloned()
            .collect::<Vec<_>>()
            .join(separator)
    };
    Some(FormulaPart {
        tex: format!(
            "{tex_open}{}{tex_close}",
            joined(|part| &part.tex, COLLECTION_TEX_SEPARATOR)
        ),
        fallback: format!(
            "{open}{}{close}",
            joined(|part| &part.fallback, COLLECTION_SEPARATOR)
        ),
        code: format!(
            "{open}{}{close}",
            joined(|part| &part.code, COLLECTION_SEPARATOR)
        ),
        maths: parts.iter().any(|part| part.maths),
    })
}

struct Block {
    lines: Vec<Line<'static>>,
    baseline: usize,
    /// The links drawn among `lines`, placed from the block's top left corner.
    links: Vec<Link>,
    /// The weakest form that drew TeX among `lines`; none when none was.
    form: Option<Form>,
}

impl Block {
    fn new(lines: Vec<Line<'static>>, baseline: usize) -> Self {
        Self {
            lines,
            baseline,
            links: Vec::new(),
            form: None,
        }
    }

    fn drawn_as(self, form: Form) -> Self {
        Self {
            form: Some(form),
            ..self
        }
    }

    /// Lines set on their middle row, or on the upper of the two middle rows.
    fn centred(lines: Vec<Line<'static>>) -> Self {
        let baseline = lines.len().saturating_sub(1) / 2;
        Self::new(lines, baseline)
    }

    fn text(text: &str, ink: Style) -> Self {
        Self::centred(
            text.split('\n')
                .map(|line| Line::from(Span::styled(line.to_string(), ink)))
                .collect(),
        )
    }

    /// The same block, all of it a link to `target`.
    fn linked(mut self, target: &str) -> Self {
        let cells = self
            .lines
            .iter()
            .enumerate()
            .filter_map(|(row, line)| {
                let width = line.to_string().trim_end().width();
                (width > 0).then(|| Rect::new(0, row as u16, width as u16, 1))
            })
            .collect();
        self.links = vec![Link {
            target: target.into(),
            cells,
        }];
        self
    }

    /// Blocks one below another, each keeping its own links.
    fn stacked(blocks: Vec<Self>) -> Self {
        let mut lines = Vec::new();
        let mut links = Vec::new();
        let mut form = None;
        for block in blocks {
            form = Form::weakest(form, block.form);
            let offset = Offset {
                x: 0,
                y: lines.len() as i32,
            };
            links.extend(block.links.into_iter().map(|link| link.moved(offset)));
            lines.extend(block.lines);
        }
        Self {
            links,
            form,
            ..Self::centred(lines)
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
        let mut links = Vec::new();
        let mut column = 0;
        let mut form = None;
        for block in [self, other] {
            form = Form::weakest(form, block.form);
            let width = block.width();
            let offset = baseline - block.baseline;
            let height = block.lines.len();
            let placed = Offset {
                x: column as i32,
                y: offset as i32,
            };
            links.extend(block.links.into_iter().map(|link| link.moved(placed)));
            column += width;
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
        Self {
            lines,
            baseline,
            links,
            form,
        }
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
        Self::new(lines, baseline)
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
            None => maths::colored_tex(&tex, groups, style),
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
        Some(Self::new(lines, baseline).drawn_as(Form::Picture))
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
                Some(groups.style(HLGroup::Type)),
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
                    Some(lines) => Self::centred(lines).drawn_as(Form::Unicode),
                    None => {
                        Self::wrapped(fallback, text_ink(HLGroup::Value), width).drawn_as(Form::Net)
                    }
                }
            }
            M2Value::Token { text, role } => Self::wrapped(text, text_ink(role.group()), width),
            M2Value::Linked { value, target } => {
                Self::value(value, width, groups, style, ink, picture).linked(target)
            }
            M2Value::Location { text, .. } => {
                Self::wrapped(text, text_ink(HLGroup::MarkupLink), width)
            }
            M2Value::Net(text) => Self::wrapped(text, text_ink(HLGroup::String), width),
            M2Value::Markup { fallback, .. } => {
                Self::wrapped(fallback, text_ink(HLGroup::Value), width)
            }
            M2Value::Collection { kind, items } if *kind != CollectionKind::VerticalList => {
                let whole = collection_formula(*kind, items, groups).and_then(|part| {
                    let source = format!("${}$", part.tex);
                    Self::pictured(&source, &part.code, width, groups, style, ink, picture)
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
                let rows: Vec<_> = rows
                    .into_iter()
                    .enumerate()
                    .map(|(index, row)| {
                        let left = if index == 0 {
                            row.fence(open, groups, ink)
                        } else {
                            Self::text(&" ".repeat(open.width()), Style::default())
                        };
                        let right = (index + 1 == count).then(|| row.fence(close, groups, ink));
                        let row = left.beside(row);
                        match right {
                            Some(right) => row.beside(right),
                            None => row,
                        }
                    })
                    .collect();
                // A single row keeps its own baseline; several are set on their middle.
                match <[Self; 1]>::try_from(rows) {
                    Ok([row]) => row,
                    Err(rows) => Self::stacked(rows),
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
        Self::text(" ", Style::default())
            .beside(Self::stacked(rows))
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
        let fenced = |kind: CollectionKind, items: &str| {
            let (open, close) = kind.tex_delimiters();
            format!("{open}{items}{close}")
        };
        let angled = fenced(CollectionKind::AngleBarList, r"1,\,2");
        let sequence = fenced(CollectionKind::Sequence, "3");
        let formula = fenced(CollectionKind::Array, &format!(r"{angled},\,{sequence}"));
        assert_eq!(text(&block.lines), [format!("{formula} = [<|1, 2|>, (3)]")]);
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
            let Some(part) = collection_formula(kind, &[tex("$1$", "1"), tex("$2$", "2")], &groups)
            else {
                panic!("a formula for {kind:?}");
            };
            assert_ne!(
                maths::colored_tex(&part.tex, &groups, MathStyle::default()),
                part.tex,
                "{kind:?}"
            );
        }
    }

    fn word(text: &str, role: Role) -> M2Value {
        M2Value::Token {
            text: text.into(),
            role,
        }
    }

    fn hex(groups: &Groups, group: HLGroup) -> String {
        let Some(Color::Rgb(r, g, b)) = groups.style(group).fg else {
            panic!("{group:?} has an RGB foreground");
        };
        format!("#{r:02x}{g:02x}{b:02x}")
    }

    #[test]
    fn words_in_a_collection_are_typewriter_text_in_the_colour_of_their_role() {
        let groups = Groups::default();
        let items = [
            word("1", Role::String),
            word("Z", Role::Symbol),
            tex(r"$\frac{1}{2}$", "1/2"),
        ];

        let part = collection_formula(CollectionKind::List, &items, &groups).unwrap();

        let string = format!(
            r"\textcolor{{{}}}{{\texttt{{1}}}}",
            hex(&groups, HLGroup::String)
        );
        let symbol = format!(
            r"\textcolor{{{}}}{{\texttt{{Z}}}}",
            hex(&groups, HLGroup::Variable)
        );
        assert!(part.tex.contains(&string), "{}", part.tex);
        assert!(part.tex.contains(&symbol), "{}", part.tex);
        assert!(part.tex.contains(r"\frac{1}{2}"), "{}", part.tex);
        assert_eq!(part.fallback, "{1, Z, 1/2}");
        assert_eq!(part.code, "{\"1\", Z, 1/2}");
    }

    #[test]
    fn a_type_and_nested_lists_stay_in_one_formula() {
        let groups = Groups::default();
        let value = M2Value::Collection {
            kind: CollectionKind::List,
            items: vec![
                M2Value::Collection {
                    kind: CollectionKind::List,
                    items: vec![tex(r"$\frac{x}{3}$", "x/3")],
                },
                M2Value::Collection {
                    kind: CollectionKind::List,
                    items: vec![
                        M2Value::Type(Box::new(tex(r"$\mathbb{Z}$", "ZZ"))),
                        word("null", Role::Null),
                    ],
                },
            ],
        };

        let block = Block::value(&value, 80, &groups, MathStyle::default(), None, spelled);

        let drawn = text(&block.lines);
        assert_eq!(drawn.len(), 1, "{drawn:?}");
        assert!(drawn[0].contains(r"\mathbb{Z}"), "{drawn:?}");
        assert!(drawn[0].contains(r"\texttt{null}"), "{drawn:?}");
        assert!(drawn[0].ends_with("= {{x/3}, {ZZ, null}}"), "{drawn:?}");
    }

    #[test]
    fn the_form_names_what_drew_the_tex_not_what_was_tried() {
        let groups = Groups::default();
        let refused: Picture = |_, _, _| None;
        let draw = |value: &M2Value, picture| {
            Block::value(value, 80, &groups, MathStyle::default(), None, picture).form
        };
        let drawable = tex(r"$\frac{x}{3}$", "x/3");
        let undrawable = tex(r"$\undefinedcommand{x}$", "x");

        assert_eq!(draw(&drawable, spelled), Some(Form::Picture));
        assert_eq!(draw(&drawable, refused), Some(Form::Unicode));
        assert_eq!(draw(&undrawable, refused), Some(Form::Net));
        assert_eq!(draw(&word("null", Role::Null), refused), None);

        let mixed = M2Value::Option {
            key: Box::new(drawable),
            value: Box::new(undrawable),
        };
        assert_eq!(draw(&mixed, refused), Some(Form::Net));
    }

    #[test]
    fn a_collection_with_an_item_that_has_no_tex_form_is_drawn_item_by_item() {
        let groups = Groups::default();
        let value = M2Value::Collection {
            kind: CollectionKind::AngleBarList,
            items: vec![
                tex(r"$\frac{2}{3}$", "2/3"),
                M2Value::Net("two\nrows".into()),
            ],
        };

        assert!(
            collection_formula(CollectionKind::List, std::slice::from_ref(&value), &groups)
                .is_none()
        );
        assert!(matches!(&value, M2Value::Collection { items, .. }
            if collection_formula(CollectionKind::AngleBarList, items, &groups).is_none()));
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
    fn a_net_beside_maths_joins_its_formula_in_typewriter_face() {
        let groups = Groups::default();
        let fraction = tex(r"$\frac{2}{3}$", "2/3");
        for net in [false, true] {
            let mut items = vec![
                fraction.clone(),
                word("2/3", Role::String),
                word("null", Role::Null),
            ];
            if net {
                items.push(M2Value::Net("2/3".into()));
            }
            let value = M2Value::Collection {
                kind: CollectionKind::Sequence,
                items,
            };

            let block = Block::value(&value, 80, &groups, MathStyle::default(), None, spelled);

            let drawn = text(&block.lines);
            assert_eq!(drawn.len(), 1, "{drawn:?}");
            assert!(drawn[0].contains(r"\frac{2}{3}"), "{drawn:?}");
            assert_eq!(
                drawn[0].matches(r"\texttt{2/3}").count(),
                1 + usize::from(net)
            );
            assert!(drawn[0].contains(r"\texttt{null}"), "{drawn:?}");
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
