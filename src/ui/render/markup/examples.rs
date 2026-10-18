//! Archived M2 example transcripts presented through the session's own blocks,
//! with the values of any computed again drawn from their structure instead.

use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::iter;
use std::rc::Rc;
use std::sync::LazyLock;

use m2_syn::native::parse_native;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use regex::Regex;
use unicode_width::UnicodeWidthStr;

use crate::backend::transcript::{captured_error, strip_m2_location};
use crate::backend::{M2Response, M2Value};
use crate::code::highlight::Highlighter;
use crate::ui::block::Block;
use crate::ui::cell::{response_blocks, CellState, EvaluationBlock, OutputBlock, TextBlock};
use crate::ui::config::UI;
use crate::ui::render::{RenderContext, RECENT_OUTPUTS};

use super::CODE;

static INPUT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^i+([1-9][0-9]*) : ?").expect("valid example input prompt"));
static OUTPUT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^o+([1-9][0-9]*) ([=:]) ?").expect("valid example output prompt")
});

thread_local! {
    /// What examples answered when computed again, by page and input number.
    static COMPUTED: RefCell<HashMap<(u64, u32), Rc<M2Response>>> = RefCell::default();
}

/// Keep what example `number` of `page` answered when computed again, to draw
/// from from now on.
pub fn remember(page: u64, number: u32, response: M2Response) {
    COMPUTED.with_borrow_mut(|computed| computed.insert((page, number), Rc::new(response)));
}

fn computed(page: u64, number: u32) -> Option<Rc<M2Response>> {
    COMPUTED.with_borrow(|computed| computed.get(&(page, number)).cloned())
}

/// An example as its documentation recorded it: what it ran and whether it
/// failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchivedExample {
    pub number: u32,
    pub source: String,
    pub failed: bool,
}

/// The examples one stored transcript records, in order.
pub fn archived(source: &str, example_width: usize) -> Vec<ArchivedExample> {
    transcript(source, example_width)
        .unwrap_or_default()
        .into_iter()
        .map(|example| ArchivedExample {
            number: example.number,
            failed: example.error.is_some(),
            source: example.source,
        })
        .collect()
}

/// Every entry of a stored transcript, which must open with an input.
fn transcript(source: &str, example_width: usize) -> Option<Vec<Example>> {
    let lines: Vec<_> = source.trim_matches('\n').lines().collect();
    let starts: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| INPUT.is_match(line).then_some(index))
        .collect();
    if starts.first() != Some(&0) {
        return None;
    }
    starts
        .iter()
        .enumerate()
        .map(|(position, &start)| {
            let end = starts.get(position + 1).copied().unwrap_or(lines.len());
            Example::parse(&lines[start..end], example_width)
        })
        .collect()
}

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
    /// Read one archived transcript entry, run with M2's print width for
    /// examples, `example_width`.
    fn parse(lines: &[&str], example_width: usize) -> Option<Self> {
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
            let net = output[start..end]
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
                .join("\n");
            // M2 wrapped the value to what the print width left beside its prompt.
            unwrapped(
                net.trim_end(),
                baseline - start,
                example_width.saturating_sub(prefix),
            )
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

    /// The example's input, waiting for what it answers.
    fn input(&self) -> EvaluationBlock {
        let marked = CODE.with_borrow_mut(|highlighter| highlighter.highlight(&self.source));
        EvaluationBlock::new(0, [(self.source.clone(), marked)])
    }

    /// The example answered as `response` says, the way a live cell is.
    fn computed(&self, response: &M2Response) -> EvaluationBlock {
        let mut block = self.input();
        if response.is_error {
            let error = captured_error(&response.transcript, &self.source);
            let printed = (!error.printed.is_empty()).then(|| TextBlock::printed(error.printed));
            block.fail(0, self.number, printed, error.diagnostic);
        } else {
            let (outputs, printed, value) = response_blocks(response, &self.source);
            let value = value.map(|value| value.numbered(self.number));
            block.succeed(
                0,
                self.number,
                CellState::of(response),
                outputs,
                printed,
                value,
            );
        }
        block
    }

    /// The example answered as its documentation recorded.
    fn block(self) -> EvaluationBlock {
        let mut block = self.input();
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

/// A net M2 wrapped to `rule` columns, joined back into one. M2 cuts a net too
/// wide for its print width into column slices, stacked under rules of `rule`
/// dashes. It cuts at a blank column when one lies far enough along, dropping
/// that column, and otherwise between two characters. `baseline` is the first
/// slice's row the prompt labels.
fn unwrapped(net: &str, baseline: usize, rule: usize) -> String {
    let rows: Vec<&str> = net.lines().collect();
    let is_rule = |row: &str| {
        let row = row.trim_end();
        row.chars().count() == rule && row.chars().all(|ch| ch == '-')
    };
    let cuts: Vec<usize> = (baseline + 1..rows.len())
        .filter(|&index| is_rule(rows[index]))
        .collect();
    if cuts.is_empty() {
        return net.to_string();
    }
    let bounds = iter::once(0)
        .chain(cuts.iter().map(|cut| cut + 1))
        .zip(cuts.iter().copied().chain(iter::once(rows.len())));
    let slices: Vec<Slice> = bounds
        .enumerate()
        .map(|(index, (start, end))| {
            Slice::new(&rows[start..end], (index == 0).then_some(baseline))
        })
        .collect();
    let above = slices.iter().map(|slice| slice.baseline).max().unwrap_or(0);
    let below = slices
        .iter()
        .map(|slice| slice.rows.len().saturating_sub(slice.baseline + 1))
        .max()
        .unwrap_or(0);
    (0..=above + below)
        .map(|row| {
            let mut line = String::new();
            for (index, slice) in slices.iter().enumerate() {
                if index > 0 && slices[index - 1].cut_at_blank(rule) {
                    line.push(' ');
                }
                let text = row
                    .checked_sub(above - slice.baseline)
                    .and_then(|row| slice.rows.get(row))
                    .map_or("", String::as_str);
                line.push_str(&format!("{text:<width$}", width = slice.width));
            }
            line.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// One column slice of a wrapped net.
struct Slice {
    rows: Vec<String>,
    /// The row level with the other slices' baselines.
    baseline: usize,
    width: usize,
}

impl Slice {
    /// A slice whose baseline is known, or else taken to be its fullest row:
    /// the rows around it hold only sparse exponents and subscripts.
    fn new(rows: &[&str], baseline: Option<usize>) -> Self {
        let rows: Vec<String> = rows.iter().map(|row| row.trim_end().to_string()).collect();
        let baseline = baseline.unwrap_or_else(|| {
            rows.iter()
                .enumerate()
                .max_by_key(|(index, row)| {
                    let filled = row.chars().filter(|ch| *ch != ' ').count();
                    (filled, Reverse(*index))
                })
                .map_or(0, |(index, _)| index)
        });
        let width = rows
            .iter()
            .map(|row| row.chars().count())
            .max()
            .unwrap_or(0);
        Self {
            rows,
            baseline,
            width,
        }
    }

    /// Whether M2 cut after this slice at a blank column, which it prefers
    /// whenever one lies past the first third of the width.
    fn cut_at_blank(&self, rule: usize) -> bool {
        let earliest = (rule / 3).max(1) - 1;
        (earliest..self.width).any(|column| {
            self.rows
                .iter()
                .all(|row| row.chars().nth(column).is_none_or(|ch| ch == ' '))
        })
    }
}

/// Render a stored example of `page` through the same input, printed, error
/// and result blocks as a live submission, no wider than the transcript M2
/// recorded it in. Its code carries no number or separator: an example's input
/// numbers are its own, not the session's, while its outputs keep the `oN` the
/// example itself may refer back to. An example computed again is drawn from
/// what it answered, the rest from what was recorded.
pub fn render(
    source: &str,
    example_width: usize,
    page: u64,
    context: &RenderContext,
) -> Option<Vec<Line<'static>>> {
    let gutter = UI.cell_margin + UI.min_number_digits + UI.cell_gap;
    if context.width() < gutter + UI.min_box_width {
        return None;
    }
    let mut rendered = Vec::new();
    for example in transcript(source, example_width)? {
        let block = match computed(page, example.number) {
            Some(response) => example.computed(&response),
            None => example.block(),
        };
        rendered.extend(painted(&block, example_width, context));
    }
    Some(rendered)
}

/// M2 code shown as typed, such as a usage, drawn as an example's input is
/// with nothing following it; `None` for text that is not M2 code.
pub fn render_code(
    source: &str,
    example_width: usize,
    context: &RenderContext,
) -> Option<Vec<Line<'static>>> {
    let source = source.trim_matches('\n');
    if context.width() < UI.min_number_digits + UI.cell_gap + UI.min_box_width {
        return None;
    }
    parse_native(source).ok()?;
    let marked = CODE.with_borrow_mut(|highlighter| highlighter.highlight(source));
    let block = EvaluationBlock::shown([(source.to_string(), marked)]);
    Some(painted(&block, example_width, context))
}

/// `block` painted no wider than an example transcript, its code's gutter left
/// empty.
fn painted(
    block: &EvaluationBlock,
    example_width: usize,
    context: &RenderContext,
) -> Vec<Line<'static>> {
    const INSET: u16 = 2;
    let gutter = block.code_gutter(context);
    let width = u16::try_from(example_width)
        .unwrap_or(u16::MAX)
        .saturating_add(gutter.width)
        .min(context.width())
        .saturating_sub(INSET * 2);
    let context = context
        .with_width(width)
        .full_output()
        .with_recent_outputs([None; RECENT_OUTPUTS]);
    let area = Rect::new(0, 0, context.width(), block.height(&context));
    let mut buffer = Buffer::empty(area);
    block.render_local(&context, &mut buffer);
    for position in block.code_gutter(&context).positions() {
        buffer[position].set_symbol(" ");
    }
    buffer_lines(&buffer)
        .into_iter()
        .map(|mut line| {
            line.spans.insert(0, Span::raw(" ".repeat(INSET.into())));
            line
        })
        .collect()
}

/// The rows of `buffer` as lines. A colour no block painted is left unset, so
/// the surface the lines are placed on shows through.
fn buffer_lines(buffer: &Buffer) -> Vec<Line<'static>> {
    let unpainted = |color: Option<Color>| color.filter(|color| *color != Color::Reset);
    let area = buffer.area;
    (area.y..area.bottom())
        .map(|y| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut x = area.x;
            while x < area.right() {
                let cell = &buffer[(x, y)];
                let symbol = cell.symbol();
                let style = cell.style();
                let style = Style {
                    fg: unpainted(style.fg),
                    bg: unpainted(style.bg),
                    ..style
                };
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

    /// The print width M2 runs documentation examples at.
    const EXAMPLE_WIDTH: usize = 77;
    /// A page none of whose examples were computed again.
    const UNCOMPUTED: u64 = 0;

    /// The value M2 printed for `lines`, run at a print width of `width`.
    fn value(lines: &[&str], width: usize) -> String {
        Example::parse(lines, width).unwrap().value.unwrap()
    }

    #[test]
    fn an_example_computed_again_is_drawn_from_what_it_answered() {
        let page = 7;
        let source = "i1 : 2+2\n\no1 = 4\n\no1 : ZZ";
        let context = RenderContext::new(48, 0);
        let drawn = |page| {
            render(source, EXAMPLE_WIDTH, page, &context)
                .unwrap()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        };
        remember(
            page,
            1,
            M2Response {
                number: 1,
                is_error: false,
                transcript: String::new(),
                value: Some(M2Value::Net("four".into())),
                type_info: Some("ZZ".into()),
                type_parts: Vec::new(),
                has_value: true,
                depth: 1,
                names: Vec::new(),
            },
        );

        let computed = drawn(page);
        assert!(computed.contains("four"), "{computed}");
        assert!(computed.contains("o1"), "{computed}");
        let recorded = drawn(UNCOMPUTED);
        assert!(
            !recorded.contains("four"),
            "other pages keep what was recorded"
        );
    }

    #[test]
    fn values_m2_wrapped_are_joined_back_into_one() {
        let rule = "-".repeat(25);
        assert_eq!(
            value(
                &[
                    "i2 : toList(1..20)",
                    "",
                    "o2 = {1, 2, 3, 4, 5, 6, 7, 8,",
                    &format!("     {rule}"),
                    "     9, 10, 11, 12, 13, 14,",
                    &format!("     {rule}"),
                    "     15, 16, 17, 18, 19, 20}",
                ],
                30,
            ),
            format!(
                "{{{}}}",
                (1..=20)
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        );
        assert_eq!(
            value(
                &[
                    "i4 : gens R",
                    "",
                    "o4 = {x , x , x ,",
                    "       1   2   3",
                    &format!("     {rule}"),
                    "     x , x  }",
                    "      4   12",
                ],
                30,
            ),
            "{x , x , x , x , x  }\n  1   2   3   4   12"
        );
        assert_eq!(
            value(
                &[
                    "i5 : matrix{{x_1^2+x_2^3+x_3^4+x_4^5+x_5^6+x_6^7}}",
                    "",
                    "o5 = | x_6^7+x_5^6+x_4^5+x_3^4",
                    &format!("     {rule}"),
                    "     +x_2^3+x_1^2 |",
                ],
                30,
            ),
            "| x_6^7+x_5^6+x_4^5+x_3^4+x_2^3+x_1^2 |",
            "a cut with no blank column to drop joins without a space"
        );
    }

    #[test]
    fn a_fraction_bar_is_not_mistaken_for_a_wrap() {
        assert_eq!(
            value(
                &["i1 : 1/2", "", "     1", "o1 = -", "     2"],
                EXAMPLE_WIDTH
            ),
            "1\n-\n2"
        );
    }

    #[test]
    fn example_outputs_take_the_surface_they_are_placed_on() {
        let context = RenderContext::new(48, 0);
        let lines = render(
            "i1 : 2+2\n\no1 = 4\n\no1 : ZZ",
            EXAMPLE_WIDTH,
            UNCOMPUTED,
            &context,
        )
        .unwrap();
        let output = lines
            .iter()
            .find(|line| line.to_string().contains('4') && !line.to_string().contains('+'))
            .expect("the value is drawn");
        assert!(
            output
                .spans
                .iter()
                .all(|span| span.style.bg != Some(Color::Reset)),
            "{output:?}"
        );
    }

    #[test]
    fn inputs_printed_text_values_and_types_stay_separate() {
        let example = Example::parse(
            &[
                "i12 : (print \"hello\"; 2+2)",
                "hello",
                "",
                "o12 = 4",
                "",
                "o12 : ZZ",
            ],
            EXAMPLE_WIDTH,
        )
        .unwrap();
        assert_eq!(example.number, 12);
        assert_eq!(example.source, "(print \"hello\"; 2+2)");
        assert_eq!(example.printed, "hello");
        assert_eq!(example.value.as_deref(), Some("4"));
        assert_eq!(example.ty.as_deref(), Some("ZZ"));
    }

    #[test]
    fn multiline_source_and_two_dimensional_nets_keep_their_alignment() {
        let example = Example::parse(
            &[
                "i1 : matrix {",
                "       {x^2, 1}}",
                "",
                "      2",
                "o1 = x  + 1",
                "",
                "          2       2",
                "o1 : Map R  <--- R",
            ],
            EXAMPLE_WIDTH,
        )
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
            EXAMPLE_WIDTH,
            UNCOMPUTED,
            &context,
        )
        .unwrap();
        let text = lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("o1"), "outputs keep their labels: {text}");
        assert!(
            text.lines()
                .filter(|row| row.contains("2+2"))
                .all(|row| row.trim_start().starts_with("2+2")),
            "code carries no number or separator: {text}"
        );
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
    fn code_examples_share_the_transcript_width_and_wrap_in_narrow_panes() {
        let wide = render_code("f + g", EXAMPLE_WIDTH, &RenderContext::new(100, 0)).unwrap();
        let other = render_code("a * f", EXAMPLE_WIDTH, &RenderContext::new(100, 0)).unwrap();
        assert!(wide.iter().all(|line| line.width() == other[0].width()));
        assert!(wide[0].width() > 30);

        let narrow = render_code(
            "result = veryLongIdentifier + anotherLongIdentifier",
            EXAMPLE_WIDTH,
            &RenderContext::new(24, 0),
        )
        .unwrap();
        assert!(narrow.iter().all(|line| line.width() <= 24));
        let text = narrow.iter().map(ToString::to_string).collect::<String>();
        let joined: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
        assert!(joined.contains("veryLongIdentifier"), "{text}");
        assert!(joined.contains("anotherLongIdentifier"), "{text}");
        for line in &narrow {
            assert_eq!(line.spans[0].content.as_ref(), "  ");
            assert_eq!(line.spans[0].style.bg, None);
            assert_eq!(line.width(), 22);
        }
    }

    #[test]
    fn ordinary_code_and_tiny_panes_keep_the_text_fallback() {
        assert!(render(
            "R = QQ[x]",
            EXAMPLE_WIDTH,
            UNCOMPUTED,
            &RenderContext::new(40, 0)
        )
        .is_none());
        assert!(render(
            "i1 : 1\n\no1 = 1",
            EXAMPLE_WIDTH,
            UNCOMPUTED,
            &RenderContext::new(4, 0)
        )
        .is_none());
    }
}
