//! The session's panes: which views they show, which has focus, and how they
//! share the width.

use crossterm::event::KeyCode;
use ratatui::layout::Rect;

use crate::ui::action::Action;
use crate::ui::controller::{ViewKind, Viewport};
use crate::ui::key::KeyEvent;

pub const MIN_PANE_WIDTH: u16 = 40;
pub const MIN_SPLIT_WIDTH: u16 = MIN_PANE_WIDTH * 2 + 1;

/// One visible or width-collapsed window onto the session.
pub struct Pane {
    pub view: ViewKind,
    pub viewport: Viewport,
}

impl Pane {
    pub fn new(view: ViewKind) -> Self {
        Self {
            view,
            viewport: Viewport::at_bottom(),
        }
    }

    /// Consume a command owned by the current preview, or return it unchanged.
    pub fn handle_action(&mut self, action: Action) -> Option<Action> {
        if self.view == ViewKind::Shell {
            return Some(action);
        }
        match action {
            Action::ScrollFeedUp => self.viewport.scroll = self.viewport.scroll.saturating_sub(10),
            Action::ScrollFeedDown => self.viewport.scroll += 10,
            _ => return Some(action),
        }
        None
    }

    /// Interpret unbound `Up` and `Down` as scrolling the current preview. The
    /// shell and the reference pane keep their keys.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if !self.view.is_preview() {
            return false;
        }
        match key.code {
            KeyCode::Up => self.viewport.scroll = self.viewport.scroll.saturating_sub(1),
            KeyCode::Down => self.viewport.scroll += 1,
            _ => return false,
        }
        true
    }
}

/// The session's panes and which one has focus.
///
/// The first pane is always the session's shell. A second pane exists only to
/// show another view of the same session beside it; an independent session is a
/// new terminal pane away.
pub struct Panes {
    pub list: Vec<Pane>,
    pub active: usize,
    /// Whether the last layout had a split too narrow to show side by side.
    collapsed: bool,
}

impl Panes {
    /// Index of the pane that shows a view beside the session.
    pub const SPLIT: usize = 1;

    pub fn new() -> Self {
        Self {
            list: vec![Pane::new(ViewKind::Shell)],
            active: 0,
            collapsed: false,
        }
    }

    pub fn focused(&self) -> &Pane {
        &self.list[self.active]
    }

    pub fn focused_mut(&mut self) -> &mut Pane {
        &mut self.list[self.active]
    }

    pub fn split(&self) -> bool {
        self.list.len() > Self::SPLIT
    }

    /// Show `view` beside the session; showing the view already beside it takes it
    /// back down.
    pub fn open_view(&mut self, view: ViewKind) {
        if !self.split() {
            self.list.push(Pane::new(view));
        } else if self.list[Self::SPLIT].view == view {
            self.close_view();
        } else {
            self.list[Self::SPLIT] = Pane::new(view);
        }
    }

    /// Close the view beside the session and return focus to the shell.
    pub fn close_view(&mut self) {
        self.list.truncate(Self::SPLIT);
        self.active = 0;
    }

    /// Fit the panes to `width` columns. A split too narrow to show side by side
    /// shows the view full width instead, from the moment it opens or the window
    /// narrows; closing the view brings the shell back.
    pub fn fit(&mut self, width: u16) {
        let collapsed = self.split() && width < MIN_SPLIT_WIDTH;
        if collapsed && !self.collapsed {
            self.focus(Self::SPLIT);
        }
        self.collapsed = collapsed;
    }

    pub fn focus(&mut self, pane: usize) {
        if pane < self.list.len() {
            self.active = pane;
        }
    }

    /// The direction `Ctrl+PageUp` or `Ctrl+PageDown` steps the view beside the
    /// session while the shell has focus, its arrows moving through the input.
    pub fn split_step(&self, key: KeyEvent) -> Option<i8> {
        let direction = match key.code {
            KeyCode::PageUp => -1,
            KeyCode::PageDown => 1,
            _ => return None,
        };
        (key.ctrl() && !key.shift() && !key.alt() && self.split() && self.active != Self::SPLIT)
            .then_some(direction)
    }
}

pub fn ratio_for_width(width: u16, total: u16) -> u16 {
    (u32::from(width) * 1000).div_ceil(u32::from(total)) as u16
}

pub fn pane_layout(
    area: Rect,
    panes: usize,
    active: usize,
    ratio: u16,
) -> (Vec<Rect>, Option<Rect>) {
    if panes == 1 {
        return (vec![area], None);
    }
    if area.width < MIN_SPLIT_WIDTH {
        let mut areas = vec![Rect::ZERO; 2];
        areas[active] = area;
        return (areas, None);
    }
    let content = area.width - 1;
    let left = ((u32::from(content) * u32::from(ratio) / 1000) as u16)
        .clamp(MIN_PANE_WIDTH, content - MIN_PANE_WIDTH);
    (
        vec![
            Rect::new(area.x, area.y, left, area.height),
            Rect::new(area.x + left + 1, area.y, content - left, area.height),
        ],
        Some(Rect::new(area.x + left, area.y, 1, area.height)),
    )
}
