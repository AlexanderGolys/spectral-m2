use ratatui::style::{Color, Modifier, Style};

use crate::code::highlight::MacaulayHighlight;

use super::PaletteData;

/// Scheme-specific colors assigned to scheme-independent UI roles.
#[derive(Clone, Copy)]
struct UiPalette {
    background: Color,
    background_dim: Color,
    border: Color,
    selection: Color,
    text: Color,
    text_muted: Color,
    error: Color,
    warning: Color,
    yellow: Color,
    purple: Color,
    prompt: Color,
}

/// A palette slot referenced by a scheme-independent UI highlight group.
#[derive(Clone, Copy)]
enum PaletteColor {
    Background,
    BackgroundDim,
    Border,
    Selection,
    Text,
    TextMuted,
    Error,
    Warning,
    Yellow,
    Purple,
    Prompt,
}

impl PaletteColor {
    fn resolve(self, palette: UiPalette) -> Color {
        match self {
            Self::Background => palette.background,
            Self::BackgroundDim => palette.background_dim,
            Self::Border => palette.border,
            Self::Selection => palette.selection,
            Self::Text => palette.text,
            Self::TextMuted => palette.text_muted,
            Self::Error => palette.error,
            Self::Warning => palette.warning,
            Self::Yellow => palette.yellow,
            Self::Purple => palette.purple,
            Self::Prompt => palette.prompt,
        }
    }
}

/// A foreground supplied by either the palette or a scheme's syntax mapping.
#[derive(Clone, Copy)]
enum Foreground {
    Palette(PaletteColor),
    Macaulay(MacaulayHighlight),
}

/// Declarative styling for one UI highlight group.
#[derive(Clone, Copy)]
struct GroupConfig {
    foreground: Option<Foreground>,
    background: Option<PaletteColor>,
    special: Option<PaletteColor>,
    modifiers: Modifier,
}

impl GroupConfig {
    const fn foreground(color: PaletteColor) -> Self {
        Self {
            foreground: Some(Foreground::Palette(color)),
            background: None,
            special: None,
            modifiers: Modifier::empty(),
        }
    }

    const fn background(color: PaletteColor) -> Self {
        Self {
            foreground: None,
            background: Some(color),
            special: None,
            modifiers: Modifier::empty(),
        }
    }

    const fn macaulay(kind: MacaulayHighlight) -> Self {
        Self {
            foreground: Some(Foreground::Macaulay(kind)),
            background: None,
            special: None,
            modifiers: Modifier::empty(),
        }
    }

    const fn special(color: PaletteColor) -> Self {
        Self {
            foreground: None,
            background: None,
            special: Some(color),
            modifiers: Modifier::empty(),
        }
    }

    const fn with_special(mut self, color: PaletteColor) -> Self {
        self.special = Some(color);
        self
    }

    const fn modifiers(mut self, modifiers: Modifier) -> Self {
        self.modifiers = modifiers;
        self
    }

    fn resolve(self, palette: UiPalette, macaulay: &[Style; MacaulayHighlight::COUNT]) -> Style {
        let mut style = match self.foreground {
            Some(Foreground::Palette(color)) => Style::default().fg(color.resolve(palette)),
            Some(Foreground::Macaulay(kind)) => macaulay[kind.index()],
            None => Style::default(),
        };
        if let Some(background) = self.background {
            style = style.bg(background.resolve(palette));
        }
        if let Some(special) = self.special {
            style = style.underline_color(special.resolve(palette));
        }
        style.add_modifier(self.modifiers)
    }
}

macro_rules! ui_groups {
    ($($name:ident: $config:expr),+ $(,)?) => {
        /// The complete scheme-independent assignment of UI elements to highlight groups.
        #[derive(Clone, Copy)]
        struct UiGroups {
            $($name: GroupConfig),+
        }

        const UI_GROUPS: UiGroups = UiGroups {
            $($name: $config),+
        };

        /// UI and semantic highlight groups resolved from one palette.
        #[derive(Clone, Copy)]
        pub struct Groups {
            $($name: Style),+,
            macaulay: [Style; MacaulayHighlight::COUNT],
        }

        impl Groups {
            fn from_palette(
                palette: &PaletteData,
                ui: UiPalette,
                macaulay_style: fn(MacaulayHighlight, &PaletteData) -> Style,
            ) -> Self {
                let macaulay = std::array::from_fn(|index| {
                    let highlight = MacaulayHighlight::from_index(index)
                        .expect("Macaulay highlight indices are contiguous");
                    macaulay_style(highlight, palette)
                });
                Self {
                    $($name: UI_GROUPS.$name.resolve(ui, &macaulay)),+,
                    macaulay,
                }
            }

            $(pub fn $name(&self) -> Style {
                self.$name
            })+

            pub fn highlight_groups(&self) -> Vec<(&'static str, Style)> {
                let mut groups = vec![$((concat!("ui.", stringify!($name)), self.$name)),+];
                groups.extend(MacaulayHighlight::ALL.iter().map(|&highlight| {
                    (highlight.group_name(), self.macaulay(highlight))
                }));
                groups
            }
        }
    };
}

ui_groups! {
    transcript: GroupConfig::background(PaletteColor::Background),
    code: GroupConfig::background(PaletteColor::BackgroundDim),
    picker: GroupConfig::background(PaletteColor::Background),
    picker_selected: GroupConfig::background(PaletteColor::BackgroundDim),
    value: GroupConfig::foreground(PaletteColor::Text),
    error: GroupConfig::foreground(PaletteColor::Error).with_special(PaletteColor::Error),
    warning: GroupConfig::foreground(PaletteColor::Warning).with_special(PaletteColor::Warning),
    preview_mode: GroupConfig::foreground(PaletteColor::Yellow),
    tree_code: GroupConfig::foreground(PaletteColor::Purple),
    diagnostic: GroupConfig::special(PaletteColor::Error).modifiers(Modifier::UNDERLINED),
    type_info: GroupConfig::macaulay(MacaulayHighlight::Type).modifiers(Modifier::ITALIC),
    border: GroupConfig::foreground(PaletteColor::Border).with_special(PaletteColor::Border),
    prompt: GroupConfig::foreground(PaletteColor::Prompt),
    selection: GroupConfig::background(PaletteColor::Selection),
    muted: GroupConfig::foreground(PaletteColor::TextMuted),
    printed: GroupConfig::macaulay(MacaulayHighlight::String),
}

impl Groups {
    pub fn error_border(&self) -> Style {
        self.error
    }

    pub fn warning_border(&self) -> Style {
        self.warning
    }

    pub fn macaulay(&self, highlight: MacaulayHighlight) -> Style {
        self.macaulay[highlight.index()]
    }
}

pub fn catppuccin_groups(palette: &PaletteData) -> Groups {
    Groups::from_palette(
        palette,
        UiPalette {
            background: palette.bg[2],
            background_dim: palette.bg[1],
            border: palette.bg[5],
            selection: palette.bg[5],
            text: palette.fg[2],
            text_muted: palette.gray[0],
            error: palette.red[0],
            warning: palette.yellow[0],
            yellow: palette.yellow[0],
            purple: palette.purple[0],
            prompt: palette.aqua[0],
        },
        catppuccin,
    )
}

pub fn tokyo_night_groups(palette: &PaletteData) -> Groups {
    Groups::from_palette(
        palette,
        UiPalette {
            background: palette.bg[2],
            background_dim: palette.bg[1],
            border: palette.bg[4],
            selection: palette.bg[5],
            text: palette.fg[1],
            text_muted: palette.fg[0],
            error: palette.red[1],
            warning: palette.yellow[0],
            yellow: palette.yellow[0],
            purple: palette.purple[0],
            prompt: palette.aqua[0],
        },
        tokyo_night,
    )
}

pub fn rose_pine_groups(palette: &PaletteData) -> Groups {
    Groups::from_palette(
        palette,
        UiPalette {
            background: palette.bg[1],
            background_dim: palette.bg[0],
            border: palette.bg[4],
            selection: palette.bg[5],
            text: palette.fg[0],
            text_muted: palette.gray[1],
            error: palette.red[0],
            warning: palette.yellow[0],
            yellow: palette.yellow[0],
            purple: palette.purple[0],
            prompt: palette.aqua[0],
        },
        rose_pine,
    )
}

fn kanagawa_groups(
    palette: &PaletteData,
    warning: Color,
    macaulay: fn(MacaulayHighlight, &PaletteData) -> Style,
) -> Groups {
    Groups::from_palette(
        palette,
        UiPalette {
            background: palette.bg[3],
            background_dim: palette.bg[1],
            border: palette.bg[2],
            selection: palette.bg[6],
            text: palette.fg[1],
            text_muted: palette.fg[0],
            error: palette.red[0],
            warning,
            yellow: palette.yellow[0],
            purple: palette.purple[0],
            prompt: palette.aqua[0],
        },
        macaulay,
    )
}

pub fn kanagawa_wave_groups(palette: &PaletteData) -> Groups {
    kanagawa_groups(palette, palette.yellow[3], kanagawa_wave)
}

pub fn kanagawa_dragon_groups(palette: &PaletteData) -> Groups {
    kanagawa_groups(palette, palette.yellow[2], kanagawa_dragon)
}

pub fn kanagawa_lotus_groups(palette: &PaletteData) -> Groups {
    kanagawa_groups(palette, palette.orange[1], kanagawa_lotus)
}

pub fn gruvbox_groups(palette: &PaletteData) -> Groups {
    Groups::from_palette(
        palette,
        UiPalette {
            background: palette.bg[0],
            background_dim: palette.bg[0],
            border: palette.bg[3],
            selection: palette.bg[3],
            text: palette.fg[1],
            text_muted: palette.gray[0],
            error: palette.red[1],
            warning: palette.yellow[1],
            yellow: palette.yellow[1],
            purple: palette.purple[0],
            prompt: palette.aqua[1],
        },
        gruvbox,
    )
}

pub fn gruvbox_material_groups(palette: &PaletteData) -> Groups {
    Groups::from_palette(
        palette,
        UiPalette {
            background: palette.bg[1],
            background_dim: palette.bg[0],
            border: palette.bg[4],
            selection: palette.bg[13],
            text: palette.fg[0],
            text_muted: palette.gray[1],
            error: palette.red[0],
            warning: palette.yellow[0],
            yellow: palette.yellow[0],
            purple: palette.purple[0],
            prompt: palette.aqua[0],
        },
        gruvbox_material,
    )
}

pub fn everforest_groups(palette: &PaletteData) -> Groups {
    let colors = palette.spectrum(0);
    Groups::from_palette(
        palette,
        UiPalette {
            background: palette.bg[1],
            background_dim: palette.bg[0],
            border: palette.bg[4],
            selection: palette.bg[7],
            text: palette.fg[0],
            text_muted: palette.gray[1],
            error: colors.red,
            warning: colors.yellow,
            yellow: colors.yellow,
            purple: colors.purple,
            prompt: colors.aqua,
        },
        everforest,
    )
}

impl Default for Groups {
    fn default() -> Self {
        super::Appearance::default().groups()
    }
}

fn fg(color: Color) -> Style {
    Style::default().fg(color)
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

fn catppuccin(kind: MacaulayHighlight, palette: &PaletteData) -> Style {
    match kind {
        MacaulayHighlight::Comment => italic(palette.gray[2]),
        MacaulayHighlight::Error => fg(palette.red[0]),
        MacaulayHighlight::Int | MacaulayHighlight::Float | MacaulayHighlight::Boolean => {
            fg(palette.orange[0])
        }
        MacaulayHighlight::String => fg(palette.green[0]),
        MacaulayHighlight::StringUrl => underlined(palette.blue[0]).add_modifier(Modifier::ITALIC),
        MacaulayHighlight::CharacterSpecial => fg(palette.purple[1]),
        MacaulayHighlight::Variable => fg(palette.fg[2]),
        MacaulayHighlight::VariableBuiltin => fg(palette.red[0]),
        MacaulayHighlight::Parameter => fg(palette.red[1]),
        MacaulayHighlight::Constant | MacaulayHighlight::CompiledFunction => fg(palette.orange[0]),
        MacaulayHighlight::Function
        | MacaulayHighlight::FunctionCall
        | MacaulayHighlight::Macro => fg(palette.blue[0]),
        MacaulayHighlight::Label => fg(palette.blue[1]),
        MacaulayHighlight::Type | MacaulayHighlight::TypeParameter => fg(palette.yellow[0]),
        MacaulayHighlight::Property => fg(palette.blue[2]),
        MacaulayHighlight::Package | MacaulayHighlight::Namespace => italic(palette.yellow[0]),
        MacaulayHighlight::Operator => fg(palette.aqua[1]),
        MacaulayHighlight::Keyword
        | MacaulayHighlight::KeywordConditional
        | MacaulayHighlight::KeywordRepeat
        | MacaulayHighlight::KeywordReturn
        | MacaulayHighlight::KeywordException
        | MacaulayHighlight::KeywordModifier
        | MacaulayHighlight::KeywordDebug
        | MacaulayHighlight::KeywordOperator => fg(palette.purple[0]),
        MacaulayHighlight::PunctuationBracket | MacaulayHighlight::PunctuationDelimiter => {
            fg(palette.gray[2])
        }
    }
}

fn tokyo_night(kind: MacaulayHighlight, palette: &PaletteData) -> Style {
    match kind {
        MacaulayHighlight::Comment => italic(palette.gray[0]),
        MacaulayHighlight::Error => fg(palette.red[1]),
        MacaulayHighlight::Int | MacaulayHighlight::Float | MacaulayHighlight::Boolean => {
            fg(palette.orange[0])
        }
        MacaulayHighlight::String => fg(palette.green[0]),
        MacaulayHighlight::StringUrl => underlined(palette.fg[1]),
        MacaulayHighlight::CharacterSpecial | MacaulayHighlight::Constant => fg(palette.blue[1]),
        MacaulayHighlight::Variable => fg(palette.fg[1]),
        MacaulayHighlight::VariableBuiltin => fg(palette.red[0]),
        MacaulayHighlight::Parameter => fg(palette.yellow[0]),
        MacaulayHighlight::Function
        | MacaulayHighlight::FunctionCall
        | MacaulayHighlight::Macro => fg(palette.blue[0]),
        MacaulayHighlight::CompiledFunction => fg(palette.blue[1]),
        MacaulayHighlight::Label => fg(palette.blue[0]),
        MacaulayHighlight::Type | MacaulayHighlight::TypeParameter => fg(palette.blue[1]),
        MacaulayHighlight::Property => fg(palette.green[1]),
        MacaulayHighlight::Package | MacaulayHighlight::Namespace => fg(palette.aqua[0]),
        MacaulayHighlight::Operator
        | MacaulayHighlight::KeywordOperator
        | MacaulayHighlight::PunctuationDelimiter => fg(palette.blue[2]),
        MacaulayHighlight::Keyword
        | MacaulayHighlight::KeywordReturn
        | MacaulayHighlight::KeywordModifier => italic(palette.purple[0]),
        MacaulayHighlight::KeywordConditional
        | MacaulayHighlight::KeywordRepeat
        | MacaulayHighlight::KeywordException => fg(palette.purple[1]),
        MacaulayHighlight::KeywordDebug => fg(palette.orange[0]),
        MacaulayHighlight::PunctuationBracket => fg(palette.fg[0]),
    }
}

fn rose_pine(kind: MacaulayHighlight, palette: &PaletteData) -> Style {
    match kind {
        MacaulayHighlight::Comment => italic(palette.gray[1]),
        MacaulayHighlight::Error => fg(palette.red[0]),
        MacaulayHighlight::Int | MacaulayHighlight::Float => fg(palette.yellow[0]),
        MacaulayHighlight::Boolean => fg(palette.red[1]),
        MacaulayHighlight::String => fg(palette.yellow[0]),
        MacaulayHighlight::StringUrl => underlined(palette.purple[0]),
        MacaulayHighlight::CharacterSpecial => fg(palette.yellow[0]),
        MacaulayHighlight::Variable => italic(palette.fg[0]),
        MacaulayHighlight::VariableBuiltin => italic(palette.red[0]).add_modifier(Modifier::BOLD),
        MacaulayHighlight::Parameter => italic(palette.purple[0]),
        MacaulayHighlight::Constant => fg(palette.yellow[0]),
        MacaulayHighlight::Function
        | MacaulayHighlight::FunctionCall
        | MacaulayHighlight::Macro => fg(palette.red[1]),
        MacaulayHighlight::CompiledFunction => bold(palette.red[1]),
        MacaulayHighlight::Label => fg(palette.aqua[0]),
        MacaulayHighlight::Type | MacaulayHighlight::TypeParameter => fg(palette.aqua[0]),
        MacaulayHighlight::Property => italic(palette.aqua[0]),
        MacaulayHighlight::Package | MacaulayHighlight::Namespace => fg(palette.fg[0]),
        MacaulayHighlight::Operator | MacaulayHighlight::KeywordOperator => fg(palette.gray[1]),
        MacaulayHighlight::Keyword
        | MacaulayHighlight::KeywordConditional
        | MacaulayHighlight::KeywordRepeat
        | MacaulayHighlight::KeywordReturn
        | MacaulayHighlight::KeywordException => fg(palette.blue[0]),
        MacaulayHighlight::KeywordModifier => fg(palette.aqua[0]),
        MacaulayHighlight::KeywordDebug => fg(palette.red[1]),
        MacaulayHighlight::PunctuationBracket | MacaulayHighlight::PunctuationDelimiter => {
            fg(palette.gray[1])
        }
    }
}

/// Kanagawa colors assigned by semantic role.
#[derive(Clone, Copy)]
struct KanagawaColors {
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
    r#type: Color,
    property: Color,
    operator: Color,
    r#return: Color,
    punctuation: Color,
}

fn kanagawa_wave(kind: MacaulayHighlight, palette: &PaletteData) -> Style {
    kanagawa(
        kind,
        KanagawaColors {
            comment: palette.gray[0],
            error: palette.red[0],
            number: palette.purple[4],
            boolean: palette.orange[0],
            string: palette.green[0],
            url: palette.blue[1],
            text: palette.fg[1],
            builtin: palette.red[1],
            parameter: palette.purple[1],
            function: palette.blue[0],
            label: palette.purple[0],
            r#type: palette.aqua[0],
            property: palette.yellow[0],
            operator: palette.yellow[1],
            r#return: palette.red[2],
            punctuation: palette.purple[2],
        },
    )
}

fn kanagawa_dragon(kind: MacaulayHighlight, palette: &PaletteData) -> Style {
    kanagawa(
        kind,
        KanagawaColors {
            comment: palette.gray[0],
            error: palette.red[0],
            number: palette.purple[4],
            boolean: palette.orange[0],
            string: palette.green[0],
            url: palette.blue[1],
            text: palette.fg[1],
            builtin: palette.red[1],
            parameter: palette.purple[1],
            function: palette.blue[0],
            label: palette.purple[0],
            r#type: palette.aqua[0],
            property: palette.yellow[0],
            operator: palette.red[1],
            r#return: palette.red[1],
            punctuation: palette.purple[2],
        },
    )
}

fn kanagawa_lotus(kind: MacaulayHighlight, palette: &PaletteData) -> Style {
    kanagawa(
        kind,
        KanagawaColors {
            comment: palette.gray[0],
            error: palette.red[0],
            number: palette.purple[1],
            boolean: palette.orange[0],
            string: palette.green[0],
            url: palette.aqua[3],
            text: palette.fg[1],
            builtin: palette.red[1],
            parameter: palette.blue[1],
            function: palette.blue[0],
            label: palette.purple[0],
            r#type: palette.aqua[0],
            property: palette.yellow[0],
            operator: palette.yellow[1],
            r#return: palette.red[1],
            punctuation: palette.aqua[2],
        },
    )
}

fn kanagawa(kind: MacaulayHighlight, colors: KanagawaColors) -> Style {
    match kind {
        MacaulayHighlight::Comment => italic(colors.comment),
        MacaulayHighlight::Error => fg(colors.error),
        MacaulayHighlight::Int | MacaulayHighlight::Float => fg(colors.number),
        MacaulayHighlight::Boolean => bold(colors.boolean),
        MacaulayHighlight::String | MacaulayHighlight::CharacterSpecial => fg(colors.string),
        MacaulayHighlight::StringUrl => underlined(colors.url),
        MacaulayHighlight::Variable => fg(colors.text),
        MacaulayHighlight::VariableBuiltin => italic(colors.builtin),
        MacaulayHighlight::Parameter => fg(colors.parameter),
        MacaulayHighlight::Constant => fg(colors.boolean),
        MacaulayHighlight::Function
        | MacaulayHighlight::FunctionCall
        | MacaulayHighlight::Macro => fg(colors.function),
        MacaulayHighlight::CompiledFunction => bold(colors.function),
        MacaulayHighlight::Label => bold(colors.label),
        MacaulayHighlight::Type | MacaulayHighlight::TypeParameter => fg(colors.r#type),
        MacaulayHighlight::Property => fg(colors.property),
        MacaulayHighlight::Package | MacaulayHighlight::Namespace => fg(colors.r#type),
        MacaulayHighlight::Operator => fg(colors.operator),
        MacaulayHighlight::Keyword | MacaulayHighlight::KeywordModifier => italic(colors.label),
        MacaulayHighlight::KeywordConditional | MacaulayHighlight::KeywordRepeat => {
            bold(colors.label)
        }
        MacaulayHighlight::KeywordReturn | MacaulayHighlight::KeywordException => {
            italic(colors.r#return)
        }
        MacaulayHighlight::KeywordDebug => fg(colors.url),
        MacaulayHighlight::KeywordOperator => bold(colors.operator),
        MacaulayHighlight::PunctuationBracket | MacaulayHighlight::PunctuationDelimiter => {
            fg(colors.punctuation)
        }
    }
}

fn gruvbox(kind: MacaulayHighlight, palette: &PaletteData) -> Style {
    match kind {
        MacaulayHighlight::Comment => italic(palette.gray[0]),
        MacaulayHighlight::Error => bold(palette.red[1]).add_modifier(Modifier::REVERSED),
        MacaulayHighlight::Int | MacaulayHighlight::Float | MacaulayHighlight::Boolean => {
            fg(palette.purple[1])
        }
        MacaulayHighlight::String => italic(palette.green[1]),
        MacaulayHighlight::StringUrl => underlined(palette.blue[1]),
        MacaulayHighlight::CharacterSpecial | MacaulayHighlight::Constant => fg(palette.orange[1]),
        MacaulayHighlight::Variable | MacaulayHighlight::Package | MacaulayHighlight::Namespace => {
            fg(palette.fg[1])
        }
        MacaulayHighlight::VariableBuiltin | MacaulayHighlight::CompiledFunction => {
            fg(palette.orange[1])
        }
        MacaulayHighlight::Parameter | MacaulayHighlight::Property => fg(palette.blue[1]),
        MacaulayHighlight::Function
        | MacaulayHighlight::FunctionCall
        | MacaulayHighlight::Macro => bold(palette.green[1]),
        MacaulayHighlight::Label
        | MacaulayHighlight::Keyword
        | MacaulayHighlight::KeywordConditional
        | MacaulayHighlight::KeywordRepeat
        | MacaulayHighlight::KeywordReturn
        | MacaulayHighlight::KeywordException
        | MacaulayHighlight::KeywordModifier
        | MacaulayHighlight::KeywordOperator => fg(palette.red[1]),
        MacaulayHighlight::Type | MacaulayHighlight::TypeParameter => fg(palette.yellow[1]),
        MacaulayHighlight::Operator
        | MacaulayHighlight::KeywordDebug
        | MacaulayHighlight::PunctuationBracket
        | MacaulayHighlight::PunctuationDelimiter => fg(palette.orange[1]),
    }
}

fn gruvbox_material(kind: MacaulayHighlight, palette: &PaletteData) -> Style {
    sainnhe(kind, palette, true)
}

fn everforest(kind: MacaulayHighlight, palette: &PaletteData) -> Style {
    sainnhe(kind, palette, false)
}

fn sainnhe(kind: MacaulayHighlight, palette: &PaletteData, bold_functions: bool) -> Style {
    match kind {
        MacaulayHighlight::Comment => italic(palette.gray[1]),
        MacaulayHighlight::Error => fg(palette.red[0]),
        MacaulayHighlight::Int | MacaulayHighlight::Float | MacaulayHighlight::Boolean => {
            fg(palette.purple[0])
        }
        MacaulayHighlight::String => fg(palette.aqua[0]),
        MacaulayHighlight::StringUrl => underlined(palette.blue[0]),
        MacaulayHighlight::CharacterSpecial => fg(palette.yellow[0]),
        MacaulayHighlight::Variable => fg(palette.fg[0]),
        MacaulayHighlight::VariableBuiltin | MacaulayHighlight::Constant => {
            italic(palette.purple[0])
        }
        MacaulayHighlight::Parameter => fg(palette.fg[0]),
        MacaulayHighlight::Function
        | MacaulayHighlight::FunctionCall
        | MacaulayHighlight::CompiledFunction
        | MacaulayHighlight::Macro => {
            if bold_functions {
                bold(palette.green[0])
            } else {
                fg(palette.green[0])
            }
        }
        MacaulayHighlight::Label
        | MacaulayHighlight::Operator
        | MacaulayHighlight::KeywordOperator => fg(palette.orange[0]),
        MacaulayHighlight::Type | MacaulayHighlight::TypeParameter => italic(palette.yellow[0]),
        MacaulayHighlight::Property => fg(palette.blue[0]),
        MacaulayHighlight::Package | MacaulayHighlight::Namespace => italic(palette.yellow[0]),
        MacaulayHighlight::Keyword
        | MacaulayHighlight::KeywordConditional
        | MacaulayHighlight::KeywordRepeat
        | MacaulayHighlight::KeywordReturn
        | MacaulayHighlight::KeywordException
        | MacaulayHighlight::KeywordModifier => fg(palette.red[0]),
        MacaulayHighlight::KeywordDebug => fg(palette.orange[0]),
        MacaulayHighlight::PunctuationBracket => fg(palette.fg[0]),
        MacaulayHighlight::PunctuationDelimiter => fg(palette.gray[1]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::controller::settings::theme::SCHEMES;

    #[test]
    fn mappings_keep_the_installed_neovim_differences() {
        let catppuccin = SCHEMES[0].variants()[0].groups();
        let tokyo = SCHEMES[1].variants()[0].groups();
        let gruvbox = SCHEMES[2].variants()[1].groups();
        let material = SCHEMES[3].variants()[1].groups();
        let everforest = SCHEMES[4].variants()[1].groups();

        assert_eq!(
            catppuccin.macaulay(MacaulayHighlight::KeywordOperator).fg,
            catppuccin.macaulay(MacaulayHighlight::Keyword).fg
        );
        assert_eq!(
            tokyo.macaulay(MacaulayHighlight::KeywordOperator).fg,
            tokyo.macaulay(MacaulayHighlight::Operator).fg
        );
        assert_eq!(
            gruvbox.macaulay(MacaulayHighlight::PunctuationBracket).fg,
            gruvbox.macaulay(MacaulayHighlight::Operator).fg
        );
        assert!(material
            .macaulay(MacaulayHighlight::Function)
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(!everforest
            .macaulay(MacaulayHighlight::Function)
            .add_modifier
            .contains(Modifier::BOLD));
        assert_eq!(
            catppuccin.macaulay(MacaulayHighlight::Property).fg,
            Some(Color::Rgb(0xb4, 0xbe, 0xfe))
        );
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
}
