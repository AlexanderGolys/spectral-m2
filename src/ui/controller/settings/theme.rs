//! Named colour schemes loaded from embedded TOML files.

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use ratatui::style::{Color, Style};
use serde::{Deserialize, Serialize};

pub mod groups;
mod palette;

use self::groups::{Base, Groups, Overrides, StyleSpec};
use self::palette::Palette;
use super::Persisted;
use crate::ui::controller::cycled;

/// One palette leaf exposed by the flat appearance picker.
pub struct Variant {
    name: String,
    #[cfg(test)]
    palette: Palette,
    groups: Groups,
}

/// A named palette variant with its canonical mode.
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

    pub const fn mode(&self) -> Mode {
        match self {
            Self::Dark(_) => Mode::Dark,
            Self::Light(_) => Mode::Light,
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

    /// Indices of the variants supplied for `mode`, in declaration order.
    fn variants_in(&self, mode: Mode) -> Vec<usize> {
        (0..self.variants.len())
            .filter(|&variant| self.variants[variant].mode() == mode)
            .collect()
    }
}

/// Canonical terminal background mode supplied by a scheme variant.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Dark,
    Light,
}

impl fmt::Display for Mode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Dark => "dark",
            Self::Light => "light",
        })
    }
}

/// Variant names divided by their terminal-background mode.
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
    fn get(&self, mode: Mode, variant: &str) -> Option<Color> {
        match mode {
            Mode::Dark => &self.dark,
            Mode::Light => &self.light,
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
    fn palette(&self, mode: Mode, variant: &str) -> Result<Palette, String> {
        self.0
            .iter()
            .map(|(name, colors)| {
                colors
                    .get(mode, variant)
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
        for (mode, names) in [
            (Mode::Dark, self.variants.dark),
            (Mode::Light, self.variants.light),
        ] {
            for name in names {
                let palette = self.colors.palette(mode, &name)?;
                let groups = Groups::resolve(&self.base, &self.overrides, &palette)?;
                let variant = Variant {
                    name,
                    #[cfg(test)]
                    palette,
                    groups,
                };
                variants.push(match mode {
                    Mode::Dark => ColorVariant::Dark(variant),
                    Mode::Light => ColorVariant::Light(variant),
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "StoredAppearance", into = "StoredAppearance")]
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
    #[serde(alias = "brightness")]
    mode: Mode,
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
            .position(|variant| variant.label() == "medium" && variant.mode() == Mode::Dark)
            .expect("everforest dark medium is embedded");
        Self {
            scheme: SchemeIndex(scheme),
            variant: VariantIndex(variant),
            transparent_background: false,
        }
    }
}

impl Persisted for Appearance {
    const FILE: &'static str = "appearance.json";
}

impl From<Appearance> for StoredAppearance {
    fn from(appearance: Appearance) -> Self {
        Self {
            scheme: appearance.scheme().label().into(),
            variant: appearance.variant().label().into(),
            mode: appearance.mode(),
            transparent_background: appearance.transparent_background,
        }
    }
}

impl TryFrom<StoredAppearance> for Appearance {
    type Error = String;

    /// The appearance a stored one names, unless its scheme or variant is gone.
    fn try_from(stored: StoredAppearance) -> Result<Self, Self::Error> {
        let scheme = SCHEMES
            .iter()
            .position(|scheme| scheme.label() == stored.scheme)
            .ok_or_else(|| format!("no scheme {}", stored.scheme))?;
        let variant = SCHEMES[scheme]
            .variants()
            .iter()
            .position(|variant| variant.label() == stored.variant && variant.mode() == stored.mode)
            .ok_or_else(|| format!("no variant {} of {}", stored.variant, stored.scheme))?;
        Ok(Self {
            scheme: SchemeIndex(scheme),
            variant: VariantIndex(variant),
            transparent_background: stored.transparent_background,
        })
    }
}

impl Appearance {
    pub fn scheme(self) -> &'static Scheme {
        &SCHEMES[self.scheme.get()]
    }

    pub fn variant(self) -> &'static ColorVariant {
        &self.scheme().variants()[self.variant.get()]
    }

    pub fn mode(self) -> Mode {
        self.variant().mode()
    }

    /// Step to a neighbouring scheme offering the current mode.
    pub fn cycle_theme(self, delta: i8) -> Self {
        let mode = self.mode();
        let schemes = (0..SCHEMES.len())
            .filter(|&scheme| !SCHEMES[scheme].variants_in(mode).is_empty())
            .collect::<Vec<_>>();
        self.retarget(cycled(&schemes, self.scheme.get(), delta), mode)
    }

    /// Step to a neighbouring variant of the current scheme and mode.
    pub fn cycle_contrast(self, delta: i8) -> Self {
        let variants = self.scheme().variants_in(self.mode());
        Self {
            variant: VariantIndex(cycled(&variants, self.variant.get(), delta)),
            ..self
        }
    }

    /// Step to the scheme's neighbouring mode.
    pub fn cycle_mode(self, delta: i8) -> Self {
        let mut modes = Vec::new();
        for variant in self.scheme().variants() {
            if !modes.contains(&variant.mode()) {
                modes.push(variant.mode());
            }
        }
        self.retarget(self.scheme.get(), cycled(&modes, self.mode(), delta))
    }

    /// Select `scheme` in `mode`, keeping the current variant's name when offered and
    /// otherwise its position among its mode's variants.
    fn retarget(self, scheme: usize, mode: Mode) -> Self {
        let target = &SCHEMES[scheme];
        let candidates = target.variants_in(mode);
        let position = self
            .scheme()
            .variants_in(self.mode())
            .iter()
            .position(|&variant| variant == self.variant.get())
            .expect("the selected variant belongs to its mode");
        let variant = candidates
            .iter()
            .copied()
            .find(|&variant| target.variants()[variant].label() == self.variant().label())
            .unwrap_or_else(|| candidates[position.min(candidates.len() - 1)]);
        Self {
            scheme: SchemeIndex(scheme),
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

    /// The scheme to paint with. A transparent background takes the canvas colour
    /// out of the scheme itself, so whatever is set in the canvas group -- the
    /// padding inside a laid-out value included -- shows the terminal through.
    pub fn groups(self) -> Groups {
        let mut groups = self.variant().groups();
        if self.transparent_background {
            groups.Canvas.bg = None;
        }
        groups
    }

    pub fn canvas(self, opaque: Style) -> Style {
        if self.transparent_background {
            Style::default()
        } else {
            opaque
        }
    }
}

#[cfg(test)]
mod tests {
    use super::groups::HLGroup;
    use super::*;
    use crate::ui::controller::settings::{read, write};
    use std::env;
    use std::fs;
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
        let modes: Vec<_> = variants
            .iter()
            .map(|variant| variant.mode().to_string())
            .collect();

        assert_eq!(labels, ["mocha", "macchiato", "frappe", "latte"]);
        assert_eq!(modes, ["dark", "dark", "dark", "light"]);
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
                .map(|variant| (variant.mode().to_string(), variant.label()))
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
                .map(|variant| (variant.mode().to_string(), variant.label()))
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

        assert_eq!(appearance.cycle_contrast(1).variant().label(), "soft");
        assert_eq!(appearance.cycle_contrast(-1).variant().label(), "hard");
        assert_eq!(appearance.cycle_theme(1).scheme().label(), "rosé pine");
        assert_eq!(
            appearance.cycle_theme(-1).scheme().label(),
            "gruvbox material"
        );
        assert_eq!(appearance.cycle_mode(1).variant().label(), "medium");
        assert_eq!(appearance.cycle_mode(-1).variant().label(), "medium");
        assert_eq!(appearance.background_label(), "opaque");
        assert_eq!(
            appearance.cycle_background().background_label(),
            "transparent"
        );
    }

    #[test]
    fn theme_changes_keep_the_contrast_name_or_position() {
        let appearance = Appearance::default();
        let gruvbox = appearance.cycle_theme(-1).cycle_theme(-1);
        assert_eq!(gruvbox.scheme().label(), "gruvbox");
        assert_eq!(gruvbox.variant().label(), "medium");

        let rose_pine = appearance.cycle_theme(1);
        assert_eq!(rose_pine.variant().label(), "moon");
        assert_eq!(rose_pine.cycle_theme(-1), appearance);

        let latte = appearance.cycle_theme(-4).cycle_mode(1);
        assert_eq!(latte.variant().label(), "latte");
        assert_eq!(latte.cycle_contrast(1), latte);
    }

    #[test]
    fn appearance_saved_with_brightness_still_loads() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "spectral-m2-legacy-appearance-{}-{nonce}.json",
            std::process::id()
        ));
        fs::write(
            &path,
            r#"{"scheme":"gruvbox","variant":"soft","brightness":"light","transparent_background":false}"#,
        )
        .unwrap();

        let appearance: Appearance = read(&path).expect("legacy appearance");
        let _ = fs::remove_file(&path);
        assert_eq!(appearance.scheme().label(), "gruvbox");
        assert_eq!(appearance.variant().label(), "soft");
        assert!(appearance.mode() == Mode::Light);
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
        let appearance = Appearance::default().cycle_theme(1).cycle_background();
        write(&path, &appearance).unwrap();

        assert_eq!(read(&path), Some(appearance));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn transparent_background_only_clears_the_canvas() {
        let opaque = Appearance::default();
        let transparent = opaque.cycle_background();
        let groups = opaque.groups();

        assert_eq!(transparent.groups().transcript().bg, None);
        assert_eq!(transparent.groups().transcript().fg, groups.transcript().fg);
        assert_eq!(
            transparent.groups().style(HLGroup::Canvas).bg,
            None,
            "padding set in the canvas group shows the terminal through"
        );
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
        assert!(appearance.variant().mode() == Mode::Dark);
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
                .filter(|variant| variant.mode() == Mode::Dark)
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
