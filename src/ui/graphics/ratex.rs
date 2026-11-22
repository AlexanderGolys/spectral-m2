//! TeX set as vector graphics by RaTeX: KaTeX's layout and fonts, in pure Rust.

use std::borrow::Cow;
use std::fmt::Write;

use ratex_font::{get_char_metrics, FontId};
use ratex_layout::{layout, to_display_list, validate_layout, LayoutOptions};
use ratex_parser::parser::parse;
use ratex_svg::{render_to_svg, SvgOptions};
use ratex_types::color::Color;
use ratex_types::display_item::{DisplayItem, DisplayList};
use ratex_types::math_style::MathStyle;
use thiserror::Error;

use super::typewriter::TerminalFont;

pub mod diagram;
mod layout;

pub use layout::Set;

/// SVG user units in one em of a typeset formula.
pub const EM: f32 = 10.0;
/// Space around visible formula ink, in SVG units, to protect antialiased edges.
pub const PADDING: f32 = EM * 0.1;
/// Where an SVG document ends, before which further shapes are drawn on top.
const SVG_END: &str = "</svg>";
/// Points of height a break mark's rule gains with each rank: too little to
/// show or to move anything, enough to read the rank back from.
const MARK_RANK_POINTS: f64 = 0.001;
/// TeX points in an em of a formula set at RaTeX's size.
const POINTS_PER_EM: f64 = 10.0;
/// How far apart, in ems, two places across a formula still count as one.
const TOLERANCE: f64 = 1e-6;
/// The character whose advance every glyph of the typewriter face shares.
const TYPEWRITER_SAMPLE: char = 'M';
/// How RaTeX begins its message about a command it does not know.
const UNDEFINED_COMMAND: &str = "Undefined control sequence: ";
/// How many commands run into the letters after them one formula is parted for.
const MAX_PARTINGS: usize = 32;

/// The percentage of row spacing that TeX's `\arraystretch` calls one.
pub const PERCENT_PER_ARRAYSTRETCH: u16 = 100;

/// What a picture is set from.
///
/// `QQ[x]` is a formula. A hash table is rows between braces, each a key, an
/// arrow and a value:
/// `Fenced { left: '{', right: '}', ink, body: Rows(vec![vec![key, arrow, value]]) }`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// TeX, or a diagram's description as [`diagram::Description`] writes it:
    /// one row of ink, which breaks at its marks. `painted` is the same with
    /// the theme's colour scopes.
    Formula { tex: String, painted: String },
    /// Rows set one under another, each breaking on its own: every cell but a
    /// row's last in a column shared with the rows it fits beside, the last
    /// running on.
    Rows(Vec<Vec<Source>>),
    /// A body between fences grown to its height and painted in `ink`.
    Fenced {
        left: char,
        right: char,
        ink: Option<Rgb>,
        body: Box<Source>,
    },
}

/// A formula laid out and set as a picture.
pub struct Picture {
    pub svg: String,
    /// How far down the picture its first line's baseline runs, as a share of
    /// its height.
    pub baseline: f32,
}

/// TeX marking a place a row of ink may break, as its writer chose it: a rule
/// of no width, which RaTeX keeps as a line nothing shows of. A row breaks at
/// its marks of the lowest rank first, so the rank is how deep the mark lies:
/// 0 between a list's items, 1 between the terms of one of them.
pub fn break_mark(rank: usize) -> String {
    format!(r"\rule{{0pt}}{{{}pt}}", rank as f64 * MARK_RANK_POINTS)
}

/// The TeX of a delimiter a fence is drawn as.
pub fn delimiter(fence: char) -> String {
    match fence {
        '{' => r"\{".into(),
        '}' => r"\}".into(),
        '⟨' => r"\langle".into(),
        '⟩' => r"\rangle".into(),
        _ => fence.to_string(),
    }
}

/// Ems across `tex` set on its own, when RaTeX sets it.
pub fn width(tex: &str) -> Option<f64> {
    // Its ink is of no matter to how wide it is.
    let ink = Rgb([0; 3]);
    display(tex, ink).ok().map(|list| list.width)
}

/// Ems across one cell of typewriter text in a formula.
pub fn typewriter_advance() -> f64 {
    get_char_metrics(FontId::TypewriterRegular, u32::from(TYPEWRITER_SAMPLE))
        .expect("the typewriter face has its letters")
        .width
}

/// TeX that RaTeX cannot set, with its reason.
#[derive(Debug, Error)]
#[error("RaTeX cannot set it: {0}")]
pub struct Unset(String);

/// A colour in eight-bit channels, as a formula is painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
    /// `tex` painted in this colour.
    pub fn scope(self, tex: &str) -> String {
        let Self([r, g, b]) = self;
        format!(r"{{\color{{#{r:02x}{g:02x}{b:02x}}}{tex}}}")
    }
}

impl From<Rgb> for Color {
    fn from(Rgb(channels): Rgb) -> Self {
        let [r, g, b] = channels.map(|value| f32::from(value) / f32::from(u8::MAX));
        Color::rgb(r, g, b)
    }
}

/// Paint a formula from coloured TeX while retaining its original spacing and geometry.
/// Colour scopes can change atom classes in the layout engine, so only their paint
/// is transferred; the unmodified formula remains responsible for every position.
fn set(tex: &str, painted: &str, colour: Rgb) -> Result<DisplayList, Unset> {
    let mut original = match display(tex, colour) {
        Ok(original) => original,
        Err(unset) => {
            let parted_tex = parted(tex);
            if parted_tex == tex {
                return Err(unset);
            }
            return set(&parted_tex, &parted(painted), colour);
        }
    };
    if tex != painted {
        if let Ok(paint) = display(painted, colour) {
            if original.items.len() == paint.items.len()
                && original
                    .items
                    .iter()
                    .zip(&paint.items)
                    .all(|(original, painted)| compatible(original, painted))
            {
                for (original, mut painted) in original.items.iter_mut().zip(paint.items) {
                    *item_color(original) = *item_color(&mut painted);
                }
            }
        }
    }
    Ok(original)
}

/// A laid out formula as an SVG, with its baseline.
fn pictured(mut list: DisplayList) -> Picture {
    let svg = SvgOptions {
        font_size: f64::from(EM),
        padding: f64::from(PADDING),
        // Only unfilled paths are stroked; a stroke on the rest thickens every glyph.
        stroke_width: 0.0,
        embed_glyphs: true,
        font_dir: String::new(),
    };
    let baseline = (list.height as f32 * EM + PADDING)
        / ((list.height + list.depth) as f32 * EM + 2.0 * PADDING);
    let typewriter = TerminalFont::get().map(|font| typewriter_paths(&mut list, font));
    let mut svg = render_to_svg(&list, &svg);
    if let Some(paths) = typewriter.filter(|paths| !paths.is_empty()) {
        let end = svg.rfind(SVG_END).unwrap_or(svg.len());
        svg.insert_str(end, &paths);
    }
    Picture { svg, baseline }
}

/// Take the typewriter glyphs the terminal font can draw out of `list`, and
/// return them drawn from it as SVG paths in the cells KaTeX laid out for them.
fn typewriter_paths(list: &mut DisplayList, font: &TerminalFont) -> String {
    let typewriter = FontId::TypewriterRegular.as_str();
    let em = f64::from(EM);
    let padding = f64::from(PADDING);
    let mut paths = String::new();
    let mut items = std::mem::take(&mut list.items).into_iter().peekable();
    while let Some(item) = items.next() {
        let DisplayItem::GlyphPath {
            x,
            y,
            scale,
            font: face,
            char_code,
            color,
        } = &item
        else {
            list.items.push(item);
            continue;
        };
        let metrics = (face == typewriter)
            .then(|| {
                Some((
                    char::from_u32(*char_code)?,
                    get_char_metrics(FontId::TypewriterRegular, *char_code)?.width,
                ))
            })
            .flatten();
        let Some((character, advance)) = metrics else {
            list.items.push(item);
            continue;
        };
        let mut text = character.to_string();
        let mut width = advance * scale;
        let mut run = Vec::new();
        while let Some(DisplayItem::GlyphPath {
            x: next_x,
            y: next_y,
            scale: next_scale,
            font: next_face,
            char_code: next_code,
            color: next_color,
        }) = items.peek()
        {
            if next_face != face
                || next_y != y
                || next_scale != scale
                || next_color != color
                || (next_x - (x + width)).abs() > TOLERANCE
            {
                break;
            }
            let Some((character, advance)) = char::from_u32(*next_code).zip(
                get_char_metrics(FontId::TypewriterRegular, *next_code)
                    .map(|metrics| metrics.width),
            ) else {
                break;
            };
            text.push(character);
            width += advance * scale;
            run.push(items.next().expect("the next glyph was inspected"));
        }
        let outline = font.outline(
            &text,
            x * em + padding,
            y * em + padding,
            scale * em,
            width / scale,
        );
        let Some(outline) = outline else {
            list.items.push(item);
            list.items.extend(run);
            continue;
        };
        let channel = |value: f32| (value.clamp(0.0, 1.0) * f32::from(u8::MAX)).round();
        let _ = write!(
            paths,
            r#"<path d="{outline}" fill="rgb({},{},{})" fill-opacity="{}"/>"#,
            channel(color.r),
            channel(color.g),
            channel(color.b),
            color.a,
        );
    }
    paths
}

/// Lay out the source without modifying its mathematical atom classes.
/// `tex` with each command run into the letters after it parted from them, as
/// M2's `tex` writes `\midX` for `\mid X`: the longest known command the
/// unknown name begins with is set apart. Other faults are left for parsing to
/// report.
fn parted(tex: &str) -> Cow<'_, str> {
    let mut source = Cow::Borrowed(tex);
    for _ in 0..MAX_PARTINGS {
        let Err(error) = parse(&source) else {
            break;
        };
        let Some(unknown) = error.message.strip_prefix(UNDEFINED_COMMAND) else {
            break;
        };
        let Some(name) = unknown.strip_prefix('\\') else {
            break;
        };
        let known = (1..name.len())
            .rev()
            .filter(|end| name.is_char_boundary(*end))
            .find(|end| is_command(&name[..*end]));
        let at = error
            .loc
            .map(|loc| loc.start)
            .filter(|at| source[*at..].starts_with(unknown))
            .or_else(|| source.find(unknown));
        let (Some(known), Some(at)) = (known, at) else {
            break;
        };
        let apart = format!("\\{} {}", &name[..known], &name[known..]);
        source
            .to_mut()
            .replace_range(at..at + unknown.len(), &apart);
    }
    source
}

/// Whether RaTeX knows `\name`, whatever arguments it goes on to want.
fn is_command(name: &str) -> bool {
    parse(&format!("\\{name}"))
        .err()
        .is_none_or(|error| !error.message.starts_with(UNDEFINED_COMMAND))
}

fn display(tex: &str, colour: Rgb) -> Result<DisplayList, Unset> {
    let ast = parse(tex).map_err(|error| Unset(error.to_string()))?;
    let options = LayoutOptions::default()
        .with_style(MathStyle::Display)
        .with_color(colour.into());
    let laid = layout(&ast, &options);
    validate_layout(&laid).map_err(Unset)?;
    let list = to_display_list(&laid);
    match list.items.iter().find_map(foreign_glyph) {
        Some(glyph) => Err(Unset(format!("no maths glyph for {glyph:?}"))),
        None => Ok(list),
    }
}

/// The character `item` draws when the maths fonts lack it, as an emoji's: set as a
/// picture it would come from whichever system font could be found, so it is left to
/// the terminal, which draws such characters itself.
fn foreign_glyph(item: &DisplayItem) -> Option<char> {
    let DisplayItem::GlyphPath {
        font, char_code, ..
    } = item
    else {
        return None;
    };
    FontId::parse(font)
        .and_then(|font| get_char_metrics(font, *char_code))
        .is_none()
        .then(|| char::from_u32(*char_code).unwrap_or(char::REPLACEMENT_CHARACTER))
}

fn compatible(original: &DisplayItem, painted: &DisplayItem) -> bool {
    match (original, painted) {
        (
            DisplayItem::GlyphPath {
                font: a,
                char_code: ac,
                ..
            },
            DisplayItem::GlyphPath {
                font: b,
                char_code: bc,
                ..
            },
        ) => a == b && ac == bc,
        (DisplayItem::Line { .. }, DisplayItem::Line { .. })
        | (DisplayItem::Rect { .. }, DisplayItem::Rect { .. })
        | (DisplayItem::Path { .. }, DisplayItem::Path { .. }) => true,
        _ => false,
    }
}

fn item_color(item: &mut DisplayItem) -> &mut Color {
    match item {
        DisplayItem::GlyphPath { color, .. }
        | DisplayItem::Line { color, .. }
        | DisplayItem::Rect { color, .. }
        | DisplayItem::Path { color, .. } => color,
    }
}
