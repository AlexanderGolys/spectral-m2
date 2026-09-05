use std::ops::Index;

use ratatui::style::{Modifier, Style};
use serde::Deserialize;
use these_macros_should_be_illegal::enum_fn;

use super::palette::Palette;

macro_rules! style_spec {
    ($($flag:ident: $modifier:ident),+ $(,)?) => {
        /// A serialized style whose colors name entries in its scheme palette.
        #[derive(Default, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        pub(crate) struct StyleSpec {
            fg: Option<String>,
            bg: Option<String>,
            sp: Option<String>,
            $($flag: bool),+
        }

        impl StyleSpec {
            fn resolve(&self, palette: &Palette) -> Result<Style, String> {
                let mut modifiers = Modifier::empty();
                $(modifiers.set(Modifier::$modifier, self.$flag);)+
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
    };
}

style_spec! {
    bold: BOLD,
    dim: DIM,
    italic: ITALIC,
    underline: UNDERLINED,
    slow_blink: SLOW_BLINK,
    rapid_blink: RAPID_BLINK,
    reverse: REVERSED,
    hidden: HIDDEN,
    crossed_out: CROSSED_OUT,
}

macro_rules! highlight_groups {
    ($($name:ident),+ $(,)?) => {
        /// The complete scheme-independent space of syntax and interface styles.
        #[enum_fn(description: &'static str = stringify)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(crate) enum HLGroup {
            $($name),+
        }

        /// Every highlight-group value represented by the same finite field set.
        #[allow(non_snake_case)]
        #[derive(Clone, Copy, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(crate) struct Groups<S = Style> {$(
            #[doc = concat!("Style for `", stringify!($name), "` content.")]
            pub(crate) $name: S,
        )+}

        impl Groups<StyleSpec> {
            pub(crate) fn resolve(&self, palette: &Palette) -> Result<Groups, String> {
                Ok(Groups {$($name: self.$name.resolve(palette)
                    .map_err(|error| format!("{}: {error}", stringify!($name)))?),+})
            }
        }

        impl Groups {
            pub(crate) fn highlight_groups(&self) -> Vec<(&'static str, Style)> {
                vec![$((HLGroup::$name.description(), self.$name)),+]
            }
        }

        impl Index<HLGroup> for Groups {
            type Output = Style;

            fn index(&self, group: HLGroup) -> &Self::Output {
                match group {$(HLGroup::$name => &self.$name),+}
            }
        }
    };
}

highlight_groups! {
    Comment,
    Constant,
    String,
    Character,
    Number,
    Boolean,
    Float,
    Identifier,
    Function,
    Statement,
    Conditional,
    Repeat,
    Label,
    Operator,
    Keyword,
    Exception,
    PreProc,
    Include,
    Define,
    Macro,
    PreCondit,
    Type,
    StorageClass,
    Structure,
    Typedef,
    Special,
    SpecialChar,
    Tag,
    Delimiter,
    SpecialComment,
    Debug,
    Underlined,
    Ignore,
    Error,
    Todo,
    Added,
    Changed,
    Removed,
    Warning,
    Info,
    Hint,
    DiagnosticError,
    DiagnosticWarning,
    DiagnosticInfo,
    DiagnosticHint,
    Installation,
    Property,
    Namespace,
    Package,
    CompiledFunction,
    Symbol,
    Variable,
    AlgVariable,
    Parameter,
    Regex,
    Punctuation,
    Quote,
    Documentation,
    LspTypeClass,
    LspTypeComment,
    LspTypeDecorator,
    LspTypeEnum,
    LspTypeEnumMember,
    LspTypeEvent,
    LspTypeFunction,
    LspTypeInterface,
    LspTypeKeyword,
    LspTypeMacro,
    LspTypeMethod,
    LspTypeModifier,
    LspTypeNamespace,
    LspTypeNumber,
    LspTypeOperator,
    LspTypeParameter,
    LspTypeProperty,
    LspTypeRegexp,
    LspTypeString,
    LspTypeStruct,
    LspTypeType,
    LspTypeTypeParameter,
    LspTypeVariable,
    LspModAbstract,
    LspModAsync,
    LspModDeclaration,
    LspModDefaultLibrary,
    LspModDefinition,
    LspModDeprecated,
    LspModDocumentation,
    LspModModification,
    LspModReadonly,
    LspModStatic,
    Visual,
    Value,
    Muted,
    Printed,
    ShellCommand,
    ShellMessage,
    ShellError,
    HintText,
    Normal,
    CodeBlock,
    PendingBlock,
    Separator,
    ValueBorder,
    OutputNumber,
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
    Picker,
    PickerSelected,
    PickerBorder,
    Accent1,
    Accent2,
    Accent3,
    Accent4,
    Accent5,
    Accent6,
    Accent7,
    BgColor1,
    BgColor2,
    BgColor3,
    BgColor4,
}

impl Groups {
    pub(crate) fn style(&self, group: HLGroup) -> Style {
        self[group]
    }

    pub(crate) fn diagnostic(&self) -> Style {
        self.DiagnosticError
    }
}

macro_rules! accessors {
    ($($name:ident: $group:ident),+ $(,)?) => {
        impl Groups {$(
            pub(crate) fn $name(&self) -> Style {
                self.$group
            }
        )+}
    };
}

accessors! {
    transcript: Normal,
    code: CodeBlock,
    pending: PendingBlock,
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
    prompt: KeymapHint,
    selection: Visual,
    muted: Muted,
    printed: Printed,
    hint_text: HintText,
    error_border: ErrorBorder,
    warning_border: WarningBorder,
    info_border: InfoBorder,
    statusline_bg: StatuslineBg,
}

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
    fn generic_palette_slots_have_component_neutral_names() {
        let names = Groups::default().highlight_groups();
        assert_eq!(
            names
                .iter()
                .filter(|(name, _)| name.starts_with("Accent"))
                .count(),
            7
        );
        assert_eq!(
            names
                .iter()
                .filter(|(name, _)| name.starts_with("BgColor"))
                .count(),
            4
        );
        for groups in SCHEMES
            .iter()
            .flat_map(|scheme| scheme.variants())
            .map(|variant| variant.groups())
        {
            for (name, style) in groups.highlight_groups() {
                if name.starts_with("Accent") {
                    assert!(style.fg.is_some() && style.bg.is_none());
                } else if name.starts_with("BgColor") {
                    assert!(style.bg.is_some());
                }
            }
        }
    }

    #[test]
    fn group_names_describe_semantics_instead_of_palette_hues() {
        let groups = Groups::default().highlight_groups();
        assert_eq!(
            groups
                .iter()
                .filter(|(name, _)| name.starts_with("Lsp"))
                .count(),
            33
        );
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
