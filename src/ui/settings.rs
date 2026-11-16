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
pub mod session;
pub mod statusline;

use ratatui::text::Span;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub use self::config::*;
use self::editor::EditorSettings;
pub use self::keymap::*;
use self::lsp::LspSettings;
use self::rendering::RenderingSettings;
use self::row::SettingsTab;
use self::statusline::StatusLineSettings;
use crate::ui::controller::picker::{PickerItem, PickerTab, Row};
use crate::ui::controller::EditorEvent;
use crate::ui::theme::groups::scheme;
use crate::ui::theme::AppearanceSettings;

/// Declare [`Settings`] from its sections, each with the field keeping it: the
/// struct and the picker's tabs both follow this one list, in its order, so no
/// section goes without its tab nor a tab without its section.
macro_rules! sections {
    ($($(#[$attribute:meta])* $field:ident: $section:ty),* $(,)?) => {
        /// Everything the settings picker adjusts: a section for each of its tabs,
        /// in the order the tabs show.
        #[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
        #[serde(default)]
        pub struct Settings {
            $($(#[$attribute])* pub $field: $section,)*
        }

        impl Settings {
            /// A tab for each section, in order.
            pub fn tabs(self) -> Vec<PickerTab> {
                vec![$(<$section as SettingsTab>::tab(self)),*]
            }
        }
    };
}

sections! {
    /// Kept in a file of its own, beside the schemes it chooses among.
    #[serde(skip)]
    appearance: AppearanceSettings,
    rendering: RenderingSettings,
    lsp: LspSettings,
    editor: EditorSettings,
    statusline: StatusLineSettings,
    session: session::SessionSettings,
}

impl Settings {
    /// Whether returned values lay out differently, moving their heights.
    pub fn reformats(&self, other: &Self) -> bool {
        self.editor.output_rows != other.editor.output_rows
            || self.editor.code_wrap != other.editor.code_wrap
            || self.rendering.reformats(&other.rendering)
    }

    /// The button resetting settings, keymaps and appearance.
    pub fn restore_button() -> Box<dyn PickerItem> {
        let label = Span::styled("restore defaults", scheme().settings_button());
        Row::enter(vec![label.into()], || EditorEvent::RestoreDefaults)
            .described("reset settings, keymaps and appearance")
            .into()
    }
}

/// What the settings and keymap pickers keep between sessions: every section of
/// [`Settings`] but the appearance, and the keys rebound.
#[derive(Default, Serialize, Deserialize)]
#[serde(from = "StoredSettings")]
pub struct SavedSettings {
    #[serde(flatten)]
    pub settings: Settings,
    /// Every rebind made, in order. Replaying them over the default keymaps restores
    /// the swaps they caused as well as the keys they set.
    pub rebinds: Vec<ShortcutKey>,
}

impl Persisted for SavedSettings {
    const FILE: &'static str = "settings.json";
}

/// A settings file as read, in the shape of whichever version saved it.
#[derive(Deserialize)]
struct StoredSettings(Map<String, Value>);

impl From<StoredSettings> for SavedSettings {
    fn from(stored: StoredSettings) -> Self {
        let mut stored = stored.upgraded();
        let rebinds = stored.take_rebinds();
        Self {
            settings: serde_json::from_value(Value::Object(stored.0)).unwrap_or_default(),
            rebinds,
        }
    }
}

impl StoredSettings {
    /// The file in this version's shape. The editor section was once saved as
    /// `layout`, and before that every value lay in flat `editor` and `input`
    /// records with no `rendering` section; the selection was a section of its
    /// own; the language server's values were among the rendering and editor
    /// ones. A section reads only its own fields, so a larger record is harmless.
    fn upgraded(mut self) -> Self {
        if !self.0.contains_key("rendering") {
            let flat: Map<String, Value> = ["editor", "input"]
                .into_iter()
                .filter_map(|key| match self.0.remove(key) {
                    Some(Value::Object(values)) => Some(values),
                    _ => None,
                })
                .flatten()
                .collect();
            for section in ["editor", "rendering", "statusline"] {
                self.0
                    .insert(section.to_string(), Value::Object(flat.clone()));
            }
        }
        if let Some(layout) = self.0.remove("layout") {
            self.0.insert("editor".to_string(), layout);
        }
        if let Some(selection) = self.0.remove("selection") {
            self.section("editor")
                .entry("selection")
                .or_insert(selection);
        }
        if !self.0.contains_key("lsp") {
            let lsp = self
                .section("rendering")
                .clone()
                .into_iter()
                .chain(self.section("editor").clone())
                .collect();
            self.0.insert("lsp".to_string(), Value::Object(lsp));
        }
        self
    }

    /// The section saved under `key`, made empty when there is none.
    fn section(&mut self, key: &str) -> &mut Map<String, Value> {
        let value = self
            .0
            .entry(key)
            .or_insert_with(|| Value::Object(Map::new()));
        if !value.is_object() {
            *value = Value::Object(Map::new());
        }
        value
            .as_object_mut()
            .expect("the section was just made an object")
    }

    /// The saved rebinds this version still reads: one naming an action since
    /// removed is dropped rather than losing every other setting.
    fn take_rebinds(&mut self) -> Vec<ShortcutKey> {
        match self.0.remove("rebinds") {
            Some(Value::Array(rebinds)) => rebinds
                .into_iter()
                .filter_map(|rebind| serde_json::from_value(rebind).ok())
                .collect(),
            _ => Vec::new(),
        }
    }
}
