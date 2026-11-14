//! Diagrams set as one picture: each piece and label set by RaTeX on its own,
//! the pieces placed on a grid, and the arrows between them drawn as filled
//! shapes, which TeX's maths alone cannot do for one running across a column.

use std::fmt::Write;

use ratex_types::color::Color;
use ratex_types::display_item::{DisplayItem, DisplayList};
use ratex_types::path_command::PathCommand;

use super::{set, Rgb, Unset};

/// The first line of a diagram's description, which no formula starts with.
const MARK: &str = "%diagram";
const INK: &str = "ink";
const PIECE: &str = "piece";
const ARROW: &str = "arrow";

/// Ems between neighbouring columns no arrow joins.
const COLUMN_GAP: f64 = 1.2;
/// Ems between rows no arrow joins.
const ROW_GAP: f64 = 0.5;
/// The shortest arrow along a row, in ems.
const MIN_ARROW: f64 = 1.6;
/// The fewest ems an arrow between two rows runs down.
const MIN_BAND: f64 = 2.2;
/// Ems between a piece and the end of an arrow at it.
const CLEARANCE: f64 = 0.25;
/// Ems between a label and its arrow.
const LABEL_CLEARANCE: f64 = 0.15;
/// Ems kept beside a label set over an arrow along a row.
const LABEL_SIDE: f64 = 0.4;
/// Height of TeX's maths axis above the baseline, where arrows along a row run.
const AXIS: f64 = 0.25;
/// Thickness of an arrow's shaft: TeX's default rule.
const RULE: f64 = 0.04;
/// Length of an arrowhead, and half its width, in ems.
const HEAD_LENGTH: f64 = 0.3;
const HEAD_HALF_WIDTH: f64 = 0.14;
/// How far back along the head its notch reaches, as a share of the head's length.
const HEAD_NOTCH: f64 = 0.55;

/// A diagram written as text a formula's place can carry, read back by
/// [`composed`]: pieces on a grid and arrows between places, each in TeX.
///
/// ```ignore
/// let mut description = Description::new(None);
/// description.piece((0, 0), "A");
/// description.piece((1, 0), "B");
/// description.arrow((0, 0), (1, 0), Some("f"));
/// let tex = String::from(description);
/// ```
#[derive(Clone, Debug)]
pub struct Description(String);

impl Description {
    /// A diagram whose arrows are drawn in `ink`, else in the formula's colour.
    pub fn new(ink: Option<Rgb>) -> Self {
        let mut text = format!("{MARK}\n");
        if let Some(Rgb([r, g, b])) = ink {
            let _ = writeln!(text, "{INK} {r} {g} {b}");
        }
        Self(text)
    }

    pub fn piece(&mut self, (row, column): (usize, usize), tex: &str) {
        let _ = writeln!(self.0, "{PIECE} {row} {column} {}", one_line(tex));
    }

    pub fn arrow(&mut self, from: (usize, usize), to: (usize, usize), label: Option<&str>) {
        let _ = write!(self.0, "{ARROW} {} {} {} {}", from.0, from.1, to.0, to.1);
        if let Some(label) = label {
            let _ = write!(self.0, " {}", one_line(label));
        }
        self.0.push('\n');
    }
}

impl From<Description> for String {
    fn from(description: Description) -> Self {
        description.0
    }
}

fn one_line(tex: &str) -> String {
    tex.replace('\n', " ")
}

/// A description read back: its ink, pieces and arrows.
#[derive(Debug, Default)]
struct Read<'a> {
    ink: Option<Rgb>,
    pieces: Vec<((usize, usize), &'a str)>,
    arrows: Vec<((usize, usize), (usize, usize), Option<&'a str>)>,
}

impl<'a> Read<'a> {
    fn from(text: &'a str) -> Option<Self> {
        let mut lines = text.lines();
        (lines.next()? == MARK).then_some(())?;
        let mut read = Self::default();
        for line in lines {
            let (word, rest) = line.split_once(' ')?;
            match word {
                INK => {
                    let channels = rest
                        .split(' ')
                        .map(str::parse)
                        .collect::<Result<Vec<u8>, _>>()
                        .ok()?;
                    read.ink = Some(Rgb(channels.try_into().ok()?));
                }
                PIECE => {
                    let mut fields = rest.splitn(3, ' ');
                    let place = (fields.next()?.parse().ok()?, fields.next()?.parse().ok()?);
                    read.pieces.push((place, fields.next().unwrap_or("")));
                }
                ARROW => {
                    let mut fields = rest.splitn(5, ' ');
                    let mut number = || fields.next()?.parse().ok();
                    let from = (number()?, number()?);
                    let to = (number()?, number()?);
                    read.arrows.push((from, to, fields.next()));
                }
                _ => return None,
            }
        }
        Some(read)
    }
}

/// A box a piece was set in, in ems from the diagram's top left corner.
#[derive(Clone, Copy, Debug)]
struct Frame {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl Frame {
    fn centre(&self) -> (f64, f64) {
        ((self.left + self.right) / 2.0, (self.top + self.bottom) / 2.0)
    }

    /// How far from the centre along `(dx, dy)` the frame, grown by `margin`, ends.
    fn reach(&self, (dx, dy): (f64, f64), margin: f64) -> f64 {
        let half_x = (self.right - self.left) / 2.0 + margin;
        let half_y = (self.bottom - self.top) / 2.0 + margin;
        let along = |half: f64, d: f64| if d.abs() < f64::EPSILON { f64::INFINITY } else { half / d.abs() };
        along(half_x, dx).min(along(half_y, dy))
    }
}

/// The diagram `tex` describes, laid out with each piece set as `painted`
/// paints it; `None` when `tex` describes no diagram.
pub fn composed(
    tex: &str,
    painted: &str,
    colour: Rgb,
    matrix_row_spacing_percent: u16,
) -> Option<Result<DisplayList, Unset>> {
    let layout = Read::from(tex)?;
    let paint = Read::from(painted).unwrap_or_default();
    Some(compose(&layout, &paint, colour, matrix_row_spacing_percent))
}

fn compose(
    layout: &Read,
    paint: &Read,
    colour: Rgb,
    spacing: u16,
) -> Result<DisplayList, Unset> {
    let set_piece = |tex: &str, painted: Option<&str>| set(tex, painted.unwrap_or(tex), colour, spacing);
    let small = |tex: &str| format!(r"{{\scriptsize {tex}}}");
    let pieces = layout
        .pieces
        .iter()
        .enumerate()
        .map(|(index, (place, tex))| {
            let painted = paint.pieces.get(index).map(|(_, painted)| *painted);
            set_piece(tex, painted).map(|list| (*place, list))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let labels = layout
        .arrows
        .iter()
        .enumerate()
        .map(|(index, (.., label))| {
            let painted = paint.arrows.get(index).and_then(|(.., painted)| *painted);
            label
                .map(|label| set_piece(&small(label), painted.map(small).as_deref()))
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()?;

    let rows = pieces.iter().map(|((row, _), _)| row + 1).max().unwrap_or(0);
    let columns = pieces.iter().map(|((_, column), _)| column + 1).max().unwrap_or(0);
    let mut grid = Grid {
        widths: vec![0.0; columns],
        above: vec![0.0; rows],
        below: vec![0.0; rows],
        rising: vec![0.0; rows],
        hanging: vec![0.0; rows],
        gaps: vec![COLUMN_GAP; columns],
        bands: vec![ROW_GAP; rows],
        entered: (0..rows)
            .map(|row| layout.arrows.iter().any(|(from, to, _)| to.0 == row && from.0 != row))
            .collect(),
    };
    for ((row, column), list) in &pieces {
        grid.widths[*column] = grid.widths[*column].max(list.width);
        grid.above[*row] = grid.above[*row].max(list.height);
        grid.below[*row] = grid.below[*row].max(list.depth);
    }
    let label_sizes = labels
        .iter()
        .map(|label| label.as_ref().map(|label| (label.width, label.height + label.depth)))
        .collect::<Vec<_>>();
    for (index, (from, to, _)) in layout.arrows.iter().enumerate() {
        let (label_width, label_height) = label_sizes[index].unwrap_or_default();
        let left = from.1.min(to.1);
        if from.0 == to.0 && grid.entered[from.0] {
            let reach = LABEL_CLEARANCE + label_height - AXIS;
            grid.hanging[from.0] = grid.hanging[from.0].max(reach);
        } else if from.0 == to.0 {
            let reach = AXIS + LABEL_CLEARANCE + label_height;
            grid.rising[from.0] = grid.rising[from.0].max(reach);
            grid.gaps[left] = grid.gaps[left]
                .max(MIN_ARROW + 2.0 * CLEARANCE)
                .max(label_width + 2.0 * LABEL_SIDE);
            continue;
        }
        // A label beside a straight arrow is centred on the band; one beside a
        // leaning arrow hangs below the band's middle.
        let top = from.0.min(to.0);
        let room = match from.1 == to.1 {
            true => label_height,
            false => 2.0 * label_height,
        };
        grid.bands[top] = grid.bands[top]
            .max(MIN_BAND)
            .max(room + 2.0 * (LABEL_CLEARANCE + RULE));
        if from.1 == to.1 {
            let reach =
                label_width + LABEL_CLEARANCE + COLUMN_GAP / 2.0 - grid.widths[left] / 2.0;
            grid.gaps[left] = grid.gaps[left].max(reach);
        }
    }
    let sizes = pieces
        .iter()
        .map(|(place, list)| (*place, (list.width, list.height, list.depth)))
        .collect::<Vec<_>>();
    // A label beside a downward arrow must clear the arrow from the neighbouring
    // column on its side; the gap between their sources widens until it does.
    for _ in 0..MAX_WIDENINGS {
        let routes = grid.routes(&sizes, &layout.arrows);
        let mut widened = false;
        for (index, (from, to, _)) in layout.arrows.iter().enumerate() {
            let Some(size) = label_sizes[index].filter(|_| from.0 != to.0) else {
                continue;
            };
            let (left, top) = grid.label_corner(routes[index], *from, size);
            let right_side = Grid::label_on_right(routes[index]);
            let Some(column) = (match right_side {
                true => Some(from.1 + 1),
                false => from.1.checked_sub(1),
            }) else {
                continue;
            };
            for (other, (next_from, next_to, _)) in layout.arrows.iter().enumerate() {
                if next_from.0 != from.0 || next_to.0 != to.0 || next_from.1 != column {
                    continue;
                }
                let crossings = [top, top + size.1].map(|y| x_along(routes[other], y));
                let overlap = match right_side {
                    true => left + size.0 + LABEL_CLEARANCE - crossings[0].min(crossings[1]),
                    false => crossings[0].max(crossings[1]) + LABEL_CLEARANCE - left,
                };
                if overlap > 0.0 {
                    grid.gaps[from.1.min(column)] += overlap;
                    widened = true;
                }
            }
        }
        if !widened {
            break;
        }
    }
    eprintln!("{grid:?} {:?}", grid.baselines());

    let (width, height) = grid.size();
    let mut diagram = DisplayList {
        items: Vec::new(),
        width,
        height,
        depth: 0.0,
    };
    let routes = grid.routes(&sizes, &layout.arrows);
    for (place, list) in pieces {
        let frame = grid.frame(place, (list.width, list.height, list.depth));
        let baseline = frame.top + list.height;
        place_list(&mut diagram, list, frame.left, baseline);
    }
    let ink = Color::from(layout.ink.or(paint.ink).unwrap_or(colour));
    for (index, (from, to, _)) in layout.arrows.iter().enumerate() {
        let (tail, tip) = routes[index];
        diagram.items.extend(arrow(tail, tip, ink));
        if let Some(label) = labels[index].clone() {
            let size = (label.width, label.height + label.depth);
            let middle = (tail.0 + tip.0) / 2.0 - size.0 / 2.0;
            let (left, top) = match from.0 == to.0 {
                true if grid.entered[from.0] => (middle, tail.1 + LABEL_CLEARANCE),
                true => (middle, tail.1 - LABEL_CLEARANCE - size.1),
                false => grid.label_corner((tail, tip), *from, size),
            };
            let baseline = top + label.height;
            place_list(&mut diagram, label, left, baseline);
        }
    }
    Ok(diagram)
}

/// How many times the gaps between downward arrows may widen to clear labels.
const MAX_WIDENINGS: usize = 16;

/// Where an arrow from `tail` to `tip` crosses the level `y`.
fn x_along((tail, tip): ((f64, f64), (f64, f64)), y: f64) -> f64 {
    let fall = tip.1 - tail.1;
    match fall.abs() < f64::EPSILON {
        true => tail.0,
        false => tail.0 + (tip.0 - tail.0) * ((y - tail.1) / fall).clamp(0.0, 1.0),
    }
}

/// A diagram's grid in ems: each column's width and the gap after it, each row's
/// reach above and below its baseline and the band after it. Pieces stand in
/// columns and labels along a row in the gaps between them, so a row's pieces
/// and its labels reach apart, and rows are kept clear piece of piece and label
/// of label.
#[derive(Clone, Debug)]
struct Grid {
    widths: Vec<f64>,
    above: Vec<f64>,
    below: Vec<f64>,
    /// How far the labels over a row's arrows rise above its baseline.
    rising: Vec<f64>,
    /// How far the labels under a row's arrows hang below its baseline.
    hanging: Vec<f64>,
    gaps: Vec<f64>,
    bands: Vec<f64>,
    /// Whether arrows come down into each row, whose labels along it then go
    /// beneath its arrows, out of their way, as a ladder's lower row has them.
    entered: Vec<bool>,
}

type Route = ((f64, f64), (f64, f64));

impl Grid {
    fn lefts(&self) -> Vec<f64> {
        self.widths
            .iter()
            .zip(&self.gaps)
            .scan(0.0, |x, (width, gap)| {
                let left = *x;
                *x += width + gap;
                Some(left)
            })
            .collect()
    }

    fn baselines(&self) -> Vec<f64> {
        let mut baselines = Vec::with_capacity(self.above.len());
        for row in 0..self.above.len() {
            let baseline = match row.checked_sub(1) {
                None => self.above[row].max(self.rising[row]),
                Some(previous) => {
                    let pieces = self.below[previous] + self.above[row];
                    let labels = self.hanging[previous] + self.rising[row];
                    baselines[previous] + pieces.max(labels) + self.bands[previous]
                }
            };
            baselines.push(baseline);
        }
        baselines
    }

    /// Where the band below `row` begins and ends: below its pieces, above the
    /// next row's.
    fn band(&self, row: usize) -> (f64, f64) {
        let baselines = self.baselines();
        (
            baselines[row] + self.below[row],
            baselines[row + 1] - self.above[row + 1],
        )
    }

    fn size(&self) -> (f64, f64) {
        let width = self.lefts().last().zip(self.widths.last()).map_or(0.0, |(l, w)| l + w);
        let height = self
            .baselines()
            .last()
            .zip(self.below.last().zip(self.hanging.last()))
            .map_or(0.0, |(baseline, (below, hanging))| baseline + below.max(*hanging));
        (width, height)
    }

    /// The frame a piece `(width, height, depth)` is set in at `place`, centred
    /// in its column on its row's baseline.
    fn frame(&self, (row, column): (usize, usize), (width, height, depth): (f64, f64, f64)) -> Frame {
        let left = self.lefts()[column] + (self.widths[column] - width) / 2.0;
        let baseline = self.baselines()[row];
        Frame {
            left,
            top: baseline - height,
            right: left + width,
            bottom: baseline + depth,
        }
    }

    /// Where each arrow runs: along its row's maths axis between the pieces it
    /// joins, or from piece to piece as their centres line up.
    fn routes(
        &self,
        sizes: &[((usize, usize), (f64, f64, f64))],
        arrows: &[((usize, usize), (usize, usize), Option<&str>)],
    ) -> Vec<Route> {
        let frame_at = |place: (usize, usize)| {
            sizes
                .iter()
                .find(|(at, _)| *at == place)
                .map_or_else(|| self.frame(place, (0.0, 0.0, 0.0)), |(_, size)| self.frame(place, *size))
        };
        let baselines = self.baselines();
        arrows
            .iter()
            .map(|(from, to, _)| {
                let (source, target) = (frame_at(*from), frame_at(*to));
                if from.0 == to.0 {
                    let axis = baselines[from.0] - AXIS;
                    return match to.1 < from.1 {
                        true => ((source.left - CLEARANCE, axis), (target.right + CLEARANCE, axis)),
                        false => ((source.right + CLEARANCE, axis), (target.left - CLEARANCE, axis)),
                    };
                }
                let (start, end) = (source.centre(), target.centre());
                let length = (end.0 - start.0).hypot(end.1 - start.1);
                let direction = ((end.0 - start.0) / length, (end.1 - start.1) / length);
                let out = source.reach(direction, CLEARANCE);
                let into = target.reach(direction, CLEARANCE);
                (
                    (start.0 + direction.0 * out, start.1 + direction.1 * out),
                    (end.0 - direction.0 * into, end.1 - direction.1 * into),
                )
            })
            .collect()
    }

    /// The top left corner of a label `(width, height)` beside an arrow between
    /// rows, halfway down the band where the arrow crosses it: right of a straight
    /// arrow and centred on it, below a leaning one on the side it leans away
    /// from, cornered to the arrow so the arrow runs away from it.
    fn label_corner(&self, route: Route, from: (usize, usize), (width, height): (f64, f64)) -> (f64, f64) {
        let (band_top, band_bottom) = self.band(from.0);
        let level = (band_top + band_bottom) / 2.0;
        let x = x_along(route, level);
        let ((tail_x, tail_y), (tip_x, tip_y)) = route;
        let (run, fall) = (tip_x - tail_x, tip_y - tail_y);
        let gap = LABEL_CLEARANCE + RULE;
        if run.abs() < f64::EPSILON {
            return (x + gap, level - height / 2.0);
        }
        let length = run.hypot(fall);
        // The normal pointing down, on the side the arrow leans away from.
        let (normal_x, normal_y) = match fall * run < 0.0 {
            true => (fall.abs() / length, run.abs() / length),
            false => (-fall.abs() / length, run.abs() / length),
        };
        let left = x + gap * normal_x - if normal_x < 0.0 { width } else { 0.0 };
        (left, level + gap * normal_y)
    }

    /// Which side of its arrow a label between rows stands on: the right of a
    /// straight arrow or one leaning left, the left of one leaning right.
    fn label_on_right(route: Route) -> bool {
        let ((tail_x, _), (tip_x, _)) = route;
        tip_x <= tail_x + f64::EPSILON
    }
}

/// Draw `list` into `diagram` with its left edge at `left` and baseline at `baseline`.
fn place_list(diagram: &mut DisplayList, list: DisplayList, left: f64, baseline: f64) {
    let (dx, dy) = (left, baseline - list.height);
    diagram.items.extend(list.items.into_iter().map(|mut item| {
        match &mut item {
            DisplayItem::GlyphPath { x, y, .. }
            | DisplayItem::Line { x, y, .. }
            | DisplayItem::Rect { x, y, .. }
            | DisplayItem::Path { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
        }
        item
    }));
}

/// An arrow from `tail` to `tip`: a shaft of TeX's rule thickness and a notched head.
fn arrow(tail: (f64, f64), tip: (f64, f64), color: Color) -> [DisplayItem; 2] {
    let length = (tip.0 - tail.0).hypot(tip.1 - tail.1).max(f64::EPSILON);
    let along = ((tip.0 - tail.0) / length, (tip.1 - tail.1) / length);
    let across = (-along.1, along.0);
    let at = |back: f64, side: f64| PathCommand::LineTo {
        x: tip.0 - along.0 * back + across.0 * side,
        y: tip.1 - along.1 * back + across.1 * side,
    };
    let start = |command: PathCommand| match command {
        PathCommand::LineTo { x, y } => PathCommand::MoveTo { x, y },
        other => other,
    };
    let notch = HEAD_LENGTH * HEAD_NOTCH;
    let half = RULE / 2.0;
    let shaft_back = length;
    let shaft = vec![
        start(at(shaft_back, half)),
        at(notch, half),
        at(notch, -half),
        at(shaft_back, -half),
        PathCommand::Close,
    ];
    let head = vec![
        start(at(0.0, 0.0)),
        at(HEAD_LENGTH, HEAD_HALF_WIDTH),
        at(notch, 0.0),
        at(HEAD_LENGTH, -HEAD_HALF_WIDTH),
        PathCommand::Close,
    ];
    [shaft, head].map(|commands| DisplayItem::Path {
        x: 0.0,
        y: 0.0,
        commands,
        fill: true,
        color,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_description_reads_back_as_written() {
        let mut description = Description::new(Some(Rgb([1, 2, 3])));
        description.piece((0, 1), r"\frac{a}{b}");
        description.arrow((0, 1), (1, 0), Some("f\ng"));
        description.arrow((0, 1), (0, 0), None);
        let text = String::from(description);
        let read = Read::from(&text).expect("a description reads back");
        assert_eq!(read.ink, Some(Rgb([1, 2, 3])));
        assert_eq!(read.pieces, vec![((0, 1), r"\frac{a}{b}")]);
        assert_eq!(read.arrows, vec![((0, 1), (1, 0), Some("f g")), ((0, 1), (0, 0), None)]);
        assert!(Read::from(r"\frac{a}{b}").is_none(), "a formula is no description");
    }

    #[test]
    fn pieces_sharing_a_column_are_centred_on_one_line_and_arrows_are_drawn() {
        let mut description = Description::new(None);
        description.piece((0, 0), "A");
        description.piece((0, 1), "B");
        description.piece((1, 0), "CCCCCC");
        description.arrow((0, 1), (0, 0), Some("d"));
        description.arrow((0, 1), (1, 0), Some("f"));
        let text = String::from(description);
        let list = composed(&text, &text, Rgb([0, 0, 0]), 100)
            .expect("a description")
            .expect("sets");
        let filled = list
            .items
            .iter()
            .filter(|item| matches!(item, DisplayItem::Path { fill: true, x, y, .. } if *x == 0.0 && *y == 0.0))
            .count();
        assert_eq!(filled, 4, "each arrow is a shaft and a head");
        assert!(list.width > 0.0 && list.height > 0.0);
    }
}
