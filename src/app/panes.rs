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

    /// Interpret unbound navigation as private input for the current preview:
    /// `Left` and `Right` step to the neighbouring view, `Up` and `Down` scroll.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.view == ViewKind::Shell {
            return false;
        }
        match key.code {
            KeyCode::Left => *self = Self::new(self.view.cycle(-1)),
            KeyCode::Right => *self = Self::new(self.view.cycle(1)),
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

    /// Step the view beside the session with `Ctrl+Left` or `Ctrl+Right` while the
    /// shell has focus, its plain arrows moving through the input; whether it did.
    pub fn step_split_view(&mut self, key: KeyEvent) -> bool {
        let stepping = matches!(key.code, KeyCode::Left | KeyCode::Right)
            && key.ctrl()
            && !key.shift()
            && !key.alt();
        stepping
            && self.split()
            && self.active != Self::SPLIT
            && self.list[Self::SPLIT].handle_key(key.code.into())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::key::KeyMod;

    #[test]
    fn previews_consume_scroll_actions_and_forward_quit() {
        let mut preview = Pane::new(ViewKind::SyntaxTree);
        let mut shell = Pane::new(ViewKind::Shell);
        preview.viewport.scroll = 0;

        assert!(preview.handle_action(Action::ScrollFeedDown).is_none());
        assert_eq!(preview.viewport.scroll, 10);
        assert!(preview.handle_action(Action::Quit) == Some(Action::Quit));
        assert!(shell.handle_action(Action::ScrollFeedDown) == Some(Action::ScrollFeedDown));
    }

    fn views(panes: &Panes) -> Vec<ViewKind> {
        panes.list.iter().map(|pane| pane.view).collect()
    }

    #[test]
    fn control_arrows_step_the_view_beside_a_focused_shell() {
        let mut panes = Panes::new();
        assert!(!panes.step_split_view(KeyCode::Right + KeyMod::Ctrl));
        panes.open_view(ViewKind::Code);
        assert!(panes.step_split_view(KeyCode::Right + KeyMod::Ctrl));
        assert_eq!(panes.list[Panes::SPLIT].view, ViewKind::Code.cycle(1));
        assert!(panes.step_split_view(KeyCode::Left + KeyMod::Ctrl));
        assert_eq!(panes.list[Panes::SPLIT].view, ViewKind::Code);
        assert!(!panes.step_split_view(KeyCode::Right.into()), "plain arrows edit");
        assert!(
            !panes.step_split_view(KeyCode::Right + KeyMod::Ctrl + KeyMod::Shift),
            "word selection stays the input's"
        );
        assert_eq!(panes.active, 0);
    }

    #[test]
    fn a_view_opens_beside_the_session_and_keeps_the_shell_focused() {
        let mut panes = Panes::new();
        panes.open_view(ViewKind::Code);

        assert_eq!(views(&panes), [ViewKind::Shell, ViewKind::Code]);
        assert_eq!(panes.active, 0);

        panes.open_view(ViewKind::SyntaxTree);
        assert_eq!(views(&panes), [ViewKind::Shell, ViewKind::SyntaxTree]);
    }

    #[test]
    fn opening_the_view_already_beside_the_session_closes_the_split() {
        let mut panes = Panes::new();
        panes.open_view(ViewKind::Output);
        panes.focus(Panes::SPLIT);
        panes.open_view(ViewKind::Output);

        assert_eq!(views(&panes), [ViewKind::Shell]);
        assert_eq!(panes.active, 0);
    }

    /// Closing acts on the view on screen, whichever pane has focus.
    #[test]
    fn closing_the_view_works_from_either_pane() {
        let mut panes = Panes::new();
        panes.open_view(ViewKind::Output);
        panes.close_view();
        assert_eq!(views(&panes), [ViewKind::Shell]);

        panes.open_view(ViewKind::Output);
        panes.focus(Panes::SPLIT);
        panes.close_view();
        assert_eq!(views(&panes), [ViewKind::Shell]);

        panes.close_view();
        assert_eq!(views(&panes), [ViewKind::Shell]);
    }

    #[test]
    fn a_split_too_narrow_to_show_side_by_side_shows_the_view() {
        let mut panes = Panes::new();
        panes.open_view(ViewKind::SyntaxTree);
        let narrow = Rect::new(0, 0, MIN_SPLIT_WIDTH - 1, 10);
        let layout = |panes: &mut Panes| {
            panes.fit(narrow.width);
            pane_layout(narrow, panes.list.len(), panes.active, 500).0
        };

        let areas = layout(&mut panes);
        assert_eq!(panes.active, Panes::SPLIT);
        assert_eq!(areas[Panes::SPLIT], narrow);
        panes.focus(0);
        assert_eq!(layout(&mut panes)[0], narrow, "the shell stays reachable");
    }

    #[test]
    fn split_collapses_below_two_minimum_panes_without_losing_either() {
        let area = Rect::new(0, 0, MIN_SPLIT_WIDTH - 1, 20);
        let (left, divider) = pane_layout(area, 2, 0, 500);
        let (right, _) = pane_layout(area, 2, 1, 500);

        assert_eq!(left, [area, Rect::ZERO]);
        assert_eq!(right, [Rect::ZERO, area]);
        assert!(divider.is_none());
    }

    #[test]
    fn wide_split_keeps_both_panes_at_least_forty_columns() {
        let area = Rect::new(4, 2, MIN_SPLIT_WIDTH, 20);
        let (panes, divider) = pane_layout(area, 2, 0, 100);

        assert_eq!(panes[0].width, MIN_PANE_WIDTH);
        assert_eq!(panes[1].width, MIN_PANE_WIDTH);
        assert_eq!(divider.unwrap().x, panes[0].right());
    }

    #[test]
    fn ratio_roundtrip_preserves_one_column_resizes() {
        for width in MIN_PANE_WIDTH..=60 {
            let ratio = ratio_for_width(width, 99);
            assert_eq!(99 * u32::from(ratio) / 1000, u32::from(width));
        }
    }
}
