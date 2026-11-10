//! A bounded cycle of retained views and one independently bookmarked view.

use std::array;

pub const RECENT_PREVIEWS: usize = 5;

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewTab {
    Recent(usize),
    Bookmark,
}

#[derive(Debug)]
pub struct PreviewTabs<T> {
    saved: [Option<T>; RECENT_PREVIEWS],
    labels: [Option<String>; RECENT_PREVIEWS],
    bookmark: Option<T>,
    bookmark_label: Option<String>,
    pub active: Option<PreviewTab>,
    next: usize,
}

impl<T> Default for PreviewTabs<T> {
    fn default() -> Self {
        Self {
            saved: array::from_fn(|_| None),
            labels: array::from_fn(|_| None),
            bookmark: None,
            bookmark_label: None,
            active: None,
            next: 0,
        }
    }
}

impl<T> PreviewTabs<T> {
    pub fn label(&self, tab: PreviewTab) -> Option<&str> {
        match tab {
            PreviewTab::Recent(index) => self.labels.get(index)?.as_deref(),
            PreviewTab::Bookmark => self.bookmark_label.as_deref(),
        }
    }

    pub fn save(&mut self, current: Option<T>) {
        if let Some(current) = current {
            match self.active {
                Some(PreviewTab::Recent(index)) => self.saved[index] = Some(current),
                Some(PreviewTab::Bookmark) => self.bookmark = Some(current),
                None => {}
            }
        }
    }

    /// The views set aside, which are not showing.
    pub fn saved(&self) -> impl Iterator<Item = &T> {
        self.saved.iter().flatten().chain(self.bookmark.iter())
    }

    pub fn saved_mut(&mut self, tab: PreviewTab) -> Option<&mut T> {
        match tab {
            PreviewTab::Recent(index) => self.saved.get_mut(index)?.as_mut(),
            PreviewTab::Bookmark => self.bookmark.as_mut(),
        }
    }

    pub fn open(&mut self, current: Option<T>, label: String) {
        self.save(current);
        self.saved[self.next] = None;
        self.labels[self.next] = Some(label);
        self.active = Some(PreviewTab::Recent(self.next));
        self.next = (self.next + 1) % RECENT_PREVIEWS;
    }

    /// Set the active view aside for one that holds no tab: it is gone once
    /// another is chosen, and never replaces a recent one.
    pub fn open_transient(&mut self, current: Option<T>) {
        self.save(current);
        self.active = None;
    }

    pub fn select(&mut self, tab: PreviewTab, current: Option<T>) -> Option<T> {
        self.save(current);
        let selected = match tab {
            PreviewTab::Recent(index) => self.saved.get_mut(index)?.take(),
            PreviewTab::Bookmark => self.bookmark.take(),
        }?;
        self.active = Some(tab);
        Some(selected)
    }

    /// Move the active view into the bookmark, outside the replacement cycle.
    pub fn pin(&mut self) {
        let Some(PreviewTab::Recent(index)) = self.active else {
            return;
        };
        self.bookmark_label = self.labels[index].take();
        self.bookmark = None;
        self.saved[index] = None;
        self.active = Some(PreviewTab::Bookmark);
    }

    pub fn previous(&self) -> Option<PreviewTab> {
        self.neighbour(-1)
    }

    pub fn neighbour(&self, direction: i8) -> Option<PreviewTab> {
        let start = match self.active? {
            PreviewTab::Recent(index) => index,
            PreviewTab::Bookmark => self.next,
        };
        (1..=RECENT_PREVIEWS)
            .map(|step| {
                PreviewTab::Recent(if direction < 0 {
                    (start + RECENT_PREVIEWS - step) % RECENT_PREVIEWS
                } else {
                    (start + step) % RECENT_PREVIEWS
                })
            })
            .find(|tab| Some(*tab) != self.active && self.label(*tab).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_set_aside_are_listed_with_the_bookmark_but_not_the_active_one() {
        let mut tabs = PreviewTabs::default();
        tabs.open(None, "first".into());
        tabs.open(Some(1), "second".into());
        tabs.open(Some(2), "third".into());
        tabs.pin();
        tabs.open(Some(3), "fourth".into());

        let mut saved = tabs.saved().copied().collect::<Vec<_>>();
        saved.sort();

        assert_eq!(saved, [1, 2, 3]);
    }

    #[test]
    fn replacement_is_cyclic_even_after_visiting_an_older_view() {
        let mut tabs = PreviewTabs::default();
        let mut current = None;
        for item in 0..tabs.saved.len() {
            tabs.open(current, item.to_string());
            current = Some(item);
        }
        let first = PreviewTab::Recent(0);
        current = tabs.select(first, current);
        assert_eq!(current, Some(0));
        tabs.open(current, "replacement".into());
        assert_eq!(tabs.active, Some(first));
        assert_eq!(tabs.label(first), Some("replacement"));
        assert_eq!(tabs.select(PreviewTab::Recent(1), Some(99)), Some(1));
    }

    #[test]
    fn a_transient_view_keeps_the_tab_it_covers_and_is_gone_once_left() {
        let mut tabs = PreviewTabs::default();
        tabs.open(None, "kept".into());
        let kept = tabs.active.unwrap();
        tabs.open_transient(Some(1));
        assert_eq!(tabs.active, None);
        assert_eq!(tabs.select(kept, Some(2)), Some(1));
        assert!(tabs.saved().all(|saved| *saved != 2));
    }

    #[test]
    fn bookmark_survives_more_than_a_complete_replacement_cycle() {
        let mut tabs = PreviewTabs::default();
        tabs.open(None, "kept".into());
        tabs.pin();
        let mut current = Some(42);
        for item in 0..tabs.saved.len() * 2 {
            tabs.open(current, item.to_string());
            current = Some(item);
        }
        assert_eq!(tabs.label(PreviewTab::Bookmark), Some("kept"));
        assert_eq!(tabs.select(PreviewTab::Bookmark, current), Some(42));
    }
}
