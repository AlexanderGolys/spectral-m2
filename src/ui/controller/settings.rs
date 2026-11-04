//! User-adjustable settings, one section for each tab of the settings picker.
//!
//! Values the user is expected to tune live here rather than as constants, so the
//! settings picker has one place to read and write and the defaults stay visible:
//! a tab's module holds its values, their defaults and its rows.
//! Whatever the pickers change is kept between sessions, each record in a file of
//! its own under the user's configuration directory.

pub mod appearance;
pub mod keymap;
pub mod layout;
pub mod rendering;
pub mod selection;
pub mod statusline;
pub mod tabs;
pub mod theme;

#[cfg(not(test))]
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use self::keymap::Shortcut;
use self::layout::LayoutSettings;
use self::rendering::RenderingSettings;
use self::selection::SelectionSettings;
use self::statusline::StatusLineSettings;
use self::theme::Appearance;
use crate::ui::key::KeyEvent;

/// A settings record kept in a file of its own under the configuration directory.
pub trait Persisted: Serialize + DeserializeOwned + Default {
    /// The record's file inside the shell's configuration directory.
    const FILE: &'static str;

    /// The saved record, or the defaults when none was saved or it no longer reads.
    fn load() -> Self {
        config_file(Self::FILE)
            .as_deref()
            .and_then(read)
            .unwrap_or_default()
    }

    /// Keep this record for the next session.
    fn save(&self) {
        if let Some(path) = config_file(Self::FILE) {
            let _ = write(&path, self);
        }
    }
}

pub fn read<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

pub fn write<T: Serialize>(path: &Path, record: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(record)?)
}

#[cfg(not(test))]
pub(super) fn config_file(name: &str) -> Option<PathBuf> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join(env!("CARGO_PKG_NAME")).join(name))
}

/// Tests have no configuration directory, so a developer's saved settings neither
/// leak into them nor are overwritten by them.
#[cfg(test)]
pub(super) fn config_file(_: &str) -> Option<PathBuf> {
    None
}

/// Everything the settings picker adjusts, a section for each of its tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Settings {
    pub appearance: Appearance,
    pub layout: LayoutSettings,
    pub statusline: StatusLineSettings,
    pub rendering: RenderingSettings,
    pub selection: SelectionSettings,
}

impl Settings {
    /// Whether returned values lay out differently, moving their heights.
    pub fn reformats(&self, other: &Self) -> bool {
        self.layout.output_rows != other.layout.output_rows
            || self.rendering.reformats(&other.rendering)
    }
}

/// What the settings and keymap pickers adjust, apart from the appearance, which
/// keeps a file of its own.
#[derive(Default, Serialize, Deserialize)]
#[serde(from = "StoredSettings")]
pub struct SavedSettings {
    pub layout: LayoutSettings,
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
    layout: LayoutSettings,
    statusline: StatusLineSettings,
    rendering: RenderingSettings,
    selection: SelectionSettings,
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

/// The section reading its own fields out of an older file's values.
fn section_of<Section: DeserializeOwned + Default>(older: &Map<String, Value>) -> Section {
    serde_json::from_value(Value::Object(older.clone())).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crossterm::event::KeyCode::Char;

    use super::keymap::Keymaps;
    use super::statusline::{SegmentPlacement, StatusLinePosition};
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
        let path = env::temp_dir().join(format!(
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

        write(&path, &saved).expect("write settings");
        let loaded: SavedSettings = read(&path).expect("read settings");
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

    /// A record from an older version, missing fields added since, keeps the rest.
    #[test]
    fn a_partial_record_fills_in_defaults() {
        let saved: SavedSettings =
            serde_json::from_str(r#"{"rendering":{"fractions":"underlined"}}"#).expect("parses");

        assert_eq!(saved.rendering.fractions, FractionStyle::Underlined);
        assert_eq!(saved.layout, LayoutSettings::default());
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
        assert_eq!(saved.statusline.idle.scheme, SegmentPlacement::Right);
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
        saved.statusline.idle.hints = SegmentPlacement::Hidden;
        let roundtrip: SavedSettings =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert!(!roundtrip.rendering.semantic_highlighting);
        assert_eq!(roundtrip.statusline.idle.hints, SegmentPlacement::Hidden);
    }
}
