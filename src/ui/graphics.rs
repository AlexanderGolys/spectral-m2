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
use std::env;
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
use std::ops::Range;

use resvg::tiny_skia::{Pixmap, PixmapPaint, Transform};
use resvg::usvg::{Options, Tree};

use crate::ui::config::UI;
use crate::ui::debug::mebibytes;
use kitty::{ImageId, KittyImage, Passthrough, Piece};
use ratex::{Break, Rgb, EM, PADDING};

// @##magic
const TMUX: &str = "tmux";
/// Set in every process tmux runs.
const TMUX_VARIABLE: &str = "TMUX";
/// Lets this pane's escape sequences through tmux to the terminal.
const TMUX_ALLOW_PASSTHROUGH: [&str; 4] = ["set", "-p", "allow-passthrough", "on"];
/// tmux's names for the terminal its client runs in, one a line: as the terminal
/// reports itself, then its `TERM`.
const TMUX_CLIENT_TERMINAL: &str = "#{client_termtype}\n#{client_termname}";
/// Variables naming the terminal outside tmux, most specific first.
const TERMINAL_VARIABLES: [&str; 2] = ["TERM_PROGRAM", "TERM"];
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
    let canvas = match env::var_os(TMUX_VARIABLE).is_some() {
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
    let _ = Command::new(TMUX)
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
        let names = match env::var_os(TMUX_VARIABLE).is_some() {
            true => tmux_client_names(),
            false => TERMINAL_VARIABLES
                .iter()
                .filter_map(|variable| env::var(variable).ok())
                .collect(),
        };
        names.iter().find_map(|name| printable_name(name))
    });
    TERMINAL.as_deref()
}

fn tmux_client_names() -> Vec<String> {
    Command::new(TMUX)
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

/// A formula to draw: its TeX, that TeX with theme colour scopes, the M2 code
/// a copy of its picture gives, and how far apart its matrix rows stand.
#[derive(Clone, Copy, Debug)]
pub struct Typesetting<'a> {
    pub tex: &'a str,
    pub painted: &'a str,
    pub code: &'a str,
    pub matrix_row_spacing_percent: u16,
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
/// copying. Returns placeholder lines no wider than `width`, or none when pictures
/// are unavailable or the formula cannot be set legibly in that width.
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

/// A formula as set in one colour at one matrix row spacing.
type FormulaKey = (String, Rgb, u16);
/// A formula's picture at a column count and cell pixel size, for its copied code.
/// Values sharing TeX but not code each get their own picture to copy from.
type ImageKey = (FormulaKey, u16, Size, String, Placement);

/// A formula's vector picture and the places it may be cut into rows.
struct Formula {
    tree: Tree,
    breaks: Vec<Break>,
    /// Where the text baseline runs down the picture, as a share of its height.
    baseline: f32,
}

/// Every picture made, kept so a layout at another width or a resend reuses it. Only
/// what draws a picture again is kept, never its pixels: those are made when the
/// terminal is sent them, and the terminal holds no more than its budget.
struct Gallery {
    /// Each formula's vector picture, or none when RaTeX refused it.
    formulas: HashMap<FormulaKey, Option<Arc<Formula>>>,
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
            tex,
            painted,
            code,
            matrix_row_spacing_percent,
        }: Typesetting,
        colour: Rgb,
        canvas: Canvas,
        width: u16,
        placement: Placement,
    ) -> Option<(Arc<KittyImage>, usize)> {
        let key = (painted.to_string(), colour, matrix_row_spacing_percent);
        let formula = self
            .formulas
            .entry(key.clone())
            .or_insert_with(|| {
                let typeset =
                    ratex::svg_colored(tex, painted, colour, matrix_row_spacing_percent).ok()?;
                let tree = Tree::from_str(&typeset.svg, &Options::default()).ok()?;
                Some(Arc::new(Formula {
                    tree,
                    breaks: typeset.breaks,
                    baseline: typeset.baseline,
                }))
            })
            .clone()?;
        let raster = Raster::fitted(&formula, canvas, width, placement)?;
        let baseline = raster.baseline_row();
        let image_key = (
            key,
            raster.cells.width,
            canvas.cell,
            code.to_string(),
            placement,
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
            formulas: self.formulas.values().flatten().count(),
            images: self.recipes.len(),
            resident: self.resident.len(),
            resident_bytes: self.resident_bytes(),
        }
    }
}

/// How a vector picture lands on the cell grid: its scale, the slices of it
/// stacked as rows, and the cells it covers.
#[derive(Clone)]
struct Raster {
    scale: f32,
    top: f32,
    /// Where the picture's top edge lands within a row, in pixels.
    offset: f32,
    /// The picture's slices, left to right, each drawn below the one before.
    rows: Vec<Range<f32>>,
    /// Terminal rows each slice stands in.
    row_cells: u16,
    /// Which of a slice's rows its baseline runs through.
    slice_baseline: u16,
    cells: Size,
    cell: Size,
}

impl Raster {
    /// The picture at the terminal's text size, broken into rows of at most
    /// `width` columns where it may break, and shrunk to `width` when a row it
    /// cannot break is wider; none when shrinking would make it too small to read.
    fn fitted(formula: &Formula, canvas: Canvas, width: u16, placement: Placement) -> Option<Self> {
        let tree = &formula.tree;
        // Font ascender/descender metrics can reserve more space than the ink,
        // particularly for delimiters. Only visible bounds determine occupied rows.
        let bounds = tree.root().abs_stroke_bounding_box();
        let top = bounds.top() - PADDING;
        let height = bounds.height() + 2.0 * PADDING;
        let cell_width = f32::from(canvas.cell.width);
        let cell_height = f32::from(canvas.cell.height);
        let natural = cell_height * UI.typeset.em_per_cell_height / EM;
        let room = f32::from(width) * cell_width / natural;
        let across = tree.size().width();
        let breaks = formula
            .breaks
            .iter()
            .map(|cut| cut.across(across))
            .collect::<Vec<_>>();
        let rows = wrapped(across, &breaks, room);
        let widest = rows
            .iter()
            .map(|row| row.end - row.start)
            .fold(0.0, f32::max);
        let shrink = (room / widest).min(1.0);
        if shrink < UI.typeset.min_scale {
            return None;
        }
        let scale = natural * shrink;
        let cover = |extent: f32, cell: f32| ((extent * scale / cell).ceil() as u16).max(1);
        let above = (formula.baseline * tree.size().height() - top) * scale;
        let below = (height * scale - above).max(0.0);
        let padding = PADDING * scale;
        let text_baseline = UI.typeset.baseline_per_cell_height * cell_height;
        // The rows ink needs past its own, the padding being there to spend.
        let rows_past = |reach: f32, room: f32| {
            ((reach - 2.0 * padding - room) / cell_height)
                .ceil()
                .max(0.0) as u16
        };
        let covered = cover(height, cell_height);
        let (row_cells, slice_baseline) = match placement {
            _ if covered == 1 => (1, 0),
            Placement::Alone => (covered, (covered - 1) / 2),
            Placement::AmongText => {
                let rows_above = rows_past(above, text_baseline);
                let rows_below = rows_past(below, cell_height - text_baseline);
                (rows_above + 1 + rows_below, rows_above)
            }
        };
        let row_height = f32::from(row_cells) * cell_height;
        let offset = if placement == Placement::Alone && covered > 1 {
            (row_height - height * scale) / 2.0
        } else {
            // On the text baseline where the ink leaves room for it, else as close
            // as it can sit without its ink being cut off.
            (f32::from(slice_baseline) * cell_height + text_baseline)
                .min(row_height - below)
                .max(above - padding)
                - above
        };
        Some(Self {
            scale,
            top,
            offset,
            cells: Size::new(
                cover(widest, cell_width).min(width),
                row_cells * rows.len() as u16,
            ),
            rows,
            row_cells,
            slice_baseline,
            cell: canvas.cell,
        })
    }

    /// The row a neighbour lines up with: the baseline's in a picture of one slice,
    /// the middle one of several.
    fn baseline_row(&self) -> usize {
        match self.rows.len() {
            1 => usize::from(self.slice_baseline),
            _ => usize::from(self.cells.height.saturating_sub(1) / 2),
        }
    }

    /// The picture's rows drawn across exactly its cells, as PNG.
    fn png(&self, tree: &Tree) -> Option<Vec<u8>> {
        let width = u32::from(self.cells.width) * u32::from(self.cell.width);
        let row_height = u32::from(self.row_cells) * u32::from(self.cell.height);
        let mut pixmap = Pixmap::new(width, row_height * self.rows.len() as u32)?;
        for (index, row) in self.rows.iter().enumerate() {
            // A slice as wide as its row, so what follows its end is cut away.
            let slice = (((row.end - row.start) * self.scale).ceil() as u32).clamp(1, width);
            let mut strip = Pixmap::new(slice, row_height)?;
            let transform = Transform::from_scale(self.scale, self.scale)
                .post_translate(-row.start * self.scale, self.offset - self.top * self.scale);
            resvg::render(tree, transform, &mut strip.as_mut());
            pixmap.draw_pixmap(
                0,
                (row_height * index as u32) as i32,
                strip.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            );
        }
        pixmap.encode_png().ok()
    }
}

/// The slices of a picture `width` units wide, each at most `room` wide where
/// `breaks` allow: every row takes the last break that still fits, and a row
/// with none that fits runs on to the first it can.
fn wrapped(width: f32, breaks: &[Break], room: f32) -> Vec<Range<f32>> {
    let mut rows = Vec::new();
    let mut start = 0.0;
    while width - start > room {
        let ahead = || breaks.iter().filter(|cut| cut.start > start);
        let Some(cut) = ahead()
            .take_while(|cut| cut.end - start <= room)
            .last()
            .or_else(|| ahead().next())
        else {
            break;
        };
        rows.push(start..cut.end);
        start = cut.start;
    }
    rows.push(start..width);
    rows
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use resvg::tiny_skia::PremultipliedColorU8;

    use super::*;

    const CANVAS: Canvas = Canvas {
        cell: Size::new(10, 20),
        passthrough: Passthrough::Direct,
    };

    #[test]
    fn terminals_are_named_without_versions_or_families() {
        assert_eq!(printable_name("ghostty 1.3.1").as_deref(), Some("ghostty"));
        assert_eq!(printable_name("kitty(0.36.4)").as_deref(), Some("kitty"));
        assert_eq!(printable_name("xterm-ghostty").as_deref(), Some("ghostty"));
        assert_eq!(printable_name("WezTerm").as_deref(), Some("wezterm"));
        assert_eq!(printable_name("xterm-256color"), None);
        assert_eq!(printable_name(""), None);
    }

    #[test]
    fn a_type_annotation_of_words_and_rings_is_set() {
        let annotation = r"\textrm{Matrix}\ R^{1}\ \textrm{<--}\ \mathbb{Q}[x,y]^{2}";
        assert!(ratex::svg(annotation, Rgb([255, 255, 255])).is_ok());
    }

    fn formula(tex: &str) -> Formula {
        let typeset = ratex::svg_colored(
            tex,
            tex,
            Rgb([255, 255, 255]),
            UI.matrix_row_spacing_percent,
        )
        .unwrap();
        Formula {
            tree: Tree::from_str(&typeset.svg, &Options::default()).unwrap(),
            breaks: typeset.breaks,
            baseline: typeset.baseline,
        }
    }

    #[test]
    fn ordinary_delimiters_do_not_add_a_terminal_row() {
        for tex in [
            "1",
            "(1)",
            "[1]",
            r"\left(1\right)",
            r"\left[1\right]",
            r"\left\{1\right\}",
        ] {
            let formula = formula(tex);
            let raster = Raster::fitted(&formula, CANVAS, 80, Placement::AmongText).unwrap();
            assert_eq!(
                raster.cells.height,
                1,
                "{tex}: SVG height {}, scale {}",
                formula.tree.size().height(),
                raster.scale
            );
        }
    }

    #[test]
    fn one_row_formulas_rest_their_ink_on_the_text_baseline() {
        let cell = Size::new(20, 44);
        let canvas = Canvas { cell, ..CANVAS };
        let baseline = UI.typeset.baseline_per_cell_height * f32::from(cell.height);
        for tex in ["a", "c", "ab", "ac", "1", "x", r"a^{2}", r"a^{2}b"] {
            let formula = formula(tex);
            let raster = Raster::fitted(&formula, canvas, 80, Placement::AmongText).unwrap();
            assert_eq!(raster.cells.height, 1, "{tex}");
            let pixmap = Pixmap::decode_png(&raster.png(&formula.tree).unwrap()).unwrap();
            let last_inked_row = pixmap
                .pixels()
                .chunks(pixmap.width() as usize)
                .rposition(|row| row.iter().any(|pixel| pixel.alpha() > u8::MAX / 2))
                .unwrap();
            let below_baseline = last_inked_row as f32 + 1.0 - baseline;
            assert!(below_baseline.abs() <= 1.5, "{tex}: {below_baseline}");
        }
    }

    #[test]
    fn a_tall_formula_beside_text_stands_around_the_row_of_its_baseline() {
        let cell = Size::new(20, 44);
        let canvas = Canvas { cell, ..CANVAS };
        let formula = formula(r"\frac{1}{2}");
        let raster = Raster::fitted(&formula, canvas, 80, Placement::AmongText).unwrap();
        let pixmap = Pixmap::decode_png(&raster.png(&formula.tree).unwrap()).unwrap();
        let inked = |row: &[PremultipliedColorU8]| row.iter().any(|pixel| pixel.alpha() > 0);
        let row_of = |pixel_row: usize| pixel_row / usize::from(cell.height);
        let mut rows = pixmap.pixels().chunks(pixmap.width() as usize);
        let first = rows.clone().position(inked).unwrap();
        let last = rows.rposition(inked).unwrap();
        let baseline = raster.baseline_row();
        assert!(
            row_of(first) < baseline,
            "numerator above: {first}, {baseline}"
        );
        assert!(
            row_of(last) > baseline,
            "denominator below: {last}, {baseline}"
        );
        assert_eq!(usize::from(raster.cells.height), baseline * 2 + 1);

        let alone = Raster::fitted(&formula, canvas, 80, Placement::Alone).unwrap();
        assert!(alone.cells.height < raster.cells.height);
    }

    #[test]
    fn a_picture_covers_whole_cells_and_fills_them_exactly() {
        let formula = formula(r"\frac{x+y}{x-z}");

        let raster = Raster::fitted(&formula, CANVAS, 80, Placement::AmongText).unwrap();
        let png = raster.png(&formula.tree).unwrap();

        let pixmap = Pixmap::decode_png(&png).unwrap();
        assert_eq!(pixmap.width(), u32::from(raster.cells.width) * 10);
        assert_eq!(pixmap.height(), u32::from(raster.cells.height) * 20);
        assert!(
            raster.cells.height > 1,
            "a fraction stands taller than a row"
        );

        let enlarged = Canvas {
            cell: Size::new(CANVAS.cell.width * 2, CANVAS.cell.height * 2),
            ..CANVAS
        };
        let zoomed = Raster::fitted(&formula, enlarged, 80, Placement::AmongText).unwrap();
        let zoomed = Pixmap::decode_png(&zoomed.png(&formula.tree).unwrap()).unwrap();
        assert_eq!(zoomed.width(), pixmap.width() * 2);
        assert_eq!(zoomed.height(), pixmap.height() * 2);
    }

    #[test]
    fn a_wide_formula_shrinks_to_the_width_until_it_would_be_unreadable() {
        let formula = formula(r"1\,267\,650\,600\,228\,229\,401\,496\,703\,205\,376");
        let natural = Raster::fitted(&formula, CANVAS, u16::MAX, Placement::AmongText)
            .unwrap()
            .cells
            .width;

        let shrunk = Raster::fitted(&formula, CANVAS, natural - 1, Placement::AmongText).unwrap();
        let refused = Raster::fitted(&formula, CANVAS, 1, Placement::AmongText);

        assert!(shrunk.cells.width < natural);
        assert!(refused.is_none());
    }

    #[test]
    fn a_list_of_maths_and_typewriter_words_in_their_own_colours_is_one_picture() {
        let mut gallery = Gallery::default();
        let tex = concat!(
            r"\left\{\left\{\frac{x}{3}\right\},\,",
            r"\left\{\textcolor{#83c092}{\texttt{Z}},\,\frac{1}{2}\right\},\,",
            r"\left\{1,\,\textcolor{#d699b6}{\texttt{1}}\right\}\right\}"
        );
        let formula = Typesetting {
            tex,
            painted: tex,
            code: "{{x/3}, {Z, 1/2}, {1, \"1\"}}",
            matrix_row_spacing_percent: UI.matrix_row_spacing_percent,
        };

        let picture = gallery.formula(formula, Rgb([1, 2, 3]), CANVAS, 80, Placement::Alone);

        assert!(picture.is_some());
    }

    #[test]
    fn a_long_list_wraps_after_its_commas_at_full_size() {
        let entries = (1..=60).map(|n| n.to_string()).collect::<Vec<_>>();
        let formula = formula(&format!(r"\left\{{{}\right\}}", entries.join(r",\,")));
        let unbroken = Raster::fitted(&formula, CANVAS, u16::MAX, Placement::AmongText).unwrap();
        let width = unbroken.cells.width / 3;

        let wrapped = Raster::fitted(&formula, CANVAS, width, Placement::AmongText).unwrap();

        assert_eq!(unbroken.rows.len(), 1);
        assert!(wrapped.rows.len() >= 3, "{} rows", wrapped.rows.len());
        assert_eq!(wrapped.scale, unbroken.scale, "wrapping never shrinks");
        assert!(wrapped.cells.width <= width);
        assert_eq!(
            wrapped.cells.height,
            unbroken.cells.height * wrapped.rows.len() as u16
        );
    }

    #[test]
    fn a_fraction_bar_is_never_cut() {
        let formula = formula(r"\frac{1,\,2,\,3,\,4,\,5,\,6,\,7,\,8}{9}");
        assert!(formula.breaks.is_empty());
    }

    #[test]
    fn another_matrix_row_spacing_makes_another_picture() {
        let mut gallery = GALLERY.lock().unwrap();
        let tex = r"\begin{array}{c}x\\y\end{array}";
        let formula = |matrix_row_spacing_percent| Typesetting {
            tex,
            painted: tex,
            code: tex,
            matrix_row_spacing_percent,
        };
        let ink = Rgb([1, 2, 3]);
        let placement = Placement::Alone;

        let tight = gallery.formula(formula(100), ink, CANVAS, 80, placement);
        let spaced = gallery.formula(formula(200), ink, CANVAS, 80, placement);

        assert!(!Arc::ptr_eq(&tight.unwrap().0, &spaced.unwrap().0));
    }

    #[test]
    fn a_formula_is_made_once_whatever_the_wider_width() {
        let mut gallery = GALLERY.lock().unwrap();
        let made = gallery.recipes.len();
        let ink = Rgb([1, 2, 3]);

        let square = Typesetting {
            tex: "x^2",
            painted: r"\textcolor{red}{x^2}",
            code: "x^2",
            matrix_row_spacing_percent: UI.matrix_row_spacing_percent,
        };

        let placement = Placement::AmongText;
        let first = gallery.formula(square, ink, CANVAS, 80, placement);
        let again = gallery.formula(square, ink, CANVAS, 120, placement);

        assert!(Arc::ptr_eq(&first.unwrap().0, &again.unwrap().0));
        assert_eq!(gallery.recipes.len() - made, 1);
        assert_eq!(
            gallery.sources.get(&gallery.last_id).map(String::as_str),
            Some(square.code)
        );

        let enlarged = Canvas {
            cell: Size::new(CANVAS.cell.width * 2, CANVAS.cell.height * 2),
            ..CANVAS
        };
        let (original, _) = gallery.formula(square, ink, CANVAS, 80, placement).unwrap();
        let (zoomed, _) = gallery
            .formula(square, ink, enlarged, 80, placement)
            .unwrap();
        assert!(!Arc::ptr_eq(&original, &zoomed));
        assert_eq!(original.lines().len(), zoomed.lines().len());
        assert_eq!(original.lines()[0].width(), zoomed.lines()[0].width());
        assert_eq!(gallery.recipes.len() - made, 2);

        let (restored, _) = gallery.formula(square, ink, CANVAS, 80, placement).unwrap();
        assert!(Arc::ptr_eq(&original, &restored));
        assert_eq!(gallery.recipes.len() - made, 2);
    }

    fn picture(gallery: &mut Gallery, tex: &str) -> ImageId {
        let formula = Typesetting {
            tex,
            painted: tex,
            code: tex,
            matrix_row_spacing_percent: UI.matrix_row_spacing_percent,
        };
        let (image, _) = gallery
            .formula(formula, Rgb([1, 2, 3]), CANVAS, 80, Placement::Alone)
            .unwrap();
        image.id()
    }

    fn shown(ids: &[ImageId]) -> HashSet<ImageId> {
        ids.iter().copied().collect()
    }

    /// Frames showing `ids`, the pictures off screen standing `near` the viewport as
    /// a feed reports them afresh each frame, until the worker has encoded all asked.
    fn settled_near(gallery: &mut Gallery, ids: &[ImageId], near: &[(ImageId, usize)]) -> String {
        let mut frame = |gallery: &mut Gallery| {
            gallery.distances = near.iter().copied().collect();
            gallery.synchronise(&shown(ids), Passthrough::Direct)
        };
        let mut sequences = frame(gallery);
        while !gallery.encoding.is_empty() {
            thread::sleep(Duration::from_millis(1));
            sequences.push_str(&frame(gallery));
        }
        sequences
    }

    fn settled(gallery: &mut Gallery, ids: &[ImageId]) -> String {
        settled_near(gallery, ids, &[])
    }
    #[test]
    fn a_picture_is_queued_for_encoding_when_first_shown_and_sent_once_encoded() {
        let mut gallery = Gallery::default();
        let id = picture(&mut gallery, "x^2");

        assert_eq!(gallery.synchronise(&shown(&[]), Passthrough::Direct), "");
        gallery.synchronise(&shown(&[id]), Passthrough::Direct);
        assert!(gallery.encoding.contains(&id));
        assert!(!gallery.resident.contains_key(&id));
        let sent = settled(&mut gallery, &[id]);
        let again = gallery.synchronise(&shown(&[id]), Passthrough::Direct);

        assert!(sent.contains("a=T"));
        assert_eq!(again, "");
        assert_eq!(gallery.usage().resident, 1);
        assert!(gallery.usage().resident_bytes > 0);
    }

    #[test]
    fn an_encoded_picture_is_sent_without_a_frame_of_its_own() {
        let mut gallery = Gallery::default();
        let id = picture(&mut gallery, "x^2");
        gallery.synchronise(&shown(&[id]), Passthrough::Direct);

        let mut sent = String::new();
        while sent.is_empty() {
            thread::sleep(Duration::from_millis(1));
            sent = gallery.send_encoded(Passthrough::Direct);
        }

        assert!(sent.contains("a=T"));
        assert!(gallery.encoding.is_empty());
        assert!(gallery.resident.contains_key(&id));
    }

    #[test]
    fn pictures_off_screen_stay_in_the_terminal_while_their_block_holds_them() {
        let mut gallery = Gallery::default();
        let id = picture(&mut gallery, "x^2");
        settled(&mut gallery, &[id]);
        gallery.distances = HashMap::from([(id, 5)]);

        let sequences = gallery.synchronise(&shown(&[]), Passthrough::Direct);

        assert_eq!(sequences, "");
        assert_eq!(gallery.usage().resident, 1);
    }

    #[test]
    fn a_picture_no_block_holds_is_deleted_at_once() {
        let mut gallery = Gallery::default();
        let id = picture(&mut gallery, "x^2");
        settled(&mut gallery, &[id]);

        let sequences = gallery.synchronise(&shown(&[]), Passthrough::Direct);

        assert!(sequences.contains(&id.deletion(Passthrough::Direct)));
        assert_eq!(gallery.usage().resident, 0);
    }

    #[test]
    fn at_the_same_distance_the_longest_unseen_picture_is_deleted_first_over_budget() {
        let mut gallery = Gallery::default();
        let old = picture(&mut gallery, "x^2");
        let recent = picture(&mut gallery, "y^3");
        let current = picture(&mut gallery, "z^4");
        let near = [(old, 5), (recent, 5)];
        settled_near(&mut gallery, &[old], &near);
        settled_near(&mut gallery, &[recent], &near);
        gallery.budget = gallery.resident_bytes();

        let sequences = settled_near(&mut gallery, &[current], &near);

        assert!(sequences.contains(&old.deletion(Passthrough::Direct)));
        assert!(!sequences.contains(&current.deletion(Passthrough::Direct)));
        assert!(gallery.resident.contains_key(&current));
        assert!(gallery.resident.contains_key(&recent));
        assert!(!gallery.resident.contains_key(&old));
    }

    #[test]
    fn the_picture_furthest_from_the_viewport_goes_before_one_seen_longer_ago() {
        let mut gallery = Gallery::default();
        let near = picture(&mut gallery, "x^2");
        let far = picture(&mut gallery, "y^3");
        let current = picture(&mut gallery, "z^4");
        let distances = [(near, 3), (far, 90)];
        settled_near(&mut gallery, &[near], &distances);
        settled_near(&mut gallery, &[far], &distances);
        gallery.budget = gallery.resident_bytes();

        settled_near(&mut gallery, &[current], &distances);

        assert!(!gallery.resident.contains_key(&far));
        assert!(gallery.resident.contains_key(&near));
    }

    #[test]
    fn a_picture_on_screen_is_never_deleted_for_the_budget() {
        let mut gallery = Gallery::default();
        let id = picture(&mut gallery, "x^2");
        gallery.budget = 0;

        let sequences = settled(&mut gallery, &[id]);

        assert!(sequences.contains("a=T"));
        assert!(!sequences.contains("a=d"));
        assert!(gallery.resident.contains_key(&id));
    }

    #[test]
    fn a_deleted_picture_is_sent_again_when_it_comes_back_into_view() {
        let mut gallery = Gallery::default();
        let id = picture(&mut gallery, "x^2");
        let other = picture(&mut gallery, "y^3");
        settled(&mut gallery, &[id]);
        gallery.budget = 0;
        settled(&mut gallery, &[other]);
        assert!(!gallery.resident.contains_key(&id));

        let sequences = settled(&mut gallery, &[id]);

        assert!(sequences.contains("a=T"));
        assert!(gallery.resident.contains_key(&id));
    }

    #[test]
    fn resending_sends_what_is_on_screen_again_and_deletes_the_rest() {
        let mut gallery = Gallery::default();
        let visible = picture(&mut gallery, "x^2");
        let hidden = picture(&mut gallery, "y^3");
        settled(&mut gallery, &[visible, hidden]);
        gallery.resend = true;

        let sequences = settled(&mut gallery, &[visible]);

        assert!(sequences.contains("a=T"));
        assert!(sequences.contains(&hidden.deletion(Passthrough::Direct)));
        assert!(!sequences.contains(&visible.deletion(Passthrough::Direct)));
        assert_eq!(gallery.resident.len(), 1);
        assert!(!gallery.resend);
    }
}
