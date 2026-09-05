use std::collections::HashMap;

use ratatui::style::Color;

/// Every named color resolved for one scheme variant.
pub(crate) struct Palette(HashMap<String, Color>);

impl Palette {
    /// Build one resolved variant palette.
    pub(crate) fn new(colors: HashMap<String, Color>) -> Self {
        Self(colors)
    }

    /// Resolve an optional serialized color name.
    pub(crate) fn resolve(&self, name: Option<&str>) -> Result<Option<Color>, String> {
        name.map(|name| {
            self.0
                .get(name)
                .copied()
                .ok_or_else(|| format!("unknown palette color `{name}`"))
        })
        .transpose()
    }

    /// Every color available to the selected variant in deterministic name order.
    #[cfg(test)]
    pub(crate) fn colors(&self) -> Vec<(&str, Color)> {
        let mut colors = self
            .0
            .iter()
            .map(|(name, &color)| (name.as_str(), color))
            .collect::<Vec<_>>();
        colors.sort_unstable_by_key(|&(name, _)| name);
        colors
    }

    /// Whether this variant defines the named color.
    #[cfg(test)]
    pub(crate) fn defines(&self, color: &str) -> bool {
        self.0.contains_key(color)
    }

    /// Number of colors supplied by this variant.
    #[cfg(test)]
    pub(crate) fn color_count(&self) -> usize {
        self.0.len()
    }
}
