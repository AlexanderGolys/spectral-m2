use std::ops::Index;
#[cfg(not(test))]
use std::sync::{LazyLock, RwLock};

use ratatui::style::{Modifier, Style};
use serde::Deserialize;
use these_macros_should_be_illegal::{enum_fn, strutuct};

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

/// `style` underlined in its own colour.
fn underlined(style: Style) -> Style {
    Style {
        underline_color: style.fg,
        ..style
    }
    .underlined()
}

/// Only an underline, drawn in `style`'s colour, over whatever it marks.
fn underline_in(style: Style) -> Style {
    Style {
        underline_color: style.fg,
        ..Style::new()
    }
    .underlined()
}

/// Declares the group set once, in two layers, and derives from it the [`HLGroup`]
/// selector enum, the [`Groups`] style table, its resolution and indexing.
///
/// A scheme sets only the `base` styles from its palette. Every `linked` group is a
/// Rust expression over the resolved base styles, which a scheme may replace through
/// its optional overrides; so adding, renaming or relinking a group is one line here.
macro_rules! highlight_groups {
    (
        base { $($base:ident),+ $(,)? }
        linked($from:ident) { $($linked:ident = $link:expr),+ $(,)? }
    ) => {
        /// The complete scheme-independent space of syntax and interface styles.
        #[enum_fn(description: &'static str = stringify)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum HLGroup {
            $($base,)+
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
            $(pub $linked: Style),+
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
                Ok(Self {
                    $($base: $from.$base,)+
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
                    $((HLGroup::$linked.description(), self.$linked)),+
                ]
            }
        }

        impl Index<HLGroup> for Groups {
            type Output = Style;

            fn index(&self, group: HLGroup) -> &Self::Output {
                match group {
                    $(HLGroup::$base => &self.$base,)+
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
        Module,
        // Signals: what asks for attention, and each severity.
        Accent,
        Error,
        Warning,
        Info,
        Hint,
    }
    linked(base) {
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
        Parameter = base.Text,
        Property = base.Text,
        Installation = base.Function,
        CompiledFunction = base.Function,
        Builtin = base.Symbol,
        Quote = base.Symbol,
        Namespace = base.Module,
        Package = base.Module,
        Regex = underlined(base.Info),
        Documentation = underlined(base.Info),
        // Diagnostic underlines, coloured through the special (underline) colour.
        DiagnosticError = underline_in(base.Error),
        DiagnosticWarning = underline_in(base.Warning),
        DiagnosticInfo = underline_in(base.Info),
        DiagnosticHint = underline_in(base.Hint),
        // Transcript: values, printed text and the frames around them.
        Value = base.Text,
        FractionRule = underline_in(base.Operator),
        Printed = base.String.patch(base.Raised),
        HintText = base.Muted,
        Separator = Style {
            fg: base.Canvas.bg,
            ..Style::new()
        },
        ValueBorder = base.Faint,
        OutputNumber = base.Accent.bold(),
        ErrorBorder = base.Error,
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
    prompt: KeymapHint,
    selection: Selection,
    muted: Muted,
    printed: Printed,
    hint_text: HintText,
    error_border: ErrorBorder,
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
        let palette = scheme
            .colors
            .palette(Mode::Dark, name)
            .expect("variant palette");
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
    fn output_numbers_match_the_prompt_colour_with_bold_emphasis() {
        for groups in SCHEMES
            .iter()
            .flat_map(|scheme| scheme.variants())
            .map(|variant| variant.groups())
        {
            assert_eq!(groups.output_number().fg, groups.prompt().fg);
            assert!(groups.output_number().add_modifier.contains(Modifier::BOLD));
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
    fn group_names_describe_semantics_instead_of_palette_hues() {
        let groups = Groups::default().highlight_groups();
        assert!(groups.iter().all(|(name, _)| ![
            "White", "Gray", "Red", "Orange", "Yellow", "Green", "Aqua", "Teal", "Cyan", "Blue",
            "Purple", "Magenta", "Pink"
        ]
        .iter()
        .any(|color| name.starts_with(color))));
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
