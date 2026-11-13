//! Evaluated source history with a browse cursor. The in-progress draft is stashed on
//! first browse and restored when you step back past the newest entry.
//!
//! Every session appends to one history file and takes in what the others appended,
//! so input submitted in any session can be recalled in all of them.

use crate::env::ENV;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Input text and cursor position restored from history.
#[derive(Clone, Debug)]
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

/// Which recorded entries a step through history may land on.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recall {
    /// Only entries beginning with what was typed when browsing began.
    Prefixed,
    /// Every entry, whatever was typed.
    Any,
}

/// One walk through history: the entry shown, and the draft it began from with
/// the prefix that draft fixed. Only a submission ends it outright.
struct Browse {
    pos: usize,
    draft: HistoryBuffer,
    prefix: String,
    /// Whether the input was edited or its cursor moved since the entry was shown:
    /// a prefixed step then searches afresh, a forced one walks on.
    detached: bool,
}

/// The recorded submissions and the walk through them, if one is under way.
pub struct History {
    entries: Vec<String>,
    /// Where each distinct entry last occurs, the one occurrence a search visits.
    newest: HashMap<String, usize>,
    browse: Option<Browse>,
    path: Option<PathBuf>,
    /// How many bytes of the history file have been read into `entries`.
    synced: u64,
}

impl History {
    pub fn browsing(&self, current: &str) -> bool {
        self.browse
            .as_ref()
            .is_some_and(|browse| !browse.detached && self.entries[browse.pos] == current)
    }

    pub fn stop_browsing(&mut self) {
        if let Some(browse) = &mut self.browse {
            browse.detached = true;
        }
    }

    pub fn new() -> Self {
        Self::from_path(history_file_path())
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::from_path(None)
    }

    fn from_path(path: Option<PathBuf>) -> Self {
        let mut history = History {
            entries: Vec::new(),
            newest: HashMap::new(),
            browse: None,
            path,
            synced: 0,
        };
        history.sync();
        history
    }

    /// Record a submission in the shared file, then read it back with whatever
    /// other sessions appended before it, so every session keeps one order.
    pub fn record(&mut self, entry: &str) {
        let entry = entry.trim_end();
        self.sync();
        if self.continues_with(entry) {
            let appended = self
                .path
                .as_deref()
                .is_some_and(|path| append(path, entry).is_ok());
            if appended {
                self.sync();
            } else {
                self.push(entry);
            }
        }
        self.browse = None;
    }

    /// Whether `entry` is worth keeping and not a repeat of the newest one.
    fn continues_with(&self, entry: &str) -> bool {
        Self::keeps(entry) && self.entries.last().map(String::as_str) != Some(entry)
    }

    fn push(&mut self, entry: &str) {
        if self.continues_with(entry) {
            self.newest.insert(entry.to_string(), self.entries.len());
            self.entries.push(entry.to_string());
        }
    }

    /// Take in the complete records appended to the history file since it was
    /// last read. A file shorter than what was read has been rewritten, so it is
    /// read again from the start. Malformed records are skipped.
    fn sync(&mut self) {
        let Some(path) = &self.path else {
            return;
        };
        let Ok(mut file) = File::open(path) else {
            return;
        };
        let Ok(metadata) = file.metadata() else {
            return;
        };
        if metadata.len() < self.synced {
            self.entries.clear();
            self.newest.clear();
            self.synced = 0;
        }
        let mut appended = Vec::new();
        let read = file
            .seek(SeekFrom::Start(self.synced))
            .and_then(|_| file.read_to_end(&mut appended));
        if read.is_err() {
            return;
        }
        let complete = appended
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(0, |index| index + 1);
        self.synced += complete as u64;
        for line in String::from_utf8_lossy(&appended[..complete]).lines() {
            if let Ok(entry) = serde_json::from_str::<String>(line) {
                self.push(entry.trim_end());
            }
        }
    }

    fn keeps(entry: &str) -> bool {
        !entry.trim().is_empty()
    }

    /// Step to an older entry, as a shell's history search does: the prefix is
    /// what stood before the cursor when browsing began (the whole draft when it
    /// spans lines), so typing `R =` and stepping back visits only the entries
    /// that begin with it. [`Recall::Any`] steps past the prefix to the very
    /// previous entry without forgetting it for the steps after.
    pub fn previous(&mut self, current: &str, cursor: usize, recall: Recall) -> Option<HistoryBuffer> {
        self.end_search_if_edited(current, recall);
        let end = match &self.browse {
            Some(browse) => browse.pos,
            None => {
                self.sync();
                self.entries.len()
            }
        };
        let draft = match &self.browse {
            Some(browse) => browse.draft.clone(),
            None => HistoryBuffer {
                text: current.to_string(),
                cursor,
            },
        };
        let prefix = match &self.browse {
            Some(browse) => browse.prefix.clone(),
            None if draft.text.contains('\n') => draft.text.clone(),
            None => draft.text[..draft.cursor].to_string(),
        };
        let filter = Self::filter(&prefix, recall);
        let found = (0..end).rev().find(|&index| self.matches(index, filter, &draft.text))?;
        self.browse = Some(Browse {
            pos: found,
            draft,
            prefix,
            detached: false,
        });
        Some(HistoryBuffer::at_end(self.entries[found].clone()))
    }

    /// Step to a newer entry, or back to the draft once there are none.
    pub fn next(&mut self, current: &str, recall: Recall) -> Option<HistoryBuffer> {
        self.end_search_if_edited(current, recall);
        let browse = self.browse.as_ref()?;
        let filter = Self::filter(&browse.prefix, recall);
        let found = (browse.pos + 1..self.entries.len())
            .find(|&index| self.matches(index, filter, &browse.draft.text));
        match found {
            Some(index) => {
                let browse = self.browse.as_mut()?;
                browse.pos = index;
                browse.detached = false;
                Some(HistoryBuffer::at_end(self.entries[index].clone()))
            }
            None => self.browse.take().map(|browse| browse.draft),
        }
    }

    fn filter(prefix: &str, recall: Recall) -> &str {
        match recall {
            Recall::Prefixed => prefix,
            Recall::Any => "",
        }
    }

    /// Text edited after it was recalled starts a new prefixed search from what it
    /// now says, rather than stepping on with the prefix it no longer begins with.
    fn end_search_if_edited(&mut self, current: &str, recall: Recall) {
        if recall == Recall::Prefixed && !self.browsing(current) {
            self.browse = None;
        }
    }

    /// Whether the entry at `index` starts with `prefix`, is the newest
    /// occurrence of its text, and differs from the draft, so every step shows
    /// something new.
    fn matches(&self, index: usize, prefix: &str, draft: &str) -> bool {
        let entry = &self.entries[index];
        entry.starts_with(prefix) && self.newest.get(entry) == Some(&index) && entry != draft
    }
}

/// Append one record to the history file, creating it private to the user.
fn append(path: &Path, entry: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    writeln!(file, "{}", serde_json::to_string(entry)?)
}

/// Tests keep no history file, so what they submit never reaches a developer's.
#[cfg(test)]
fn history_file_path() -> Option<PathBuf> {
    None
}

#[cfg(not(test))]
fn history_file_path() -> Option<PathBuf> {
    let state_dir = ENV.state_home()?;
    Some(state_dir.join("spectral-m2").join("history.jsonl"))
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
        ENV.temp_dir().join(format!(
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

        let mut reloaded = History::from_path(Some(path.clone()));
        assert_eq!(
            reloaded.previous("", 0, Recall::Prefixed).map(|buffer| buffer.text),
            Some("  f = i -> (\n  i + 1\n)".to_string())
        );
        assert_eq!(
            reloaded
                .previous("  f = i -> (\n  i + 1\n)", 0, Recall::Prefixed)
                .map(|buffer| buffer.text),
            Some("R = QQ[x]".to_string())
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn sessions_recall_what_each_other_submitted() {
        let path = temporary_history_path();
        let mut first = History::from_path(Some(path.clone()));
        let mut second = History::from_path(Some(path.clone()));
        let text = |buffer: Option<HistoryBuffer>| buffer.map(|buffer| buffer.text);

        first.record("R = QQ[x]");
        second.record("f = x^2");
        first.record("g = 1");

        assert_eq!(text(second.previous("", 0, Recall::Prefixed)), Some("g = 1".into()));
        assert_eq!(text(second.previous("g = 1", 0, Recall::Prefixed)), Some("f = x^2".into()));
        assert_eq!(text(second.previous("f = x^2", 0, Recall::Prefixed)), Some("R = QQ[x]".into()));
        assert_eq!(text(first.previous("f", 1, Recall::Prefixed)), Some("f = x^2".into()));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn browsing_visits_only_entries_starting_with_the_typed_prefix() {
        let mut history = History::in_memory();
        for entry in ["R = QQ[x]", "f = 1", "R = ZZ[y]", "g = 2", "R = ZZ[y]"] {
            history.record(entry);
        }
        let text = |buffer: Option<HistoryBuffer>| buffer.map(|buffer| buffer.text);

        assert_eq!(text(history.previous("R =", 3, Recall::Prefixed)), Some("R = ZZ[y]".into()));
        assert_eq!(text(history.previous("R = ZZ[y]", 9, Recall::Prefixed)), Some("R = QQ[x]".into()));
        assert_eq!(text(history.previous("R = QQ[x]", 9, Recall::Prefixed)), None);
        assert_eq!(text(history.next("R = QQ[x]", Recall::Prefixed)), Some("R = ZZ[y]".into()));
        assert_eq!(text(history.next("R = ZZ[y]", Recall::Prefixed)), Some("R =".into()));

        assert_eq!(text(history.previous("h", 1, Recall::Prefixed)), None);
        assert_eq!(
            text(history.next("h", Recall::Prefixed)),
            None,
            "a failed search leaves the draft alone"
        );
        assert_eq!(text(history.previous("", 0, Recall::Prefixed)), Some("R = ZZ[y]".into()));
        assert_eq!(text(history.previous("R = ZZ[y]", 9, Recall::Prefixed)), Some("g = 2".into()));
        assert_eq!(text(history.previous("g = 2", 5, Recall::Prefixed)), Some("f = 1".into()));
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

        assert_eq!(text(history.previous("o", 1, Recall::Prefixed)), Some("ooo".into()));
        assert_eq!(text(history.previous("1", 1, Recall::Prefixed)), Some("11".into()));
        assert_eq!(text(history.previous("11", 1, Recall::Prefixed)), Some("1/2".into()));
        assert_eq!(text(history.next("1/2", Recall::Prefixed)), Some("11".into()));
        assert_eq!(text(history.next("11", Recall::Prefixed)), Some("1".into()));
    }

    /// A forced step lands on the very previous entry, and the steps after it go
    /// back to the prefix the search began with, also once a new search starts.
    #[test]
    fn a_forced_step_ignores_the_prefix_only_once() {
        let mut history = History::in_memory();
        for entry in ["R = QQ[x]", "f = 1", "R = ZZ[y]", "g = 2"] {
            history.record(entry);
        }
        let text = |buffer: Option<HistoryBuffer>| buffer.map(|buffer| buffer.text);

        assert_eq!(text(history.previous("R", 1, Recall::Any)), Some("g = 2".into()));
        assert_eq!(text(history.previous("g = 2", 5, Recall::Prefixed)), Some("R = ZZ[y]".into()));
        assert_eq!(text(history.previous("R = ZZ[y]", 9, Recall::Any)), Some("f = 1".into()));
        assert_eq!(text(history.next("f = 1", Recall::Prefixed)), Some("R = ZZ[y]".into()));
        assert_eq!(text(history.next("R = ZZ[y]", Recall::Prefixed)), Some("R".into()));

        history.stop_browsing();
        assert_eq!(text(history.previous("f", 1, Recall::Prefixed)), Some("f = 1".into()));
    }

    #[test]
    fn malformed_records_do_not_hide_valid_history() {
        let path = temporary_history_path();
        fs::write(&path, "not json\n\" \\t\"\n\"/debug\"\n\"valid entry\"\n")
            .expect("write test history");

        let mut history = History::from_path(Some(path.clone()));
        assert_eq!(
            history.previous("", 0, Recall::Prefixed).map(|buffer| buffer.text),
            Some("valid entry".to_string())
        );

        let _ = fs::remove_file(path);
    }
}
