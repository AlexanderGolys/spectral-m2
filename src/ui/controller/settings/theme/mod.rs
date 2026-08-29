//! Named colour schemes loaded from embedded TOML files.

use std::fmt;
use std::sync::LazyLock;

use ratatui::style::Style;
use serde::de::DeserializeOwned;

mod groups;
pub mod palette;

pub use groups::Groups;
use palette::StylePalette;
pub use palette::{
    Catppuccin, Everforest, Gruvbox, GruvboxMaterial, Kanagawa, NamedColor, PaletteData, RosePine,
    TokyoNight,
};

/// Builds highlight groups from a source palette.
type GroupsFactory = fn(&PaletteData) -> Groups;

/// One palette leaf exposed by the flat appearance picker.
pub struct Variant {
    name: &'static str,
    palette: &'static dyn StylePalette,
    groups: GroupsFactory,
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
        self.variant().name
    }

    pub const fn brightness(&self) -> Brightness {
        match self {
            Self::Dark(_) => Brightness::Dark,
            Self::Light(_) => Brightness::Light,
        }
    }

    pub fn palette(&self) -> &'static PaletteData {
        self.variant().palette.data()
    }

    pub fn native_colors(&self) -> Vec<NamedColor> {
        self.variant().palette.native_colors()
    }

    pub fn groups(&self) -> Groups {
        (self.variant().groups)(self.palette())
    }
}

/// A named colour scheme and the variants actually supplied for it.
pub struct Scheme {
    name: &'static str,
    variants: Vec<ColorVariant>,
}

impl Scheme {
    fn new(name: &'static str, variants: Vec<ColorVariant>) -> Self {
        Self { name, variants }
    }

    pub fn label(&self) -> &str {
        self.name
    }

    pub fn variants(&self) -> &[ColorVariant] {
        &self.variants
    }
}

/// Canonical terminal background brightness supplied by a scheme variant.
#[derive(Clone, Copy, PartialEq, Eq)]
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

fn embedded<T: DeserializeOwned>(source: &str) -> T {
    toml::from_str(source).expect("embedded colour scheme must be valid TOML")
}

macro_rules! embedded {
    ($name:ident: $ty:ty = $file:literal) => {
        /// A complete embedded source palette collection.
        pub static $name: LazyLock<$ty> =
            LazyLock::new(|| embedded(include_str!(concat!("schemes/", $file))));
    };
}

embedded!(CATPPUCCIN: Catppuccin = "catppuccin.toml");
embedded!(TOKYO_NIGHT: TokyoNight = "tokyo-night.toml");
embedded!(GRUVBOX: Gruvbox = "gruvbox.toml");
embedded!(GRUVBOX_MATERIAL: GruvboxMaterial = "gruvbox-material.toml");
embedded!(EVERFOREST: Everforest = "everforest.toml");
embedded!(ROSE_PINE: RosePine = "rose-pine.toml");
embedded!(KANAGAWA: Kanagawa = "kanagawa.toml");

macro_rules! variants {
    ($groups:path; $($brightness:ident $name:literal => $palette:expr),+ $(,)?) => {
        vec![$(ColorVariant::$brightness(Variant {
            name: $name,
            palette: $palette,
            groups: $groups,
        })),+]
    };
    ($($brightness:ident $name:literal => $palette:tt with $groups:path),+ $(,)?) => {
        vec![$(ColorVariant::$brightness(Variant {
            name: $name,
            palette: $palette,
            groups: $groups,
        })),+]
    };
}

/// Installed schemes in picker order.
pub static SCHEMES: LazyLock<Vec<Scheme>> = LazyLock::new(|| {
    vec![
        Scheme::new(
            "catppuccin",
            variants!(groups::catppuccin_groups;
                Dark "mocha" => &CATPPUCCIN.dark.mocha,
                Dark "macchiato" => &CATPPUCCIN.dark.macchiato,
                Dark "frappé" => &CATPPUCCIN.dark.frappe,
                Light "latte" => &CATPPUCCIN.light.latte,
            ),
        ),
        Scheme::new(
            "tokyo night",
            variants!(groups::tokyo_night_groups;
                Dark "night" => &TOKYO_NIGHT.dark.night,
                Dark "storm" => &TOKYO_NIGHT.dark.storm,
                Dark "moon" => &TOKYO_NIGHT.dark.moon,
                Light "day" => &TOKYO_NIGHT.light.day,
            ),
        ),
        Scheme::new(
            "gruvbox",
            variants!(groups::gruvbox_groups;
                Dark "hard" => &GRUVBOX.dark.hard,
                Dark "medium" => &GRUVBOX.dark.medium,
                Dark "soft" => &GRUVBOX.dark.soft,
                Light "hard" => &GRUVBOX.light.hard,
                Light "medium" => &GRUVBOX.light.medium,
                Light "soft" => &GRUVBOX.light.soft,
            ),
        ),
        Scheme::new(
            "gruvbox material",
            variants!(groups::gruvbox_material_groups;
                Dark "material" => &GRUVBOX_MATERIAL.dark.material,
                Dark "mix" => &GRUVBOX_MATERIAL.dark.mix,
                Dark "original" => &GRUVBOX_MATERIAL.dark.original,
                Light "material" => &GRUVBOX_MATERIAL.light.material,
                Light "mix" => &GRUVBOX_MATERIAL.light.mix,
                Light "original" => &GRUVBOX_MATERIAL.light.original,
            ),
        ),
        Scheme::new(
            "everforest",
            variants!(groups::everforest_groups;
                Dark "hard" => &EVERFOREST.dark.hard,
                Dark "medium" => &EVERFOREST.dark.medium,
                Dark "soft" => &EVERFOREST.dark.soft,
                Light "hard" => &EVERFOREST.light.hard,
                Light "medium" => &EVERFOREST.light.medium,
                Light "soft" => &EVERFOREST.light.soft,
            ),
        ),
        Scheme::new(
            "rosé pine",
            variants!(groups::rose_pine_groups;
                Dark "main" => &ROSE_PINE.dark.main,
                Dark "moon" => &ROSE_PINE.dark.moon,
                Light "dawn" => &ROSE_PINE.light.dawn,
            ),
        ),
        Scheme::new(
            "kanagawa",
            variants!(
                Dark "wave" => (&KANAGAWA.dark.wave) with groups::kanagawa_wave_groups,
                Dark "dragon" => (&KANAGAWA.dark.dragon) with groups::kanagawa_dragon_groups,
                Light "lotus" => (&KANAGAWA.light.lotus) with groups::kanagawa_lotus_groups,
            ),
        ),
    ]
});

/// Position of a scheme in [`SCHEMES`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SchemeIndex(usize);

impl SchemeIndex {
    pub const fn get(self) -> usize {
        self.0
    }
}

/// Position of a colour variant within its scheme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct VariantIndex(usize);

impl VariantIndex {
    pub const fn get(self) -> usize {
        self.0
    }
}

/// A selected scheme variant and its terminal-background treatment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Appearance {
    pub scheme: SchemeIndex,
    pub variant: VariantIndex,
    pub transparent_background: bool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::highlight::MacaulayHighlight;

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
                assert!(groups.error().underline_color.is_some());
                assert!(groups.warning().fg.is_some());
                assert!(groups.warning().underline_color.is_some());
                assert!(groups.diagnostic().underline_color.is_some());
                assert!(groups.type_info().fg.is_some());
                assert!(groups.border().fg.is_some());
                assert!(groups.border().underline_color.is_some());
                assert!(groups.prompt().fg.is_some());
                assert!(groups.selection().bg.is_some());
                assert!(groups.muted().fg.is_some());
                assert!(groups.printed().fg.is_some());
                for index in 0..MacaulayHighlight::COUNT {
                    let highlight = MacaulayHighlight::from_index(index)
                        .expect("Macaulay highlight indices are contiguous");
                    assert!(groups.macaulay(highlight).fg.is_some());
                }
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

        assert_eq!(labels, ["mocha", "macchiato", "frappé", "latte"]);
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
            transparent.groups().macaulay(MacaulayHighlight::String),
            groups.macaulay(MacaulayHighlight::String)
        );
    }

    #[test]
    fn the_default_is_everforest_dark_medium() {
        let appearance = Appearance::default();
        let palette = Appearance::default().variant().palette();

        assert_eq!(appearance.scheme().label(), "everforest");
        assert_eq!(appearance.variant().label(), "medium");
        assert!(appearance.variant().brightness() == Brightness::Dark);
        assert_eq!(palette.bg[1], ratatui::style::Color::Rgb(0x2d, 0x35, 0x3b));
        assert_eq!(palette.fg[0], ratatui::style::Color::Rgb(0xd3, 0xc6, 0xaa));
    }

    #[test]
    fn named_palette_trees_keep_every_family_addressable() {
        assert_eq!(
            GRUVBOX.dark.soft.red[1],
            ratatui::style::Color::Rgb(0xfb, 0x49, 0x34)
        );
        assert_eq!(
            CATPPUCCIN.light.latte.purple[0],
            ratatui::style::Color::Rgb(0x88, 0x39, 0xef)
        );
        for palette in SCHEMES
            .iter()
            .flat_map(Scheme::variants)
            .map(ColorVariant::palette)
        {
            assert!([
                &palette.red,
                &palette.green,
                &palette.yellow,
                &palette.blue,
                &palette.purple,
                &palette.aqua,
                &palette.orange,
                &palette.gray,
                &palette.bg,
                &palette.fg,
            ]
            .into_iter()
            .all(|family| !family.is_empty()));
        }
    }

    #[test]
    fn source_palettes_are_typed_and_keep_native_names() {
        assert_eq!(
            EVERFOREST.dark.medium.native.bg_dim,
            ratatui::style::Color::Rgb(0x23, 0x2a, 0x2e)
        );
        assert_eq!(
            CATPPUCCIN.dark.mocha.native.rosewater,
            ratatui::style::Color::Rgb(0xf5, 0xe0, 0xdc)
        );
        assert_eq!(
            KANAGAWA.dark.wave.native.lotusWhite5,
            ratatui::style::Color::Rgb(0xe4, 0xd7, 0x94)
        );
        assert!(SCHEMES
            .iter()
            .flat_map(Scheme::variants)
            .all(|variant| !variant.native_colors().is_empty()));
    }
}
