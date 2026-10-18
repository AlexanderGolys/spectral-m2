use std::ops::Index;
#[cfg(not(test))]
use std::sync::{LazyLock, RwLock};

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use these_macros_should_be_illegal::enum_fn;

use super::palette::Palette;

/// A serialized style whose colors name entries in its scheme palette.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StyleSpec {
    fg: Option<String>,
    bg: Option<String>,
    sp: Option<String>,
    bold: bool,
    italic: bool,
    underline: bool,
    reverse: bool,
    crossed_out: bool,
}
impl StyleSpec {
    /// Turn the declared modifiers and palette-named colours into a concrete [`Style`].
    fn resolve(&self, palette: &Palette) -> Result<Style, String> {
        let mut modifiers = Modifier::empty();
        modifiers.set(Modifier::BOLD, self.bold);
        modifiers.set(Modifier::ITALIC, self.italic);
        modifiers.set(Modifier::UNDERLINED, self.underline);
        modifiers.set(Modifier::REVERSED, self.reverse);
        modifiers.set(Modifier::CROSSED_OUT, self.crossed_out);
        let mut style = Style::new().add_modifier(modifiers);
        if let Some(color) = palette.resolve(self.fg.as_deref())? {
            style = style.fg(color);
        }
        if let Some(color) = palette.resolve(self.bg.as_deref())? {
            style = style.bg(color);
        }
        if let Some(color) = palette.resolve(self.sp.as_deref())? {
            style = style.underline_color(color);
        }
        Ok(style)
    }
}

/// How much of a string's saturation printed text keeps, in percent.
const PRINTED_SATURATION: f32 = 55.0;
/// How far printed text's value rises toward white, in percent of what remains.
const PRINTED_LIGHTENING: f32 = 15.0;

/// `style` with its foreground desaturated to `saturation` percent of what it was and
/// lightened by `lightening` percent of the way to white, its hue untouched. A colour
/// that is not RGB has no channels to move and is left as it is.
fn softened(style: Style, saturation: f32, lightening: f32) -> Style {
    let Some(Color::Rgb(r, g, b)) = style.fg else {
        return style;
    };
    let [r, g, b] = [r, g, b].map(|channel| f32::from(channel) / 255.0);
    let (high, low) = (r.max(g).max(b), r.min(g).min(b));
    let chroma = high - low;
    let value = high + (1.0 - high) * lightening / 100.0;
    let kept = if high == 0.0 { 0.0 } else { chroma / high } * saturation / 100.0;
    let floor = value * (1.0 - kept);
    // The same position between the darkest and brightest channel as before, which is
    // what keeps the hue where it was.
    let channel = |from: f32| {
        let along = if chroma == 0.0 {
            0.0
        } else {
            (from - low) / chroma
        };
        let byte = (floor + (value - floor) * along) * 255.0;
        byte.round().clamp(0.0, 255.0) as u8
    };
    style.fg(Color::Rgb(channel(r), channel(g), channel(b)))
}

/// Darken a label's colour while keeping its hue and modifiers.
fn darkened(style: Style) -> Style {
    match style.fg {
        Some(fg) => super::palette::blend(fg, Color::Rgb(0, 0, 0), 0.25)
            .map_or(style, |color| style.fg(color)),
        None => style,
    }
}

/// Only an underline, drawn in `style`'s colour, over whatever it marks.
fn underline_in(style: Style) -> Style {
    Style {
        underline_color: style.fg,
        ..Style::new()
    }
    .underlined()
}

/// Declares the base, fixed palette, and linked groups once, deriving the [`HLGroup`]
/// selector enum, the [`Groups`] style table, its resolution and indexing.
///
/// A scheme sets the `base` styles; fixed palette groups expose all resolved colors.
/// Every `linked` group is a
/// Rust expression over the resolved base styles, which a scheme may replace through
/// its optional overrides; so adding, renaming or relinking a group is one line here.
macro_rules! highlight_groups {
    (
        base { $($base:ident),+ $(,)? }
        palette { $($palette:ident: $channel:ident = $color:literal),+ $(,)? }
        linked($from:ident, $hue:ident) { $($linked:ident = $link:expr),+ $(,)? }
    ) => {
        /// The complete scheme-independent space of syntax and interface styles.
        #[enum_fn(description: &'static str = stringify)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum HLGroup {
            $($base,)+
            $($palette,)+
            $($linked),+
        }

        /// The styles a scheme sets from its palette, which every other group links to.
        #[allow(non_snake_case)]
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct Base<S = Style> {$(
            pub $base: S,
        )+}

        /// A scheme's own styles for linked groups, each replacing its link outright.
        #[allow(non_snake_case)]
        #[derive(Default, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        pub struct Overrides {$(
            $linked: Option<StyleSpec>,
        )+}

        /// Every highlight group's resolved style.
        #[allow(non_snake_case)]
        #[derive(Clone, Copy)]
        pub struct Groups {
            $(pub $base: Style,)+
            $(pub $palette: Style,)+
            $(pub $linked: Style,)+
        }

        impl Groups {
            /// Resolve the base styles from the palette, then every linked group from
            /// them unless the scheme overrides it, naming the offending group when a
            /// colour is unknown.
            pub fn resolve(
                base: &Base<StyleSpec>,
                overrides: &Overrides,
                palette: &Palette,
            ) -> Result<Self, String> {
                let resolve = |name: &str, spec: &StyleSpec| {
                    spec.resolve(palette).map_err(|error| format!("{name}: {error}"))
                };
                let $from = Base {$(
                    $base: resolve(stringify!($base), &base.$base)?,
                )+};
                let $hue = |name| -> Result<Style, String> {
                    Ok(Style::new().fg(palette.resolve(Some(name))?.expect("fixed palette colors exist")))
                };
                Ok(Self {
                    $($base: $from.$base,)+
                    $($palette: Style::new().$channel(palette.resolve(Some($color))?.expect("fixed palette colors exist")),)+
                    $($linked: match &overrides.$linked {
                        Some(spec) => resolve(stringify!($linked), spec)?,
                        None => $link,
                    }),+
                })
            }

            /// Every group's name paired with its style, in declaration order.
            pub fn highlight_groups(&self) -> Vec<(&'static str, Style)> {
                vec![
                    $((HLGroup::$base.description(), self.$base),)+
                    $((HLGroup::$palette.description(), self.$palette),)+
                    $((HLGroup::$linked.description(), self.$linked)),+
                ]
            }
        }

        impl Index<HLGroup> for Groups {
            type Output = Style;

            fn index(&self, group: HLGroup) -> &Self::Output {
                match group {
                    $(HLGroup::$base => &self.$base,)+
                    $(HLGroup::$palette => &self.$palette,)+
                    $(HLGroup::$linked => &self.$linked),+
                }
            }
        }
    };
}

highlight_groups! {
    base {
        // Surfaces: the transcript, code, raised popups and printed text, a pending
        // cell, selected text and the focused row of a list.
        Canvas,
        Code,
        Raised,
        Pending,
        Selection,
        Focus,
        // Text: ordinary, receding, and the faint lines that frame blocks.
        Text,
        Muted,
        Faint,
        // Syntax.
        Keyword,
        Function,
        Type,
        String,
        Number,
        Operator,
        Symbol,
        // Named after Neovim's semantic-token roles where one exists (`@lsp.type.*`);
        // the rest follow the tree-sitter captures they are styled from.
        Parameter,
        Property,
        EnumVariant,
        Macro,
        Attribute,
        Builtin,
        Regex,
        // Signals: what asks for attention, and each severity.
        Accent,
        Error,
        Warning,
        Info,
        Hint,
    }
    palette {
        Red: fg = "red",
        RedLight: fg = "red_light",
        RedDark: fg = "red_dark",
        Green: fg = "green",
        GreenLight: fg = "green_light",
        GreenDark: fg = "green_dark",
        Yellow: fg = "yellow",
        YellowLight: fg = "yellow_light",
        YellowDark: fg = "yellow_dark",
        Blue: fg = "blue",
        BlueLight: fg = "blue_light",
        BlueDark: fg = "blue_dark",
        Purple: fg = "purple",
        PurpleLight: fg = "purple_light",
        PurpleDark: fg = "purple_dark",
        Aqua: fg = "aqua",
        AquaLight: fg = "aqua_light",
        AquaDark: fg = "aqua_dark",
        Orange: fg = "orange",
        OrangeLight: fg = "orange_light",
        OrangeDark: fg = "orange_dark",
        Gray: fg = "gray",
        GrayLight: fg = "gray_light",
        GrayDark: fg = "gray_dark",
        Fg0: fg = "fg0",
        Fg1: fg = "fg1",
        Fg2: fg = "fg2",
        Fg3: fg = "fg3",
        Fg4: fg = "fg4",
        Bg0: bg = "bg0",
        Bg1: bg = "bg1",
        Bg2: bg = "bg2",
        Bg3: bg = "bg3",
        Bg4: bg = "bg4",
        BgRed: bg = "bg_red",
        BgGreen: bg = "bg_green",
        BgYellow: bg = "bg_yellow",
        BgBlue: bg = "bg_blue",
        BgPurple: bg = "bg_purple",
        BgAqua: bg = "bg_aqua",
        BgOrange: bg = "bg_orange",
        BgGray: bg = "bg_gray",
    }
    linked(base, hue) {
        // Tree-sitter captures of M2 source.
        Comment = base.Muted.italic(),
        Constant = base.Number,
        Boolean = base.Number,
        Float = base.Number,
        Statement = base.Keyword,
        Conditional = base.Keyword,
        Repeat = base.Keyword,
        Exception = base.Keyword,
        StorageClass = base.Keyword,
        Debug = base.Keyword,
        Delimiter = base.Muted,
        Punctuation = base.Muted,
        Variable = base.Text,
        Installation = base.Function,
        CompiledFunction = base.Function,
        Quote = base.Symbol,
        // Modules read as types, set apart by italics.
        Namespace = base.Type.italic(),
        Package = base.Type.italic(),
        Documentation = hue("blue_light")?,
        // Markup keeps syntax colours beneath emphasis, code and links; a link is
        // underlined only while the pointer is over it.
        MarkupHeading1 = hue("yellow_light")?.bold(),
        // A title's brief, beside its name.
        MarkupBrief = hue("yellow_light")?.italic(),
        MarkupHeading2 = base.Function.bold(),
        MarkupHeading3 = hue("aqua")?.bold(),
        MarkupStrong = Style::new().bold(),
        MarkupEmphasis = Style::new().italic(),
        MarkupStrongEmphasis = Style::new().bold().italic(),
        MarkupStrike = Style::new().crossed_out(),
        MarkupLinkHover = Style::new().underlined(),
        // Diagnostic underlines, coloured through the special (underline) colour.
        DiagnosticError = underline_in(base.Error),
        DiagnosticWarning = underline_in(base.Warning),
        DiagnosticInfo = underline_in(base.Info),
        DiagnosticHint = underline_in(base.Hint),
        // Transcript: values, printed text and the frames around them.
        Value = base.Text,
        FractionRule = underline_in(base.Operator),
        Printed = softened(base.String, PRINTED_SATURATION, PRINTED_LIGHTENING).patch(base.Raised),
        HintText = base.Muted,
        // @##Trash
        Separator = Style {
            fg: base.Canvas.bg,
            ..Style::new()
        },
        ValueBorder = base.Faint,
        // The latest three `oN` labels are bold; older labels are plain.
        OutputNumber = base.Accent,
        RecentOutput = base.Accent.bold(),
        RecentCellNumber = darkened(base.Accent),
        ErrorBorder = base.Error,
        ErrorMessage = hue("red_light")?,
        ErrorCode = base.Code.bg(hue("bg_red")?.fg.expect("fixed palette colors exist")),
        WarningBorder = base.Warning,
        InfoBorder = base.Info,
        // Status line segments and key-hint labels.
        Statusline1 = base.String,
        Statusline2 = base.Warning,
        Statusline3 = base.Error,
        StatuslineBg = base.Code,
        StatuslineError = base.Error,
        StatuslineWarning = base.Warning,
        KeymapHint = base.Accent,
        // A command typed after the command prefix, and its name in the picker.
        Command = base.Accent,
    }
}

/// The one scheme in effect, republished whenever the appearance changes.
///
/// A scheme is a mapping from [`HLGroup`] to [`Style`] — reflection over an enum, not
/// state any element owns — so it is read from here rather than carried through render
/// calls. Carrying it made every context a copy of every style and made a repaint look
/// like a rebuild. Each published table is leaked so a reader can hold `&'static
/// Groups` without a guard; republishing happens only when a person changes the theme.
#[cfg(not(test))]
static SCHEME: LazyLock<RwLock<&'static Groups>> =
    LazyLock::new(|| RwLock::new(Box::leak(Box::new(Groups::default()))));

/// Read the scheme currently in effect.
#[cfg(not(test))]
pub fn scheme() -> &'static Groups {
    *SCHEME
        .read()
        .expect("the scheme lock is never held across a panic")
}

/// Publish a newly resolved scheme for every later read.
#[cfg(not(test))]
pub fn publish(groups: Groups) {
    *SCHEME
        .write()
        .expect("the scheme lock is never held across a panic") = Box::leak(Box::new(groups));
}

// Each test runs on its own thread, so a per-thread scheme keeps one test's theme
// changes from repainting another running in parallel.
#[cfg(test)]
thread_local! {
    static SCHEME: std::cell::Cell<&'static Groups> =
        std::cell::Cell::new(Box::leak(Box::new(Groups::default())));
}

#[cfg(test)]
pub fn scheme() -> &'static Groups {
    SCHEME.get()
}

#[cfg(test)]
pub fn publish(groups: Groups) {
    SCHEME.set(Box::leak(Box::new(groups)));
}

impl Groups {
    /// The style of one group, selected by its [`HLGroup`] tag.
    pub fn style(&self, group: HLGroup) -> Style {
        self[group]
    }

    /// The underline-only style that marks a diagnostic span without recolouring its text.
    pub fn diagnostic(&self) -> Style {
        self.DiagnosticError
    }
}

/// Generates one named style accessor per interface role, each reading a single group.
/// The names say what the UI draws; the groups say what a scheme colours.
macro_rules! accessors {
    ($($name:ident: $group:ident),+ $(,)?) => {
        impl Groups {$(
            pub fn $name(&self) -> Style {
                self.$group
            }
        )+}
    };
}

// Interface role -> the group a scheme styles it with.
// @##Trash
accessors! {
    transcript: Canvas,
    code: Code,
    pending: Pending,
    separator: Separator,
    picker: Raised,
    picker_selected: Focus,
    value: Value,
    error: Error,
    warning: Warning,
    info: Info,
    preview_mode: Documentation,
    tree_code: Symbol,
    type_info: Type,
    border: ValueBorder,
    output_number: OutputNumber,
    recent_output: RecentOutput,
    recent_cell_number: RecentCellNumber,
    prompt: KeymapHint,
    command: Command,
    selection: Selection,
    muted: Muted,
    printed: Printed,
    hint_text: HintText,
    error_border: ErrorBorder,
    error_message: ErrorMessage,
    error_code: ErrorCode,
    warning_border: WarningBorder,
    info_border: InfoBorder,
    statusline_bg: StatuslineBg,
}

/// The default appearance's resolved scheme, used before any theme is published.
impl Default for Groups {
    fn default() -> Self {
        super::Appearance::default().groups()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::ui::controller::settings::theme::{Mode, SchemeFile, SCHEMES, SCHEME_FILES};

    #[test]
    fn every_palette_color_is_used_by_a_highlight_group() {
        for scheme in SCHEMES.iter() {
            for variant in scheme.variants() {
                let groups = variant.groups();
                let styles = groups.highlight_groups();
                for (name, color) in variant.palette().colors() {
                    assert!(
                        styles.iter().any(|(_, style)| {
                            let actual = if name.starts_with("bg") {
                                style.bg
                            } else {
                                style.fg
                            };
                            actual == Some(color)
                        }),
                        "{} {}: {name} needs a group in its intended channel",
                        scheme.label(),
                        variant.label()
                    );
                }
            }
        }
    }

    #[test]
    fn palette_groups_are_scheme_independent() {
        for scheme in SCHEMES.iter() {
            for variant in scheme.variants() {
                let groups = variant.groups();
                assert_eq!(groups.style(HLGroup::Bg0), groups.Bg0);
                assert_eq!(groups.style(HLGroup::BgRed), groups.BgRed);
                assert_eq!(groups.style(HLGroup::Green), groups.Green);
            }
        }
    }

    /// HSV saturation and hue of an RGB foreground, or None for any other colour.
    fn saturation_and_hue(style: Style) -> Option<(f32, f32)> {
        let Some(Color::Rgb(r, g, b)) = style.fg else {
            return None;
        };
        let [r, g, b] = [r, g, b].map(|channel| f32::from(channel) / 255.0);
        let (high, low) = (r.max(g).max(b), r.min(g).min(b));
        let chroma = high - low;
        let hue = if chroma == 0.0 {
            0.0
        } else if high == r {
            ((g - b) / chroma).rem_euclid(6.0)
        } else if high == g {
            (b - r) / chroma + 2.0
        } else {
            (r - g) / chroma + 4.0
        } * 60.0;
        Some((if high == 0.0 { 0.0 } else { chroma / high }, hue))
    }

    #[test]
    fn printed_text_is_a_quieter_shade_of_a_string() {
        for groups in SCHEMES
            .iter()
            .flat_map(|scheme| scheme.variants())
            .map(|variant| variant.groups())
        {
            let (string, printed) = (
                saturation_and_hue(groups.String).expect("schemes colour in RGB"),
                saturation_and_hue(groups.Printed).expect("schemes colour in RGB"),
            );
            assert!(
                printed.0 < string.0,
                "printed text should be less saturated"
            );
            let apart = (printed.1 - string.1).abs();
            assert!(
                apart.min(360.0 - apart) < 30.0,
                "printed text should keep the hue"
            );
        }
    }

    #[test]
    fn every_scheme_fills_the_complete_group_space() {
        for groups in SCHEMES
            .iter()
            .flat_map(|scheme| scheme.variants())
            .map(|variant| variant.groups())
        {
            assert!(groups.highlight_groups().iter().all(|&(_, style)| {
                style.fg.is_some()
                    || style.bg.is_some()
                    || style.underline_color.is_some()
                    || !style.add_modifier.is_empty()
                    || !style.sub_modifier.is_empty()
            }));
        }
    }

    #[test]
    fn linked_groups_follow_their_base_style() {
        let groups = Groups::default();
        assert_eq!(groups.Conditional, groups.Keyword);
        assert_eq!(groups.Comment.fg, groups.Muted.fg);
        assert_eq!(groups.OutputNumber.fg, groups.Accent.fg);
        assert_eq!(groups.FractionRule.underline_color, groups.Operator.fg);
        assert_eq!(groups.FractionRule.fg, None);
    }

    #[test]
    fn an_override_replaces_its_link_outright() {
        let scheme: SchemeFile = toml::from_str(SCHEME_FILES[0]).expect("embedded scheme parses");
        let name = &scheme.variants.dark[0];
        let palette = scheme.colors.palette(Mode::Dark, name).unwrap();
        let overrides = Overrides {
            Conditional: Some(StyleSpec {
                bold: true,
                ..StyleSpec::default()
            }),
            ..Overrides::default()
        };

        let linked = Groups::resolve(&scheme.base, &Overrides::default(), &palette).unwrap();
        let overridden = Groups::resolve(&scheme.base, &overrides, &palette).unwrap();
        assert_eq!(linked.Conditional, linked.Keyword);
        assert_eq!(overridden.Conditional, Style::new().bold());
        assert_eq!(overridden.Repeat, linked.Repeat);
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
    }

    #[test]
    fn diagnostic_text_does_not_fill_its_box() {
        for groups in SCHEMES
            .iter()
            .flat_map(|scheme| scheme.variants())
            .map(|variant| variant.groups())
        {
            for text in [groups.error(), groups.warning(), groups.info()] {
                assert!(text.fg.is_some() && text.bg.is_none());
            }
        }
    }

    #[test]
    fn value_borders_are_visible_beside_dark_separators() {
        for groups in SCHEMES
            .iter()
            .flat_map(|scheme| scheme.variants())
            .map(|variant| variant.groups())
        {
            assert_ne!(groups.border().fg, groups.separator().fg);
        }
    }

    #[test]
    fn documentation_groups_follow_the_requested_palette_colours() {
        for groups in SCHEMES
            .iter()
            .flat_map(|scheme| scheme.variants())
            .map(|variant| variant.groups())
        {
            assert_eq!(
                groups[HLGroup::Documentation].fg,
                groups[HLGroup::BlueLight].fg
            );
            assert_eq!(
                groups[HLGroup::MarkupHeading1].fg,
                groups[HLGroup::YellowLight].fg
            );
            assert_eq!(groups[HLGroup::MarkupHeading3].fg, groups[HLGroup::Aqua].fg);
            assert!(!groups[HLGroup::Documentation]
                .add_modifier
                .contains(Modifier::UNDERLINED));
            assert!(groups[HLGroup::MarkupLinkHover]
                .add_modifier
                .contains(Modifier::UNDERLINED));
        }
    }

    #[test]
    fn output_numbers_match_the_prompt_colour_and_only_the_recent_ones_are_bold() {
        for groups in SCHEMES
            .iter()
            .flat_map(|scheme| scheme.variants())
            .map(|variant| variant.groups())
        {
            assert_eq!(groups.output_number().fg, groups.prompt().fg);
            assert_eq!(groups.recent_output().fg, groups.prompt().fg);
            assert!(!groups.output_number().add_modifier.contains(Modifier::BOLD));
            assert!(groups.recent_output().add_modifier.contains(Modifier::BOLD));
        }
    }

    #[test]
    fn every_severity_has_its_own_underline() {
        for groups in SCHEMES
            .iter()
            .flat_map(|scheme| scheme.variants())
            .map(|variant| variant.groups())
        {
            for (diagnostic, severity) in [
                (groups.DiagnosticError, groups.Error),
                (groups.DiagnosticWarning, groups.Warning),
                (groups.DiagnosticInfo, groups.Info),
                (groups.DiagnosticHint, groups.Hint),
            ] {
                assert_eq!(diagnostic.underline_color, severity.fg);
                assert!(diagnostic.add_modifier.contains(Modifier::UNDERLINED));
            }
        }
    }

    #[test]
    fn transcript_surfaces_are_distinct() {
        for groups in SCHEMES
            .iter()
            .flat_map(|scheme| scheme.variants())
            .map(|variant| variant.groups())
        {
            assert_eq!(
                [
                    groups.transcript().bg,
                    groups.code().bg,
                    groups.printed().bg,
                ]
                .into_iter()
                .collect::<HashSet<_>>()
                .len(),
                3
            );
        }
    }

    #[test]
    fn pickers_reuse_printed_backgrounds_and_keep_their_old_selection_surface() {
        for groups in SCHEMES
            .iter()
            .flat_map(|scheme| scheme.variants())
            .map(|variant| variant.groups())
        {
            assert_eq!(groups.picker().bg, groups.printed().bg);
            assert!(groups.picker_selected().bg.is_some());
            assert!(groups.picker_selected().fg.is_none());
        }
    }
}
