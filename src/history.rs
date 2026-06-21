//! Submitted-input history with a browse cursor. Recorded on every submission, browsed
//! with up/down; the in-progress draft is stashed on first browse and restored when you
//! step back past the newest entry.

use std::mem;

/// The recorded submissions, the browse position (`None` = editing the live draft), and
/// the draft stashed when browsing began.
pub struct History {
    entries: Vec<String>,
    pos: Option<usize>,
    draft: String,
}

impl History {
    pub fn new() -> Self {
        History {
            entries: Vec::new(),
            pos: None,
            draft: String::new(),
        }
    }

    pub fn record(&mut self, entry: &str) {
        let entry = entry.trim();
        if !entry.is_empty() && self.entries.last().map(String::as_str) != Some(entry) {
            self.entries.push(entry.to_string());
        }
        self.pos = None;
        self.draft.clear();
    }

    pub fn prev(&mut self, current: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let idx = match self.pos {
            None => {
                self.draft = current.to_string();
                self.entries.len() - 1
            }
            Some(0) => return None,
            Some(i) => i - 1,
        };
        self.pos = Some(idx);
        Some(self.entries[idx].clone())
    }

    pub fn next(&mut self) -> Option<String> {
        let idx = match self.pos {
            None => return None,
            Some(i) if i + 1 < self.entries.len() => i + 1,
            Some(_) => {
                self.pos = None;
                return Some(mem::take(&mut self.draft));
            }
        };
        self.pos = Some(idx);
        Some(self.entries[idx].clone())
    }
}
