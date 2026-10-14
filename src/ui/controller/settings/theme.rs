//! Named colour schemes loaded from embedded TOML files.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

#[cfg(not(test))]
use std::{env, path::PathBuf};

use ratatui::style::{Color, Style};
use serde::{Deserialize, Serialize};

pub mod groups;
mod palette;

use self::groups::{Base, Groups, Overrides, StyleSpec};
use self::palette::Palette;

/// One palette leaf exposed by the flat appearance picker.
pub struct Variant {
    name: String,
    #[cfg(test)]
    palette: Palette,
    groups: Groups,
}

/// A named palette variant with its canonical brightness.
pub enum ColorVariant {
    Dark(Variant),
    Light(Variant),
}

impl ColorVariant {
    fn variant(&self) -> &Variant {
        match self {
            Self::Dark(variant) | Self::Light(variant) => variant,
        }
    }

    pub fn label(&self) -> &str {
        &self.variant().name
    }

    pub const fn brightness(&self) -> Brightness {
        match self {
            Self::Dark(_) => Brightness::Dark,
            Self::Light(_) => Brightness::Light,
        }
    }

    #[cfg(test)]
    pub fn palette(&self) -> &Palette {
        &self.variant().palette
    }

    fn groups(&self) -> Groups {
        self.variant().groups
    }
}

/// A named colour scheme and the variants actually supplied for it.
pub struct Scheme {
    name: String,
    variants: Vec<ColorVariant>,
}

impl Scheme {
    fn embedded(source: &str) -> Self {
        let file: SchemeFile =
            toml::from_str(source).expect("embedded colour scheme must be valid TOML");
        file.resolve()
            .expect("embedded colour scheme references must be valid")
    }

    pub fn label(&self) -> &str {
        &self.name
    }

    pub fn variants(&self) -> &[ColorVariant] {
        &self.variants
    }
}

/// Canonical terminal background brightness supplied by a scheme variant.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Brightness {
    Dark,
    Light,
}

impl fmt::Display for Brightness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Dark => "dark",
            Self::Light => "light",
        })
    }
}

/// Variant names divided by their terminal-background brightness.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Variants {
    #[serde(default)]
    dark: Vec<String>,
    #[serde(default)]
    light: Vec<String>,
}

/// One named color's values across every scheme variant.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VariantColors {
    #[serde(default)]
    dark: HashMap<String, Color>,
    #[serde(default)]
    light: HashMap<String, Color>,
}

impl VariantColors {
    fn get(&self, brightness: Brightness, variant: &str) -> Option<Color> {
        match brightness {
            Brightness::Dark => &self.dark,
            Brightness::Light => &self.light,
        }
        .get(variant)
        .copied()
    }
}

/// The complete color matrix keyed first by palette name.
#[derive(Deserialize)]
#[serde(transparent)]
struct Colors(HashMap<String, VariantColors>);

impl Colors {
    fn palette(&self, brightness: Brightness, variant: &str) -> Result<Palette, String> {
        self.0
            .iter()
            .map(|(name, colors)| {
                colors
                    .get(brightness, variant)
                    .map(|color| (name.clone(), color))
                    .ok_or_else(|| format!("color `{name}` has no `{variant}` value"))
            })
            .collect::<Result<_, _>>()
            .map(Palette::new)
    }
}

/// A complete serialized scheme before its named colors are resolved.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemeFile {
    name: String,
    variants: Variants,
    colors: Colors,
    base: Base<StyleSpec>,
    #[serde(default)]
    overrides: Overrides,
}

impl SchemeFile {
    fn resolve(self) -> Result<Scheme, String> {
        let mut variants = Vec::new();
        for (brightness, names) in [
            (Brightness::Dark, self.variants.dark),
            (Brightness::Light, self.variants.light),
        ] {
            for name in names {
                let palette = self.colors.palette(brightness, &name)?;
                let groups = Groups::resolve(&self.base, &self.overrides, &palette)?;
                let variant = Variant {
                    name,
                    #[cfg(test)]
                    palette,
                    groups,
                };
                variants.push(match brightness {
                    Brightness::Dark => ColorVariant::Dark(variant),
                    Brightness::Light => ColorVariant::Light(variant),
                });
            }
        }
        Ok(Scheme {
            name: self.name,
            variants,
        })
    }
}

const SCHEME_FILES: &[&str] = &[
    include_str!("theme/schemes/catppuccin.toml"),
    include_str!("theme/schemes/tokyo-night.toml"),
    include_str!("theme/schemes/gruvbox.toml"),
    include_str!("theme/schemes/gruvbox-material.toml"),
    include_str!("theme/schemes/everforest.toml"),
    include_str!("theme/schemes/rose-pine.toml"),
    include_str!("theme/schemes/kanagawa.toml"),
];

/// Installed schemes in picker order.
static SCHEMES: LazyLock<Vec<Scheme>> = LazyLock::new(|| {
    SCHEME_FILES
        .iter()
        .map(|source| Scheme::embedded(source))
        .collect()
});

/// Position of a scheme in [`SCHEMES`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct SchemeIndex(usize);

impl SchemeIndex {
    const fn get(self) -> usize {
        self.0
    }
}

/// Position of a colour variant within its scheme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct VariantIndex(usize);

impl VariantIndex {
    const fn get(self) -> usize {
        self.0
    }
}

/// A selected scheme variant and its terminal-background treatment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Appearance {
    scheme: SchemeIndex,
    variant: VariantIndex,
    pub transparent_background: bool,
}

/// Stable appearance names stored independently of in-memory picker indices.
#[derive(Serialize, Deserialize)]
struct StoredAppearance {
    scheme: String,
    variant: String,
    brightness: Brightness,
    transparent_background: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        let scheme = SCHEMES
            .iter()
            .position(|scheme| scheme.label() == "everforest")
            .expect("everforest is embedded");
        let variant = SCHEMES[scheme]
            .variants()
            .iter()
            .position(|variant| {
                variant.label() == "medium" && variant.brightness() == Brightness::Dark
            })
            .expect("everforest dark medium is embedded");
        Self {
            scheme: SchemeIndex(scheme),
            variant: VariantIndex(variant),
            transparent_background: false,
        }
    }
}

impl Appearance {
    /// Load the last selected appearance, falling back to Everforest medium.
    #[cfg(not(test))]
    pub fn load() -> Self {
        appearance_file_path()
            .as_deref()
            .and_then(Self::read)
            .unwrap_or_default()
    }

    /// Keep tests independent of a developer's saved appearance.
    #[cfg(test)]
    pub fn load() -> Self {
        Self::default()
    }

    /// Persist this appearance for the next process.
    #[cfg(not(test))]
    pub fn save(self) {
        if let Some(path) = appearance_file_path() {
            let _ = self.write(&path);
        }
    }

    /// Keep tests from modifying the developer's saved appearance.
    #[cfg(test)]
    pub fn save(self) {}

    fn read(path: &Path) -> Option<Self> {
        let stored: StoredAppearance =
            serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
        let scheme = SCHEMES
            .iter()
            .position(|scheme| scheme.label() == stored.scheme)?;
        let variant = SCHEMES[scheme].variants().iter().position(|variant| {
            variant.label() == stored.variant && variant.brightness() == stored.brightness
        })?;
        Some(Self {
            scheme: SchemeIndex(scheme),
            variant: VariantIndex(variant),
            transparent_background: stored.transparent_background,
        })
    }

    fn write(self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            path,
            serde_json::to_vec(&StoredAppearance {
                scheme: self.scheme().label().into(),
                variant: self.variant().label().into(),
                brightness: self.variant().brightness(),
                transparent_background: self.transparent_background,
            })?,
        )
    }

    pub fn scheme(self) -> &'static Scheme {
        &SCHEMES[self.scheme.get()]
    }

    pub fn variant(self) -> &'static ColorVariant {
        &self.scheme().variants()[self.variant.get()]
    }

    pub fn colorscheme_label(self) -> String {
        format!("{} {}", self.scheme().label(), self.variant().label())
    }

    pub fn cycle_colorscheme(self, delta: i8) -> Self {
        let brightness = self.variant().brightness();
        let choices = SCHEMES
            .iter()
            .enumerate()
            .flat_map(|(scheme, data)| {
                data.variants()
                    .iter()
                    .enumerate()
                    .filter(move |(_, variant)| variant.brightness() == brightness)
                    .map(move |(variant, _)| (scheme, variant))
            })
            .collect::<Vec<_>>();
        let current = choices
            .iter()
            .position(|choice| *choice == (self.scheme.get(), self.variant.get()))
            .expect("the selected colorscheme belongs to its brightness");
        let next = (current as isize + isize::from(delta)).rem_euclid(choices.len() as isize);
        let (scheme, variant) = choices[next as usize];
        Self {
            scheme: SchemeIndex(scheme),
            variant: VariantIndex(variant),
            ..self
        }
    }

    pub fn cycle_brightness(self, delta: i8) -> Self {
        let variants = self.scheme().variants();
        let mut choices = Vec::new();
        for variant in variants {
            let brightness = variant.brightness();
            if !choices.contains(&brightness) {
                choices.push(brightness);
            }
        }
        let current = choices
            .iter()
            .position(|brightness| *brightness == self.variant().brightness())
            .expect("the selected variant supplies an available brightness");
        let next = (current as isize + isize::from(delta)).rem_euclid(choices.len() as isize);
        let name = self.variant().label();
        let variant = variants
            .iter()
            .position(|variant| {
                variant.brightness() == choices[next as usize] && variant.label() == name
            })
            .or_else(|| {
                variants
                    .iter()
                    .position(|variant| variant.brightness() == choices[next as usize])
            })
            .expect("every available brightness has a variant");
        Self {
            variant: VariantIndex(variant),
            ..self
        }
    }

    pub fn cycle_background(self) -> Self {
        Self {
            transparent_background: !self.transparent_background,
            ..self
        }
    }

    pub const fn background_label(self) -> &'static str {
        if self.transparent_background {
            "transparent"
        } else {
            "opaque"
        }
    }

    pub fn groups(self) -> Groups {
        self.variant().groups()
    }

    pub fn canvas(self, opaque: Style) -> Style {
        if self.transparent_background {
            Style::default()
        } else {
            opaque
        }
    }
}

#[cfg(not(test))]
fn appearance_file_path() -> Option<PathBuf> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("spectral-m2").join("appearance.json"))
}

#[cfg(test)]
mod tests {
    use super::groups::HLGroup;
    use super::*;
    use std::env;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn every_configured_variant_paints_every_syntax_role() {
        for scheme in SCHEMES.iter() {
            for variant in scheme.variants() {
                let groups = variant.groups();
                assert!(groups.transcript().bg.is_some());
                assert!(groups.code().bg.is_some());
                assert!(groups.picker().bg.is_some());
                assert!(groups.picker_selected().fg.is_none());
                assert!(groups.picker_selected().bg.is_some());
                assert!(groups.value().fg.is_some());
                assert!(groups.error().fg.is_some());
                assert!(groups.warning().fg.is_some());
                assert!(groups.diagnostic().underline_color.is_some());
                assert!(groups.type_info().fg.is_some());
                assert!(groups.border().fg.is_some());
                assert!(groups.prompt().fg.is_some());
                assert!(groups.selection().bg.is_some());
                assert!(groups.muted().fg.is_some());
                assert!(groups.printed().fg.is_some());
            }
        }
    }

    #[test]
    fn catppuccin_encodes_its_four_canonical_flavours() {
        let variants = SCHEMES[0].variants();
        let labels: Vec<_> = variants.iter().map(ColorVariant::label).collect();
        let brightness: Vec<_> = variants
            .iter()
            .map(|variant| variant.brightness().to_string())
            .collect();

        assert_eq!(labels, ["mocha", "macchiato", "frappe", "latte"]);
        assert_eq!(brightness, ["dark", "dark", "dark", "light"]);
    }

    #[test]
    fn gruvbox_schemes_each_expose_six_variants() {
        assert_eq!(SCHEMES[2].label(), "gruvbox");
        assert_eq!(SCHEMES[2].variants().len(), 6);
        assert_eq!(SCHEMES[3].label(), "gruvbox material");
        assert_eq!(SCHEMES[3].variants().len(), 6);
    }

    #[test]
    fn rose_pine_and_kanagawa_expose_only_their_canonical_variants() {
        let rose_pine = SCHEMES
            .iter()
            .find(|scheme| scheme.label() == "rosé pine")
            .unwrap();
        let kanagawa = SCHEMES
            .iter()
            .find(|scheme| scheme.label() == "kanagawa")
            .unwrap();

        assert_eq!(
            rose_pine
                .variants()
                .iter()
                .map(|variant| (variant.brightness().to_string(), variant.label()))
                .collect::<Vec<_>>(),
            [
                ("dark".into(), "main"),
                ("dark".into(), "moon"),
                ("light".into(), "dawn")
            ]
        );
        assert_eq!(
            kanagawa
                .variants()
                .iter()
                .map(|variant| (variant.brightness().to_string(), variant.label()))
                .collect::<Vec<_>>(),
            [
                ("dark".into(), "wave"),
                ("dark".into(), "dragon"),
                ("light".into(), "lotus")
            ]
        );
    }

    #[test]
    fn appearance_cycles_each_axis_in_both_directions() {
        let appearance = Appearance::default();

        assert_eq!(
            appearance.cycle_colorscheme(1).colorscheme_label(),
            "everforest soft"
        );
        assert_eq!(
            appearance.cycle_colorscheme(-1).colorscheme_label(),
            "everforest hard"
        );
        assert_eq!(appearance.cycle_brightness(1).variant().label(), "medium");
        assert_eq!(appearance.cycle_brightness(-1).variant().label(), "medium");
        assert_eq!(appearance.background_label(), "opaque");
        assert_eq!(
            appearance.cycle_background().background_label(),
            "transparent"
        );
    }

    #[test]
    fn appearance_survives_reloading() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "spectral-m2-appearance-{}-{nonce}.json",
            std::process::id()
        ));
        let appearance = Appearance::default()
            .cycle_colorscheme(1)
            .cycle_background();
        appearance.write(&path).unwrap();

        assert_eq!(Appearance::read(&path), Some(appearance));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn transparent_background_only_clears_the_canvas() {
        let opaque = Appearance::default();
        let transparent = opaque.cycle_background();
        let groups = opaque.groups();

        assert_eq!(transparent.groups().transcript(), groups.transcript());
        assert_eq!(transparent.groups().code(), groups.code());
        assert_eq!(transparent.groups().picker(), groups.picker());
        assert_eq!(transparent.canvas(groups.transcript()).bg, None);
        assert_eq!(opaque.canvas(groups.transcript()), groups.transcript());
        assert_eq!(
            transparent.groups().style(HLGroup::String),
            groups.style(HLGroup::String)
        );
    }

    #[test]
    fn the_default_is_everforest_dark_medium() {
        let appearance = Appearance::default();
        let palette = Appearance::default().variant().palette();

        assert_eq!(appearance.scheme().label(), "everforest");
        assert_eq!(appearance.variant().label(), "medium");
        assert!(appearance.variant().brightness() == Brightness::Dark);
        assert_eq!(
            palette.resolve(Some("bg0")).unwrap(),
            Some(Color::Rgb(0x2d, 0x35, 0x3b))
        );
        assert_eq!(
            palette.resolve(Some("fg")).unwrap(),
            Some(Color::Rgb(0xd3, 0xc6, 0xaa))
        );
    }

    #[test]
    fn serialized_schemes_keep_native_colors() {
        let palette = |scheme: &str, variant: &str| {
            SCHEMES
                .iter()
                .find(|candidate| candidate.label() == scheme)
                .unwrap()
                .variants()
                .iter()
                .find(|candidate| candidate.label() == variant)
                .unwrap()
                .palette()
        };
        assert_eq!(
            palette("gruvbox", "soft")
                .resolve(Some("bright_red"))
                .unwrap(),
            Some(Color::Rgb(0xfb, 0x49, 0x34))
        );
        assert_eq!(
            palette("catppuccin", "latte")
                .resolve(Some("mauve"))
                .unwrap(),
            Some(Color::Rgb(0x88, 0x39, 0xef))
        );
        assert_eq!(
            palette("everforest", "medium")
                .resolve(Some("bg_dim"))
                .unwrap(),
            Some(Color::Rgb(0x23, 0x2a, 0x2e))
        );
        assert_eq!(
            palette("kanagawa", "wave")
                .resolve(Some("lotusWhite5"))
                .unwrap(),
            Some(Color::Rgb(0xe4, 0xd7, 0x94))
        );
    }

    #[test]
    fn bright_dark_theme_surfaces_have_dark_text() {
        let luma = |color| match color {
            Color::Rgb(red, green, blue) => {
                Some((299 * u32::from(red) + 587 * u32::from(green) + 114 * u32::from(blue)) / 1000)
            }
            _ => None,
        };
        for scheme in SCHEMES.iter() {
            for variant in scheme
                .variants()
                .iter()
                .filter(|variant| variant.brightness() == Brightness::Dark)
            {
                for (name, style) in variant.groups().highlight_groups() {
                    if style.bg.and_then(luma).is_some_and(|value| value > 140) {
                        assert!(
                            style.fg.and_then(luma).is_some_and(|value| value < 80),
                            "{} {}: {name} has a bright background without dark text",
                            scheme.label(),
                            variant.label()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_variant_defines_the_same_complete_color_matrix() {
        for scheme in SCHEMES.iter() {
            let expected = scheme.variants()[0].palette().colors();
            for variant in scheme.variants() {
                let palette = variant.palette();
                assert_eq!(palette.color_count(), expected.len());
                assert!(expected.iter().all(|&(name, _)| palette.defines(name)));
            }
        }
    }
}
