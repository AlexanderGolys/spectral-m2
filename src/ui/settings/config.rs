use std::path::{Path, PathBuf};
use std::{fs, io};

#[cfg(not(test))]
use crate::env::ENV;
use serde::de::DeserializeOwned;
use serde::Serialize;

/// A settings record kept in a file of its own under the configuration directory.
pub trait Persisted: Serialize + DeserializeOwned + Default {
    /// The record's file inside the shell's configuration directory.
    const FILE: &'static str;

    /// The saved record, or the defaults when none was saved or it no longer reads.
    fn load() -> Self {
        config_file(Self::FILE)
            .as_deref()
            .and_then(Self::read)
            .unwrap_or_default()
    }

    /// Keep this record for the next session.
    fn save(&self) {
        if let Some(path) = config_file(Self::FILE) {
            let _ = self.write(&path);
        }
    }

    /// The record saved at `path`, when there is one that still reads.
    fn read(path: &Path) -> Option<Self> {
        serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
    }

    fn write(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(self)?)
    }
}

#[cfg(not(test))]
pub fn config_file(name: &str) -> Option<PathBuf> {
    let config = ENV.config_home()?;
    Some(config.join(env!("CARGO_PKG_NAME")).join(name))
}

/// Tests have no configuration directory, so a developer's saved settings neither
/// leak into them nor are overwritten by them.
#[cfg(test)]
pub fn config_file(_: &str) -> Option<PathBuf> {
    None
}
