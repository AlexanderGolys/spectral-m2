//! Interactive editor: submitted feed, live input and history, navigation,
//! scrolling, and rendering.

pub mod commands;
mod completion;
mod highlight_debug;
pub(crate) mod picker;
mod screen;
mod view;

use std::ops::Range;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use lsp_types::ServerInfo;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as TuiBlock, Clear, Paragraph, Widget};
use ratatui::Frame;

use self::highlight_debug::{DebugEntry, DebugSection};
use self::picker::{Picker, PickerController, PickerMode};
use self::screen::{ScreenCapture, ScreenSelection};
pub use self::view::{CommittedCells, ReferenceEntry, ViewKind, Viewport};
use self::{
    commands::Command,
    completion::{Names, Request},
};
use super::cell::{CellState, EvaluationBlock, IncompleteBlock, OutputBlock, TextBlock};
use super::config::UI;
use super::debug::{resident_bytes, FrameStats};
use super::element::EventListener;
use super::feed::Feed;
use super::key::{KeyEvent, KeyMod};
use super::render::maths::MathDisplay;
use super::render::selection::Inclusions;
use super::render::RenderContext;
use super::statusline::{
    LanguageServerState, StatusDiagnostics, StatusLine, StatusLineContext, StatusLocation,
    StatusRuntime,
};
use super::{
    action::Action,
    block::{Block, Element, HLSpan, Link, Reformatted, Rethemed, TextSpan, Ticked},
};
use crate::code::file::abbreviate_home;
use crate::code::highlight::Highlighter;
use crate::code::semantic::query::Finding;
use crate::code::semantic::SourceId;
use crate::code::semantic::SEMANTIC_GROUPS;
use crate::env::ENV;
use crate::input::{InputCode, InputContext};
use crate::ui::floating::{Anchor, Floating};
use crate::ui::settings::selection::CopyFormat;
use crate::ui::settings::statusline::{StatusComponent, StatusLinePosition, StatusSegment};
use crate::ui::settings::{tabs as settings_tabs, *};
use crate::ui::theme::groups::{publish, scheme, HLGroup, GROUP_DECLARATIONS};
use crate::ui::theme::palette::blend;
use crate::ui::theme::Appearance;

/// How far a hovered link's colours move toward the text colour.
const LINK_HOVER_BRIGHTENING: f32 = 0.35;

/// Left clicks on one cell, each within the double-click pause of the one before.
struct ClickRun {
    at: Instant,
    position: Position,
    count: u8,
}

/// `count` of a noun, as a status line says it: `1 line`, `2 lines`.
fn counted(count: usize, noun: &str) -> String {
    match count {
        1 => format!("1 {noun}"),
        _ => format!("{count} {noun}s"),
    }
}

/// An outward effect produced while handling input or an [`Action`].
pub enum Reaction {
    Continue,
    Submit,
    /// Show a documentation page apart from the session.
    Documentation(String),
    /// Show a linked documentation page in the session feed.
    DocumentationInFeed(String),
    /// Browse the archived sessions.
    BrowseSessions,
    /// Preview a source file without evaluating it.
    ViewFile(String),
    /// Save selected source to a file.
    SaveSelection(String, String),
    Restart,
    Quit,
    /// Put the current selection on the system clipboard.
    Copy(String),
    /// Scroll the focused pane by rows, positive toward older content.
    ScrollView(i8),
    Open(ViewKind),
    ClosePane,
    /// Return to the preceding reference without evaluating it again.
    ReferenceBack,
    /// Write the session's code to the file typed, or where the session is kept
    /// on exit when none was.
    Save(Option<String>),
    /// Run the code in the file typed.
    Load(String),
    /// Run the code the previous session left.
    Resume,
    /// Ask the language server about the name at a point in code.
    Lookup(Lookup, CodePoint),
    /// Have the language server format this input.
    Format(String),
}

/// What is asked about a name in code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Lookup {
    /// What it is, in brief, beside where it was asked.
    Brief,
    /// All that is known about it, beside the session.
    Description,
    /// Where it is defined.
    Definition,
}

/// A point in code that something was asked about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodePoint {
    /// The input, or a source the session holds.
    pub source: Option<SourceId>,
    /// Bytes into that source.
    pub offset: usize,
    /// Where on screen it was asked.
    pub anchor: Anchor,
}

/// A session result to apply to the editor model.
pub enum EditorUpdate {
    Exit,
    Ignore,
    Batch(Vec<Self>),
    Incomplete {
        source: String,
        spans: Vec<HLSpan>,
        reason: String,
    },
    SyntaxFailed {
        id: u64,
        source: String,
        spans: Vec<HLSpan>,
        message: String,
    },
    /// Provisional whitespace retained only if later code makes it leading.
    WhitespaceAccepted,
    Diagnostic(String),
    Message(Box<dyn Block>),
    /// The opening message, written again once more is known about the session.
    Welcome(String),
    /// Every name a freshly started M2 knows, replacing those of any before it.
    KnownNames(Vec<String>),
    /// Names the session bound since M2 last reported.
    BoundNames(Vec<String>),
    /// What the language server read in the session: the names its source binds,
    /// replacing those read before.
    AnalysedNames(Vec<String>),
    /// What the language server found in the input as it last saw it.
    InputFindings(Vec<Finding>),
    /// Float something beside a point until the next key or click.
    Float(Floating),
    /// Put the input's cursor at a byte offset.
    InputCursor(usize),
    /// Where code shown beside the session folds became known.
    FoldsLearned,
    /// Where the name under the cursor occurs in the input as it read at a revision.
    InputOccurrences {
        revision: u64,
        ranges: Vec<Range<usize>>,
    },
    /// Select the `index`th code cell of a submission in the feed.
    SelectCell {
        submission: u64,
        index: usize,
    },
    /// Replace the input's text, as one undoable edit, if it still reads `was`.
    InputRewritten {
        was: String,
        now: String,
    },
    /// Documentation examples were computed again or their code analysed, so
    /// the pages showing them draw them differently.
    ExamplesComputed,
    Pending {
        id: u64,
        cells: Vec<(String, Vec<HLSpan>)>,
    },
    Queued {
        id: u64,
        cells: Vec<(String, Vec<HLSpan>)>,
    },
    Started {
        id: u64,
    },
    CellSucceeded {
        id: u64,
        index: usize,
        number: u32,
        state: CellState,
        /// Warnings and anything else the cell produced besides its printed text.
        outputs: Vec<Box<dyn Block>>,
        printed: Option<TextBlock>,
        value: Option<OutputBlock>,
    },
    CellFailed {
        id: u64,
        index: usize,
        number: u32,
        printed: Option<TextBlock>,
        message: String,
    },
}

/// A state change requested by a picker or one of its lines.
pub enum EditorEvent {
    EditCommand(KeyEvent),
    ViewFile(String),
    ResumeTyping(char),
    ClosePickers,
    RunCommand(Command),
    ClosePicker,
    RestoreDefaults,
    /// Put every shortcut back on its default key, leaving other settings be.
    RestoreKeymaps,
    SetSettings(Settings),
    Rebind(Shortcut, KeyEvent),
}

/// The editor child that receives an otherwise unclaimed input event.
#[derive(Clone, Copy)]
enum ElementFocus {
    /// The editable input field.
    Input,
    /// The topmost picker and its keyboard policy.
    Picker(PickerMode),
}

impl EditorUpdate {
    /// Whether applying this adds to the transcript or changes the height of what
    /// it holds.
    fn moves_feed_rows(&self) -> bool {
        matches!(
            self,
            Self::Incomplete { .. }
                | Self::SyntaxFailed { .. }
                | Self::Message(_)
                | Self::Welcome(_)
                | Self::ExamplesComputed
                | Self::Pending { .. }
                | Self::Queued { .. }
                | Self::CellSucceeded { .. }
                | Self::CellFailed { .. }
        )
    }
}

/// The transcript feed, live input, and viewport state.
pub struct Editor {
    feed: Feed,
    reference: Feed,
    reference_file: Option<String>,
    input: InputCode,
    settings: Settings,
    keymaps: Keymaps,
    status_line: StatusLine,
    status_runtime: StatusRuntime,
    continuation: Option<String>,
    processing: bool,
    processing_started: Option<Instant>,
    cancel_hint_shown: bool,
    /// The names M2 knows, which the input completes.
    names: Names,
    pickers: Vec<PickerController>,
    feed_area: Rect,
    feed_top: i64,
    screen_selecting: bool,
    screen_selection: Option<ScreenSelection>,
    /// What each pane drew last, to select and copy from.
    captures: Vec<ScreenCapture>,
    selection_inclusions: Inclusions,
    split: bool,
    previewing: bool,
    pane_focused: bool,
    highlight_debug: bool,
    highlight_debug_scroll: usize,
    /// What floats beside where it was asked for, until the next key or click.
    floating: Option<Floating>,
    /// Where the input's cursor was last drawn.
    input_cursor: Option<Position>,
    frame_stats: FrameStats,
    /// The run of left clicks on one cell that the latest belongs to.
    last_click: Option<ClickRun>,
    /// What the latest copy took, and when, until the status line stops saying so.
    copied: Option<(Instant, String)>,
    /// Where the mouse last was, so the link under it is underlined.
    pointer: Option<Position>,
    /// Whether Ctrl is held, so the link under the pointer shows it would open.
    control_held: bool,
}

impl Editor {
    pub fn new(welcome: String) -> Self {
        let appearance = Appearance::load();
        publish(appearance.groups());
        let saved = SavedSettings::load();
        let mut input = InputCode::new();
        input.set_settings(saved.layout);
        let mut editor = Self {
            feed: Feed::new(Box::new(TextBlock::system(welcome))),
            reference: Feed::new(Box::new(TextBlock::system(String::new()))),
            reference_file: None,
            input,
            settings: Settings {
                appearance,
                layout: saved.layout,
                statusline: saved.statusline,
                rendering: saved.rendering,
                selection: saved.selection,
            },
            keymaps: Keymaps::replaying(&saved.rebinds),
            status_line: StatusLine::default(),
            status_runtime: StatusRuntime::default(),
            continuation: None,
            processing: false,
            processing_started: None,
            cancel_hint_shown: false,
            names: Names::default(),
            pickers: Vec::new(),
            feed_area: Rect::ZERO,
            feed_top: 0,
            screen_selecting: false,
            screen_selection: None,
            captures: Vec::new(),
            selection_inclusions: Inclusions::default(),
            split: false,
            previewing: false,
            pane_focused: true,
            highlight_debug: false,
            highlight_debug_scroll: 0,
            floating: None,
            input_cursor: None,
            frame_stats: FrameStats::default(),
            last_click: None,
            copied: None,
            pointer: None,
            control_held: false,
        };
        editor.refresh_status_line();
        editor
    }

    /// Record the version reported by the session interpreter.
    /// Name the M2 process whose memory the debug component reports.
    pub fn set_m2_process(&mut self, process: u32) {
        self.status_runtime.m2_process = Some(process);
    }

    pub fn set_m2_version(&mut self, version: String) {
        self.status_runtime.m2_version = Some(version);
    }

    /// Record the language server's initialization response, or its failure.
    pub fn set_language_server_info(&mut self, server: Option<ServerInfo>) {
        self.status_runtime.language_server = server.map_or(
            LanguageServerState::Unavailable,
            LanguageServerState::Running,
        );
    }

    /// Reflect whether this editor's backend currently owns unfinished work.
    pub fn set_processing(&mut self, processing: bool) {
        if self.processing != processing {
            self.processing = processing;
            self.processing_started = processing.then(Instant::now);
            self.cancel_hint_shown = false;
            self.refresh_status_line();
        }
    }

    /// Redraw whatever moves with the clock while the backend works; false when
    /// nothing does, so an idle shell never repaints on its own.
    pub fn animate(&mut self) -> bool {
        if self.processing {
            self.feed.on_event(&Ticked);
            let show_hint = self
                .processing_started
                .is_some_and(|started| started.elapsed() >= Duration::from_secs(3));
            if self.cancel_hint_shown != show_hint {
                self.cancel_hint_shown = show_hint;
                self.refresh_status_line();
            }
        }
        self.processing
    }

    const COPY_NOTICE: Duration = Duration::from_millis(UI.copy_notice_ms);

    /// Say in the status line what a copy took, for a moment.
    pub fn note_copied(&mut self, text: &str) {
        let lines = text.lines().count();
        let taken = match lines {
            0 | 1 => counted(text.chars().count(), "character"),
            lines => counted(lines, "line"),
        };
        self.copied = Some((Instant::now(), format!("Copied {taken}")));
    }

    /// How long until something on screen changes by itself: the input's typing
    /// prompt resting, or the note of a copy lapsing.
    pub fn settles_in(&self) -> Option<Duration> {
        let notice = self
            .copied
            .as_ref()
            .and_then(|(at, _)| Self::COPY_NOTICE.checked_sub(at.elapsed()));
        [self.input.typing_remaining(), notice]
            .into_iter()
            .flatten()
            .min()
    }

    /// Let go of what has run its time, the typing prompt and the note of a copy;
    /// whether either did.
    pub fn settle(&mut self) -> bool {
        let lapsed = self
            .copied
            .as_ref()
            .is_some_and(|(at, _)| at.elapsed() >= Self::COPY_NOTICE);
        if lapsed {
            self.copied = None;
        }
        self.input.settle_typing() | lapsed
    }

    /// How long each documentation example may take to be computed again.
    pub fn example_timeout(&self) -> Duration {
        self.settings.rendering.example_timeout()
    }

    /// Whether text may be drawn in italics.
    pub fn italics(&self) -> bool {
        self.settings.rendering.italics
    }

    /// Whether the persisted settings run the language server.
    pub fn language_server(&self) -> bool {
        self.settings.rendering.language_server
    }

    /// Whether source takes semantic colors from the language server.
    pub fn semantic_highlighting(&self) -> bool {
        self.settings.rendering.semantic_colors()
    }

    /// Source and revision for the current draft, excluding shell commands.
    pub fn analysis_input(&self) -> (&str, Option<&str>, u64) {
        let text = self.input.text();
        let draft = if text.starts_with(commands::PREFIX) {
            ""
        } else {
            text
        };
        (draft, self.feed.incomplete_source(), self.input.revision())
    }

    /// Where the input's cursor is, as a byte offset into its text.
    pub fn input_cursor(&self) -> usize {
        self.input.field().cursor()
    }

    /// Refresh transcript source colors without changing editor interaction state.
    pub fn rehighlight(&mut self, highlighter: &mut crate::code::semantic::SemanticHighlighter) {
        let sources = highlighter
            .segments()
            .iter()
            .map(|segment| segment.id)
            .collect::<Vec<_>>();
        self.feed.rehighlight(&sources, highlighter);
    }

    pub fn submission(&self) -> &str {
        self.input.text()
    }

    pub fn set_context(&mut self, split: bool, previewing: bool, pane_focused: bool) {
        self.split = split;
        self.previewing = previewing;
        self.pane_focused = pane_focused;
    }

    pub fn picker_open(&self) -> bool {
        matches!(self.focus(), ElementFocus::Picker(_))
    }

    /// Resolve a terminal key through the active command keymaps.
    pub fn resolve_action(&self, key: KeyEvent) -> Option<Action> {
        if self
            .pickers
            .last()
            .is_some_and(|controller| controller.picker.capturing())
        {
            return None;
        }
        self.keymaps.action(
            key,
            matches!(self.focus(), ElementFocus::Picker(PickerMode::Keymaps)),
        )
    }

    /// Forward unbound preview input to its focused component.
    pub fn handle_preview_key(&mut self, key: KeyEvent) -> Reaction {
        if matches!(self.focus(), ElementFocus::Picker(_)) {
            return self.handle_picker_key(key);
        }
        if let Some(action) = self.handle_debug_key(key) {
            return action;
        }
        Reaction::Continue
    }

    /// Apply a session result. Returns whether the shell should keep running.
    pub fn apply(&mut self, update: EditorUpdate) -> bool {
        self.release_selection_for(&update);
        match update {
            EditorUpdate::Exit => return false,
            EditorUpdate::Ignore => {}
            EditorUpdate::Batch(updates) => {
                return updates.into_iter().all(|update| self.apply(update));
            }
            EditorUpdate::Incomplete {
                source,
                spans,
                reason,
            } => {
                self.input.accept_submission();
                self.feed
                    .set_incomplete(IncompleteBlock::new(source, spans));
                self.continuation = Some(reason);
            }
            EditorUpdate::SyntaxFailed {
                id,
                source,
                spans,
                message,
            } => {
                self.continuation = None;
                if !self.input.text().is_empty() {
                    self.input.restore(source.clone());
                    self.input.accept_submission();
                }
                self.push_evaluation(EvaluationBlock::invalid(id, source, spans, message));
            }
            EditorUpdate::WhitespaceAccepted => {
                self.continuation = None;
                self.input.accept_submission();
            }
            EditorUpdate::Diagnostic(diagnostic) => self.input.set_diagnostic(diagnostic, None),
            EditorUpdate::Message(block) => {
                self.push_block(block);
            }
            EditorUpdate::Welcome(text) => {
                self.feed.replace_opening(Box::new(TextBlock::system(text)));
            }
            EditorUpdate::KnownNames(names) => {
                self.names.reset(names);
                self.input.forget_completion();
            }
            EditorUpdate::BoundNames(names) => {
                self.names.bind(names);
                self.input.forget_completion();
            }
            EditorUpdate::AnalysedNames(names) => {
                self.names.analyse(names);
                self.input.forget_completion();
            }
            EditorUpdate::InputFindings(findings) => self.input.set_findings(findings),
            EditorUpdate::Float(floating) => self.floating = Some(floating),
            EditorUpdate::InputCursor(offset) => self.input.field_mut().place_cursor(offset),
            EditorUpdate::InputOccurrences { revision, ranges } => {
                self.input.set_occurrences(revision, ranges)
            }
            EditorUpdate::SelectCell { submission, index } => {
                if !self.feed.select_cell(submission, index) {
                    self.input
                        .set_diagnostic("the cell defining it is no longer shown".into(), None);
                }
            }
            EditorUpdate::InputRewritten { was, now } => {
                if self.input.text() == was {
                    self.input.field_mut().select_all();
                    self.input.field_mut().insert_str(&now);
                }
            }
            EditorUpdate::FoldsLearned => self.reference.on_event(&Reformatted),
            EditorUpdate::ExamplesComputed => {
                self.feed.on_event(&Reformatted);
                self.reference.on_event(&Reformatted);
            }
            EditorUpdate::Pending { id, cells } => {
                self.continuation = None;
                self.input.accept_submission();
                self.push_evaluation(EvaluationBlock::new(id, cells));
            }
            EditorUpdate::Queued { id, cells } => {
                self.continuation = None;
                self.input.accept_submission();
                self.push_evaluation(EvaluationBlock::queued(id, cells));
            }
            EditorUpdate::Started { id } => {
                self.feed.update_evaluation(id, EvaluationBlock::start);
            }
            EditorUpdate::CellSucceeded {
                id,
                index,
                number,
                state,
                outputs,
                printed,
                value,
            } => {
                self.input.clear_diagnostic();
                let names = &mut self.names;
                self.feed.update_evaluation(id, |evaluation| {
                    if let Some(source) = evaluation.source(index) {
                        names.record(source);
                    }
                    evaluation.succeed(index, number, state, outputs, printed, value)
                });
                self.input.forget_completion();
            }
            EditorUpdate::CellFailed {
                id,
                index,
                number,
                printed,
                message,
            } => {
                self.input.clear_diagnostic();
                self.feed.update_evaluation(id, |evaluation| {
                    evaluation.fail(index, number, printed, message)
                });
            }
        }
        self.refresh_status_line();
        true
    }

    pub fn post(&mut self, update: EditorUpdate) -> bool {
        self.release_selection_for(&update);
        match update {
            EditorUpdate::Batch(updates) => updates.into_iter().all(|update| self.post(update)),
            EditorUpdate::Pending { id, cells } => {
                self.push_evaluation(EvaluationBlock::new(id, cells));
                self.refresh_status_line();
                true
            }
            EditorUpdate::Queued { id, cells } => {
                self.push_evaluation(EvaluationBlock::queued(id, cells));
                self.refresh_status_line();
                true
            }
            update => self.apply(update),
        }
    }

    /// A selection names rows, which `update` is about to move if it changes the
    /// transcript, so one no longer held lets go rather than land on other text.
    fn release_selection_for(&mut self, update: &EditorUpdate) {
        if update.moves_feed_rows() && !self.screen_selecting {
            self.screen_selection = None;
        }
    }

    fn push_evaluation(&mut self, block: EvaluationBlock) {
        self.feed.push_evaluation(block);
    }

    fn push_block(&mut self, block: Box<dyn Block>) {
        self.feed.push(block);
    }

    /// Discard a pending continuation and the current draft.
    pub fn cancel_input(&mut self) -> bool {
        let pending = self.feed.take_incomplete().is_some();
        let changed = pending || !self.input.text().is_empty();
        if changed {
            self.input.clear();
            self.continuation = None;
            self.refresh_status_line();
        }
        changed
    }

    pub fn paste(&mut self, text: &str) {
        self.input.insert(text);
        self.refresh_status_line();
    }

    /// Longest pause between two clicks on one cell that still reads as a double click.
    const DOUBLE_CLICK: Duration = Duration::from_millis(400);

    /// Clicks a run counts to before starting over: one, a word, a paragraph.
    const LONGEST_CLICK_RUN: u8 = 3;

    pub fn handle_mouse(&mut self, event: MouseEvent) -> Reaction {
        if matches!(event.kind, MouseEventKind::Down(_)) {
            self.floating = None;
        }
        let reaction = self.route_mouse(event);
        self.copied_on_select(reaction)
    }

    /// What a click on code asks about the name under it: a middle click what it
    /// is, a click with Shift held where it is defined.
    fn lookup_click(event: MouseEvent) -> Option<Lookup> {
        match event.kind {
            MouseEventKind::Down(MouseButton::Middle) => Some(Lookup::Brief),
            MouseEventKind::Down(MouseButton::Left)
                if event.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                Some(Lookup::Definition)
            }
            _ => None,
        }
    }

    /// What finishing a mouse selection does: copy it, or leave it to the copy key.
    fn copied_on_select(&self, reaction: Reaction) -> Reaction {
        match reaction {
            Reaction::Copy(_) if !self.settings.selection.copy_on_select => Reaction::Continue,
            reaction => reaction,
        }
    }

    /// Route the wheel by what it is pointing at: over the input pane it moves
    /// through the input's lines, everywhere else it scrolls the feed. A double click on a folded
    /// value expands it, and on an expanded one folds it back.
    fn route_mouse(&mut self, event: MouseEvent) -> Reaction {
        let pointer = Position::new(event.column, event.row);
        self.pointer = Some(pointer);
        self.control_held = event.modifiers.contains(KeyModifiers::CONTROL);
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Left)) {
            self.feed.deselect();
        }
        if let Some(lookup) = Self::lookup_click(event) {
            if let Some(offset) = self.input.offset_under(pointer) {
                return Reaction::Lookup(
                    lookup,
                    CodePoint {
                        source: None,
                        offset,
                        anchor: Anchor::At(pointer),
                    },
                );
            }
        }
        if let Some(in_feed) = Self::link_click(event) {
            if let Some(link) = self.feed.link_at(pointer) {
                self.screen_selection = None;
                return Self::open_link(link.target.to_string(), in_feed);
            }
        }
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
            && self.feed.cycle_renderer(pointer)
        {
            self.screen_selection = None;
            return Reaction::Continue;
        }
        let clicks = self.clicks(event);
        if clicks >= 2 && self.feed.toggle_fold(pointer) {
            self.last_click = None;
            self.screen_selection = None;
            return Reaction::Continue;
        }
        if clicks >= 2 && self.input.contains(event) {
            self.screen_selection = None;
            return self
                .input
                .handle_repeated_click(event, clicks)
                .map_or(Reaction::Continue, Reaction::Copy);
        }
        if clicks >= 2 {
            let picked = match self.outside_text(event) {
                true => self.select_block_at(pointer, clicks),
                false => self.pick_screen_text(event, clicks),
            };
            if let Some(reaction) = picked {
                return reaction;
            }
        }
        if let Some(action) = self.handle_screen_mouse(event) {
            return action;
        }
        match event.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let delta = if event.kind == MouseEventKind::ScrollUp {
                    1
                } else {
                    -1
                };
                if self.input.contains(event) {
                    let field = self.input.field_mut();
                    match delta > 0 {
                        true => field.move_up(),
                        false => field.move_down(),
                    }
                } else {
                    self.feed.scroll(delta);
                }
            }
            _ => {}
        }
        let (handled, copy) = self.input.handle_mouse(event);
        if handled {
            self.screen_selection = None;
            return copy.map_or(Reaction::Continue, Reaction::Copy);
        }
        Reaction::Continue
    }

    /// A middle click opens a reference; Ctrl-left opens in the transcript.
    fn link_click(event: MouseEvent) -> Option<bool> {
        match event.kind {
            MouseEventKind::Down(MouseButton::Middle) => Some(false),
            MouseEventKind::Down(MouseButton::Left)
                if event.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                Some(true)
            }
            _ => None,
        }
    }

    fn open_link(target: String, in_feed: bool) -> Reaction {
        if in_feed {
            Reaction::DocumentationInFeed(target)
        } else {
            Reaction::Documentation(target)
        }
    }

    /// Track Ctrl pressed or released on its own, which no mouse event reports.
    pub fn hold_control(&mut self, held: bool) {
        self.control_held = held;
    }

    /// Record a left press, reporting how many presses in a row the run it belongs to
    /// has: each repeating the previous one in time and place, counted again from one
    /// after the third. None for anything but a press.
    fn clicks(&mut self, event: MouseEvent) -> u8 {
        let MouseEventKind::Down(MouseButton::Left) = event.kind else {
            return 0;
        };
        let (at, position) = (Instant::now(), Position::new(event.column, event.row));
        let count = match &self.last_click {
            Some(run)
                if run.position == position && at.duration_since(run.at) <= Self::DOUBLE_CLICK =>
            {
                run.count % Self::LONGEST_CLICK_RUN + 1
            }
            _ => 1,
        };
        self.last_click = Some(ClickRun {
            at,
            position,
            count,
        });
        count
    }

    /// Take the cell or result under a click outside any text, or with a third click
    /// everything its submission printed and returned, copying it as the settings say.
    fn select_block_at(&mut self, pointer: Position, clicks: u8) -> Option<Reaction> {
        if !self.feed.select_at(pointer, clicks == 3) {
            return None;
        }
        self.screen_selection = None;
        self.screen_selecting = false;
        self.input.field_mut().clear_selection();
        Some(
            self.feed
                .selected_text(self.settings.selection.copy_format)
                .map_or(Reaction::Continue, Reaction::Copy),
        )
    }

    pub fn handle_preview_mouse(&mut self, event: MouseEvent) -> Reaction {
        let pointer = Position::new(event.column, event.row);
        self.pointer = Some(pointer);
        self.control_held = event.modifiers.contains(KeyModifiers::CONTROL);
        if let Some(in_feed) = Self::link_click(event) {
            if let Some(link) = self.reference.link_at(pointer) {
                self.screen_selection = None;
                return Self::open_link(link.target.to_string(), in_feed);
            }
        }
        let clicks = self.clicks(event);
        if clicks >= 2 && self.reference.toggle_fold(pointer) {
            self.last_click = None;
            self.screen_selection = None;
            return Reaction::Continue;
        }
        let reaction = (clicks >= 2)
            .then(|| self.pick_screen_text(event, clicks))
            .flatten()
            .or_else(|| self.handle_screen_mouse(event))
            .unwrap_or(Reaction::Continue);
        self.copied_on_select(reaction)
    }

    /// Move the feed by signed rows, positive toward older content.
    pub fn scroll_feed(&mut self, rows: i8) {
        self.feed.scroll(rows);
    }

    /// Forward unbound shell input to its focused component.
    pub fn handle_input(&mut self, key: KeyEvent) -> Reaction {
        self.floating = None;
        if matches!(self.focus(), ElementFocus::Picker(_)) {
            return self.handle_picker_key(key);
        }
        if let Some(action) = self.handle_debug_key(key) {
            return action;
        }
        if self.feed.has_selection() {
            match self.handle_block_key(key) {
                Some(reaction) => return reaction,
                None => self.feed.deselect(),
            }
        }
        let edit = self.keymaps.edit(key);
        self.screen_selection = None;
        self.refresh_completion();
        self.input.handle_edit_key(key, edit);
        if self.input.text().is_empty() {
            self.continuation = None;
        }
        if self.input.text() == commands::PREFIX {
            self.open_filtered_commands();
        }
        self.refresh_status_line();
        Reaction::Continue
    }

    /// What a key does while a cell or result is selected, if it is one of the keys
    /// that act on it; any other leaves the selection and types as usual.
    fn handle_block_key(&mut self, key: KeyEvent) -> Option<Reaction> {
        if key.ctrl() || key.alt() || key.shift() {
            return None;
        }
        Some(match key.code {
            KeyCode::Up => self.step_block_selection(-1),
            KeyCode::Down => self.step_block_selection(1),
            KeyCode::Esc => {
                self.feed.deselect();
                Reaction::Continue
            }
            KeyCode::Char('y') => self
                .feed
                .selected_text(self.settings.selection.copy_format)
                .map_or(Reaction::Continue, Reaction::Copy),
            _ => return None,
        })
    }

    /// Move the selection to the previous or next cell or result, and off whatever
    /// text was selected.
    fn step_block_selection(&mut self, delta: i8) -> Reaction {
        if self.feed.step_selection(delta) {
            self.screen_selection = None;
            self.screen_selecting = false;
            self.input.field_mut().clear_selection();
        }
        Reaction::Continue
    }

    /// The text a copy or a save takes: what is selected in the input, else on screen,
    /// else the cell or result selected whole, as `format` copies it.
    fn selected_text(&self, format: CopyFormat) -> Option<String> {
        let typed = match self.previewing {
            true => None,
            false => self.input.field().selected_text().map(str::to_string),
        };
        typed
            .or_else(|| self.selected_screen_text())
            .or_else(|| self.feed.selected_text(format).filter(|_| !self.previewing))
    }

    /// React to a mapped command after its originating key has been discarded.
    pub fn handle_action(&mut self, action: Action) -> Reaction {
        self.floating = None;
        if matches!(self.focus(), ElementFocus::Picker(_)) {
            return self.handle_picker_action(action);
        }
        if let Some(result) = self.handle_debug_action(action) {
            return result;
        }
        if !matches!(
            action,
            Action::CopySelection | Action::SaveSelection | Action::Submit | Action::OpenCommands
        ) {
            self.screen_selection = None;
        }
        if !matches!(
            action,
            Action::SelectPreviousBlock
                | Action::SelectNextBlock
                | Action::CopySelection
                | Action::SaveSelection
                | Action::ScrollFeedUp
                | Action::ScrollFeedDown
        ) {
            self.feed.deselect();
        }
        if let Some(reaction) = self.perform(action, "") {
            return reaction;
        }

        if action == Action::CopySelection {
            return self
                .selected_text(self.settings.selection.copy_format)
                .map_or(Reaction::Continue, Reaction::Copy);
        }
        if self.previewing {
            return Reaction::Continue;
        }
        match action {
            Action::SelectPreviousBlock => return self.step_block_selection(-1),
            Action::SelectNextBlock => return self.step_block_selection(1),
            _ => {}
        }
        if action == Action::Submit {
            self.input.clear_diagnostic();
            if let Some((command, argument)) = commands::exact(self.input.text()) {
                let argument = argument.to_string();
                self.continuation = None;
                self.input.accept_submission();
                return self
                    .perform(command.action, &argument)
                    .expect("a command's action means the same whatever has focus");
            }
            // Submitting is not an edit, so an incomplete buffer's inserted
            // newline must not consume a live selection.
            self.input.field_mut().clear_selection();
            self.refresh_status_line();
            return Reaction::Submit;
        }

        let at_cursor = |lookup| {
            Reaction::Lookup(
                lookup,
                CodePoint {
                    source: None,
                    offset: self.input.field().cursor(),
                    anchor: Anchor::InputCursor,
                },
            )
        };
        match action {
            Action::Describe => return at_cursor(Lookup::Brief),
            Action::DescribeBeside => return at_cursor(Lookup::Description),
            Action::GoToDefinition => return at_cursor(Lookup::Definition),
            Action::FormatInput => return Reaction::Format(self.input.text().to_string()),
            Action::CopySelection => unreachable!("copy selection is handled before editing"),
            Action::SelectPreviousBlock | Action::SelectNextBlock => {
                unreachable!("block selection is handled before editing")
            }
            Action::Submit => unreachable!("submission is handled before component dispatch"),
            Action::ScrollFeedUp => self.feed.scroll(1),
            Action::ScrollFeedDown => self.feed.scroll(-1),
            Action::OpenCommands => self.toggle_picker(PickerMode::Commands, Self::open_commands),
            Action::ToggleHighlightDebug => unreachable!("debug toggles before input handling"),
            Action::CancelExecution => unreachable!("cancelling is handled by the app"),
            Action::OpenSettings
            | Action::OpenKeymaps
            | Action::ClearFeed
            | Action::OpenCode
            | Action::OpenValues
            | Action::OpenStdOut
            | Action::OpenOutput
            | Action::OpenSyntaxTree
            | Action::ClosePane
            | Action::ReferenceBack
            | Action::Restart
            | Action::Quit
            | Action::RunViewedFile
            | Action::BrowseSessions
            | Action::ViewFile
            | Action::SaveSelection
            | Action::SaveSession
            | Action::LoadCode
            | Action::ResumeSession => {
                unreachable!("focus-independent actions perform before input handling")
            }
        }
        self.refresh_status_line();
        Reaction::Continue
    }

    /// Carry out an action that means the same whatever has focus, however it was
    /// triggered: by its key or by typing its command. Actions whose meaning
    /// depends on the focused component are left to it.
    fn perform(&mut self, action: Action, argument: &str) -> Option<Reaction> {
        Some(match action {
            Action::OpenSettings => {
                self.open_from_commands(PickerMode::Settings, Self::settings_picker);
                Reaction::Continue
            }
            Action::OpenKeymaps => {
                self.open_keymaps();
                Reaction::Continue
            }
            Action::ClearFeed => {
                self.clear_feed();
                Reaction::Continue
            }
            Action::OpenCode => self.leave_pickers(Reaction::Open(ViewKind::Code)),
            Action::OpenValues => self.leave_pickers(Reaction::Open(ViewKind::Values)),
            Action::OpenStdOut => self.leave_pickers(Reaction::Open(ViewKind::StdOut)),
            Action::OpenOutput => self.leave_pickers(Reaction::Open(ViewKind::Output)),
            Action::OpenSyntaxTree => self.leave_pickers(Reaction::Open(ViewKind::SyntaxTree)),
            Action::ClosePane => self.leave_pickers(Reaction::ClosePane),
            Action::ReferenceBack => self.leave_pickers(Reaction::ReferenceBack),
            Action::Restart => self.leave_pickers(Reaction::Restart),
            Action::Quit => self.leave_pickers(Reaction::Quit),
            Action::BrowseSessions => self.leave_pickers(Reaction::BrowseSessions),
            Action::ViewFile if !argument.is_empty() => {
                self.leave_pickers(Reaction::ViewFile(argument.to_string()))
            }
            Action::SaveSelection => match self.selected_text(CopyFormat::Raw) {
                Some(source) => {
                    self.leave_pickers(Reaction::SaveSelection(argument.to_string(), source))
                }
                None => {
                    self.input
                        .set_diagnostic("select code before saving".into(), None);
                    Reaction::Continue
                }
            },
            Action::ViewFile => {
                self.input.set_diagnostic("name a file".into(), None);
                Reaction::Continue
            }
            Action::SaveSession => self.leave_pickers(Reaction::Save(
                (!argument.is_empty()).then(|| argument.to_string()),
            )),
            Action::LoadCode if argument.is_empty() => {
                self.input
                    .set_diagnostic("name a file to run".to_string(), None);
                Reaction::Continue
            }
            Action::LoadCode => self.leave_pickers(Reaction::Load(argument.to_string())),
            Action::ResumeSession => self.leave_pickers(Reaction::Resume),
            Action::RunViewedFile
            | Action::CancelExecution
            | Action::Submit
            | Action::ScrollFeedUp
            | Action::ScrollFeedDown
            | Action::OpenCommands
            | Action::ToggleHighlightDebug
            | Action::SelectPreviousBlock
            | Action::SelectNextBlock
            | Action::Describe
            | Action::DescribeBeside
            | Action::GoToDefinition
            | Action::FormatInput
            | Action::CopySelection => return None,
        })
    }

    /// Close the pickers before `reaction` takes effect outside them.
    fn leave_pickers(&mut self, reaction: Reaction) -> Reaction {
        self.close_pickers();
        reaction
    }

    fn open_commands(&mut self) {
        let commands = commands::matching("");
        self.pickers = vec![PickerController {
            mode: PickerMode::Commands,
            picker: commands::picker(&commands, &self.keymaps, ""),
        }];
        self.refresh_status_line();
    }

    fn toggle_picker(&mut self, mode: PickerMode, open: impl FnOnce(&mut Self)) {
        if !self.close_if_open(mode) {
            open(self);
        }
    }

    fn close_if_open(&mut self, mode: PickerMode) -> bool {
        let open = self
            .pickers
            .iter()
            .any(|controller| controller.mode == mode);
        if open {
            self.pickers.clear();
            self.refresh_status_line();
        }
        open
    }

    fn open_filtered_commands(&mut self) {
        let commands = commands::matching("");
        self.pickers = vec![PickerController {
            mode: PickerMode::FilteredCommands,
            picker: commands::picker(&commands, &self.keymaps, ""),
        }];
        self.refresh_status_line();
    }

    fn refresh_commands(&mut self) {
        let Some(query) = self.input.text().strip_prefix(commands::PREFIX) else {
            self.pickers.clear();
            self.refresh_status_line();
            return;
        };
        let commands = commands::matching(query);
        // A command typed or completed whole needs no picker to find it, unless
        // its file is still to be typed.
        let complete = commands::exact(self.input.text())
            .is_some_and(|(command, _)| command.argument.is_none());
        if commands.is_empty() || complete {
            self.pickers.clear();
            self.refresh_status_line();
            return;
        }
        let Some(selected) = self
            .pickers
            .last()
            .map(|controller| controller.picker.selected())
        else {
            return;
        };
        let mut picker = commands::picker(&commands, &self.keymaps, query);
        picker.select(selected);
        self.pickers.last_mut().unwrap().picker = picker;
    }

    fn close_pickers(&mut self) {
        let filtered = self
            .pickers
            .last()
            .is_some_and(|controller| matches!(controller.mode, PickerMode::FilteredCommands));
        self.pickers.clear();
        if filtered {
            self.input.clear();
        }
        self.refresh_status_line();
    }

    /// Run a command picked from a picker. One picked while typing its name
    /// goes into history as the whole command, not the part typed to find it.
    fn run_command(&mut self, command: Command) -> Reaction {
        if matches!(command.argument, Some(commands::Argument::Required(_))) {
            self.close_pickers();
            self.paste(&format!("{} ", command.literal()));
            return Reaction::Continue;
        }
        if self
            .pickers
            .last()
            .is_some_and(|controller| matches!(controller.mode, PickerMode::FilteredCommands))
        {
            self.input.restore(command.literal());
            self.input.accept_submission();
        }
        self.handle_action(command.action)
    }

    /// The command selected in the picker filtering commands by the name typed.
    fn picked_command(&self) -> Option<Command> {
        let controller = self
            .pickers
            .last()
            .filter(|controller| controller.mode == PickerMode::FilteredCommands)?;
        let query = self.input.text().strip_prefix(commands::PREFIX)?;
        commands::matching(query)
            .get(controller.picker.selected())
            .copied()
    }

    /// Offer the input a completion for its text as it stands, unless the one
    /// offered already is for this text.
    fn refresh_completion(&mut self) {
        if !self.input.needs_completion() {
            return;
        }
        self.input
            .set_command_range(commands::name_range(self.input.text()));
        let completion = completion::complete(Request {
            text: self.input.text(),
            cursor: self.input.field().cursor(),
            pending: self.feed.incomplete_source(),
            picked: self.picked_command(),
            names: &self.names,
            complete_after: usize::from(self.settings.layout.complete_after),
        });
        self.input.offer(completion);
    }

    fn clear_feed(&mut self) {
        self.close_pickers();
        self.feed.clear();
        self.screen_selection = None;
    }

    fn settings_picker(&self) -> Picker {
        Picker::tabbed(
            settings_tabs::tabs(self.settings, &self.keymaps),
            Some(settings_tabs::restore_defaults()),
        )
    }

    /// Show the current settings in the settings picker on top.
    fn refresh_settings_picker(&mut self) {
        let tabs = settings_tabs::tabs(self.settings, &self.keymaps);
        self.pickers.last_mut().unwrap().picker.set_tabs(tabs);
    }

    fn keymaps_picker(&self) -> Picker {
        let mut picker = self.settings_picker();
        picker.select_mode(PickerMode::Keymaps);
        picker
    }

    fn open_keymaps(&mut self) {
        if let Some(controller) = self
            .pickers
            .last_mut()
            .filter(|controller| controller.mode == PickerMode::Settings)
        {
            controller.picker.select_mode(PickerMode::Keymaps);
            self.refresh_status_line();
        } else {
            self.open_from_commands(PickerMode::Settings, Self::keymaps_picker);
        }
    }

    fn handle_picker_key(&mut self, key: KeyEvent) -> Reaction {
        let ElementFocus::Picker(_) = self.focus() else {
            return Reaction::Continue;
        };
        // The command picked while typing its name is what the input completes
        // to, so moving through the picker changes the completion.
        self.input.forget_completion();
        if let Some(event) = self.pickers.last_mut().unwrap().handle_key(key) {
            let action = self.handle_picker_event(event);
            if !matches!(action, Reaction::Continue) {
                return action;
            }
        }
        self.refresh_status_line();
        Reaction::Continue
    }

    fn handle_picker_action(&mut self, action: Action) -> Reaction {
        let ElementFocus::Picker(mode) = self.focus() else {
            return Reaction::Continue;
        };
        if mode == PickerMode::Keymaps
            && matches!(action, Action::OpenKeymaps | Action::OpenSettings)
        {
            self.close_if_open(PickerMode::Settings);
            return Reaction::Continue;
        }
        if action == Action::Quit {
            return self.leave_pickers(Reaction::Quit);
        }
        if mode != PickerMode::Keymaps {
            if let Some(result) = self.handle_debug_action(action) {
                return result;
            }
            if action == Action::OpenCommands {
                self.toggle_picker(PickerMode::Commands, Self::open_commands);
                return Reaction::Continue;
            }
            if let Some(reaction) = self.perform(action, "") {
                return reaction;
            }
        }
        if let Some(event) = self.pickers.last_mut().unwrap().handle_action(action) {
            let result = self.handle_picker_event(event);
            if !matches!(result, Reaction::Continue) {
                return result;
            }
        }
        self.refresh_status_line();
        Reaction::Continue
    }

    fn focus(&self) -> ElementFocus {
        self.pickers.last().map_or(ElementFocus::Input, |picker| {
            ElementFocus::Picker(picker.active_mode())
        })
    }

    fn open_from_commands(&mut self, mode: PickerMode, picker: fn(&Self) -> Picker) {
        if self.close_if_open(mode) {
            return;
        }
        if matches!(
            self.pickers.last().map(|controller| controller.mode),
            Some(PickerMode::FilteredCommands)
        ) {
            self.input.clear();
        }
        self.pickers.push(PickerController {
            mode,
            picker: picker(self),
        });
        self.refresh_status_line();
    }

    fn handle_picker_event(&mut self, event: EditorEvent) -> Reaction {
        match event {
            EditorEvent::EditCommand(key) => {
                let revision = self.input.revision();
                match key.code {
                    KeyCode::Tab if key.ctrl() => {
                        self.pickers.last_mut().unwrap().picker.move_selection(-1)
                    }
                    KeyCode::Tab => {
                        if let Some(command) = self.picked_command() {
                            self.input.restore(commands::completed(command));
                        }
                    }
                    KeyCode::BackTab => self.pickers.last_mut().unwrap().picker.move_selection(1),
                    KeyCode::Char(character) => self.input.field_mut().insert_char(character),
                    KeyCode::Backspace => self.input.field_mut().backspace(),
                    KeyCode::Delete => self.input.field_mut().delete_forward(),
                    KeyCode::Left => self.input.field_mut().move_left(),
                    KeyCode::Right => self.input.field_mut().move_right(),
                    KeyCode::Home => self.input.field_mut().move_home(),
                    KeyCode::End => self.input.field_mut().move_end(),
                    _ => {}
                }
                self.input.note_keystroke_since(revision);
                self.refresh_commands();
            }
            EditorEvent::ResumeTyping(character) => {
                self.pickers.clear();
                if !self.previewing {
                    let revision = self.input.revision();
                    self.input.clear_diagnostic();
                    self.input.field_mut().insert_char(character);
                    self.input.note_keystroke_since(revision);
                }
            }
            EditorEvent::ClosePickers => {
                // Escape only puts the pickers away; what was typed to find a
                // command stays for the user to keep editing.
                self.pickers.clear();
                self.refresh_status_line();
            }
            EditorEvent::ViewFile(path) => return self.leave_pickers(Reaction::ViewFile(path)),
            EditorEvent::RunCommand(command) => return self.run_command(command),
            EditorEvent::ClosePicker => {
                self.pickers.pop();
                if matches!(
                    self.pickers.last().map(|controller| controller.mode),
                    Some(PickerMode::Settings)
                ) {
                    self.refresh_settings_picker();
                }
                if matches!(
                    self.pickers.last().map(|controller| controller.mode),
                    Some(PickerMode::FilteredCommands)
                ) {
                    self.input.restore(commands::PREFIX.to_string());
                    self.refresh_commands();
                }
            }
            EditorEvent::RestoreDefaults => {
                self.keymaps = Keymaps::default();
                self.set_settings(Settings::default());
            }
            EditorEvent::RestoreKeymaps => {
                self.keymaps = Keymaps::default();
                self.save_settings();
                self.refresh_settings_picker();
            }
            EditorEvent::SetSettings(settings) => self.set_settings(settings),
            EditorEvent::Rebind(shortcut, key) => {
                if self.keymaps.rebind(shortcut, key) {
                    self.save_settings();
                    self.refresh_settings_picker();
                }
            }
        }
        Reaction::Continue
    }

    fn set_settings(&mut self, settings: Settings) {
        if settings.rendering.language_server != self.settings.rendering.language_server {
            self.status_runtime.language_server = LanguageServerState::Starting;
        }
        if settings.reformats(&self.settings) {
            self.feed.on_event(&Reformatted);
            self.reference.on_event(&Reformatted);
        }
        self.apply_appearance(settings.appearance);
        self.settings = settings;
        self.input.set_settings(settings.layout);
        self.save_settings();
        self.refresh_settings_picker();
    }

    /// Keep what the settings and keymap pickers changed for the next session.
    fn save_settings(&self) {
        SavedSettings {
            layout: self.settings.layout,
            statusline: self.settings.statusline,
            rendering: self.settings.rendering,
            selection: self.settings.selection,
            rebinds: self.keymaps.rebinds().to_vec(),
        }
        .save();
    }

    fn apply_appearance(&mut self, appearance: Appearance) {
        if self.settings.appearance == appearance {
            return;
        }
        self.settings.appearance = appearance;
        publish(appearance.groups());
        self.feed.on_event(&Rethemed);
        self.reference.on_event(&Rethemed);
        appearance.save();
    }

    fn handle_debug_key(&mut self, key: KeyEvent) -> Option<Reaction> {
        if self.highlight_debug && key.code == KeyCode::Esc {
            return Some(self.toggle_highlight_debug());
        }
        if self.highlight_debug {
            match key.code {
                KeyCode::Up => {
                    self.highlight_debug_scroll = self.highlight_debug_scroll.saturating_sub(1)
                }
                KeyCode::Down => {
                    self.highlight_debug_scroll = self.highlight_debug_scroll.saturating_add(1)
                }
                KeyCode::PageUp => {
                    self.highlight_debug_scroll = self.highlight_debug_scroll.saturating_sub(10)
                }
                KeyCode::PageDown => {
                    self.highlight_debug_scroll = self.highlight_debug_scroll.saturating_add(10)
                }
                _ => return None,
            }
            return Some(Reaction::Continue);
        }
        None
    }

    fn handle_debug_action(&mut self, action: Action) -> Option<Reaction> {
        if action == Action::ToggleHighlightDebug {
            return Some(self.toggle_highlight_debug());
        }
        if self.highlight_debug {
            match action {
                Action::ScrollFeedUp => {
                    self.highlight_debug_scroll = self.highlight_debug_scroll.saturating_sub(10)
                }
                Action::ScrollFeedDown => {
                    self.highlight_debug_scroll = self.highlight_debug_scroll.saturating_add(10)
                }
                _ => return None,
            }
            return Some(Reaction::Continue);
        }
        None
    }

    fn toggle_highlight_debug(&mut self) -> Reaction {
        self.highlight_debug = !self.highlight_debug;
        self.highlight_debug_scroll = 0;
        Reaction::Continue
    }

    fn highlight_debug_sections(&self) -> Vec<DebugSection> {
        let mut lsp = Vec::new();
        let mut tree_sitter = Vec::new();
        let mut interface = Vec::new();
        for (group, style) in scheme().styled_groups() {
            let entry = DebugEntry::group(group.description(), style);
            if SEMANTIC_GROUPS.contains(&group) {
                lsp.push(entry);
            } else if GROUP_DECLARATIONS.draws_captures(group) {
                tree_sitter.push(entry);
            } else {
                interface.push(entry);
            }
        }
        let palette = self
            .settings
            .appearance
            .variant()
            .palette()
            .colors()
            .into_iter()
            .map(|(name, color)| DebugEntry::swatch(name, color))
            .collect();
        vec![
            DebugSection::new("LSP highlights", lsp),
            DebugSection::new("tree-sitter highlights", tree_sitter),
            DebugSection::new("UI", interface),
            DebugSection::new("original palette", palette),
        ]
    }

    fn draw_highlight_debug(&mut self, frame: &mut Frame) {
        let available = self.feed_area;
        if !self.highlight_debug || available.width < 4 || available.height < 5 {
            return;
        }
        let sections = self.highlight_debug_sections();
        let title = format!(
            " {} {} · debug ",
            self.settings.appearance.scheme().label(),
            self.settings.appearance.variant().label()
        );
        let entry_widths = sections
            .iter()
            .map(|section| {
                section
                    .entries
                    .iter()
                    .map(DebugEntry::width)
                    .max()
                    .unwrap_or(1)
            })
            .collect::<Vec<_>>();
        let tallest = sections
            .iter()
            .map(|section| section.entries.len())
            .max()
            .unwrap_or(0);
        let visible_rows = usize::from(available.height - 4);
        let inner_width = usize::from(available.width - 2);
        let section_width = |section: &DebugSection, entry_width: usize, rows: usize| {
            (section.entries.len().div_ceil(rows).max(1) * entry_width)
                .max(section.title.chars().count() + 2)
        };
        let total_width = |rows: usize| {
            sections
                .iter()
                .zip(&entry_widths)
                .map(|(section, &entry_width)| section_width(section, entry_width, rows))
                .sum::<usize>()
        };
        // Sections grow sideways into more sub-columns before the panel scrolls.
        let rows = (visible_rows.min(tallest).max(1)..=tallest.max(1))
            .find(|&rows| total_width(rows) <= inner_width)
            .unwrap_or(tallest.max(1));
        let content_width = total_width(rows).max(title.chars().count());
        self.highlight_debug_scroll = self
            .highlight_debug_scroll
            .min(rows.saturating_sub(visible_rows));
        let heading = scheme().prompt().add_modifier(Modifier::BOLD);
        let section_widths = sections
            .iter()
            .zip(&entry_widths)
            .map(|(section, &entry_width)| section_width(section, entry_width, rows))
            .collect::<Vec<_>>();
        let mut lines = vec![
            Line::from(Span::styled(
                format!(
                    "sp2 {}  {}  {}",
                    env!("CARGO_PKG_VERSION"),
                    self.frame_stats.label(),
                    crate::ui::graphics::usage()
                ),
                scheme().frame_stats(),
            )),
            Line::from(
                sections
                    .iter()
                    .zip(&section_widths)
                    .map(|(section, &width)| {
                        Span::styled(format!("{:<width$}", section.title), heading)
                    })
                    .collect::<Vec<_>>(),
            ),
        ];
        let shown_rows = rows.min(visible_rows);
        lines.extend((0..shown_rows).map(|row| {
            let row = self.highlight_debug_scroll + row;
            Line::from(
                sections
                    .iter()
                    .zip(&entry_widths)
                    .zip(&section_widths)
                    .flat_map(|((section, &entry_width), &width)| {
                        let sub_columns = width / entry_width;
                        let mut spans = (0..sub_columns)
                            .flat_map(|column| match section.entries.get(column * rows + row) {
                                Some(entry) => entry.spans(entry_width),
                                None => vec![Span::raw(" ".repeat(entry_width))],
                            })
                            .collect::<Vec<_>>();
                        spans.push(Span::raw(" ".repeat(width % entry_width)));
                        spans
                    })
                    .collect::<Vec<_>>(),
            )
        }));
        let width = (content_width + 2).min(usize::from(available.width)) as u16;
        let height = lines.len() as u16 + 2;
        let area = Rect::new(
            available.right() - width,
            available.bottom() - height,
            width,
            height,
        );
        let base = scheme().transcript().patch(scheme().value());
        let block = TuiBlock::bordered()
            .style(base)
            .border_style(scheme().highlight_inspector_border())
            .title(Span::styled(title, scheme().highlight_inspector_title()));
        let block = if rows > visible_rows {
            let first = self.highlight_debug_scroll + 1;
            let last = self.highlight_debug_scroll + shown_rows;
            block.title_bottom(Span::styled(
                format!(" ↑↓ {first}–{last} of {rows} "),
                scheme().highlight_inspector_title(),
            ))
        } else {
            block
        };
        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(lines).style(base).block(block), area);
        self.selection_inclusions.cover(area);
    }

    /// Hand the status line the keys the session's state puts on its right, if
    /// the state calls for any.
    fn refresh_status_line(&mut self) {
        self.status_line.state_keys = if self.cancel_hint_shown {
            Some(vec![
                TextSpan::new(
                    format!(
                        "{} ",
                        self.keymaps
                            .key_for(Action::CancelExecution)
                            .unwrap_or_default()
                    ),
                    HLGroup::KeymapHint,
                ),
                TextSpan::new("to cancel", HLGroup::StatuslineHintLabel),
            ])
        } else if let Some(controller) = self.pickers.last() {
            Some(controller.hints())
        } else if self.continuation.is_some() && self.input.text().is_empty() {
            Some(vec![
                TextSpan::new(
                    format!("{} ", KeyCode::Up + KeyMod::Ctrl),
                    HLGroup::KeymapHint,
                ),
                TextSpan::new("continue", HLGroup::StatuslineHintLabel),
            ])
        } else {
            None
        };
    }

    /// The key hints segment of the idle line; typing leaves no use for the
    /// commands picker's key.
    fn idle_hints(&self) -> Vec<TextSpan> {
        if self.feed.has_selection() {
            return [("↑↓ ", "move · "), ("y ", "yank · "), ("Esc ", "deselect")]
                .into_iter()
                .flat_map(|(key, label)| {
                    [
                        TextSpan::new(key, HLGroup::KeymapHint),
                        TextSpan::new(label, HLGroup::StatuslineHintLabel),
                    ]
                })
                .collect();
        }
        let settings_key = self
            .keymaps
            .key_for(Action::OpenSettings)
            .unwrap_or_default();
        let settings = [
            TextSpan::new(format!("{settings_key} "), HLGroup::KeymapHint),
            TextSpan::new("settings", HLGroup::StatuslineHintLabel),
        ];
        match self.input.text().is_empty() {
            true => {
                let commands_key = self
                    .keymaps
                    .key_for(Action::OpenCommands)
                    .unwrap_or_default();
                let mut hints = vec![
                    TextSpan::new(format!("{commands_key} "), HLGroup::KeymapHint),
                    TextSpan::new("commands · ", HLGroup::StatuslineHintLabel),
                ];
                hints.extend(settings);
                hints
            }
            false => settings.into(),
        }
    }

    /// Memory, pictures and frame rate, read only while the debug component is placed.
    fn status_diagnostics(&self) -> StatusDiagnostics {
        if self.settings.statusline.placement(StatusComponent::Debug) == StatusSegment::Hidden {
            return StatusDiagnostics::default();
        }
        StatusDiagnostics {
            sp2_memory: resident_bytes(None),
            m2_memory: self
                .status_runtime
                .m2_process
                .and_then(|id| resident_bytes(Some(id))),
            pictures: crate::ui::graphics::usage(),
            frames_per_second: self.frame_stats.average().round() as u32,
        }
    }

    /// What the status line draws from in any view of the session.
    fn status_context(&self) -> StatusLineContext {
        StatusLineContext {
            layout: self.settings.statusline,
            location: Some(self.location()),
            hints: self.idle_hints(),
            state: self.shell_state(),
            math_renderer: if self.settings.rendering.maths.available() {
                self.settings.rendering.maths.label().to_string()
            } else {
                format!(
                    "{} ({} unavailable)",
                    MathDisplay::Unicode.label(),
                    self.settings.rendering.maths.label()
                )
            },
            runtime: StatusRuntime {
                language_server: if self.settings.rendering.language_server {
                    self.status_runtime.language_server.clone()
                } else {
                    LanguageServerState::Off
                },
                ..self.status_runtime.clone()
            },
            diagnostics: self.status_diagnostics(),
            ..StatusLineContext::default()
        }
    }

    /// Shared rendering preferences for transcript and reference content.
    fn render_context(&self, width: u16) -> RenderContext {
        RenderContext::new(width, 0)
            .with_fractions(self.settings.rendering.fractions)
            .with_blackboard(self.settings.rendering.math_fonts)
            .with_matrix_row_spacing(self.settings.rendering.matrix_row_spacing_percent)
            .with_display(self.settings.rendering.maths)
            .with_ink(self.settings.rendering.math_ink)
            .with_script_colors(
                self.settings.rendering.index_color,
                self.settings.rendering.exponent_color,
            )
            .with_original_text(self.settings.rendering.original_output)
            .with_recent_outputs(self.feed.recent_outputs())
            .with_output_limit(self.settings.layout.output_rows)
    }

    /// Tell the pictures how far their blocks stand from the viewport, for both feeds
    /// whether this frame draws them or not: a picture belongs to its block still.
    pub fn report_picture_distances(&self) {
        crate::ui::graphics::report_distances(
            self.feed
                .picture_distances()
                .into_iter()
                .chain(self.reference.picture_distances()),
        );
    }

    pub fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        highlighter: &mut dyn Highlighter,
        in_debugger: bool,
    ) {
        if crate::ui::graphics::refresh_cell_size() {
            self.feed.on_event(&Reformatted);
            self.reference.on_event(&Reformatted);
        }
        self.report_picture_distances();
        if self.pane_focused {
            self.frame_stats.tick();
        }
        self.selection_inclusions.clear();
        let theme = *scheme();
        let input_prefix = UI.prompt_width();
        frame.render_widget(Clear, area);

        let editor_area = area;
        self.refresh_completion();

        let input_height = self
            .input
            .height(editor_area.width, input_prefix, editor_area.height);
        // A split shares one status line across both panes, drawn by the app.
        let status_height = u16::from(self.statusline_shown() && !self.split);
        let picker_height = if self.pane_focused {
            self.pickers.last().map_or_else(
                || {
                    self.input
                        .offered()
                        .map_or(0, |completion| completion.list_height())
                },
                |controller| controller.picker.height(editor_area.width),
            )
        } else {
            0
        };
        // A long list must never squeeze the input out of the frame: the cursor
        // would then be placed outside it.
        let picker_height = picker_height.min(
            editor_area
                .height
                .saturating_sub(input_height + status_height),
        );
        let feed_height = editor_area
            .height
            .saturating_sub(input_height + picker_height + status_height);
        let constraints = if self.settings.statusline.position == StatusLinePosition::AboveInput {
            [
                Constraint::Min(feed_height),
                Constraint::Length(picker_height),
                Constraint::Length(status_height),
                Constraint::Length(input_height),
            ]
        } else {
            [
                Constraint::Min(feed_height),
                Constraint::Length(picker_height),
                Constraint::Length(input_height),
                Constraint::Length(status_height),
            ]
        };
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(editor_area);
        let (input_area, status_area) =
            if self.settings.statusline.position == StatusLinePosition::AboveInput {
                (sections[3], sections[2])
            } else {
                (sections[2], sections[3])
            };
        let context = self.render_context(editor_area.width);
        let input_context = RenderContext::new(editor_area.width, input_prefix);

        self.feed_area = sections[0];
        self.draw_feed(frame, sections[0], &context);
        if let Some(controller) = self.pickers.last().filter(|_| self.pane_focused) {
            controller
                .picker
                .render(&(), sections[1], frame.buffer_mut());
        } else if let Some(completion) = self
            .input
            .offered()
            .filter(|completion| self.pane_focused && completion.list_height() > 0)
        {
            completion.render_list(sections[1], frame.buffer_mut(), &theme);
        }
        self.refresh_completion();
        let pending = self.feed.incomplete_source();
        let command = pending.is_none() && commands::typed(self.input.text());
        if let Some(cursor) = self.input.render(
            InputContext {
                render: &input_context,
                highlighter,
                command,
                debugging: in_debugger,
                focused: self.pane_focused,
                pending,
            },
            input_area,
            frame.buffer_mut(),
        ) {
            frame.set_cursor_position(cursor);
            self.input_cursor = Some(cursor);
        }
        if let Some(floating) = &self.floating {
            floating.draw(frame.buffer_mut(), area, self.input_cursor);
        }
        TuiBlock::default()
            .style(theme.statusline_bg())
            .render(status_area, frame.buffer_mut());
        self.status_line
            .render(&self.status_context(), status_area, frame.buffer_mut());
        self.draw_highlight_debug(frame);
        self.capture_screen(frame);
        self.paint_screen_selection(frame);
    }

    /// Whether the settings give the status line a row.
    pub fn statusline_shown(&self) -> bool {
        self.settings.statusline.position != StatusLinePosition::Off
    }

    /// The colour scheme in use and the directory the session runs in.
    fn location(&self) -> StatusLocation {
        StatusLocation {
            scheme: self.settings.appearance.scheme().label().to_string(),
            directory: ENV
                .current_dir()
                .map(|directory| abbreviate_home(&directory))
                .unwrap_or_default(),
        }
    }

    /// What the session is doing, as the status line's left side says it, when
    /// it is doing anything.
    fn shell_state(&self) -> Option<TextSpan> {
        if let Some((_, copied)) = &self.copied {
            Some(TextSpan::new(copied.clone(), HLGroup::StatuslineState))
        } else if self.processing {
            Some(TextSpan::new("Processing", HLGroup::StatuslineWarning))
        } else {
            self.continuation.as_ref().map(|reason| {
                TextSpan::new(format!("Waiting for {reason}"), HLGroup::StatuslineState)
            })
        }
    }

    fn paint_link_hover(&self, frame: &mut Frame, link: Option<Link>) {
        if let Some(link) = link {
            let hovered = scheme().style(if self.control_held {
                HLGroup::MarkupLinkOpen
            } else {
                HLGroup::MarkupLinkHover
            });
            let text = scheme().style(HLGroup::Text).fg;
            for cells in link.cells {
                frame.buffer_mut().set_style(cells, hovered);
                for position in cells.positions() {
                    let cell = &mut frame.buffer_mut()[position];
                    if let (Some(text), Color::Rgb(..)) = (text, cell.fg) {
                        if let Ok(brightened) = blend(cell.fg, text, LINK_HOVER_BRIGHTENING) {
                            cell.set_fg(brightened);
                        }
                    }
                }
            }
        }
    }

    fn draw_feed(&mut self, frame: &mut Frame, area: Rect, context: &RenderContext) -> bool {
        if area.width == 0 || area.height == 0 {
            self.feed_top = 0;
            return false;
        }
        let (underfilled, inclusions) = self.feed.render(
            context,
            area,
            frame.buffer_mut(),
            self.settings
                .appearance
                .canvas(context.groups().transcript()),
        );
        self.feed_top = -(self.feed.offset() as i64);
        self.selection_inclusions.merge(inclusions);
        self.paint_link_hover(
            frame,
            self.pointer.and_then(|pointer| self.feed.link_at(pointer)),
        );
        underfilled
    }
}

#[cfg(test)]
mod mouse_tests {
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::buffer::Buffer;
    use ratatui::layout::{Position, Rect};
    use ratatui::style::Style;

    use super::{Editor, Reaction};
    use crate::backend::M2Value;
    use crate::ui::cell::OutputBlock;
    use crate::ui::feed::Feed;
    use crate::ui::render::RenderContext;

    const AREA: Rect = Rect::new(0, 0, 60, 10);

    /// An editor showing one documentation link, and a position on it.
    fn editor_with_link() -> (Editor, Position) {
        let mut editor = Editor::new(String::new());
        editor.feed = Feed::new(Box::new(OutputBlock::rendered(
            M2Value::Markup {
                html: "<p>See <a href='&quot;isPrime&quot;'>isPrime</a>.</p>".into(),
                fallback: "See isPrime.".into(),
                example_width: 60,
            },
            None,
        )));
        editor.feed.render(
            &RenderContext::new(AREA.width, 0),
            AREA,
            &mut Buffer::empty(AREA),
            Style::default(),
        );
        let pointer = (0..AREA.height)
            .flat_map(|row| (0..AREA.width).map(move |column| Position::new(column, row)))
            .find(|&position| editor.feed.link_at(position).is_some())
            .expect("the documentation link is visible");
        (editor, pointer)
    }

    #[test]
    fn standalone_folded_output_expands_on_a_double_click() {
        let mut editor = Editor::new(String::new());
        let area = Rect::new(0, 0, 80, 16);
        let context = RenderContext::new(area.width, 0).with_output_limit(8);
        let source = (0..context.output_limit().unwrap() + 20)
            .map(|row| format!("standalone row {row}"))
            .collect::<Vec<_>>()
            .join("\n");
        editor.feed = Feed::new(Box::new(OutputBlock::value(source, None)));
        let draw = |editor: &mut Editor| {
            let mut buffer = Buffer::empty(area);
            editor
                .feed
                .render(&context, area, &mut buffer, Style::default());
            (0..area.height)
                .map(|y| {
                    (0..area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
        };
        let folded = draw(&mut editor);
        let row = folded
            .iter()
            .position(|row| row.contains("rows hidden"))
            .expect("output starts folded");
        let press = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: row as u16,
            modifiers: KeyModifiers::NONE,
        };
        editor.handle_mouse(press);
        editor.handle_mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            ..press
        });
        draw(&mut editor);
        editor.handle_mouse(press);
        let expanded = draw(&mut editor);
        assert!(
            !expanded.iter().any(|row| row.contains("rows hidden")),
            "{expanded:?}"
        );
        assert!(expanded.iter().any(|row| row.contains("standalone row")));
    }

    #[test]
    fn link_clicks_choose_the_feed_or_reference_pane() {
        let (mut editor, pointer) = editor_with_link();
        let click = |modifiers| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: pointer.x,
            row: pointer.y,
            modifiers,
        };

        assert!(matches!(
            editor.handle_mouse(click(KeyModifiers::NONE)),
            Reaction::Continue
        ));
        assert!(matches!(
            editor.handle_mouse(click(KeyModifiers::CONTROL)),
            Reaction::DocumentationInFeed(command) if command == "\"isPrime\""
        ));
        assert!(matches!(
            editor.handle_mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Middle),
                ..click(KeyModifiers::NONE)
            }),
            Reaction::Documentation(command) if command == "\"isPrime\""
        ));
    }
}

#[cfg(test)]
mod processing_tests {
    use super::*;
    use ratatui::buffer::Buffer;

    #[test]
    fn cancel_hint_is_delayed_uses_the_binding_and_clears_when_idle() {
        let mut editor = Editor::new(String::new());
        let text = |editor: &Editor| {
            let area = Rect::new(0, 0, 80, 1);
            let mut buffer = Buffer::empty(area);
            editor
                .status_line
                .render(&StatusLineContext::default(), area, &mut buffer);
            (0..80).map(|x| buffer[(x, 0)].symbol()).collect::<String>()
        };
        editor.set_processing(true);
        editor.animate();
        assert!(!text(&editor).contains("to cancel"));
        editor.processing_started = Some(Instant::now() - Duration::from_secs(3));
        editor.animate();
        assert!(text(&editor).contains("to cancel"));
        assert!(text(&editor).contains(&editor.keymaps.key_for(Action::CancelExecution).unwrap()));
        assert!(editor
            .keymaps
            .rebind(Action::CancelExecution, KeyEvent::new(KeyCode::F(24))));
        editor.refresh_status_line();
        assert!(text(&editor).contains(&editor.keymaps.key_for(Action::CancelExecution).unwrap()));
        editor.set_processing(false);
        assert!(!text(&editor).contains("to cancel"));
        editor.set_processing(true);
        editor.animate();
        assert!(!text(&editor).contains("to cancel"));
    }
}

#[cfg(test)]
mod picker_tests {
    use crossterm::event::KeyCode;

    use super::{Editor, PickerMode};
    use crate::ui::action::Action;

    #[test]
    fn a_setting_closes_every_picker_once_chosen_after_its_typed_command_ran() {
        let mut editor = Editor::new(String::new());
        for character in "/setting".chars() {
            editor.handle_input(KeyCode::Char(character).into());
        }
        assert_eq!(
            editor.pickers.last().map(|controller| controller.mode),
            Some(PickerMode::FilteredCommands)
        );
        editor.handle_action(Action::Submit);
        assert_eq!(
            editor.pickers.last().map(|controller| controller.mode),
            Some(PickerMode::Settings)
        );
        editor.handle_action(Action::Submit);
        assert!(editor.pickers.is_empty());
    }
}

#[cfg(test)]
mod saved_setup_tests {
    use super::{Action, Editor, Reaction};

    #[test]
    fn saving_selected_setup_preserves_the_live_draft() {
        let mut editor = Editor::new(String::new());
        editor.paste("setup = 7;\nunfinished");
        editor.input.field_mut().start_selection(0);
        editor
            .input
            .field_mut()
            .continue_selection("setup = 7;".len());
        let reaction = editor.handle_action(Action::SaveSelection);
        assert!(
            matches!(reaction, Reaction::SaveSelection(path, source) if path.is_empty() && source == "setup = 7;")
        );
        assert_eq!(editor.submission(), "setup = 7;\nunfinished");
    }
}

#[cfg(test)]
mod unified_settings_tests {
    use super::*;
    use crate::ui::settings::keymap::settings_tab;

    #[test]
    fn keymaps_open_within_settings_capture_shortcuts_and_keep_the_tab_after_rebinding() {
        let mut editor = Editor::new(String::new());
        editor.handle_action(Action::OpenSettings);
        editor.handle_action(Action::OpenKeymaps);
        assert_eq!(editor.pickers.len(), 1);
        assert_eq!(editor.pickers[0].mode, PickerMode::Settings);
        assert_eq!(editor.pickers[0].active_mode(), PickerMode::Keymaps);
        let shortcut = Shortcut::Command(Action::ClearFeed);
        let tab = settings_tab(&editor.keymaps);
        let row = tab
            .items
            .iter()
            .position(|item| item.cells()[0].to_string() == shortcut.description())
            .unwrap();
        editor.pickers[0].picker.select(row);
        let replacement = editor.keymaps.key(Action::OpenSettings).unwrap();
        editor.handle_action(Action::Submit);
        assert!(editor.pickers[0].picker.capturing());
        assert_eq!(editor.resolve_action(replacement), None);
        editor.handle_input(replacement);
        assert_eq!(editor.keymaps.key(shortcut), Some(replacement));
        assert_eq!(editor.pickers[0].active_mode(), PickerMode::Keymaps);
        assert!(!editor.pickers[0].picker.capturing());
        editor.handle_input(KeyCode::Tab.into());
        assert_eq!(editor.pickers[0].active_mode(), PickerMode::Settings);
        editor.handle_action(Action::OpenKeymaps);
        editor.settings.rendering.math_fonts = !editor.settings.rendering.math_fonts;
        let rendering = editor.settings.rendering;
        editor.handle_picker_event(EditorEvent::RestoreKeymaps);
        assert_eq!(editor.settings.rendering, rendering);
        assert_eq!(editor.pickers[0].active_mode(), PickerMode::Keymaps);
    }

    #[test]
    fn escape_cancels_key_capture_without_closing_the_settings_tabs() {
        let mut editor = Editor::new(String::new());
        editor.handle_action(Action::OpenKeymaps);
        editor.handle_action(Action::Submit);
        assert!(editor.pickers[0].picker.capturing());
        editor.handle_input(KeyCode::Esc.into());
        assert_eq!(editor.pickers.len(), 1);
        assert!(!editor.pickers[0].picker.capturing());
        assert_eq!(editor.pickers[0].active_mode(), PickerMode::Keymaps);
    }
}

#[cfg(test)]
mod documentation_rendering_tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::style::Style;
    use std::cell::Cell;
    use std::rc::Rc;

    struct CountedBlock(Rc<Cell<usize>>);

    impl Block for CountedBlock {
        fn height(&self, _: &RenderContext) -> u16 {
            1
        }
        fn render_local(&self, _: &RenderContext, _: &mut Buffer) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn completed_examples_and_semantic_tokens_repaint_both_cached_panes() {
        let mut editor = Editor::new(String::new());
        let transcript = Rc::new(Cell::new(0));
        let reference = Rc::new(Cell::new(0));
        editor.feed = Feed::new(Box::new(CountedBlock(transcript.clone())));
        editor.set_reference(vec![Box::new(CountedBlock(reference.clone()))]);
        let area = Rect::new(0, 0, 60, 8);
        let context = RenderContext::new(area.width, 0);
        let render = |editor: &mut Editor| {
            editor
                .feed
                .render(&context, area, &mut Buffer::empty(area), Style::default());
            editor
                .reference
                .render(&context, area, &mut Buffer::empty(area), Style::default());
        };
        render(&mut editor);
        let before = (transcript.get(), reference.get());
        render(&mut editor);
        assert_eq!((transcript.get(), reference.get()), before);
        editor.apply(EditorUpdate::ExamplesComputed);
        render(&mut editor);
        assert!(transcript.get() > before.0);
        assert!(reference.get() > before.1);
    }
}

#[cfg(test)]
mod language_server_tests {
    use ratatui::backend::TestBackend;
    use ratatui::text::Line;
    use ratatui::Terminal;

    use super::*;
    use crate::code::highlight::SyntaxHighlighter;
    use crate::ui::floating::FloatingContent;

    const PANEL_TEXT: &str = "floating words";

    struct Words;

    impl FloatingContent for Words {
        fn lines(&self, _width: u16) -> Vec<Line<'static>> {
            vec![Line::from(PANEL_TEXT)]
        }
    }

    /// The symbol of each cell drawn, row by row.
    fn cells(editor: &mut Editor) -> Vec<Vec<String>> {
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).expect("test terminal");
        terminal
            .draw(|frame| editor.draw(frame, frame.area(), &mut SyntaxHighlighter::new(), false))
            .expect("drawing");
        let buffer = terminal.backend().buffer();
        (0..20)
            .map(|y| {
                (0..60)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect()
    }

    fn screen(editor: &mut Editor) -> String {
        cells(editor)
            .iter()
            .map(|row| row.concat() + "\n")
            .collect()
    }

    #[test]
    fn the_name_at_the_cursor_is_looked_up_where_the_cursor_is() {
        let mut editor = Editor::new(String::new());
        editor.paste("ideal x");
        editor.input.field_mut().place_cursor(2);
        for (action, lookup) in [
            (Action::Describe, Lookup::Brief),
            (Action::DescribeBeside, Lookup::Description),
            (Action::GoToDefinition, Lookup::Definition),
        ] {
            let Reaction::Lookup(asked, point) = editor.handle_action(action) else {
                panic!("{action:?} asks the language server");
            };
            assert_eq!(asked, lookup);
            assert_eq!(point.source, None);
            assert_eq!(point.offset, 2);
        }
    }

    #[test]
    fn a_floating_panel_shows_until_the_next_key() {
        let mut editor = Editor::new(String::new());
        editor.paste("ideal x");
        editor.apply(EditorUpdate::Float(Floating::new(
            Words,
            Anchor::InputCursor,
        )));
        assert!(screen(&mut editor).contains(PANEL_TEXT));
        editor.handle_input(KeyCode::Left.into());
        assert!(!screen(&mut editor).contains(PANEL_TEXT));
    }

    #[test]
    fn a_middle_click_on_the_input_looks_up_the_name_under_it() {
        let mut editor = Editor::new(String::new());
        editor.paste("ideal x");
        let shown = cells(&mut editor);
        let row = shown
            .iter()
            .position(|row| row.concat().contains("ideal x"))
            .unwrap();
        let column = shown[row].iter().position(|cell| cell == "x").unwrap();
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Middle),
            column: column as u16,
            row: row as u16,
            modifiers: KeyModifiers::NONE,
        };
        let Reaction::Lookup(Lookup::Brief, point) = editor.handle_mouse(click) else {
            panic!("a middle click on code asks what it is");
        };
        assert_eq!(point.offset, "ideal ".len());
        assert_eq!(
            point.anchor,
            Anchor::At(Position::new(column as u16, row as u16))
        );
    }

    #[test]
    fn formatting_replaces_the_input_only_while_it_reads_as_it_was_sent() {
        let mut editor = Editor::new(String::new());
        editor.paste("f=x->x");
        let Reaction::Format(sent) = editor.handle_action(Action::FormatInput) else {
            panic!("formatting asks the language server");
        };
        editor.apply(EditorUpdate::InputRewritten {
            was: sent.clone(),
            now: "f = x -> x".into(),
        });
        assert_eq!(editor.submission(), "f = x -> x");
        editor.apply(EditorUpdate::InputRewritten {
            was: sent,
            now: "stale".into(),
        });
        assert_eq!(editor.submission(), "f = x -> x");
    }
}
