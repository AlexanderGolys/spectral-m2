//! The views beside the session: a bounded cycle of the ones the user opened, one
//! independently bookmarked, and one for each view that is always there to open.

use std::array;
use std::collections::HashMap;

use crate::ui::controller::ViewKind;

pub const RECENT_PREVIEWS: usize = 5;

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewTab {
    Recent(usize),
    Bookmark,
    /// One of [`ViewKind::PREVIEWS`], whatever else is open.
    Fixed(ViewKind),
}

/// What a tab the user opened shows, which picks its icon.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenedKind {
    Documentation,
    Source,
}

/// A tab the user opened: its title and what it shows.
#[derive(Clone, Debug)]
pub struct Opened {
    pub label: String,
    pub kind: OpenedKind,
}

#[derive(Debug)]
pub struct PreviewTabs<T> {
    saved: [Option<T>; RECENT_PREVIEWS],
    opened: [Option<Opened>; RECENT_PREVIEWS],
    bookmark: Option<T>,
    bookmarked: Option<Opened>,
    fixed: HashMap<ViewKind, T>,
    pub active: Option<PreviewTab>,
    next: usize,
}

impl<T> Default for PreviewTabs<T> {
    fn default() -> Self {
        Self {
            saved: array::from_fn(|_| None),
            opened: array::from_fn(|_| None),
            bookmark: None,
            bookmarked: None,
            fixed: HashMap::new(),
            active: None,
            next: 0,
        }
    }
}

impl<T> PreviewTabs<T> {
    pub fn label(&self, tab: PreviewTab) -> Option<&str> {
        match tab {
            PreviewTab::Fixed(view) => Some(view.label()),
            tab => self.opened(tab).map(|opened| opened.label.as_str()),
        }
    }

    /// The tab the user opened as `tab`, if it holds one.
    pub fn opened(&self, tab: PreviewTab) -> Option<&Opened> {
        match tab {
            PreviewTab::Recent(index) => self.opened.get(index)?.as_ref(),
            PreviewTab::Bookmark => self.bookmarked.as_ref(),
            PreviewTab::Fixed(_) => None,
        }
    }

    /// The tabs the user opened that hold a view, oldest slot first, then the bookmark.
    pub fn opened_tabs(&self) -> impl Iterator<Item = PreviewTab> + '_ {
        (0..RECENT_PREVIEWS)
            .map(PreviewTab::Recent)
            .chain([PreviewTab::Bookmark])
            .filter(|&tab| self.opened(tab).is_some())
    }

    pub fn save(&mut self, current: Option<T>) {
        if let Some(current) = current {
            match self.active {
                Some(PreviewTab::Recent(index)) => self.saved[index] = Some(current),
                Some(PreviewTab::Bookmark) => self.bookmark = Some(current),
                Some(PreviewTab::Fixed(view)) => {
                    self.fixed.insert(view, current);
                }
                None => {}
            }
        }
    }

    /// The views set aside, which are not showing.
    pub fn saved(&self) -> impl Iterator<Item = &T> {
        self.saved
            .iter()
            .flatten()
            .chain(self.bookmark.iter())
            .chain(self.fixed.values())
    }

    pub fn saved_mut(&mut self, tab: PreviewTab) -> Option<&mut T> {
        match tab {
            PreviewTab::Recent(index) => self.saved.get_mut(index)?.as_mut(),
            PreviewTab::Bookmark => self.bookmark.as_mut(),
            PreviewTab::Fixed(view) => self.fixed.get_mut(&view),
        }
    }

    pub fn open(&mut self, current: Option<T>, opened: Opened) {
        self.save(current);
        self.saved[self.next] = None;
        self.opened[self.next] = Some(opened);
        self.active = Some(PreviewTab::Recent(self.next));
        self.next = (self.next + 1) % RECENT_PREVIEWS;
    }

    /// Set the active view aside for one that holds no tab: it is gone once
    /// another is chosen, and never replaces a recent one.
    pub fn open_transient(&mut self, current: Option<T>) {
        self.save(current);
        self.active = None;
    }

    /// Show `tab`, setting `current` aside: the view it kept, or none for a fixed
    /// view not yet opened, which then starts afresh.
    pub fn select(&mut self, tab: PreviewTab, current: Option<T>) -> Option<T> {
        self.save(current);
        let selected = match tab {
            PreviewTab::Recent(index) => self.saved.get_mut(index)?.take(),
            PreviewTab::Bookmark => self.bookmark.take(),
            PreviewTab::Fixed(view) => {
                self.active = Some(tab);
                return self.fixed.remove(&view);
            }
        }?;
        self.active = Some(tab);
        Some(selected)
    }

    /// Move the active view into the bookmark, outside the replacement cycle.
    pub fn pin(&mut self) {
        let Some(PreviewTab::Recent(index)) = self.active else {
            return;
        };
        self.bookmarked = self.opened[index].take();
        self.bookmark = None;
        self.saved[index] = None;
        self.active = Some(PreviewTab::Bookmark);
    }

    /// The tab the user opened before the one showing.
    pub fn previous(&self) -> Option<PreviewTab> {
        self.recent_neighbour(-1)
    }

    /// The tab `direction` steps away among its own kind: the views always there
    /// step among themselves, the ones the user opened among theirs.
    pub fn neighbour(&self, direction: i8) -> Option<PreviewTab> {
        match self.active? {
            PreviewTab::Fixed(view) => Some(PreviewTab::Fixed(view.cycle(direction))),
            _ => self.recent_neighbour(direction),
        }
    }

    fn recent_neighbour(&self, direction: i8) -> Option<PreviewTab> {
        let start = match self.active {
            Some(PreviewTab::Recent(index)) => index,
            _ => self.next,
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

    fn opened(label: &str) -> Opened {
        Opened {
            label: label.to_string(),
            kind: OpenedKind::Source,
        }
    }

    #[test]
    fn fixed_views_keep_their_state_apart_from_the_tabs_the_user_opened() {
        let mut tabs = PreviewTabs::default();
        tabs.open(None, opened("page"));
        let page = tabs.active.unwrap();
        let code = PreviewTab::Fixed(ViewKind::Code);
        assert_eq!(tabs.select(code, Some(1)), None);
        assert_eq!(tabs.opened_tabs().collect::<Vec<_>>(), [page]);
        assert_eq!(
            tabs.neighbour(1),
            Some(PreviewTab::Fixed(ViewKind::Code.cycle(1)))
        );
        assert_eq!(tabs.select(page, Some(2)), Some(1));
        assert_eq!(tabs.select(code, Some(3)), Some(2));
        assert_eq!(tabs.previous(), Some(page));
    }

    #[test]
    fn views_set_aside_are_listed_with_the_bookmark_but_not_the_active_one() {
        let mut tabs = PreviewTabs::default();
        tabs.open(None, opened("first"));
        tabs.open(Some(1), opened("second"));
        tabs.open(Some(2), opened("third"));
        tabs.pin();
        tabs.open(Some(3), opened("fourth"));

        let mut saved = tabs.saved().copied().collect::<Vec<_>>();
        saved.sort();

        assert_eq!(saved, [1, 2, 3]);
    }

    #[test]
    fn replacement_is_cyclic_even_after_visiting_an_older_view() {
        let mut tabs = PreviewTabs::default();
        let mut current = None;
        for item in 0..tabs.saved.len() {
            tabs.open(current, opened(&item.to_string()));
            current = Some(item);
        }
        let first = PreviewTab::Recent(0);
        current = tabs.select(first, current);
        assert_eq!(current, Some(0));
        tabs.open(current, opened("replacement"));
        assert_eq!(tabs.active, Some(first));
        assert_eq!(tabs.label(first), Some("replacement"));
        assert_eq!(tabs.select(PreviewTab::Recent(1), Some(99)), Some(1));
    }

    #[test]
    fn a_transient_view_keeps_the_tab_it_covers_and_is_gone_once_left() {
        let mut tabs = PreviewTabs::default();
        tabs.open(None, opened("kept"));
        let kept = tabs.active.unwrap();
        tabs.open_transient(Some(1));
        assert_eq!(tabs.active, None);
        assert_eq!(tabs.select(kept, Some(2)), Some(1));
        assert!(tabs.saved().all(|saved| *saved != 2));
    }

    #[test]
    fn bookmark_survives_more_than_a_complete_replacement_cycle() {
        let mut tabs = PreviewTabs::default();
        tabs.open(None, opened("kept"));
        tabs.pin();
        let mut current = Some(42);
        for item in 0..tabs.saved.len() * 2 {
            tabs.open(current, opened(&item.to_string()));
            current = Some(item);
        }
        assert_eq!(tabs.label(PreviewTab::Bookmark), Some("kept"));
        assert_eq!(tabs.select(PreviewTab::Bookmark, current), Some(42));
    }
}
