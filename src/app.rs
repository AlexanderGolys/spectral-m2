//! The application's session, its panes, and the terminal event loop.

mod documentation;
mod examples;
mod panes;
mod previews;
pub mod session;

use std::collections::HashMap;
use std::fs;
use std::io;
use std::iter;
use std::path::Path;
use std::time::Duration;

use crossterm::clipboard::CopyToClipboard;
use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use crossterm::execute;
use lsp_types::ServerInfo;
use ratatui::layout::{Constraint, Layout};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use self::documentation::DocumentationBrowser;
use self::panes::{pane_layout, ratio_for_width, Pane, Panes, MIN_PANE_WIDTH};
use self::previews::{PreviewTab, PreviewTabs, RECENT_PREVIEWS};
use self::session::Session;
use crate::backend::M2Backend;
use crate::code::file as code_file;
use crate::code::location::SourceLink;
use crate::code::semantic::SemanticHighlighter;
use crate::errors::Result;
use crate::ui::action::Action;
use crate::ui::block::Block;
use crate::ui::cell::{response_blocks, SourceBlock, TextBlock};
use crate::ui::config::UI;
use crate::ui::controller::{
    CommittedCells, Editor, EditorUpdate, Reaction, ReferenceEntry, ViewKind,
};
use crate::ui::graphics;
use crate::ui::key::{KeyEvent, Keyboard};
use crate::ui::render::markup;
use crate::ui::terminal::ReplTerminal;
use crate::ui::theme::groups::{scheme, HLGroup};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(80);
/// How often to look for pictures the worker has finished encoding while any remain.
const PICTURE_POLL_INTERVAL: Duration = Duration::from_millis(10);

struct PreviewSnapshot {
    pane: Pane,
    reference: Option<ReferenceEntry>,
}

#[derive(Clone, Copy)]
enum PreviewControl {
    Select(PreviewTab),
    Back,
    Bookmark,
}
/// The M2 process behind the shell, with its transcript, draft, history, and syntax state.
struct Workspace {
    session: Session,
    editor: Editor,
    highlighter: SemanticHighlighter,
    // @##wtf is this type?
    analysis_state: Option<(u64, u64, Option<String>)>,
    /// The Macaulay2 version the welcome message names.
    // @##redundant
    m2_version: String,
}

impl Workspace {
    fn new(mut backend: M2Backend) -> Self {
        let names = backend.take_names();
        let startup_output = backend.take_startup_output();
        let session = Session::new(backend);
        let version = session.version().unwrap_or_else(|_| "unknown".to_string());
        let mut editor = Editor::new(welcome_message(&version, None));
        editor.set_m2_version(version.clone());
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
            m2_version: version,
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
    documentation_requests: HashMap<u64, PreviewTab>,
    previews: PreviewTabs<PreviewSnapshot>,
    preview_controls: Vec<(Rect, PreviewControl)>,
    archived_code: String,
}

impl App {
    fn new(backend: M2Backend) -> Self {
        let _ = code_file::archive_previous_session();
        Self {
            workspace: Workspace::new(backend),
            panes: Panes::new(),
            split_ratio: 500,
            pane_areas: vec![Rect::ZERO],
            divider: None,
            dragging_divider: false,
            keyboard: Keyboard::default(),
            documentation: None,
            documentation_requests: HashMap::new(),
            previews: PreviewTabs::default(),
            preview_controls: Vec::new(),
            archived_code: String::new(),
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
        if !editor.picker_open()
            && self.panes.split()
            && self.previews.active.is_some()
            && !key.alt()
            && !key.shift()
            && (previewing || key.ctrl())
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
            if let Some(tab) = self.previews.neighbour(direction) {
                self.select_preview(tab);
            }
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
        if !editor.picker_open()
            && (self.panes.step_split_view(key) || self.panes.focused_mut().handle_key(key))
        {
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
            Action::ClosePane => {
                return self.handle_reaction(Reaction::ClosePane);
            }
            Action::ReferenceBack => {
                return self.handle_reaction(Reaction::ReferenceBack);
            }
            _ => {}
        }
        if self.panes.focused().view == ViewKind::Reference {
            match action {
                Action::ScrollFeedUp => {
                    self.workspace.editor.scroll_reference(10);
                    return true;
                }
                Action::ScrollFeedDown => {
                    self.workspace.editor.scroll_reference(-10);
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
            if let Some((_, control)) = self
                .preview_controls
                .iter()
                .find(|(area, _)| area.contains(pointer))
            {
                match *control {
                    PreviewControl::Select(tab) => self.select_preview(tab),
                    PreviewControl::Back => {
                        self.handle_reaction(Reaction::ReferenceBack);
                    }
                    PreviewControl::Bookmark => {
                        if !matches!(self.previews.active, Some(PreviewTab::Recent(_))) {
                            return true;
                        }
                        let previous = self.previews.active;
                        self.documentation_requests
                            .retain(|_, tab| *tab != PreviewTab::Bookmark);
                        self.previews.pin();
                        for tab in self.documentation_requests.values_mut() {
                            if Some(*tab) == previous {
                                *tab = PreviewTab::Bookmark;
                            }
                        }
                    }
                }
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
        let pane = self.panes.focused_mut();
        if pane.view == ViewKind::Reference {
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.workspace.editor.scroll_reference(1);
                    return true;
                }
                MouseEventKind::ScrollDown => {
                    self.workspace.editor.scroll_reference(-1);
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
        if matches!(reaction, Reaction::BrowseSessions) {
            self.keep_session();
        }
        let workspace = &mut self.workspace;
        match reaction {
            Reaction::Continue => true,
            Reaction::Documentation(target) => {
                self.open_documentation(target);
                true
            }
            Reaction::DocumentationInFeed(target) => {
                let code = if target.starts_with("file:") {
                    match SourceLink::parse(&target) {
                        Some(source) => source.command(),
                        None => return true,
                    }
                } else {
                    format!("help {target}")
                };
                self.run_code(&code)
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
                let _ = execute!(io::stdout(), CopyToClipboard::to_clipboard_from(text));
                true
            }
            Reaction::ScrollView(rows) => {
                let pane = self.panes.focused_mut();
                match pane.view {
                    ViewKind::Shell => workspace.editor.scroll_feed(rows),
                    _ => {
                        pane.viewport.scroll = pane
                            .viewport
                            .scroll
                            .saturating_add_signed(-isize::from(rows))
                    }
                }
                true
            }
            Reaction::Open(view) => {
                self.open_preview(view, view.label().to_string());
                true
            }
            Reaction::ClosePane => {
                let current = self.take_preview();
                self.previews.save(current);
                self.panes.close_view();
                true
            }
            Reaction::ReferenceBack => {
                if let Some(previous) = self.previews.previous() {
                    self.select_preview(previous);
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
            if code != self.archived_code {
                if let Some(archive) = code_file::archived_session_path() {
                    if code_file::write(&archive, &code).is_ok() {
                        self.archived_code = code;
                    }
                }
            }
        }
    }

    fn take_preview(&mut self) -> Option<PreviewSnapshot> {
        if !self.panes.split() {
            return None;
        }
        let pane = std::mem::replace(
            &mut self.panes.list[Panes::SPLIT],
            Pane::new(ViewKind::Reference),
        );
        let reference =
            (pane.view == ViewKind::Reference).then(|| self.workspace.editor.take_reference());
        Some(PreviewSnapshot { pane, reference })
    }

    fn open_preview(&mut self, view: ViewKind, label: String) {
        let current = self.take_preview();
        self.previews.open(current, label);
        let replaced = self.previews.active;
        self.documentation_requests
            .retain(|_, tab| Some(*tab) != replaced);
        if self.panes.split() {
            self.panes.list[Panes::SPLIT] = Pane::new(view);
        } else {
            self.panes.open_view(view);
        }
    }

    fn select_preview(&mut self, tab: PreviewTab) {
        if self.previews.label(tab).is_none()
            || (self.panes.split() && self.previews.active == Some(tab))
        {
            return;
        }
        let current = self.take_preview();
        if let Some(snapshot) = self.previews.select(tab, current) {
            if self.panes.split() {
                self.panes.list[Panes::SPLIT] = snapshot.pane;
            } else {
                self.panes.list.push(snapshot.pane);
            }
            if let Some(reference) = snapshot.reference {
                self.workspace.editor.restore_reference(reference);
            }
        }
    }

    fn draw_preview_header(&mut self, frame: &mut Frame<'_>) {
        self.preview_controls.clear();
        let Some(area) = self
            .pane_areas
            .get(Panes::SPLIT)
            .copied()
            .filter(|area| !area.is_empty())
        else {
            return;
        };
        let header = Rect::new(area.x, area.y, area.width, 1);
        let theme = scheme();
        let mut controls = vec![
            (PreviewControl::Back, " ‹ ".to_string()),
            (PreviewControl::Bookmark, " ☆ ".to_string()),
        ];
        controls.extend((0..RECENT_PREVIEWS).map(|index| {
            let tab = PreviewTab::Recent(index);
            (
                PreviewControl::Select(tab),
                format!(
                    " {}:{} ",
                    index + 1,
                    self.previews.label(tab).unwrap_or("—")
                ),
            )
        }));
        controls.push((
            PreviewControl::Select(PreviewTab::Bookmark),
            format!(
                " ★:{} ",
                self.previews.label(PreviewTab::Bookmark).unwrap_or("—")
            ),
        ));
        let constraints = controls.iter().enumerate().map(|(index, _)| {
            if index < 2 {
                Constraint::Length(3)
            } else {
                Constraint::Ratio(1, (RECENT_PREVIEWS + 1) as u32)
            }
        });
        let cells = Layout::horizontal(constraints).split(header);
        for ((control, label), area) in controls.into_iter().zip(cells.iter().copied()) {
            let selected =
                matches!(control, PreviewControl::Select(tab) if self.previews.active == Some(tab));
            let style = if selected {
                theme
                    .style(HLGroup::MarkupLink)
                    .add_modifier(Modifier::REVERSED)
            } else {
                theme.style(HLGroup::Muted)
            };
            frame.render_widget(Paragraph::new(label).style(style), area);
            self.preview_controls.push((area, control));
        }
    }

    fn open_source_file(&mut self, source: SourceLink) -> bool {
        let path = &source.path;
        match read_code(&path) {
            Ok(code) => {
                let page = markup::page(&format!("{}\n{code}", path.display()));
                self.workspace
                    .highlighter
                    .analyse_page(page, vec![code.clone()]);
                let focus = source.range.map(|range| range.bytes(&code));
                let block = SourceBlock::new(code, page, focus);
                let blocks: Vec<Box<dyn Block>> = vec![
                    Box::new(TextBlock::system(format!(
                        "{} — F8 runs this file",
                        path.display()
                    ))),
                    Box::new(block),
                ];
                self.open_preview(
                    ViewKind::Reference,
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                );
                self.workspace
                    .editor
                    .open_reference(blocks, Some(path.to_string_lossy().into_owned()));
                true
            }
            Err(update) => self.workspace.editor.apply(update),
        }
    }

    fn open_documentation(&mut self, target: String) {
        if target.starts_with("file:") {
            match SourceLink::parse(&target) {
                Some(source) => {
                    self.open_source_file(source);
                }
                None => {
                    self.workspace
                        .editor
                        .apply(EditorUpdate::Diagnostic(format!(
                            "cannot open source link: {target}"
                        )));
                }
            }
            return;
        }
        self.open_preview(ViewKind::Reference, target.trim_matches('"').to_string());
        self.workspace.editor.open_reference(
            vec![Box::new(TextBlock::system("opening documentation…"))],
            None,
        );
        let request = self
            .documentation
            .get_or_insert_with(DocumentationBrowser::new)
            .open(target);
        self.documentation_requests.insert(
            request,
            self.previews
                .active
                .expect("opening a preview selects a tab"),
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
            self.previews
                .saved()
                .filter_map(|snapshot| snapshot.reference.as_ref())
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
                area.y += 1;
                area.height = area.height.saturating_sub(1);
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
        let pages = self
            .documentation
            .as_ref()
            .map(|browser| browser.answered().collect::<Vec<_>>())
            .unwrap_or_default();
        for page in pages {
            let Some(tab) = self.documentation_requests.remove(&page.request) else {
                continue;
            };
            let blocks = match page.response {
                Ok(response) => {
                    self.workspace
                        .session
                        .prepare_documentation(response.value.as_ref());
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
            if self.panes.split() && self.previews.active == Some(tab) {
                self.workspace.editor.open_reference(blocks, None);
            } else if let Some(snapshot) = self.previews.saved_mut(tab) {
                snapshot.reference = Some(ReferenceEntry::new(blocks, None));
            }
            dirty = true;
        }
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
        let polled = workspace.highlighter.poll();
        recolored |= polled.recolored;
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
            workspace
                .editor
                .set_language_server_info(Some(server.clone()));
            running &= workspace
                .editor
                .apply(EditorUpdate::Welcome(welcome_message(
                    &workspace.m2_version,
                    Some(&server),
                )));
            dirty = true;
        }
        if let Some(message) = polled.failure {
            workspace.editor.set_language_server_info(None);
            running &= workspace
                .editor
                .apply(EditorUpdate::Message(Box::new(TextBlock::system(format!(
                    "Language server unavailable: {message}"
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

/// The versions of the shell, its Macaulay2, and the language server once it runs.
fn welcome_message(m2_version: &str, server: Option<&ServerInfo>) -> String {
    let mut message = format!(
        "spectral-m2 v{}\nMacaulay2 v{m2_version}",
        env!("CARGO_PKG_VERSION")
    );
    if let Some(server) = server {
        message.push('\n');
        message.push_str(&server.name);
        if let Some(version) = &server.version {
            message.push_str(&format!(" v{version}"));
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
        let timeout = match graphics::encoding() {
            true => app.event_timeout().min(PICTURE_POLL_INTERVAL),
            false => app.event_timeout(),
        };
        let events = terminal.next_events(timeout)?;
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
mod tests;
