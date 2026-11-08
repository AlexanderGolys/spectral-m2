//! Wrapping of semantic lines of spans.
//!
//! Content is held as lines of spans carrying `HLGroup`s, never resolved `Style`s,
//! because a scheme decides how groups map onto styles and a scheme can change. Wrapping
//! therefore works on the spans themselves and leaves their groups untouched.

use super::block::TextSpan;
use super::wrap::WrapPolicy;

/// One logical line of semantic spans, before any width is known.
pub type SpanLine = Vec<TextSpan>;

/// Split one logical line of spans into visual rows no wider than `width`.
///
/// Breaks are chosen by `policy` over the line's combined text, so span boundaries
/// never constrain where a row ends; a span straddling a break is divided between the
/// rows it spans and each part keeps the group of the whole.
pub fn wrap(line: &[TextSpan], width: u16, policy: &dyn WrapPolicy) -> Vec<SpanLine> {
    let mut text = String::new();
    let bounds = line
        .iter()
        .map(|span| {
            let start = text.len();
            text.push_str(&span.content);
            (start..text.len(), span)
        })
        .collect::<Vec<_>>();
    policy
        .wrap(&text, width)
        .into_iter()
        .map(|visual| {
            bounds
                .iter()
                .filter_map(|(range, span)| {
                    let start = range.start.max(visual.range.start);
                    let end = range.end.min(visual.range.end);
                    (start < end).then(|| span.with_content(&text[start..end]))
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::groups::HLGroup;
    use crate::ui::wrap::WordWrap;

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
}
