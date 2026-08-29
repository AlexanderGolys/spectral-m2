//! Submitted-input history with a browse cursor. Recorded on every submission, browsed
//! with up/down; the in-progress draft is stashed on first browse and restored when you
//! step back past the newest entry.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Input text and cursor position restored from history.
pub struct HistoryBuffer {
    pub text: String,
    pub cursor: usize,
}

impl HistoryBuffer {
    fn at_end(text: String) -> Self {
        let cursor = text.len();
        Self { text, cursor }
    }
}

/// The recorded submissions, the browse position (`None` = editing the live draft), and
/// the draft stashed when browsing began.
pub struct History {
    entries: Vec<String>,
    pos: Option<usize>,
    draft: Option<HistoryBuffer>,
    path: Option<PathBuf>,
}

impl History {
    pub fn new() -> Self {
        Self::from_path(history_file_path())
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::from_path(None)
    }

    fn from_path(path: Option<PathBuf>) -> Self {
        let entries = path.as_deref().map(load_entries).unwrap_or_default();
        History {
            entries,
            pos: None,
            draft: None,
            path,
        }
    }

    pub fn record(&mut self, entry: &str) {
        let entry = entry.trim();
        if !entry.is_empty() && self.entries.last().map(String::as_str) != Some(entry) {
            self.entries.push(entry.to_string());
            self.append(entry);
        }
        self.pos = None;
        self.draft = None;
    }

    pub fn prev(&mut self, current: &str, cursor: usize) -> Option<HistoryBuffer> {
        if self.entries.is_empty() {
            return None;
        }
        let idx = match self.pos {
            None => {
                self.draft = Some(HistoryBuffer {
                    text: current.to_string(),
                    cursor,
                });
                self.entries.len() - 1
            }
            Some(0) => return None,
            Some(i) => i - 1,
        };
        self.pos = Some(idx);
        Some(HistoryBuffer::at_end(self.entries[idx].clone()))
    }

    pub fn next(&mut self) -> Option<HistoryBuffer> {
        let idx = match self.pos {
            None => return None,
            Some(i) if i + 1 < self.entries.len() => i + 1,
            Some(_) => {
                self.pos = None;
                return self.draft.take();
            }
        };
        self.pos = Some(idx);
        Some(HistoryBuffer::at_end(self.entries[idx].clone()))
    }

    fn append(&self, entry: &str) {
        let Some(path) = &self.path else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        if fs::create_dir_all(parent).is_err() {
            return;
        }
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        options.mode(0o600);
        let Ok(mut file) = options.open(path) else {
            return;
        };
        if let Ok(encoded) = serde_json::to_string(entry) {
            let _ = writeln!(file, "{encoded}");
        }
    }
}

fn history_file_path() -> Option<PathBuf> {
    let state_dir = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))?;
    Some(state_dir.join("spectral-m2").join("history.jsonl"))
}

fn load_entries(path: &Path) -> Vec<String> {
    let Ok(contents) = fs::read_to_string(path) else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    for line in contents.lines() {
        let Ok(entry) = serde_json::from_str::<String>(line) else {
            continue;
        };
        let entry = entry.trim();
        if !entry.is_empty() && entries.last().map(String::as_str) != Some(entry) {
            entries.push(entry.to_string());
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_history_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        env::temp_dir().join(format!(
            "spectral-m2-history-{}-{nonce}.jsonl",
            std::process::id()
        ))
    }

    #[test]
    fn submissions_survive_reloading() {
        let path = temporary_history_path();
        let mut history = History::from_path(Some(path.clone()));
        history.record("R = QQ[x]");
        history.record("f = i -> (\n  i + 1\n)");

        let mut reloaded = History::from_path(Some(path.clone()));
        assert_eq!(
            reloaded.prev("", 0).map(|buffer| buffer.text),
            Some("f = i -> (\n  i + 1\n)".to_string())
        );
        assert_eq!(
            reloaded.prev("", 0).map(|buffer| buffer.text),
            Some("R = QQ[x]".to_string())
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn malformed_records_do_not_hide_valid_history() {
        let path = temporary_history_path();
        fs::write(&path, "not json\n\"valid entry\"\n").expect("write test history");

        let mut history = History::from_path(Some(path.clone()));
        assert_eq!(
            history.prev("", 0).map(|buffer| buffer.text),
            Some("valid entry".to_string())
        );

        let _ = fs::remove_file(path);
    }
}
