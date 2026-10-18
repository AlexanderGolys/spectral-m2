//! Fixed theme palettes and reusable RGB blending.

use std::collections::HashMap;

use ratatui::style::Color;
use serde::Deserialize;

/// The hue families supplied by every scheme.
pub const HUES: [&str; 8] = [
    "red", "green", "yellow", "blue", "purple", "aqua", "orange", "gray",
];
/// Number of foreground and background steps in a resolved palette.
pub const SURFACE_STEPS: usize = 5;
const SHADE_MIX: f32 = 0.25;
/// Fraction of the canvas retained by a tinted background.
pub const BACKGROUND_TINT: f32 = 0.95;
const FOREGROUND_SPREAD: f32 = 0.65;
const BACKGROUND_SPREAD: f32 = 0.20;

/// A baseline color and any native lighter or darker shades.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shades {
    base: Color,
    light: Option<Color>,
    dark: Option<Color>,
}

/// One hue's shades or the supplied anchors of a foreground/background ramp.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum ColorSet {
    Single(Color),
    Steps(Vec<Color>),
    Shades(Shades),
}

/// Blend RGB colors by the fraction of `toward`, rounding each channel once.
pub fn blend(from: Color, toward: Color, amount: f32) -> Result<Color, String> {
    if !amount.is_finite() || !(0.0..=1.0).contains(&amount) {
        return Err("blend amount must be between 0 and 1".into());
    }
    let (Color::Rgb(r, g, b), Color::Rgb(tr, tg, tb)) = (from, toward) else {
        return Err("palette blending requires RGB colors".into());
    };
    let channel =
        |a: u8, z: u8| (f32::from(a) + (f32::from(z) - f32::from(a)) * amount).round() as u8;
    Ok(Color::Rgb(channel(r, tr), channel(g, tg), channel(b, tb)))
}

fn brightness(color: Color) -> Result<u32, String> {
    let Color::Rgb(r, g, b) = color else {
        return Err("palettes require RGB colors".into());
    };
    Ok(299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b))
}

impl ColorSet {
    fn anchors(&self) -> Result<Vec<Color>, String> {
        match self {
            Self::Single(color) => Ok(vec![*color]),
            Self::Steps(colors) if (1..=SURFACE_STEPS).contains(&colors.len()) => {
                Ok(colors.clone())
            }
            _ => Err(format!(
                "foreground/background needs 1–{SURFACE_STEPS} colors"
            )),
        }
    }

    fn shades(&self) -> Result<(Color, Option<Color>, Option<Color>), String> {
        match self {
            Self::Single(color) => Ok((*color, None, None)),
            Self::Shades(shades) => Ok((shades.base, shades.light, shades.dark)),
            Self::Steps(colors) if (1..=3).contains(&colors.len()) => {
                Ok((colors[0], colors.get(1).copied(), colors.get(2).copied()))
            }
            _ => Err("a hue needs 1–3 colors (base, light, dark)".into()),
        }
    }
}

/// Fill a ramp by inserting midpoints into its widest RGB gaps, preserving anchors.
fn ramp(mut colors: Vec<Color>, opposite: Color, spread: f32) -> Result<Vec<Color>, String> {
    if colors.len() == 1 {
        colors.push(blend(colors[0], opposite, spread)?);
    }
    while colors.len() < SURFACE_STEPS {
        let distance = |pair: &[Color]| -> Result<u32, String> {
            let (Color::Rgb(r, g, b), Color::Rgb(tr, tg, tb)) = (pair[0], pair[1]) else {
                return Err("palette blending requires RGB colors".into());
            };
            Ok([(r, tr), (g, tg), (b, tb)]
                .into_iter()
                .map(|(a, z)| (i32::from(a) - i32::from(z)).unsigned_abs().pow(2))
                .sum())
        };
        let mut widest = (0, 0);
        for (index, pair) in colors.windows(2).enumerate() {
            let gap = distance(pair)?;
            if gap > widest.1 {
                widest = (index, gap);
            }
        }
        colors.insert(
            widest.0 + 1,
            blend(colors[widest.0], colors[widest.0 + 1], 0.5)?,
        );
    }
    Ok(colors)
}

/// Hue shades, five foregrounds, five backgrounds, and subtle hue-tinted backgrounds.
pub struct Palette(HashMap<String, Color>);

impl Palette {
    /// Resolve native colors and synthesize only the missing shades and ramp steps.
    pub fn build(sets: HashMap<String, &ColorSet>) -> Result<Self, String> {
        let get = |name: &str| {
            sets.get(name)
                .copied()
                .ok_or_else(|| format!("missing palette family `{name}`"))
        };
        if let Some(name) = sets
            .keys()
            .find(|name| !HUES.contains(&name.as_str()) && !["fg", "bg"].contains(&name.as_str()))
        {
            return Err(format!("unknown palette family `{name}`"));
        }
        let fg = get("fg")?.anchors()?;
        let bg = get("bg")?.anchors()?;
        let (default_fg, default_bg) = (fg[0], bg[0]);
        let (light, dark) = if brightness(default_fg)? >= brightness(default_bg)? {
            (default_fg, default_bg)
        } else {
            (default_bg, default_fg)
        };
        let mut colors = HashMap::new();
        for (name, values) in [
            ("fg", ramp(fg, default_bg, FOREGROUND_SPREAD)?),
            ("bg", ramp(bg, default_fg, BACKGROUND_SPREAD)?),
        ] {
            for (index, color) in values.into_iter().enumerate() {
                brightness(color)?;
                colors.insert(format!("{name}{index}"), color);
            }
        }
        for name in HUES {
            let (base, native_light, native_dark) = get(name)?.shades()?;
            brightness(base)?;
            let light = match native_light {
                Some(color) => color,
                None => blend(base, light, SHADE_MIX)?,
            };
            let dark = match native_dark {
                Some(color) => color,
                None => blend(base, dark, SHADE_MIX)?,
            };
            brightness(light)?;
            brightness(dark)?;
            colors.insert(
                format!("bg_{name}"),
                blend(base, default_bg, BACKGROUND_TINT)?,
            );
            colors.insert(name.into(), base);
            colors.insert(format!("{name}_light"), light);
            colors.insert(format!("{name}_dark"), dark);
        }
        Ok(Self(colors))
    }

    /// Resolve an optional serialized color name.
    pub fn resolve(&self, name: Option<&str>) -> Result<Option<Color>, String> {
        name.map(|name| {
            self.0
                .get(name)
                .copied()
                .ok_or_else(|| format!("unknown palette color `{name}`"))
        })
        .transpose()
    }

    /// Every resolved color in deterministic name order.
    #[cfg(test)]
    pub fn colors(&self) -> Vec<(&str, Color)> {
        let mut colors = self
            .0
            .iter()
            .map(|(name, &color)| (name.as_str(), color))
            .collect::<Vec<_>>();
        colors.sort_unstable_by_key(|&(name, _)| name);
        colors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: Color = Color::Rgb(0, 0, 0);
    const WHITE: Color = Color::Rgb(255, 255, 255);
    const RED: Color = Color::Rgb(255, 0, 0);

    fn sets() -> HashMap<String, ColorSet> {
        let mut sets = HUES
            .into_iter()
            .map(|name| (name.into(), ColorSet::Single(RED)))
            .collect::<HashMap<_, _>>();
        sets.insert("fg".into(), ColorSet::Single(WHITE));
        sets.insert("bg".into(), ColorSet::Single(BLACK));
        sets
    }

    fn build(sets: &HashMap<String, ColorSet>) -> Result<Palette, String> {
        Palette::build(sets.iter().map(|(name, set)| (name.clone(), set)).collect())
    }

    #[test]
    fn rgb_blending_preserves_endpoints_and_rounds_channels() {
        assert_eq!(blend(RED, BLACK, 0.0).unwrap(), RED);
        assert_eq!(blend(RED, BLACK, 1.0).unwrap(), BLACK);
        assert_eq!(blend(BLACK, WHITE, 0.5).unwrap(), Color::Rgb(128, 128, 128));
        assert_eq!(blend(RED, BLACK, 0.95).unwrap(), Color::Rgb(13, 0, 0));
        for amount in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
            assert!(blend(RED, BLACK, amount).is_err());
        }
        assert!(blend(Color::Reset, BLACK, 0.5).is_err());
    }

    #[test]
    fn interpolation_keeps_native_anchors_and_adds_midpoints() {
        assert_eq!(
            ramp(vec![BLACK, WHITE], RED, 0.2).unwrap(),
            vec![
                BLACK,
                Color::Rgb(64, 64, 64),
                Color::Rgb(128, 128, 128),
                Color::Rgb(192, 192, 192),
                WHITE
            ]
        );
        for native in [
            vec![BLACK, RED, WHITE],
            vec![BLACK, Color::Rgb(32, 32, 32), RED, WHITE],
            vec![BLACK, BLACK, RED, RED, WHITE],
        ] {
            let filled = ramp(native.clone(), RED, 0.2).unwrap();
            assert_eq!(filled.len(), SURFACE_STEPS);
            let mut remaining = filled.iter();
            for anchor in native {
                assert!(remaining.any(|color| *color == anchor));
            }
        }
    }

    #[test]
    fn missing_shades_and_tints_use_the_shared_blend() {
        let palette = build(&sets()).unwrap();
        assert_eq!(
            palette.resolve(Some("red_light")).unwrap(),
            Some(blend(RED, WHITE, SHADE_MIX).unwrap())
        );
        assert_eq!(
            palette.resolve(Some("red_dark")).unwrap(),
            Some(blend(RED, BLACK, SHADE_MIX).unwrap())
        );
        assert_eq!(
            palette.resolve(Some("bg_red")).unwrap(),
            Some(Color::Rgb(13, 0, 0))
        );
        assert_eq!(palette.colors().len(), HUES.len() * 4 + SURFACE_STEPS * 2);
    }

    #[test]
    fn explicit_shades_are_preserved_and_light_mode_uses_the_brighter_endpoint() {
        let mut colors = sets();
        colors.insert(
            "red".into(),
            ColorSet::Shades(Shades {
                base: RED,
                light: Some(Color::Rgb(255, 100, 100)),
                dark: None,
            }),
        );
        colors.insert("fg".into(), ColorSet::Single(BLACK));
        colors.insert("bg".into(), ColorSet::Single(WHITE));
        let palette = build(&colors).unwrap();
        assert_eq!(
            palette.resolve(Some("red_light")).unwrap(),
            Some(Color::Rgb(255, 100, 100))
        );
        assert_eq!(
            palette.resolve(Some("green_light")).unwrap(),
            Some(blend(RED, WHITE, SHADE_MIX).unwrap())
        );
        assert_eq!(
            palette.resolve(Some("red_dark")).unwrap(),
            Some(blend(RED, BLACK, SHADE_MIX).unwrap())
        );
        assert_eq!(
            palette.resolve(Some("bg_red")).unwrap(),
            Some(blend(RED, WHITE, BACKGROUND_TINT).unwrap())
        );
    }

    #[test]
    fn invalid_palette_shapes_are_rejected() {
        for family in ["red", "fg"] {
            for count in [0, 6] {
                let mut colors = sets();
                colors.insert(family.into(), ColorSet::Steps(vec![RED; count]));
                assert!(build(&colors).is_err());
            }
        }
        let mut colors = sets();
        colors.remove("red");
        assert!(build(&colors).is_err());
        colors.insert("pink".into(), ColorSet::Single(RED));
        assert!(build(&colors).is_err());
    }
}
