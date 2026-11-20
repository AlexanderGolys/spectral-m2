//! Independent formatter results retained for an output's preview controls.

use serde::{Deserialize, Serialize};

use crate::ui::cycle::cyclic;

cyclic! {
    /// The M2 representation selected for one output, independent of display settings.
    #[discriminated_str(name)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[non_exhaustive]
    pub enum PreviewMode {
        /// Structured expression data from the Spectral formatter.
        #[serde(rename = "rod", alias = "specNet")]
        Rod = "rod",
        /// M2's two-dimensional terminal spelling.
        #[serde(rename = "net")]
        Net = "net",
        /// M2's string conversion.
        #[serde(rename = "string")]
        String = "string",
        /// A semantic documentation page.
        #[serde(rename = "markup")]
        Markup = "markup",
        /// M2's re-readable spelling, `toExternalString`.
        #[serde(rename = "external")]
        External = "external",
        /// The value inspected as `peek` inspects it, drawn as Spectral's
        /// `speek` draws it.
        #[serde(rename = "speek")]
        Speek = "speek",
    }
}

/// One formatter's output, captured without re-evaluating the original input.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ValuePreview {
    /// The formatter that produced this representation.
    pub mode: PreviewMode,
    /// The formatter's complete text, before terminal wrapping or typesetting.
    pub text: String,
}

impl ValuePreview {
    pub fn new(mode: PreviewMode, text: impl Into<String>) -> Self {
        Self {
            mode,
            text: text.into(),
        }
    }
}
