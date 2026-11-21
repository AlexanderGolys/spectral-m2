//! The application's session, its panes, and the terminal event loop.

mod documentation;
mod examples;
mod panes;
pub mod session;
mod tabs;

use crate::ui::glyphs::{Glyphs, Spelled};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::iter;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::clipboard::CopyToClipboard;
use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use crossterm::execute;
use lsp_types::ServerInfo;
use ratatui::layout::Alignment;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block as TuiBlock, Borders, Paragraph};
use ratatui::Frame;

use self::documentation::DocumentationBrowser;
use self::panes::{pane_layout, ratio_for_width, Pane, Panes, MIN_PANE_WIDTH};
use self::session::Session;
use self::tabs::{TabId, Tabs, Target};
use crate::backend::DocPage;
use crate::backend::{Announcement, M2Backend, Versions};
use crate::code::file as code_file;
use crate::code::location::SourceLink;
use crate::code::location::{SourcePoint, SourceRange};
use crate::code::semantic::query::{Answer, Place, QueryId, Question};
use crate::code::semantic::SemanticHighlighter;
use crate::code::semantic::SourceId;
use crate::env::ENV;
use crate::errors::Result;
use crate::ui::action::{Action, Opening};
use crate::ui::block::Block;
use crate::ui::cell::OutputBlock;
use crate::ui::cell::{response_blocks, SourceBlock, TextBlock};
use crate::ui::config::UI;
use crate::ui::controller::{
    CodePoint, CodeSource, CommittedCells, Editor, EditorUpdate, Lookup, Reaction, ReferenceEntry,
    ViewKind,
};
use crate::ui::floating::{Anchor, Floating, MarkupContent};
use crate::ui::fold::FoldRanges;
use crate::ui::graphics;
use crate::ui::key::{KeyEvent, Keyboard};
use crate::ui::relayout;
use crate::ui::render::markdown;
use crate::ui::render::markup;
use crate::ui::terminal::ReplTerminal;
use crate::ui::theme::groups::{scheme, HLGroup};
use crate::updates::{UpdateChecks, LANGUAGE_SERVER, SHELL, SPECTRAL};

/// How often the loop looks for results other threads posted (M2, m2-ls, update
/// checks) while nothing on screen moves; input wakes it at once regardless.
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(80);
const NO_LANGUAGE_SERVER: &str = "the language server is not running";
const NOTHING_KNOWN: &str = "nothing is known about this";
const CODE_NOT_IN_SESSION: &str = "that code is not part of the running session";
const NO_DEFINITION: &str = "no definition found";
const NO_NAME_AT_POINT: &str = "no name here to look up";
/// Rows above the view beside the session: its title, then a cell for each tab.
const PREVIEW_HEADER_ROWS: u16 = 2;
/// Columns from one cell of that header to the next: its icon and a gap.
const HEADER_CELL_STEP: u16 = 2;

/// What a [`Target`] shows, kept with its scroll and folds while it does not.
struct Page {
    pane: Pane,
    reference: Option<ReferenceEntry>,
}

/// Something in the header of the view beside the session that a click works.
#[derive(Clone, Copy, PartialEq)]
enum PanelControl {
    Tab(TabId),
    View(ViewKind),
    /// Step through history, back when negative.
    History(isize),
}
/// The M2 process behind the shell, with its transcript, draft, history, and syntax state.
struct Workspace {
    session: Session,
    editor: Editor,
    highlighter: SemanticHighlighter,
    // @##wtf is this type?
    analysis_state: Option<(u64, u64, Option<String>)>,
}

impl Workspace {
    fn new(mut backend: M2Backend) -> Self {
        let Announcement {
            versions,
            names,
            startup_output,
        } = backend.take_announcement();
        let session = Session::new(backend);
        let mut editor = Editor::new(welcome_message(&versions, None));
        editor.set_m2_version(versions.m2);
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
    documentation: Option<DocumentationBrowser>,
    /// The pages asked of the documentation interpreter and not yet answered.
    documentation_requests: HashMap<u64, Target>,
    tabs: Tabs,
    /// The pages set aside, each under what it shows.
    pages: HashMap<Target, Page>,
    /// What the view beside the session shows; nothing while it is hidden.
    shown: Option<Target>,
    panel_controls: Vec<(Rect, PanelControl)>,
    archived_code: String,
    archived_directory: Option<PathBuf>,
    /// Questions put to the language server, by what their answers are for.
    asked: HashMap<QueryId, Asked>,
    /// The cursor when the server was last asked about it.
    asked_at: Option<CursorAt>,
    /// The cursor as last seen, and since when it holds.
    cursor_seen: Option<(CursorAt, Instant)>,
    updates: UpdateChecks,
    /// When the session was last seen evaluating.
    last_busy: Option<Instant>,
}

/// Where a question about the input's cursor stands: the input's revision and
/// the cursor's byte offset in it. An answer about any other is out of date.
type CursorAt = (u64, usize);

/// What an answer from the language server is for.
#[derive(Clone, Debug)]
enum Asked {
    /// A lookup about `name`, the name at the point it was asked, when known.
    Lookup {
        lookup: Lookup,
        anchor: Anchor,
        name: Option<String>,
    },
    /// Where the name under the cursor occurs, asked with the cursor `at`.
    Occurrences(CursorAt),
    /// The call the cursor is in, asked with the cursor `at`.
    Signature(CursorAt),
    /// Formatting the input, which then read this.
    Format(String),
    /// The stretches of a file shown beside the session that fold.
    Folds(FoldRanges),
}

impl App {
    fn new(backend: M2Backend) -> Self {
        let mut app = Self {
            workspace: Workspace::new(backend),
            panes: Panes::new(),
            split_ratio: 500,
            pane_areas: vec![Rect::ZERO],
            divider: None,
            dragging_divider: false,
            keyboard: Keyboard::default(),
            documentation: None,
            documentation_requests: HashMap::new(),
            tabs: Tabs::default(),
            pages: HashMap::new(),
            shown: None,
            panel_controls: Vec::new(),
            archived_code: String::new(),
            archived_directory: None,
            asked: HashMap::new(),
            asked_at: None,
            cursor_seen: None,
            updates: UpdateChecks::new(),
            last_busy: None,
        };
        let _ = code_file::archive_previous_session();
        app.check_for_updates();
        app
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
                match self.panes.focused().view {
                    ViewKind::Shell => self.workspace.editor.paste(&text),
                    ViewKind::Notes => self.workspace.editor.paste_notes(&text),
                    _ => {}
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
        if self.panes.focused().view == ViewKind::Notes && !editor.picker_open() {
            if key.code == crossterm::event::KeyCode::Esc {
                self.panes.focus(0);
                return true;
            }
            let reaction = editor.handle_notes_key(key);
            return self.handle_reaction(reaction);
        }
        if let Some(reaction) = editor.handle_caret_key(key) {
            return self.handle_reaction(reaction);
        }
        if !editor.picker_open()
            && self.panes.split()
            && self.shown.is_some()
            && !key.alt()
            && !key.shift()
            && previewing
            && matches!(
                key.code,
                crossterm::event::KeyCode::Left | crossterm::event::KeyCode::Right
            )
        {
            let direction = if key.code == crossterm::event::KeyCode::Left {
                -1
            } else {
                1
            };
            self.tabs.step_along(direction);
            self.show_current();
            return true;
        }
        if !editor.picker_open() && self.panes.focused().view == ViewKind::Reference {
            match key.code {
                crossterm::event::KeyCode::Up => {
                    editor.scroll_reference(1);
                    return true;
                }
                crossterm::event::KeyCode::Down => {
                    editor.scroll_reference(-1);
                    return true;
                }
                _ => {}
            }
        }
        if let Some(direction) = self.panes.split_step(key).filter(|_| !editor.picker_open()) {
            self.tabs.step_along(direction);
            self.show_current();
            return true;
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
        if self.panes.focused().view == ViewKind::Notes {
            if action == Action::ToggleViewCursor {
                self.panes.focus(0);
                return true;
            }
            if let Some(reaction) = self.workspace.editor.notes_action(action) {
                return self.handle_reaction(reaction);
            }
        }
        match action {
            Action::CancelExecution => {
                // A cancel meant for a run that just ended must not clear or quit.
                let just_ended = self
                    .last_busy
                    .is_some_and(|at| at.elapsed() < UI::CANCEL_GRACE);
                if just_ended && !self.workspace.session.busy() {
                    return true;
                }
                if !self.workspace.session.busy() {
                    if self.workspace.editor.cancel_input() {
                        self.workspace.session.reset_incomplete();
                        return true;
                    }
                    return self.handle_action(Action::Quit);
                }
                let update = self.workspace.session.interrupt();
                self.workspace.editor.apply(update);
                return true;
            }
            Action::RunViewedFile => {
                return match self
                    .workspace
                    .editor
                    .reference_file()
                    .map(str::to_owned)
                    .filter(|_| {
                        self.panes.split()
                            && self.panes.list[Panes::SPLIT].view == ViewKind::Reference
                    }) {
                    Some(path) => self.handle_reaction(Reaction::Load(path)),
                    None => true,
                };
            }
            Action::CloseTab => return self.handle_reaction(Reaction::CloseTab),
            Action::HistoryBack => return self.handle_reaction(Reaction::History(-1)),
            Action::HistoryForward => return self.handle_reaction(Reaction::History(1)),
            _ => {}
        }
        if self.panes.focused().view == ViewKind::Reference {
            match action {
                Action::ScrollFeedUp => {
                    self.workspace.editor.page_reference(1);
                    return true;
                }
                Action::ScrollFeedDown => {
                    self.workspace.editor.page_reference(-1);
                    return true;
                }
                Action::ScrollBlockStart => {
                    self.workspace.editor.reference_to_block_start();
                    return true;
                }
                Action::ScrollBlockEnd => {
                    self.workspace.editor.reference_to_block_end();
                    return true;
                }
                _ => {}
            }
        }
        let Some(action) = self.panes.focused_mut().handle_action(action) else {
            return true;
        };
        let reaction = self.workspace.editor.handle_action(action);
        self.handle_reaction(reaction)
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        let pointer = Position::new(mouse.column, mouse.row);
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            if let Some(&(_, control)) = self
                .panel_controls
                .iter()
                .find(|(area, _)| area.contains(pointer))
            {
                match control {
                    PanelControl::Tab(tab) => self.tabs.select(tab),
                    PanelControl::View(view) => self.tabs.view(view),
                    PanelControl::History(delta) => {
                        self.tabs.step(delta);
                    }
                }
                self.show_current();
                self.panes.focus(Panes::SPLIT);
                return true;
            }
        }
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
        if matches!(mouse.kind, MouseEventKind::Down(_)) {
            if let Some(index) = self
                .pane_areas
                .iter()
                .position(|area| area.contains(pointer))
            {
                self.panes.focus(index);
            }
        }
        if self.panes.focused().view == ViewKind::Notes && !self.workspace.editor.picker_open() {
            let reaction = self.workspace.editor.notes_mouse(mouse);
            return self.handle_reaction(reaction);
        }
        let wheel_rows = self.workspace.editor.wheel_rows().unsigned_abs();
        let pane = self.panes.focused_mut();
        if pane.view == ViewKind::Reference {
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    let rows = self.workspace.editor.wheel_rows();
                    self.workspace.editor.scroll_reference(rows);
                    return true;
                }
                MouseEventKind::ScrollDown => {
                    let rows = self.workspace.editor.wheel_rows();
                    self.workspace.editor.scroll_reference(-rows);
                    return true;
                }
                _ => {}
            }
        }
        let reaction = if pane.view == ViewKind::Shell {
            self.workspace.editor.handle_mouse(mouse)
        } else {
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    pane.viewport.scroll = pane.viewport.scroll.saturating_sub(wheel_rows);
                    return true;
                }
                MouseEventKind::ScrollDown => {
                    pane.viewport.scroll += wheel_rows;
                    return true;
                }
                _ => self.workspace.editor.handle_preview_mouse(mouse),
            }
        };
        self.handle_reaction(reaction)
    }

    fn handle_reaction(&mut self, reaction: Reaction) -> bool {
        if matches!(reaction, Reaction::BrowseSessions) {
            self.keep_session();
        }
        let workspace = &mut self.workspace;
        match reaction {
            Reaction::Continue => true,
            Reaction::Follow(link, opening) => match Target::link(&link) {
                Some(target) => self.open(target, opening),
                None => workspace.editor.apply(EditorUpdate::Diagnostic(format!(
                    "cannot open source link: {link}"
                ))),
            },
            Reaction::RecentSession => {
                match code_file::previous_session().filter(|path| path.is_file()) {
                    Some(path) => self.open_source_file(SourceLink { path, range: None }),
                    None => workspace.editor.apply(EditorUpdate::Diagnostic(
                        "no previous session to view".into(),
                    )),
                }
            }
            Reaction::BrowseSessions => match code_file::archived_sessions() {
                Ok(paths) if paths.is_empty() => workspace
                    .editor
                    .apply(EditorUpdate::Diagnostic("no saved sessions".into())),
                Ok(paths) => {
                    workspace.editor.browse_sessions(paths);
                    true
                }
                Err(error) => workspace
                    .editor
                    .apply(EditorUpdate::Diagnostic(error.to_string())),
            },
            Reaction::Lookup(lookup, point) => {
                self.look_up(lookup, point);
                true
            }
            Reaction::Format(code) => {
                match workspace.highlighter.ask_apart(&code, Question::Format) {
                    Some(id) => {
                        self.asked.insert(id, Asked::Format(code));
                        true
                    }
                    None => workspace
                        .editor
                        .apply(EditorUpdate::Diagnostic(NO_LANGUAGE_SERVER.to_string())),
                }
            }
            Reaction::ViewFile(typed) => self.open_source_file(SourceLink {
                path: code_file::resolve(&typed),
                range: None,
            }),
            Reaction::SaveSelection(typed, source) => {
                let path = if typed.is_empty() {
                    code_file::archived_session_path()
                } else {
                    Some(code_file::resolve(&typed))
                };
                let update = match path {
                    Some(path) => match code_file::write(&path, &source) {
                        Ok(()) => EditorUpdate::Message(Box::new(TextBlock::system(format!(
                            "saved selected code to {}",
                            path.display()
                        )))),
                        Err(error) => EditorUpdate::Diagnostic(error.to_string()),
                    },
                    None => {
                        EditorUpdate::Diagnostic("no session storage directory available".into())
                    }
                };
                workspace.editor.apply(update)
            }
            Reaction::Submit => {
                let update = workspace
                    .session
                    .submit(workspace.editor.submission(), &mut workspace.highlighter);
                let running = workspace.editor.apply(update);
                self.follow_new_input();
                running
            }
            Reaction::Restart => {
                self.keep_session();
                let workspace = &mut self.workspace;
                workspace.editor.apply(workspace.session.restart())
            }
            Reaction::Copy(text) => {
                workspace.editor.note_copied(&text);
                let _ = execute!(io::stdout(), CopyToClipboard::to_clipboard_from(text));
                true
            }
            Reaction::ScrollView(rows) => {
                let pane = self.panes.focused_mut();
                match pane.view {
                    ViewKind::Shell => workspace.editor.scroll_feed(rows),
                    ViewKind::Reference => workspace.editor.scroll_reference(rows),
                    _ => pane.viewport.scroll = pane.viewport.scroll.saturating_add_signed(-rows),
                }
                true
            }
            Reaction::Open(view) => {
                match self.shown == Some(Target::View(view)) {
                    true => self.hide_panel(),
                    false => {
                        self.tabs.view(view);
                        self.show_current();
                    }
                }
                true
            }
            Reaction::CloseTab => {
                match self.tabs.close() {
                    true => self.show_current(),
                    false => self.hide_panel(),
                }
                true
            }
            Reaction::History(delta) => {
                if self.tabs.step(delta) {
                    self.show_current();
                }
                true
            }
            Reaction::FocusPanel => {
                match self.panes.active == Panes::SPLIT {
                    true => self.panes.focus(0),
                    false => {
                        self.show_panel();
                        self.panes.focus(Panes::SPLIT);
                    }
                }
                true
            }
            Reaction::TogglePanel => {
                match self.panes.split() {
                    true => self.hide_panel(),
                    false => self.show_panel(),
                }
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
            let block = code_file::without_comments(&block);
            if block.trim().is_empty() {
                continue;
            }
            let update = workspace
                .session
                .submit(block.trim(), &mut workspace.highlighter);
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
    fn keep_session(&mut self) {
        let code = self.workspace.session.code();
        if let Some(path) = code_file::previous_session().filter(|_| !code.is_empty()) {
            let _ = code_file::write(&path, &code);
            if code != self.archived_code || self.archived_directory.as_deref() != path.parent() {
                if let Some(archive) = code_file::archived_session_path() {
                    if code_file::write(&archive, &code).is_ok() {
                        self.archived_code = code;
                        self.archived_directory = path.parent().map(Path::to_path_buf);
                    }
                }
            }
        }
    }

    /// Set what shows aside under its target, leaving the view to be filled.
    fn set_aside(&mut self) {
        let Some(target) = self.shown.take() else {
            return;
        };
        let pane = std::mem::replace(
            &mut self.panes.list[Panes::SPLIT],
            Pane::new(ViewKind::Reference),
        );
        let reference =
            (pane.view == ViewKind::Reference).then(|| self.workspace.editor.take_reference());
        self.pages.insert(target, Page { pane, reference });
    }

    /// Hide the view beside the session, keeping its tabs and their pages.
    fn hide_panel(&mut self) {
        self.set_aside();
        self.panes.close_view();
    }

    /// Show the view beside the session where it was left, or on the first of
    /// the views always there when it never showed anything.
    fn show_panel(&mut self) {
        if self.tabs.current().is_none() {
            self.tabs.view(ViewKind::PREVIEWS[0]);
        }
        self.show_current();
    }

    /// Show what history's cursor is on, setting aside what showed before.
    fn show_current(&mut self) {
        self.tabs.settle();
        let Some(target) = self.tabs.current().map(|visit| visit.target.clone()) else {
            return;
        };
        if self.shown.as_ref() == Some(&target) {
            return;
        }
        self.set_aside();
        let page = match self.pages.remove(&target) {
            Some(page) => page,
            None => self.load(&target).unwrap_or_else(|unloaded| Page {
                pane: Pane::new(ViewKind::Reference),
                reference: Some(ReferenceEntry::new(
                    vec![Box::new(TextBlock::system(unloaded))],
                    None,
                )),
            }),
        };
        match self.panes.split() {
            true => self.panes.list[Panes::SPLIT] = page.pane,
            false => self.panes.list.push(page.pane),
        }
        if let Some(reference) = page.reference {
            self.workspace.editor.restore_reference(reference);
        }
        self.shown = Some(target);
        self.forget_far_pages();
    }

    /// Drop the pages far from the cursor in history and off the tab line, and
    /// the answers still awaited for them; a visit fetches one again.
    fn forget_far_pages(&mut self) {
        let kept = self.tabs.kept();
        self.pages
            .retain(|target, _| matches!(target, Target::View(_)) || kept.contains(target));
        let (pages, shown) = (&self.pages, &self.shown);
        self.documentation_requests
            .retain(|_, target| pages.contains_key(target) || shown.as_ref() == Some(target));
    }

    /// Open `target` beside the session as `opening` says, on the page already
    /// kept for it when there is one. A view not showing has no tab to stay on,
    /// so there the target shows at once.
    fn open(&mut self, target: Target, opening: Opening) -> bool {
        if self.shown.as_ref() != Some(&target) && !self.pages.contains_key(&target) {
            match self.load(&target) {
                Ok(page) => self.pages.insert(target.clone(), page),
                Err(unloaded) => {
                    return self
                        .workspace
                        .editor
                        .apply(EditorUpdate::Diagnostic(unloaded))
                }
            };
        }
        let opening = match (opening, self.panes.split()) {
            (Opening::Beside, false) => Opening::BesideAndJump,
            (opening, _) => opening,
        };
        match self.tabs.open(target, opening) {
            true => self.show_current(),
            false => self.forget_far_pages(),
        }
        true
    }

    /// The page `target` shows, made anew: a file read, documentation asked for,
    /// which arrives later, or what is already known laid out. Fails with why a
    /// file cannot be read.
    fn load(&mut self, target: &Target) -> std::result::Result<Page, String> {
        let reference = match target {
            Target::View(view) => {
                return Ok(Page {
                    pane: Pane::new(*view),
                    reference: None,
                })
            }
            Target::Documentation { topic, .. } => {
                let request = self
                    .documentation
                    .get_or_insert_with(DocumentationBrowser::new)
                    .open(topic.clone());
                self.documentation_requests.insert(request, target.clone());
                ReferenceEntry::new(
                    vec![Box::new(TextBlock::system(format!(
                        "opening documentation{}",
                        Glyphs::Ellipsis.current()
                    )))],
                    None,
                )
            }
            Target::Hover { text, .. } => {
                let page = DocPage {
                    html: markdown::html(text),
                    example_width: self.pane_areas.last().map_or(0, |area| area.width).into(),
                    type_names: Vec::new(),
                };
                let described = OutputBlock::value(text.clone(), None).documented(Some(page));
                ReferenceEntry::new(vec![Box::new(described.without_frame())], None)
            }
            Target::Source(source) => {
                let path = &source.path;
                let code = fs::read_to_string(path).map_err(|error| unreadable(path, &error))?;
                let page = markup::page(&format!("{}\n{code}", path.display()));
                self.workspace
                    .highlighter
                    .analyse_page(page, vec![code.clone()]);
                let focus = source.range.map(|range| range.bytes(&code));
                let folds = FoldRanges::default();
                if let Some(id) = self.workspace.highlighter.ask_apart(&code, Question::Folds) {
                    self.asked.insert(id, Asked::Folds(folds.clone()));
                }
                let heading = TextBlock::system(format!(
                    "{} {} F8 runs this file",
                    path.display(),
                    Glyphs::Dash.current()
                ));
                let source = SourceBlock::new(code, page, focus).with_folds(folds);
                ReferenceEntry::new(
                    vec![Box::new(heading), Box::new(source)],
                    Some(path.to_string_lossy().into_owned()),
                )
            }
        };
        Ok(Page {
            pane: Pane::new(ViewKind::Reference),
            reference: Some(reference),
        })
    }

    /// Put `lookup` to the language server about the name at `point`.
    fn look_up(&mut self, lookup: Lookup, point: CodePoint) {
        let workspace = &mut self.workspace;
        let source = match point.source {
            CodeSource::Input => workspace.highlighter.draft(),
            CodeSource::Cell { submission, index } => workspace.highlighter.cell(submission, index),
        };
        let name = source.and_then(|source| workspace.highlighter.name_at(source, point.offset));
        let question = match lookup {
            Lookup::Brief | Lookup::Description => Question::Hover,
            Lookup::Definition => Question::Definition,
            Lookup::Documentation => {
                match name {
                    Some(name) => self.open_documentation(format!("symbol {name}"), name),
                    None => {
                        workspace
                            .editor
                            .apply(EditorUpdate::Diagnostic(NO_NAME_AT_POINT.to_string()));
                    }
                }
                return;
            }
        };
        let asked =
            source.and_then(|source| workspace.highlighter.ask(source, point.offset, question));
        let unanswered = match (asked, workspace.highlighter.serving()) {
            (Some(id), _) => {
                let asked = Asked::Lookup {
                    lookup,
                    anchor: point.anchor,
                    name,
                };
                self.asked.insert(id, asked);
                return;
            }
            (None, false) => NO_LANGUAGE_SERVER,
            (None, true) => CODE_NOT_IN_SESSION,
        };
        workspace
            .editor
            .apply(EditorUpdate::Diagnostic(unanswered.to_string()));
    }

    /// Ask where the name under the input's cursor occurs once the cursor or the
    /// text has moved and come to rest, and after an edit which call the cursor
    /// is in.
    fn ask_about_cursor(&mut self) {
        let empty = self.workspace.editor.analysis_input().0.is_empty();
        let at = self.cursor_at();
        let (revision, _) = at;
        if self.cursor_seen.is_none_or(|(seen, _)| seen != at) {
            self.cursor_seen = Some((at, Instant::now()));
        }
        if self.asked_at == Some(at) || empty || self.cursor_settles_in().is_some() {
            return;
        }
        let workspace = &mut self.workspace;
        let edited = self.asked_at.is_none_or(|(asked, _)| asked != revision);
        self.asked_at = Some(at);
        let Some(source) = workspace.highlighter.draft() else {
            return;
        };
        let mut ask = |question: fn(usize) -> Question, asked: Asked| {
            if let Some(id) = workspace.highlighter.ask(source, at.1, question) {
                self.asked.insert(id, asked);
            }
        };
        ask(Question::Highlights, Asked::Occurrences(at));
        if edited && workspace.editor.signature_hints() {
            ask(Question::Signature, Asked::Signature(at));
        }
    }

    fn cursor_at(&self) -> CursorAt {
        let editor = &self.workspace.editor;
        (editor.analysis_input().2, editor.input_cursor())
    }

    /// Carry out what the language server answered to a question asked for `asked`.
    fn answered(&mut self, asked: Asked, answer: Answer) -> bool {
        let cursor = self.cursor_at();
        let editor = &mut self.workspace.editor;
        match (asked, answer) {
            (
                Asked::Lookup {
                    lookup: Lookup::Brief,
                    anchor,
                    ..
                },
                Answer::Hover(Some(text)),
            ) => editor.apply(EditorUpdate::Float(Floating::new(
                MarkupContent::from_markdown(markdown::lead(&text)),
                anchor,
            ))),
            (
                Asked::Lookup {
                    lookup: Lookup::Description,
                    name,
                    ..
                },
                Answer::Hover(Some(text)),
            ) => {
                let name = name.unwrap_or_else(|| ViewKind::Reference.label().to_string());
                self.open(Target::Hover { name, text }, Opening::BesideAndJump)
            }
            (Asked::Lookup { .. }, Answer::Hover(None)) => {
                editor.apply(EditorUpdate::Diagnostic(NOTHING_KNOWN.to_string()))
            }
            (Asked::Lookup { .. }, Answer::Definition(places)) => match places.into_iter().next() {
                Some(place) => self.go_to(place),
                None => editor.apply(EditorUpdate::Diagnostic(NO_DEFINITION.to_string())),
            },
            (Asked::Occurrences(at @ (revision, _)), Answer::Highlights(places))
                if at == cursor =>
            {
                let draft = self.workspace.highlighter.draft();
                let ranges = places
                    .into_iter()
                    .filter(|(source, _)| Some(*source) == draft)
                    .map(|(_, range)| range)
                    .collect();
                editor.apply(EditorUpdate::InputOccurrences { revision, ranges })
            }
            (Asked::Signature(at), Answer::Signature(signatures))
                if at == cursor && editor.signature_hints() =>
            {
                match signatures.is_empty() {
                    false => editor.apply(EditorUpdate::Float(Floating::new(
                        signatures,
                        Anchor::InputCursor,
                    ))),
                    true => true,
                }
            }
            (Asked::Folds(folds), Answer::Folds(ranges)) => {
                *folds.borrow_mut() = ranges;
                editor.apply(EditorUpdate::FoldsLearned)
            }
            (Asked::Format(was), Answer::Formatted(Some(now))) => {
                editor.apply(EditorUpdate::InputRewritten { was, now })
            }
            _ => true,
        }
    }

    fn go_to(&mut self, place: Place) -> bool {
        let editor = &mut self.workspace.editor;
        match place {
            Place::Source(SourceId::Draft(_), range) => {
                editor.apply(EditorUpdate::InputCursor(range.start))
            }
            Place::Source(
                SourceId::Cell {
                    submission, index, ..
                },
                _,
            ) => editor.apply(EditorUpdate::SelectCell { submission, index }),
            Place::Source(_, _) => true,
            Place::File { path, line, column } => self.open_source_file(SourceLink {
                path,
                range: Some(SourceRange {
                    start: SourcePoint {
                        line: line as usize + 1,
                        column: column as usize,
                    },
                    end: None,
                }),
            }),
        }
    }

    /// The title of what shows centred over its own row, the history controls
    /// at its start; beneath, a cell for each opened tab there is room for on
    /// the left and for each view always there on the right.
    fn draw_preview_header(&mut self, frame: &mut Frame<'_>) {
        self.panel_controls.clear();
        let Some(area) = self
            .pane_areas
            .get(Panes::SPLIT)
            .copied()
            .filter(|area| area.height >= PREVIEW_HEADER_ROWS)
        else {
            return;
        };
        let title = self
            .shown
            .as_ref()
            .map(|target| target.label().into_owned())
            .unwrap_or_default();
        frame.render_widget(
            Paragraph::new(title)
                .style(scheme().style(HLGroup::ViewLabelActive))
                .alignment(Alignment::Center),
            Rect::new(area.x, area.y, area.width, 1),
        );
        let fixed = ViewKind::PREVIEWS;
        let fixed_start = area
            .right()
            .saturating_sub(HEADER_CELL_STEP * fixed.len() as u16);
        // A cell is left empty between the opened tabs and the views always there.
        let room = fixed_start.saturating_sub(area.x + 2) / HEADER_CELL_STEP;
        let opened = self.tabs.window(room.into()).to_vec();
        let history = [
            (Glyphs::TabBack, PanelControl::History(-1)),
            (Glyphs::TabForward, PanelControl::History(1)),
        ];
        let rows = [
            (area.y, area.x + 1, history.to_vec()),
            (
                area.y + 1,
                area.x + 1,
                opened
                    .iter()
                    .map(|tab| (tab.target.glyph(), PanelControl::Tab(tab.id)))
                    .collect(),
            ),
            (
                area.y + 1,
                fixed_start,
                fixed
                    .map(|view| (Target::View(view).glyph(), PanelControl::View(view)))
                    .to_vec(),
            ),
        ];
        for (row, start, cells) in rows {
            for (index, (glyph, control)) in cells.into_iter().enumerate() {
                let column = start + HEADER_CELL_STEP * index as u16;
                self.draw_header_cell(frame, Position::new(column, row), glyph, control);
            }
        }
    }

    /// One cell of the header, highlighted when it stands for what shows.
    fn draw_header_cell(
        &mut self,
        frame: &mut Frame<'_>,
        at: Position,
        glyph: Glyphs,
        control: PanelControl,
    ) {
        let showing = self
            .tabs
            .current()
            .map(|visit| match (visit.tab, &visit.target) {
                (Some(tab), _) => PanelControl::Tab(tab),
                (None, target) => PanelControl::View(target.view()),
            });
        let style = match showing == Some(control) && self.shown.is_some() {
            true => scheme().style(HLGroup::ViewLabelActive),
            false => scheme().style(HLGroup::Muted),
        };
        if frame.area().contains(at) {
            frame.buffer_mut()[at]
                .set_symbol(glyph.current())
                .set_style(style);
            self.panel_controls
                .push((Rect::new(at.x, at.y, 1, 1), control));
        }
    }

    fn open_source_file(&mut self, source: SourceLink) -> bool {
        self.open(Target::Source(source), Opening::BesideAndJump)
    }

    /// Open the documentation `help topic` shows in a new tab titled `label`.
    fn open_documentation(&mut self, topic: String, label: String) {
        self.open(
            Target::Documentation { topic, label },
            Opening::BesideAndJump,
        );
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

    /// Views set aside keep their pictures, however far from showing.
    fn report_hidden_pictures(&self) {
        graphics::report_distances(
            self.pages
                .values()
                .filter_map(|page| page.reference.as_ref())
                .flat_map(ReferenceEntry::pictures)
                .map(|image| (image, graphics::HIDDEN_VIEW_DISTANCE)),
        );
    }

    fn draw(&mut self, frame: &mut Frame) {
        self.report_hidden_pictures();
        let active = self.panes.active;
        let split = self.panes.split();
        let workspace = &mut self.workspace;
        workspace.editor.set_processing(workspace.session.busy());
        workspace
            .editor
            .set_m2_process(workspace.session.m2_process());
        let screen = frame.area();
        let margin = Rect::new(
            screen.x,
            screen.y,
            UI::SCREEN_MARGIN.min(screen.width),
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
        if let Some(area) = self
            .pane_areas
            .get_mut(Panes::SPLIT)
            .filter(|area| !area.is_empty())
        {
            let border = TuiBlock::default()
                .style(self.workspace.editor.view_canvas())
                .borders(Borders::ALL)
                .border_set(Glyphs::SQUARE_BORDER.current())
                .border_style(scheme().view_divider());
            let inner = border.inner(*area);
            frame.render_widget(border, *area);
            *area = inner;
        }
        self.draw_preview_header(frame);
        let workspace = &mut self.workspace;
        if let Some(divider) = self.divider {
            workspace.editor.draw_divider(frame, divider);
        }
        for pane in (0..self.panes.list.len())
            .filter(|pane| *pane != active)
            .chain(iter::once(active))
        {
            let mut area = self.pane_areas[pane];
            if area == Rect::ZERO {
                continue;
            }
            if pane == Panes::SPLIT {
                area.y += PREVIEW_HEADER_ROWS;
                area.height = area.height.saturating_sub(PREVIEW_HEADER_ROWS);
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
        if !workspace.editor.italics() {
            frame
                .buffer_mut()
                .set_style(screen, Style::new().remove_modifier(Modifier::ITALIC));
        }
    }

    /// Bring every view to its newest content, as the shell follows its own feed.
    /// Advance whatever the shell animates while M2 works, and settle the typing
    /// prompt once typing pauses; whether anything changed.
    fn animate(&mut self) -> bool {
        let panes_settled = self.panes.list.iter().any(|pane| pane.viewport.width.due());
        let settled = self.workspace.editor.settle() | panes_settled;
        let animated = self.workspace.session.busy() && self.workspace.editor.animate();
        settled || animated
    }

    /// How much longer the cursor must rest before the server is asked about it,
    /// while it has moved since last asked.
    fn cursor_settles_in(&self) -> Option<Duration> {
        let (at, since) = self.cursor_seen?;
        (self.asked_at != Some(at))
            .then(|| UI::CURSOR_QUESTION_SETTLE.saturating_sub(since.elapsed()))
            .filter(|remaining| !remaining.is_zero())
    }

    /// The loop's own cadence: a frame's time while anything moves (an evaluation,
    /// work on a worker, pictures being encoded), else the idle interval.
    fn frame_interval(&self) -> Duration {
        let moving = self.workspace.session.busy() || relayout::busy() || graphics::encoding();
        match moving {
            true => UI::FRAME_INTERVAL,
            false => IDLE_POLL_INTERVAL,
        }
    }

    /// How long to wait for events before something on screen changes by itself.
    fn event_timeout(&self) -> Duration {
        self.panes
            .list
            .iter()
            .filter_map(|pane| pane.viewport.width.settles_in())
            .chain(self.workspace.editor.settles_in())
            .chain(self.cursor_settles_in())
            .min()
            .map_or(self.frame_interval(), |remaining| {
                remaining.min(self.frame_interval())
            })
    }

    fn follow_new_input(&mut self) {
        for pane in &mut self.panes.list {
            pane.viewport.follow_bottom();
        }
    }

    /// Look for newer versions of sp2, of the language server it runs and of the
    /// Spectral package its session loaded, wherever a look is due.
    fn check_for_updates(&mut self) {
        let server = self.workspace.editor.language_server_version();
        self.updates.poll(&[
            (SHELL, Some(env!("CARGO_PKG_VERSION"))),
            (LANGUAGE_SERVER, server),
            (SPECTRAL, Some(&self.workspace.session.versions().spectral)),
        ]);
    }

    fn poll(&mut self) -> (bool, bool) {
        if self.workspace.session.busy() {
            self.last_busy = Some(Instant::now());
        }
        let mut dirty = false;
        let mut running = true;
        let timeout = self.workspace.editor.example_timeout();
        self.workspace.session.set_example_timeout(timeout);
        let pages = self
            .documentation
            .as_ref()
            .map(|browser| browser.answered().collect::<Vec<_>>())
            .unwrap_or_default();
        for page in pages {
            let Some(target) = self.documentation_requests.remove(&page.request) else {
                continue;
            };
            let blocks = match page.response {
                Ok(response) => {
                    self.workspace
                        .session
                        .prepare_documentation(response.document.as_ref());
                    let (mut blocks, printed, value) = response_blocks(&response, "");
                    if let Some(printed) = printed {
                        blocks.push(Box::new(printed));
                    }
                    if let Some(value) = value {
                        blocks.push(Box::new(value.without_frame()));
                    }
                    blocks
                }
                Err(error) => {
                    vec![Box::new(TextBlock::system(error)) as Box<dyn Block>]
                }
            };
            if self.shown.as_ref() == Some(&target) {
                self.workspace.editor.open_reference(blocks, None);
            } else if let Some(kept) = self.pages.get_mut(&target) {
                kept.reference = Some(ReferenceEntry::new(blocks, None));
            }
            dirty = true;
        }
        for update in self.workspace.session.poll() {
            dirty = true;
            running &= self.workspace.editor.apply(update);
        }
        if self.workspace.session.take_restarted() {
            self.updates.forget();
        }
        self.check_for_updates();
        for update in self.updates.found() {
            dirty = true;
            running &=
                self.workspace
                    .editor
                    .apply(EditorUpdate::Message(Box::new(TextBlock::system(
                        update.to_string(),
                    ))));
        }
        if let Some(code) = self.workspace.session.take_after_restart() {
            dirty = true;
            running &= self.run_code(&code);
        }
        if dirty {
            self.follow_new_input();
        }
        let workspace = &mut self.workspace;
        let mut recolored = workspace.highlighter.configure(
            workspace.editor.language_server(),
            workspace.editor.semantic_highlighting(),
        );
        let semantic =
            workspace.editor.language_server() && workspace.editor.semantic_highlighting();
        if recolored && !semantic {
            markup::forget_colors();
            running &= workspace.editor.apply(EditorUpdate::ExamplesComputed);
        }
        for (page, sources) in workspace.session.take_pages_to_analyse() {
            workspace.highlighter.analyse_page(page, sources);
        }
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
        self.ask_about_cursor();
        let workspace = &mut self.workspace;
        let polled = workspace.highlighter.poll();
        recolored |= polled.recolored;
        if (recolored || polled.failure.is_some()) && !workspace.highlighter.serving() {
            // Nothing the server found or read stands once it is gone.
            running &= workspace
                .editor
                .apply(EditorUpdate::InputFindings(Vec::new()));
            running &= workspace
                .editor
                .apply(EditorUpdate::AnalysedNames(Vec::new()));
        }
        if let Some(findings) = polled.findings {
            let draft = workspace.highlighter.draft();
            let findings = findings
                .into_iter()
                .filter(|(source, _)| Some(*source) == draft)
                .map(|(_, finding)| finding)
                .collect();
            running &= workspace
                .editor
                .apply(EditorUpdate::InputFindings(findings));
            dirty = true;
        }
        for (id, answer) in polled.answers {
            if let Some(asked) = self.asked.remove(&id) {
                running &= self.answered(asked, answer);
                dirty = true;
            }
        }
        let workspace = &mut self.workspace;
        if let Some(bindings) = polled.bindings {
            running &= workspace.editor.apply(EditorUpdate::AnalysedNames(
                bindings.into_iter().map(|binding| binding.name).collect(),
            ));
        }
        if !polled.pages.is_empty() {
            polled.pages.into_iter().for_each(markup::remember_colors);
            running &= workspace.editor.apply(EditorUpdate::ExamplesComputed);
            dirty = true;
        }
        if recolored {
            workspace.editor.rehighlight(&mut workspace.highlighter);
            dirty = true;
        }
        if let Some(server) = polled.started {
            self.updates.forget();
            workspace
                .editor
                .set_language_server_info(Some(server.clone()));
            running &= workspace
                .editor
                .apply(EditorUpdate::Welcome(welcome_message(
                    workspace.session.versions(),
                    Some(&server),
                )));
            dirty = true;
        }
        if let Some(message) = polled.failure {
            workspace.editor.set_language_server_info(None);
            let note = match polled.restart_in {
                Some(delay) => format!(
                    "Language server stopped: {message}\nRestarting it in {} s.",
                    delay.as_secs()
                ),
                None => format!("Language server unavailable: {message}"),
            };
            running &= workspace
                .editor
                .apply(EditorUpdate::Message(Box::new(TextBlock::system(note))));
            dirty = true;
        }
        (running, dirty)
    }
}

/// The code in `path`, or the diagnostic saying why it cannot be read.
fn unreadable(path: &Path, error: &io::Error) -> String {
    format!("cannot read {}: {error}", path.display())
}

fn read_code(path: &Path) -> std::result::Result<String, EditorUpdate> {
    fs::read_to_string(path).map_err(|error| EditorUpdate::Diagnostic(unreadable(path, &error)))
}

/// The versions of the shell, its Macaulay2 and Spectral package, and the
/// language server once it runs.
fn welcome_message(versions: &Versions, server: Option<&ServerInfo>) -> String {
    let mut message = format!(
        "spectral-m2 v{}\nMacaulay2 v{}\nSpectral v{}",
        ENV.shell_version(),
        versions.m2,
        SPECTRAL.version(&versions.spectral)
    );
    if let Some(server) = server {
        message.push('\n');
        message.push_str(&server.name);
        if let Some(version) = &server.version {
            message.push_str(&format!(" v{}", LANGUAGE_SERVER.version(version)));
        }
    }
    message
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
        terminal.send_pictures()?;
        let events = terminal.next_events(app.event_timeout());
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
