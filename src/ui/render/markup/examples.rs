//! Archived M2 example transcripts presented through the session's own blocks,
//! with the values of any computed again drawn from their structure instead.

use std::cell::{Cell, RefCell};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::iter;
use std::rc::Rc;
use std::sync::LazyLock;

use m2_syn::native::parse_native;
use ratatui::buffer::Buffer;
use ratatui::layout::{Offset, Position, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use regex::Regex;
use unicode_width::UnicodeWidthStr;

use crate::backend::transcript::strip_m2_location;
use crate::backend::{M2Response, PreviewMode};
use crate::code::highlight::Highlighter;
use crate::code::semantic::{merge, PageColors};
use crate::ui::block::{Block, HLSpan, Link};
use crate::ui::cell::{
    error_blocks, response_blocks, CellState, EvaluationBlock, OutputBlock, TextBlock,
};
use crate::ui::config::UI;
use crate::ui::render::selection::Inclusions;
use crate::ui::render::{RenderContext, RECENT_OUTPUTS};
use crate::ui::theme::groups::HLGroup;

use super::CODE;

static INPUT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^i+([1-9][0-9]*) : ?").expect("valid example input prompt"));
static OUTPUT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^o+([1-9][0-9]*) ([=:]) ?").expect("valid example output prompt")
});

type ExampleFormatters = HashMap<(u64, u32), Rc<Cell<Option<PreviewMode>>>>;

thread_local! {
    /// What examples answered when computed again, by page and input number.
    static COMPUTED: RefCell<HashMap<(u64, u32), Rc<M2Response>>> = RefCell::default();
    /// What a page's analysis found in its code, by page and source.
    static TYPES: RefCell<HashMap<u64, HashSet<String>>> = RefCell::default();
    static MODES: RefCell<ExampleFormatters> = RefCell::default();
    static COLORS: RefCell<HashMap<u64, HashMap<String, Vec<HLSpan>>>> = RefCell::default();
}

pub fn remember_types(page: u64, types: &[String]) -> bool {
    TYPES.with_borrow_mut(|known| {
        let types = types.iter().cloned().collect::<HashSet<_>>();
        if known.get(&page) == Some(&types) {
            return false;
        }
        known.insert(page, types);
        true
    })
}

pub fn remember_colors(colors: PageColors) {
    COLORS.with_borrow_mut(|known| {
        known.insert(colors.page, colors.sources.into_iter().collect());
    });
}

pub fn forget_colors() {
    COLORS.with_borrow_mut(HashMap::clear);
}

/// `source` of `page` in its syntax colours, under what analysis found in it.
pub fn highlighted(page: u64, source: &str) -> Vec<HLSpan> {
    let lexical = CODE.with_borrow_mut(|highlighter| highlighter.highlight(source));
    let marked =
        COLORS.with_borrow(
            |known| match known.get(&page).and_then(|code| code.get(source)) {
                Some(semantic) => merge(lexical.clone(), semantic),
                None => lexical.clone(),
            },
        );
    let types = TYPES.with_borrow(|known| {
        let Some(types) = known.get(&page) else {
            return Vec::new();
        };
        let mut overlays = Vec::new();
        let mut start = None;
        for (offset, character) in source.char_indices().chain(iter::once((source.len(), ' '))) {
            if character.is_alphanumeric() || character == '_' {
                start.get_or_insert(offset);
            } else if let Some(from) = start.take() {
                if types.contains(&source[from..offset])
                    && !lexical.iter().any(|span| {
                        span.content.contains(&from)
                            && matches!(span.kind, HLGroup::String | HLGroup::Comment)
                    })
                {
                    overlays.push(HLSpan::new(from..offset, HLGroup::Type));
                }
            }
        }
        overlays
    });
    merge(marked, &types)
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

/// The M2 code a preformatted block draws: an example transcript's inputs, or
/// the block itself when it is code shown as typed.
pub fn code_sources(source: &str, example_width: usize) -> Vec<String> {
    match transcript(source, example_width) {
        Some(examples) => examples.into_iter().map(|example| example.source).collect(),
        None => {
            let source = source.trim_matches('\n');
            parse_native(source)
                .map(|_| source.to_string())
                .into_iter()
                .collect()
        }
    }
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
    fn input(&self, page: u64) -> EvaluationBlock {
        let marked = highlighted(page, &self.source);
        EvaluationBlock::new(0, [(self.source.clone(), marked)]).with_framed_code()
    }

    /// The example answered as `response` says, the way a live cell is.
    fn computed(&self, page: u64, response: &M2Response) -> EvaluationBlock {
        let mut block = self.input(page);
        if response.is_error {
            let (outputs, printed, message) = error_blocks(response, &self.source);
            block.fail(0, self.number, outputs, printed, message);
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
    fn block(self, page: u64) -> EvaluationBlock {
        let mut block = self.input(page);
        let printed = (!self.printed.is_empty()).then(|| TextBlock::printed(self.printed));
        if let Some(error) = self.error {
            block.fail(0, self.number, Vec::new(), printed, error);
        } else {
            let state = if self.value.is_some() {
                CellState::Value
            } else if self.source.trim_end().ends_with(';') {
                CellState::Muted
            } else {
                CellState::Null
            };
            let value = self
                .value
                .map(|value| OutputBlock::value(value, self.ty).numbered(self.number));
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

/// Blocks painted into lines, and where among those lines their text lies.
pub(super) struct EmbeddedControl {
    pub area: Rect,
    pub block: Rc<RefCell<EvaluationBlock>>,
    pub context: RenderContext,
}

impl EmbeddedControl {
    pub fn cycle(&self, position: Position) -> bool {
        self.area.contains(position)
            && self.block.borrow_mut().cycle_renderer(
                Position::new(position.x - self.area.x, position.y - self.area.y),
                &self.context,
            )
    }
}

pub(super) struct Painted {
    pub controls: Vec<EmbeddedControl>,
    pub lines: Vec<Line<'static>>,
    pub inclusions: Inclusions,
    pub links: Vec<Link>,
}

impl Painted {
    /// Drop the bottom row, a margin no link reaches into.
    fn trim_last_row(&mut self) {
        if self.lines.pop().is_some() {
            let rows = u16::try_from(self.lines.len()).unwrap_or(u16::MAX);
            self.inclusions = self.inclusions.project(0..rows);
        }
    }
}

/// Render a stored example of `page` through the same input, printed, error
/// and result blocks as a live submission. Its code carries no number or separator: an example's input
/// numbers are its own, not the session's, while its outputs keep the `oN` the
/// example itself may refer back to. An example computed again is drawn from
/// what it answered, the rest from what was recorded.
pub fn render(
    source: &str,
    example_width: usize,
    page: u64,
    context: &RenderContext,
) -> Option<Painted> {
    let gutter = UI::CELL_MARGIN + UI::MIN_NUMBER_DIGITS + UI::CELL_GAP;
    if context.width() < gutter + UI::MIN_BOX_WIDTH {
        return None;
    }
    let mut rendered = Painted {
        lines: Vec::new(),
        inclusions: Inclusions::default(),
        links: Vec::new(),
        controls: Vec::new(),
    };
    let examples = transcript(source, example_width)?;
    let count = examples.len();
    for (index, example) in examples.into_iter().enumerate() {
        let number = example.number;
        let mut block = match computed(page, example.number) {
            Some(response) => example.computed(page, &response),
            None => example.block(page),
        };
        let choice =
            MODES.with_borrow_mut(|modes| modes.entry((page, number)).or_default().clone());
        block.share_formatter(choice);
        let mut next = painted(&block, context);
        // An example with nothing to show keeps its code's bottom margin only
        // when it ends the transcript: the next example's code opens with its
        // own margin, and the two together would read as an empty line.
        if index + 1 < count && block.outputs().next().is_none() {
            next.trim_last_row();
        }
        let top = i32::try_from(rendered.lines.len()).unwrap_or(i32::MAX);
        rendered
            .inclusions
            .merge(next.inclusions + Offset::new(0, top));
        rendered.controls.push(EmbeddedControl {
            area: Rect::new(0, top as u16, context.width(), next.lines.len() as u16),
            block: Rc::new(RefCell::new(block)),
            context: context
                .with_inline_layout()
                .with_recent_outputs([None; RECENT_OUTPUTS]),
        });
        rendered.lines.extend(next.lines);
        rendered.links.extend(
            next.links
                .into_iter()
                .map(|link| link.moved(Offset::new(0, top))),
        );
    }
    Some(rendered)
}

/// M2 code shown as typed, such as a usage, drawn as an example's input is
/// with nothing following it; `None` for text that is not M2 code.
pub fn render_code(source: &str, page: u64, context: &RenderContext) -> Option<Painted> {
    let source = source.trim_matches('\n');
    if context.width() < UI::MIN_NUMBER_DIGITS + UI::CELL_GAP + UI::MIN_BOX_WIDTH {
        return None;
    }
    parse_native(source).ok()?;
    let marked = highlighted(page, source);
    let block = EvaluationBlock::shown([(source.to_string(), marked)]).with_framed_code();
    Some(painted(&block, context))
}

/// `block` painted with its code's gutter left empty and its code's surface
/// kept to the columns its results' frame takes.
fn painted(block: &EvaluationBlock, context: &RenderContext) -> Painted {
    let context = context
        .with_inline_layout()
        .with_recent_outputs([None; RECENT_OUTPUTS]);
    let area = Rect::new(0, 0, context.width(), block.height(&context));
    let mut buffer = Buffer::empty(area);
    block.render_local(&context, &mut buffer);
    let gutter = block.code_gutter(&context);
    for position in gutter.positions() {
        buffer[position].set_symbol(" ");
    }
    let frame = block.frame_columns(&context);
    for y in 0..gutter.height {
        for x in (0..area.width).filter(|x| !frame.contains(x)) {
            buffer[(x, y)].reset();
        }
    }
    let mut inclusions = Inclusions::default();
    block.local_selection_inclusions(&context, &mut inclusions);
    let links = block.links(&context);
    Painted {
        controls: Vec::new(),
        lines: buffer_lines(&buffer),
        inclusions,
        links,
    }
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
