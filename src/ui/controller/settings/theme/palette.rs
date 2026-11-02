//! Native scheme palette colors.

use ratatui::style::Color;
use std::collections::HashMap;

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
