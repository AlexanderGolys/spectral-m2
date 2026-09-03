use std::collections::HashMap;
use std::ops::Index;

use ratatui::style::Color;
use serde::Deserialize;
use these_macros_should_be_illegal::{discriminated_str, strutuct};

/// One scheme variant's colors, indexed by their original palette names.
#[derive(Deserialize)]
pub struct Palette {
    native: HashMap<String, Color>,
}

/// A scheme-specific color name accepted by the universal palette map.
pub trait PaletteColor: Copy {
    fn name(self) -> &'static str;
}

impl<C: PaletteColor> Index<C> for Palette {
    type Output = Color;

    fn index(&self, color: C) -> &Self::Output {
        &self.native[color.name()]
    }
}

impl Palette {
    /// Every original palette entry in deterministic name order.
    pub fn colors(&self) -> Vec<(&str, Color)> {
        let mut colors = self
            .native
            .iter()
            .map(|(name, &color)| (name.as_str(), color))
            .collect::<Vec<_>>();
        colors.sort_unstable_by_key(|&(name, _)| name);
        colors
    }

    /// Whether this source palette defines the finite color name.
    #[cfg(test)]
    pub fn defines<C: PaletteColor>(&self, color: C) -> bool {
        self.native.contains_key(color.name())
    }

    /// Number of original color names supplied by this variant.
    #[cfg(test)]
    pub fn color_count(&self) -> usize {
        self.colors().len()
    }
}

macro_rules! palette_colors {
    ($($scheme:ident { $($color:ident),+ $(,)? })+) => {$(
        #[doc = concat!("Original color names defined by `", stringify!($scheme), "`.")]
        #[discriminated_str(stringify)]
        #[allow(dead_code, non_camel_case_types)]
        #[derive(Clone, Copy)]
        pub enum $scheme {
            $($color),+
        }

        impl $scheme {
            #[cfg(test)]
            pub const ALL: &[Self] = &[$(Self::$color),+];
        }

        impl PaletteColor for $scheme {
            fn name(self) -> &'static str {
                <$scheme>::name(&self)
            }
        }
    )+};
}

palette_colors! {
    CatppuccinColor {
        base, blue, crust, flamingo, green, lavender, mantle, maroon, mauve, overlay0,
        overlay1, overlay2, peach, pink, red, rosewater, sapphire, sky, subtext0, subtext1,
        surface0, surface1, surface2, teal, text, yellow,
    }
    EverforestColor {
        aqua, bg0, bg1, bg2, bg3, bg4, bg5, bg_blue, bg_dim, bg_green, bg_purple, bg_red,
        bg_visual, bg_yellow, blue, fg, green, grey0, grey1, grey2, orange, purple, red,
        statusline1, statusline2, statusline3, yellow,
    }
    GruvboxMaterialColor {
        aqua, bg0, bg1, bg2, bg3, bg4, bg5, bg_current_word, bg_diff_blue, bg_diff_green,
        bg_diff_red, bg_dim, bg_green, bg_red, bg_statusline1, bg_statusline2,
        bg_statusline3, bg_visual_blue, bg_visual_green, bg_visual_purple, bg_visual_red,
        bg_visual_yellow, bg_yellow, blue, fg0, fg1, green, grey0, grey1, grey2, orange,
        purple, red, yellow,
    }
    GruvboxColor {
        bright_aqua, bright_blue, bright_green, bright_orange, bright_purple, bright_red,
        bright_yellow, dark0, dark0_hard, dark0_soft, dark1, dark2, dark3, dark4, dark4_256,
        faded_aqua, faded_blue, faded_green, faded_orange, faded_purple, faded_red,
        faded_yellow, gray_244, gray_245, light0, light0_hard, light0_soft, light1, light2,
        light3, light4, light4_256, neutral_aqua, neutral_blue, neutral_green,
        neutral_orange, neutral_purple, neutral_red, neutral_yellow,
    }
    KanagawaColor {
        autumnGreen, autumnRed, autumnYellow, boatYellow1, boatYellow2, carpYellow,
        crystalBlue, dragonAqua, dragonAsh, dragonBlack0, dragonBlack1, dragonBlack2,
        dragonBlack3, dragonBlack4, dragonBlack5, dragonBlack6, dragonBlue, dragonBlue2,
        dragonGray, dragonGray2, dragonGray3, dragonGreen, dragonGreen2, dragonOrange,
        dragonOrange2, dragonPink, dragonRed, dragonTeal, dragonViolet, dragonWhite,
        dragonYellow, fujiGray, fujiWhite, katanaGray, lightBlue, lotusAqua, lotusAqua2,
        lotusBlue1, lotusBlue2, lotusBlue3, lotusBlue4, lotusBlue5, lotusCyan, lotusGray,
        lotusGray2, lotusGray3, lotusGreen, lotusGreen2, lotusGreen3, lotusInk1, lotusInk2,
        lotusOrange, lotusOrange2, lotusPink, lotusRed, lotusRed2, lotusRed3, lotusRed4,
        lotusTeal1, lotusTeal2, lotusTeal3, lotusViolet1, lotusViolet2, lotusViolet3,
        lotusViolet4, lotusWhite0, lotusWhite1, lotusWhite2, lotusWhite3, lotusWhite4,
        lotusWhite5, lotusYellow, lotusYellow2, lotusYellow3, lotusYellow4, oldWhite,
        oniViolet, oniViolet2, peachRed, roninYellow, sakuraPink, samuraiRed, springBlue,
        springGreen, springViolet1, springViolet2, sumiInk0, sumiInk1, sumiInk2, sumiInk3,
        sumiInk4, sumiInk5, sumiInk6, surimiOrange, waveAqua1, waveAqua2, waveBlue1,
        waveBlue2, waveRed, winterBlue, winterGreen, winterRed, winterYellow,
    }
    RosePineColor {
        _nc, base, foam, gold, highlight_high, highlight_low, highlight_med, iris, leaf,
        love, muted, overlay, pine, rose, subtle, surface, text,
    }
    TokyoNightColor {
        bg, bg_dark, bg_dark1, bg_float, bg_highlight, bg_popup, bg_search, bg_sidebar,
        bg_statusline, bg_visual, black, blue, blue0, blue1, blue2, blue5, blue6, blue7,
        border, border_highlight, comment, cyan, dark3, dark5, error, fg, fg_dark, fg_float,
        fg_gutter, fg_sidebar, green, green1, green2, hint, info, magenta, magenta2, orange,
        purple, red, red1, teal, terminal_black, todo, warning, yellow,
    }
}

strutuct! {
    /// The complete Catppuccin palette collection.
    #[derive(Deserialize)]
    Catppuccin
    dark: CatppuccinDark {
        frappe: Palette,
        macchiato: Palette,
        mocha: Palette,
    },
    light: CatppuccinLight {
        latte: Palette,
    },
}

strutuct! {
    /// The complete Tokyo Night palette collection.
    #[derive(Deserialize)]
    TokyoNight
    dark: TokyoNightDark {
        night: Palette,
        storm: Palette,
        moon: Palette,
    },
    light: TokyoNightLight {
        day: Palette,
    },
}

strutuct! {
    /// The complete Gruvbox palette collection.
    #[derive(Deserialize)]
    Gruvbox
    dark: GruvboxDark {
        soft: Palette,
        medium: Palette,
        hard: Palette,
    },
    light: GruvboxLight {
        soft: Palette,
        medium: Palette,
        hard: Palette,
    },
}

strutuct! {
    /// The complete Gruvbox Material palette collection.
    #[derive(Deserialize)]
    GruvboxMaterial
    dark: GruvboxMaterialDark {
        material: Palette,
        mix: Palette,
        original: Palette,
    },
    light: GruvboxMaterialLight {
        material: Palette,
        mix: Palette,
        original: Palette,
    },
}

strutuct! {
    /// The complete Everforest palette collection.
    #[derive(Deserialize)]
    Everforest
    dark: EverforestDark {
        soft: Palette,
        medium: Palette,
        hard: Palette,
    },
    light: EverforestLight {
        soft: Palette,
        medium: Palette,
        hard: Palette,
    },
}

strutuct! {
    /// The complete Rosé Pine palette collection.
    #[derive(Deserialize)]
    RosePine
    dark: RosePineDark {
        main: Palette,
        moon: Palette,
    },
    light: RosePineLight {
        dawn: Palette,
    },
}

strutuct! {
    /// The complete Kanagawa palette collection.
    #[derive(Deserialize)]
    Kanagawa
    dark: KanagawaDark {
        wave: Palette,
        dragon: Palette,
    },
    light: KanagawaLight {
        lotus: Palette,
    },
}
//
// strutuct! { S (
//     E {
//         A, 
//         B(B),
//         (Vec<u8>),
//         _2(C1, D1),
//         _3{E, F},
//         D(E1, F),
//         G{H, I},
//     },
//     u8)
// }


