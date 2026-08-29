use std::ops::{Deref, Index};

use ratatui::style::Color;
use serde::Deserialize;
use these_macros_should_be_illegal::strutuct;

strutuct! {
    #[derive(Clone, Copy)]
    ColorSpectrum
    red: Color,
    green: Color,
    yellow: Color,
    blue: Color,
    purple: Color,
    aqua: Color,
    orange: Color,
    gray: Color,
}

impl Index<u8> for ColorSpectrum {
    type Output = Color;

    fn index(&self, index: u8) -> &Self::Output {
        match index % 8 {
            0 => &self.red,
            1 => &self.green,
            2 => &self.yellow,
            3 => &self.blue,
            4 => &self.purple,
            5 => &self.aqua,
            6 => &self.orange,
            7 => &self.gray,
            _ => unreachable!(),
        }
    }
}

strutuct! {
    #[derive(Deserialize)]
    PaletteData
    red: Vec<Color>,
    green: Vec<Color>,
    yellow: Vec<Color>,
    blue: Vec<Color>,
    purple: Vec<Color>,
    aqua: Vec<Color>,
    orange: Vec<Color>,
    gray: Vec<Color>,
    bg: Vec<Color>,
    fg: Vec<Color>,
}

/// One color under the name assigned by its source scheme.
#[derive(Clone, Copy)]
pub struct NamedColor {
    pub name: &'static str,
    pub color: Color,
}

/// A typed source palette whose colors can also be enumerated.
pub trait NativePalette: Sync {
    fn colors(&self) -> Vec<NamedColor>;
}

macro_rules! native_palette {
    ($name:ident { $($field:ident),+ $(,)? }) => {
        strutuct! {
            #[allow(non_snake_case)]
            #[derive(Deserialize)]
            $name
            $($field: Color),+
        }

        impl NativePalette for $name {
            fn colors(&self) -> Vec<NamedColor> {
                vec![$(NamedColor {
                    name: stringify!($field),
                    color: self.$field,
                }),+]
            }
        }
    }
}

native_palette!(CatppuccinPalette {
    base,
    blue,
    crust,
    flamingo,
    green,
    lavender,
    mantle,
    maroon,
    mauve,
    overlay0,
    overlay1,
    overlay2,
    peach,
    pink,
    red,
    rosewater,
    sapphire,
    sky,
    subtext0,
    subtext1,
    surface0,
    surface1,
    surface2,
    teal,
    text,
    yellow,
});
native_palette!(EverforestPalette {
    aqua,
    bg0,
    bg1,
    bg2,
    bg3,
    bg4,
    bg5,
    bg_blue,
    bg_dim,
    bg_green,
    bg_purple,
    bg_red,
    bg_visual,
    bg_yellow,
    blue,
    fg,
    green,
    grey0,
    grey1,
    grey2,
    orange,
    purple,
    red,
    statusline1,
    statusline2,
    statusline3,
    yellow,
});
native_palette!(GruvboxMaterialPalette {
    aqua,
    bg0,
    bg1,
    bg2,
    bg3,
    bg4,
    bg5,
    bg_current_word,
    bg_diff_blue,
    bg_diff_green,
    bg_diff_red,
    bg_dim,
    bg_green,
    bg_red,
    bg_statusline1,
    bg_statusline2,
    bg_statusline3,
    bg_visual_blue,
    bg_visual_green,
    bg_visual_purple,
    bg_visual_red,
    bg_visual_yellow,
    bg_yellow,
    blue,
    fg0,
    fg1,
    green,
    grey0,
    grey1,
    grey2,
    orange,
    purple,
    red,
    yellow,
});
native_palette!(GruvboxPalette {
    bright_aqua,
    bright_blue,
    bright_green,
    bright_orange,
    bright_purple,
    bright_red,
    bright_yellow,
    dark0,
    dark0_hard,
    dark0_soft,
    dark1,
    dark2,
    dark3,
    dark4,
    dark4_256,
    faded_aqua,
    faded_blue,
    faded_green,
    faded_orange,
    faded_purple,
    faded_red,
    faded_yellow,
    gray_244,
    gray_245,
    light0,
    light0_hard,
    light0_soft,
    light1,
    light2,
    light3,
    light4,
    light4_256,
    neutral_aqua,
    neutral_blue,
    neutral_green,
    neutral_orange,
    neutral_purple,
    neutral_red,
    neutral_yellow,
});
native_palette!(KanagawaPalette {
    autumnGreen,
    autumnRed,
    autumnYellow,
    boatYellow1,
    boatYellow2,
    carpYellow,
    crystalBlue,
    dragonAqua,
    dragonAsh,
    dragonBlack0,
    dragonBlack1,
    dragonBlack2,
    dragonBlack3,
    dragonBlack4,
    dragonBlack5,
    dragonBlack6,
    dragonBlue,
    dragonBlue2,
    dragonGray,
    dragonGray2,
    dragonGray3,
    dragonGreen,
    dragonGreen2,
    dragonOrange,
    dragonOrange2,
    dragonPink,
    dragonRed,
    dragonTeal,
    dragonViolet,
    dragonWhite,
    dragonYellow,
    fujiGray,
    fujiWhite,
    katanaGray,
    lightBlue,
    lotusAqua,
    lotusAqua2,
    lotusBlue1,
    lotusBlue2,
    lotusBlue3,
    lotusBlue4,
    lotusBlue5,
    lotusCyan,
    lotusGray,
    lotusGray2,
    lotusGray3,
    lotusGreen,
    lotusGreen2,
    lotusGreen3,
    lotusInk1,
    lotusInk2,
    lotusOrange,
    lotusOrange2,
    lotusPink,
    lotusRed,
    lotusRed2,
    lotusRed3,
    lotusRed4,
    lotusTeal1,
    lotusTeal2,
    lotusTeal3,
    lotusViolet1,
    lotusViolet2,
    lotusViolet3,
    lotusViolet4,
    lotusWhite0,
    lotusWhite1,
    lotusWhite2,
    lotusWhite3,
    lotusWhite4,
    lotusWhite5,
    lotusYellow,
    lotusYellow2,
    lotusYellow3,
    lotusYellow4,
    oldWhite,
    oniViolet,
    oniViolet2,
    peachRed,
    roninYellow,
    sakuraPink,
    samuraiRed,
    springBlue,
    springGreen,
    springViolet1,
    springViolet2,
    sumiInk0,
    sumiInk1,
    sumiInk2,
    sumiInk3,
    sumiInk4,
    sumiInk5,
    sumiInk6,
    surimiOrange,
    waveAqua1,
    waveAqua2,
    waveBlue1,
    waveBlue2,
    waveRed,
    winterBlue,
    winterGreen,
    winterRed,
    winterYellow,
});
native_palette!(RosePinePalette {
    _nc,
    base,
    foam,
    gold,
    highlight_high,
    highlight_low,
    highlight_med,
    iris,
    leaf,
    love,
    muted,
    overlay,
    pine,
    rose,
    subtle,
    surface,
    text,
});
native_palette!(TokyoNightPalette {
    bg,
    bg_dark,
    bg_dark1,
    bg_float,
    bg_highlight,
    bg_popup,
    bg_search,
    bg_sidebar,
    bg_statusline,
    bg_visual,
    black,
    blue,
    blue0,
    blue1,
    blue2,
    blue5,
    blue6,
    blue7,
    border,
    border_highlight,
    comment,
    cyan,
    dark3,
    dark5,
    error,
    fg,
    fg_dark,
    fg_float,
    fg_gutter,
    fg_sidebar,
    green,
    green1,
    green2,
    hint,
    info,
    magenta,
    magenta2,
    orange,
    purple,
    red,
    red1,
    teal,
    terminal_black,
    todo,
    warning,
    yellow,
});

/// One scheme leaf with its native palette and universal highlight lookup.
#[derive(Deserialize)]
pub struct SchemePalette<P> {
    pub native: P,
    #[serde(flatten)]
    data: PaletteData,
}

impl<P> Deref for SchemePalette<P> {
    type Target = PaletteData;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

/// Type-erased access used by the flat appearance picker.
pub trait StylePalette: Sync {
    fn data(&self) -> &PaletteData;
    fn native_colors(&self) -> Vec<NamedColor>;
}

impl<P: NativePalette> StylePalette for SchemePalette<P> {
    fn data(&self) -> &PaletteData {
        self
    }

    fn native_colors(&self) -> Vec<NamedColor> {
        self.native.colors()
    }
}

impl PaletteData {
    /// Selects one cyclic shade from every hue family.
    ///
    /// # Panics
    ///
    /// Panics when an embedded palette leaves a hue family empty.
    pub fn spectrum(&self, index: usize) -> ColorSpectrum {
        let color = |family: &[Color]| family[index % family.len()];
        ColorSpectrum {
            red: color(&self.red),
            green: color(&self.green),
            yellow: color(&self.yellow),
            blue: color(&self.blue),
            purple: color(&self.purple),
            aqua: color(&self.aqua),
            orange: color(&self.orange),
            gray: color(&self.gray),
        }
    }
}

strutuct! {
    /// The complete Catppuccin palette collection.
    #[derive(Deserialize)]
    Catppuccin
    dark: CatppuccinDark {
        frappe: SchemePalette<CatppuccinPalette>,
        macchiato: SchemePalette<CatppuccinPalette>,
        mocha: SchemePalette<CatppuccinPalette>,
    },
    light: CatppuccinLight {
        latte: SchemePalette<CatppuccinPalette>,
    },
}

strutuct! {
    /// The complete Tokyo Night palette collection.
    #[derive(Deserialize)]
    TokyoNight
    dark: TokyoNightDark {
        night: SchemePalette<TokyoNightPalette>,
        storm: SchemePalette<TokyoNightPalette>,
        moon: SchemePalette<TokyoNightPalette>,
    },
    light: TokyoNightLight {
        day: SchemePalette<TokyoNightPalette>,
    },
}

strutuct! {
    /// The complete Gruvbox palette collection.
    #[derive(Deserialize)]
    Gruvbox
    dark: GruvboxDark {
        soft: SchemePalette<GruvboxPalette>,
        medium: SchemePalette<GruvboxPalette>,
        hard: SchemePalette<GruvboxPalette>,
    },
    light: GruvboxLight {
        soft: SchemePalette<GruvboxPalette>,
        medium: SchemePalette<GruvboxPalette>,
        hard: SchemePalette<GruvboxPalette>,
    },
}

strutuct! {
    /// The complete Gruvbox Material palette collection.
    #[derive(Deserialize)]
    GruvboxMaterial
    dark: GruvboxMaterialDark {
        material: SchemePalette<GruvboxMaterialPalette>,
        mix: SchemePalette<GruvboxMaterialPalette>,
        original: SchemePalette<GruvboxMaterialPalette>,
    },
    light: GruvboxMaterialLight {
        material: SchemePalette<GruvboxMaterialPalette>,
        mix: SchemePalette<GruvboxMaterialPalette>,
        original: SchemePalette<GruvboxMaterialPalette>,
    },
}

strutuct! {
    /// The complete Everforest palette collection.
    #[derive(Deserialize)]
    Everforest
    dark: EverforestDark {
        soft: SchemePalette<EverforestPalette>,
        medium: SchemePalette<EverforestPalette>,
        hard: SchemePalette<EverforestPalette>,
    },
    light: EverforestLight {
        soft: SchemePalette<EverforestPalette>,
        medium: SchemePalette<EverforestPalette>,
        hard: SchemePalette<EverforestPalette>,
    },
}

strutuct! {
    /// The complete Rosé Pine palette collection.
    #[derive(Deserialize)]
    RosePine
    dark: RosePineDark {
        main: SchemePalette<RosePinePalette>,
        moon: SchemePalette<RosePinePalette>,
    },
    light: RosePineLight {
        dawn: SchemePalette<RosePinePalette>,
    },
}

strutuct! {
    /// The complete Kanagawa palette collection.
    #[derive(Deserialize)]
    Kanagawa
    dark: KanagawaDark {
        wave: SchemePalette<KanagawaPalette>,
        dragon: SchemePalette<KanagawaPalette>,
    },
    light: KanagawaLight {
        lotus: SchemePalette<KanagawaPalette>,
    },
}
