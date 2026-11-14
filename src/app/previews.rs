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
    /// What the language server says about a name.
    Hover,
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
