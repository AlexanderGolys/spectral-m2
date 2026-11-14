//! Independent formatter results retained for an output's preview controls.

use serde::{Deserialize, Serialize};

use super::SpecNet;
use crate::ui::cycle::cyclic;

cyclic! {
    /// The M2 representation selected for one output, independent of display settings.
    #[discriminated_str(name)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[non_exhaustive]
    pub enum PreviewMode {
        /// Structured expression data from the Spectral formatter.
        #[serde(rename = "specNet")]
        SpecNet = "specNet",
        /// M2's two-dimensional terminal spelling.
        #[serde(rename = "net")]
        Net = "net",
        /// M2's string conversion.
        #[serde(rename = "string")]
        String = "string",
        /// M2's TeX formatter.
        #[serde(rename = "tex")]
        Tex = "tex",
        /// A semantic documentation page.
        #[serde(rename = "markup")]
        Markup = "markup",
        /// M2's re-readable spelling, `toExternalString`.
        #[serde(rename = "external")]
        External = "external",
        /// The value's structure as `speek` draws it: each part's class and
        /// what it holds, coloured and set as `specNet` sets a value.
        #[serde(rename = "speek")]
        Speek = "speek",
        /// One level of the value's inner structure, as M2's `peek` shows it.
        #[serde(rename = "peek")]
        Peek = "peek",
    }
}

/// One formatter's output, captured without re-evaluating the original input.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ValuePreview {
    /// The formatter that produced this representation.
    pub mode: PreviewMode,
    /// The formatter's complete text, before terminal wrapping or typesetting.
    pub text: String,
    /// The drawing of a formatter that answers with a value of its own to show,
    /// as `speek` does, which the text only spells.
    #[serde(default)]
    pub value: Option<SpecNet>,
}

impl ValuePreview {
    pub fn new(mode: PreviewMode, text: impl Into<String>) -> Self {
        Self {
            mode,
            text: text.into(),
            value: None,
        }
    }
}
