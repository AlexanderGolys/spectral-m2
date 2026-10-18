//! User-adjustable presentation settings.
//!
//! Values the user is expected to tune live here rather than as constants, so a
//! settings picker has one place to read and write and the defaults stay visible.
//! Whatever the pickers change is kept between sessions, each record in a file of
//! its own under the user's configuration directory.

pub mod keymap;
pub mod theme;

#[cfg(not(test))]
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::input::InputSettings;
use crate::ui::action::Action;
use crate::ui::config::UI;
use crate::ui::key::KeyEvent;
use crate::ui::render::maths::FractionStyle;

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
fn config_file(name: &str) -> Option<PathBuf> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join(env!("CARGO_PKG_NAME")).join(name))
}

/// Tests have no configuration directory, so a developer's saved settings neither
/// leak into them nor are overwritten by them.
#[cfg(test)]
fn config_file(_: &str) -> Option<PathBuf> {
    None
}

/// What the settings and keymap pickers adjust, apart from the appearance.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedSettings {
    pub editor: Settings,
    pub input: InputSettings,
    /// Every rebind made, in order. Replaying them over the default keymaps restores
    /// the swaps they caused as well as the keys they set.
    pub rebinds: Vec<(Action, KeyEvent)>,
}

impl Persisted for SavedSettings {
    const FILE: &'static str = "settings.json";
}

/// Presentation choices shared by the editor and its renderers.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub statusline: StatusLinePosition,
    pub fractions: FractionStyle,
    /// Whether rings are drawn as blackboard glyphs (`ℤ`) rather than by name (`ZZ`).
    pub math_fonts: bool,
    /// Rows a returned value shows before its middle folds away.
    pub output_rows: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            statusline: StatusLinePosition::default(),
            fractions: FractionStyle::default(),
            math_fonts: false,
            output_rows: UI.max_output_rows,
        }
    }
}

impl Settings {
    /// Move the output fold threshold by picker steps, keeping room for a head, the
    /// fold row, and a tail.
    pub fn adjust_output_rows(&mut self, delta: i8) {
        let step = usize::from(delta.unsigned_abs());
        self.output_rows = match delta > 0 {
            true => self.output_rows.saturating_add(step),
            false => self.output_rows.saturating_sub(step),
        }
        .max(UI.min_output_rows);
    }

    /// Whether returned values lay out differently, moving their heights.
    pub fn reformats(&self, other: &Self) -> bool {
        (self.fractions, self.math_fonts, self.output_rows)
            != (other.fractions, other.math_fonts, other.output_rows)
    }
}

/// Placement of the editor status line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusLinePosition {
    #[default]
    BelowInput,
    AboveInput,
    Off,
}

impl StatusLinePosition {
    pub const fn cycle(self, delta: i8) -> Self {
        match (self, delta > 0) {
            (Self::BelowInput, true) | (Self::Off, false) => Self::AboveInput,
            (Self::AboveInput, true) => Self::Off,
            (Self::AboveInput, false) => Self::BelowInput,
            (Self::Off, true) => Self::BelowInput,
            (Self::BelowInput, false) => Self::Off,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crossterm::event::KeyCode::Char;

    use super::keymap::Keymaps;
    use super::*;
    use crate::ui::key::KeyMod::Alt;

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
        saved.editor.fractions = FractionStyle::Inline;
        saved.editor.statusline = StatusLinePosition::Off;
        saved.editor.output_rows = 7;
        saved.input.scrolloff = 5;

        write(&path, &saved).expect("write settings");
        let loaded: SavedSettings = read(&path).expect("read settings");
        let _ = fs::remove_file(&path);

        assert_eq!(loaded.editor.fractions, FractionStyle::Inline);
        assert_eq!(loaded.editor.statusline, StatusLinePosition::Off);
        assert_eq!(loaded.editor.output_rows, 7);
        assert_eq!(loaded.input.scrolloff, 5);
        let replayed = Keymaps::replaying(&loaded.rebinds);
        for action in [Action::ClearFeed, Action::OpenOutput] {
            assert_eq!(replayed.key_for(action), keymaps.key_for(action));
        }
    }

    /// A record from an older version, missing fields added since, keeps the rest.
    #[test]
    fn a_partial_record_fills_in_defaults() {
        let saved: SavedSettings =
            serde_json::from_str(r#"{"editor":{"fractions":"underlined"}}"#).expect("parses");

        assert_eq!(saved.editor.fractions, FractionStyle::Underlined);
        assert_eq!(saved.editor.output_rows, Settings::default().output_rows);
        assert!(saved.rebinds.is_empty());
    }
}
