//! Pictures in the transcript, drawn through the terminal's graphics protocol.
//!
//! What the terminal can draw is learned as it starts, and read from here as
//! the scheme is: it is a fact about the terminal, not state any element owns. A
//! picture is made while the value it shows is laid out, kept for every later
//! layout, and sent to the terminal after the frame that first places it.

pub mod kitty;
pub mod ratex;
mod typewriter;

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::fmt::{self, Display, Formatter};
use std::mem;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock};
use std::thread;

use crossterm::terminal::window_size;
use ratatui::buffer::Buffer;
use ratatui::layout::Size;
use ratatui::text::Line;
use ratatui_image::picker::{Picker, ProtocolType};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{Options, Tree};

use crate::env::ENV;
use crate::ui::config::{TypesetSizing, UI};
use crate::ui::debug::mebibytes;
use kitty::{ImageId, KittyImage, Passthrough, Piece};
use ratex::{Rgb, Set, Source, EM, PADDING};

// @##magic
/// Lets this pane's escape sequences through tmux to the terminal.
const TMUX_ALLOW_PASSTHROUGH: [&str; 4] = ["set", "-p", "allow-passthrough", "on"];
/// tmux's names for the terminal its client runs in, one a line: as the terminal
/// reports itself, then its `TERM`.
const TMUX_CLIENT_TERMINAL: &str = "#{client_termtype}\n#{client_termname}";
/// What a `TERM` puts before the terminal's own name, as in `xterm-ghostty`.
const TERM_FAMILY_PREFIX: &str = "xterm-";
/// Terminals drawing kitty pictures by Unicode placeholders, as they name themselves.
const PLACEHOLDER_TERMINALS: [&str; 2] = ["kitty", "ghostty"];
/// The distance of a picture in a view that is not showing, further than any row.
pub const HIDDEN_VIEW_DISTANCE: usize = usize::MAX;
/// Bytes of decoded pixels the terminal may hold for pictures off the screen. Kitty
/// and Ghostty each evict pictures, on screen or not, past a quota of 320 MiB, so
/// this keeps well clear of it.
const TERMINAL_IMAGE_BUDGET: usize = 96 * 1024 * 1024;
/// Bytes a pixel takes once the terminal has decoded a picture.
const DECODED_PIXEL_BYTES: usize = 4;
/// Dots an inch at which a point, the unit RaTeX sizes its pictures in, is one
/// pixel, so a picture is read in the units it was drawn in.
const POINT_PER_PIXEL_DPI: f32 = 72.0;

/// A terminal that shows pictures: the pixels in one of its cells, and how escape
/// sequences reach it.
#[derive(Clone, Copy, Debug)]
struct Canvas {
    cell: Size,
    passthrough: Passthrough,
}

/// Set only when the terminal answered that it shows kitty pictures.
static CANVAS: OnceLock<Mutex<Canvas>> = OnceLock::new();

static GALLERY: LazyLock<Mutex<Gallery>> = LazyLock::new(|| Mutex::new(Gallery::default()));

// @##issue free functions
/// Learn what the terminal can draw. Runs on the alternate screen before any event
/// is read, since a terminal asked directly answers as input.
pub fn detect() {
    let canvas = match ENV.in_tmux() {
        true => tmux_canvas(),
        false => queried_canvas(),
    };
    if let Some(canvas) = canvas {
        let _ = CANVAS.set(Mutex::new(canvas));
    }
}

/// The canvas of a terminal asked directly, by escape sequences it answers on stdin.
fn queried_canvas() -> Option<Canvas> {
    let picker = Picker::from_query_stdio().ok()?;
    let font = picker.font_size();
    (picker.protocol_type() == ProtocolType::Kitty).then(|| Canvas {
        cell: Size::new(font.width, font.height),
        passthrough: Passthrough::Direct,
    })
}

/// The canvas of the terminal tmux runs in, learned without asking it anything:
/// tmux keeps the terminal's answer to a graphics query, and a query left
/// unanswered leaves a reader on stdin that takes the next keypress. The terminal
/// is known by the name tmux gives it, its cells by the pixels tmux reports.
fn tmux_canvas() -> Option<Canvas> {
    terminal().filter(|name| PLACEHOLDER_TERMINALS.contains(name))?;
    let window = window_size().ok()?;
    if window.columns == 0 || window.rows == 0 || window.width == 0 || window.height == 0 {
        return None;
    }
    let _ = Command::new("tmux")
        .args(TMUX_ALLOW_PASSTHROUGH)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    Some(Canvas {
        cell: Size::new(window.width / window.columns, window.height / window.rows),
        passthrough: Passthrough::Tmux,
    })
}

/// The terminal drawing the screen, as one lowercase word fit to print; inside
/// tmux, the terminal tmux runs in.
pub fn terminal() -> Option<&'static str> {
    static TERMINAL: LazyLock<Option<String>> = LazyLock::new(|| {
        let names = match ENV.in_tmux() {
            true => tmux_client_names(),
            false => ENV.terminal_names(),
        };
        names.iter().find_map(|name| printable_name(name))
    });
    TERMINAL.as_deref()
}

fn tmux_client_names() -> Vec<String> {
    Command::new("tmux")
        .args(["display-message", "-p", TMUX_CLIENT_TERMINAL])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// `name` as one lowercase word: `ghostty 1.3.1` and `kitty(0.36.4)` lose their
/// versions, `xterm-ghostty` its family; none for what is not a name, such as
/// `xterm-256color`.
fn printable_name(name: &str) -> Option<String> {
    let name = name.split([' ', '(']).next()?;
    let name = name
        .strip_prefix(TERM_FAMILY_PREFIX)
        .unwrap_or(name)
        .to_lowercase();
    let is_word = name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric());
    is_word.then_some(name)
}

/// Whether this terminal shows pictures.
// @##wtf
pub fn available() -> bool {
    CANVAS.get().is_some()
}

/// The pixel dimensions used to draw pictures in each terminal cell.
pub fn cell_size() -> Option<Size> {
    CANVAS.get()?.lock().ok().map(|canvas| canvas.cell)
}

/// Refresh the cell's pixel dimensions after terminal zoom or a display change.
/// Returns whether pictures need to be laid out again, keeping the last valid
/// dimensions when the terminal reports no pixel size.
pub fn refresh_cell_size() -> bool {
    let Some(canvas) = CANVAS.get() else {
        return false;
    };
    let Ok(window) = window_size() else {
        return false;
    };
    if window.columns == 0 || window.rows == 0 {
        return false;
    }
    let cell = Size::new(window.width / window.columns, window.height / window.rows);
    if cell.width == 0 || cell.height == 0 {
        return false;
    }
    let mut canvas = canvas
        .lock()
        .expect("the canvas lock is never held across a panic");
    if canvas.cell == cell {
        return false;
    }
    canvas.cell = cell;
    true
}

/// A formula to draw: what it is set from and the M2 code a copy of its picture
/// gives.
#[derive(Clone, Copy, Debug)]
pub struct Typesetting<'a> {
    pub source: &'a Source,
    pub code: &'a str,
    /// Preferred size relative to terminal text.
    pub scale_percent: u16,
}

/// A formula's placeholder lines and the one its baseline runs through.
#[derive(Clone, Debug)]
pub struct Pictured {
    pub lines: Vec<Line<'static>>,
    pub baseline: usize,
}

/// How a picture taller than a row takes up the rows it stands in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Placement {
    /// With nothing beside it: as few rows as hold it, its ink centred down them.
    Alone,
    /// Beside text: its baseline on the text baseline of a row, so that row lines
    /// up with what is beside it, at the cost of the rows that takes.
    AmongText,
}

/// Typeset `formula` in `colour` to stand beside text, retaining its code for
/// copying. Returns placeholder lines no wider than `width`: the formula broken
/// into lines where it may be, shrunk only for what no break shortens, by at most
/// [`UI::MAX_MATH_SHRINK_PERCENT`] and never below the smallest maths size, then
/// cut off at the right edge; or none when
/// pictures are unavailable.
pub fn typeset(formula: Typesetting, colour: Rgb, width: u16) -> Option<Pictured> {
    pictured(formula, colour, width, Placement::AmongText)
}

/// [`typeset`] for a formula with nothing beside it.
pub fn typeset_alone(formula: Typesetting, colour: Rgb, width: u16) -> Option<Pictured> {
    pictured(formula, colour, width, Placement::Alone)
}

fn pictured(
    formula: Typesetting,
    colour: Rgb,
    width: u16,
    placement: Placement,
) -> Option<Pictured> {
    let canvas = *CANVAS.get()?.lock().ok()?;
    gallery()
        .formula(formula, colour, canvas, width, placement)
        .map(|(image, baseline)| Pictured {
            lines: image.lines(),
            baseline,
        })
}

/// The M2 code the picture `id` shows.
pub fn source(id: ImageId) -> Option<String> {
    gallery().sources.get(&id).cloned()
}

/// Settle the terminal's pictures with those `frame` shows: have each it lacks
/// encoded, send what is encoded, and drop the ones it can spare once it holds more
/// than its budget. Returns the escape sequences to write after the frame; a picture
/// still being encoded is drawn blank until a later call sends it.
pub fn synchronise(frame: &Buffer) -> String {
    let Some(canvas) = CANVAS
        .get()
        .and_then(|canvas| canvas.lock().ok().map(|canvas| *canvas))
    else {
        return String::new();
    };
    gallery().synchronise(&shown_in(frame), canvas.passthrough)
}

/// The pictures whose placeholders `cells` holds.
pub fn shown_in(cells: &Buffer) -> HashSet<ImageId> {
    cells
        .content()
        .iter()
        .filter_map(|cell| Piece::placed(cell.symbol(), cell.fg))
        .map(|piece| piece.image)
        .collect()
}

/// The pictures whose placeholders `lines` hold.
pub fn shown_in_lines<'a>(lines: impl IntoIterator<Item = &'a Line<'a>>) -> HashSet<ImageId> {
    lines
        .into_iter()
        .flat_map(|line| &line.spans)
        .filter_map(|span| {
            let from = span.content.find(kitty::PLACEHOLDER)?;
            Piece::placed(&span.content[from..], span.style.fg?)
        })
        .map(|piece| piece.image)
        .collect()
}

/// Rows between the viewport and the block holding each picture, as a frame stands:
/// every picture a block holds is reported, those on screen at none. A picture the
/// terminal holds that no frame reports belongs to no block, so is dropped, and when
/// the terminal holds too much, the furthest goes first.
pub fn report_distances(distances: impl IntoIterator<Item = (ImageId, usize)>) {
    let mut gallery = gallery();
    for (image, rows) in distances {
        gallery
            .distances
            .entry(image)
            .and_modify(|nearest| *nearest = (*nearest).min(rows))
            .or_insert(rows);
    }
}

/// The escape sequences sending each picture encoded since the last frame, which
/// need no frame of their own to be sent.
pub fn send_encoded() -> String {
    let Some(canvas) = CANVAS
        .get()
        .and_then(|canvas| canvas.lock().ok().map(|canvas| *canvas))
    else {
        return String::new();
    };
    gallery().send_encoded(canvas.passthrough)
}

/// Whether pictures are still being encoded, so their sequences are yet to come.
pub fn encoding() -> bool {
    !gallery().encoding.is_empty()
}

/// Send the pictures on screen again and drop the rest, through tmux, which loses
/// those sent while its pane is hidden; called each time the pane regains focus.
pub fn resend_all() {
    if !CANVAS.get().is_some_and(|canvas| {
        canvas
            .lock()
            .is_ok_and(|canvas| canvas.passthrough == Passthrough::Tmux)
    }) {
        return;
    }
    gallery().resend = true;
}

/// What the pictures take up, here and in the terminal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Formulas typeset and kept as vector pictures.
    pub formulas: usize,
    /// Pictures made, each kept as the recipe that draws it again.
    pub images: usize,
    /// Pictures the terminal holds now.
    pub resident: usize,
    /// Bytes of decoded pixels the terminal holds now.
    pub resident_bytes: usize,
}

impl Display for Usage {
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        write!(
            formatter,
            "pictures {} kept / {} in terminal ({}) / {} formulas",
            self.images,
            self.resident,
            mebibytes(self.resident_bytes as u64),
            self.formulas,
        )
    }
}

pub fn usage() -> Usage {
    gallery().usage()
}

fn gallery() -> MutexGuard<'static, Gallery> {
    GALLERY
        .lock()
        .expect("the gallery lock is never held across a panic")
}

/// A formula as set in one colour.
type FormulaKey = (Source, Rgb);
/// A formula as laid out for a room: the room in ems, bit for bit, or none when
/// it has all the room it wants, where every wider room lays it out the same.
type LayoutKey = (FormulaKey, Option<u64>);
/// A formula's picture at a column count and cell pixel size, for its copied code.
/// Values sharing TeX but not code each get their own picture to copy from.
type ImageKey = (LayoutKey, u16, Size, String, Placement, u16);

/// A formula's vector picture, as laid out for one room.
struct Formula {
    tree: Tree,
    /// Where the text baseline runs down the picture, as a share of its height.
    baseline: f32,
}

/// Every picture made, kept so a layout at another width or a resend reuses it. Only
/// what draws a picture again is kept, never its pixels: those are made when the
/// terminal is sent them, and the terminal holds no more than its budget.
struct Gallery {
    /// Each formula typeset, or none when RaTeX refused it.
    sets: HashMap<FormulaKey, Option<Arc<Set>>>,
    /// Each formula's vector picture in each room it was laid out for.
    formulas: HashMap<LayoutKey, Option<Arc<Formula>>>,
    /// Each formula rasterised to a number of columns at a cell pixel size.
    images: HashMap<ImageKey, Option<Arc<KittyImage>>>,
    last_id: ImageId,
    recipes: HashMap<ImageId, Recipe>,
    /// The pictures the terminal holds, as far as anything sent to it says.
    resident: HashMap<ImageId, Resident>,
    jobs: Sender<Encoding>,
    encoded: Receiver<Encoded>,
    /// Pictures on the worker's queue, which are not queued again.
    encoding: HashSet<ImageId>,
    /// Pictures the worker could not encode, which are not tried again.
    unencodable: HashSet<ImageId>,
    /// How far, in rows, the block holding each picture stands from the viewport, as
    /// reported while the frame was drawn.
    distances: HashMap<ImageId, usize>,
    /// Bytes of decoded pixels the terminal may hold for pictures off the screen.
    budget: usize,
    /// Frames synchronised so far.
    frame: u64,
    /// Whether the terminal may have lost what it was sent, so the next frame sends
    /// again what it shows.
    resend: bool,
    /// The M2 code each picture shows, so copying it gives back what M2 reads.
    sources: HashMap<ImageId, String>,
}

/// What draws a picture again.
struct Recipe {
    image: Arc<KittyImage>,
    formula: Arc<Formula>,
    raster: Raster,
}

impl Recipe {
    fn decoded_bytes(&self) -> usize {
        let Raster { cells, cell, .. } = self.raster;
        usize::from(cells.width)
            * usize::from(cell.width)
            * usize::from(cells.height)
            * usize::from(cell.height)
            * DECODED_PIXEL_BYTES
    }
}

/// A picture to encode as PNG on the worker thread.
struct Encoding {
    id: ImageId,
    formula: Arc<Formula>,
    raster: Raster,
}

/// A picture the worker has encoded, none when it could not be.
type Encoded = (ImageId, Option<Vec<u8>>);

/// The worker encoding pictures one at a time, off the thread that draws frames:
/// the queue to send it pictures on, and the pictures it has encoded, none when a
/// picture could not be.
fn encoder() -> (Sender<Encoding>, Receiver<Encoded>) {
    let (jobs, queue) = channel::<Encoding>();
    let (encoded, results) = channel();
    thread::spawn(move || {
        for Encoding {
            id,
            formula,
            raster,
        } in queue
        {
            if encoded.send((id, raster.png(&formula.tree))).is_err() {
                break;
            }
        }
    });
    (jobs, results)
}

/// A picture held by the terminal.
struct Resident {
    bytes: usize,
    last_shown: u64,
}

impl Default for Gallery {
    fn default() -> Self {
        let (jobs, encoded) = encoder();
        Self {
            sets: Default::default(),
            formulas: Default::default(),
            images: Default::default(),
            last_id: ImageId::from(rand::random::<u32>()),
            recipes: Default::default(),
            resident: Default::default(),
            jobs,
            encoded,
            encoding: Default::default(),
            unencodable: Default::default(),
            distances: Default::default(),
            budget: TERMINAL_IMAGE_BUDGET,
            frame: 0,
            resend: false,
            sources: Default::default(),
        }
    }
}

impl Gallery {
    fn formula(
        &mut self,
        Typesetting {
            source,
            code,
            scale_percent,
        }: Typesetting,
        colour: Rgb,
        canvas: Canvas,
        width: u16,
        placement: Placement,
    ) -> Option<(Arc<KittyImage>, usize)> {
        let key = (source.clone(), colour);
        let set = self
            .sets
            .entry(key.clone())
            .or_insert_with(|| Set::new(source, colour).ok().map(Arc::new))
            .clone()?;
        let fit = Fit::new(canvas, width, scale_percent);
        let layout = (key, (set.width() > fit.room).then(|| fit.room.to_bits()));
        let formula = self
            .formulas
            .entry(layout.clone())
            .or_insert_with(|| {
                let picture = set.picture(fit.room, f64::from(fit.least)).ok()?;
                let options = Options {
                    dpi: POINT_PER_PIXEL_DPI,
                    ..Options::default()
                };
                let tree = Tree::from_str(&picture.svg, &options).ok()?;
                Some(Arc::new(Formula {
                    tree,
                    baseline: picture.baseline,
                }))
            })
            .clone()?;
        let raster = Raster::fitted(&formula, canvas, width, placement, fit);
        let baseline = usize::from(raster.baseline);
        let image_key = (
            layout,
            raster.cells.width,
            canvas.cell,
            code.to_string(),
            placement,
            scale_percent,
        );
        if let Some(image) = self.images.get(&image_key) {
            return image.clone().map(|image| (image, baseline));
        }
        self.last_id = self.last_id.next();
        let image = KittyImage::new(self.last_id, raster.cells)
            .ok()
            .map(Arc::new);
        if let Some(image) = &image {
            self.sources.insert(image.id(), code.to_string());
            self.recipes.insert(
                image.id(),
                Recipe {
                    image: image.clone(),
                    formula,
                    raster,
                },
            );
        }
        self.images.insert(image_key, image.clone());
        image.map(|image| (image, baseline))
    }

    /// The sequences sending each picture the worker has finished.
    fn send_encoded(&mut self, passthrough: Passthrough) -> String {
        let mut sequences = String::new();
        while let Ok((id, png)) = self.encoded.try_recv() {
            self.encoding.remove(&id);
            let (Some(png), Some(recipe)) = (png, self.recipes.get(&id)) else {
                self.unencodable.insert(id);
                continue;
            };
            sequences.push_str(&recipe.image.transmission(&png, passthrough));
            self.resident.insert(
                id,
                Resident {
                    bytes: recipe.decoded_bytes(),
                    last_shown: self.frame,
                },
            );
        }
        sequences
    }

    fn synchronise(&mut self, shown: &HashSet<ImageId>, passthrough: Passthrough) -> String {
        self.frame += 1;
        let mut sequences = self.send_encoded(passthrough);
        if mem::take(&mut self.resend) {
            for (id, _) in self.resident.drain() {
                if !shown.contains(&id) {
                    sequences.push_str(&id.deletion(passthrough));
                }
            }
        }
        for &id in shown {
            if let Some(resident) = self.resident.get_mut(&id) {
                resident.last_shown = self.frame;
                continue;
            }
            if self.encoding.contains(&id) || self.unencodable.contains(&id) {
                continue;
            }
            let Some(recipe) = self.recipes.get(&id) else {
                continue;
            };
            let job = Encoding {
                id,
                formula: recipe.formula.clone(),
                raster: recipe.raster.clone(),
            };
            if self.jobs.send(job).is_ok() {
                self.encoding.insert(id);
            }
        }
        let unreachable = self
            .resident
            .keys()
            .filter(|id| !shown.contains(id) && !self.distances.contains_key(id))
            .copied()
            .collect::<Vec<_>>();
        for id in unreachable {
            self.resident.remove(&id);
            sequences.push_str(&id.deletion(passthrough));
        }
        while self.resident_bytes() > self.budget {
            let Some(furthest) = self
                .resident
                .iter()
                .filter(|(id, _)| !shown.contains(id))
                .filter_map(|(id, resident)| Some((id, resident, *self.distances.get(id)?)))
                .max_by_key(|(_, resident, distance)| (*distance, Reverse(resident.last_shown)))
                .map(|(id, ..)| *id)
            else {
                break;
            };
            self.resident.remove(&furthest);
            sequences.push_str(&furthest.deletion(passthrough));
        }
        self.distances.clear();
        sequences
    }

    fn resident_bytes(&self) -> usize {
        self.resident.values().map(|resident| resident.bytes).sum()
    }

    fn usage(&self) -> Usage {
        Usage {
            formulas: self.sets.values().flatten().count(),
            images: self.recipes.len(),
            resident: self.resident.len(),
            resident_bytes: self.resident_bytes(),
        }
    }
}

/// How a formula fits a room some cells wide.
#[derive(Clone, Copy, Debug)]
struct Fit {
    /// Pixels in an em of the formula at its own size.
    em: f32,
    /// Ems of ink the room holds at that size.
    room: f64,
    /// The least of its size the formula may shrink to: by a little at most,
    /// and never below the smallest size the maths size setting offers.
    least: f32,
}

impl Fit {
    /// The fit of a formula set at `scale_percent` in `width` cells of `canvas`.
    /// Typewriter text in a formula stands in cells
    /// [`ratex::typewriter_advance`] of an em wide; the em is the one that
    /// makes those the terminal's own cells, so code in a formula is the size
    /// of the code around it.
    fn new(canvas: Canvas, width: u16, scale_percent: u16) -> Self {
        let cell_width = f32::from(canvas.cell.width);
        let em = cell_width / ratex::typewriter_advance() as f32 * f32::from(scale_percent) / 100.0;
        let smallest = f32::from(UI::MIN_MATH_SIZE_PERCENT) / f32::from(scale_percent);
        let least = 1.0 - f32::from(UI::MAX_MATH_SHRINK_PERCENT) / 100.0;
        Self {
            em,
            room: f64::from(f32::from(width) * cell_width / em - 2.0 * PADDING / EM),
            least: smallest.max(least).min(1.0),
        }
    }
}

/// How a vector picture lands on the cell grid: its scale and the cells it
/// covers.
#[derive(Clone)]
struct Raster {
    /// Pixels to one of the picture's units.
    scale: f32,
    top: f32,
    /// Where the picture's top edge lands within its rows, in pixels.
    offset: f32,
    /// Which of its rows its baseline runs through, the one a neighbour lines
    /// up with.
    baseline: u16,
    cells: Size,
    cell: Size,
}

impl Raster {
    fn fitted(
        formula: &Formula,
        canvas: Canvas,
        width: u16,
        placement: Placement,
        fit: Fit,
    ) -> Self {
        let tree = &formula.tree;
        // Font ascender/descender metrics can reserve more space than the ink,
        // particularly for delimiters. Only visible bounds determine occupied rows.
        let bounds = tree.root().abs_stroke_bounding_box();
        let top = bounds.top() - PADDING;
        let height = bounds.height() + 2.0 * PADDING;
        let cell_width = f32::from(canvas.cell.width);
        let cell_height = f32::from(canvas.cell.height);
        let natural = fit.em / EM;
        let across = tree.size().width();
        // A picture keeps its size wherever breaking brings it inside, so text
        // reads at one size throughout the feed. Only what no break shortens
        // shrinks, as little as it may; what still passes the edge is cut off.
        let shrink = (f32::from(width) * cell_width / (across * natural)).clamp(fit.least, 1.0);
        let scale = natural * shrink;
        let cover = |extent: f32, cell: f32| ((extent * scale / cell).ceil() as u16).max(1);
        let above = (formula.baseline * tree.size().height() - top) * scale;
        let below = (height * scale - above).max(0.0);
        let padding = PADDING * scale;
        let text_baseline = TypesetSizing::BASELINE_PER_CELL_HEIGHT * cell_height;
        // The rows ink needs past its own, the padding being there to spend.
        let rows_past = |reach: f32, room: f32| {
            ((reach - 2.0 * padding - room) / cell_height)
                .ceil()
                .max(0.0) as u16
        };
        let covered = cover(height, cell_height);
        let (rows, baseline) = match placement {
            _ if covered == 1 => (1, 0),
            Placement::Alone => (covered, (covered - 1) / 2),
            Placement::AmongText => {
                let rows_above = rows_past(above, text_baseline);
                let rows_below = rows_past(below, cell_height - text_baseline);
                (rows_above + 1 + rows_below, rows_above)
            }
        };
        let rows_height = f32::from(rows) * cell_height;
        let offset = if placement == Placement::Alone && covered > 1 {
            (rows_height - height * scale) / 2.0
        } else {
            // On the text baseline where the ink leaves room for it, else as close
            // as it can sit without its ink being cut off.
            (f32::from(baseline) * cell_height + text_baseline)
                .min(rows_height - below)
                .max(above - padding)
                - above
        };
        Self {
            scale,
            top,
            offset,
            baseline,
            cells: Size::new(cover(across, cell_width).min(width), rows),
            cell: canvas.cell,
        }
    }

    /// The picture drawn across exactly its cells, as PNG; what passes them is
    /// cut away.
    fn png(&self, tree: &Tree) -> Option<Vec<u8>> {
        let mut pixmap = Pixmap::new(
            u32::from(self.cells.width) * u32::from(self.cell.width),
            u32::from(self.cells.height) * u32::from(self.cell.height),
        )?;
        let transform = Transform::from_scale(self.scale, self.scale)
            .post_translate(0.0, self.offset - self.top * self.scale);
        resvg::render(tree, transform, &mut pixmap.as_mut());
        pixmap.encode_png().ok()
    }
}
