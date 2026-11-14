//! The *LSP* tab: whether the language server runs, and what it adds.

use serde::{Deserialize, Serialize};

use super::rendering::switch;
use super::row::{SettingRow, SettingsTab};
use super::Settings;
use crate::code::semantic::SERVER_EXECUTABLE;
use crate::ui::controller::picker::{PickerItem, PickerValue};
use crate::ui::cycle::cyclic;

/// The values of the *LSP* tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LspSettings {
    /// Whether an installed language server analyses the session's source.
    pub language_server: bool,
    /// Show automatic signature-help popups while typing.
    pub signature_hints: bool,
    /// Whether the language server enriches source colors, which it can only do
    /// while it runs.
    pub semantic_highlighting: bool,
    /// Show automatic language-server diagnostics and their underlines.
    pub diagnostics: bool,
}

impl Default for LspSettings {
    fn default() -> Self {
        Self {
            language_server: true,
            signature_hints: false,
            semantic_highlighting: true,
            diagnostics: false,
        }
    }
}

impl LspSettings {
    /// Whether source takes semantic colors: chosen, and the server providing them runs.
    pub const fn semantic_colors(&self) -> bool {
        self.language_server && self.semantic_highlighting
    }
}

impl SettingsTab for LspSettings {
    const LABEL: &'static str = "LSP";

    fn rows(settings: Settings) -> Vec<Box<dyn PickerItem>> {
        LspSetting::rows(LspSetting::CHOICES, settings).collect()
    }
}

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
            Self::LanguageServer => "choose language server for editor support",
            Self::SemanticHighlighting => "Enrich the base highlighter with LSP semantic tokens",
        }
    }

    fn value(self, settings: Settings) -> PickerValue {
        match self {
            Self::SignatureHints => switch(settings.lsp.signature_hints, "on"),
            Self::Diagnostics => switch(settings.lsp.diagnostics, "on"),
            Self::LanguageServer => switch(settings.lsp.language_server, SERVER_EXECUTABLE),
            Self::SemanticHighlighting => switch(settings.lsp.semantic_colors(), "on"),
        }
    }

    fn adjust(self, settings: &mut Settings, _: i8) {
        match self {
            Self::SignatureHints => settings.lsp.signature_hints = !settings.lsp.signature_hints,
            Self::Diagnostics => settings.lsp.diagnostics = !settings.lsp.diagnostics,
            Self::LanguageServer => settings.lsp.language_server = !settings.lsp.language_server,
            Self::SemanticHighlighting => {
                settings.lsp.semantic_highlighting = !settings.lsp.semantic_highlighting
            }
        }
    }

    fn applies(self, settings: Settings) -> bool {
        match self {
            Self::SignatureHints | Self::Diagnostics | Self::SemanticHighlighting => {
                settings.lsp.language_server
            }
            Self::LanguageServer => true,
        }
    }
}
