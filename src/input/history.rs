//! Evaluated source history with a browse cursor. The in-progress draft is stashed on
//! first browse and restored when you step back past the newest entry.

use std::collections::HashMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Input text and cursor position restored from history.
pub(crate) struct HistoryBuffer {
    pub(crate) text: String,
    pub(crate) cursor: usize,
}

impl HistoryBuffer {
    fn at_end(text: String) -> Self {
        let cursor = text.len();
        Self { text, cursor }
    }
}

/// The recorded submissions, the browse position (`None` = editing the live draft), and
/// the draft stashed when browsing began.
pub(crate) struct History {
    entries: Vec<String>,
    /// Where each distinct entry last occurs, the one occurrence a search visits.
    newest: HashMap<String, usize>,
    pos: Option<usize>,
    draft: Option<HistoryBuffer>,
    path: Option<PathBuf>,
}

impl History {
    pub(crate) fn new() -> Self {
        Self::from_path(history_file_path())
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        Self::from_path(None)
    }

    fn from_path(path: Option<PathBuf>) -> Self {
        let entries = path.as_deref().map(load_entries).unwrap_or_default();
        let newest = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.clone(), index))
            .collect();
        History {
            entries,
            newest,
            pos: None,
            draft: None,
            path,
        }
    }

    pub(crate) fn record(&mut self, entry: &str) {
        let entry = entry.trim_end();
        if Self::keeps(entry) && self.entries.last().map(String::as_str) != Some(entry) {
            self.newest.insert(entry.to_string(), self.entries.len());
            self.entries.push(entry.to_string());
            self.append(entry);
        }
        self.pos = None;
        self.draft = None;
    }

    fn keeps(entry: &str) -> bool {
        !entry.trim().is_empty() && (!entry.starts_with('/') || entry.contains('\n'))
    }

    /// The newest older entry that starts with the draft's text before its cursor.
    ///
    /// As a shell's history search does: the prefix is fixed when browsing begins,
    /// so typing `R =` and stepping back visits only the entries that begin with it.
    /// An empty prefix matches everything, which is plain browsing.
    pub fn prev(&mut self, current: &str, cursor: usize) -> Option<HistoryBuffer> {
        self.end_search_if_edited(current);
        if self.pos.is_none() {
            self.draft = Some(HistoryBuffer {
                text: current.to_string(),
                cursor,
            });
        }
        let end = self.pos.unwrap_or(self.entries.len());
        let found = (0..end).rev().find(|&index| self.matches(index));
        if found.is_none() && self.pos.is_none() {
            self.draft = None;
        }
        self.show(found?)
    }

    /// The next newer matching entry, or the draft once there are none.
    pub fn next(&mut self, current: &str) -> Option<HistoryBuffer> {
        self.end_search_if_edited(current);
        let start = self.pos? + 1;
        match (start..self.entries.len()).find(|&index| self.matches(index)) {
            Some(index) => self.show(index),
            None => {
                self.pos = None;
                self.draft.take()
            }
        }
    }

    fn show(&mut self, index: usize) -> Option<HistoryBuffer> {
        self.pos = Some(index);
        Some(HistoryBuffer::at_end(self.entries[index].clone()))
    }

    /// Text edited after it was recalled starts a new search from what it now says,
    /// rather than stepping on with the prefix it no longer begins with.
    fn end_search_if_edited(&mut self, current: &str) {
        if self.pos.is_some_and(|pos| self.entries[pos] != current) {
            self.pos = None;
            self.draft = None;
        }
    }

    /// Whether the entry at `index` starts with the search prefix, is the newest
    /// occurrence of its text, and differs from the draft, so every step shows
    /// something new.
    fn matches(&self, index: usize) -> bool {
        let Some(draft) = &self.draft else {
            return false;
        };
        let entry = &self.entries[index];
        entry.starts_with(&draft.text[..draft.cursor])
            && self.newest.get(entry) == Some(&index)
            && *entry != draft.text
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
        let entry = entry.trim_end().to_string();
        if History::keeps(&entry) && entries.last().map(String::as_str) != Some(entry.as_str()) {
            entries.push(entry);
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
        history.record("  f = i -> (\n  i + 1\n)  \n");
        history.record(" \t");
        history.record("/ast");

        let mut reloaded = History::from_path(Some(path.clone()));
        assert_eq!(
            reloaded.prev("", 0).map(|buffer| buffer.text),
            Some("  f = i -> (\n  i + 1\n)".to_string())
        );
        assert_eq!(
            reloaded
                .prev("  f = i -> (\n  i + 1\n)", 0)
                .map(|buffer| buffer.text),
            Some("R = QQ[x]".to_string())
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn browsing_visits_only_entries_starting_with_the_typed_prefix() {
        let mut history = History::in_memory();
        for entry in ["R = QQ[x]", "f = 1", "R = ZZ[y]", "g = 2", "R = ZZ[y]"] {
            history.record(entry);
        }
        let text = |buffer: Option<HistoryBuffer>| buffer.map(|buffer| buffer.text);

        assert_eq!(text(history.prev("R =", 3)), Some("R = ZZ[y]".into()));
        assert_eq!(text(history.prev("R = ZZ[y]", 9)), Some("R = QQ[x]".into()));
        assert_eq!(text(history.prev("R = QQ[x]", 9)), None);
        assert_eq!(text(history.next("R = QQ[x]")), Some("R = ZZ[y]".into()));
        assert_eq!(text(history.next("R = ZZ[y]")), Some("R =".into()));

        assert_eq!(text(history.prev("h", 1)), None);
        assert_eq!(text(history.next("h")), None, "a failed search leaves the draft alone");
        assert_eq!(text(history.prev("", 0)), Some("R = ZZ[y]".into()));
        assert_eq!(text(history.prev("R = ZZ[y]", 9)), Some("g = 2".into()));
        assert_eq!(text(history.prev("g = 2", 5)), Some("f = 1".into()));
    }

    /// Editing a recalled entry ends the search, so the next step searches by what
    /// the input now begins with rather than the prefix it started from.
    #[test]
    fn editing_a_recalled_entry_starts_a_new_search() {
        let mut history = History::in_memory();
        for entry in ["1/2", "oo", "11", "ooo"] {
            history.record(entry);
        }
        let text = |buffer: Option<HistoryBuffer>| buffer.map(|buffer| buffer.text);

        assert_eq!(text(history.prev("o", 1)), Some("ooo".into()));
        assert_eq!(text(history.prev("1", 1)), Some("11".into()));
        assert_eq!(text(history.prev("11", 1)), Some("1/2".into()));
        assert_eq!(text(history.next("1/2")), Some("11".into()));
        assert_eq!(text(history.next("11")), Some("1".into()));
    }

    #[test]
    fn malformed_records_do_not_hide_valid_history() {
        let path = temporary_history_path();
        fs::write(&path, "not json\n\" \\t\"\n\"/debug\"\n\"valid entry\"\n")
            .expect("write test history");

        let mut history = History::from_path(Some(path.clone()));
        assert_eq!(
            history.prev("", 0).map(|buffer| buffer.text),
            Some("valid entry".to_string())
        );

        let _ = fs::remove_file(path);
    }
}
