//! Storage shared by session archives, notes, and input history.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use serde::{Deserialize, Serialize};

use super::rendering::switch;
use super::row::{SettingRow, SettingsTab};
use super::Settings;
use crate::env::ENV;
use crate::ui::controller::picker::{PickerItem, PickerValue};

/// Session storage policy. With `local` enabled, a repository keeps its own
/// `.spectral-m2` directory instead of sharing the user's global history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionSettings {
    /// Keep sessions and history in the nearest repository, or current directory.
    pub local: bool,
}

static LOCAL: AtomicBool = AtomicBool::new(false);

impl SessionSettings {
    /// Publish the storage policy for subsequent reads and writes.
    pub fn apply(self) {
        LOCAL.store(self.local, Relaxed);
    }

    /// The active storage directory, without creating it.
    pub fn directory() -> Option<PathBuf> {
        if LOCAL.load(Relaxed) {
            let cwd = ENV.current_dir().ok()?;
            let root = cwd
                .ancestors()
                .find(|path| path.join(".git").exists())
                .unwrap_or(&cwd);
            Some(root.join(".spectral-m2"))
        } else {
            Some(ENV.state_home()?.join(env!("CARGO_PKG_NAME")))
        }
    }
}

/// Whether session files belong to this repository or to the user.
#[derive(Clone, Copy)]
pub struct LocalStorage;

impl SettingRow for LocalStorage {
    fn label(self) -> &'static str {
        "local session files"
    }

    fn description(self, _: Settings) -> &'static str {
        "keep sessions, notes and input history in this repository; off uses global storage"
    }

    fn value(self, settings: Settings) -> PickerValue {
        switch(settings.session.local, "on")
    }

    fn adjust(self, settings: &mut Settings, _: i8) {
        settings.session.local = !settings.session.local;
    }
}

impl SettingsTab for SessionSettings {
    const LABEL: &'static str = "session";

    fn rows(settings: Settings) -> Vec<Box<dyn PickerItem>> {
        vec![LocalStorage.row(settings)]
    }
}
