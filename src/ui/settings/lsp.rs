//! Language-server controls in the settings picker.

use these_macros_should_be_illegal::discriminated_str;

use super::rendering::switch;
use super::row::SettingRow;
use super::Settings;
use crate::code::semantic::SERVER_EXECUTABLE;
use crate::ui::controller::picker::PickerValue;
use crate::ui::cycle::cyclic;

cyclic! {
    /// One row of the LSP tab.
    #[non_exhaustive]
    #[discriminated_str(label)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum LspSetting {
        LanguageServer = "language server",
        Diagnostics = "diagnostics",
        SignatureHints = "signature hints",
        SemanticHighlighting = "semantic highlighting",
    }
}

impl SettingRow for LspSetting {
    fn label(self) -> &'static str {
        Self::label(&self)
    }

    fn description(self, _: Settings) -> &'static str {
        match self {
            Self::SignatureHints => "show the active call signature while typing",
            Self::Diagnostics => "show automatic diagnostics and source underlines",
            Self::LanguageServer => "analyse the session's source with m2-ls when it is installed",
            Self::SemanticHighlighting => "enrich source colors using the language server",
        }
    }

    fn value(self, settings: Settings) -> PickerValue {
        match self {
            Self::SignatureHints => switch(settings.rendering.signature_hints, "on"),
            Self::Diagnostics => switch(settings.editor.diagnostics, "on"),
            Self::LanguageServer => switch(settings.rendering.language_server, SERVER_EXECUTABLE),
            Self::SemanticHighlighting => switch(settings.rendering.semantic_colors(), "on"),
        }
    }

    fn adjust(self, settings: &mut Settings, _: i8) {
        match self {
            Self::SignatureHints => {
                settings.rendering.signature_hints = !settings.rendering.signature_hints
            }
            Self::Diagnostics => settings.editor.diagnostics = !settings.editor.diagnostics,
            Self::LanguageServer => {
                settings.rendering.language_server = !settings.rendering.language_server
            }
            Self::SemanticHighlighting => {
                settings.rendering.semantic_highlighting = !settings.rendering.semantic_highlighting
            }
        }
    }

    fn applies(self, settings: Settings) -> bool {
        match self {
            Self::SignatureHints | Self::Diagnostics | Self::SemanticHighlighting => {
                settings.rendering.language_server
            }
            Self::LanguageServer => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_highlighting_reads_off_and_locks_without_the_language_server() {
        let mut settings = Settings::default();
        settings.rendering.semantic_highlighting = true;
        settings.rendering.language_server = false;
        let row = LspSetting::SemanticHighlighting.row(settings);
        assert!(!row.selectable());
        assert!(matches!(
            row.value(),
            Some(PickerValue::Choice { options, chosen }) if options[*chosen].label == "off"
        ));
        settings.rendering.language_server = true;
        assert!(LspSetting::SemanticHighlighting.row(settings).selectable());
    }
}
