//! Named colour schemes loaded from embedded TOML files.

use std::fmt;
use std::sync::LazyLock;

use ratatui::style::Style;
use serde::de::DeserializeOwned;

pub mod groups;
mod palette;

use self::groups::Groups;
use self::palette::{
    Catppuccin, Everforest, Gruvbox, GruvboxMaterial, Kanagawa, Palette, RosePine, TokyoNight,
};

/// Builds highlight groups from a source palette.
type GroupsFactory = fn(&Palette) -> Groups;

/// One palette leaf exposed by the flat appearance picker.
pub struct Variant {
    name: &'static str,
    palette: &'static Palette,
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

    pub fn palette(&self) -> &'static Palette {
        self.variant().palette
    }

    fn groups(&self) -> Groups {
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
        static $name: LazyLock<$ty> =
            LazyLock::new(|| embedded(include_str!(concat!("theme/schemes/", $file))));
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
static SCHEMES: LazyLock<Vec<Scheme>> = LazyLock::new(|| {
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
            variants!(
                Dark "night" => (&TOKYO_NIGHT.dark.night) with groups::tokyo_night_groups,
                Dark "storm" => (&TOKYO_NIGHT.dark.storm) with groups::tokyo_night_groups,
                Dark "moon" => (&TOKYO_NIGHT.dark.moon) with groups::tokyo_night_groups,
                Light "day" => (&TOKYO_NIGHT.light.day) with groups::tokyo_day_groups,
            ),
        ),
        Scheme::new(
            "gruvbox",
            variants!(
                Dark "hard" => (&GRUVBOX.dark.hard) with groups::gruvbox_dark_hard_groups,
                Dark "medium" => (&GRUVBOX.dark.medium) with groups::gruvbox_dark_medium_groups,
                Dark "soft" => (&GRUVBOX.dark.soft) with groups::gruvbox_dark_soft_groups,
                Light "hard" => (&GRUVBOX.light.hard) with groups::gruvbox_light_hard_groups,
                Light "medium" => (&GRUVBOX.light.medium) with groups::gruvbox_light_medium_groups,
                Light "soft" => (&GRUVBOX.light.soft) with groups::gruvbox_light_soft_groups,
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
    use super::groups::HLGroup;
    use super::*;

    #[test]
    fn every_configured_variant_paints_every_syntax_role() {
        for scheme in SCHEMES.iter() {
            for variant in scheme.variants() {
                let groups = variant.groups();
                assert!(groups.transcript().bg.is_some());
                assert!(groups.code().bg.is_some());
                assert!(groups.picker().bg.is_some());
                assert!(groups.picker_selected().fg.is_some());
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
            palette[palette::EverforestColor::bg0],
            ratatui::style::Color::Rgb(0x2d, 0x35, 0x3b)
        );
        assert_eq!(
            palette[palette::EverforestColor::fg],
            ratatui::style::Color::Rgb(0xd3, 0xc6, 0xaa)
        );
    }

    #[test]
    fn palette_trees_use_finite_native_color_names() {
        assert_eq!(
            GRUVBOX.dark.soft[palette::GruvboxColor::bright_red],
            ratatui::style::Color::Rgb(0xfb, 0x49, 0x34)
        );
        assert_eq!(
            CATPPUCCIN.light.latte[palette::CatppuccinColor::mauve],
            ratatui::style::Color::Rgb(0x88, 0x39, 0xef)
        );
    }

    #[test]
    fn source_palettes_keep_native_names() {
        assert_eq!(
            EVERFOREST.dark.medium[palette::EverforestColor::bg_dim],
            ratatui::style::Color::Rgb(0x23, 0x2a, 0x2e)
        );
        assert_eq!(
            CATPPUCCIN.dark.mocha[palette::CatppuccinColor::rosewater],
            ratatui::style::Color::Rgb(0xf5, 0xe0, 0xdc)
        );
        assert_eq!(
            KANAGAWA.dark.wave[palette::KanagawaColor::lotusWhite5],
            ratatui::style::Color::Rgb(0xe4, 0xd7, 0x94)
        );
    }

    #[test]
    fn every_palette_defines_its_finite_color_enum() {
        macro_rules! complete {
            ($color:ty; $($palette:expr),+ $(,)?) => {
                for palette in [$($palette),+] {
                    assert_eq!(palette.color_count(), <$color>::ALL.len());
                    assert!(<$color>::ALL.iter().all(|&color| palette.defines(color)));
                }
            };
        }

        complete!(palette::CatppuccinColor;
            &CATPPUCCIN.dark.mocha, &CATPPUCCIN.dark.macchiato,
            &CATPPUCCIN.dark.frappe, &CATPPUCCIN.light.latte);
        complete!(palette::TokyoNightColor;
            &TOKYO_NIGHT.dark.night, &TOKYO_NIGHT.dark.storm,
            &TOKYO_NIGHT.dark.moon, &TOKYO_NIGHT.light.day);
        complete!(palette::GruvboxColor;
            &GRUVBOX.dark.soft, &GRUVBOX.dark.medium, &GRUVBOX.dark.hard,
            &GRUVBOX.light.soft, &GRUVBOX.light.medium, &GRUVBOX.light.hard);
        complete!(palette::GruvboxMaterialColor;
            &GRUVBOX_MATERIAL.dark.material, &GRUVBOX_MATERIAL.dark.mix,
            &GRUVBOX_MATERIAL.dark.original, &GRUVBOX_MATERIAL.light.material,
            &GRUVBOX_MATERIAL.light.mix, &GRUVBOX_MATERIAL.light.original);
        complete!(palette::EverforestColor;
            &EVERFOREST.dark.soft, &EVERFOREST.dark.medium, &EVERFOREST.dark.hard,
            &EVERFOREST.light.soft, &EVERFOREST.light.medium, &EVERFOREST.light.hard);
        complete!(palette::RosePineColor;
            &ROSE_PINE.dark.main, &ROSE_PINE.dark.moon, &ROSE_PINE.light.dawn);
        complete!(palette::KanagawaColor;
            &KANAGAWA.dark.wave, &KANAGAWA.dark.dragon, &KANAGAWA.light.lotus);
    }
}
