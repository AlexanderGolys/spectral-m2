//! Application workspaces, panes, and the terminal event loop.

pub mod session;

use std::io;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;

use crossterm::clipboard::CopyToClipboard;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use ratatui::layout::{Position, Rect};
use ratatui::Frame;

use self::session::Session;
use crate::backend::M2Backend;
use crate::code::highlight::SyntaxHighlighter;
use crate::errors::Result;
use crate::ui::controller::{Editor, EditorAction, EditorConfig, OpenTarget, ViewKind, Viewport};
use crate::ui::terminal::ReplTerminal;

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(80);
const MIN_PANE_WIDTH: u16 = 40;
const MIN_SPLIT_WIDTH: u16 = MIN_PANE_WIDTH * 2 + 1;

fn leaves_preview(key: KeyEvent, view: ViewKind) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('q' | 'c'))
        && view != ViewKind::Shell
}

/// One independent M2 process, transcript, draft, history, and syntax state.
struct Workspace {
    session: Session,
    editor: Editor,
    highlighter: SyntaxHighlighter,
}

impl Workspace {
    fn new(backend: M2Backend, config: Option<&EditorConfig>) -> Self {
        let session = Session::new(backend);
        let version = session.version().unwrap_or_else(|_| "unknown".to_string());
        let mut editor = Editor::new(welcome_message(&version)).with_macaulay_version(version);
        if let Some(config) = config {
            editor.set_config(config);
        }
        Self {
            session,
            editor,
            highlighter: SyntaxHighlighter::new(),
        }
    }
}

/// One visible or width-collapsed window onto a workspace.
struct Pane {
    id: u64,
    workspace: usize,
    view: ViewKind,
    viewport: Viewport,
}

impl Pane {
    fn new(id: u64, workspace: usize, view: ViewKind) -> Self {
        Self {
            id,
            workspace,
            view,
            viewport: Viewport::for_view(view),
        }
    }
}

/// An M2 process being prepared before its source pane is changed.
struct PendingNew {
    pane: u64,
    target: OpenTarget,
    result: Receiver<std::result::Result<M2Backend, String>>,
}

/// The two-pane application model and its responsive layout state.
struct App {
    workspaces: Vec<Option<Workspace>>,
    panes: Vec<Pane>,
    active: usize,
    next_pane_id: u64,
    split_ratio: u16,
    pane_areas: Vec<Rect>,
    divider: Option<Rect>,
    dragging_divider: bool,
    pending_new: Option<PendingNew>,
}

impl App {
    fn new(backend: M2Backend) -> Self {
        Self {
            workspaces: vec![Some(Workspace::new(backend, None)), None],
            panes: vec![Pane::new(0, 0, ViewKind::Shell)],
            active: 0,
            next_pane_id: 1,
            split_ratio: 500,
            pane_areas: vec![Rect::ZERO],
            divider: None,
            dragging_divider: false,
            pending_new: None,
        }
    }

    fn handle_event(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) => self.handle_key(key),
            Event::Paste(text) => {
                if self.panes[self.active].view == ViewKind::Shell {
                    self.active_workspace_mut().editor.paste(&text);
                }
                true
            }
            Event::Mouse(mouse) => self.handle_mouse(mouse),
            _ => true,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if leaves_preview(key, self.panes[self.active].view) {
            self.leave_preview();
            return true;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Left if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.resize_split(-1);
                    return true;
                }
                KeyCode::Right if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.resize_split(1);
                    return true;
                }
                KeyCode::Left => {
                    self.focus(0);
                    return true;
                }
                KeyCode::Right => {
                    self.focus(1);
                    return true;
                }
                _ => {}
            }
        }

        let pane = self.active;
        let workspace = self.panes[pane].workspace;
        let split = self.panes.len() == 2;
        let previewing = self.panes[pane].view != ViewKind::Shell;
        self.workspaces[workspace]
            .as_mut()
            .unwrap()
            .editor
            .set_context(split, previewing, true);
        if previewing
            && !self.workspaces[workspace]
                .as_ref()
                .unwrap()
                .editor
                .picker_open()
        {
            match key.code {
                KeyCode::Left if self.panes[pane].view == ViewKind::Output => {
                    self.panes[pane].viewport.cycle_output(-1);
                    return true;
                }
                KeyCode::Right if self.panes[pane].view == ViewKind::Output => {
                    self.panes[pane].viewport.cycle_output(1);
                    return true;
                }
                KeyCode::Up => {
                    self.panes[pane].viewport.scroll =
                        self.panes[pane].viewport.scroll.saturating_sub(1);
                    return true;
                }
                KeyCode::Down => {
                    self.panes[pane].viewport.scroll += 1;
                    return true;
                }
                KeyCode::PageUp => {
                    self.panes[pane].viewport.scroll =
                        self.panes[pane].viewport.scroll.saturating_sub(10);
                    return true;
                }
                KeyCode::PageDown => {
                    self.panes[pane].viewport.scroll += 10;
                    return true;
                }
                _ => {}
            }
        }
        let action = if previewing {
            self.workspaces[workspace]
                .as_mut()
                .unwrap()
                .editor
                .handle_preview_key(key)
        } else {
            self.workspaces[workspace]
                .as_mut()
                .unwrap()
                .editor
                .handle_key(key)
        };
        self.handle_editor_action(workspace, action)
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
                self.active = index;
            }
        }
        let pane = self.active;
        let workspace = self.panes[pane].workspace;
        if self.panes[pane].view != ViewKind::Shell {
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.panes[pane].viewport.scroll =
                        self.panes[pane].viewport.scroll.saturating_sub(1)
                }
                MouseEventKind::ScrollDown => self.panes[pane].viewport.scroll += 1,
                _ => {
                    let action = self.workspaces[workspace]
                        .as_mut()
                        .unwrap()
                        .editor
                        .handle_preview_mouse(mouse);
                    return self.handle_editor_action(workspace, action);
                }
            }
            return true;
        }
        let action = self.workspaces[workspace]
            .as_mut()
            .unwrap()
            .editor
            .handle_mouse(mouse);
        self.handle_editor_action(workspace, action)
    }

    fn handle_editor_action(&mut self, workspace: usize, action: EditorAction) -> bool {
        let keep_running = match action {
            EditorAction::Continue => true,
            EditorAction::Submit => {
                let workspace = self.workspaces[workspace].as_mut().unwrap();
                let update = workspace
                    .session
                    .submit(workspace.editor.submission(), &mut workspace.highlighter);
                workspace.editor.apply(update)
            }
            EditorAction::Post(source) => {
                let workspace = self.workspaces[workspace].as_mut().unwrap();
                let update = workspace.session.submit(source, &mut workspace.highlighter);
                workspace.editor.post(update)
            }
            EditorAction::Restart => {
                let workspace = self.workspaces[workspace].as_mut().unwrap();
                workspace.editor.apply(workspace.session.restart())
            }
            EditorAction::Copy(text) => {
                let _ = execute!(io::stdout(), CopyToClipboard::to_clipboard_from(text));
                true
            }
            EditorAction::Open(view, target) => {
                self.open_view(view, target);
                true
            }
            EditorAction::New(target) => {
                self.spawn_workspace(target);
                true
            }
            EditorAction::ClosePane => {
                self.close_pane();
                true
            }
            EditorAction::Quit => false,
        };
        if keep_running && self.workspaces[workspace].is_some() {
            self.sync_config(workspace);
        }
        keep_running
    }

    fn open_view(&mut self, view: ViewKind, target: OpenTarget) {
        match target {
            OpenTarget::Current => {
                let pane = &mut self.panes[self.active];
                pane.view = if pane.view == view {
                    ViewKind::Shell
                } else {
                    view
                };
                pane.viewport = Viewport::for_view(pane.view);
            }
            OpenTarget::Split if self.panes.len() == 1 => {
                let workspace = self.panes[self.active].workspace;
                self.panes
                    .push(Pane::new(self.next_pane_id, workspace, view));
                self.next_pane_id += 1;
                self.active = 1;
            }
            OpenTarget::Split => {}
        }
    }

    fn spawn_workspace(&mut self, target: OpenTarget) {
        if self.pending_new.is_some() {
            self.active_workspace_mut()
                .editor
                .append_error("a new workspace is already starting");
            return;
        }
        let pane = self.panes[self.active].id;
        let (sender, result) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(M2Backend::spawn().map_err(|error| error.to_string()));
        });
        self.pending_new = Some(PendingNew {
            pane,
            target,
            result,
        });
    }

    fn poll_new_workspace(&mut self) {
        let result = match self
            .pending_new
            .as_ref()
            .map(|pending| pending.result.try_recv())
        {
            Some(Ok(result)) => result,
            Some(Err(TryRecvError::Disconnected)) => {
                Err("workspace startup task stopped".to_string())
            }
            Some(Err(TryRecvError::Empty)) | None => return,
        };
        let pending = self.pending_new.take().unwrap();
        let Some(pane) = self.panes.iter().position(|pane| pane.id == pending.pane) else {
            return;
        };
        match result {
            Ok(backend) => self.install_workspace(pane, pending.target, backend),
            Err(error) => self.workspaces[self.panes[pane].workspace]
                .as_mut()
                .unwrap()
                .editor
                .append_error(format!("could not open workspace: {error}")),
        }
    }

    fn install_workspace(&mut self, pane: usize, target: OpenTarget, backend: M2Backend) {
        if target == OpenTarget::Split && self.panes.len() == 2 {
            return;
        }
        let old = self.panes[pane].workspace;
        let references = self
            .panes
            .iter()
            .filter(|candidate| candidate.workspace == old)
            .count();
        let slot = if target == OpenTarget::Current && references == 1 {
            old
        } else if let Some(slot) = self.workspaces.iter().position(Option::is_none) {
            slot
        } else {
            self.workspaces[old]
                .as_mut()
                .unwrap()
                .editor
                .append_error("only two independent workspaces are supported");
            return;
        };
        let config = self.workspaces[old].as_ref().unwrap().editor.config();
        self.workspaces[slot] = Some(Workspace::new(backend, Some(&config)));
        match target {
            OpenTarget::Current => {
                self.panes[pane] = Pane::new(self.panes[pane].id, slot, ViewKind::Shell);
                self.active = pane;
            }
            OpenTarget::Split => {
                self.panes
                    .push(Pane::new(self.next_pane_id, slot, ViewKind::Shell));
                self.next_pane_id += 1;
                self.active = 1;
            }
        }
        self.cleanup_workspaces();
    }

    fn leave_preview(&mut self) {
        if self.panes.len() == 2 {
            self.close_pane();
        } else {
            self.panes[0].view = ViewKind::Shell;
            self.panes[0].viewport = Viewport::default();
        }
    }

    fn close_pane(&mut self) {
        if self.panes.len() == 1 {
            return;
        }
        self.panes.remove(self.active);
        self.active = self.active.min(self.panes.len() - 1);
        self.cleanup_workspaces();
    }

    fn cleanup_workspaces(&mut self) {
        for (index, workspace) in self.workspaces.iter_mut().enumerate() {
            if !self.panes.iter().any(|pane| pane.workspace == index) {
                *workspace = None;
            }
        }
    }

    fn sync_config(&mut self, source: usize) {
        let config = self.workspaces[source].as_ref().unwrap().editor.config();
        for (index, workspace) in self.workspaces.iter_mut().enumerate() {
            if index != source {
                if let Some(workspace) = workspace {
                    workspace.editor.set_config(&config);
                }
            }
        }
    }

    fn focus(&mut self, pane: usize) {
        if pane < self.panes.len() {
            self.active = pane;
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
        (self.pane_areas, self.divider) =
            pane_layout(area, self.panes.len(), self.active, self.split_ratio);
    }

    fn draw(&mut self, frame: &mut Frame) {
        self.layout(frame.area());
        if let Some(divider) = self.divider {
            for y in divider.y..divider.bottom() {
                frame.buffer_mut()[(divider.x, y)].set_symbol("│");
            }
        }
        let active = self.active;
        for pane in (0..self.panes.len())
            .filter(|pane| *pane != active)
            .chain(std::iter::once(active))
        {
            let area = self.pane_areas[pane];
            if area == Rect::ZERO {
                continue;
            }
            let split = self.panes.len() == 2;
            let focused = pane == self.active;
            let view = self.panes[pane].view;
            let workspace_index = self.panes[pane].workspace;
            let workspace = self.workspaces[workspace_index].as_mut().unwrap();
            workspace
                .editor
                .set_context(split, view != ViewKind::Shell, focused);
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
                    workspace.session.committed(),
                    view,
                    &mut self.panes[pane].viewport,
                ),
            }
        }
    }

    fn poll(&mut self) -> bool {
        self.poll_new_workspace();
        self.workspaces.iter_mut().flatten().all(|workspace| {
            workspace
                .session
                .poll()
                .into_iter()
                .all(|update| workspace.editor.apply(update))
        })
    }

    fn active_workspace_mut(&mut self) -> &mut Workspace {
        let workspace = self.panes[self.active].workspace;
        self.workspaces[workspace].as_mut().unwrap()
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
    loop {
        terminal.draw(|frame| app.draw(frame))?;
        if !app.poll() {
            break;
        }
        let Some(event) = terminal.next_event(EVENT_POLL_INTERVAL)? else {
            continue;
        };
        if !app.handle_event(event) {
            break;
        }
    }
    terminal.leave()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_c_and_q_leave_previews_but_not_the_shell() {
        for code in ['c', 'q'] {
            let key = KeyEvent::new(KeyCode::Char(code), KeyModifiers::CONTROL);
            assert!(leaves_preview(key, ViewKind::SyntaxTree));
            assert!(!leaves_preview(key, ViewKind::Shell));
        }
        assert!(!leaves_preview(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
            ViewKind::SyntaxTree,
        ));
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
