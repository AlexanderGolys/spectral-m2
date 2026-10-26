//! TeX set as vector graphics by RaTeX: KaTeX's layout and fonts, in pure Rust.

use ratex_layout::{layout, to_display_list, validate_layout, LayoutOptions};
use ratex_parser::parser::parse;
use ratex_svg::{render_to_svg, SvgOptions};
use ratex_types::color::Color;
use ratex_types::display_item::{DisplayItem, DisplayList};
use ratex_types::math_style::MathStyle;
use thiserror::Error;

/// SVG user units in one em of a typeset formula.
pub const EM: f32 = 10.0;
/// Space around visible formula ink, in SVG units, to protect antialiased edges.
pub const PADDING: f32 = EM * 0.1;

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
    svg_colored(tex, tex, Rgb([r, g, b]))
}

/// Paint a formula from coloured TeX while retaining its original spacing and geometry.
/// Colour scopes can change atom classes in the layout engine, so only their paint
/// is transferred; the unmodified formula remains responsible for every position.
pub fn svg_colored(tex: &str, painted: &str, colour: Rgb) -> Result<String, Unset> {
    let mut original = display(tex, colour)?;
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
    let svg = SvgOptions {
        font_size: f64::from(EM),
        padding: f64::from(PADDING),
        // Only unfilled paths are stroked; a stroke on the rest thickens every glyph.
        stroke_width: 0.0,
        embed_glyphs: true,
        font_dir: String::new(),
    };
    Ok(render_to_svg(&original, &svg))
}

/// Lay out the source without modifying its mathematical atom classes.
fn display(tex: &str, Rgb([r, g, b]): Rgb) -> Result<DisplayList, Unset> {
    let channel = |value: u8| f32::from(value) / f32::from(u8::MAX);
    let ast = parse(tex).map_err(|error| Unset(error.to_string()))?;
    let options = LayoutOptions::default()
        .with_style(MathStyle::Display)
        .with_color(Color::rgb(channel(r), channel(g), channel(b)));
    let laid = layout(&ast, &options);
    validate_layout(&laid).map_err(Unset)?;
    Ok(to_display_list(&laid))
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
        let colors = regex::Regex::new(r"rgba\([^)]*\)").unwrap();
        for tex in [
            r"x+2=y",
            r"x^{2}+\frac{1}{3}",
            r"\mathbb{Q}\left[x,y\right]",
            r"\left(\begin{matrix}x&2\\3&y\end{matrix}\right)",
        ] {
            let painted = crate::ui::render::maths::colored_tex(tex, &groups);
            let original = svg(tex, INK).unwrap();
            let colored = svg_colored(tex, &painted, INK).unwrap();
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
    fn ratex_paints_in_the_colour_asked() {
        let svg = svg("x", INK).unwrap();

        assert!(svg.contains("rgba(18,52,86"));
    }
}
