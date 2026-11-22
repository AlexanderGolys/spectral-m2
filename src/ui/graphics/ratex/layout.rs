//! Laying out what a picture is set from in the room it has: a row of ink
//! broken into lines, rows stacked each on its own lines, and fences grown to
//! what they hold once it is laid out. Everything is in ems.

use std::borrow::Cow;
use std::iter::once;
use std::ops::Add;

use ratex_font::{get_char_metrics_with_fallback, FontId};
use ratex_types::display_item::{DisplayItem, DisplayList};
use ratex_types::path_command::PathCommand;

use super::diagram::{self, place_list, AXIS};
use super::{
    delimiter, pictured, set, Picture, Rgb, Source, Unset, MARK_RANK_POINTS, POINTS_PER_EM,
    TOLERANCE,
};

/// Ems between lines set one under another, of one row of ink and of rows
/// alike: amsmath's `\jot`, the space it opens between the lines of a display.
const LINE_GAP: f64 = 0.3;
/// Ems a line continuing a part stands in from where the part began: a quad.
const CONTINUATION_INDENT: f64 = 1.0;
/// TeX's `\arraycolsep`, the space an array keeps on each side of a column:
/// twice this between two columns, once between a fence and what it holds.
const COLUMN_SEP: f64 = 0.5;
/// The least a row reaches above and below its baseline: the strut of a TeX
/// array's row, 0.7 and 0.3 of its 12pt `\baselineskip`.
const STRUT: (f64, f64) = (0.84, 0.36);

/// What ink laid out in a room takes up: how wide, how far above and below its
/// first line's baseline, and where its last line ends, across and below that
/// baseline.
#[derive(Clone, Copy, Debug)]
struct Extent {
    width: f64,
    height: f64,
    depth: f64,
    end: (f64, f64),
}

impl Extent {
    /// What a list set on one line takes up.
    fn of(list: &DisplayList) -> Self {
        Self {
            width: list.width,
            height: list.height,
            depth: list.depth,
            end: (list.width, 0.0),
        }
    }

    /// How badly this stands in `room`, the worse the greater: whether it
    /// passes the room, then how tall it is.
    fn cost(&self, room: f64) -> (bool, f64) {
        (self.width > room + TOLERANCE, self.height + self.depth)
    }
}

/// A source typeset, ready to be laid out in any room.
#[derive(Clone, Debug)]
pub struct Set {
    part: Part,
    /// What it takes up with all the room it could want.
    natural: Extent,
}

#[derive(Clone, Debug)]
enum Part {
    Flow(Flow),
    Stack(Stack),
    Fenced { fences: Fences, body: Box<Set> },
}

impl Set {
    /// `source` set in `colour`.
    pub fn new(source: &Source, colour: Rgb) -> Result<Self, Unset> {
        let part = match source {
            Source::Formula { tex, painted } => {
                Part::Flow(Flow::new(match diagram::composed(tex, painted, colour) {
                    Some(diagram) => diagram?,
                    None => set(tex, painted, colour)?,
                }))
            }
            Source::Rows(rows) => Part::Stack(Stack(
                rows.iter()
                    .map(|row| {
                        row.iter()
                            .map(|cell| Self::new(cell, colour))
                            .collect::<Result<_, _>>()
                            .map(Row)
                    })
                    .collect::<Result<_, _>>()?,
            )),
            Source::Fenced {
                left,
                right,
                ink,
                body,
            } => Part::Fenced {
                fences: Fences {
                    left: *left,
                    right: *right,
                    ink: *ink,
                    colour,
                },
                body: Box::new(Self::new(body, colour)?),
            },
        };
        let natural = part.extent(f64::INFINITY)?;
        Ok(Self { part, natural })
    }

    /// One row of ink, already set.
    fn flow(list: DisplayList) -> Self {
        let natural = Extent::of(&list);
        Self {
            part: Part::Flow(Flow::new(list)),
            natural,
        }
    }

    /// Ems across with all the room it could want.
    pub fn width(&self) -> f64 {
        self.natural.width
    }

    /// The picture of this in a room of `room` ems. What no break brings
    /// inside the room is laid out for the room it has once shrunk, to `least`
    /// of its size at most, so shrinking is the last resort and the least that
    /// will do.
    pub fn picture(&self, room: f64, least: f64) -> Result<Picture, Unset> {
        let widest = self.extent(room)?.width;
        let room = match widest > room + TOLERANCE {
            true => widest.min(room / least),
            false => room,
        };
        Ok(pictured(self.laid(room)?))
    }

    fn extent(&self, room: f64) -> Result<Extent, Unset> {
        match self.natural.width <= room + TOLERANCE {
            true => Ok(self.natural),
            false => self.part.extent(room),
        }
    }

    fn laid(&self, room: f64) -> Result<DisplayList, Unset> {
        match &self.part {
            Part::Flow(flow) => Ok(flow.laid(room)),
            Part::Stack(stack) => stack.arranged(room)?.drawn(),
            Part::Fenced { fences, body } => fences.around(body, room)?.drawn(),
        }
    }
}

impl Part {
    fn extent(&self, room: f64) -> Result<Extent, Unset> {
        Ok(match self {
            Self::Flow(flow) => flow.extent(&flow.lines(room)),
            Self::Stack(stack) => stack.arranged(room)?.extent,
            Self::Fenced { fences, body } => fences.around(body, room)?.extent,
        })
    }
}

/// A place a row of ink may break: where the line before it ends, where the
/// line after it starts, and how deep in the row's structure it lies.
#[derive(Clone, Copy, Debug)]
struct Cut {
    end: f64,
    start: f64,
    rank: usize,
}

/// What a way of breaking a row into lines costs, the worse first: the ems its
/// lines pass the room by; how deep its breaks lie, added up, so a row breaks
/// inside a nested part only when nothing shallower fits; and how empty its
/// lines are, as the sum of their squared slack, so they come out even, none
/// left short by a break early on or near the end.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
struct Cost {
    overflow: f64,
    depth: usize,
    slack: f64,
}

impl Add for Cost {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            overflow: self.overflow + other.overflow,
            depth: self.depth + other.depth,
            slack: self.slack + other.slack,
        }
    }
}

/// One row of ink and the places it may break, left to right.
#[derive(Clone, Debug)]
struct Flow {
    list: DisplayList,
    cuts: Vec<Cut>,
}

impl Flow {
    /// `list` without its break marks, cut where they stood: at each mark
    /// with ink before it that nothing drawn reaches across, as a fraction
    /// bar, a radical or a wider entry of the same matrix column would.
    fn new(mut list: DisplayList) -> Self {
        let marks = list.items.iter().filter_map(mark).collect::<Vec<_>>();
        list.items.retain(|item| mark(item).is_none());
        let reaches = list.items.iter().map(reach).collect::<Vec<_>>();
        let leftmost = reaches
            .iter()
            .map(|(left, _)| *left)
            .fold(f64::INFINITY, f64::min);
        let mut cuts = marks
            .into_iter()
            .filter(|(end, _)| *end > leftmost + TOLERANCE)
            .filter_map(|(end, rank)| {
                let start = reaches
                    .iter()
                    .map(|(left, _)| *left)
                    .filter(|left| *left >= end - TOLERANCE)
                    .fold(f64::INFINITY, f64::min);
                let bridged = reaches
                    .iter()
                    .any(|(left, right)| *left < end - TOLERANCE && *right > start + TOLERANCE);
                (start.is_finite() && !bridged).then_some(Cut { end, start, rank })
            })
            .collect::<Vec<_>>();
        cuts.sort_by(|a, b| a.end.total_cmp(&b.end).then(a.rank.cmp(&b.rank)));
        cuts.dedup_by(|later, earlier| later.start - earlier.start < TOLERANCE);
        Self { list, cuts }
    }

    /// Where each line of this starts and ends across the row when it is
    /// broken for `room`, at the breaks costing least.
    fn lines(&self, room: f64) -> Vec<(f64, f64)> {
        if self.list.width <= room + TOLERANCE {
            return vec![(0.0, self.list.width)];
        }
        // A line runs between two of the row's places: its ends and its cuts.
        let starts = once(0.0)
            .chain(self.cuts.iter().map(|cut| cut.start))
            .collect::<Vec<_>>();
        let ends = self
            .cuts
            .iter()
            .map(|cut| (cut.end, cut.rank))
            .chain(once((self.list.width, 0)))
            .collect::<Vec<_>>();
        // The least a row broken at each place costs up to it, and the place
        // its last line starts at.
        let mut least = vec![(
            Cost {
                overflow: 0.0,
                depth: 0,
                slack: 0.0,
            },
            0,
        )];
        for (place, (end, rank)) in ends.iter().enumerate() {
            let mut best: Option<(Cost, usize)> = None;
            let mut from = place;
            loop {
                // A line from the row's first place is its first line.
                let slack = line_room(room, from) - (end - starts[from]);
                // What passes the room alone stands on a line of its own.
                let passes = slack < -TOLERANCE && from < place;
                let cost = least[from].0
                    + Cost {
                        overflow: if slack < -TOLERANCE { -slack } else { 0.0 },
                        depth: *rank,
                        slack: slack.max(0.0).powi(2),
                    };
                if !passes && best.is_none_or(|(cheapest, _)| cost < cheapest) {
                    best = Some((cost, from));
                }
                // Starting earlier only widens a line, so past one too wide
                // only the first line, standing in by nothing, may still fit.
                from = match (from, passes) {
                    (0, _) => break,
                    (_, true) => 0,
                    (_, false) => from - 1,
                };
            }
            least.push(best.expect("a line from the place before always stands"));
        }
        let mut lines = Vec::new();
        let mut place = ends.len();
        while place > 0 {
            let from = least[place].1;
            lines.push((starts[from], ends[place - 1].0));
            place = from;
        }
        lines.reverse();
        lines
    }

    /// The row broken into `lines`, one under another.
    fn extent(&self, lines: &[(f64, f64)]) -> Extent {
        let across = |line: usize| line_indent(line) + lines[line].1 - lines[line].0;
        let last = lines.len() - 1;
        Extent {
            width: (0..lines.len()).map(across).fold(0.0, f64::max),
            height: self.list.height,
            depth: self.list.depth + last as f64 * self.line_advance(),
            end: (across(last), last as f64 * self.line_advance()),
        }
    }

    /// Ems from one line's baseline to the next: every line is as tall as the
    /// row.
    fn line_advance(&self) -> f64 {
        self.list.height + self.list.depth + LINE_GAP
    }

    fn laid(&self, room: f64) -> DisplayList {
        let lines = self.lines(room);
        let Extent {
            width,
            height,
            depth,
            ..
        } = self.extent(&lines);
        let mut laid = DisplayList {
            items: self.list.items.clone(),
            width,
            height,
            depth,
        };
        for item in &mut laid.items {
            let left = reach(item).0;
            let line = lines
                .partition_point(|(start, _)| *start <= left + TOLERANCE)
                .saturating_sub(1);
            shift(
                item,
                line_indent(line) - lines[line].0,
                line as f64 * self.line_advance(),
            );
        }
        laid
    }
}

/// Ems a row's line stands in from where the row began: none for its first.
fn line_indent(line: usize) -> f64 {
    match line {
        0 => 0.0,
        _ => CONTINUATION_INDENT,
    }
}

/// Ems a row's line has of `room`.
fn line_room(room: f64, line: usize) -> f64 {
    room - line_indent(line)
}

/// A part placed in an arrangement, to be laid out in a room of its own:
/// `left` across, its first baseline `drop` below the arrangement's.
#[derive(Clone, Debug)]
struct Placed<'a> {
    set: Cow<'a, Set>,
    room: f64,
    left: f64,
    drop: f64,
}

/// Parts placed beside and under one another, and what they take up together.
#[derive(Clone, Debug)]
struct Arrangement<'a> {
    placed: Vec<Placed<'a>>,
    extent: Extent,
}

impl Arrangement<'_> {
    fn drawn(self) -> Result<DisplayList, Unset> {
        let Extent {
            width,
            height,
            depth,
            ..
        } = self.extent;
        let mut list = DisplayList {
            items: Vec::new(),
            width,
            height,
            depth,
        };
        for part in self.placed {
            place_list(
                &mut list,
                part.set.laid(part.room)?,
                part.left,
                height + part.drop,
            );
        }
        Ok(list)
    }
}

/// A row's cells: all but the last, its lead, each in a column; the last, its
/// tail, running on in what room the row has left.
#[derive(Clone, Debug)]
struct Row(Vec<Set>);

impl Row {
    fn lead(&self) -> &[Set] {
        self.0.split_last().map_or(&[], |(_, lead)| lead)
    }

    /// Ems across the lead's cells, the measure rows leave the columns by.
    fn lead_width(&self) -> f64 {
        self.lead().iter().map(Set::width).sum()
    }

    /// The row with each lead cell in its column of `columns` and the tail
    /// beside them, on lines of its own in the room they leave.
    fn beside(&self, columns: &[f64], room: f64) -> Result<Arrangement<'_>, Unset> {
        let mut placed = Vec::new();
        let (mut left, mut height, mut depth) = (0.0, 0.0_f64, 0.0_f64);
        for (cell, column) in self.lead().iter().zip(columns) {
            placed.push(Placed {
                set: Cow::Borrowed(cell),
                room: f64::INFINITY,
                left,
                drop: 0.0,
            });
            height = height.max(cell.natural.height);
            depth = depth.max(cell.natural.depth);
            left += column + 2.0 * COLUMN_SEP;
        }
        let mut extent = Extent {
            width: left,
            height,
            depth,
            end: (left, 0.0),
        };
        if let Some(tail) = self.0.last() {
            let beside = tail.extent(room - left)?;
            placed.push(Placed {
                set: Cow::Borrowed(tail),
                room: room - left,
                left,
                drop: 0.0,
            });
            extent = Extent {
                width: left + beside.width,
                height: height.max(beside.height),
                depth: depth.max(beside.depth),
                end: (left + beside.end.0, beside.end.1),
            };
        }
        Ok(Arrangement { placed, extent })
    }

    /// The row out of the columns: its first cell on lines of its own across
    /// the room, the rest of the lead following its last line, and the tail
    /// beneath them, indented as what continues the row.
    fn beneath(&self, room: f64) -> Result<Arrangement<'_>, Unset> {
        let (Some((first, rest)), Some(tail)) = (self.lead().split_first(), self.0.last()) else {
            return self.beside(&[], room);
        };
        let follows = |cell: &Set| 2.0 * COLUMN_SEP + cell.width();
        let opening_room = room - rest.iter().map(follows).sum::<f64>();
        let opening = first.extent(opening_room)?;
        let mut placed = vec![Placed {
            set: Cow::Borrowed(first),
            room: opening_room,
            left: 0.0,
            drop: 0.0,
        }];
        let (mut left, drop) = opening.end;
        let (mut height, mut depth) = (opening.height, opening.depth);
        for cell in rest {
            placed.push(Placed {
                set: Cow::Borrowed(cell),
                room: f64::INFINITY,
                left: left + 2.0 * COLUMN_SEP,
                drop,
            });
            left += follows(cell);
            height = height.max(cell.natural.height - drop);
            depth = depth.max(drop + cell.natural.depth);
        }
        let hung_room = room - CONTINUATION_INDENT;
        let hung = tail.extent(hung_room)?;
        let tail_drop = depth + LINE_GAP + hung.height;
        placed.push(Placed {
            set: Cow::Borrowed(tail),
            room: hung_room,
            left: CONTINUATION_INDENT,
            drop: tail_drop,
        });
        Ok(Arrangement {
            placed,
            extent: Extent {
                width: opening
                    .width
                    .max(left)
                    .max(CONTINUATION_INDENT + hung.width),
                height,
                depth: tail_drop + hung.depth,
                end: (CONTINUATION_INDENT + hung.end.0, tail_drop + hung.end.1),
            },
        })
    }
}

/// Rows set one under another, each on lines of its own.
#[derive(Clone, Debug)]
struct Stack(Vec<Row>);

impl Stack {
    /// How wide each column is for the rows `keeping` to the columns: as wide
    /// as the widest cell of theirs in it.
    fn columns(&self, keeping: &[bool]) -> Vec<f64> {
        let mut columns = Vec::new();
        for (row, _) in self.0.iter().zip(keeping).filter(|(_, keeps)| **keeps) {
            let lead = row.lead();
            columns.resize(columns.len().max(lead.len()), 0.0_f64);
            for (column, cell) in columns.iter_mut().zip(lead) {
                *column = column.max(cell.width());
            }
        }
        columns
    }

    /// The rows in `room`. A row's tail stands beside its lead, in the columns
    /// the rows share, or beneath it, whichever is shorter. The columns are
    /// only as wide as the rows keeping to them need: while a row would be
    /// shorter out of them, the rows of the widest lead among such rows leave,
    /// so one long key does not push every value to the far side.
    fn arranged(&self, room: f64) -> Result<Arrangement<'_>, Unset> {
        let mut beneath = self
            .0
            .iter()
            .map(|row| row.beneath(room).map(Some))
            .collect::<Result<Vec<_>, _>>()?;
        let mut keeping = vec![true; self.0.len()];
        let columns = loop {
            let columns = self.columns(&keeping);
            let mut leaving = Vec::new();
            for (index, row) in self.0.iter().enumerate() {
                let Some(beneath) = beneath[index].as_ref().filter(|_| keeping[index]) else {
                    continue;
                };
                if beneath.extent.cost(room) < row.beside(&columns, room)?.extent.cost(room) {
                    leaving.push(index);
                }
            }
            let lead = |index: &usize| self.0[*index].lead_width();
            let Some(widest) = leaving.iter().map(lead).max_by(f64::total_cmp) else {
                break columns;
            };
            for index in leaving {
                keeping[index] = lead(&index) < widest - TOLERANCE;
            }
        };
        let mut placed = Vec::new();
        let (mut width, mut top) = (0.0_f64, 0.0);
        for (index, row) in self.0.iter().enumerate() {
            let row = match beneath[index].take().filter(|_| !keeping[index]) {
                Some(beneath) => beneath,
                None => row.beside(&columns, room)?,
            };
            let baseline = top + row.extent.height.max(STRUT.0);
            placed.extend(row.placed.into_iter().map(|part| Placed {
                drop: part.drop + baseline,
                ..part
            }));
            width = width.max(row.extent.width);
            top = baseline + row.extent.depth.max(STRUT.1) + LINE_GAP;
        }
        // As a TeX array does, the rows stand centred on the maths axis.
        let total = (top - LINE_GAP).max(0.0);
        let height = total / 2.0 + AXIS;
        for part in &mut placed {
            part.drop -= height;
        }
        Ok(Arrangement {
            placed,
            extent: Extent {
                width,
                height,
                depth: total - height,
                end: (width, 0.0),
            },
        })
    }
}

/// The fences around a body: what each is drawn as, their paint, and the
/// colour of what is not painted.
#[derive(Clone, Copy, Debug)]
struct Fences {
    left: char,
    right: char,
    ink: Option<Rgb>,
    colour: Rgb,
}

impl Fences {
    /// Both fences grown to reach `height` above the baseline and `depth`
    /// below it, as `\left` and `\right` grow them around ink reaching so far.
    fn grown(&self, height: f64, depth: f64) -> Result<[Set; 2], Unset> {
        let strut = format!(
            r"\vphantom{{\rule[{:.4}em]{{1pt}}{{{:.4}em}}}}",
            -depth,
            height + depth
        );
        let fence = |tex: String| {
            let painted = self.ink.map_or_else(|| tex.clone(), |ink| ink.scope(&tex));
            set(&tex, &painted, self.colour).map(Set::flow)
        };
        Ok([
            fence(format!(r"\left{}{strut}\right.", delimiter(self.left)))?,
            fence(format!(r"\left.{strut}\right{}", delimiter(self.right)))?,
        ])
    }

    /// `body` in `room` between its fences. A body laid out taller wants
    /// taller fences, and taller fences are wider, leaving it less room: it is
    /// laid out again for as long as its fences widen, which the sizes TeX
    /// draws a delimiter at end.
    fn around<'a>(&self, body: &'a Set, room: f64) -> Result<Arrangement<'a>, Unset> {
        let taken = |fences: &[Set; 2]| fences.iter().map(Set::width).sum::<f64>();
        let mut fences = self.grown(body.natural.height, body.natural.depth)?;
        loop {
            let within = room - taken(&fences) - 2.0 * COLUMN_SEP;
            let held = body.extent(within)?;
            let fitted = self.grown(held.height, held.depth)?;
            if taken(&fitted) > taken(&fences) + TOLERANCE {
                fences = fitted;
                continue;
            }
            let [left, right] = fitted;
            let inside = left.width() + COLUMN_SEP;
            let closing = inside + held.width + COLUMN_SEP;
            let width = closing + right.width();
            let reaching = |pick: fn(&Extent) -> f64| {
                [&left.natural, &right.natural, &held]
                    .into_iter()
                    .map(pick)
                    .fold(0.0, f64::max)
            };
            let extent = Extent {
                width,
                height: reaching(|extent| extent.height),
                depth: reaching(|extent| extent.depth),
                end: (width, 0.0),
            };
            let place = |set: Cow<'a, Set>, left: f64, room: f64| Placed {
                set,
                room,
                left,
                drop: 0.0,
            };
            return Ok(Arrangement {
                placed: vec![
                    place(Cow::Owned(left), 0.0, f64::INFINITY),
                    place(Cow::Borrowed(body), inside, within),
                    place(Cow::Owned(right), closing, f64::INFINITY),
                ],
                extent,
            });
        }
    }
}

/// Where a break mark stands across its row and its rank, when `item` is one:
/// a rule of no width, its height counting the rank.
fn mark(item: &DisplayItem) -> Option<(f64, usize)> {
    match item {
        DisplayItem::Line {
            x,
            width,
            thickness,
            ..
        } if *width == 0.0 => Some((
            *x,
            (thickness * POINTS_PER_EM / MARK_RANK_POINTS).round() as usize,
        )),
        _ => None,
    }
}

/// How far across, in ems, `item` reaches, from its left edge to its right.
fn reach(item: &DisplayItem) -> (f64, f64) {
    match item {
        DisplayItem::GlyphPath {
            x,
            scale,
            font,
            char_code,
            ..
        } => {
            let advance = FontId::parse(font)
                .and_then(|font| get_char_metrics_with_fallback(font, *char_code))
                .map_or(0.0, |metrics| metrics.width * scale);
            (*x, x + advance)
        }
        DisplayItem::Line { x, width, .. } | DisplayItem::Rect { x, width, .. } => (*x, x + width),
        DisplayItem::Path { x, commands, .. } => commands
            .iter()
            .filter_map(|command| match command {
                PathCommand::MoveTo { x, .. }
                | PathCommand::LineTo { x, .. }
                | PathCommand::CubicTo { x, .. }
                | PathCommand::QuadTo { x, .. } => Some(*x),
                PathCommand::Close => None,
            })
            .fold(
                (f64::INFINITY, f64::NEG_INFINITY),
                |(left, right), offset| (left.min(x + offset), right.max(x + offset)),
            ),
    }
}

/// Move `item` by `(dx, dy)` ems.
pub fn shift(item: &mut DisplayItem, dx: f64, dy: f64) {
    match item {
        DisplayItem::GlyphPath { x, y, .. }
        | DisplayItem::Line { x, y, .. }
        | DisplayItem::Rect { x, y, .. }
        | DisplayItem::Path { x, y, .. } => {
            *x += dx;
            *y += dy;
        }
    }
}
