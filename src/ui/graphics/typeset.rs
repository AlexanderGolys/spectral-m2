//! TeX set as vector graphics by RaTeX: KaTeX's layout and fonts, in pure Rust.

use ratex_layout::{layout, to_display_list, validate_layout, LayoutOptions};
use ratex_parser::parser::parse;
use ratex_svg::{render_to_svg, SvgOptions};
use ratex_types::color::Color;
use ratex_types::math_style::MathStyle;
use thiserror::Error;

/// SVG user units in one em of a typeset formula.
pub const EM: f32 = 10.0;
/// Space kept around a set formula, in ems, so antialiased edges are not cut.
const MARGIN: f32 = 0.1;

/// TeX that RaTeX cannot set, with its reason.
#[derive(Debug, Error)]
#[error("RaTeX cannot set it: {0}")]
pub struct Unset(String);

/// A colour in eight-bit channels, as a formula is painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb(pub [u8; 3]);

/// `tex`, in display style and painted `colour`, as an SVG of [`EM`] units per em.
pub fn svg(tex: &str, Rgb([r, g, b]): Rgb) -> Result<String, Unset> {
    let channel = |value: u8| f32::from(value) / f32::from(u8::MAX);
    let ast = parse(tex).map_err(|error| Unset(error.to_string()))?;
    let options = LayoutOptions::default()
        .with_style(MathStyle::Display)
        .with_color(Color::rgb(channel(r), channel(g), channel(b)));
    let laid = layout(&ast, &options);
    validate_layout(&laid).map_err(Unset)?;
    let svg = SvgOptions {
        font_size: f64::from(EM),
        padding: f64::from(EM * MARGIN),
        // Only unfilled paths are stroked; a stroke on the rest thickens every glyph.
        stroke_width: 0.0,
        embed_glyphs: true,
        font_dir: String::new(),
    };
    Ok(render_to_svg(&to_display_list(&laid), &svg))
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
    fn ratex_paints_in_the_colour_asked() {
        let svg = svg("x", INK).unwrap();

        assert!(svg.contains("rgba(18,52,86"));
    }
}
