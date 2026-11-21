//! The tabs of the view beside the session: as many as the user opens, the views
//! that are always there, and one history across them all.

use std::borrow::Cow;
use std::collections::HashSet;
use std::ops::Range;

use crate::code::location::SourceLink;
use crate::ui::action::Opening;
use crate::ui::config::UI;
use crate::ui::controller::ViewKind;
use crate::ui::glyphs::Glyphs;

/// What a tab shows, and the key its page is kept under: two tabs on the same
/// target show the same page.
///
/// ```ignore
/// Target::link("ideal")              // the page `help ideal` shows
/// Target::link("file:///a.m2#3:0")   // a.m2, at line 3
/// Target::View(ViewKind::Code)       // the session's code, always there
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// The page `help <topic>` shows, titled `label`.
    Documentation {
        topic: String,
        label: String,
    },
    Source(SourceLink),
    /// What the language server said about `name`.
    Hover {
        name: String,
        text: String,
    },
    /// One of [`ViewKind::PREVIEWS`].
    View(ViewKind),
}

impl Target {
    /// What a link to `target` opens: a file for a `file:` URL, else documentation.
    pub fn link(target: &str) -> Option<Self> {
        match target.starts_with("file:") {
            true => SourceLink::parse(target).map(Self::Source),
            false => Some(Self::Documentation {
                topic: target.to_string(),
                label: target.trim_matches('"').to_string(),
            }),
        }
    }

    pub fn label(&self) -> Cow<'_, str> {
        match self {
            Self::Documentation { label, .. } => label.into(),
            Self::Source(source) => source
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
            Self::Hover { name, .. } => name.into(),
            Self::View(view) => view.label().into(),
        }
    }

    pub fn glyph(&self) -> Glyphs {
        match self {
            Self::Documentation { .. } => Glyphs::TabDocumentation,
            Self::Source(_) => Glyphs::TabSource,
            Self::Hover { .. } => Glyphs::TabHover,
            Self::View(ViewKind::Code) => Glyphs::TabCode,
            Self::View(ViewKind::SyntaxTree) => Glyphs::TabSyntaxTree,
            Self::View(ViewKind::Notes) => Glyphs::TabNotes,
            Self::View(_) => Glyphs::TabOutput,
        }
    }

    /// The kind of view drawing this target.
    pub fn view(&self) -> ViewKind {
        match self {
            Self::View(view) => *view,
            _ => ViewKind::Reference,
        }
    }
}

/// The identity of a tab the user opened, kept by history after the tab closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabId(u64);

/// A tab the user opened, showing whatever it was last sent to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub id: TabId,
    pub target: Target,
}

/// One step of history: `target` showing in the opened tab `tab`, or, with no
/// tab, as the view that is always there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Visit {
    pub tab: Option<TabId>,
    pub target: Target,
}

/// The tabs the user opened, in the order of the tab line, and the history of
/// what was shown, whose cursor is on what shows now.
#[derive(Debug, Default)]
pub struct Tabs {
    opened: Vec<Tab>,
    visits: Vec<Visit>,
    cursor: usize,
    next_id: u64,
    /// The opened tabs the tab line last had room for.
    shown: Range<usize>,
}

impl Tabs {
    /// What shows, or would were the view open.
    pub fn current(&self) -> Option<&Visit> {
        self.visits.get(self.cursor)
    }

    /// Where the tab showing stands among the opened ones; none for a view
    /// always there.
    fn position(&self) -> Option<usize> {
        let id = self.current()?.tab?;
        self.opened.iter().position(|tab| tab.id == id)
    }

    /// Record `visit` after the cursor, dropping what was ahead of it and, past
    /// [`UI::TAB_HISTORY`] visits, the oldest.
    fn visit(&mut self, visit: Visit) {
        if self.current() == Some(&visit) {
            return;
        }
        self.visits.truncate(self.cursor + 1);
        self.visits.push(visit);
        let excess = self.visits.len().saturating_sub(UI::TAB_HISTORY);
        self.visits.drain(..excess);
        self.cursor = self.visits.len() - 1;
    }

    /// Open `target` as `opening` says; whether it now shows. With no opened tab
    /// showing there is no place to open it in, so it takes a new one.
    pub fn open(&mut self, target: Target, opening: Opening) -> bool {
        let id = match (opening, self.position()) {
            (Opening::Here, Some(position)) => {
                self.opened[position].target = target.clone();
                self.opened[position].id
            }
            (_, position) => {
                self.next_id += 1;
                let id = TabId(self.next_id);
                let tab = Tab {
                    id,
                    target: target.clone(),
                };
                self.opened.insert(
                    position.map_or(self.opened.len(), |position| position + 1),
                    tab,
                );
                id
            }
        };
        let shows = opening != Opening::Beside;
        if shows {
            self.visit(Visit {
                tab: Some(id),
                target,
            });
        }
        shows
    }

    pub fn select(&mut self, id: TabId) {
        if let Some(tab) = self.opened.iter().find(|tab| tab.id == id) {
            let visit = Visit {
                tab: Some(id),
                target: tab.target.clone(),
            };
            self.visit(visit);
        }
    }

    pub fn view(&mut self, view: ViewKind) {
        let visit = Visit {
            tab: None,
            target: Target::View(view),
        };
        self.visit(visit);
    }

    /// Close the opened tab showing and show the one that takes its place, else
    /// the one before; whether a tab was there to show.
    pub fn close(&mut self) -> bool {
        let Some(position) = self.position() else {
            return false;
        };
        self.opened.remove(position);
        let next = position.min(self.opened.len().saturating_sub(1));
        match self.opened.get(next).map(|tab| tab.id) {
            Some(id) => {
                self.select(id);
                true
            }
            None => false,
        }
    }

    /// Move the cursor `delta` visits through history; whether it moved.
    pub fn step(&mut self, delta: isize) -> bool {
        let moved = self
            .cursor
            .checked_add_signed(delta)
            .filter(|&cursor| cursor < self.visits.len());
        if let Some(cursor) = moved {
            self.cursor = cursor;
            self.settle();
        }
        moved.is_some()
    }

    /// Make the opened tabs agree with the visit under the cursor: its tab shows
    /// the visit's target again, and is opened again if it was closed.
    pub fn settle(&mut self) {
        let Some(Visit {
            tab: Some(id),
            target,
        }) = self.current().cloned()
        else {
            return;
        };
        match self.opened.iter_mut().find(|tab| tab.id == id) {
            Some(tab) => tab.target = target,
            None => self.opened.push(Tab { id, target }),
        }
    }

    /// Show the tab `direction` steps along the tab line, wrapping, among its own
    /// kind: the views always there among themselves, the opened among theirs.
    pub fn step_along(&mut self, direction: i8) {
        match (self.position(), self.current().map(|visit| &visit.target)) {
            (Some(position), _) => {
                let count = self.opened.len() as isize;
                let next = (position as isize + isize::from(direction)).rem_euclid(count);
                self.select(self.opened[next as usize].id);
            }
            (None, Some(Target::View(view))) => self.view(view.cycle(direction)),
            _ => {}
        }
    }

    /// The opened tabs a tab line with room for `capacity` shows: as few steps
    /// from the ones it showed before as keep the tab showing in sight.
    pub fn window(&mut self, capacity: usize) -> &[Tab] {
        let mut first = self.shown.start;
        if let Some(position) = self.position() {
            first = first.clamp((position + 1).saturating_sub(capacity), position);
        }
        first = first.min(self.opened.len().saturating_sub(capacity));
        self.shown = first..self.opened.len().min(first + capacity);
        &self.opened[self.shown.clone()]
    }

    /// The targets whose pages are worth keeping: those of the tabs in the tab
    /// line, and of the [`UI::TAB_PAGES_KEPT`] distinct ones nearest the cursor in
    /// history.
    pub fn kept(&self) -> HashSet<&Target> {
        let mut visits = self.visits.iter().enumerate().collect::<Vec<_>>();
        visits.sort_by_key(|(index, _)| index.abs_diff(self.cursor));
        let mut kept = HashSet::new();
        for (_, visit) in visits {
            if kept.len() == UI::TAB_PAGES_KEPT {
                break;
            }
            kept.insert(&visit.target);
        }
        let shown = self.opened.get(self.shown.clone()).into_iter().flatten();
        kept.extend(shown.map(|tab| &tab.target));
        kept
    }
}
