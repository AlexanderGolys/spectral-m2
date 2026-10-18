//! Archived M2 example transcripts presented through the session's own blocks.

use std::sync::LazyLock;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use regex::Regex;
use unicode_width::UnicodeWidthStr;

use crate::backend::{transcript::strip_m2_location, M2Value};
use crate::code::highlight::Highlighter;
use crate::ui::block::Block;
use crate::ui::cell::{CellState, EvaluationBlock, OutputBlock, TextBlock};
use crate::ui::config::UI;
use crate::ui::render::{RenderContext, RECENT_OUTPUTS};

use super::CODE;

static INPUT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^i+([1-9][0-9]*) : ?").expect("valid example input prompt"));
static OUTPUT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^o+([1-9][0-9]*) ([=:]) ?").expect("valid example output prompt")
});

#[derive(Debug)]
struct Example {
    number: u32,
    source: String,
    printed: String,
    value: Option<String>,
    ty: Option<String>,
    error: Option<String>,
}

impl Example {
    fn parse(lines: &[&str]) -> Option<Self> {
        let input = INPUT.captures(lines.first()?)?;
        let number = input[1].parse().ok()?;
        let prefix = input.get(0)?.end();
        let indent = " ".repeat(prefix);
        let mut source = lines[0][prefix..].to_string();
        let mut start = 1;
        while let Some(line) = lines.get(start).filter(|line| !line.trim().is_empty()) {
            let Some(continuation) = line.strip_prefix(&indent) else {
                break;
            };
            source.push('\n');
            source.push_str(continuation);
            start += 1;
        }
        let output = &lines[start..];
        let label = |line: &str, kind: &str| {
            OUTPUT.captures(line).and_then(|label| {
                (label[1].parse::<u32>().ok()? == number && &label[2] == kind)
                    .then(|| label.get(0).unwrap().end())
            })
        };
        let value = output
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, line)| label(line, "=").map(|prefix| (index, prefix)));
        let ty = output
            .iter()
            .enumerate()
            .rev()
            .filter(|(index, _)| value.is_none_or(|(baseline, _)| *index > baseline))
            .find_map(|(index, line)| label(line, ":").map(|prefix| (index, prefix)));
        // A Net's raised rows precede its baseline label. Keep those rows with
        // the value or type, removing exactly the prompt's alignment columns.
        let start_of = |index: usize, prefix: usize| {
            let indent = " ".repeat(prefix);
            let mut start = index;
            while start > 0
                && !output[start - 1].trim().is_empty()
                && output[start - 1].starts_with(&indent)
            {
                start -= 1;
            }
            start
        };
        let type_start = ty
            .map(|(index, prefix)| start_of(index, prefix))
            .unwrap_or(output.len());
        let value_start = value
            .map(|(index, prefix)| start_of(index, prefix))
            .unwrap_or(type_start);
        let net = |start: usize, end: usize, baseline: usize, prefix: usize| {
            let indent = " ".repeat(prefix);
            output[start..end]
                .iter()
                .enumerate()
                .map(|(offset, line)| {
                    if start + offset == baseline {
                        &line[prefix..]
                    } else {
                        line.strip_prefix(&indent).unwrap_or(line)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
                .trim_end()
                .to_string()
        };
        let value = value.map(|(baseline, prefix)| net(value_start, type_start, baseline, prefix));
        let ty = ty.map(|(baseline, prefix)| net(type_start, output.len(), baseline, prefix));
        let printed = output[..value_start].join("\n");
        let (printed, error) = match printed
            .lines()
            .position(|line| strip_m2_location(line).starts_with("error: "))
        {
            Some(index) => {
                let lines: Vec<_> = printed.lines().collect();
                (lines[..index].join("\n"), Some(lines[index..].join("\n")))
            }
            None => (printed, None),
        };
        Some(Self {
            number,
            source,
            printed: printed.trim_matches('\n').to_string(),
            value,
            ty,
            error,
        })
    }

    fn block(self) -> EvaluationBlock {
        let marked = CODE.with_borrow_mut(|highlighter| highlighter.highlight(&self.source));
        let mut block = EvaluationBlock::new(0, [(self.source.clone(), marked)]);
        let printed = (!self.printed.is_empty()).then(|| TextBlock::printed(self.printed));
        if let Some(error) = self.error {
            block.fail(0, self.number, printed, error);
        } else {
            let state = if self.value.is_some() {
                CellState::Value
            } else if self.source.trim_end().ends_with(';') {
                CellState::Muted
            } else {
                CellState::Null
            };
            let value = self.value.map(|value| {
                OutputBlock::rendered(M2Value::Net(value), self.ty).numbered(self.number)
            });
            block.succeed(0, self.number, state, Vec::new(), printed, value);
        }
        block
    }
}

/// Render a stored example through the same input, printed, error and result
/// blocks as a live submission. Archived examples are never evaluated again.
pub fn render(source: &str, context: &RenderContext) -> Option<Vec<Line<'static>>> {
    let gutter = UI.cell_margin + UI.min_number_digits + UI.cell_gap;
    if context.width() < gutter + UI.min_box_width {
        return None;
    }
    let lines: Vec<_> = source.trim_matches('\n').lines().collect();
    let starts: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| INPUT.is_match(line).then_some(index))
        .collect();
    if starts.first() != Some(&0) {
        return None;
    }
    let mut rendered = Vec::new();
    for (position, &start) in starts.iter().enumerate() {
        let end = starts.get(position + 1).copied().unwrap_or(lines.len());
        let block = Example::parse(&lines[start..end])?.block();
        let context = context
            .full_output()
            .with_recent_outputs([None; RECENT_OUTPUTS]);
        let area = Rect::new(0, 0, context.width(), block.height(&context));
        let mut buffer = Buffer::empty(area);
        block.render_local(&context, &mut buffer);
        rendered.extend(buffer_lines(&buffer));
    }
    Some(rendered)
}

fn buffer_lines(buffer: &Buffer) -> Vec<Line<'static>> {
    let area = buffer.area;
    (area.y..area.bottom())
        .map(|y| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut x = area.x;
            while x < area.right() {
                let cell = &buffer[(x, y)];
                let symbol = cell.symbol();
                let style = cell.style();
                if let Some(last) = spans.last_mut().filter(|span| span.style == style) {
                    last.content.to_mut().push_str(symbol);
                } else {
                    spans.push(Span::styled(symbol.to_string(), style));
                }
                x += u16::try_from(symbol.width().max(1)).unwrap_or(1);
            }
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_printed_text_values_and_types_stay_separate() {
        let example = Example::parse(&[
            "i12 : (print \"hello\"; 2+2)",
            "hello",
            "",
            "o12 = 4",
            "",
            "o12 : ZZ",
        ])
        .unwrap();
        assert_eq!(example.number, 12);
        assert_eq!(example.source, "(print \"hello\"; 2+2)");
        assert_eq!(example.printed, "hello");
        assert_eq!(example.value.as_deref(), Some("4"));
        assert_eq!(example.ty.as_deref(), Some("ZZ"));
    }

    #[test]
    fn multiline_source_and_two_dimensional_nets_keep_their_alignment() {
        let example = Example::parse(&[
            "i1 : matrix {",
            "       {x^2, 1}}",
            "",
            "      2",
            "o1 = x  + 1",
            "",
            "          2       2",
            "o1 : Map R  <--- R",
        ])
        .unwrap();
        assert_eq!(example.source, "matrix {\n  {x^2, 1}}");
        assert_eq!(example.value.as_deref(), Some(" 2\nx  + 1"));
        assert_eq!(example.ty.as_deref(), Some("     2       2\nMap R  <--- R"));
    }

    #[test]
    fn rendered_examples_use_the_session_frames_and_type_placement() {
        let context = RenderContext::new(48, 0);
        let lines = render(
            "i1 : 2+2\n\no1 = 4\n\no1 : ZZ\n\ni2 : print \"hello\"\nhello\n",
            &context,
        )
        .unwrap();
        let text = lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("2+2"), "{text}");
        assert!(text.contains("o1"), "{text}");
        assert!(text.contains("ZZ"), "{text}");
        assert!(text.contains("hello"), "{text}");
        assert!(text.contains('─'), "{text}");
        assert!(!text.contains("i1 :"), "{text}");
        let groups = context.groups();
        assert!(lines
            .iter()
            .flat_map(|line| &line.spans)
            .any(|span| span.style.bg == groups.code().bg && span.content.contains("2")));
    }

    #[test]
    fn ordinary_code_and_tiny_panes_keep_the_text_fallback() {
        assert!(render("R = QQ[x]", &RenderContext::new(40, 0)).is_none());
        assert!(render("i1 : 1\n\no1 = 1", &RenderContext::new(4, 0)).is_none());
    }
}
