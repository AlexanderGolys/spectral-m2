//! TeX set as vector graphics by RaTeX: KaTeX's layout and fonts, in pure Rust.

use std::fmt::Write;

use ratex_font::{get_char_metrics, FontId};
use ratex_layout::{layout, to_display_list, validate_layout, LayoutOptions};
use ratex_parser::parser::parse;
use ratex_svg::{render_to_svg, SvgOptions};
use ratex_types::color::Color;
use ratex_types::display_item::{DisplayItem, DisplayList};
use ratex_types::math_style::MathStyle;
use ratex_types::path_command::PathCommand;
use thiserror::Error;

use super::typewriter::TerminalFont;

/// SVG user units in one em of a typeset formula.
pub const EM: f32 = 10.0;
/// Space around visible formula ink, in SVG units, to protect antialiased edges.
pub const PADDING: f32 = EM * 0.1;
/// Where an SVG document ends, before which further shapes are drawn on top.
const SVG_END: &str = "</svg>";
/// Separators after which a formula's rows may break, as a list breaks after its commas.
const BREAK_AFTER: [char; 1] = [','];
/// How far, in ems, an item may start before a separator ends and still follow it.
const BREAK_TOLERANCE: f64 = 1e-6;
/// The percentage of row spacing that TeX's `\arraystretch` calls one.
const PERCENT_PER_ARRAYSTRETCH: u16 = 100;

/// A formula set as a picture, with the places its rows may break.
pub struct Typeset {
    pub svg: String,
    /// Places the picture may be cut into rows, left to right.
    pub breaks: Vec<Break>,
    /// How far down the picture the text baseline runs, as a share of its height.
    pub baseline: f32,
}

/// A place between a separator and what follows it where one row of a formula
/// may end and the next begin, each as a share of the picture's width, with
/// room kept around the ink on both sides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Break {
    /// Where the row before it ends: just past the separator.
    pub end: f32,
    /// Where the row after it starts: just before the next item drawn.
    pub start: f32,
}

impl Break {
    /// The break at `width` units across the picture.
    pub fn across(self, width: f32) -> Self {
        Self {
            end: self.end * width,
            start: self.start * width,
        }
    }
}

/// TeX that RaTeX cannot set, with its reason.
#[derive(Debug, Error)]
#[error("RaTeX cannot set it: {0}")]
pub struct Unset(String);

/// A colour in eight-bit channels, as a formula is painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb(pub [u8; 3]);

/// `tex`, in display style and painted `colour`, as an SVG of [`EM`] units per em.
#[cfg(test)]
pub fn svg(tex: &str, Rgb([r, g, b]): Rgb) -> Result<String, Unset> {
    svg_colored(tex, tex, Rgb([r, g, b]), PERCENT_PER_ARRAYSTRETCH).map(|typeset| typeset.svg)
}

/// Paint a formula from coloured TeX while retaining its original spacing and geometry.
/// Colour scopes can change atom classes in the layout engine, so only their paint
/// is transferred; the unmodified formula remains responsible for every position.
pub fn svg_colored(
    tex: &str,
    painted: &str,
    colour: Rgb,
    matrix_row_spacing_percent: u16,
) -> Result<Typeset, Unset> {
    let mut original = display(tex, colour, matrix_row_spacing_percent)?;
    if tex != painted {
        if let Ok(paint) = display(painted, colour, matrix_row_spacing_percent) {
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
    let svg = SvgOptions {
        font_size: f64::from(EM),
        padding: f64::from(PADDING),
        // Only unfilled paths are stroked; a stroke on the rest thickens every glyph.
        stroke_width: 0.0,
        embed_glyphs: true,
        font_dir: String::new(),
    };
    let breaks = breaks(&original);
    let baseline = (original.height as f32 * EM + PADDING)
        / ((original.height + original.depth) as f32 * EM + 2.0 * PADDING);
    let typewriter = TerminalFont::get().map(|font| typewriter_paths(&mut original, font));
    let mut svg = render_to_svg(&original, &svg);
    if let Some(paths) = typewriter.filter(|paths| !paths.is_empty()) {
        let end = svg.rfind(SVG_END).unwrap_or(svg.len());
        svg.insert_str(end, &paths);
    }
    Ok(Typeset {
        svg,
        breaks,
        baseline,
    })
}

/// Where the formula may be cut into rows: after each separator set in the
/// maths font, when nothing drawn reaches across from it to what follows, as a
/// fraction bar or a radical would.
fn breaks(list: &DisplayList) -> Vec<Break> {
    let separator_font = FontId::MainRegular.as_str();
    let extent = |item: &DisplayItem| match item {
        DisplayItem::GlyphPath { x, .. } => (*x, *x),
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
    };
    let extents = list.items.iter().map(extent).collect::<Vec<_>>();
    let em = f64::from(EM);
    let padding = f64::from(PADDING);
    let width = list.width * em + 2.0 * padding;
    let mut breaks = list
        .items
        .iter()
        .filter_map(|item| {
            let DisplayItem::GlyphPath {
                x,
                scale,
                font,
                char_code,
                ..
            } = item
            else {
                return None;
            };
            let separator = char::from_u32(*char_code).is_some_and(|c| BREAK_AFTER.contains(&c));
            if !separator || font != separator_font {
                return None;
            }
            let end = x + get_char_metrics(FontId::MainRegular, *char_code)?.width * scale;
            let start = extents
                .iter()
                .map(|(left, _)| *left)
                .filter(|left| *left >= end - BREAK_TOLERANCE)
                .fold(f64::INFINITY, f64::min);
            let reached_across = extents
                .iter()
                .any(|(left, right)| *left < end - BREAK_TOLERANCE && *right > start);
            (start.is_finite() && !reached_across).then(|| Break {
                end: ((end * em + 2.0 * padding) / width) as f32,
                start: (start * em / width) as f32,
            })
        })
        .collect::<Vec<_>>();
    breaks.sort_by(|a, b| a.end.total_cmp(&b.end));
    breaks
}

/// Take the typewriter glyphs the terminal font can draw out of `list`, and
/// return them drawn from it as SVG paths in the cells KaTeX laid out for them.
fn typewriter_paths(list: &mut DisplayList, font: &TerminalFont) -> String {
    let typewriter = FontId::TypewriterRegular.as_str();
    let em = f64::from(EM);
    let padding = f64::from(PADDING);
    let mut paths = String::new();
    list.items.retain(|item| {
        let DisplayItem::GlyphPath {
            x,
            y,
            scale,
            font: face,
            char_code,
            color,
        } = item
        else {
            return true;
        };
        let outline = (face == typewriter)
            .then(|| {
                let character = char::from_u32(*char_code)?;
                let advance = get_char_metrics(FontId::TypewriterRegular, *char_code)?.width;
                font.outline(
                    character,
                    x * em + padding,
                    y * em + padding,
                    scale * em,
                    advance,
                )
            })
            .flatten();
        let Some(outline) = outline else {
            return true;
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
        false
    });
    paths
}

/// Lay out the source without modifying its mathematical atom classes.
fn display(
    tex: &str,
    Rgb([r, g, b]): Rgb,
    matrix_row_spacing_percent: u16,
) -> Result<DisplayList, Unset> {
    let channel = |value: u8| f32::from(value) / f32::from(u8::MAX);
    let arraystretch = f64::from(matrix_row_spacing_percent) / f64::from(PERCENT_PER_ARRAYSTRETCH);
    let stretched = format!(r"\def\arraystretch{{{arraystretch}}}{tex}");
    let ast = parse(&stretched).map_err(|error| Unset(error.to_string()))?;
    let options = LayoutOptions::default()
        .with_style(MathStyle::Display)
        .with_color(Color::rgb(channel(r), channel(g), channel(b)));
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

#[cfg(test)]
mod tests {
    use super::*;

    const INK: Rgb = Rgb([0x12, 0x34, 0x56]);

    #[test]
    fn ratex_sets_what_macaulay2_writes() {
        for tex in [
            r"\frac{{\mathbb Z}}{101}\mathopen{}\left[a\,{.}{.}\,d\right]",
            r"\texttt{ideal}{}\left(x^{2},\,y\,z,\,z^{3}-x\right)",
            r"\left(\!\begin{array}{cc} x&y^{2}\\ \frac{1}{2}&z \end{array}\!\right)",
            r"\underset{\vphantom{\Bigg|}0}{R^{1}}\,\xleftarrow{\scriptsize \begin{bmatrix} x&y&z\end{bmatrix}}\,\underset{\vphantom{\Bigg|}1}{R^{3}}",
        ] {
            let svg = svg(tex, INK).unwrap();
            assert!(svg.starts_with("<svg"), "{tex}");
        }
    }

    #[test]
    fn theme_colors_preserve_every_glyph_and_its_geometry() {
        let groups = crate::ui::controller::settings::theme::groups::Groups::default();
        let colors = regex::Regex::new(r"rgba?\([^)]*\)").unwrap();
        for tex in [
            r"x+2=y",
            r"x^{2}+\frac{1}{3}",
            r"\mathbb{Q}\left[x,y\right]",
            r"{\mathbb R}_{53}\left[x_2\right]",
            r"{\mathcal O}_{\texttt{Proj}\left({\mathbb R}_{6}\mathopen{}\left[x_{1},\,y_{\left\{\left\{1\right\}\right\}},\,w\right]\right)}^{5}\left(-2\right)",
            r"\left(\begin{matrix}x&2\\3&y\end{matrix}\right)",
            r"\texttt{MdCode}\left\{\texttt{ring R}\right\},\,\texttt{ -- get the ring}",
        ] {
            let painted = crate::ui::render::maths::colored_tex(tex, &groups, Default::default());
            let original = svg(tex, INK).unwrap();
            let colored = svg_colored(tex, &painted, INK, PERCENT_PER_ARRAYSTRETCH)
                .unwrap()
                .svg;
            assert_ne!(
                original, colored,
                "colours were not transferred: {tex}: {painted}"
            );
            assert_eq!(
                colors.replace_all(&original, "PAINT"),
                colors.replace_all(&colored, "PAINT"),
                "{tex}"
            );
        }
    }

    #[test]
    fn matrix_rows_stand_further_apart_at_a_wider_spacing() {
        for tex in [
            r"\left(\!\begin{array}{cc}\frac{1}{2}&x\\\frac{3}{4}&y\end{array}\!\right)",
            r"\left(\begin{matrix}\frac{1}{2}&x\\\frac{3}{4}&y\end{matrix}\right)",
        ] {
            let height = |percent| {
                let list = display(tex, INK, percent).unwrap();
                list.height + list.depth
            };
            assert!(height(200) > height(100), "{tex}");
        }
    }

    #[test]
    fn ratex_paints_in_the_colour_asked() {
        let svg = svg("x", INK).unwrap();

        assert!(svg.contains("rgba(18,52,86"));
    }

    #[test]
    fn an_explicit_text_colour_is_painted_as_written() {
        let tex = r"\textcolor{#112233}{\texttt{Z}}";

        let typeset = svg_colored(tex, tex, INK, PERCENT_PER_ARRAYSTRETCH).unwrap();

        assert!(typeset.svg.contains("rgba(17,34,51"), "{}", typeset.svg);
    }

    #[test]
    fn a_collection_formula_may_break_after_its_commas() {
        for tex in [
            r"\left(1,\,2,\,3\right)",
            r"\left\langle\!\left|1,\,2,\,3\right|\!\right\rangle",
        ] {
            let typeset = svg_colored(tex, tex, INK, PERCENT_PER_ARRAYSTRETCH).unwrap();
            assert_eq!(typeset.breaks.len(), 2, "{tex}");
        }
    }

    #[test]
    fn a_glyph_outside_the_maths_fonts_is_not_set() {
        for tex in ["\u{1F600}", "\\texttt{\u{1F600}}", "1+\\text{\u{4E2D}}"] {
            assert!(svg(tex, INK).is_err(), "{tex}");
        }
    }

    #[test]
    fn the_maths_fonts_set_every_kind_of_glyph_m2_asks_for() {
        for tex in [
            r"\sqrt{x}+\sqrt[3]{\frac{1}{2}}",
            r"\left\langle\!\left|\begin{pmatrix}1&2\\3&4\\5&6\end{pmatrix},\,1\right|\!\right\rangle",
            r"\mathbb{Z}\mathfrak{g}\mathcal{O}\mathbf{i}\mathrm{G}\alpha\Gamma\infty\pi",
            r"\texttt{abc}\textrm{ of }\Longrightarrow\leftarrow\otimes\oplus\cdot\ldots\to",
            r"\sum_{i=0}^{n}\int_{0}^{1}\prod_{j}\bigoplus_{k}\widetilde{x}\overline{y}\hat{z}",
        ] {
            assert!(svg(tex, INK).is_ok(), "{tex}");
        }
    }

    #[test]
    fn typewriter_glyphs_are_drawn_from_another_font_in_their_cells() {
        let Some(font) = TerminalFont::load(fontdb::Family::Monospace) else {
            return;
        };
        let mut list = display(r"\texttt{ideal}\,x", INK, PERCENT_PER_ARRAYSTRETCH).unwrap();
        let glyphs = list.items.len();
        let paths = typewriter_paths(&mut list, &font);
        assert_eq!(paths.matches("<path").count(), "ideal".len());
        assert_eq!(list.items.len(), glyphs - "ideal".len(), "x stays KaTeX's");
        assert!(paths.contains("fill=\"rgb(18,52,86)\""), "{paths}");
    }
}
