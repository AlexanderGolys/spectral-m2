//! The terminal's own monospace font for typewriter maths, drawn in the cells
//! KaTeX lays out for its Typewriter face so the spacing of a formula is kept.

use std::env;
use std::fmt::Write;
use std::fs;
use std::path::PathBuf;
use std::sync::LazyLock;

use fontdb::{Database, Family, Query};
use ttf_parser::{Face, OutlineBuilder};

/// Where each terminal that shows pictures keeps its configuration, under the
/// configuration directory, and the key naming its font there.
const KITTY_CONFIG: &str = "kitty/kitty.conf";
const KITTY_FONT: &str = "font_family";
/// The `family=` field of kitty's structured font specification.
const KITTY_FAMILY_FIELD: &str = "family=";
const GHOSTTY_CONFIGS: [&str; 2] = ["ghostty/config", "ghostty/config.ghostty"];
const GHOSTTY_FONT: &str = "font-family";
/// Font names that leave the choice to the system rather than naming a family.
const GENERIC_FAMILIES: [&str; 2] = ["auto", "monospace"];

/// An installed font, loaded once, that typewriter glyphs are drawn from.
pub struct TerminalFont {
    data: Vec<u8>,
    index: u32,
}

impl TerminalFont {
    /// The font the terminal draws text in, when its configuration names an
    /// installed family.
    pub fn get() -> Option<&'static Self> {
        static FONT: LazyLock<Option<TerminalFont>> =
            LazyLock::new(|| TerminalFont::load(Family::Name(&configured_family()?)));
        FONT.as_ref()
    }

    /// The installed face of `family`, when there is one.
    pub fn load(family: Family) -> Option<Self> {
        let mut database = Database::new();
        database.load_system_fonts();
        let id = database.query(&Query {
            families: &[family],
            ..Query::default()
        })?;
        let (data, index) = database.with_face_data(id, |data, index| (data.to_vec(), index))?;
        Face::parse(&data, index).ok()?;
        Some(Self { data, index })
    }

    /// SVG path data for `character` with its baseline origin at (`x`, `y`) and
    /// `em` units to the em, centred in a cell `advance` ems wide and shrunk
    /// evenly when its own advance is wider, so it keeps its shape; none when
    /// the font has no outline for it.
    pub fn outline(
        &self,
        character: char,
        x: f64,
        y: f64,
        em: f64,
        advance: f64,
    ) -> Option<String> {
        let face = Face::parse(&self.data, self.index).ok()?;
        let glyph = face.glyph_index(character)?;
        let natural = em / f64::from(face.units_per_em());
        let own = face
            .glyph_hor_advance(glyph)
            .filter(|&own| own > 0)
            .map(f64::from);
        let cell = em * advance;
        let scale = own.map_or(natural, |own| natural.min(cell / own));
        let inset = own.map_or(0.0, |own| (cell - own * scale) / 2.0);
        let mut path = SvgPath {
            data: String::new(),
            origin: (x + inset, y),
            scale,
        };
        face.outline_glyph(glyph, &mut path)?;
        Some(path.data)
    }
}

/// Glyph outline commands written as SVG path data, from font units with y up
/// to SVG units with y down.
struct SvgPath {
    data: String,
    origin: (f64, f64),
    scale: f64,
}

impl SvgPath {
    fn point(&mut self, x: f32, y: f32) {
        let (left, baseline) = self.origin;
        let _ = write!(
            self.data,
            " {:.3} {:.3}",
            left + f64::from(x) * self.scale,
            baseline - f64::from(y) * self.scale
        );
    }
}

impl OutlineBuilder for SvgPath {
    fn move_to(&mut self, x: f32, y: f32) {
        self.data.push('M');
        self.point(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.data.push('L');
        self.point(x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.data.push('Q');
        self.point(x1, y1);
        self.point(x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.data.push('C');
        self.point(x1, y1);
        self.point(x2, y2);
        self.point(x, y);
    }

    fn close(&mut self) {
        self.data.push('Z');
    }
}

/// The font family the running terminal's configuration names.
fn configured_family() -> Option<String> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    let read = |name: &str| fs::read_to_string(config.join(name)).ok();
    let family = match super::terminal()? {
        "kitty" => kitty_family(&read(KITTY_CONFIG)?),
        "ghostty" => GHOSTTY_CONFIGS
            .iter()
            .find_map(|name| ghostty_family(&read(name)?)),
        _ => None,
    }?;
    (!GENERIC_FAMILIES.contains(&family.as_str())).then_some(family)
}

/// The last `font_family` kitty reads, plain or as a `family="…"` specification.
fn kitty_family(config: &str) -> Option<String> {
    let value = config
        .lines()
        .filter_map(|line| line.trim().strip_prefix(KITTY_FONT))
        .rfind(|rest| rest.starts_with(char::is_whitespace))?
        .trim();
    match value.split_once(KITTY_FAMILY_FIELD) {
        Some((_, field)) => match field.strip_prefix(['"', '\'']) {
            Some(quoted) => quoted.split(['"', '\'']).next(),
            None => field.split_whitespace().next(),
        }
        .map(str::to_string),
        None => Some(value.to_string()),
    }
}

/// The primary `font-family` Ghostty reads: the first after the last empty one,
/// which clears the list.
fn ghostty_family(config: &str) -> Option<String> {
    config
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == GHOSTTY_FONT).then(|| value.trim().trim_matches('"').to_string())
        })
        .fold(None, |primary, family| match family.is_empty() {
            true => None,
            false => primary.or(Some(family)),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kitty_names_its_font_plainly_or_by_specification() {
        assert_eq!(
            kitty_family("font_family Fira Code\nfont_family JetBrains Mono\n").as_deref(),
            Some("JetBrains Mono")
        );
        assert_eq!(
            kitty_family(r#"font_family family="Iosevka Term" style=Medium"#).as_deref(),
            Some("Iosevka Term")
        );
        assert_eq!(kitty_family("font_size 12\nfont_family_extra x"), None);
    }

    #[test]
    fn ghostty_takes_its_primary_family_after_any_reset() {
        assert_eq!(
            ghostty_family("font-family = \"A\"\nfont-family = B\n").as_deref(),
            Some("A")
        );
        assert_eq!(
            ghostty_family("font-family = A\nfont-family = \"\"\nfont-family = C").as_deref(),
            Some("C")
        );
        assert_eq!(ghostty_family("font-size = 12"), None);
    }
}
