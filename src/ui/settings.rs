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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crossterm::event::KeyCode::Char;

    use crate::env::ENV;

    use super::keymap::Keymaps;
    use super::statusline::{StatusComponent, StatusLinePosition, StatusSegment};
    use super::*;
    use crate::ui::action::Action;
    use crate::ui::key::KeyMod::Alt;
    use crate::ui::render::maths::FractionStyle;

    /// A record written by one session reads back whole in the next, and the
    /// rebinds it replays restore the swap a taken key caused.
    #[test]
    fn saved_settings_survive_a_restart() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let path = ENV.temp_dir().join(format!(
            "spectral-m2-settings-{}-{nonce}.json",
            std::process::id()
        ));
        let mut keymaps = Keymaps::default();
        let taken = KeyEvent::from(Char('o')) + Alt;
        assert!(keymaps.rebind(Action::ClearFeed, taken));
        let mut saved = SavedSettings {
            rebinds: keymaps.rebinds().to_vec(),
            ..SavedSettings::default()
        };
        saved.rendering.fractions = FractionStyle::Inline;
        saved.statusline.position = StatusLinePosition::Off;
        saved.layout.output_rows = 7;
        saved.layout.scrolloff = 5;
        saved.selection.copy_on_select = false;
        saved.selection.raw = true;

        saved.write(&path).expect("write settings");
        let loaded = SavedSettings::read(&path).expect("read settings");
        let _ = fs::remove_file(&path);

        assert_eq!(loaded.rendering.fractions, FractionStyle::Inline);
        assert_eq!(loaded.statusline.position, StatusLinePosition::Off);
        assert_eq!(loaded.layout.output_rows, 7);
        assert_eq!(loaded.layout.scrolloff, 5);
        assert_eq!(loaded.selection, saved.selection);
        let replayed = Keymaps::replaying(&loaded.rebinds);
        for action in [Action::ClearFeed, Action::OpenOutput] {
            assert_eq!(replayed.key_for(action), keymaps.key_for(action));
        }
    }

    /// A rebind of an action this version no longer has costs only that rebind.
    #[test]
    fn a_rebind_of_a_removed_action_is_dropped_alone() {
        let keymaps = Keymaps::replaying(&[]);
        let kept = serde_json::to_value((
            Shortcut::Command(Action::ClearFeed),
            keymaps.key(Action::OpenOutput).expect("output has a key"),
        ))
        .expect("serializes");
        let record = serde_json::json!({
            "rendering": {"fractions": "underlined"},
            "rebinds": [["an action that was removed", kept[1].clone()], kept],
        });
        let saved: SavedSettings = serde_json::from_value(record).expect("parses");

        assert_eq!(saved.rendering.fractions, FractionStyle::Underlined);
        assert_eq!(saved.rebinds.len(), 1);
    }

    /// A record from an older version, missing fields added since, keeps the rest.
    #[test]
    fn a_partial_record_fills_in_defaults() {
        let saved: SavedSettings =
            serde_json::from_str(r#"{"rendering":{"fractions":"underlined"}}"#).expect("parses");

        assert_eq!(saved.rendering.fractions, FractionStyle::Underlined);
        assert_eq!(saved.layout, EditorSettings::default());
        assert!(saved.rebinds.is_empty());
        assert_eq!(saved.selection, SelectionSettings::default());
    }

    /// A file from before the values were sorted into sections keeps every one.
    #[test]
    fn a_record_keeping_values_under_editor_and_input_reads_into_the_sections() {
        let saved: SavedSettings = serde_json::from_str(
            r#"{
                "editor": {
                    "language_server": false,
                    "statusline": "above_input",
                    "idle_statusline": {"scheme": "right"},
                    "fractions": "inline",
                    "output_rows": 50,
                    "example_seconds": 2
                },
                "input": {"max_height_percent": 60, "complete_after": 3, "hintCompletionBelow": 4},
                "rebinds": [["CopySelection", {"code": {"Char": "y"}, "modifiers": 2}]]
            }"#,
        )
        .expect("parses");

        assert!(!saved.rendering.language_server);
        assert_eq!(saved.rendering.fractions, FractionStyle::Inline);
        assert_eq!(saved.rendering.example_seconds, 2);
        assert_eq!(saved.statusline.position, StatusLinePosition::AboveInput);
        assert_eq!(
            saved.statusline.placement(StatusComponent::Theme),
            StatusSegment::Right
        );
        assert_eq!(saved.layout.output_rows, 50);
        assert_eq!(saved.layout.max_input_height_percent, 60);
        assert_eq!(saved.layout.complete_after, 3);
        assert_eq!(saved.layout.hint_completion_below, 4);
        assert_eq!(saved.rebinds.len(), 1);
    }

    #[test]
    fn a_saved_record_reads_back_as_written() {
        let mut saved = SavedSettings::default();
        saved.rendering.semantic_highlighting = false;
        saved.rendering.reference_code_background = true;
        saved
            .statusline
            .set_placement(StatusComponent::Versions, StatusSegment::Right);
        let roundtrip: SavedSettings =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert!(!roundtrip.rendering.semantic_highlighting);
        assert!(roundtrip.rendering.reference_code_background);
        assert_eq!(roundtrip.statusline.components, saved.statusline.components);
    }
}
