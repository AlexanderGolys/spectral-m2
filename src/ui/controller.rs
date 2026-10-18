//! Interactive editor: submitted feed, live input and history, navigation,
//! scrolling, and rendering.

pub mod commands;
mod completion;
mod picker;
mod screen;
pub mod settings;
mod view;

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as TuiBlock, Clear, Paragraph, Widget};
use ratatui::Frame;
use these_macros_should_be_illegal::discriminated_str;

use self::commands::Command;
use self::completion::{Names, Request};
use self::picker::{Picker, PickerController, PickerInput, PickerItem, PickerMode, Row};
use self::screen::{ScreenCapture, ScreenSelection};
use self::settings::keymap::Keymaps;
use self::settings::theme::groups::{publish, scheme, HLGroup};
use self::settings::theme::Appearance;
use self::settings::{Persisted, SavedSettings, Settings, StatusLinePosition};
pub use self::view::{CommittedCells, ViewKind, Viewport};
use super::action::Action;
use super::block::{Block, Element, HLSpan, Reformatted, Rethemed, TextSpan, Ticked};
use super::cell::{CellState, EvaluationBlock, IncompleteBlock, OutputBlock, TextBlock};
use super::config::UI;
use super::debug::FrameStats;
use super::element::EventListener;
use super::feed::Feed;
use super::key::{KeyEvent, KeyMod};
use super::render::selection::Exclusions;
use super::render::RenderContext;
use super::statusline::{StatusLine, StatusLineContext, StatusLineEvent};
use crate::code::highlight::Highlighter;
use crate::input::{InputCode, InputContext, InputSetting, InputSettings};

/// An outward effect produced while handling input or an [`Action`].
pub enum Reaction {
    Continue,
    Submit,
    ResumeIncomplete,
    /// Run source beside the input, leaving what is typed there alone.
    Post(String),
    Restart,
    Quit,
    /// Put the current selection on the system clipboard.
    Copy(String),
    Open(ViewKind),
    ClosePane,
    /// Write the session's code to the file typed, or where the session is kept
    /// on exit when none was.
    Save(Option<String>),
    /// Run the code in the file typed.
    Load(String),
    /// Run the code the previous session left.
    Resume,
}

/// The choice `delta` steps after `current`, wrapping around `choices`.
pub fn cycled<T: Copy + PartialEq>(choices: &[T], current: T, delta: i8) -> T {
    let position = choices
        .iter()
        .position(|choice| *choice == current)
        .expect("the current choice is among the choices");
    choices[(position as isize + isize::from(delta)).rem_euclid(choices.len() as isize) as usize]
}

/// A session result to apply to the editor model.
pub enum EditorUpdate {
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
    /// Every name a freshly started M2 knows, replacing those of any before it.
    KnownNames(Vec<String>),
    /// Names the session bound since M2 last reported.
    BoundNames(Vec<String>),
    /// Documentation examples were computed again, so the pages showing them
    /// draw them differently.
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
    ResumeTyping(char),
    ClosePickers,
    RunCommand(Command),
    ClosePicker,
    RestoreDefaults,
    SetSettings(Settings),
    SetInput(InputSettings),
    SetAppearance(Appearance),
    Rebind(Action, KeyEvent),
}

/// One independently cyclic axis of a colour appearance.
#[discriminated_str(label)]
#[derive(Clone, Copy)]
enum AppearanceField {
    Theme = "theme",
    Contrast = "contrast",
    Mode = "mode",
    Background = "background",
}

impl AppearanceField {
    fn value(self, appearance: Appearance) -> String {
        match self {
            Self::Theme => appearance.scheme().label().to_string(),
            Self::Contrast => appearance.variant().label().to_string(),
            Self::Mode => appearance.mode().to_string(),
            Self::Background => appearance.background_label().to_string(),
        }
    }

    fn adjust(self, appearance: Appearance, delta: i8) -> Appearance {
        match self {
            Self::Theme => appearance.cycle_theme(delta),
            Self::Contrast => appearance.cycle_contrast(delta),
            Self::Mode => appearance.cycle_mode(delta),
            Self::Background => appearance.cycle_background(),
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::Theme => "colour scheme family",
            Self::Contrast => "the theme's variants for the current mode",
            Self::Mode => "dark or light palette",
            Self::Background => "paint empty canvas or let the terminal show through",
        }
    }
}

const SHELL_ITEMS: &[ShellItem] = &[
    ShellItem::Appearance(AppearanceField::Theme),
    ShellItem::Appearance(AppearanceField::Contrast),
    ShellItem::Appearance(AppearanceField::Mode),
    ShellItem::Appearance(AppearanceField::Background),
    ShellItem::Input(InputSetting::MaxHeight),
    ShellItem::Input(InputSetting::Scrolloff),
    ShellItem::Input(InputSetting::CompleteAfter),
    ShellItem::Statusline,
    ShellItem::Fractions,
    ShellItem::MathFonts,
    ShellItem::OutputRows,
    ShellItem::ExampleTimeout,
    ShellItem::RestoreDefault,
];

/// One item listed in the settings picker.
#[derive(Clone, Copy)]
enum ShellItem {
    Appearance(AppearanceField),
    Input(InputSetting),
    Statusline,
    Fractions,
    MathFonts,
    OutputRows,
    ExampleTimeout,
    RestoreDefault,
}

impl ShellItem {
    /// The setting's name, leading its row.
    const fn label(self) -> &'static str {
        match self {
            Self::Appearance(field) => field.label(),
            Self::Input(field) => field.label(),
            Self::Statusline => "statusline",
            Self::Fractions => "fractions",
            Self::MathFonts => "math fonts",
            Self::OutputRows => "max output height",
            Self::ExampleTimeout => "compute examples",
            Self::RestoreDefault => "restore default",
        }
    }

    /// What the setting controls, shown after its value.
    const fn description(self) -> &'static str {
        match self {
            Self::Appearance(field) => field.description(),
            Self::Input(field) => field.description(),
            Self::Statusline => "where the status line sits",
            Self::Fractions => "how rational values are drawn",
            Self::MathFonts => "use TeX math fonts like mathbb, mathcal",
            Self::OutputRows => "rows a value shows before its middle folds away",
            Self::ExampleTimeout => {
                "run documentation examples again, each for at most this long, to draw their values"
            }
            Self::RestoreDefault => "reset settings, keymaps and appearance",
        }
    }
}

/// The editor child that receives an otherwise unclaimed input event.
#[derive(Clone, Copy)]
enum ElementFocus {
    /// The editable input field.
    Input,
    /// The topmost picker and its keyboard policy.
    Picker(PickerMode),
}

/// The transcript feed, live input, and viewport state.
pub struct Editor {
    feed: Feed,
    input: InputCode,
    settings: Settings,
    keymaps: Keymaps,
    appearance: Appearance,
    status_line: StatusLine,
    continuation: Option<String>,
    processing: bool,
    /// The names M2 knows, which the input completes.
    names: Names,
    pickers: Vec<PickerController>,
    feed_area: Rect,
    feed_top: i64,
    screen_selecting: bool,
    screen_selection: Option<ScreenSelection>,
    /// What each pane drew last, to select and copy from.
    captures: Vec<ScreenCapture>,
    selection_exclusions: Exclusions,
    split: bool,
    previewing: bool,
    pane_focused: bool,
    highlight_debug: bool,
    highlight_debug_scroll: usize,
    frame_stats: FrameStats,
    /// When and where the previous left click landed, to recognise a double click.
    last_click: Option<(Instant, Position)>,
    /// Where the mouse last was, so the link under it is underlined.
    pointer: Option<Position>,
}

impl Editor {
    pub fn new(welcome: String) -> Self {
        let appearance = Appearance::load();
        publish(appearance.groups());
        let saved = SavedSettings::load();
        let mut input = InputCode::new();
        input.set_settings(saved.input);
        let mut editor = Self {
            feed: Feed::new(Box::new(TextBlock::system(welcome))),
            input,
            settings: saved.editor,
            keymaps: Keymaps::replaying(&saved.rebinds),
            appearance,
            status_line: StatusLine::default(),
            continuation: None,
            processing: false,
            names: Names::default(),
            pickers: Vec::new(),
            feed_area: Rect::ZERO,
            feed_top: 0,
            screen_selecting: false,
            screen_selection: None,
            captures: Vec::new(),
            selection_exclusions: Exclusions::default(),
            split: false,
            previewing: false,
            pane_focused: true,
            highlight_debug: false,
            highlight_debug_scroll: 0,
            frame_stats: FrameStats::default(),
            last_click: None,
            pointer: None,
        };
        editor.refresh_status_line();
        editor
    }

    /// Reflect whether this editor's backend currently owns unfinished work.
    pub fn set_processing(&mut self, processing: bool) {
        if self.processing != processing {
            self.processing = processing;
            self.refresh_status_line();
        }
    }

    /// Redraw whatever moves with the clock while the backend works; false when
    /// nothing does, so an idle shell never repaints on its own.
    pub fn animate(&mut self) -> bool {
        if self.processing {
            self.feed.on_event(&Ticked);
        }
        self.processing
    }

    /// How much longer the input shows the typing prompt, if it does.
    pub fn typing_remaining(&self) -> Option<Duration> {
        self.input.typing_remaining()
    }

    /// Return the input to its resting prompt once typing has paused; whether it did.
    pub fn settle_typing(&mut self) -> bool {
        self.input.settle_typing()
    }

    /// How long each documentation example may take to be computed again.
    pub fn example_timeout(&self) -> Duration {
        self.settings.example_timeout()
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
        match update {
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
            EditorUpdate::KnownNames(names) => {
                self.names.reset(names);
                self.input.forget_completion();
            }
            EditorUpdate::BoundNames(names) => {
                self.names.bind(names);
                self.input.forget_completion();
            }
            EditorUpdate::ExamplesComputed => self.feed.on_event(&Reformatted),
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
                self.feed.update_evaluation(id, |evaluation| {
                    evaluation.succeed(index, number, state, outputs, printed, value)
                });
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

    fn push_evaluation(&mut self, block: EvaluationBlock) {
        self.feed.push_evaluation(block);
    }

    /// Move the feed's unfinished tail back into the live editor.
    pub fn take_incomplete(&mut self) -> Option<String> {
        let source = self.feed.take_incomplete()?.into_source();
        self.continuation = None;
        self.input.restore(source.clone());
        self.refresh_status_line();
        Some(source)
    }

    fn push_block(&mut self, block: Box<dyn Block>) {
        self.feed.push(block);
    }

    pub fn paste(&mut self, text: &str) {
        self.input.insert(text);
        self.refresh_status_line();
    }

    /// Longest pause between two clicks on one cell that still reads as a double click.
    const DOUBLE_CLICK: Duration = Duration::from_millis(400);

    /// Route the wheel by what it is pointing at: over the input pane it resizes
    /// the pane, everywhere else it scrolls the feed. A double click on a folded
    /// value expands it, and on an expanded one folds it back.
    pub fn handle_mouse(&mut self, event: MouseEvent) -> Reaction {
        let pointer = Position::new(event.column, event.row);
        self.pointer = Some(pointer);
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
            && event.modifiers.contains(KeyModifiers::CONTROL)
        {
            if let Some(link) = self.feed.link_at(pointer) {
                self.screen_selection = None;
                return Reaction::Post(format!("help {}", link.target));
            }
        }
        if self.double_clicked(event)
            && self
                .feed
                .toggle_fold(Position::new(event.column, event.row))
        {
            self.last_click = None;
            self.screen_selection = None;
            return Reaction::Continue;
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
                    self.input.adjust(InputSetting::MaxHeight, delta);
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

    /// Record a left press, reporting whether it repeats the previous one in time and place.
    fn double_clicked(&mut self, event: MouseEvent) -> bool {
        let MouseEventKind::Down(MouseButton::Left) = event.kind else {
            return false;
        };
        let click = (Instant::now(), Position::new(event.column, event.row));
        self.last_click
            .replace(click)
            .is_some_and(|(at, position)| {
                position == click.1 && click.0.duration_since(at) <= Self::DOUBLE_CLICK
            })
    }

    pub fn handle_preview_mouse(&mut self, event: MouseEvent) -> Reaction {
        self.handle_screen_mouse(event)
            .unwrap_or(Reaction::Continue)
    }

    /// Forward unbound shell input to its focused component.
    pub fn handle_input(&mut self, key: KeyEvent) -> Reaction {
        if matches!(self.focus(), ElementFocus::Picker(_)) {
            return self.handle_picker_key(key);
        }
        if let Some(action) = self.handle_debug_key(key) {
            return action;
        }
        if key.code == KeyCode::Up
            && key.ctrl()
            && self.input.text().is_empty()
            && self.continuation.is_some()
        {
            return Reaction::ResumeIncomplete;
        }
        self.screen_selection = None;
        self.refresh_completion();
        self.input.handle_key(key);
        if self.input.text().is_empty() {
            self.continuation = None;
        }
        if self.input.text() == commands::PREFIX {
            self.open_filtered_commands();
        }
        self.refresh_status_line();
        Reaction::Continue
    }

    /// React to a mapped command after its originating key has been discarded.
    pub fn handle_action(&mut self, action: Action) -> Reaction {
        if matches!(self.focus(), ElementFocus::Picker(_)) {
            return self.handle_picker_action(action);
        }
        if let Some(result) = self.handle_debug_action(action) {
            return result;
        }
        if action != Action::CopySelection {
            self.screen_selection = None;
        }
        if let Some(reaction) = self.perform(action, "") {
            return reaction;
        }

        if action == Action::CopySelection {
            let text = if self.previewing {
                self.selected_screen_text()
            } else {
                self.input
                    .field()
                    .selected_text()
                    .map(str::to_string)
                    .or_else(|| self.selected_screen_text())
            };
            return text.map_or(Reaction::Continue, Reaction::Copy);
        }
        if self.previewing {
            return Reaction::Continue;
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

        match action {
            Action::CopySelection => unreachable!("copy selection is handled before editing"),
            Action::Submit => unreachable!("submission is handled before component dispatch"),
            Action::ScrollFeedUp => self.feed.scroll(1),
            Action::ScrollFeedDown => self.feed.scroll(-1),
            Action::OpenCommands => self.toggle_picker(PickerMode::Commands, Self::open_commands),
            Action::ToggleHighlightDebug => unreachable!("debug toggles before input handling"),
            Action::FocusPaneLeft
            | Action::FocusPaneRight
            | Action::ResizePaneLeft
            | Action::ResizePaneRight => unreachable!("panes are handled by the app"),
            Action::OpenSettings
            | Action::OpenKeymaps
            | Action::ClearFeed
            | Action::OpenCode
            | Action::OpenValues
            | Action::OpenStdOut
            | Action::OpenOutput
            | Action::OpenSyntaxTree
            | Action::ClosePane
            | Action::Restart
            | Action::Quit
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
                self.open_from_commands(PickerMode::Keymaps, Self::keymaps_picker);
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
            Action::Restart => self.leave_pickers(Reaction::Restart),
            Action::Quit => self.leave_pickers(Reaction::Quit),
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
            Action::FocusPaneLeft
            | Action::FocusPaneRight
            | Action::ResizePaneLeft
            | Action::ResizePaneRight
            | Action::Submit
            | Action::ScrollFeedUp
            | Action::ScrollFeedDown
            | Action::OpenCommands
            | Action::ToggleHighlightDebug
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
            complete_after: usize::from(self.input.settings().complete_after),
        });
        self.input.offer(completion);
    }

    fn clear_feed(&mut self) {
        self.close_pickers();
        self.feed.clear();
        self.screen_selection = None;
    }

    fn settings_picker(&self) -> Picker {
        Picker::new("settings", self.shell_items())
    }

    fn keymaps_picker(&self) -> Picker {
        Picker::new("keymaps", self.keymap_items())
    }

    fn shell_items(&self) -> Vec<Box<dyn PickerItem>> {
        let groups = scheme();
        SHELL_ITEMS
            .iter()
            .copied()
            .map(|item| {
                let cells = |value: &str| {
                    vec![
                        Span::styled(item.label(), groups.muted()).into(),
                        Span::styled(value.to_string(), groups.value()).into(),
                        Span::styled(item.description(), groups.muted()).into(),
                    ]
                };
                match item {
                    ShellItem::Appearance(field) => {
                        let appearance = self.appearance;
                        Row::adjust(cells(&field.value(appearance)), move |delta| {
                            EditorEvent::SetAppearance(field.adjust(appearance, delta))
                        })
                    }
                    ShellItem::Input(field) => {
                        let mut settings = self.input.settings();
                        Row::adjust(cells(&field.value(settings)), move |delta| {
                            field.adjust(&mut settings, delta);
                            EditorEvent::SetInput(settings)
                        })
                    }
                    ShellItem::Statusline => {
                        let mut settings = self.settings;
                        let value = match settings.statusline {
                            StatusLinePosition::BelowInput => "below input",
                            StatusLinePosition::AboveInput => "above input",
                            StatusLinePosition::Off => "off",
                        };
                        Row::adjust(cells(value), move |delta| {
                            settings.statusline = settings.statusline.cycle(delta);
                            EditorEvent::SetSettings(settings)
                        })
                    }
                    ShellItem::Fractions => {
                        let mut settings = self.settings;
                        Row::adjust(cells(settings.fractions.label()), move |delta| {
                            settings.fractions = settings.fractions.cycle(delta);
                            EditorEvent::SetSettings(settings)
                        })
                    }
                    ShellItem::MathFonts => {
                        let mut settings = self.settings;
                        let value = if settings.math_fonts { "on" } else { "off" };
                        Row::adjust(cells(value), move |_| {
                            settings.math_fonts = !settings.math_fonts;
                            EditorEvent::SetSettings(settings)
                        })
                    }
                    ShellItem::OutputRows => {
                        let mut settings = self.settings;
                        let value = format!("{} rows", settings.output_rows);
                        Row::adjust(cells(&value), move |delta| {
                            settings.adjust_output_rows(delta);
                            EditorEvent::SetSettings(settings)
                        })
                    }
                    ShellItem::ExampleTimeout => {
                        let mut settings = self.settings;
                        let value = match settings.example_seconds {
                            0 => "off".to_string(),
                            seconds => format!("{seconds}s"),
                        };
                        Row::adjust(cells(&value), move |delta| {
                            settings.adjust_example_seconds(delta);
                            EditorEvent::SetSettings(settings)
                        })
                    }
                    ShellItem::RestoreDefault => {
                        Row::enter(cells(""), || EditorEvent::RestoreDefaults)
                    }
                }
                .into()
            })
            .collect()
    }

    fn keymap_items(&self) -> Vec<Box<dyn PickerItem>> {
        let groups = scheme();
        self.keymaps
            .remappable()
            .into_iter()
            .map(|(action, key)| {
                Row::new(
                    vec![
                        Span::styled(action.description(), groups.muted()).into(),
                        Span::styled(key.to_string(), groups.style(HLGroup::KeymapHint)).into(),
                    ],
                    move |input| match input {
                        PickerInput::Key(key) => Some(EditorEvent::Rebind(action, key)),
                        _ => None,
                    },
                )
                .into()
            })
            .collect()
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
        if mode == PickerMode::Keymaps && action == Action::OpenKeymaps {
            self.close_if_open(PickerMode::Keymaps);
            return Reaction::Continue;
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
            ElementFocus::Picker(picker.mode)
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
            EditorEvent::RunCommand(command) => return self.run_command(command),
            EditorEvent::ClosePicker => {
                self.pickers.pop();
                if matches!(
                    self.pickers.last().map(|controller| controller.mode),
                    Some(PickerMode::Settings)
                ) {
                    let items = self.shell_items();
                    self.pickers.last_mut().unwrap().picker.set_items(items);
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
                if self.settings.reformats(&Settings::default()) {
                    self.feed.on_event(&Reformatted);
                }
                self.settings = Settings::default();
                self.input.set_settings(InputSettings::default());
                self.keymaps = Keymaps::default();
                self.save_settings();
                self.apply_appearance(Appearance::default());
                let items = self.shell_items();
                self.pickers.last_mut().unwrap().picker.set_items(items);
            }
            EditorEvent::SetSettings(settings) => {
                if settings.reformats(&self.settings) {
                    self.feed.on_event(&Reformatted);
                }
                self.settings = settings;
                self.save_settings();
                let items = self.shell_items();
                self.pickers.last_mut().unwrap().picker.set_items(items);
            }
            EditorEvent::SetInput(settings) => {
                self.input.set_settings(settings);
                self.save_settings();
                let items = self.shell_items();
                self.pickers.last_mut().unwrap().picker.set_items(items);
            }
            EditorEvent::SetAppearance(appearance) => {
                self.apply_appearance(appearance);
                let items = self.shell_items();
                self.pickers.last_mut().unwrap().picker.set_items(items);
            }
            EditorEvent::Rebind(action, key) => {
                if self.keymaps.rebind(action, key) {
                    self.save_settings();
                    let items = self.keymap_items();
                    self.pickers.last_mut().unwrap().picker.set_items(items);
                }
            }
        }
        Reaction::Continue
    }

    /// Keep what the settings and keymap pickers changed for the next session.
    fn save_settings(&self) {
        SavedSettings {
            editor: self.settings,
            input: self.input.settings(),
            rebinds: self.keymaps.rebinds().to_vec(),
        }
        .save();
    }

    fn apply_appearance(&mut self, appearance: Appearance) {
        if self.appearance == appearance {
            return;
        }
        self.appearance = appearance;
        publish(appearance.groups());
        self.feed.on_event(&Rethemed);
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

    fn highlight_debug_entries(&self) -> Vec<(String, ratatui::style::Style)> {
        let heading = scheme()
            .prompt()
            .add_modifier(ratatui::style::Modifier::BOLD);
        let mut entries = vec![
            (self.frame_stats.label(), scheme().muted()),
            ("highlight groups".into(), heading),
        ];
        entries.extend(
            scheme()
                .highlight_groups()
                .into_iter()
                .map(|(name, style)| (name.into(), style)),
        );
        entries
    }

    fn draw_highlight_debug(&mut self, frame: &mut Frame) {
        let available = self.feed_area;
        if !self.highlight_debug || available.width < 4 || available.height < 5 {
            return;
        }
        let entries = self.highlight_debug_entries();
        let title = format!(
            " {} {} · debug ",
            self.appearance.scheme().label(),
            self.appearance.variant().label()
        );
        let header_width = entries[..2]
            .iter()
            .map(|(name, _)| name.chars().count())
            .chain([title.chars().count()])
            .max()
            .unwrap_or(1);
        let groups = &entries[2..];
        let rows = usize::from(available.height - 4);
        let column_width = groups
            .iter()
            .map(|(name, _)| name.chars().count() + 2)
            .max()
            .unwrap_or(1)
            .min(usize::from(available.width - 2));
        let columns = groups
            .len()
            .div_ceil(rows)
            .min((usize::from(available.width - 2) / column_width).max(1));
        let visible = rows * columns;
        self.highlight_debug_scroll = self
            .highlight_debug_scroll
            .min(groups.len().saturating_sub(visible));
        let visible_groups = groups
            .iter()
            .skip(self.highlight_debug_scroll)
            .take(visible)
            .collect::<Vec<_>>();
        let shown_rows = visible_groups.len().min(rows);
        let shown_columns = visible_groups.len().div_ceil(shown_rows.max(1));
        let content_width = (column_width * shown_columns).max(header_width);
        let width = (content_width + 2).min(usize::from(available.width)) as u16;
        let mut lines = entries[..2]
            .iter()
            .map(|(label, style)| Line::from(Span::styled(label.clone(), *style)))
            .collect::<Vec<_>>();
        lines.extend((0..shown_rows).map(|row| {
            Line::from(
                (0..shown_columns)
                    .filter_map(|column| visible_groups.get(column * shown_rows + row))
                    .map(|(label, style)| Span::styled(format!("{label:<column_width$}"), *style))
                    .collect::<Vec<_>>(),
            )
        }));
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
            .border_style(scheme().border())
            .title(Span::styled(title, scheme().prompt()));
        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(lines).style(base).block(block), area);
        self.selection_exclusions.push(area);
    }

    fn refresh_status_line(&mut self) {
        let commands_key = self
            .keymaps
            .key_for(Action::OpenCommands)
            .unwrap_or_default();
        let settings_key = self
            .keymaps
            .key_for(Action::OpenSettings)
            .unwrap_or_default();
        let event = if self.input.text().is_empty() {
            let spans = match &self.continuation {
                Some(_) => vec![
                    TextSpan::new(
                        format!("{} ", KeyCode::Up + KeyMod::Ctrl),
                        HLGroup::KeymapHint,
                    ),
                    TextSpan::new("continue", HLGroup::Muted),
                ],
                None => vec![
                    TextSpan::new(format!("{commands_key} "), HLGroup::KeymapHint),
                    TextSpan::new("commands · ", HLGroup::Muted),
                    TextSpan::new(format!("{settings_key} "), HLGroup::KeymapHint),
                    TextSpan::new("settings", HLGroup::Muted),
                ],
            };
            StatusLineEvent::Show {
                alignment: Alignment::Right,
                spans,
            }
        } else {
            StatusLineEvent::Show {
                alignment: Alignment::Right,
                spans: vec![
                    TextSpan::new(format!("{settings_key} "), HLGroup::KeymapHint),
                    TextSpan::new("settings", HLGroup::Muted),
                ],
            }
        };
        self.status_line.handle(event);
    }

    pub fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        highlighter: &mut dyn Highlighter,
        in_debugger: bool,
    ) {
        if self.pane_focused {
            self.frame_stats.tick();
        }
        self.selection_exclusions.clear();
        let theme = *scheme();
        let input_prefix = UI.prompt_width();
        frame.render_widget(Clear, area);

        let editor_area = area;

        let input_height = self
            .input
            .height(editor_area.width, input_prefix, editor_area.height);
        // A split shares one status line across both panes, drawn by the app.
        let status_height = u16::from(self.statusline_shown() && !self.split);
        let picker_height = self
            .pickers
            .last()
            .map_or(0, |controller| controller.picker.height());
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
        let constraints = if self.settings.statusline == StatusLinePosition::AboveInput {
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
            if self.settings.statusline == StatusLinePosition::AboveInput {
                (sections[3], sections[2])
            } else {
                (sections[2], sections[3])
            };
        let context = RenderContext::new(editor_area.width, 0)
            .with_fractions(self.settings.fractions)
            .with_blackboard(self.settings.math_fonts)
            .with_recent_outputs(self.feed.recent_outputs())
            .with_output_limit(self.settings.output_rows);
        let input_context = RenderContext::new(editor_area.width, input_prefix);

        self.feed_area = sections[0];
        self.draw_feed(frame, sections[0], &context);
        let code_above = picker_height == 0
            && self.settings.statusline != StatusLinePosition::AboveInput
            && self.feed.ends_in_code();
        if let Some(controller) = self.pickers.last() {
            controller
                .picker
                .render(&(), sections[1], frame.buffer_mut());
            self.selection_exclusions.push(sections[1]);
        }
        self.selection_exclusions.push(input_area);
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
                code_above,
                pending,
            },
            input_area,
            frame.buffer_mut(),
        ) {
            frame.set_cursor_position(cursor);
        }
        TuiBlock::default()
            .style(theme.statusline_bg())
            .render(status_area, frame.buffer_mut());
        self.status_line.render(
            &StatusLineContext {
                idle_left: self.shell_state(),
                ..StatusLineContext::default()
            },
            status_area,
            frame.buffer_mut(),
        );
        if status_height > 0 {
            self.selection_exclusions.push(status_area);
        }
        self.draw_highlight_debug(frame);
        self.capture_screen(frame);
        self.paint_screen_selection(frame);
    }

    /// Whether the settings give the status line a row.
    pub fn statusline_shown(&self) -> bool {
        self.settings.statusline != StatusLinePosition::Off
    }

    /// What the session is doing, as the status line's left side says it, when
    /// it is doing anything.
    fn shell_state(&self) -> Option<TextSpan> {
        if self.processing {
            Some(TextSpan::new("Processing", HLGroup::StatuslineWarning))
        } else {
            self.continuation
                .as_ref()
                .map(|reason| TextSpan::new(format!("Waiting for {reason}"), HLGroup::Muted))
        }
    }

    fn draw_feed(&mut self, frame: &mut Frame, area: Rect, context: &RenderContext) -> bool {
        if area.width == 0 || area.height == 0 {
            self.feed_top = 0;
            return false;
        }
        let (underfilled, exclusions) = self.feed.render(
            context,
            area,
            frame.buffer_mut(),
            self.appearance.canvas(context.groups().transcript()),
        );
        self.feed_top = -(self.feed.offset() as i64);
        self.selection_exclusions.extend(exclusions);
        if let Some(link) = self.pointer.and_then(|pointer| self.feed.link_at(pointer)) {
            let hovered = context.groups().style(HLGroup::MarkupLinkHover);
            for cells in link.cells {
                frame.buffer_mut().set_style(cells, hovered);
            }
        }
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

    #[test]
    fn only_control_click_opens_a_documentation_link() {
        let mut editor = Editor::new(String::new());
        editor.feed = Feed::new(Box::new(OutputBlock::rendered(
            M2Value::Markup {
                html: "<p>See <a href='&quot;isPrime&quot;'>isPrime</a>.</p>".into(),
                fallback: "See isPrime.".into(),
                example_width: 60,
            },
            None,
        )));
        let area = Rect::new(0, 0, 60, 10);
        editor.feed.render(
            &RenderContext::new(area.width, 0),
            area,
            &mut Buffer::empty(area),
            Style::default(),
        );
        let pointer = (0..area.height)
            .flat_map(|row| (0..area.width).map(move |column| Position::new(column, row)))
            .find(|&position| editor.feed.link_at(position).is_some())
            .expect("the documentation link is visible");
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
            Reaction::Post(command) if command == "help \"isPrime\""
        ));
    }
}
