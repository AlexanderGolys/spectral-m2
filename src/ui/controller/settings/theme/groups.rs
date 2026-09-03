use std::ops::Index;

use ratatui::style::{Color, Modifier, Style};
use these_macros_should_be_illegal::discriminated_str;

use super::palette::{
    CatppuccinColor as Cp, EverforestColor as Ef, GruvboxColor as Gb, GruvboxMaterialColor as Gm,
    KanagawaColor as Kg, Palette, RosePineColor as Rp, TokyoNightColor as Tn,
};

macro_rules! highlight_groups {
    ($($name:ident),+ $(,)?) => {
        /// The complete scheme-independent space of syntax and interface styles.
        #[discriminated_str(description = stringify)]
        #[repr(usize)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum HLGroup {
            $($name),+
        }

        /// All highlight-group styles resolved for one scheme variant.
        #[derive(Clone, Copy)]
        pub struct Groups([Style; [$(stringify!($name)),+].len()]);

        impl Groups {
            fn new(scheme: impl Fn(HLGroup) -> Style) -> Self {
                Self([$(scheme(HLGroup::$name)),+])
            }

            pub fn highlight_groups(&self) -> Vec<(&'static str, Style)> {
                vec![$((HLGroup::$name.description(), self[HLGroup::$name])),+]
            }
        }
    };
}

highlight_groups! {
    Comment,
    Error,
    ErrorBg,
    Warning,
    WarningBg,
    Info,
    Hint,
    String,
    Number,
    Boolean,
    Operator,
    Keyword,
    Type,
    Function,
    Installation,
    Macro,
    Property,
    Namespace,
    Package,
    CompiledFunction,
    Symbol,
    Variable,
    AlgVariable,
    Constant,
    Parameter,
    Regex,
    Punctuation,
    Delimiter,
    Quote,
    Documentation,
    Selection,
    Value,
    Muted,
    Printed,
    ShellCommand,
    ShellMessage,
    ShellError,
    HintText,
    Normal,
    BgCode,
    ValueBorder,
    ErrorBorder,
    WarningBorder,
    InfoBorder,
    Statusline1,
    Statusline2,
    Statusline3,
    StatuslineBorder,
    StatuslineBg,
    StatuslineError,
    StatuslineWarning,
    KeymapHint,
    PickerText,
    PickerBg,
    PickerSelected,
    PickerBorder,
}

impl Groups {
    pub fn style(&self, group: HLGroup) -> Style {
        self[group]
    }

    pub fn diagnostic(&self) -> Style {
        Style::new()
            .underline_color(self[HLGroup::Error].underline_color.unwrap())
            .add_modifier(Modifier::UNDERLINED)
    }
}

impl Index<HLGroup> for Groups {
    type Output = Style;

    fn index(&self, group: HLGroup) -> &Self::Output {
        &self.0[group as usize]
    }
}

macro_rules! accessors {
    ($($name:ident: $group:ident),+ $(,)?) => {
        impl Groups {$(
            pub fn $name(&self) -> Style {
                self[HLGroup::$group]
            }
        )+}
    };
}

accessors! {
    transcript: Normal,
    code: BgCode,
    picker: PickerBg,
    picker_selected: PickerSelected,
    value: Value,
    error: Error,
    error_bg: ErrorBg,
    warning: Warning,
    warning_bg: WarningBg,
    preview_mode: Documentation,
    tree_code: Symbol,
    type_info: Type,
    border: ValueBorder,
    prompt: KeymapHint,
    selection: Selection,
    muted: Muted,
    printed: Printed,
    error_border: ErrorBorder,
    warning_border: WarningBorder,
}

impl Default for Groups {
    fn default() -> Self {
        super::Appearance::default().groups()
    }
}

fn fg(color: Color) -> Style {
    Style::new().fg(color)
}

fn bg(color: Color) -> Style {
    Style::new().bg(color)
}

fn surface(foreground: Color, background: Color) -> Style {
    fg(foreground).bg(background)
}

fn italic(color: Color) -> Style {
    fg(color).add_modifier(Modifier::ITALIC)
}

fn bold(color: Color) -> Style {
    fg(color).add_modifier(Modifier::BOLD)
}

fn underlined(color: Color) -> Style {
    fg(color)
        .underline_color(color)
        .add_modifier(Modifier::UNDERLINED)
}

fn bordered(color: Color) -> Style {
    fg(color).underline_color(color)
}

macro_rules! styles {
    ($group:expr; $($($variant:ident)|+ => $style:expr),+ $(,)?) => {
        match $group {$($(HLGroup::$variant)|+ => $style),+}
    };
}

macro_rules! factories {
    ($($factory:ident: $scheme:ident),+ $(,)?) => {$(
        pub fn $factory(palette: &Palette) -> Groups {
            Groups::new(|group| $scheme(group, palette))
        }
    )+};
}

factories! {
    catppuccin_groups: catppuccin,
    tokyo_night_groups: tokyo_night,
    tokyo_day_groups: tokyo_day,
    gruvbox_material_groups: gruvbox_material,
    everforest_groups: everforest,
    rose_pine_groups: rose_pine,
    kanagawa_wave_groups: kanagawa_wave,
    kanagawa_dragon_groups: kanagawa_dragon,
    kanagawa_lotus_groups: kanagawa_lotus,
}

fn catppuccin(g: HLGroup, p: &Palette) -> Style {
    styles!(g;
        Comment => italic(p[Cp::overlay2]),
        Documentation | Regex => underlined(p[Cp::blue]).add_modifier(Modifier::ITALIC),
        Error | ShellError | ErrorBorder | StatuslineError => bordered(p[Cp::red]),
        ErrorBg => surface(p[Cp::base], p[Cp::red]),
        Warning | WarningBorder | StatuslineWarning => bordered(p[Cp::yellow]),
        WarningBg => surface(p[Cp::base], p[Cp::yellow]),
        Info | InfoBorder => bordered(p[Cp::blue]),
        Hint | ShellCommand | KeymapHint => fg(p[Cp::teal]),
        String | Printed | Statusline1 => fg(p[Cp::green]),
        Number | Boolean | Constant | CompiledFunction => fg(p[Cp::peach]),
        Operator => fg(p[Cp::sky]),
        Keyword => fg(p[Cp::mauve]),
        Type | Statusline2 => fg(p[Cp::yellow]),
        Function | Macro => fg(p[Cp::blue]),
        Installation => fg(p[Cp::sapphire]),
        Property => fg(p[Cp::lavender]),
        Namespace | Package => italic(p[Cp::yellow]),
        Symbol | Quote => fg(p[Cp::pink]),
        Variable | Value | PickerText => fg(p[Cp::text]),
        AlgVariable | Statusline3 => fg(p[Cp::red]),
        Parameter => fg(p[Cp::maroon]),
        Punctuation | Delimiter => fg(p[Cp::overlay2]),
        Selection => bg(p[Cp::surface2]),
        Muted | ShellMessage | HintText => fg(p[Cp::overlay0]),
        Normal | PickerBg => bg(p[Cp::base]),
        BgCode => bg(p[Cp::mantle]),
        ValueBorder | PickerBorder | StatuslineBorder => bordered(p[Cp::surface2]),
        StatuslineBg => bg(p[Cp::crust]),
        PickerSelected => surface(p[Cp::base], p[Cp::green]),
    )
}

fn tokyo(g: HLGroup, p: &Palette, selection: Tn, statusline_bg: Tn) -> Style {
    styles!(g;
        Comment => italic(p[Tn::comment]),
        Documentation | Regex => underlined(p[Tn::fg]),
        Error | ShellError | ErrorBorder | StatuslineError => bordered(p[Tn::error]),
        ErrorBg => surface(p[Tn::bg], p[Tn::error]),
        Warning | WarningBorder | StatuslineWarning => bordered(p[Tn::warning]),
        WarningBg => surface(p[Tn::bg], p[Tn::warning]),
        Info | InfoBorder => bordered(p[Tn::blue]),
        Hint | ShellCommand | KeymapHint => fg(p[Tn::cyan]),
        String | Printed | Statusline1 => fg(p[Tn::green]),
        Number | Boolean => fg(p[Tn::orange]),
        Operator | Delimiter => fg(p[Tn::blue5]),
        Keyword => italic(p[Tn::purple]),
        Type | CompiledFunction | Constant | Quote => fg(p[Tn::blue1]),
        Function | Macro | Installation => fg(p[Tn::blue]),
        Property => fg(p[Tn::green1]),
        Namespace | Package => fg(p[Tn::cyan]),
        Symbol => fg(p[Tn::blue1]),
        Variable | Value | PickerText => fg(p[Tn::fg]),
        AlgVariable | Statusline3 => fg(p[Tn::red]),
        Parameter | Statusline2 => fg(p[Tn::yellow]),
        Punctuation => fg(p[Tn::fg_dark]),
        Selection => bg(p[selection]),
        Muted | ShellMessage | HintText => fg(p[Tn::fg_dark]),
        Normal | PickerBg => bg(p[Tn::bg]),
        BgCode => bg(p[Tn::bg_dark]),
        ValueBorder | PickerBorder | StatuslineBorder => bordered(p[Tn::terminal_black]),
        StatuslineBg => bg(p[statusline_bg]),
        PickerSelected => surface(p[Tn::bg], p[Tn::green]),
    )
}

fn tokyo_night(group: HLGroup, palette: &Palette) -> Style {
    tokyo(group, palette, Tn::blue7, Tn::bg_dark1)
}

fn tokyo_day(group: HLGroup, palette: &Palette) -> Style {
    tokyo(group, palette, Tn::bg_visual, Tn::bg_dark)
}

fn rose_pine(g: HLGroup, p: &Palette) -> Style {
    styles!(g;
        Comment => italic(p[Rp::subtle]),
        Documentation | Regex => underlined(p[Rp::iris]),
        Error | ShellError | ErrorBorder | StatuslineError => bordered(p[Rp::love]),
        ErrorBg => surface(p[Rp::base], p[Rp::love]),
        Warning | WarningBorder | StatuslineWarning => bordered(p[Rp::gold]),
        WarningBg => surface(p[Rp::base], p[Rp::gold]),
        Info | InfoBorder => bordered(p[Rp::foam]),
        Hint => fg(p[Rp::pine]),
        String | Number | Constant | Printed | Statusline2 => fg(p[Rp::gold]),
        Boolean | Function | Macro | CompiledFunction => fg(p[Rp::rose]),
        Operator | Punctuation | Delimiter => fg(p[Rp::subtle]),
        Keyword => fg(p[Rp::pine]),
        Type | Installation => fg(p[Rp::foam]),
        Property => italic(p[Rp::foam]),
        Namespace | Package | Value | PickerText => fg(p[Rp::text]),
        Symbol | Parameter => italic(p[Rp::iris]),
        Variable => italic(p[Rp::text]),
        AlgVariable => bold(p[Rp::love]).add_modifier(Modifier::ITALIC),
        Quote => fg(p[Rp::gold]),
        Selection => bg(p[Rp::highlight_med]),
        ShellCommand | KeymapHint => fg(p[Rp::foam]),
        Muted | ShellMessage | HintText => fg(p[Rp::subtle]),
        Normal | PickerBg => bg(p[Rp::base]),
        BgCode => bg(p[Rp::_nc]),
        ValueBorder | PickerBorder | StatuslineBorder => bordered(p[Rp::highlight_low]),
        Statusline1 => fg(p[Rp::leaf]),
        Statusline3 => fg(p[Rp::love]),
        StatuslineBg => bg(p[Rp::_nc]),
        PickerSelected => surface(p[Rp::base], p[Rp::leaf]),
    )
}

macro_rules! gruvbox {
    ($name:ident; red: $red:ident, yellow: $yellow:ident, blue: $blue:ident,
     aqua: $aqua:ident, green: $green:ident, purple: $purple:ident,
     orange: $orange:ident, text: $text:ident, selection: $selection:ident,
     statusline: $statusline:ident) => {
        fn $name(g: HLGroup, p: &Palette, background: Gb) -> Style {
            styles!(g;
                Comment => italic(p[Gb::gray_244]),
                Documentation | Regex => underlined(p[Gb::$blue]),
                Error | ShellError => bold(p[Gb::$red])
                    .add_modifier(Modifier::REVERSED)
                    .underline_color(p[Gb::$red]),
                ErrorBorder | StatuslineError => bordered(p[Gb::$red]),
                ErrorBg => surface(p[background], p[Gb::$red]),
                Warning | WarningBorder | StatuslineWarning => bordered(p[Gb::$yellow]),
                WarningBg => surface(p[background], p[Gb::$yellow]),
                Info | InfoBorder => bordered(p[Gb::$blue]),
                Hint | ShellCommand | KeymapHint => fg(p[Gb::$aqua]),
                String | Printed => italic(p[Gb::$green]),
                Statusline1 => fg(p[Gb::$green]),
                Number | Boolean => fg(p[Gb::$purple]),
                Operator | Punctuation | Delimiter => fg(p[Gb::$orange]),
                Installation | Keyword | Statusline3 => fg(p[Gb::$red]),
                Type | Statusline2 => fg(p[Gb::$yellow]),
                Function | Macro => bold(p[Gb::$green]),
                Property | Parameter => fg(p[Gb::$blue]),
                Namespace | Package | Variable | Value | PickerText => fg(p[Gb::$text]),
                Symbol | Quote | Constant | CompiledFunction | AlgVariable => fg(p[Gb::$orange]),
                Selection => bg(p[Gb::$selection]),
                Muted | ShellMessage | HintText => fg(p[Gb::gray_244]),
                Normal | BgCode | PickerBg => bg(p[background]),
                ValueBorder | PickerBorder | StatuslineBorder => bordered(p[Gb::$selection]),
                StatuslineBg => bg(p[Gb::$statusline]),
                PickerSelected => surface(p[background], p[Gb::$green]),
            )
        }
    };
}

gruvbox!(gruvbox_dark;
    red: bright_red, yellow: bright_yellow, blue: bright_blue, aqua: bright_aqua,
    green: bright_green, purple: bright_purple, orange: bright_orange, text: light1,
    selection: dark3, statusline: dark2
);
gruvbox!(gruvbox_light;
    red: faded_red,
    yellow: faded_yellow,
    blue: faded_blue,
    aqua: faded_aqua,
    green: faded_green,
    purple: faded_purple,
    orange: faded_orange,
    text: dark1,
    selection: light3,
    statusline: light2
);

macro_rules! gruvbox_factories {
    ($($factory:ident: $scheme:ident($background:ident)),+ $(,)?) => {$(
        pub fn $factory(palette: &Palette) -> Groups {
            Groups::new(|group| $scheme(group, palette, Gb::$background))
        }
    )+};
}

gruvbox_factories! {
    gruvbox_dark_soft_groups: gruvbox_dark(dark0_soft),
    gruvbox_dark_medium_groups: gruvbox_dark(dark0),
    gruvbox_dark_hard_groups: gruvbox_dark(dark0_hard),
    gruvbox_light_soft_groups: gruvbox_light(light0_soft),
    gruvbox_light_medium_groups: gruvbox_light(light0),
    gruvbox_light_hard_groups: gruvbox_light(light0_hard),
}

macro_rules! sainnhe {
    ($name:ident, $color:ident, $text:ident, $selection:ident, $function:ident,
     $error_bg:ident, $warning_bg:ident, $statusline1:ident, $statusline2:ident,
     $statusline3:ident) => {
        fn $name(g: HLGroup, p: &Palette) -> Style {
            styles!(g;
                Comment => italic(p[$color::grey1]),
                Documentation | Regex => underlined(p[$color::blue]),
                Error | ShellError | ErrorBorder | StatuslineError => bordered(p[$color::red]),
                ErrorBg => surface(p[$color::red], p[$color::$error_bg]),
                Warning | WarningBorder | StatuslineWarning => bordered(p[$color::yellow]),
                WarningBg => surface(p[$color::yellow], p[$color::$warning_bg]),
                Info | InfoBorder => bordered(p[$color::blue]),
                Hint | ShellCommand | KeymapHint => fg(p[$color::aqua]),
                String | Printed => fg(p[$color::aqua]),
                Number | Boolean => fg(p[$color::purple]),
                Operator | Installation => fg(p[$color::orange]),
                Keyword => fg(p[$color::red]),
                Type => italic(p[$color::yellow]),
                Function | Macro | CompiledFunction => $function(p[$color::green]),
                Property | Symbol => fg(p[$color::blue]),
                Namespace | Package => italic(p[$color::yellow]),
                Variable | Parameter | Value | PickerText => fg(p[$color::$text]),
                AlgVariable | Constant => italic(p[$color::purple]),
                Punctuation => fg(p[$color::grey2]),
                Delimiter | Muted => fg(p[$color::grey1]),
                ShellMessage | HintText => fg(p[$color::grey0]),
                Quote => fg(p[$color::yellow]),
                Selection => bg(p[$color::$selection]),
                Normal | PickerBg => bg(p[$color::bg0]),
                BgCode => bg(p[$color::bg_dim]),
                ValueBorder => bordered(p[$color::bg3]),
                PickerBorder => bordered(p[$color::bg4]),
                StatuslineBorder => bordered(p[$color::bg5]),
                Statusline1 => fg(p[$color::$statusline1]),
                Statusline2 => fg(p[$color::$statusline2]),
                Statusline3 => fg(p[$color::$statusline3]),
                StatuslineBg => bg(p[$color::bg1]),
                PickerSelected => surface(p[$color::bg0], p[$color::green]),
            )
        }
    };
}

sainnhe!(
    gruvbox_material,
    Gm,
    fg0,
    bg_visual_blue,
    bold,
    bg_visual_red,
    bg_visual_yellow,
    green,
    yellow,
    red
);
sainnhe!(
    everforest,
    Ef,
    fg,
    bg_visual,
    fg,
    bg_red,
    bg_yellow,
    statusline1,
    statusline2,
    statusline3
);

/// Variant-specific colors used by the shared Kanagawa style function.
#[derive(Clone, Copy)]
struct Kanagawa {
    comment: Color,
    error: Color,
    number: Color,
    boolean: Color,
    string: Color,
    url: Color,
    text: Color,
    builtin: Color,
    parameter: Color,
    function: Color,
    label: Color,
    ty: Color,
    property: Color,
    operator: Color,
    ret: Color,
    punctuation: Color,
    background: Color,
    background_dim: Color,
    border: Color,
    selection: Color,
    warning: Color,
    error_bg: Color,
    warning_bg: Color,
}

fn kanagawa(g: HLGroup, c: Kanagawa) -> Style {
    styles!(g;
        Comment => italic(c.comment),
        Error | ShellError | ErrorBorder | StatuslineError => bordered(c.error),
        ErrorBg => surface(c.error, c.error_bg),
        Warning | WarningBorder | StatuslineWarning | Statusline2 => bordered(c.warning),
        WarningBg => surface(c.warning, c.warning_bg),
        Info | InfoBorder => bordered(c.url),
        Hint | ShellCommand | KeymapHint => fg(c.property),
        String | Printed => fg(c.string),
        Number => fg(c.number),
        Boolean => bold(c.boolean),
        Constant | Symbol => fg(c.boolean),
        Quote => fg(c.string),
        Operator | Delimiter => fg(c.operator),
        Keyword => italic(c.label),
        Type => fg(c.ty),
        Function | Macro | Statusline1 => fg(c.function),
        CompiledFunction => bold(c.function),
        Installation => bold(c.label),
        Property => fg(c.property),
        Namespace | Package => fg(c.ty),
        Variable | Value | PickerText => fg(c.text),
        AlgVariable => italic(c.builtin),
        Statusline3 => fg(c.ret),
        Parameter => fg(c.parameter),
        Regex | Documentation => underlined(c.url),
        Punctuation => fg(c.punctuation),
        Selection => bg(c.selection),
        Muted | ShellMessage | HintText => fg(c.comment),
        Normal | PickerBg => bg(c.background),
        BgCode | StatuslineBg => bg(c.background_dim),
        ValueBorder | PickerBorder | StatuslineBorder => bordered(c.border),
        PickerSelected => surface(c.background, c.string),
    )
}

macro_rules! kanagawa {
    ($name:ident; $p:ident; $($field:ident: $value:expr),+ $(,)?) => {
        fn $name(group: HLGroup, $p: &Palette) -> Style {
            kanagawa(group, Kanagawa { $($field: $value),+ })
        }
    };
}

kanagawa!(kanagawa_wave; p;
    comment: p[Kg::fujiGray], error: p[Kg::samuraiRed], number: p[Kg::sakuraPink],
    boolean: p[Kg::surimiOrange], string: p[Kg::springGreen], url: p[Kg::springBlue],
    text: p[Kg::fujiWhite], builtin: p[Kg::waveRed], parameter: p[Kg::oniViolet2],
    function: p[Kg::crystalBlue], label: p[Kg::oniViolet], ty: p[Kg::waveAqua2],
    property: p[Kg::carpYellow], operator: p[Kg::boatYellow2], ret: p[Kg::peachRed],
    punctuation: p[Kg::springViolet2], background: p[Kg::sumiInk3],
    background_dim: p[Kg::sumiInk1], border: p[Kg::sumiInk2], selection: p[Kg::waveBlue1],
    warning: p[Kg::roninYellow], error_bg: p[Kg::winterRed], warning_bg: p[Kg::winterYellow],
);

kanagawa!(kanagawa_dragon; p;
    comment: p[Kg::dragonGray], error: p[Kg::samuraiRed], number: p[Kg::dragonPink],
    boolean: p[Kg::dragonOrange], string: p[Kg::dragonGreen2], url: p[Kg::dragonBlue2],
    text: p[Kg::dragonWhite], builtin: p[Kg::dragonRed], parameter: p[Kg::dragonViolet],
    function: p[Kg::dragonBlue], label: p[Kg::dragonViolet], ty: p[Kg::dragonAqua],
    property: p[Kg::dragonYellow], operator: p[Kg::dragonRed], ret: p[Kg::dragonRed],
    punctuation: p[Kg::dragonViolet], background: p[Kg::dragonBlack3],
    background_dim: p[Kg::dragonBlack1], border: p[Kg::dragonBlack2],
    selection: p[Kg::dragonBlack6], warning: p[Kg::dragonYellow],
    error_bg: p[Kg::winterRed], warning_bg: p[Kg::winterYellow],
);

kanagawa!(kanagawa_lotus; p;
    comment: p[Kg::lotusGray3], error: p[Kg::lotusRed], number: p[Kg::lotusViolet3],
    boolean: p[Kg::lotusOrange], string: p[Kg::lotusGreen], url: p[Kg::lotusTeal3],
    text: p[Kg::lotusInk1], builtin: p[Kg::lotusRed2], parameter: p[Kg::lotusBlue2],
    function: p[Kg::lotusBlue4], label: p[Kg::lotusViolet1], ty: p[Kg::lotusAqua],
    property: p[Kg::lotusYellow], operator: p[Kg::lotusYellow2], ret: p[Kg::lotusRed2],
    punctuation: p[Kg::lotusTeal2], background: p[Kg::lotusWhite3],
    background_dim: p[Kg::lotusWhite1], border: p[Kg::lotusWhite2],
    selection: p[Kg::lotusWhite4], warning: p[Kg::lotusOrange2],
    error_bg: p[Kg::lotusRed4], warning_bg: p[Kg::lotusYellow4],
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::controller::settings::theme::SCHEMES;

    #[test]
    fn every_scheme_fills_the_complete_group_space() {
        for groups in SCHEMES
            .iter()
            .flat_map(|scheme| scheme.variants())
            .map(|variant| variant.groups())
        {
            assert!(groups.highlight_groups().iter().all(|&(_, style)| {
                style.fg.is_some() || style.bg.is_some() || style.underline_color.is_some()
            }));
        }
    }

    #[test]
    fn groups_have_one_uniform_indexed_representation() {
        let groups = Groups::default();
        assert_eq!(groups.style(HLGroup::Number), groups[HLGroup::Number]);
        assert_eq!(groups.style(HLGroup::Operator), groups[HLGroup::Operator]);
    }

    #[test]
    fn special_colours_drive_underlines_without_replacing_foregrounds() {
        let groups = Groups::default();
        assert_eq!(groups.diagnostic().fg, None);
        assert_eq!(groups.diagnostic().bg, None);
        assert_eq!(groups.diagnostic().underline_color, groups.error().fg);
        assert!(groups
            .diagnostic()
            .add_modifier
            .contains(Modifier::UNDERLINED));
        assert_eq!(groups.border().underline_color, groups.border().fg);
    }

    #[test]
    fn diagnostic_surfaces_supply_both_colours() {
        for groups in SCHEMES
            .iter()
            .flat_map(|scheme| scheme.variants())
            .map(|variant| variant.groups())
        {
            for surface in [groups.error_bg(), groups.warning_bg()] {
                assert!(surface.fg.is_some() && surface.bg.is_some());
            }
        }
    }
}
