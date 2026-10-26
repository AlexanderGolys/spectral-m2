//! Highlight groups: their TOML declarations and the resolved styles drawn from them.
//!
//! Styles are written as [`StyleDeclaration`]s in two places: the shared
//! `groups.toml` ([`GroupDeclarations`]), which lists every group with its default
//! link and Tree-sitter captures, and each scheme file, which colours the required
//! groups from its palette and may override any other. [`Groups::resolve`] layers
//! them (variant override, then scheme, then shared default), follows links and
//! channel references, applies colour adjustments, and yields one ratatui [`Style`]
//! per [`HLGroup`]. Rendering only ever sees the resolved [`Groups`].

use std::collections::HashMap;
use std::ops::Index;
use std::sync::LazyLock;
#[cfg(not(test))]
use std::sync::RwLock;

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use these_macros_should_be_illegal::enum_fn;

use super::palette::Palette;

/// One highlight group's style as written in TOML, before it is resolved into a
/// [`Style`]. Every key is optional, and a declaration replaces a lower layer's one for
/// the same group as a whole rather than merging with it.
///
/// Keys apply in field order: start from `link`, set the channels, toggle
/// modifiers, then adjust colours. Colours are native palette names; `Group.fg`,
/// `Group.bg` and `Group.sp` name another group's resolved channel.
#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StyleDeclaration {
    /// Only in `groups.toml`: every scheme must declare this group itself.
    required: bool,
    /// Only in `groups.toml`: the Tree-sitter captures this group draws.
    captures: Option<Vec<String>>,
    /// Group whose resolved style this one starts from.
    link: Option<String>,
    /// Palette colour for each channel (`sp` is the underline colour); `"none"`
    /// clears what a link supplied.
    fg: Option<String>,
    bg: Option<String>,
    sp: Option<String>,
    /// Channel copied from another group, so one group's colour can serve in a
    /// different channel here (an error foreground as an underline). Exclusive with
    /// the direct channel of the same name.
    fg_from: Option<String>,
    bg_from: Option<String>,
    sp_from: Option<String>,
    /// Modifiers; `false` removes one a link supplied.
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    reverse: Option<bool>,
    crossed_out: Option<bool>,
    /// Drawn by the terminal blended toward its background.
    dim: Option<bool>,
    /// Foreground kept at this percentage of its saturation, hue unchanged.
    saturation: Option<f32>,
    /// Foreground moved this percentage of the way to white.
    lightening: Option<f32>,
    /// Foreground blended toward black by this fraction.
    darken: Option<f32>,
    /// Background blended toward a palette colour by `bg_blend_amount`, a fraction,
    /// for schemes whose guidelines specify an overlay's opacity.
    bg_blend: Option<String>,
    bg_blend_amount: Option<f32>,
    /// Background blended toward another group's channel by `tint`, a fraction.
    tint_from: Option<String>,
    tint: Option<f32>,
}

/// Declarations keyed by highlight group name, as one TOML table holds them.
pub type StyleTable = HashMap<String, StyleDeclaration>;

/// The parsed `groups.toml`: the bottom layer of every scheme, declaring each group
/// once with its default link and captures and marking which groups a scheme must
/// colour itself. A user's copy may relink groups and move captures but not add
/// groups or change which are required.
pub struct GroupDeclarations(StyleTable);

const DEFAULTS: &str = include_str!("groups.toml");

impl GroupDeclarations {
    fn embedded() -> Self {
        Self(toml::from_str(DEFAULTS).expect("embedded group declarations must parse"))
    }

    /// Merge a user's declarations over the built-in defaults.
    pub fn parse(source: &str) -> Result<Self, String> {
        let custom: StyleTable = toml::from_str(source).map_err(|e| e.to_string())?;
        let mut declarations = Self::embedded();
        for (name, mut declaration) in custom {
            let original = declarations
                .0
                .get(&name)
                .ok_or_else(|| format!("unknown highlight group `{name}`"))?;
            if original.required && !declaration.required {
                return Err(format!(
                    "required group `{name}` cannot become a default link"
                ));
            }
            if !original.required && declaration.required {
                return Err(format!("linked group `{name}` cannot become required"));
            }
            declaration.required = original.required;
            if declaration.captures.is_none() {
                declaration.captures.clone_from(&original.captures);
            }
            declarations.0.insert(name, declaration);
        }
        declarations.captures()?;
        Ok(declarations)
    }

    /// Every Tree-sitter capture paired with the group that draws it.
    pub fn captures(&self) -> Result<Vec<(String, HLGroup)>, String> {
        let mut owners = HashMap::new();
        for (name, declaration) in &self.0 {
            let group = HLGroup::named(name)?;
            for capture in declaration.captures.iter().flatten() {
                if let Some(owner) = owners.insert(capture.clone(), group) {
                    return Err(format!(
                        "capture `@{capture}` belongs to both `{}` and `{name}`",
                        owner.description()
                    ));
                }
            }
        }
        Ok(owners.into_iter().collect())
    }

    fn load() -> Self {
        match super::config_source("groups.toml", DEFAULTS).and_then(|s| Self::parse(&s)) {
            Ok(declarations) => {
                // Validate against every built-in palette before publishing defaults.
                // A bad user link must not also break the fallback schemes.
                for (name, source) in super::SCHEME_FILES {
                    let file: super::SchemeFile =
                        toml::from_str(source).expect("embedded scheme must parse");
                    if let Err(error) = file.resolve_with(&declarations) {
                        eprintln!(
                            "highlight groups for {name}: {error}; using built-in declarations"
                        );
                        return Self::embedded();
                    }
                }
                declarations
            }
            Err(error) => {
                eprintln!("highlight groups: {error}; using built-in declarations");
                Self::embedded()
            }
        }
    }
}

/// The user's `groups.toml` over the built-in one, or the built-in one alone when the
/// user's copy fails to parse or to resolve against any embedded scheme.
pub static GROUP_DECLARATIONS: LazyLock<GroupDeclarations> = LazyLock::new(GroupDeclarations::load);

/// Generate the selector, fields, reflection and indexing from the sole TOML list.
macro_rules! highlight_groups {
    ($($group:ident),+ $(,)?) => {
        /// Every syntax and interface highlight group.
        #[enum_fn(description: &'static str = stringify)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum HLGroup {$(
            #[doc = concat!("The `", stringify!($group), "` highlight group.")]
            $group,
        )+}

        impl HLGroup {
            fn named(name: &str) -> Result<Self, String> {
                match name {
                    $(stringify!($group) => Ok(Self::$group),)+
                    _ => Err(format!("unknown highlight group `{name}`")),
                }
            }
        }

        /// Every highlight group's resolved style.
        #[allow(non_snake_case)]
        #[derive(Clone, Copy)]
        pub struct Groups {$(
            #[doc = concat!("Resolved style for `", stringify!($group), "`.")]
            pub $group: Style,
        )+}

        impl Groups {
            /// Resolve every group from its topmost declaration: a variant's
            /// `overrides`, else the scheme's `styles`, else the shared `declarations`.
            /// Links and channel references resolve against the same layering.
            pub fn resolve(
                styles: &StyleTable,
                overrides: &StyleTable,
                palette: &Palette,
                declarations: &GroupDeclarations,
            ) -> Result<Self, String> {
                for (name, declaration) in styles.iter().chain(overrides) {
                    HLGroup::named(name)?;
                    if declaration.required || declaration.captures.is_some() {
                        return Err(format!(
                            "{name}: required and captures belong only in groups.toml"
                        ));
                    }
                }
                for (name, declaration) in &declarations.0 {
                    if declaration.required && !styles.contains_key(name) {
                        return Err(format!("missing required highlight group `{name}`"));
                    }
                }
                let mut resolver = StyleResolver {
                    declarations, styles, overrides, palette,
                    resolved: HashMap::new(), visiting: Vec::new(),
                };
                Ok(Self {$($group: resolver.group(stringify!($group))?,)+})
            }

            /// Every group's name and style, in declaration order.
            pub fn highlight_groups(&self) -> Vec<(&'static str, Style)> {
                vec![$((HLGroup::$group.description(), self.$group),)+]
            }
        }

        impl Index<HLGroup> for Groups {
            type Output = Style;
            fn index(&self, group: HLGroup) -> &Self::Output {
                match group {$(HLGroup::$group => &self.$group,)+}
            }
        }
    };
}

include!(concat!(env!("OUT_DIR"), "/highlight_groups.rs"));

/// Turns layered declarations into styles group by group, memoising each result and
/// tracking the link chain in progress to report cycles.
struct StyleResolver<'a> {
    declarations: &'a GroupDeclarations,
    styles: &'a StyleTable,
    overrides: &'a StyleTable,
    palette: &'a Palette,
    resolved: HashMap<String, Style>,
    visiting: Vec<String>,
}

impl StyleResolver<'_> {
    fn channel(&mut self, reference: &str) -> Result<Option<Color>, String> {
        let (name, channel) = reference
            .rsplit_once('.')
            .ok_or_else(|| format!("expected Group.fg, Group.bg or Group.sp: `{reference}`"))?;
        let style = self.group(name)?;
        match channel {
            "fg" => Ok(style.fg),
            "bg" => Ok(style.bg),
            "sp" => Ok(style.underline_color),
            _ => Err(format!("unknown style channel `{reference}`")),
        }
    }

    fn group(&mut self, name: &str) -> Result<Style, String> {
        if let Some(style) = self.resolved.get(name) {
            return Ok(*style);
        }
        HLGroup::named(name)?;
        if self.visiting.iter().any(|entry| entry == name) {
            return Err(format!(
                "cyclic highlight links: {} -> {name}",
                self.visiting.join(" -> ")
            ));
        }
        let declaration = self
            .overrides
            .get(name)
            .or_else(|| self.styles.get(name))
            .or_else(|| self.declarations.0.get(name))
            .cloned()
            .ok_or_else(|| format!("missing highlight declaration `{name}`"))?;
        self.visiting.push(name.into());
        let style = self
            .style(&declaration)
            .map_err(|error| format!("{name}: {error}"))?;
        self.visiting.pop();
        self.resolved.insert(name.into(), style);
        Ok(style)
    }

    fn style(&mut self, declaration: &StyleDeclaration) -> Result<Style, String> {
        let mut style = match &declaration.link {
            Some(link) => self.group(link)?,
            None => Style::new(),
        };
        for (direct, reference, channel) in [
            (&declaration.fg, &declaration.fg_from, "fg"),
            (&declaration.bg, &declaration.bg_from, "bg"),
            (&declaration.sp, &declaration.sp_from, "sp"),
        ] {
            if direct.is_some() && reference.is_some() {
                return Err(format!("{channel} and {channel}_from cannot both be set"));
            }
            let color = match reference {
                Some(reference) => Some(self.channel(reference)?),
                None => direct
                    .as_deref()
                    .map(|name| {
                        if name == "none" {
                            Ok(None)
                        } else {
                            self.palette.resolve(Some(name))
                        }
                    })
                    .transpose()?,
            };
            if let Some(color) = color {
                match channel {
                    "fg" => style.fg = color,
                    "bg" => style.bg = color,
                    _ => style.underline_color = color,
                }
            }
        }
        for (value, modifier) in [
            (declaration.bold, Modifier::BOLD),
            (declaration.italic, Modifier::ITALIC),
            (declaration.underline, Modifier::UNDERLINED),
            (declaration.reverse, Modifier::REVERSED),
            (declaration.crossed_out, Modifier::CROSSED_OUT),
            (declaration.dim, Modifier::DIM),
        ] {
            if let Some(value) = value {
                style = if value {
                    style.add_modifier(modifier)
                } else {
                    style.remove_modifier(modifier)
                };
            }
        }
        if declaration.saturation.is_some() || declaration.lightening.is_some() {
            let saturation = declaration.saturation.unwrap_or(100.0);
            let lightening = declaration.lightening.unwrap_or(0.0);
            for amount in [saturation, lightening] {
                if !amount.is_finite() || !(0.0..=100.0).contains(&amount) {
                    return Err("saturation and lightening must be between 0 and 100".into());
                }
            }
            style = softened(style, saturation, lightening);
        }
        if let Some(amount) = declaration.darken {
            if !amount.is_finite() || !(0.0..=1.0).contains(&amount) {
                return Err("darken must be between 0 and 1".into());
            }
            if let Some(fg) = style.fg {
                style.fg = Some(super::palette::blend(fg, Color::Rgb(0, 0, 0), amount)?);
            }
        }
        match (&declaration.bg_blend, declaration.bg_blend_amount) {
            (Some(color), Some(amount)) => {
                let bg = style.bg.ok_or("bg_blend needs a background")?;
                let color = self
                    .palette
                    .resolve(Some(color))?
                    .expect("a named palette color exists");
                style.bg = Some(super::palette::blend(bg, color, amount)?);
            }
            (None, None) => {}
            _ => return Err("bg_blend and bg_blend_amount must be set together".into()),
        }
        match (&declaration.tint_from, declaration.tint) {
            (Some(reference), Some(amount)) => {
                if !amount.is_finite() || !(0.0..=1.0).contains(&amount) {
                    return Err("tint must be between 0 and 1".into());
                }
                if let (Some(bg), Some(fg)) = (style.bg, self.channel(reference)?) {
                    style.bg = Some(super::palette::blend(bg, fg, amount)?);
                }
            }
            (None, None) => {}
            _ => return Err("tint_from and tint must be set together".into()),
        }
        Ok(style)
    }
}

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
    picker: Picker,
    picker_selected: PickerSelected,
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
    prompt: InputPrompt,
    command: Command,
    selection: Selection,
    muted: Muted,
    printed: Printed,
    completion_hint: CompletionHint,
    placeholder_hint: PlaceholderHint,
    picker_marker: PickerMarker,
    picker_selected_marker: PickerSelectedMarker,
    picker_title: PickerTitle,
    picker_border: PickerBorder,
    picker_tab: PickerTab,
    picker_tab_selected: PickerTabSelected,
    cell_number: CellNumber,
    pending_cell_marker: PendingCellMarker,
    queued_cell_marker: QueuedCellMarker,
    failed_cell_marker: FailedCellMarker,
    rejected_cell_marker: RejectedCellMarker,
    printed_cell_number: PrintedCellNumber,
    pending_separator: PendingSeparator,
    muted_code_text: MutedCodeText,
    settings_item_label: SettingsItemLabel,
    settings_item_value: SettingsItemValue,
    settings_item_unavailable: SettingsItemUnavailable,
    settings_item_disabled: SettingsItemDisabled,
    settings_unavailable_note: SettingsUnavailableNote,
    settings_item_description: SettingsItemDescription,
    settings_step_arrow: SettingsStepArrow,
    settings_button: SettingsButton,
    keymap_action_label: KeymapActionLabel,
    command_action_description: CommandActionDescription,
    command_shortcut: CommandShortcut,
    frame_stats: FrameStats,
    highlight_inspector_title: HighlightInspectorTitle,
    highlight_inspector_border: HighlightInspectorBorder,
    view_divider: ViewDivider,
    syntax_tree_branch: SyntaxTreeBranch,
    syntax_tree_token_marker: SyntaxTreeTokenMarker,
    syntax_tree_label_separator: SyntaxTreeLabelSeparator,
    syntax_tree_border: SyntaxTreeBorder,
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
    use crate::ui::controller::settings::theme::SCHEMES;

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

    fn fixture() -> (StyleTable, Palette) {
        let file: super::super::SchemeFile =
            toml::from_str(super::super::SCHEME_FILES[0].1).unwrap();
        let palette = file
            .palette
            .palette(super::super::Mode::Dark, "mocha")
            .unwrap();
        (file.groups, palette)
    }

    #[test]
    fn overrides_propagate_through_links_and_channel_references() {
        let (styles, palette) = fixture();
        let overrides = toml::from_str(
            r#"
            Accent = { fg = "red", italic = true }
            Error = { fg = "blue" }
            Canvas = { bg = "mantle" }
        "#,
        )
        .unwrap();
        let groups = Groups::resolve(
            &styles,
            &overrides,
            &palette,
            &GroupDeclarations::embedded(),
        )
        .unwrap();
        assert_eq!(groups.OutputNumber, groups.Accent);
        assert_eq!(groups.InputPrompt, groups.Accent);
        assert!(groups
            .RecentOutput
            .add_modifier
            .contains(Modifier::ITALIC | Modifier::BOLD));
        assert_eq!(groups.DiagnosticError.underline_color, groups.Error.fg);
        assert_eq!(groups.Separator.fg, groups.Canvas.bg);
    }

    #[test]
    fn explicit_styles_replace_links_and_can_remove_inherited_modifiers() {
        let (styles, palette) = fixture();
        let overrides = toml::from_str(
            r#"
            Picker = { bold = true }
            RecentOutput = { link = "Accent", bold = false }
        "#,
        )
        .unwrap();
        let groups = Groups::resolve(
            &styles,
            &overrides,
            &palette,
            &GroupDeclarations::embedded(),
        )
        .unwrap();
        assert_eq!(groups.Picker, Style::new().bold());
        assert!(!groups.RecentOutput.add_modifier.contains(Modifier::BOLD));
        assert!(groups.RecentOutput.sub_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn older_user_defaults_inherit_new_groups_and_can_relink_existing_ones() {
        let (styles, palette) = fixture();
        let declarations =
            GroupDeclarations::parse("CompletionHint = { link = 'Info', italic = true }").unwrap();
        let groups = Groups::resolve(&styles, &HashMap::new(), &palette, &declarations).unwrap();
        assert_eq!(groups.CompletionHint, groups.Info.italic());
        assert_eq!(groups.PlaceholderHint, groups.Muted);
        assert!(
            GroupDeclarations::parse("CompletinoHint = { link = 'Info' }")
                .err()
                .unwrap()
                .contains("CompletinoHint")
        );
        assert!(GroupDeclarations::parse("Text = { link = 'Info' }").is_err());
    }

    #[test]
    fn captures_are_declared_once_and_inherited_by_older_user_defaults() {
        let owner = |declarations: &GroupDeclarations, capture: &str| {
            declarations
                .captures()
                .unwrap()
                .into_iter()
                .find(|(name, _)| name == capture)
                .map(|(_, group)| group)
        };
        let older = GroupDeclarations::parse("Conditional = { link = 'Statement' }").unwrap();
        assert_eq!(
            owner(&older, "keyword.conditional"),
            owner(&GroupDeclarations::embedded(), "keyword.conditional")
        );
        let moved =
            GroupDeclarations::parse("Conditional = { link = 'Keyword', captures = [] }\nRepeat = { link = 'Keyword', captures = ['keyword.repeat', 'keyword.conditional'] }")
                .unwrap();
        assert_eq!(owner(&moved, "keyword.conditional"), Some(HLGroup::Repeat));
        assert!(GroupDeclarations::parse(
            "Repeat = { link = 'Keyword', captures = ['keyword.conditional'] }"
        )
        .err()
        .unwrap()
        .contains("keyword.conditional"));

        let (styles, palette) = fixture();
        let overrides = toml::from_str("Comment = { fg = 'red', captures = ['comment'] }").unwrap();
        assert!(Groups::resolve(
            &styles,
            &overrides,
            &palette,
            &GroupDeclarations::embedded()
        )
        .is_err());
    }

    #[test]
    fn cycles_missing_required_styles_and_bad_references_are_rejected() {
        let (mut styles, palette) = fixture();
        let overrides: StyleTable = toml::from_str(
            r#"
            CompletionHint = { link = "PlaceholderHint" }
            PlaceholderHint = { fg_from = "CompletionHint.fg" }
        "#,
        )
        .unwrap();
        let error = Groups::resolve(
            &styles,
            &overrides,
            &palette,
            &GroupDeclarations::embedded(),
        )
        .err()
        .unwrap();
        assert!(error.contains("cyclic"), "{error}");
        for source in [
            "CompletionHint = { link = 'NoSuchGroup' }",
            "CompletionHint = { fg_from = 'Text.typo' }",
            "CompletionHint = { fg = 'invented_color' }",
            "CompletionHint = { bg = 'base', bg_blend = 'red', bg_blend_amount = 1.5 }",
        ] {
            let overrides = toml::from_str(source).unwrap();
            assert!(
                Groups::resolve(
                    &styles,
                    &overrides,
                    &palette,
                    &GroupDeclarations::embedded()
                )
                .is_err(),
                "{source}"
            );
        }
        styles.remove("Text");
        assert!(Groups::resolve(
            &styles,
            &HashMap::new(),
            &palette,
            &GroupDeclarations::embedded()
        )
        .err()
        .unwrap()
        .contains("missing required highlight group `Text`"));
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
        // Groups declared without a style lay nothing over the text beneath them.
        let transparent =
            [HLGroup::LspVariable, HLGroup::Unresolved].map(|group| group.description());
        for groups in SCHEMES
            .iter()
            .flat_map(|scheme| scheme.variants())
            .map(|variant| variant.groups())
        {
            assert!(groups.highlight_groups().iter().all(|&(group, style)| {
                transparent.contains(&group)
                    || style.fg.is_some()
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
    fn input_hints_have_independent_named_groups() {
        let groups = Groups::default();
        assert_eq!(groups.completion_hint(), groups.Muted);
        assert_eq!(groups.placeholder_hint(), groups.Muted);
        assert_eq!(groups.prompt(), groups.Accent);
        assert!(groups
            .highlight_groups()
            .iter()
            .any(|(name, _)| *name == "CompletionHint"));
        assert!(groups
            .highlight_groups()
            .iter()
            .any(|(name, _)| *name == "PlaceholderHint"));
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
    fn documentation_links_are_underlined_only_while_hovered() {
        for groups in SCHEMES
            .iter()
            .flat_map(|scheme| scheme.variants())
            .map(|variant| variant.groups())
        {
            assert!(!groups[HLGroup::MarkupLink]
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
                3,
                "surfaces: {:?}",
                [
                    groups.transcript().bg,
                    groups.code().bg,
                    groups.printed().bg
                ]
            );
        }
    }

    #[test]
    fn pickers_have_their_own_surface_and_selection_groups() {
        for groups in SCHEMES
            .iter()
            .flat_map(|scheme| scheme.variants())
            .map(|variant| variant.groups())
        {
            assert!(groups.picker().bg.is_some());
            assert!(groups.picker_selected().bg.is_some());
        }
    }
}
