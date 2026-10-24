//! Named color schemes with native palettes and user-editable TOML declarations.

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

#[cfg(test)]
use ratatui::style::Color;
use ratatui::style::Style;
use serde::{Deserialize, Serialize};

pub mod groups;
pub mod palette;

#[cfg(test)]
use self::groups::StyleDeclaration;
use self::groups::{Groups, StyleTable};
use self::palette::Palette;
use super::Persisted;
use crate::ui::cycle::cycled;
use ratatui::style::Color as PaletteColor;

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
    dark: Option<ModeColors>,
    #[serde(default)]
    light: Option<ModeColors>,
}

/// A mode-wide color, or colors keyed by variant when they differ.
#[derive(Deserialize)]
#[serde(untagged)]
enum ModeColors {
    Shared(PaletteColor),
    Variants(HashMap<String, PaletteColor>),
}

impl VariantColors {
    fn get(&self, mode: Mode, variant: &str) -> Option<&PaletteColor> {
        match mode {
            Mode::Dark => &self.dark,
            Mode::Light => &self.light,
        }
        .as_ref()
        .and_then(|colors| match colors {
            ModeColors::Shared(color) => Some(color),
            ModeColors::Variants(colors) => colors.get(variant),
        })
    }
}

/// Named colors, each available in the modes and variants that define it.
#[derive(Deserialize)]
#[serde(transparent)]
struct Colors(HashMap<String, VariantColors>);

impl Colors {
    fn palette(&self, mode: Mode, variant: &str) -> Result<Palette, String> {
        let sets = self
            .0
            .iter()
            .map(|(name, colors)| {
                colors
                    .get(mode, variant)
                    .map(|colors| (name.clone(), *colors))
                    .ok_or_else(|| {
                        format!("palette color `{name}` has no {mode} `{variant}` value")
                    })
            })
            .collect::<Result<_, _>>()?;
        Palette::build(sets)
    }
}

/// Mode-qualified overrides keep identically named dark and light variants separate.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct VariantOverrides {
    dark: HashMap<String, StyleTable>,
    light: HashMap<String, StyleTable>,
}

impl VariantOverrides {
    fn get(&self, mode: Mode, variant: &str) -> Option<&StyleTable> {
        match mode {
            Mode::Dark => &self.dark,
            Mode::Light => &self.light,
        }
        .get(variant)
    }
}

/// A complete serialized scheme before its named colors are resolved.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemeFile {
    name: String,
    variants: Variants,
    palette: Colors,
    groups: StyleTable,
    /// Styles that differ from the scheme defaults for one named variant.
    #[serde(default)]
    variant_overrides: VariantOverrides,
}

impl SchemeFile {
    fn resolve(self) -> Result<Scheme, String> {
        self.resolve_with(&groups::GROUP_DECLARATIONS)
    }

    fn resolve_with(self, declarations: &groups::GroupDeclarations) -> Result<Scheme, String> {
        if self.variants.dark.is_empty() && self.variants.light.is_empty() {
            return Err("scheme must have at least one variant".into());
        }
        for (mode, names) in [
            (Mode::Dark, &self.variants.dark),
            (Mode::Light, &self.variants.light),
        ] {
            let mut seen = std::collections::HashSet::new();
            for name in names {
                if !seen.insert(name) {
                    return Err(format!("duplicate {mode} variant `{name}`"));
                }
            }
        }
        for (name, color) in &self.palette.0 {
            for (mode, values, variants) in [
                (Mode::Dark, &color.dark, &self.variants.dark),
                (Mode::Light, &color.light, &self.variants.light),
            ] {
                if let Some(ModeColors::Variants(values)) = values {
                    for variant in values.keys() {
                        if !variants.contains(variant) {
                            return Err(format!(
                                "palette `{name}` has unknown {mode} variant `{variant}`"
                            ));
                        }
                    }
                }
            }
        }
        for (overrides, names) in [
            (&self.variant_overrides.dark, &self.variants.dark),
            (&self.variant_overrides.light, &self.variants.light),
        ] {
            for name in overrides.keys() {
                if !names.contains(name) {
                    return Err(format!("variant overrides name unknown variant `{name}`"));
                }
            }
        }
        let mut variants = Vec::new();
        for (mode, names) in [
            (Mode::Dark, self.variants.dark),
            (Mode::Light, self.variants.light),
        ] {
            for name in names {
                let palette = self.palette.palette(mode, &name)?;
                let empty = HashMap::new();
                let overrides = self.variant_overrides.get(mode, &name).unwrap_or(&empty);
                let groups = Groups::resolve(&self.groups, overrides, &palette, declarations)?;
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

/// Read a theme config, installing its bundled default on first use.
fn config_source(name: &str, default: &str) -> Result<String, String> {
    let Some(path) = super::config_file(&format!("themes/{name}")) else {
        return Ok(default.into());
    };
    config_at(&path, default)
}

fn config_at(path: &std::path::Path, default: &str) -> Result<String, String> {
    if !path.exists() {
        std::fs::create_dir_all(path.parent().expect("theme config has a parent"))
            .map_err(|error| format!("{}: {error}", path.display()))?;
        use std::io::Write;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut file) => file
                .write_all(default.as_bytes())
                .map_err(|error| format!("{}: {error}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("{}: {error}", path.display())),
        }
    }
    std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
}

const SCHEME_FILES: &[(&str, &str)] = &[
    (
        "catppuccin.toml",
        include_str!("theme/schemes/catppuccin.toml"),
    ),
    (
        "tokyo-night.toml",
        include_str!("theme/schemes/tokyo-night.toml"),
    ),
    ("gruvbox.toml", include_str!("theme/schemes/gruvbox.toml")),
    (
        "gruvbox-material.toml",
        include_str!("theme/schemes/gruvbox-material.toml"),
    ),
    (
        "everforest.toml",
        include_str!("theme/schemes/everforest.toml"),
    ),
    (
        "rose-pine.toml",
        include_str!("theme/schemes/rose-pine.toml"),
    ),
    ("kanagawa.toml", include_str!("theme/schemes/kanagawa.toml")),
];

/// Installed schemes in picker order.
static SCHEMES: LazyLock<Vec<Scheme>> = LazyLock::new(|| {
    SCHEME_FILES
        .iter()
        .map(|(name, source)| {
            let resolved = config_source(name, source).and_then(|custom| {
                let file: SchemeFile = toml::from_str(&custom).map_err(|e| e.to_string())?;
                file.resolve()
            });
            resolved.unwrap_or_else(|error| {
                eprintln!("theme {name}: {error}; using built-in scheme");
                Scheme::embedded(source)
            })
        })
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

    /// This appearance carried to every scheme offering the current mode, in picker order.
    pub fn themes(self) -> Vec<Self> {
        let mode = self.mode();
        (0..SCHEMES.len())
            .filter(|&scheme| !SCHEMES[scheme].variants_in(mode).is_empty())
            .map(|scheme| self.retarget(scheme, mode))
            .collect()
    }

    /// This appearance in each variant of the current scheme and mode.
    pub fn contrasts(self) -> Vec<Self> {
        self.scheme()
            .variants_in(self.mode())
            .into_iter()
            .map(|variant| Self {
                variant: VariantIndex(variant),
                ..self
            })
            .collect()
    }

    /// This appearance in each mode the current scheme offers.
    pub fn modes(self) -> Vec<Self> {
        let mut modes = Vec::new();
        for variant in self.scheme().variants() {
            if !modes.contains(&variant.mode()) {
                modes.push(variant.mode());
            }
        }
        modes
            .into_iter()
            .map(|mode| self.retarget(self.scheme.get(), mode))
            .collect()
    }

    /// This appearance with an opaque and with a transparent background.
    pub fn backgrounds(self) -> Vec<Self> {
        [false, true]
            .map(|transparent_background| Self {
                transparent_background,
                ..self
            })
            .to_vec()
    }

    /// Step to a neighbouring scheme offering the current mode.
    pub fn cycle_theme(self, delta: i8) -> Self {
        cycled(&self.themes(), self, delta)
    }

    /// Step to a neighbouring variant of the current scheme and mode.
    pub fn cycle_contrast(self, delta: i8) -> Self {
        cycled(&self.contrasts(), self, delta)
    }

    /// Step to the scheme's neighbouring mode.
    pub fn cycle_mode(self, delta: i8) -> Self {
        cycled(&self.modes(), self, delta)
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
        cycled(&self.backgrounds(), self, 1)
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
    fn mode_qualified_overrides_do_not_affect_the_other_mode() {
        let mut file: SchemeFile = toml::from_str(SCHEME_FILES[4].1).unwrap();
        file.variant_overrides.light.insert(
            "hard".into(),
            toml::from_str("CompletionHint = { fg = 'red', italic = true }").unwrap(),
        );
        let scheme = file.resolve().unwrap();
        let dark = scheme
            .variants()
            .iter()
            .find(|v| v.mode() == Mode::Dark && v.label() == "hard")
            .unwrap();
        let light = scheme
            .variants()
            .iter()
            .find(|v| v.mode() == Mode::Light && v.label() == "hard")
            .unwrap();
        assert_eq!(dark.groups().CompletionHint, dark.groups().Muted);
        assert_eq!(light.groups().CompletionHint.fg, light.groups().Error.fg);
        assert!(light
            .groups()
            .CompletionHint
            .add_modifier
            .contains(ratatui::style::Modifier::ITALIC));
    }

    #[test]
    fn missing_variant_colors_and_misspelled_variant_keys_are_rejected() {
        for bad_palette in [
            "[palette.mauve]\ndark = { mocha = '#cba6f7' }\nlight = '#8839ef'",
            "[palette.mauve]\ndark = { mocha = '#cba6f7', machiato = '#c6a0f6', frappe = '#ca9ee6' }\nlight = '#8839ef'",
        ] {
            let mut table: toml::Table = toml::from_str(SCHEME_FILES[0].1).unwrap();
            let bad: toml::Table = toml::from_str(bad_palette).unwrap();
            table["palette"]["mauve"] = bad["palette"]["mauve"].clone();
            let file: SchemeFile = table.try_into().unwrap();
            assert!(file.resolve().is_err());
        }
    }

    #[test]
    fn native_palette_names_are_identical_across_variants_without_generated_aliases() {
        for scheme in SCHEMES.iter() {
            let names = |v: &ColorVariant| {
                v.palette()
                    .colors()
                    .iter()
                    .map(|(name, _)| name.to_string())
                    .collect::<Vec<_>>()
            };
            let expected = names(&scheme.variants()[0]);
            for variant in scheme.variants() {
                assert_eq!(names(variant), expected);
            }
        }
        let rose = &SCHEMES[5].variants()[0];
        assert_eq!(rose.palette().colors().len(), 15);
        for invented in [
            "red",
            "red_light",
            "fg0",
            "bg_red",
            "keyword",
            "leaf",
            "_nc",
        ] {
            assert!(rose.palette().resolve(Some(invented)).is_err());
        }
        assert_eq!(
            rose.palette().resolve(Some("iris")).unwrap(),
            Some(Color::Rgb(0xc4, 0xa7, 0xe7))
        );
    }

    #[test]
    fn config_defaults_are_installed_once_and_user_edits_are_preserved() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "spectral-theme-config-{}-{nonce}",
            std::process::id()
        ));
        let path = dir.join("themes/groups.toml");
        assert_eq!(config_at(&path, "original").unwrap(), "original");
        fs::write(&path, "custom").unwrap();
        assert_eq!(config_at(&path, "new defaults").unwrap(), "custom");
        fs::remove_file(&path).unwrap();
        assert_eq!(config_at(&path, "new defaults").unwrap(), "new defaults");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn every_configured_variant_paints_every_syntax_role() {
        for scheme in SCHEMES.iter() {
            for variant in scheme.variants() {
                let groups = variant.groups();
                assert!(groups.transcript().bg.is_some());
                assert!(groups.code().bg.is_some());
                assert!(groups.picker().bg.is_some());
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
    fn one_variant_can_replace_one_group_without_changing_its_siblings() {
        let mut file: SchemeFile = toml::from_str(include_str!("theme/schemes/kanagawa.toml"))
            .expect("kanagawa scheme parses");
        file.variant_overrides
            .light
            .get_mut("lotus")
            .expect("lotus has its own group table")
            .insert(
                "MarkupHeading3".into(),
                toml::from_str("fg = 'lotusRed'\nbold = true").unwrap(),
            );
        let scheme = file.resolve().expect("variant resolves");
        let wave = scheme.variants()[0].groups();
        let dragon = scheme.variants()[1].groups();
        let lotus = scheme.variants()[2].groups();
        assert_eq!(wave.MarkupHeading3.fg, wave.Function.fg);
        assert_eq!(dragon.MarkupHeading3.fg, dragon.Function.fg);
        assert_ne!(lotus.MarkupHeading3.fg, lotus.Function.fg);
        assert!(lotus
            .MarkupHeading3
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD));
    }

    #[test]
    fn kanagawa_pickers_use_the_native_popup_colors() {
        let kanagawa = SCHEMES
            .iter()
            .find(|scheme| scheme.label() == "kanagawa")
            .unwrap();
        let expected = [
            (
                "wave",
                Color::Rgb(0x22, 0x32, 0x49),
                Color::Rgb(0x2d, 0x4f, 0x67),
            ),
            (
                "dragon",
                Color::Rgb(0x22, 0x32, 0x49),
                Color::Rgb(0x2d, 0x4f, 0x67),
            ),
            (
                "lotus",
                Color::Rgb(0xc7, 0xd7, 0xe0),
                Color::Rgb(0x9f, 0xb5, 0xc9),
            ),
        ];
        for (name, popup, selected) in expected {
            let groups = kanagawa
                .variants()
                .iter()
                .find(|variant| variant.label() == name)
                .unwrap()
                .groups();
            assert_eq!(groups.picker().bg, Some(popup), "{name}");
            assert_eq!(groups.picker_selected().bg, Some(selected), "{name}");
            assert_ne!(groups.picker().bg, groups.printed().bg, "{name}");
        }
    }

    #[test]
    fn variant_overrides_reject_unknown_variants_and_groups() {
        let mut file: SchemeFile = toml::from_str(include_str!("theme/schemes/kanagawa.toml"))
            .expect("kanagawa scheme parses");
        file.variant_overrides
            .light
            .insert("louts".into(), HashMap::new());
        assert!(file.resolve().err().unwrap().contains("louts"));

        let mut file: SchemeFile = toml::from_str(include_str!("theme/schemes/kanagawa.toml"))
            .expect("kanagawa scheme parses");
        file.variant_overrides
            .light
            .get_mut("lotus")
            .unwrap()
            .insert("MarkupHedaing3".into(), StyleDeclaration::default());
        assert!(file.resolve().err().unwrap().contains("MarkupHedaing3"));
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
            palette("kanagawa", "lotus")
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
                    if !name.starts_with("Bg")
                        && style.bg.and_then(luma).is_some_and(|value| value > 140)
                    {
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
}
