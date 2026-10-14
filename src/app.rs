//! The application's session, its panes, and the terminal event loop.

pub mod session;

use std::io;
use std::time::Duration;

use crossterm::clipboard::CopyToClipboard;
use crossterm::event::{Event, KeyCode, MouseButton, MouseEvent, MouseEventKind};
use crossterm::execute;
use ratatui::layout::{Position, Rect};
use ratatui::Frame;

use self::session::Session;
use crate::backend::M2Backend;
use crate::code::highlight::SyntaxHighlighter;
use crate::errors::Result;
use crate::ui::action::Action;
use crate::ui::controller::{CommittedCells, Editor, OpenTarget, Reaction, ViewKind, Viewport};
use crate::ui::key::{KeyEvent, Keyboard};
use crate::ui::terminal::ReplTerminal;

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(80);
const MIN_PANE_WIDTH: u16 = 40;
const MIN_SPLIT_WIDTH: u16 = MIN_PANE_WIDTH * 2 + 1;

/// The M2 process behind the shell, with its transcript, draft, history, and syntax state.
struct Workspace {
    session: Session,
    editor: Editor,
    highlighter: SyntaxHighlighter,
}

impl Workspace {
    fn new(backend: M2Backend) -> Self {
        let session = Session::new(backend);
        let version = session.version().unwrap_or_else(|_| "unknown".to_string());
        Self {
            session,
            editor: Editor::new(welcome_message(&version)),
            highlighter: SyntaxHighlighter::new(),
        }
    }
}

/// One visible or width-collapsed window onto the session.
struct Pane {
    view: ViewKind,
    viewport: Viewport,
}

impl Pane {
    fn new(view: ViewKind) -> Self {
        Self {
            view,
            viewport: Viewport::at_bottom(),
        }
    }

    /// Consume a command owned by the current preview, or return it unchanged.
    fn handle_action(&mut self, action: Action) -> Option<Action> {
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

    /// Interpret unbound navigation as private input for the current preview.
    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.view == ViewKind::Shell {
            return false;
        }
        match key.code {
            KeyCode::Left if self.view == ViewKind::Output => self.viewport.cycle_output(-1),
            KeyCode::Right if self.view == ViewKind::Output => self.viewport.cycle_output(1),
            KeyCode::Up => self.viewport.scroll = self.viewport.scroll.saturating_sub(1),
            KeyCode::Down => self.viewport.scroll += 1,
            _ => return false,
        }
        true
    }
}

/// The session's panes and which one has focus.
///
/// The first pane is the session itself, its shell or a view shown in place. A
/// second pane exists only to show another view of the same session beside it;
/// an independent session is a new terminal pane away.
struct Panes {
    list: Vec<Pane>,
    active: usize,
    /// Whether the last layout had a split too narrow to show side by side.
    collapsed: bool,
}

impl Panes {
    /// Index of the pane that shows a view beside the session.
    const SPLIT: usize = 1;

    fn new() -> Self {
        Self {
            list: vec![Pane::new(ViewKind::Shell)],
            active: 0,
            collapsed: false,
        }
    }

    fn focused(&self) -> &Pane {
        &self.list[self.active]
    }

    fn focused_mut(&mut self) -> &mut Pane {
        &mut self.list[self.active]
    }

    fn split(&self) -> bool {
        self.list.len() > Self::SPLIT
    }

    /// Show `view` beside the session or in place of the focused pane; showing the
    /// view a pane already has takes it back down.
    fn open_view(&mut self, view: ViewKind, target: OpenTarget) {
        let pane = match target {
            OpenTarget::Split if !self.split() => {
                self.list.push(Pane::new(view));
                return;
            }
            OpenTarget::Split => Self::SPLIT,
            OpenTarget::InPlace => self.active,
        };
        let shown = if self.list[pane].view == view {
            ViewKind::Shell
        } else {
            view
        };
        if pane == Self::SPLIT && shown == ViewKind::Shell {
            self.close_split();
        } else {
            self.list[pane] = Pane::new(shown);
        }
    }

    /// Return the focused pane to the session: the split view closes, an in-place
    /// view gives way to the shell.
    fn leave_preview(&mut self) {
        if self.active == Self::SPLIT {
            self.close_split();
        } else {
            *self.focused_mut() = Pane::new(ViewKind::Shell);
        }
    }

    /// Close the view on screen, whichever pane has focus: the one beside the
    /// session, or else the one shown in place of it.
    fn close_view(&mut self) {
        if self.split() {
            self.close_split();
        } else if self.focused().view != ViewKind::Shell {
            self.leave_preview();
        }
    }

    fn close_split(&mut self) {
        self.list.truncate(Self::SPLIT);
        self.active = 0;
    }

    /// Fit the panes to `width` columns. A split too narrow to show side by side
    /// shows the view full width instead, from the moment it opens or the window
    /// narrows; `Ctrl+Left` still brings the shell back.
    fn fit(&mut self, width: u16) {
        let collapsed = self.split() && width < MIN_SPLIT_WIDTH;
        if collapsed && !self.collapsed {
            self.focus(Self::SPLIT);
        }
        self.collapsed = collapsed;
    }

    fn focus(&mut self, pane: usize) {
        if pane < self.list.len() {
            self.active = pane;
        }
    }
}

/// The session, its panes, and their responsive layout state.
struct App {
    workspace: Workspace,
    panes: Panes,
    split_ratio: u16,
    pane_areas: Vec<Rect>,
    divider: Option<Rect>,
    dragging_divider: bool,
    keyboard: Keyboard,
}

impl App {
    fn new(backend: M2Backend) -> Self {
        Self {
            workspace: Workspace::new(backend),
            panes: Panes::new(),
            split_ratio: 500,
            pane_areas: vec![Rect::ZERO],
            divider: None,
            dragging_divider: false,
            keyboard: Keyboard::default(),
        }
    }

    fn handle_event(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) => {
                let Some(key) = self.keyboard.event(key) else {
                    return true;
                };
                self.handle_key(key)
            }
            Event::Paste(text) => {
                if self.panes.focused().view == ViewKind::Shell {
                    self.workspace.editor.paste(&text);
                }
                true
            }
            Event::Mouse(mouse) => self.handle_mouse(mouse),
            _ => true,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        let split = self.panes.split();
        let previewing = self.panes.focused().view != ViewKind::Shell;
        let editor = &mut self.workspace.editor;
        editor.set_context(split, previewing, true);
        if let Some(action) = editor.resolve_action(key) {
            return self.handle_action(action);
        }
        if !editor.picker_open() && self.panes.focused_mut().handle_key(key) {
            return true;
        }
        let reaction = if previewing {
            editor.handle_preview_key(key)
        } else {
            editor.handle_input(key)
        };
        self.handle_reaction(reaction)
    }

    fn handle_action(&mut self, action: Action) -> bool {
        match action {
            Action::FocusPaneLeft => {
                self.panes.focus(0);
                return true;
            }
            Action::FocusPaneRight => {
                self.panes.focus(Panes::SPLIT);
                return true;
            }
            Action::ResizePaneLeft => {
                self.resize_split(-1);
                return true;
            }
            Action::ResizePaneRight => {
                self.resize_split(1);
                return true;
            }
            Action::ClosePane => {
                self.panes.close_view();
                return true;
            }
            Action::Quit if self.panes.focused().view != ViewKind::Shell => {
                self.panes.leave_preview();
                return true;
            }
            _ => {}
        }
        let Some(action) = self.panes.focused_mut().handle_action(action) else {
            return true;
        };
        let reaction = self.workspace.editor.handle_action(action);
        self.handle_reaction(reaction)
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        let pointer = Position::new(mouse.column, mouse.row);
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && self.divider.is_some_and(|area| area.contains(pointer))
        {
            self.dragging_divider = true;
            return true;
        }
        if self.dragging_divider {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) => self.drag_split(mouse.column),
                MouseEventKind::Up(MouseButton::Left) => self.dragging_divider = false,
                _ => {}
            }
            return true;
        }
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            if let Some(index) = self
                .pane_areas
                .iter()
                .position(|area| area.contains(pointer))
            {
                self.panes.focus(index);
            }
        }
        let pane = self.panes.focused_mut();
        let reaction = if pane.view == ViewKind::Shell {
            self.workspace.editor.handle_mouse(mouse)
        } else {
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    pane.viewport.scroll = pane.viewport.scroll.saturating_sub(1);
                    return true;
                }
                MouseEventKind::ScrollDown => {
                    pane.viewport.scroll += 1;
                    return true;
                }
                _ => self.workspace.editor.handle_preview_mouse(mouse),
            }
        };
        self.handle_reaction(reaction)
    }

    fn handle_reaction(&mut self, reaction: Reaction) -> bool {
        let workspace = &mut self.workspace;
        match reaction {
            Reaction::Continue => true,
            Reaction::Submit => {
                let update = workspace
                    .session
                    .submit(workspace.editor.submission(), &mut workspace.highlighter);
                let running = workspace.editor.apply(update);
                self.follow_new_input();
                running
            }
            Reaction::ResumeIncomplete => {
                if workspace.editor.take_incomplete().is_some() {
                    workspace.session.reset_incomplete();
                }
                true
            }
            Reaction::Post(source) => {
                let update = workspace.session.submit(source, &mut workspace.highlighter);
                let running = workspace.editor.post(update);
                self.follow_new_input();
                running
            }
            Reaction::Restart => workspace.editor.apply(workspace.session.restart()),
            Reaction::Copy(text) => {
                let _ = execute!(io::stdout(), CopyToClipboard::to_clipboard_from(text));
                true
            }
            Reaction::Open(view, target) => {
                self.panes.open_view(view, target);
                true
            }
            Reaction::ClosePane => {
                self.panes.close_view();
                true
            }
            Reaction::Quit => false,
        }
    }

    fn resize_split(&mut self, delta: i8) {
        let Some(divider) = self.divider else {
            return;
        };
        let total = self.pane_areas.iter().map(|area| area.width).sum::<u16>();
        if total < MIN_PANE_WIDTH * 2 {
            return;
        }
        let width = divider.x.saturating_sub(self.pane_areas[0].x);
        let width = if delta > 0 {
            width.saturating_add(1)
        } else {
            width.saturating_sub(1)
        }
        .clamp(MIN_PANE_WIDTH, total - MIN_PANE_WIDTH);
        self.split_ratio = ratio_for_width(width, total);
    }

    fn drag_split(&mut self, column: u16) {
        let total = self.pane_areas.iter().map(|area| area.width).sum::<u16>();
        if total < MIN_PANE_WIDTH * 2 {
            return;
        }
        let width = column
            .saturating_sub(self.pane_areas[0].x)
            .clamp(MIN_PANE_WIDTH, total - MIN_PANE_WIDTH);
        self.split_ratio = ratio_for_width(width, total);
    }

    fn layout(&mut self, area: Rect) {
        self.panes.fit(area.width);
        (self.pane_areas, self.divider) = pane_layout(
            area,
            self.panes.list.len(),
            self.panes.active,
            self.split_ratio,
        );
    }

    fn draw(&mut self, frame: &mut Frame) {
        let active = self.panes.active;
        let split = self.panes.split();
        let workspace = &mut self.workspace;
        workspace.editor.set_processing(workspace.session.busy());
        let mut area = frame.area();
        let shared_status = (split && workspace.editor.statusline_shown()).then(|| {
            area.height = area.height.saturating_sub(1);
            Rect::new(area.x, area.bottom(), area.width, 1)
        });
        self.layout(area);
        let workspace = &mut self.workspace;
        if let Some(divider) = self.divider {
            workspace.editor.draw_divider(frame, divider);
        }
        for pane in (0..self.panes.list.len())
            .filter(|pane| *pane != active)
            .chain(std::iter::once(active))
        {
            let area = self.pane_areas[pane];
            if area == Rect::ZERO {
                continue;
            }
            let view = self.panes.list[pane].view;
            workspace
                .editor
                .set_context(split, view != ViewKind::Shell, pane == active);
            match view {
                ViewKind::Shell => workspace.editor.draw(
                    frame,
                    area,
                    &mut workspace.highlighter,
                    workspace.session.in_debugger(),
                ),
                view => workspace.editor.draw_preview(
                    frame,
                    area,
                    &mut workspace.highlighter,
                    CommittedCells {
                        cells: workspace.session.committed(),
                        revision: workspace.session.committed_revision(),
                    },
                    view,
                    &mut self.panes.list[pane].viewport,
                ),
            }
        }
        if let Some(status) = shared_status {
            let beside = &self.panes.list[Panes::SPLIT];
            workspace.editor.draw_shared_status(
                frame,
                status,
                beside.view,
                &beside.viewport,
                active == Panes::SPLIT,
            );
        }
    }

    /// Bring every view to its newest content, as the shell follows its own feed.
    fn follow_new_input(&mut self) {
        for pane in &mut self.panes.list {
            pane.viewport.follow_bottom();
        }
    }

    fn poll(&mut self) -> (bool, bool) {
        let mut dirty = false;
        let mut running = true;
        for update in self.workspace.session.poll() {
            dirty = true;
            running &= self.workspace.editor.apply(update);
        }
        if dirty {
            self.follow_new_input();
        }
        (running, dirty)
    }
}

fn ratio_for_width(width: u16, total: u16) -> u16 {
    (u32::from(width) * 1000).div_ceil(u32::from(total)) as u16
}

fn pane_layout(area: Rect, panes: usize, active: usize, ratio: u16) -> (Vec<Rect>, Option<Rect>) {
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

fn welcome_message(m2_version: &str) -> String {
    format!(
        "spectral-m2 v{}\nMacaulay2 v{m2_version}",
        env!("CARGO_PKG_VERSION")
    )
}

/// Run the terminal event loop until any workspace requests exit.
pub fn run(backend: M2Backend) -> Result<()> {
    let mut terminal = ReplTerminal::enter()?;
    let mut app = App::new(backend);
    let mut dirty = true;
    loop {
        if dirty {
            terminal.draw(|frame| app.draw(frame))?;
            dirty = false;
        }
        let (running, updated) = app.poll();
        if !running {
            break;
        }
        if updated {
            dirty = true;
            continue;
        }
        let events = terminal.next_events(EVENT_POLL_INTERVAL)?;
        if events.is_empty() {
            continue;
        }
        let mut running = true;
        for event in events {
            running &= app.handle_event(event);
        }
        if !running {
            break;
        }
        dirty = true;
    }
    terminal.leave()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_view_opens_beside_the_session_and_keeps_the_shell_focused() {
        let mut panes = Panes::new();
        panes.open_view(ViewKind::Code, OpenTarget::Split);

        assert_eq!(views(&panes), [ViewKind::Shell, ViewKind::Code]);
        assert_eq!(panes.active, 0);

        panes.open_view(ViewKind::SyntaxTree, OpenTarget::Split);
        assert_eq!(views(&panes), [ViewKind::Shell, ViewKind::SyntaxTree]);
    }

    #[test]
    fn opening_the_view_already_beside_the_session_closes_the_split() {
        let mut panes = Panes::new();
        panes.open_view(ViewKind::Output, OpenTarget::Split);
        panes.focus(Panes::SPLIT);
        panes.open_view(ViewKind::Output, OpenTarget::Split);

        assert_eq!(views(&panes), [ViewKind::Shell]);
        assert_eq!(panes.active, 0);
    }

    /// Closing acts on the view on screen, whichever pane has focus.
    #[test]
    fn closing_the_view_works_from_either_pane() {
        let mut panes = Panes::new();
        panes.open_view(ViewKind::Output, OpenTarget::Split);
        panes.close_view();
        assert_eq!(views(&panes), [ViewKind::Shell]);

        panes.open_view(ViewKind::Output, OpenTarget::Split);
        panes.focus(Panes::SPLIT);
        panes.close_view();
        assert_eq!(views(&panes), [ViewKind::Shell]);

        panes.open_view(ViewKind::Code, OpenTarget::InPlace);
        panes.close_view();
        assert_eq!(views(&panes), [ViewKind::Shell]);
        panes.close_view();
        assert_eq!(views(&panes), [ViewKind::Shell]);
    }

    #[test]
    fn leaving_any_view_never_loses_the_shell() {
        let mut panes = Panes::new();
        panes.open_view(ViewKind::Code, OpenTarget::InPlace);
        panes.open_view(ViewKind::Output, OpenTarget::Split);
        assert_eq!(views(&panes), [ViewKind::Code, ViewKind::Output]);

        panes.focus(Panes::SPLIT);
        panes.leave_preview();
        assert_eq!(views(&panes), [ViewKind::Code]);
        panes.leave_preview();
        assert_eq!(views(&panes), [ViewKind::Shell]);
    }

    #[test]
    fn a_view_in_place_of_the_split_toggles_without_a_second_shell() {
        let mut panes = Panes::new();
        panes.open_view(ViewKind::Code, OpenTarget::Split);
        panes.focus(Panes::SPLIT);
        panes.open_view(ViewKind::Code, OpenTarget::InPlace);

        assert_eq!(views(&panes), [ViewKind::Shell]);
    }

    #[test]
    fn a_split_too_narrow_to_show_side_by_side_shows_the_view() {
        let mut panes = Panes::new();
        panes.open_view(ViewKind::SyntaxTree, OpenTarget::Split);
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
