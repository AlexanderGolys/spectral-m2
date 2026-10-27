//! Pictures in the transcript, drawn through the terminal's graphics protocol.
//!
//! What the terminal can draw is learned once, as it starts, and read from here as
//! the scheme is: it is a fact about the terminal, not state any element owns. A
//! picture is made while the value it shows is laid out, kept for every later
//! layout, and sent to the terminal after the frame that first places it.

pub mod kitty;
pub mod typeset;
mod typewriter;

use std::collections::HashMap;
use std::env;
use std::process::{Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock};

use crossterm::terminal::window_size;
use ratatui::layout::Size;
use ratatui::text::Line;
use ratatui_image::picker::{Picker, ProtocolType};
use std::ops::Range;

use resvg::tiny_skia::{Pixmap, PixmapPaint, Transform};
use resvg::usvg::{Options, Tree};

use crate::ui::config::UI;
use kitty::{ImageId, KittyImage, Passthrough};
use typeset::{Break, Rgb, EM, PADDING};

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

/// A terminal that shows pictures: the pixels in one of its cells, and how escape
/// sequences reach it.
#[derive(Clone, Copy, Debug)]
struct Canvas {
    cell: Size,
    passthrough: Passthrough,
}

/// Set only when the terminal answered that it shows kitty pictures.
static CANVAS: OnceLock<Canvas> = OnceLock::new();

static GALLERY: LazyLock<Mutex<Gallery>> = LazyLock::new(|| {
    Mutex::new(Gallery {
        formulas: HashMap::new(),
        images: HashMap::new(),
        // Starting at random keeps two shells drawing into one terminal, as tmux
        // panes do, from storing pictures under each other's ids.
        last_id: ImageId::from(rand::random::<u32>()),
        unsent: Vec::new(),
        sources: HashMap::new(),
    })
});

/// Learn what the terminal can draw. Runs on the alternate screen before any event
/// is read, since a terminal asked directly answers as input.
pub fn detect() {
    let canvas = match env::var_os(TMUX_VARIABLE).is_some() {
        true => tmux_canvas(),
        false => queried_canvas(),
    };
    if let Some(canvas) = canvas {
        let _ = CANVAS.set(canvas);
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
pub fn available() -> bool {
    CANVAS.get().is_some()
}

/// A formula to draw: its TeX, that TeX with theme colour scopes, and the M2 code
/// a copy of its picture gives.
#[derive(Clone, Copy, Debug)]
pub struct Typesetting<'a> {
    pub tex: &'a str,
    pub painted: &'a str,
    pub code: &'a str,
}

/// Typeset `formula` in `colour`, retaining its code for copying. Returns
/// placeholder lines no wider than `width`, or none when pictures are unavailable
/// or the formula cannot be set legibly in that width.
pub fn typeset(formula: Typesetting, colour: Rgb, width: u16) -> Option<Vec<Line<'static>>> {
    let canvas = *CANVAS.get()?;
    gallery()
        .formula(formula, colour, canvas, width)
        .map(|image| image.lines())
}

/// The M2 code the picture `id` shows.
pub fn source(id: ImageId) -> Option<String> {
    gallery().sources.get(&id).cloned()
}

/// Escape sequences storing every picture made since the last call.
pub fn unsent() -> String {
    let Some(canvas) = CANVAS.get() else {
        return String::new();
    };
    gallery()
        .unsent
        .drain(..)
        .map(|image| image.transmission(canvas.passthrough))
        .collect()
}

/// Send every picture again through tmux, which drops those sent while its pane
/// is hidden; called each time the pane regains focus.
pub fn resend_all() {
    if !CANVAS
        .get()
        .is_some_and(|canvas| canvas.passthrough == Passthrough::Tmux)
    {
        return;
    }
    let mut gallery = gallery();
    let images = gallery
        .images
        .values()
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    gallery.unsent = images;
}

fn gallery() -> MutexGuard<'static, Gallery> {
    GALLERY
        .lock()
        .expect("the gallery lock is never held across a panic")
}

/// A formula as set in one colour.
type FormulaKey = (String, Rgb);
/// A formula's picture at a number of columns, for the code it copies as: values
/// sharing TeX but not code each get their own picture to copy from.
type ImageKey = (FormulaKey, u16, String);

/// A formula's vector picture and the places it may be cut into rows.
struct Formula {
    tree: Tree,
    breaks: Vec<Break>,
}

/// Every picture made, kept so a layout at another width or a resend reuses it.
struct Gallery {
    /// Each formula's vector picture, or none when RaTeX refused it.
    formulas: HashMap<FormulaKey, Option<Arc<Formula>>>,
    /// Each formula rasterised to a number of columns.
    images: HashMap<ImageKey, Option<Arc<KittyImage>>>,
    last_id: ImageId,
    unsent: Vec<Arc<KittyImage>>,
    /// The M2 code each picture shows, so copying it gives back what M2 reads.
    sources: HashMap<ImageId, String>,
}

impl Gallery {
    fn formula(
        &mut self,
        Typesetting { tex, painted, code }: Typesetting,
        colour: Rgb,
        canvas: Canvas,
        width: u16,
    ) -> Option<Arc<KittyImage>> {
        let key = (painted.to_string(), colour);
        let formula = self
            .formulas
            .entry(key.clone())
            .or_insert_with(|| {
                let typeset = typeset::svg_colored(tex, painted, colour).ok()?;
                let tree = Tree::from_str(&typeset.svg, &Options::default()).ok()?;
                Some(Arc::new(Formula {
                    tree,
                    breaks: typeset.breaks,
                }))
            })
            .clone()?;
        let raster = Raster::fitted(&formula, canvas, width)?;
        let image_key = (key, raster.cells.width, code.to_string());
        if let Some(image) = self.images.get(&image_key) {
            return image.clone();
        }
        self.last_id = self.last_id.next();
        let image = raster
            .png(&formula.tree)
            .and_then(|png| KittyImage::new(self.last_id, raster.cells, png).ok())
            .map(Arc::new);
        self.unsent.extend(image.clone());
        if image.is_some() {
            self.sources.insert(self.last_id, code.to_string());
        }
        self.images.insert(image_key, image.clone());
        image
    }
}

/// How a vector picture lands on the cell grid: its scale, the slices of it
/// stacked as rows, and the cells it covers.
struct Raster {
    scale: f32,
    top: f32,
    height: f32,
    /// The picture's slices, left to right, each drawn below the one before.
    rows: Vec<Range<f32>>,
    /// Terminal rows each slice stands in.
    row_cells: u16,
    cells: Size,
    cell: Size,
}

impl Raster {
    /// The picture at the terminal's text size, broken into rows of at most
    /// `width` columns where it may break, and shrunk to `width` when a row it
    /// cannot break is wider; none when shrinking would make it too small to read.
    fn fitted(formula: &Formula, canvas: Canvas, width: u16) -> Option<Self> {
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
        let row_cells = cover(height, cell_height);
        Some(Self {
            scale,
            top,
            height,
            cells: Size::new(
                cover(widest, cell_width).min(width),
                row_cells * rows.len() as u16,
            ),
            rows,
            row_cells,
            cell: canvas.cell,
        })
    }

    /// The picture's rows drawn across exactly its cells, each centred down its
    /// own, as PNG.
    fn png(&self, tree: &Tree) -> Option<Vec<u8>> {
        let width = u32::from(self.cells.width) * u32::from(self.cell.width);
        let row_height = u32::from(self.row_cells) * u32::from(self.cell.height);
        let mut pixmap = Pixmap::new(width, row_height * self.rows.len() as u32)?;
        let slack = row_height as f32 - self.height * self.scale;
        for (index, row) in self.rows.iter().enumerate() {
            // A slice as wide as its row, so what follows its end is cut away.
            let slice = (((row.end - row.start) * self.scale).ceil() as u32).clamp(1, width);
            let mut strip = Pixmap::new(slice, row_height)?;
            let transform = Transform::from_scale(self.scale, self.scale)
                .post_translate(-row.start * self.scale, slack / 2.0 - self.top * self.scale);
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
        assert!(typeset::svg(annotation, Rgb([255, 255, 255])).is_ok());
    }

    fn formula(tex: &str) -> Formula {
        let typeset = typeset::svg_colored(tex, tex, Rgb([255, 255, 255])).unwrap();
        Formula {
            tree: Tree::from_str(&typeset.svg, &Options::default()).unwrap(),
            breaks: typeset.breaks,
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
            let raster = Raster::fitted(&formula, CANVAS, 80).unwrap();
            assert_eq!(
                raster.cells.height,
                1,
                "{tex}: SVG height {}, ink height {}, scale {}",
                formula.tree.size().height(),
                raster.height,
                raster.scale
            );
        }
    }

    #[test]
    fn a_picture_covers_whole_cells_and_fills_them_exactly() {
        let formula = formula(r"\frac{x+y}{x-z}");

        let raster = Raster::fitted(&formula, CANVAS, 80).unwrap();
        let png = raster.png(&formula.tree).unwrap();

        let pixmap = Pixmap::decode_png(&png).unwrap();
        assert_eq!(pixmap.width(), u32::from(raster.cells.width) * 10);
        assert_eq!(pixmap.height(), u32::from(raster.cells.height) * 20);
        assert!(
            raster.cells.height > 1,
            "a fraction stands taller than a row"
        );
    }

    #[test]
    fn a_wide_formula_shrinks_to_the_width_until_it_would_be_unreadable() {
        let formula = formula(r"1\,267\,650\,600\,228\,229\,401\,496\,703\,205\,376");
        let natural = Raster::fitted(&formula, CANVAS, u16::MAX)
            .unwrap()
            .cells
            .width;

        let shrunk = Raster::fitted(&formula, CANVAS, natural - 1).unwrap();
        let refused = Raster::fitted(&formula, CANVAS, 1);

        assert!(shrunk.cells.width < natural);
        assert!(refused.is_none());
    }

    #[test]
    fn a_long_list_wraps_after_its_commas_at_full_size() {
        let entries = (1..=60).map(|n| n.to_string()).collect::<Vec<_>>();
        let formula = formula(&format!(r"\left\{{{}\right\}}", entries.join(r",\,")));
        let unbroken = Raster::fitted(&formula, CANVAS, u16::MAX).unwrap();
        let width = unbroken.cells.width / 3;

        let wrapped = Raster::fitted(&formula, CANVAS, width).unwrap();

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
    fn a_formula_is_made_once_and_sent_once_whatever_the_wider_width() {
        let mut gallery = GALLERY.lock().unwrap();
        gallery.unsent.clear();
        let ink = Rgb([1, 2, 3]);

        let square = Typesetting {
            tex: "x^2",
            painted: r"\textcolor{red}{x^2}",
            code: "x^2",
        };

        let first = gallery.formula(square, ink, CANVAS, 80);
        let again = gallery.formula(square, ink, CANVAS, 120);

        assert!(Arc::ptr_eq(&first.unwrap(), &again.unwrap()));
        assert_eq!(gallery.unsent.len(), 1);
        assert_eq!(
            gallery.sources.get(&gallery.last_id).map(String::as_str),
            Some(square.code)
        );
    }
}
