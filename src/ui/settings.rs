//! User-adjustable settings, one section for each tab of the settings picker.
//!
//! Values the user is expected to tune live here rather than as constants, so the
//! settings picker has one place to read and write and the defaults stay visible:
//! a tab's module holds its values, their defaults and its rows.
//! Whatever the pickers change is kept between sessions, each record in a file of
//! its own under the user's configuration directory.

pub mod appearance;
pub mod config;
pub mod editor;
pub mod keymap;
pub mod lsp;
pub mod rendering;
pub mod row;
pub mod selection;
pub mod statusline;
pub mod tabs;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

pub use self::config::*;
use self::editor::EditorSettings;
pub use self::keymap::*;
use self::rendering::RenderingSettings;
use self::selection::SelectionSettings;
use self::statusline::StatusLineSettings;
use crate::ui::key::KeyEvent;
use crate::ui::theme::AppearanceSettings;

/// Everything the settings picker adjusts, a section for each of its tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Settings {
    pub appearance: AppearanceSettings,
    pub editor: EditorSettings,
    pub statusline: StatusLineSettings,
    pub rendering: RenderingSettings,
    pub selection: SelectionSettings,
}

impl Settings {
    /// Whether returned values lay out differently, moving their heights.
    pub fn reformats(&self, other: &Self) -> bool {
        self.editor.output_rows != other.editor.output_rows
            || self.rendering.reformats(&other.rendering)
    }
}

/// What the settings and keymap pickers adjust, apart from the appearance, which
/// keeps a file of its own.
#[derive(Default, Serialize, Deserialize)]
#[serde(from = "StoredSettings")]
pub struct SavedSettings {
    pub layout: EditorSettings,
    pub statusline: StatusLineSettings,
    pub rendering: RenderingSettings,
    pub selection: SelectionSettings,
    /// Every rebind made, in order. Replaying them over the default keymaps restores
    /// the swaps they caused as well as the keys they set.
    pub rebinds: Vec<(Shortcut, KeyEvent)>,
}

impl Persisted for SavedSettings {
    const FILE: &'static str = "settings.json";
}

/// A settings file as read, of this version or of those keeping the values under
/// `editor` and `input`, whose fields the sections still answer to.
#[derive(Default, Deserialize)]
#[serde(default)]
struct StoredSettings {
    layout: EditorSettings,
    statusline: StatusLineSettings,
    rendering: RenderingSettings,
    selection: SelectionSettings,
    #[serde(deserialize_with = "known_rebinds")]
    rebinds: Vec<(Shortcut, KeyEvent)>,
    editor: Map<String, Value>,
    input: Map<String, Value>,
}

impl From<StoredSettings> for SavedSettings {
    fn from(stored: StoredSettings) -> Self {
        let rebinds = stored.rebinds;
        if stored.editor.is_empty() && stored.input.is_empty() {
            return Self {
                layout: stored.layout,
                statusline: stored.statusline,
                rendering: stored.rendering,
                selection: stored.selection,
                rebinds,
            };
        }
        let older: Map<_, _> = stored.editor.into_iter().chain(stored.input).collect();
        Self {
            layout: section_of(&older),
            statusline: section_of(&older),
            rendering: section_of(&older),
            selection: stored.selection,
            rebinds,
        }
    }
}

/// The saved rebinds still naming a shortcut and key this version has; one for an
/// action since removed is dropped rather than losing every other setting.
fn known_rebinds<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<(Shortcut, KeyEvent)>, D::Error> {
    Ok(Vec::<Value>::deserialize(deserializer)?
        .into_iter()
        .filter_map(|rebind| serde_json::from_value(rebind).ok())
        .collect())
}

/// The section reading its own fields out of an older file's values.
fn section_of<Section: DeserializeOwned + Default>(older: &Map<String, Value>) -> Section {
    serde_json::from_value(Value::Object(older.clone())).unwrap_or_default()
}
