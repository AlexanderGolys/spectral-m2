//! Highlight groups: their TOML declarations and the resolved styles drawn from them.
//!
//! Styles are written as [`StyleDeclaration`]s in two places: the shared
//! `groups.toml` ([`GroupDeclarations`]), which lists every group with its default
//! link and Tree-sitter captures, and each scheme file, which colours the required
//! groups from its palette and may override any other. [`Groups::resolve`] layers
//! them (variant override, then scheme, then shared default), follows links and
//! channel references, applies colour adjustments, and yields one ratatui [`Style`]
//! per [`HLGroup`]. Rendering only ever sees the resolved [`Groups`].

use std::collections::{HashMap, HashSet};
use std::ops::Index;
use std::sync::LazyLock;
#[cfg(not(test))]
use std::sync::RwLock;

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use these_macros_should_be_illegal::{enum_fn, forall};

use super::*;
use palette::{self, Palette};

/// One highlight group's style as written in TOML, before it is resolved into a
/// [`Style`]. Every key is optional, and a declaration replaces a lower layer's one for
/// the same group as a whole rather than merging with it.
///
/// Keys apply in field order: start from `link`, set the channels, toggle
/// modifiers, then adjust colours. Colours are native palette names; `Group.fg`,
/// `Group.bg` and `Group.sp` name another group's resolved channel.
#[forall(Option<>)]
#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StyleDeclaration {
    /// Only in `groups.toml`: every scheme must declare this group itself.
    #[tmsbi(skip)]
    required: bool,
    /// Only in `groups.toml`: the Tree-sitter captures this group draws.
    captures: Vec<String>,
    /// Group whose resolved style this one starts from.
    link: String,
    /// Palette colour for each channel (`sp` is the underline colour); `"none"`
    /// clears what a link supplied.
    fg: String,
    bg: String,
    sp: String,
    /// Channel copied from another group, so one group's colour can serve in a
    /// different channel here (an error foreground as an underline). Exclusive with
    /// the direct channel of the same name.
    fg_from: String,
    bg_from: String,
    sp_from: String,
    /// Modifiers; `false` removes one a link supplied.
    bold: bool,
    italic: bool,
    underline: bool,
    reverse: bool,
    crossed_out: bool,
    /// Drawn by the terminal blended toward its background.
    dim: bool,
    /// Foreground kept at this percentage of its saturation, hue unchanged.
    saturation: f32,
    /// Foreground moved this percentage of the way to the scheme's text colour.
    lightening: f32,
    /// Foreground blended toward the background by this fraction: the group's own
    /// when it has one, else the canvas.
    darken: f32,
    /// Background blended toward a palette colour by `bg_blend_amount`, a fraction,
    /// for schemes whose guidelines specify an overlay's opacity.
    bg_blend: String,
    bg_blend_amount: f32,
    /// Background blended toward another group's channel by `tint`, a fraction.
    tint_from: String,
    tint: f32,
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

    /// Merge a user's declarations over the built-in defaults. A capture the user's
    /// file assigns leaves whichever default group held it, so a copy written before
    /// a capture moved between defaults keeps loading.
    pub fn parse(source: &str) -> Result<Self, String> {
        let custom: StyleTable = toml::from_str(source).map_err(|e| e.to_string())?;
        let assigned: HashSet<String> = custom
            .values()
            .flat_map(|declaration| declaration.captures.iter().flatten().cloned())
            .collect();
        let mut declarations = Self::embedded();
        for captures in declarations.0.values_mut().flat_map(|d| &mut d.captures) {
            captures.retain(|capture| !assigned.contains(capture));
        }
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

    /// Whether Tree-sitter captures are drawn with this group.
    pub fn draws_captures(&self, group: HLGroup) -> bool {
        self.0
            .get(group.description())
            .is_some_and(|declaration| declaration.captures.iter().flatten().next().is_some())
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
        match config_source("groups.toml", DEFAULTS).and_then(|s| Self::parse(&s)) {
            Ok(declarations) => {
                // Validate against every built-in palette before publishing defaults.
                // A bad user link must not also break the fallback schemes.
                for (name, source) in SCHEME_FILES {
                    let file: SchemeFile =
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
        #[enum_fn(pub const description -> &'static str)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum HLGroup {$(
            #[doc = concat!("The `", stringify!($group), "` highlight group.")]
            $group = stringify!($group),
        )+}

        impl HLGroup {
            pub fn named(name: &str) -> Result<Self, String> {
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

        impl Groups {
            /// Every group with its resolved style, in declaration order.
            pub fn styled_groups(&self) -> Vec<(HLGroup, Style)> {
                vec![$((HLGroup::$group, self.$group),)+]
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
        if let Some(saturation) = declaration.saturation {
            if !saturation.is_finite() || !(0.0..=100.0).contains(&saturation) {
                return Err("saturation must be between 0 and 100".into());
            }
            style = desaturated(style, saturation);
        }
        if let Some(lightening) = declaration.lightening {
            if !lightening.is_finite() || !(0.0..=100.0).contains(&lightening) {
                return Err("lightening must be between 0 and 100".into());
            }
            if let (Some(fg), Some(text)) = (style.fg, self.channel("Text.fg")?) {
                style.fg = Some(palette::blend(fg, text, lightening / 100.0)?);
            }
        }
        if let Some(amount) = declaration.darken {
            if !amount.is_finite() || !(0.0..=1.0).contains(&amount) {
                return Err("darken must be between 0 and 1".into());
            }
            let canvas = self.channel("Canvas.bg")?;
            if let (Some(fg), Some(bg)) = (style.fg, style.bg.or(canvas)) {
                style.fg = Some(blend(fg, bg, amount)?);
            }
        }
        match (&declaration.bg_blend, declaration.bg_blend_amount) {
            (Some(color), Some(amount)) => {
                let bg = style.bg.ok_or("bg_blend needs a background")?;
                let color = self
                    .palette
                    .resolve(Some(color))?
                    .expect("a named palette color exists");
                style.bg = Some(blend(bg, color, amount)?);
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
                    style.bg = Some(palette::blend(bg, fg, amount)?);
                }
            }
            (None, None) => {}
            _ => return Err("tint_from and tint must be set together".into()),
        }
        Ok(style)
    }
}

/// `style` with its foreground desaturated to `saturation` percent of what it was, its
/// hue untouched. A colour that is not RGB has no channels to move and is left as it is.
fn desaturated(style: Style, saturation: f32) -> Style {
    let Some(Color::Rgb(r, g, b)) = style.fg else {
        return style;
    };
    let [r, g, b] = [r, g, b].map(|channel| f32::from(channel) / 255.0);
    let (high, low) = (r.max(g).max(b), r.min(g).min(b));
    let chroma = high - low;
    let kept = if high == 0.0 { 0.0 } else { chroma / high } * saturation / 100.0;
    let floor = high * (1.0 - kept);
    // The same position between the darkest and brightest channel as before, which is
    // what keeps the hue where it was.
    let channel = |from: f32| {
        let along = if chroma == 0.0 {
            0.0
        } else {
            (from - low) / chroma
        };
        let byte = (floor + (high - floor) * along) * 255.0;
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
    picker_selected_label: PickerSelectedLabel,
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
    settings_item_value_selected: SettingsItemValueSelected,
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
    floating_border: FloatingBorder,
    fold_marker: FoldMarker,
    name_occurrence: NameOccurrence,
    signature_active_parameter: SignatureActiveParameter,
    view_divider: ViewDivider,
    syntax_tree_branch: SyntaxTreeBranch,
    syntax_tree_token_marker: SyntaxTreeTokenMarker,
    syntax_tree_label_separator: SyntaxTreeLabelSeparator,
    syntax_tree_border: SyntaxTreeBorder,
    error_border: ErrorBorder,
    error_message: ErrorMessage,
    error_title: ErrorTitle,
    error_code: ErrorCode,
    warning_border: WarningBorder,
    info_border: InfoBorder,
    statusline_bg: StatuslineBg,
}

/// The default appearance's resolved scheme, used before any theme is published.
impl Default for Groups {
    fn default() -> Self {
        Appearance::default().groups()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (StyleTable, Palette) {
        let file: SchemeFile = toml::from_str(SCHEME_FILES[0].1).unwrap();
        let palette = file.palette.palette(Mode::Dark, "mocha").unwrap();
        (file.groups, palette)
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
    fn a_capture_assigned_by_the_user_leaves_its_default_group() {
        let (capture, _) = GroupDeclarations::embedded().captures().unwrap().remove(0);
        let moved =
            GroupDeclarations::parse(&format!("Separator = {{ captures = ['{capture}'] }}"))
                .unwrap();
        assert!(moved
            .captures()
            .unwrap()
            .contains(&(capture.clone(), HLGroup::Separator)));
        assert!(GroupDeclarations::parse(&format!(
            "Separator = {{ captures = ['{capture}'] }}\nPending = {{ captures = ['{capture}'] }}"
        ))
        .err()
        .unwrap()
        .contains(&capture));
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

    fn resolved(source: &str) -> Result<Groups, String> {
        let (styles, palette) = fixture();
        let overrides = toml::from_str(source).unwrap();
        Groups::resolve(
            &styles,
            &overrides,
            &palette,
            &GroupDeclarations::embedded(),
        )
    }

    #[test]
    fn darkening_blends_the_foreground_toward_its_own_background_else_the_canvas() {
        let canvas = resolved("").unwrap();
        let (accent, canvas_bg) = (canvas.Accent.fg.unwrap(), canvas.Canvas.bg.unwrap());

        let plain = resolved("CompletionHint = { fg_from = 'Accent.fg', darken = 0.5 }").unwrap();
        let own = resolved("CompletionHint = { fg_from = 'Accent.fg', bg = 'red', darken = 0.5 }")
            .unwrap();

        assert_eq!(
            plain.CompletionHint.fg,
            Some(palette::blend(accent, canvas_bg, 0.5).unwrap())
        );
        assert_eq!(
            own.CompletionHint.fg,
            Some(palette::blend(accent, own.CompletionHint.bg.unwrap(), 0.5).unwrap())
        );
    }

    #[test]
    fn lightening_blends_the_foreground_toward_the_text_colour_by_a_percentage() {
        let base = resolved("").unwrap();
        let (accent, text) = (base.Accent.fg.unwrap(), base.Text.fg.unwrap());

        let lightened =
            resolved("CompletionHint = { fg_from = 'Accent.fg', lightening = 40.0 }").unwrap();

        assert_eq!(
            lightened.CompletionHint.fg,
            Some(palette::blend(accent, text, 0.4).unwrap())
        );
    }

    #[test]
    fn zero_saturation_leaves_a_grey_and_the_amounts_are_checked() {
        let grey = resolved("CompletionHint = { fg_from = 'Accent.fg', saturation = 0.0 }")
            .unwrap()
            .CompletionHint
            .fg;
        let Some(Color::Rgb(r, g, b)) = grey else {
            panic!("an RGB foreground stays RGB");
        };
        assert!(r == g && g == b, "{grey:?}");
        for source in [
            "CompletionHint = { fg_from = 'Accent.fg', darken = 1.5 }",
            "CompletionHint = { fg_from = 'Accent.fg', lightening = 120.0 }",
            "CompletionHint = { fg_from = 'Accent.fg', saturation = -1.0 }",
        ] {
            assert!(resolved(source).is_err(), "{source}");
        }
    }

    #[test]
    fn groups_have_one_uniform_indexed_representation() {
        let groups = Groups::default();
        assert_eq!(groups.style(HLGroup::Number), groups[HLGroup::Number]);
        assert_eq!(groups.style(HLGroup::Operator), groups[HLGroup::Operator]);
    }
}
