//! Typeset mathematical leaves composed with terminal text around a shared baseline.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::backend::{CollectionKind, M2Value};
use crate::ui::block::Text;
use crate::ui::controller::settings::theme::groups::{Groups, HLGroup};
use crate::ui::graphics::{self, typeset::Rgb};
use crate::ui::wrap::WrappedText;

use super::maths::{self, MathBlock, MathStyle};

/// The image backend, returning terminal-cell rows for one formula.
type Picture = fn(&str, &str, Rgb, u16) -> Option<Vec<Line<'static>>>;

/// Render each value in its own representation, without converting text into TeX.
pub fn lines(
    value: &M2Value,
    width: usize,
    groups: &Groups,
    style: MathStyle,
) -> Vec<Line<'static>> {
    Block::value(value, width.max(1), groups, style, None, graphics::typeset).lines
}

/// Type words remain terminal text while mathematical parts keep their typesetting.
pub fn annotation(
    parts: &[M2Value],
    width: usize,
    groups: &Groups,
    style: MathStyle,
) -> Vec<Line<'static>> {
    let ink = groups.style(HLGroup::Type);
    let blocks = parts.iter().enumerate().map(|(index, part)| {
        let block = Block::value(
            part,
            width.max(1),
            groups,
            style,
            Some(ink),
            graphics::typeset,
        );
        if index + 1 < parts.len() {
            block.beside(Block::text(" ", ink))
        } else {
            block
        }
    });
    Block::pack(blocks, width.max(1))
        .into_iter()
        .flat_map(|block| block.lines)
        .collect()
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
        let lines = MathBlock::fence(text, self.lines.len(), self.baseline)
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
        Self {
            lines,
            baseline: self.baseline,
        }
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
            M2Value::Tex { source, fallback } => {
                if let Some(tex) = source
                    .strip_prefix('$')
                    .and_then(|tex| tex.strip_suffix('$'))
                {
                    let fg = text_ink(HLGroup::Value).fg;
                    if let Some(Color::Rgb(r, g, b)) = fg {
                        let painted = match ink {
                            Some(_) => tex.to_string(),
                            None => maths::colored_tex(tex, groups),
                        };
                        if let Some(lines) = picture(
                            tex,
                            &painted,
                            Rgb([r, g, b]),
                            u16::try_from(width).unwrap_or(u16::MAX),
                        ) {
                            return Self {
                                baseline: lines.len().saturating_sub(1) / 2,
                                lines,
                            };
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
                    lines.extend(row.lines);
                }
                Self {
                    baseline: lines.len().saturating_sub(1) / 2,
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
                        .beside(Self::text(" ⇒ ", text_ink(HLGroup::Operator)))
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

    fn picture(source: &str, _: &str, _: Rgb, _: u16) -> Option<Vec<Line<'static>>> {
        assert_eq!(source, r"\frac{2}{3}");
        Some(vec![Line::raw("2"), Line::raw("─"), Line::raw("3")])
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
    fn a_net_does_not_change_its_mathematical_siblings() {
        let groups = Groups::default();
        let fraction = M2Value::Tex {
            source: r"$\frac{2}{3}$".into(),
            fallback: "2/3".into(),
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
            assert!(block.lines[1].spans.iter().any(
                |span| span.content == "null" && span.style == groups.style(HLGroup::Constant)
            ));
        }
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
            |_, _, _, _| panic!("ordinary text must not be typeset"),
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
