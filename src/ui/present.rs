//! Semantic content retained beside the cells it last resolved into.
//!
//! Content is held as lines of spans carrying [`HLGroup`]s, never resolved `Style`s,
//! because a scheme decides how groups map onto styles and a scheme can change. A
//! resolution runs three stages the content itself outlives: wrapping the spans to a
//! width, resolving their groups through a scheme, and painting the result into cells.
//!
//! Only the ends of that pipeline are kept. Between resolutions the cells are what
//! every parent pastes, so nothing here runs again until a width or a scheme moves.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};

use super::block::{Text, TextSpan};
use super::controller::settings::theme::groups::Groups;
use super::wrap::{WordWrap, WrapPolicy};

/// One logical line of semantic spans, before any width is known.
pub(crate) type SpanLine = Vec<TextSpan>;

/// Split one logical line of spans into visual rows no wider than `width`.
///
/// Breaks are chosen by `policy` over the line's combined text, so span boundaries
/// never constrain where a row ends; a span straddling a break is divided between the
/// rows it spans and each part keeps the group of the whole.
pub(crate) fn wrap(line: &[TextSpan], width: u16, policy: &dyn WrapPolicy) -> Vec<SpanLine> {
    let mut text = String::new();
    let bounds = line
        .iter()
        .map(|span| {
            let start = text.len();
            text.push_str(&span.content);
            (start..text.len(), span.kind)
        })
        .collect::<Vec<_>>();
    policy
        .wrap(&text, width)
        .into_iter()
        .map(|visual| {
            bounds
                .iter()
                .filter_map(|(span, kind)| {
                    let start = span.start.max(visual.range.start);
                    let end = span.end.min(visual.range.end);
                    (start < end).then(|| TextSpan::new(&text[start..end], *kind))
                })
                .collect()
        })
        .collect()
}

/// Semantic lines an element re-resolves on demand, beside the cells they last painted.
///
/// The wrapping policy is fixed per presentation rather than per resolution, because
/// it belongs to the kind of content held: prose breaks at words, code at the token
/// boundaries its own spans already mark.
pub(crate) struct Presentation {
    lines: Vec<SpanLine>,
    policy: Box<dyn WrapPolicy + Send>,
    cells: Buffer,
}

impl Presentation {
    pub(crate) fn new(lines: Vec<SpanLine>, policy: impl WrapPolicy + Send + 'static) -> Self {
        Self {
            lines,
            policy: Box::new(policy),
            cells: Buffer::empty(Rect::ZERO),
        }
    }

    /// Hold prose, broken at word boundaries.
    pub(crate) fn prose(lines: Vec<SpanLine>) -> Self {
        Self::new(lines, WordWrap)
    }

    /// Rebuild the cells by wrapping to `width` and resolving groups through `groups`.
    pub(crate) fn resolve(&mut self, width: u16, groups: &Groups, base: Style) {
        let rows = self
            .lines
            .iter()
            .flat_map(|line| wrap(line, width, self.policy.as_ref()))
            .map(|row| Line::from(row.spans(groups)))
            .collect::<Vec<_>>();
        let height = u16::try_from(rows.len()).unwrap_or(u16::MAX);
        let area = Rect::new(0, 0, width, height);
        self.cells = Buffer::empty(area);
        Paragraph::new(rows)
            .style(base)
            .render(area, &mut self.cells);
    }

    pub(crate) fn cells(&self) -> &Buffer {
        &self.cells
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::controller::settings::theme::groups::HLGroup;

    fn line() -> SpanLine {
        vec![
            TextSpan::new("hello ", HLGroup::Keyword),
            TextSpan::new("world", HLGroup::Variable),
        ]
    }

    #[test]
    fn wrapping_divides_a_straddling_span_and_keeps_its_group() {
        let rows = wrap(&line(), 8, &WordWrap);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0].content, "hello");
        assert_eq!(rows[0][0].kind, HLGroup::Keyword);
        assert_eq!(rows[1][0].content, "world");
        assert_eq!(rows[1][0].kind, HLGroup::Variable);
    }

    #[test]
    fn a_row_narrower_than_a_span_splits_it_without_losing_its_group() {
        let rows = wrap(
            &[TextSpan::new("abcdefgh", HLGroup::Variable)],
            4,
            &WordWrap,
        );

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0].content, "abcd");
        assert_eq!(rows[1][0].content, "efgh");
        assert!(rows.iter().all(|row| row[0].kind == HLGroup::Variable));
    }

    #[test]
    fn resolving_paints_each_group_through_the_scheme() {
        let groups = Groups::default();
        let mut presentation = Presentation::prose(vec![line()]);

        presentation.resolve(20, &groups, Style::new());

        let cells = presentation.cells();
        assert_eq!(cells.area().width, 20);
        assert_eq!(cells[(0, 0)].symbol(), "h");
        assert_eq!(cells[(0, 0)].fg, groups.style(HLGroup::Keyword).fg.unwrap());
        assert_eq!(
            cells[(6, 0)].fg,
            groups.style(HLGroup::Variable).fg.unwrap()
        );
    }

    #[test]
    fn rescheming_repaints_the_same_wrapped_content() {
        let mut presentation = Presentation::prose(vec![line()]);
        presentation.resolve(8, &Groups::default(), Style::new());
        let wrapped = presentation.cells().area().height;

        presentation.resolve(8, &Groups::default(), Style::new());

        assert_eq!(presentation.cells().area().height, wrapped);
    }
}
