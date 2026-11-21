//! Native scheme palette colors and explicit RGB blending.

use ratatui::style::Color;
use std::collections::HashMap;

/// The sRGB transfer function and the WCAG 2 luminance and contrast definitions.
struct Srgb;

impl Srgb {
    /// Encoded channels at most this are on the transfer curve's linear segment.
    const LINEAR_SEGMENT_END: f32 = 0.04045;
    /// Slope of that linear segment.
    const LINEAR_SLOPE: f32 = 12.92;
    /// Offset and scale of the power segment `((c + a) / (1 + a))^γ`.
    const POWER_OFFSET: f32 = 0.055;
    const GAMMA: f32 = 2.4;
    /// Weights of linear red, green and blue in relative luminance (Rec. 709 primaries).
    const LUMINANCE_WEIGHTS: [f32; 3] = [0.2126, 0.7152, 0.0722];
    /// Flare added to both luminances of a contrast ratio, so black on black reads 1.
    const FLARE: f32 = 0.05;

    fn linear(channel: u8) -> f32 {
        let encoded = f32::from(channel) / f32::from(u8::MAX);
        match encoded <= Self::LINEAR_SEGMENT_END {
            true => encoded / Self::LINEAR_SLOPE,
            false => {
                ((encoded + Self::POWER_OFFSET) / (1.0 + Self::POWER_OFFSET)).powf(Self::GAMMA)
            }
        }
    }

    fn luminance(color: Color) -> Option<f32> {
        let Color::Rgb(r, g, b) = color else {
            return None;
        };
        let channels = [r, g, b].map(Self::linear);
        Some(
            Self::LUMINANCE_WEIGHTS
                .iter()
                .zip(channels)
                .map(|(w, c)| w * c)
                .sum(),
        )
    }
}

/// The WCAG contrast ratio of two RGB colors, from 1 for equal ones to 21 for
/// black against white; `None` unless both are RGB.
pub fn contrast_ratio(one: Color, other: Color) -> Option<f32> {
    let (one, other) = (Srgb::luminance(one)?, Srgb::luminance(other)?);
    Some((one.max(other) + Srgb::FLARE) / (one.min(other) + Srgb::FLARE))
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

/// A scheme's native, named RGB colors, with no generated aliases or shades.
pub struct Palette(HashMap<String, Color>);

impl Palette {
    /// Validate a complete native palette without transforming its colors.
    pub fn build(colors: HashMap<String, Color>) -> Result<Self, String> {
        if colors.is_empty() {
            return Err("palette must not be empty".into());
        }
        for (name, color) in &colors {
            if name == "none" || !matches!(color, Color::Rgb(..)) {
                return Err(format!("palette color `{name}` must be a named RGB color"));
            }
        }
        Ok(Self(colors))
    }

    /// Resolve an optional native palette name.
    pub fn resolve(&self, name: Option<&str>) -> Result<Option<Color>, String> {
        name.map(|name| {
            self.0
                .get(name)
                .copied()
                .ok_or_else(|| format!("unknown palette color `{name}`"))
        })
        .transpose()
    }

    /// Every native color in deterministic name order.
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
