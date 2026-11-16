//! Evaluated source history with a browse cursor. The in-progress draft is stashed on
//! first browse and restored when you step back past the newest entry.
//!
//! Every session appends to one history file and takes in what the others appended,
//! so input submitted in any session can be recalled in all of them.

#[cfg(not(test))]
use crate::ui::settings::session::SessionSettings;
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
        let path = history_file_path();
        if path != self.path {
            self.path = path;
            self.entries.clear();
            self.newest.clear();
            self.browse = None;
            self.synced = 0;
        }
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
    pub fn previous(
        &mut self,
        current: &str,
        cursor: usize,
        recall: Recall,
    ) -> Option<HistoryBuffer> {
        self.sync();
        self.end_search_if_edited(current, recall);
        let end = match &self.browse {
            Some(browse) => browse.pos,
            None => self.entries.len(),
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
        let found = (0..end)
            .rev()
            .find(|&index| self.matches(index, filter, &draft.text))?;
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
        self.sync();
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
    Some(SessionSettings::directory()?.join("history.jsonl"))
}
