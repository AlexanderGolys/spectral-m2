//! The application's session, its panes, and the terminal event loop.

mod examples;
mod panes;
pub mod session;

use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

use crossterm::clipboard::CopyToClipboard;
use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use crossterm::execute;
use ratatui::layout::{Position, Rect};
use ratatui::Frame;

use self::panes::{pane_layout, ratio_for_width, Panes, MIN_PANE_WIDTH};
use self::session::Session;
use crate::backend::M2Backend;
use crate::code::file as code_file;
use crate::code::semantic::SemanticHighlighter;
use crate::errors::Result;
use crate::ui::action::Action;
use crate::ui::cell::TextBlock;
use crate::ui::config::UI;
use crate::ui::controller::{CommittedCells, Editor, EditorUpdate, Reaction, ViewKind};
use crate::ui::graphics;
use crate::ui::key::{KeyEvent, Keyboard};
use crate::ui::terminal::ReplTerminal;

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(80);
/// The M2 process behind the shell, with its transcript, draft, history, and syntax state.
struct Workspace {
    session: Session,
    editor: Editor,
    highlighter: SemanticHighlighter,
    analysis_state: Option<(u64, u64, Option<String>)>,
}

impl Workspace {
    fn new(mut backend: M2Backend) -> Self {
        let names = backend.take_names();
        let startup_output = backend.take_startup_output();
        let session = Session::new(backend);
        let version = session.version().unwrap_or_else(|_| "unknown".to_string());
        let mut editor = Editor::new(welcome_message(&version));
        editor.apply(EditorUpdate::KnownNames(names));
        if !startup_output.is_empty() {
            editor.apply(EditorUpdate::Message(Box::new(TextBlock::printed(
                startup_output,
            ))));
        }
        Self {
            session,
            editor,
            highlighter: SemanticHighlighter::new(),
            analysis_state: None,
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
                let key = self.keyboard.event(key);
                self.workspace.editor.hold_control(self.keyboard.ctrl());
                let Some(key) = key else {
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
            Action::CancelExecution => {
                if !self.workspace.session.busy() {
                    return self.handle_action(Action::Quit);
                }
                let update = self.workspace.session.interrupt();
                self.workspace.editor.apply(update);
                return true;
            }
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
                let update = workspace
                    .session
                    .submit(&source, &mut workspace.highlighter);
                let running = workspace.editor.post(update);
                self.follow_new_input();
                running
            }
            Reaction::Restart => {
                self.keep_session();
                let workspace = &mut self.workspace;
                workspace.editor.apply(workspace.session.restart())
            }
            Reaction::Copy(text) => {
                let _ = execute!(io::stdout(), CopyToClipboard::to_clipboard_from(text));
                true
            }
            Reaction::Open(view) => {
                self.panes.open_view(view);
                true
            }
            Reaction::ClosePane => {
                self.panes.close_view();
                true
            }
            Reaction::Save(Some(typed)) => self.save(&code_file::resolve(&typed)),
            Reaction::Save(None) => {
                match code_file::previous_session().filter(|_| !workspace.session.code().is_empty())
                {
                    Some(path) => self.save(&path),
                    None => workspace.editor.apply(EditorUpdate::Diagnostic(
                        "the session has no code to keep".to_string(),
                    )),
                }
            }
            Reaction::Load(typed) => match read_code(&code_file::resolve(&typed)) {
                Ok(code) => {
                    self.keep_session();
                    let workspace = &mut self.workspace;
                    workspace.editor.apply(workspace.session.restart_with(code))
                }
                Err(update) => workspace.editor.apply(update),
            },
            Reaction::Resume => {
                let code = code_file::previous_session()
                    .filter(|path| path.exists())
                    .ok_or_else(|| {
                        EditorUpdate::Diagnostic("no previous session to resume".to_string())
                    })
                    .and_then(|path| read_code(&path));
                match code {
                    Ok(code) => self.run_code(&code),
                    Err(update) => workspace.editor.apply(update),
                }
            }
            Reaction::Quit => false,
        }
    }

    /// Evaluate `code` as if each submission of it had been typed into the shell
    /// in turn, so the feed reads as it did when the code was first entered.
    fn run_code(&mut self, code: &str) -> bool {
        let workspace = &mut self.workspace;
        let mut running = true;
        for block in Session::blocks(code) {
            let update = workspace.session.submit(&block, &mut workspace.highlighter);
            running &= workspace.editor.post(update);
        }
        self.follow_new_input();
        running
    }

    /// Write the session's code to `path`, reporting where it went.
    fn save(&mut self, path: &Path) -> bool {
        let workspace = &mut self.workspace;
        let update = match code_file::write(path, &workspace.session.code()) {
            Ok(()) => EditorUpdate::Message(Box::new(TextBlock::system(format!(
                "saved the session's code to {}",
                path.display()
            )))),
            Err(error) => {
                EditorUpdate::Diagnostic(format!("cannot save {}: {error}", path.display()))
            }
        };
        workspace.editor.apply(update)
    }

    /// Keep the session's code for the next session to resume, unless it has
    /// none, so an empty session never replaces the one before it.
    fn keep_session(&self) {
        let code = self.workspace.session.code();
        if let Some(path) = code_file::previous_session().filter(|_| !code.is_empty()) {
            let _ = code_file::write(&path, &code);
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
        let screen = frame.area();
        let margin = Rect::new(
            screen.x,
            screen.y,
            UI.screen_margin.min(screen.width),
            screen.height,
        );
        let mut area = Rect::new(
            margin.right(),
            screen.y,
            screen.width - margin.width,
            screen.height,
        );
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
            workspace
                .editor
                .draw_shared_status(frame, status, beside.view, active == Panes::SPLIT);
        }
        workspace.editor.draw_margin(frame, margin);
    }

    /// Bring every view to its newest content, as the shell follows its own feed.
    /// Advance whatever the shell animates while M2 works, and settle the typing
    /// prompt once typing pauses; whether anything changed.
    fn animate(&mut self) -> bool {
        let settled = self.workspace.editor.settle_typing();
        let animated = self.workspace.session.busy() && self.workspace.editor.animate();
        settled || animated
    }

    /// How long to wait for events before something on screen changes by itself.
    fn event_timeout(&self) -> Duration {
        self.workspace
            .editor
            .typing_remaining()
            .map_or(EVENT_POLL_INTERVAL, |remaining| {
                remaining.min(EVENT_POLL_INTERVAL)
            })
    }

    fn follow_new_input(&mut self) {
        for pane in &mut self.panes.list {
            pane.viewport.follow_bottom();
        }
    }

    fn poll(&mut self) -> (bool, bool) {
        let mut dirty = false;
        let mut running = true;
        let timeout = self.workspace.editor.example_timeout();
        self.workspace.session.set_example_timeout(timeout);
        for update in self.workspace.session.poll() {
            dirty = true;
            running &= self.workspace.editor.apply(update);
        }
        if let Some(code) = self.workspace.session.take_after_restart() {
            dirty = true;
            running &= self.run_code(&code);
        }
        if dirty {
            self.follow_new_input();
        }
        let workspace = &mut self.workspace;
        let enabled = workspace.editor.semantic_highlighting();
        let mut recolored = workspace.highlighter.set_enabled(enabled);
        let (draft, pending, revision) = workspace.editor.analysis_input();
        let state = (
            workspace.session.source_revision(),
            revision,
            pending.map(str::to_string),
        );
        if workspace.analysis_state.as_ref() != Some(&state) {
            workspace
                .highlighter
                .update(workspace.session.semantic_document(pending, draft));
            workspace.analysis_state = Some(state);
        }
        let (updated, failure) = workspace.highlighter.poll();
        recolored |= updated;
        if recolored {
            workspace.editor.rehighlight(&mut workspace.highlighter);
            dirty = true;
        }
        if let Some(message) = failure {
            running &= workspace
                .editor
                .apply(EditorUpdate::Message(Box::new(TextBlock::system(format!(
                    "Semantic highlighting unavailable: {message}"
                )))));
            dirty = true;
        }
        (running, dirty)
    }
}

/// The code in `path`, or the diagnostic saying why it cannot be read.
fn read_code(path: &Path) -> std::result::Result<String, EditorUpdate> {
    fs::read_to_string(path).map_err(|error| {
        EditorUpdate::Diagnostic(format!("cannot read {}: {error}", path.display()))
    })
}

fn welcome_message(m2_version: &str) -> String {
    format!(
        "spectral-m2 v{}\nMacaulay2 v{m2_version}",
        env!("CARGO_PKG_VERSION")
    )
}

/// Run the terminal event loop until any workspace requests exit, first running
/// `startup` as if it had been typed.
pub fn run(backend: M2Backend, startup: Option<String>) -> Result<()> {
    let mut terminal = ReplTerminal::enter()?;
    let mut app = App::new(backend);
    if let Some(code) = startup {
        app.run_code(&code);
    }
    let mut dirty = true;
    loop {
        if dirty {
            terminal.draw(|frame| app.draw(frame))?;
        }
        let (running, updated) = app.poll();
        if !running {
            break;
        }
        if updated {
            dirty = true;
            continue;
        }
        let events = terminal.next_events(app.event_timeout())?;
        if events.is_empty() {
            dirty = app.animate();
            continue;
        }
        let mut running = true;
        for event in events {
            if matches!(event, Event::FocusGained) {
                graphics::resend_all();
            }
            running &= app.handle_event(event);
        }
        if !running {
            break;
        }
        dirty = true;
    }
    app.keep_session();
    terminal.leave()
}

#[cfg(test)]
mod tests {
    #[test]
    fn first_session_displays_initialization_output() {
        use ratatui::{backend::TestBackend, Terminal};
        let directory =
            std::env::temp_dir().join(format!("spectral-startup-output-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let init = directory.join("init.m2");
        std::fs::write(&init, "print \"startup output probe\"\n").unwrap();
        let backend = crate::backend::M2Backend::spawn_with_init_file(Some(init)).unwrap();
        let mut app = super::App::new(backend);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let screen = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        std::fs::remove_dir_all(directory).unwrap();
        assert!(screen.contains("startup output probe"), "{screen}");
    }

    #[test]
    fn a_submitted_comment_is_visible_on_the_next_frame() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut app = super::App::new(crate::backend::M2Backend::spawn().expect("spawn M2"));
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        for character in "--a".chars() {
            app.handle_key(crossterm::event::KeyCode::Char(character).into());
        }
        assert!(app.handle_key(crossterm::event::KeyCode::Enter.into()));
        app.poll();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let screen = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(screen.contains("--a"), "{screen}");
    }

    use super::*;
    use std::time::Instant;

    #[test]
    fn ctrl_c_interrupts_while_busy_and_quits_when_idle() {
        let mut app = App::new(M2Backend::spawn().expect("spawn M2"));
        let update = app
            .workspace
            .session
            .submit("while true do null", &mut app.workspace.highlighter);
        app.workspace.editor.apply(update);
        std::thread::sleep(Duration::from_millis(100));
        assert!(app.handle_key(crate::key!(ctrl + c)));
        assert!(app.handle_key(crate::key!(ctrl + c)));
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.workspace.session.busy() && Instant::now() < deadline {
            for update in app.workspace.session.poll() {
                app.workspace.editor.apply(update);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!app.workspace.session.busy());
        assert!(!app.handle_key(crate::key!(ctrl + c)));
    }
}
