//! Interactive editor: submitted feed, live input and history, navigation,
//! scrolling, and rendering.

pub mod commands;
mod picker;
pub mod settings;

use crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as TuiBlock, Clear, Paragraph, Widget};
use ratatui::Frame;
use std::ops::Range;
use these_macros_should_be_illegal::discriminated_str;

use self::commands::Command;
#[cfg(test)]
use self::picker::{LineContext, PickerItem};
use self::picker::{Picker, PickerController, PickerInput, PickerItems, PickerMode, ValueItem};
use self::settings::keymap::Keymaps;
use self::settings::theme::groups::{Groups, HLGroup};
use self::settings::theme::Appearance;
use self::settings::{Settings, StatusLinePosition};
use super::action::Action;
use super::block::render::RenderContext;
use super::block::syntax_tree::SyntaxTreeBlock;
use super::block::{Block, BlockKind, Cell, CellState, CodeBlock, ErrorMessageBlock, TextBlock};
use super::config::UI;
use super::debug::FrameStats;
use super::element::{Element, HLSpan, StyleElement, TextSpan};
use super::feed::{BlockView, Feed};
use super::key::KeyEvent;
use super::statusline::{StatusLine, StatusLineContext, StatusLineEvent};
use super::DEBUG_POST;
use crate::app::session::CommittedCell;
use crate::code::highlight::Highlighter;
use crate::input::{InputCode, InputContext, InputSetting, InputSettings};

/// An outward effect produced while handling input or an [`Action`].
pub enum Reaction {
    Continue,
    Submit,
    Post(&'static str),
    Restart,
    Quit,
    /// Put the current selection on the system clipboard.
    Copy(String),
    Open(ViewKind, OpenTarget),
    New(OpenTarget),
    ClosePane,
}

/// Content rendered by one workspace pane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewKind {
    #[default]
    Shell,
    Code,
    Output,
    SyntaxTree,
}

/// Whether a command replaces the focused pane or creates the right pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenTarget {
    Current,
    Split,
}

// @##Hardcoded
impl OpenTarget {
    fn alternative(key: KeyEvent, can_split: bool) -> Option<Self> {
        if key.code == KeyCode::Char('v') && key.alt() && !key.ctrl() && can_split {
            Some(Self::Split)
        } else {
            None
        }
    }
}

/// The projection selected inside the output preview.
#[discriminated_str(label)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OutputMode {
    #[default]
    Evaluation = "evaluation output",
    Values = "values and types",
    Everything = "everything except input",
}

impl OutputMode {
    fn cycle(self, delta: i8) -> Self {
        match (self, delta > 0) {
            (Self::Evaluation, true) | (Self::Everything, false) => Self::Values,
            (Self::Values, true) => Self::Everything,
            (Self::Values, false) => Self::Evaluation,
            (Self::Everything, true) => Self::Evaluation,
            (Self::Evaluation, false) => Self::Everything,
        }
    }
}

/// Independent scrolling and output-filter state for one pane.
#[derive(Default)]
pub struct Viewport {
    pub scroll: usize,
    output: OutputMode,
    preview: Option<PreviewCache>,
}

/// Immutable committed cells paired with the revision that changes them.
pub struct CommittedCells<'a> {
    /// Cells in evaluation order.
    pub cells: &'a [CommittedCell],
    /// Workspace revision for cache invalidation.
    pub revision: u64,
}

/// Parsed block model retained by one code or syntax-tree preview pane.
struct PreviewCache {
    revision: u64,
    view: ViewKind,
    blocks: Vec<Box<dyn Block>>,
}

impl Viewport {
    // @##Suspicious
    pub fn for_view(view: ViewKind) -> Self {
        Self {
            scroll: if view == ViewKind::SyntaxTree {
                usize::MAX
            } else {
                0
            },
            ..Self::default()
        }
    }

    pub fn cycle_output(&mut self, delta: i8) {
        self.output = self.output.cycle(delta);
        self.scroll = 0;
    }

    fn render_committed(
        &mut self,
        committed: CommittedCells<'_>,
        view: ViewKind,
        highlighter: &mut dyn Highlighter,
        context: &RenderContext,
        area: Rect,
        buffer: &mut Buffer,
    ) -> Vec<Rect> {
        if self
            .preview
            .as_ref()
            .is_none_or(|cache| cache.revision != committed.revision || cache.view != view)
        {
            let blocks: Vec<Box<dyn Block>> = match view {
                ViewKind::Code => (!committed.cells.is_empty())
                    .then(|| {
                        Box::new(CodeBlock::new(committed.cells.iter().map(|cell| {
                            Cell::resolved(
                                cell.code.clone(),
                                cell.number,
                                cell.state,
                                highlighter.highlight(&cell.code),
                            )
                        }))) as Box<dyn Block>
                    })
                    .into_iter()
                    .collect(),
                ViewKind::SyntaxTree => committed
                    .cells
                    .iter()
                    .map(|cell| {
                        Box::new(SyntaxTreeBlock::new(
                            cell.code.clone(),
                            highlighter.highlight(&cell.code),
                            cell.tree.clone(),
                        )) as Box<dyn Block>
                    })
                    .collect(),
                _ => unreachable!("only committed previews are cached"),
            };
            self.preview = Some(PreviewCache {
                revision: committed.revision,
                view,
                blocks,
            });
        }
        let blocks = self
            .preview
            .as_ref()
            .unwrap()
            .blocks
            .iter()
            .map(Box::as_ref)
            .collect::<Vec<_>>();
        let empty = match view {
            ViewKind::Code => "no committed code",
            ViewKind::SyntaxTree => "no committed syntax trees",
            _ => unreachable!(),
        };
        BlockView::new(&blocks, empty).render(
            context,
            area,
            buffer,
            &mut self.scroll,
            context.groups().transcript(),
        )
    }
}

/// A session result to apply to the editor model.
pub enum EditorUpdate {
    Ignore,
    /// Leave the shell; the backend shuts down with it.
    Quit,
    ContinueSubmission(String),
    Diagnostic(String),
    SyntaxDiagnostic {
        message: String,
        range: Option<Range<usize>>,
    },
    Message(Box<dyn Block>),
    Pending {
        id: u64,
        cells: Vec<(String, Vec<HLSpan>)>,
    },
    CellSucceeded {
        id: u64,
        index: usize,
        number: u32,
        state: CellState,
        outputs: Vec<Box<dyn Block>>,
    },
    CellFailed {
        id: u64,
        index: usize,
        number: u32,
        message: String,
    },
}

/// A state change requested by a picker or one of its lines.
pub enum EditorEvent {
    EditCommand(KeyEvent),
    ResumeTyping(char),
    FinishCommand,
    RunCommand(Command, OpenTarget),
    ClosePicker,
    OpenStyle,
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
    Colorscheme = "colorscheme",
    Brightness = "brightness",
    Background = "background",
}

impl AppearanceField {
    fn value(self, appearance: Appearance) -> String {
        match self {
            Self::Colorscheme => appearance.colorscheme_label(),
            Self::Brightness => appearance.variant().brightness().to_string(),
            Self::Background => appearance.background_label().to_string(),
        }
    }

    fn adjust(self, appearance: Appearance, delta: i8) -> Appearance {
        match self {
            Self::Colorscheme => appearance.cycle_colorscheme(delta),
            Self::Brightness => appearance.cycle_brightness(delta),
            Self::Background => appearance.cycle_background(),
        }
    }
}

/// A selectable cell position in the captured screen.
#[derive(Clone, Copy)]
struct ScreenPosition {
    column: u16,
    row: i64,
}

/// A mouse selection spanning captured screen cells.
struct ScreenSelection {
    anchor: ScreenPosition,
    focus: ScreenPosition,
    dragged: bool,
    text: Option<String>,
}

/// Captured terminal content and whether it may be copied.
struct ScreenCell {
    symbol: String,
    selectable: bool,
}

const SHELL_ITEMS: &[ShellItem] = &[
    ShellItem::Input(InputSetting::MaxHeight),
    ShellItem::Input(InputSetting::Scrolloff),
    ShellItem::Statusline,
    ShellItem::Style,
    ShellItem::RestoreDefault,
];

/// One item listed in the settings picker.
#[derive(Clone, Copy)]
enum ShellItem {
    Input(InputSetting),
    Statusline,
    Style,
    RestoreDefault,
}

/// The editor child that receives an otherwise unclaimed input event.
#[derive(Clone, Copy)]
enum ElementFocus {
    /// The editable input field.
    Input,
    /// The topmost picker and its keyboard policy.
    Picker(PickerMode),
}

/// Presentation settings shared by every workspace.
#[derive(Clone)]
pub struct EditorConfig {
    settings: Settings,
    input: InputSettings,
    keymaps: Keymaps,
    appearance: Appearance,
}

/// The transcript feed, live input, and viewport state.
pub struct Editor {
    feed: Feed,
    input: InputCode,
    settings: Settings,
    keymaps: Keymaps,
    appearance: Appearance,
    groups: Groups,
    status_line: StatusLine,
    continuation: Option<String>,
    processing: bool,
    pickers: Vec<PickerController>,
    feed_area: Rect,
    feed_top: i64,
    screen_selecting: bool,
    screen_selection: Option<ScreenSelection>,
    screen: Vec<Vec<ScreenCell>>,
    screen_area: Rect,
    selection_exclusions: Vec<Rect>,
    split: bool,
    previewing: bool,
    pane_focused: bool,
    highlight_debug: bool,
    highlight_debug_scroll: usize,
    frame_stats: FrameStats,
}

impl Editor {
    pub fn new(welcome: String) -> Self {
        let appearance = Appearance::load();
        let mut editor = Self {
            feed: Feed::new(Box::new(TextBlock::system(welcome))),
            input: InputCode::new(),
            settings: Settings::default(),
            keymaps: Keymaps::default(),
            appearance,
            groups: appearance.groups(),
            status_line: StatusLine::default(),
            continuation: None,
            processing: false,
            pickers: Vec::new(),
            feed_area: Rect::ZERO,
            feed_top: 0,
            screen_selecting: false,
            screen_selection: None,
            screen: Vec::new(),
            screen_area: Rect::ZERO,
            selection_exclusions: Vec::new(),
            split: false,
            previewing: false,
            pane_focused: true,
            highlight_debug: false,
            highlight_debug_scroll: 0,
            frame_stats: FrameStats::default(),
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

    pub fn submission(&self) -> &str {
        self.input.text()
    }

    pub fn config(&self) -> EditorConfig {
        EditorConfig {
            settings: self.settings,
            input: self.input.settings(),
            keymaps: self.keymaps.clone(),
            appearance: self.appearance,
        }
    }

    pub fn set_config(&mut self, config: &EditorConfig) {
        self.settings = config.settings;
        self.input.set_settings(config.input);
        self.keymaps.clone_from(&config.keymaps);
        self.apply_appearance(config.appearance);
        self.refresh_status_line();
    }

    pub fn append_error(&mut self, message: impl Into<String>) {
        self.push_block(Box::new(ErrorMessageBlock::new(message.into())));
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
        (self.previewing && !self.picker_open())
            .then(|| self.keymaps.preview_action(key))
            .flatten()
            .or_else(|| {
                self.keymaps.action(
                    key,
                    matches!(self.focus(), ElementFocus::Picker(PickerMode::Keymaps)),
                )
            })
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
            EditorUpdate::Quit => return false,
            EditorUpdate::Ignore => {}
            EditorUpdate::ContinueSubmission(reason) => {
                self.continuation = Some(reason);
                self.input.field_mut().insert_char('\n');
            }
            EditorUpdate::Diagnostic(diagnostic) => self.input.set_diagnostic(diagnostic, None),
            EditorUpdate::SyntaxDiagnostic { message, range } => {
                self.continuation = None;
                self.input.set_diagnostic(message, range);
            }
            EditorUpdate::Message(block) => {
                self.push_block(block);
            }
            EditorUpdate::Pending { id, cells } => {
                self.continuation = None;
                self.input.accept_submission();
                self.push_submission(id, cells);
            }
            EditorUpdate::CellSucceeded {
                id,
                index,
                number,
                state,
                outputs,
            } => {
                self.input.clear_diagnostic();
                self.feed.update_submission(id, |submission| {
                    submission.succeed(index, number, state, outputs)
                });
            }
            EditorUpdate::CellFailed {
                id,
                index,
                number,
                message,
            } => {
                self.input.clear_diagnostic();
                self.feed
                    .update_submission(id, |submission| submission.fail(index, number, message));
            }
        }
        self.refresh_status_line();
        true
    }

    pub fn post(&mut self, update: EditorUpdate) -> bool {
        if let EditorUpdate::Pending { id, cells } = update {
            self.push_submission(id, cells);
            self.refresh_status_line();
            true
        } else {
            self.apply(update)
        }
    }

    fn push_submission(&mut self, id: u64, cells: Vec<(String, Vec<HLSpan>)>) {
        self.push_block(Box::new(super::block::SubmissionBlock::new(id, cells)));
    }

    fn push_block(&mut self, block: Box<dyn Block>) {
        self.feed.push(block);
    }

    pub fn paste(&mut self, text: &str) {
        self.input.insert(text);
        self.refresh_status_line();
    }

    /// Route the wheel by what it is pointing at: over the input pane it resizes
    /// the pane, everywhere else it scrolls the feed.
    pub fn handle_mouse(&mut self, event: MouseEvent) -> Reaction {
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

    pub fn handle_preview_mouse(&mut self, event: MouseEvent) -> Reaction {
        self.handle_screen_mouse(event)
            .unwrap_or(Reaction::Continue)
    }

    fn handle_screen_mouse(&mut self, event: MouseEvent) -> Option<Reaction> {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) if !self.input.contains(event) => {
                self.input.field_mut().clear_selection();
                let position = self.screen_position(event)?;
                self.screen_selection = Some(ScreenSelection {
                    anchor: position,
                    focus: position,
                    dragged: false,
                    text: None,
                });
                self.screen_selecting = true;
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
                if self.screen_selecting =>
            {
                if let Some(position) = self.screen_position(event) {
                    if let Some(selection) = self.screen_selection.as_mut() {
                        selection.focus = position;
                        selection.dragged |=
                            matches!(event.kind, MouseEventKind::Drag(MouseButton::Left));
                    }
                }
                if matches!(event.kind, MouseEventKind::Up(MouseButton::Left)) {
                    self.screen_selecting = false;
                    if let Some(text) = self.selected_screen_text() {
                        self.screen_selection.as_mut().unwrap().text = Some(text.clone());
                        return Some(Reaction::Copy(text));
                    }
                }
            }
            _ => return None,
        }
        Some(Reaction::Continue)
    }

    fn screen_position(&self, event: MouseEvent) -> Option<ScreenPosition> {
        let area = self.feed_area;
        if area.width == 0 || area.height == 0 {
            return None;
        }
        let pointer = Position::new(event.column, event.row);
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Left)) && !area.contains(pointer)
        {
            return None;
        }
        let column = event.column.clamp(area.x, area.right().saturating_sub(1));
        let screen_row = event.row.clamp(area.y, area.bottom().saturating_sub(1));
        Some(ScreenPosition {
            column,
            row: self.feed_top + i64::from(screen_row.saturating_sub(area.y)),
        })
    }

    fn selected_screen_text(&self) -> Option<String> {
        let selection = self.screen_selection.as_ref()?;
        if let Some(text) = selection.text.as_ref() {
            return Some(text.clone());
        }
        if !selection.dragged {
            return None;
        }
        let (start, end) = ordered_screen_positions(selection.anchor, selection.focus);
        let mut lines = Vec::new();
        for content_row in start.row..=end.row {
            if content_row < self.feed_top {
                continue;
            }
            let screen_row = (content_row - self.feed_top) as usize;
            if screen_row >= usize::from(self.feed_area.height) {
                continue;
            }
            let y = self.feed_area.y + screen_row as u16;
            let row = self
                .screen
                .get(y.saturating_sub(self.screen_area.y) as usize)?;
            let from = if content_row == start.row {
                start.column
            } else {
                self.feed_area.x
            };
            let through = if content_row == end.row {
                end.column
            } else {
                self.feed_area.right().saturating_sub(1)
            };
            let mut line = String::new();
            let mut has_content = false;
            for x in from..=through {
                if let Some(cell) = row.get(x.saturating_sub(self.screen_area.x) as usize) {
                    if cell.selectable {
                        line.push_str(&cell.symbol);
                        has_content = true;
                    }
                }
            }
            if has_content {
                lines.push(line.trim_end().to_string());
            }
        }
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    /// Forward unbound shell input to its focused component.
    pub fn handle_input(&mut self, key: KeyEvent) -> Reaction {
        if matches!(self.focus(), ElementFocus::Picker(_)) {
            return self.handle_picker_key(key);
        }
        if let Some(action) = self.handle_debug_key(key) {
            return action;
        }
        self.screen_selection = None;
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

    #[cfg(test)]
    fn handle_key(&mut self, key: KeyEvent) -> Reaction {
        match self.resolve_action(key) {
            Some(action) => self.handle_action(action),
            None => self.handle_input(key),
        }
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
        if let Some(command) = commands::shortcut(action) {
            return command.run(self, OpenTarget::Current);
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
            if let Some(command) = commands::exact(self.input.text(), self.split) {
                self.continuation = None;
                self.input.accept_submission();
                return command.run(self, OpenTarget::Current);
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
            | Action::ResizePaneRight
            | Action::Quit
            | Action::OpenSettings
            | Action::OpenStyle
            | Action::OpenKeymaps
            | Action::ClearFeed
            | Action::OpenCode
            | Action::NewWorkspace
            | Action::OpenOutput
            | Action::OpenSyntaxTree
            | Action::Restart
            | Action::ClosePane => unreachable!("commands run before input handling"),
        }
        self.refresh_status_line();
        Reaction::Continue
    }

    fn open_commands(&mut self) {
        let commands = commands::matching("", self.split);
        self.pickers = vec![PickerController {
            mode: PickerMode::Commands,
            picker: commands::picker(&commands, &self.keymaps, self.split),
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
        let commands = commands::matching("", self.split);
        self.pickers = vec![PickerController {
            mode: PickerMode::FilteredCommands,
            picker: commands::picker(&commands, &self.keymaps, self.split),
        }];
        self.refresh_status_line();
    }

    fn refresh_commands(&mut self) {
        let Some(query) = commands::query(self.input.text()) else {
            self.pickers.clear();
            self.refresh_status_line();
            return;
        };
        let commands = commands::matching(query, self.split);
        if commands.is_empty() {
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
        let mut picker = commands::picker(&commands, &self.keymaps, self.split);
        picker.select(selected);
        self.pickers.last_mut().unwrap().picker = picker;
    }

    fn finish_command(&mut self) {
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

    fn run_command(&mut self, command: Command, target: OpenTarget) -> Reaction {
        if self
            .pickers
            .last()
            .is_some_and(|controller| matches!(controller.mode, PickerMode::FilteredCommands))
        {
            self.input.clear();
            self.input.field_mut().insert_char('/');
            self.input.field_mut().insert_str(command.label());
            self.input.accept_submission();
        }
        command.run(self, target)
    }

    fn clear_feed(&mut self) {
        self.finish_command();
        self.feed.clear();
        self.screen_selection = None;
    }

    fn settings_picker(&self) -> Picker {
        Picker::new("settings", self.shell_items())
    }

    fn style_picker(&self) -> Picker {
        Picker::new("style", Self::appearance_items(self.appearance))
    }

    fn keymaps_picker(&self) -> Picker {
        Picker::new("keymaps", self.keymap_items())
    }

    fn shell_items(&self) -> PickerItems {
        SHELL_ITEMS
            .iter()
            .copied()
            .map(|item| match item {
                ShellItem::Input(field) => {
                    let mut settings = self.input.settings();
                    ValueItem::adjust(field.label(), field.value(settings), move |delta| {
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
                    ValueItem::adjust("statusline", value, move |delta| {
                        settings.statusline = settings.statusline.cycle(delta);
                        EditorEvent::SetSettings(settings)
                    })
                }
                ShellItem::Style => {
                    ValueItem::enter("style", self.appearance.colorscheme_label(), false, || {
                        EditorEvent::OpenStyle
                    })
                }
                ShellItem::RestoreDefault => {
                    ValueItem::enter("restore default", "", true, || EditorEvent::RestoreDefaults)
                }
            })
            .collect()
    }

    fn keymap_items(&self) -> PickerItems {
        Action::REMAPPABLE
            .iter()
            .map(|&action| {
                ValueItem::custom(
                    action.description(),
                    self.keymaps
                        .key_for(action)
                        .unwrap_or_else(|| "unbound".to_string()),
                    false,
                    move |input| match input {
                        PickerInput::Key(key) => Some(EditorEvent::Rebind(action, key)),
                        _ => None,
                    },
                )
            })
            .collect()
    }

    fn appearance_items(appearance: Appearance) -> PickerItems {
        [
            AppearanceField::Colorscheme,
            AppearanceField::Brightness,
            AppearanceField::Background,
        ]
        .into_iter()
        .map(|field| {
            ValueItem::adjust(field.label(), field.value(appearance), move |delta| {
                EditorEvent::SetAppearance(field.adjust(appearance, delta))
            })
        })
        .collect()
    }

    fn handle_picker_key(&mut self, key: KeyEvent) -> Reaction {
        let ElementFocus::Picker(_) = self.focus() else {
            return Reaction::Continue;
        };
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
            if let Some(command) = commands::shortcut(action) {
                return command.run(self, OpenTarget::Current);
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

    fn open_settings_from_commands(&mut self) {
        self.open_from_commands(PickerMode::Settings, Self::settings_picker);
    }

    fn open_style_from_commands(&mut self) {
        self.open_from_commands(PickerMode::Style, Self::style_picker);
    }

    fn open_keymaps_from_commands(&mut self) {
        self.open_from_commands(PickerMode::Keymaps, Self::keymaps_picker);
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

    fn open_style(&mut self) {
        self.pickers.push(PickerController {
            mode: PickerMode::Style,
            picker: self.style_picker(),
        });
        self.refresh_status_line();
    }

    fn handle_picker_event(&mut self, event: EditorEvent) -> Reaction {
        match event {
            EditorEvent::EditCommand(key) => {
                match key.code {
                    KeyCode::Char(character) => self.input.field_mut().insert_char(character),
                    KeyCode::Backspace => self.input.field_mut().backspace(),
                    KeyCode::Delete => self.input.field_mut().delete_forward(),
                    _ => {}
                }
                self.refresh_commands();
            }
            EditorEvent::ResumeTyping(character) => {
                self.pickers.clear();
                if !self.previewing {
                    self.input.clear_diagnostic();
                    self.input.field_mut().insert_char(character);
                }
            }
            EditorEvent::FinishCommand => self.finish_command(),
            EditorEvent::RunCommand(command, target) => return self.run_command(command, target),
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
                    self.input.clear();
                    self.input.field_mut().insert_char('/');
                    self.refresh_commands();
                }
            }
            EditorEvent::OpenStyle => self.open_style(),
            EditorEvent::RestoreDefaults => {
                self.settings = Settings::default();
                self.input.set_settings(InputSettings::default());
                self.keymaps = Keymaps::default();
                self.apply_appearance(Appearance::default());
                let items = self.shell_items();
                self.pickers.last_mut().unwrap().picker.set_items(items);
            }
            EditorEvent::SetSettings(settings) => {
                self.settings = settings;
                let items = self.shell_items();
                self.pickers.last_mut().unwrap().picker.set_items(items);
            }
            EditorEvent::SetInput(settings) => {
                self.input.set_settings(settings);
                let items = self.shell_items();
                self.pickers.last_mut().unwrap().picker.set_items(items);
            }
            EditorEvent::SetAppearance(appearance) => {
                self.apply_appearance(appearance);
                let items = Self::appearance_items(appearance);
                self.pickers.last_mut().unwrap().picker.set_items(items);
            }
            EditorEvent::Rebind(action, key) => {
                if self.keymaps.rebind(action, key) {
                    let items = self.keymap_items();
                    self.pickers.last_mut().unwrap().picker.set_items(items);
                }
            }
        }
        Reaction::Continue
    }

    fn apply_appearance(&mut self, appearance: Appearance) {
        if self.appearance == appearance {
            return;
        }
        self.appearance = appearance;
        self.groups = appearance.groups();
        self.feed.invalidate();
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
        self.feed.follow();
        if self.highlight_debug {
            Reaction::Post(DEBUG_POST)
        } else {
            Reaction::Continue
        }
    }

    fn highlight_debug_entries(&self) -> Vec<(String, ratatui::style::Style)> {
        let heading = self
            .groups
            .prompt()
            .add_modifier(ratatui::style::Modifier::BOLD);
        let mut entries = vec![
            (self.frame_stats.label(), self.groups.muted()),
            ("highlight groups".into(), heading),
        ];
        entries.extend(
            self.groups
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
        let base = self.groups.transcript().patch(self.groups.value());
        let block = TuiBlock::bordered()
            .style(base)
            .border_style(self.groups.border())
            .title(Span::styled(title, self.groups.prompt()));
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
            StatusLineEvent::Show {
                alignment: Alignment::Right,
                spans: vec![
                    TextSpan::new(format!("{commands_key} "), HLGroup::KeymapHint),
                    TextSpan::new("commands · ", HLGroup::Muted),
                    TextSpan::new(format!("{settings_key} "), HLGroup::KeymapHint),
                    TextSpan::new("settings", HLGroup::Muted),
                ],
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
        let theme = self.groups;
        let input_prefix = UI.prompt_width();
        frame.render_widget(Clear, area);

        let editor_area = area;

        let input_height = self
            .input
            .height(editor_area.width, input_prefix, editor_area.height);
        let status_height = u16::from(self.settings.statusline != StatusLinePosition::Off);
        let picker_height = self
            .pickers
            .last()
            .map_or(0, |controller| controller.picker.height(&theme));
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
        let context = RenderContext::new(editor_area.width, 0, &theme);
        let input_context = RenderContext::new(editor_area.width, input_prefix, &theme);

        self.feed_area = sections[0];
        self.draw_feed(frame, sections[0], &context);
        if let Some(controller) = self.pickers.last() {
            controller.picker.render_styled(
                &theme,
                sections[1],
                frame.buffer_mut(),
                theme.picker(),
            );
            self.selection_exclusions.push(sections[1]);
        }
        self.selection_exclusions.push(input_area);
        let command = commands::query(self.input.text())
            .is_some_and(|query| !commands::matching(query, self.split).is_empty());
        if let Some(cursor) = self.input.render(
            InputContext {
                render: &input_context,
                highlighter,
                command,
                debugging: in_debugger,
                focused: self.pane_focused && matches!(self.focus(), ElementFocus::Input),
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
                idle_left: Some(if self.processing {
                    TextSpan::new("Processing", HLGroup::StatuslineWarning)
                } else if let Some(reason) = &self.continuation {
                    TextSpan::new(
                        format!("Input pending ({reason})"),
                        HLGroup::StatuslineError,
                    )
                } else {
                    TextSpan::new("Ready", HLGroup::Statusline1)
                }),
                ..StatusLineContext::new(theme)
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

    pub fn draw_preview(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        highlighter: &mut dyn Highlighter,
        committed: CommittedCells<'_>,
        view: ViewKind,
        viewport: &mut Viewport,
    ) {
        if self.pane_focused {
            self.frame_stats.tick();
        }
        self.selection_exclusions.clear();
        self.input.hide();
        let theme = self.groups;
        frame.render_widget(Clear, area);
        TuiBlock::default()
            .style(self.appearance.canvas(if view == ViewKind::SyntaxTree {
                theme.code()
            } else {
                theme.transcript()
            }))
            .render(area, frame.buffer_mut());
        let picker_height = self
            .pickers
            .last()
            .map_or(0, |controller| controller.picker.height(&theme))
            .min(area.height.saturating_sub(1));
        let sections = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(picker_height),
            Constraint::Length(area.height.min(1)),
        ])
        .split(area);
        let context = RenderContext::new(sections[0].width, 0, &theme);
        self.feed_area = sections[0];

        match view {
            ViewKind::Shell => unreachable!("shells use Editor::draw"),
            ViewKind::Code => {
                self.selection_exclusions.extend(viewport.render_committed(
                    committed,
                    view,
                    highlighter,
                    &context,
                    sections[0],
                    frame.buffer_mut(),
                ));
            }
            ViewKind::SyntaxTree => {
                self.selection_exclusions.extend(viewport.render_committed(
                    committed,
                    view,
                    highlighter,
                    &context,
                    sections[0],
                    frame.buffer_mut(),
                ));
            }
            ViewKind::Output => {
                let blocks: Vec<&dyn Block> = self
                    .feed
                    .iter()
                    .flat_map(|block| {
                        let top = (viewport.output == OutputMode::Everything
                            && block.kind() != BlockKind::Input)
                            .then_some(block);
                        top.into_iter()
                            .chain(block.outputs().iter().map(Box::as_ref))
                    })
                    .filter(|block| match viewport.output {
                        OutputMode::Evaluation => matches!(
                            block.kind(),
                            BlockKind::Printed | BlockKind::Warning | BlockKind::Error
                        ),
                        OutputMode::Values => block.kind() == BlockKind::Value,
                        OutputMode::Everything => block.kind() != BlockKind::Input,
                    })
                    .collect();
                self.selection_exclusions.extend(
                    BlockView::new(&blocks, "no matching output").render(
                        &context,
                        sections[0],
                        frame.buffer_mut(),
                        &mut viewport.scroll,
                        theme.transcript(),
                    ),
                );
            }
        }
        self.feed_top = viewport.scroll as i64;
        self.selection_exclusions.extend([sections[1], sections[2]]);
        TuiBlock::default()
            .style(theme.statusline_bg())
            .render(sections[2], frame.buffer_mut());
        let center = (view == ViewKind::SyntaxTree).then(|| {
            vec![TextSpan::new(
                "Syntax Tree Preview Mode",
                HLGroup::Documentation,
            )]
        });
        if let Some(controller) = self.pickers.last() {
            controller.picker.render_styled(
                &theme,
                sections[1],
                frame.buffer_mut(),
                theme.picker(),
            );
            self.status_line.render(
                &StatusLineContext {
                    center,
                    ..StatusLineContext::new(theme)
                },
                sections[2],
                frame.buffer_mut(),
            );
        } else {
            let title = match view {
                ViewKind::Shell => unreachable!(),
                ViewKind::Code => Some("code"),
                ViewKind::Output => Some("output"),
                ViewKind::SyntaxTree => None,
            };
            let action = if self.split { "close" } else { "leave" };
            let mut right = Vec::new();
            if view == ViewKind::Output {
                right.extend([
                    TextSpan::new("←", HLGroup::KeymapHint),
                    TextSpan::new(format!(" {} ", viewport.output.label()), HLGroup::Muted),
                    TextSpan::new("→", HLGroup::KeymapHint),
                    TextSpan::new(" · ", HLGroup::Muted),
                ]);
            }
            right.extend([
                TextSpan::new("q ", HLGroup::KeymapHint),
                TextSpan::new(format!("{action} view"), HLGroup::Muted),
            ]);
            StatusLine::default().render(
                &StatusLineContext {
                    idle_left: title.map(|title| {
                        TextSpan::new(
                            title,
                            if self.pane_focused {
                                HLGroup::Value
                            } else {
                                HLGroup::Muted
                            },
                        )
                    }),
                    center,
                    right,
                    ..StatusLineContext::new(theme)
                },
                sections[2],
                frame.buffer_mut(),
            );
        }
        self.draw_highlight_debug(frame);
        self.capture_screen(frame);
        self.paint_screen_selection(frame);
    }

    fn capture_screen(&mut self, frame: &mut Frame) {
        let area = self.feed_area;
        let buffer = frame.buffer_mut();
        self.screen_area = area;
        self.screen = (area.y..area.bottom())
            .map(|y| {
                let symbols: Vec<String> = (area.x..area.right())
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect();
                let allowed: Vec<bool> = (area.x..area.right())
                    .map(|x| {
                        let position = Position::new(x, y);
                        self.selection_exclusions
                            .iter()
                            .all(|excluded| !excluded.contains(position))
                    })
                    .collect();
                let selectable = selectable_columns(&symbols, &allowed);
                symbols
                    .into_iter()
                    .zip(selectable)
                    .map(|(symbol, selectable)| ScreenCell { symbol, selectable })
                    .collect()
            })
            .collect();
    }

    fn paint_screen_selection(&self, frame: &mut Frame) {
        let Some(selection) = self.screen_selection.as_ref() else {
            return;
        };
        if !selection.dragged {
            return;
        }
        let (start, end) = ordered_screen_positions(selection.anchor, selection.focus);
        let area = self.feed_area;
        let buffer = frame.buffer_mut();
        for y in area.y..area.bottom() {
            let content_row = self.feed_top + i64::from(y - area.y);
            if content_row < start.row || content_row > end.row {
                continue;
            }
            let from = if content_row == start.row {
                start.column
            } else {
                area.x
            };
            let through = if content_row == end.row {
                end.column
            } else {
                area.right().saturating_sub(1)
            };
            for x in from..=through {
                let selectable = self
                    .screen
                    .get(y.saturating_sub(self.screen_area.y) as usize)
                    .and_then(|row| row.get(x.saturating_sub(self.screen_area.x) as usize))
                    .is_some_and(|cell| cell.selectable);
                if selectable {
                    let cell = &mut buffer[(x, y)];
                    cell.set_style(self.groups.selection());
                }
            }
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
        underfilled
    }
}

fn ordered_screen_positions(
    left: ScreenPosition,
    right: ScreenPosition,
) -> (ScreenPosition, ScreenPosition) {
    if (left.row, left.column) <= (right.row, right.column) {
        (left, right)
    } else {
        (right, left)
    }
}

fn selectable_columns(symbols: &[String], allowed: &[bool]) -> Vec<bool> {
    let mut selectable = vec![false; symbols.len()];
    let mut segment_start = 0;
    while segment_start < symbols.len() {
        while segment_start < symbols.len()
            && (!allowed[segment_start] || is_box_drawing(&symbols[segment_start]))
        {
            segment_start += 1;
        }
        let mut segment_end = segment_start;
        while segment_end < symbols.len()
            && allowed[segment_end]
            && !is_box_drawing(&symbols[segment_end])
        {
            segment_end += 1;
        }
        mark_selectable_segment(symbols, segment_start, segment_end, &mut selectable);
        segment_start = segment_end.saturating_add(1);
    }
    selectable
}

fn mark_selectable_segment(
    symbols: &[String],
    segment_start: usize,
    segment_end: usize,
    selectable: &mut [bool],
) {
    if segment_start >= segment_end || is_border_title(symbols, segment_start, segment_end) {
        return;
    }
    let Some(last_content) = symbols[segment_start..segment_end]
        .iter()
        .rposition(|symbol| !symbol.trim().is_empty())
        .map(|index| segment_start + index)
    else {
        return;
    };
    let content_start = symbols[segment_start..=last_content]
        .iter()
        .position(|symbol| !symbol.trim().is_empty())
        .map_or(segment_start, |index| segment_start + index);
    if content_start <= last_content {
        selectable[content_start..=last_content].fill(true);
    }
}

fn is_box_drawing(symbol: &str) -> bool {
    let mut characters = symbol.chars();
    matches!(characters.next(), Some('\u{2500}'..='\u{257f}')) && characters.next().is_none()
}

fn is_border_title(symbols: &[String], start: usize, end: usize) -> bool {
    start
        .checked_sub(1)
        .and_then(|index| symbols.get(index))
        .is_some_and(|symbol| symbol == "─")
        && symbols.get(end).is_some_and(|symbol| symbol == "─")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::block::{CellState, OutputBlock};
    use crossterm::event::{KeyEvent as CrosstermKeyEvent, KeyModifiers};
    use m2_syn::PrettyNode;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    struct EmptyHighlighter;

    impl Highlighter for EmptyHighlighter {
        fn highlight(&mut self, _source: &str) -> Vec<HLSpan> {
            Vec::new()
        }
    }

    struct OccupiedFeed;

    impl Element for OccupiedFeed {
        type Context = RenderContext;

        fn height(&self, _context: &RenderContext) -> u16 {
            7
        }

        fn render(&self, _context: &RenderContext, area: Rect, buf: &mut Buffer) {
            Paragraph::new("feed").render(
                Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
                buf,
            );
        }
    }

    impl Block for OccupiedFeed {}

    fn rendered_buffer(editor: &mut Editor, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut highlighter = EmptyHighlighter;
        terminal
            .draw(|frame| editor.draw(frame, frame.area(), &mut highlighter, false))
            .expect("draw editor");
        terminal.backend().buffer().clone()
    }

    fn rendered_editor(editor: &mut Editor, width: u16, height: u16) -> Vec<String> {
        let buffer = rendered_buffer(editor, width, height);
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    fn rendered_preview(
        editor: &mut Editor,
        committed: &[CommittedCell],
        view: ViewKind,
        viewport: &mut Viewport,
    ) -> Vec<String> {
        let backend = TestBackend::new(70, 18);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut highlighter = EmptyHighlighter;
        terminal
            .draw(|frame| {
                editor.draw_preview(
                    frame,
                    frame.area(),
                    &mut highlighter,
                    CommittedCells {
                        cells: committed,
                        revision: 0,
                    },
                    view,
                    viewport,
                )
            })
            .expect("draw preview");
        let buffer = terminal.backend().buffer();
        (0..18)
            .map(|y| (0..70).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect()
    }

    fn wheel(kind: MouseEventKind, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: 0,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn mouse_wheel_changes_feed_scroll_offset() {
        let mut editor = Editor::new(String::new());
        editor.handle_mouse(wheel(MouseEventKind::ScrollUp, 0));
        assert_eq!(editor.feed.offset(), 1);
        editor.handle_mouse(wheel(MouseEventKind::ScrollDown, 0));
        assert_eq!(editor.feed.offset(), 0);
    }

    #[test]
    fn ordinary_characters_reach_the_input() {
        let mut editor = Editor::new(String::new());

        press(&mut editor, KeyCode::Char('a'), KeyModifiers::NONE);

        assert_eq!(editor.input.text(), "a");
    }

    #[test]
    fn shifted_arrows_select_text_for_copying() {
        let mut editor = Editor::new(String::new());
        editor.input.field_mut().insert_str("abcd");

        press(&mut editor, KeyCode::Left, KeyModifiers::SHIFT);
        press(&mut editor, KeyCode::Left, KeyModifiers::SHIFT);

        let copied = editor.handle_key(key(KeyCode::Char('y'), KeyModifiers::CONTROL));
        let Reaction::Copy(text) = copied else {
            panic!("copy binding did not receive the selection");
        };
        assert_eq!(text, "cd");
    }

    #[test]
    fn mouse_drag_selects_input_text() {
        let mut editor = Editor::new(String::new());
        editor.input.field_mut().insert_str("abcd");
        editor
            .input
            .set_test_area(Rect::new(0, 5, 20, 3), Rect::new(5, 6, 15, 1));

        editor.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 6));
        editor.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 8, 6));
        let copied = editor.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 8, 6));

        assert_eq!(editor.input.field().selected_text(), Some("abc"));
        let Reaction::Copy(text) = copied else {
            panic!("mouse release did not copy the input selection");
        };
        assert_eq!(text, "abc");
    }

    #[test]
    fn mouse_drag_selects_rendered_feed_text_for_copying() {
        let mut editor = Editor::new("hello".to_string());
        let rows = rendered_editor(&mut editor, 40, 10);
        let row = rows.iter().position(|row| row.contains("hello")).unwrap() as u16;
        let start = rows[row as usize]
            .chars()
            .position(|character| character == 'h')
            .unwrap() as u16;

        editor.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), start, row));
        editor.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            start + 4,
            row,
        ));
        let copied =
            editor.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), start + 4, row));
        let Reaction::Copy(text) = copied else {
            panic!("mouse release did not copy the screen selection");
        };
        assert_eq!(text, "hello");
    }

    #[test]
    fn restart_completion_preserves_the_unsent_draft() {
        let mut editor = Editor::new(String::new());
        editor.paste("draft");

        assert!(matches!(
            editor.handle_key(key(KeyCode::Char('r'), KeyModifiers::ALT)),
            Reaction::Restart
        ));
        editor.apply(EditorUpdate::Message(Box::new(TextBlock::system(
            "restarted",
        ))));

        assert_eq!(editor.input.text(), "draft");
    }

    #[test]
    fn resolved_submission_keeps_its_input_and_output_numbers() {
        let mut editor = Editor::new(String::new());
        editor.apply(EditorUpdate::Pending {
            id: 4,
            cells: vec![("x".into(), Vec::new())],
        });
        editor.apply(EditorUpdate::CellSucceeded {
            id: 4,
            index: 0,
            number: 12,
            state: CellState::Value,
            outputs: vec![Box::new(
                OutputBlock::value("value".into(), None).numbered(12),
            )],
        });

        let rows = rendered_editor(&mut editor, 40, 12);

        assert!(rows.iter().any(|row| row.contains("  12▏ x")), "{rows:#?}");
        assert!(
            rows.iter().any(|row| row.contains("  o12 value")),
            "{rows:#?}"
        );
    }

    #[test]
    fn mouse_selection_skips_output_types() {
        let mut editor = Editor::new(String::new());
        editor.feed = Feed::new(Box::new(crate::ui::block::OutputBlock::value(
            "value".to_string(),
            Some("ZZ".to_string()),
        )));
        let rows = rendered_editor(&mut editor, 40, 10);
        let row = rows.iter().position(|row| row.contains("value")).unwrap() as u16;

        editor.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 0, row));
        editor.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 39, row));
        let copied = editor.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 39, row));

        let Reaction::Copy(text) = copied else {
            panic!("mouse release did not copy the output selection");
        };
        assert_eq!(text, "value");
    }

    #[test]
    fn underfilled_feed_grows_upward_from_the_input() {
        let mut editor = Editor::new("first".to_string());
        let rows = rendered_editor(&mut editor, 40, 12);
        let first = rows.iter().position(|row| row.contains("first")).unwrap();
        let border = rows.iter().position(|row| row.contains('╭')).unwrap();

        assert!(rows[..border].iter().all(|row| row.trim().is_empty()));
        editor.feed.push(Box::new(TextBlock::system("second")));
        let rows = rendered_editor(&mut editor, 40, 12);
        let moved = rows.iter().position(|row| row.contains("first")).unwrap();
        let appended = rows.iter().position(|row| row.contains("second")).unwrap();

        assert!(moved < first);
        assert!(appended > moved && appended < editor.input.area().y as usize);
    }

    #[test]
    fn scrolling_preserves_the_feed_selection() {
        let mut editor = Editor::new(
            (0..10)
                .map(|line| format!("line{line}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let rows = rendered_editor(&mut editor, 40, 10);
        let original_top = editor.feed_top;
        let start = rows[1].find("line").unwrap() as u16;
        editor.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), start, 1));
        editor.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), start + 4, 1));
        editor.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), start + 4, 1));
        let selected_row = editor.screen_selection.as_ref().unwrap().anchor.row;
        let selected_text = editor.selected_screen_text().unwrap();

        editor.handle_mouse(wheel(MouseEventKind::ScrollUp, 0));
        rendered_editor(&mut editor, 40, 10);

        assert_eq!(editor.feed_top, original_top - 1);
        assert_eq!(
            editor.screen_selection.as_ref().unwrap().anchor.row,
            selected_row
        );
        let copied = editor.handle_key(key(KeyCode::Char('y'), KeyModifiers::CONTROL));
        let Reaction::Copy(text) = copied else {
            panic!("scrolling cleared the feed selection");
        };
        assert_eq!(text, selected_text);
    }

    #[test]
    fn selectable_cells_exclude_ui_regions_and_box_drawing() {
        let symbols: Vec<String> = "  UI value │"
            .chars()
            .map(|character| character.to_string())
            .collect();
        let mut allowed = vec![true; symbols.len()];
        allowed[2..4].fill(false);
        let mask = selectable_columns(&symbols, &allowed);
        let selected: String = symbols
            .iter()
            .zip(mask.iter())
            .filter(|(_, selectable)| **selectable)
            .map(|(symbol, _)| symbol.as_str())
            .collect();

        assert_eq!(selected, "value");
    }

    // Over the input the wheel resizes the pane instead of scrolling the feed.
    #[test]
    fn mouse_wheel_over_input_resizes_it() {
        let mut editor = Editor::new(String::new());
        editor
            .input
            .set_test_area(Rect::new(0, 10, 40, 4), Rect::ZERO);
        let before = editor.input.settings().max_height_percent;

        editor.handle_mouse(wheel(MouseEventKind::ScrollUp, 11));
        assert_eq!(editor.input.settings().max_height_percent, before + 5);
        assert_eq!(editor.feed.offset(), 0, "the feed must not move");

        editor.handle_mouse(wheel(MouseEventKind::ScrollDown, 11));
        assert_eq!(editor.input.settings().max_height_percent, before);
    }

    #[test]
    fn settings_picker_adjusts_the_selected_field() {
        let mut editor = Editor::new(String::new());
        let before = editor.input.settings().scrolloff;

        open_settings(&mut editor);
        editor.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
        editor.handle_key(key(KeyCode::Right, KeyModifiers::NONE));

        assert_eq!(editor.input.settings().scrolloff, before + 1);
        assert_eq!(editor.input.text(), "", "the setting consumes the key");

        editor.handle_key(key(KeyCode::Esc, KeyModifiers::NONE));
        editor.handle_key(key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(editor.pickers.is_empty());
    }

    #[test]
    fn max_input_height_changes_by_five_percentage_points() {
        let mut editor = Editor::new(String::new());
        let before = editor.input.settings().max_height_percent;

        open_settings(&mut editor);
        press(&mut editor, KeyCode::Right, KeyModifiers::NONE);

        assert_eq!(editor.input.settings().max_height_percent, before + 5);
    }

    #[test]
    fn restore_default_resets_shell_keymaps_and_style_on_enter() {
        let mut editor = Editor::new(String::new());
        let rebound = key(KeyCode::Char('x'), KeyModifiers::CONTROL);
        let mut input = editor.input.settings();
        input.max_height_percent = 40;
        editor.input.set_settings(input);
        assert!(editor.keymaps.rebind(Action::ClearFeed, rebound));
        editor.apply_appearance(editor.appearance.cycle_colorscheme(1));
        editor.open_settings_from_commands();

        press(&mut editor, KeyCode::Down, KeyModifiers::NONE);
        press(&mut editor, KeyCode::Down, KeyModifiers::NONE);
        press(&mut editor, KeyCode::Down, KeyModifiers::NONE);
        press(&mut editor, KeyCode::Down, KeyModifiers::NONE);
        press(&mut editor, KeyCode::Enter, KeyModifiers::NONE);

        let defaults = InputSettings::default();
        assert_eq!(
            editor.input.settings().max_height_percent,
            defaults.max_height_percent
        );
        assert_eq!(editor.input.settings().scrolloff, defaults.scrolloff);
        assert!(editor.resolve_action(rebound).is_none());
        assert_eq!(editor.appearance, Appearance::default());
    }

    #[test]
    fn unchanged_config_keeps_the_retained_feed() {
        let mut editor = Editor::new("visible".into());
        rendered_buffer(&mut editor, 40, 10);
        assert!(editor.feed.retained());

        editor.set_config(&editor.config());

        assert!(editor.feed.retained());
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        CrosstermKeyEvent::new(code, modifiers).into()
    }

    fn press(editor: &mut Editor, code: KeyCode, modifiers: KeyModifiers) {
        editor.handle_key(key(code, modifiers));
    }

    fn picker_title(editor: &Editor) -> Option<&'static str> {
        editor
            .pickers
            .last()
            .map(|controller| controller.picker.title())
    }

    fn open_settings(editor: &mut Editor) {
        press(editor, KeyCode::Char('/'), KeyModifiers::NONE);
        assert_eq!(picker_title(editor), Some("commands"));
        press(editor, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(picker_title(editor), Some("settings"));
    }

    fn open_style(editor: &mut Editor) {
        for character in "/st".chars() {
            press(editor, KeyCode::Char(character), KeyModifiers::NONE);
        }
        press(editor, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(picker_title(editor), Some("style"));
    }

    fn open_keymaps(editor: &mut Editor) {
        press(editor, KeyCode::Char('/'), KeyModifiers::NONE);
        press(editor, KeyCode::Char('k'), KeyModifiers::NONE);
        press(editor, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(picker_title(editor), Some("keymaps"));
    }

    fn item_text(item: &dyn PickerItem) -> String {
        let groups = Groups::default();
        item.spans(&LineContext {
            width: 80,
            value_column: 40,
            columns: None,
            selected: false,
            groups,
        })
        .into_iter()
        .map(|span| span.content)
        .collect::<String>()
    }

    #[test]
    fn style_cycles_ordered_colorscheme_variants_in_one_axis() {
        let mut editor = Editor::new(String::new());
        open_style(&mut editor);

        press(&mut editor, KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(editor.appearance.colorscheme_label(), "everforest soft");
        press(&mut editor, KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(editor.appearance.colorscheme_label(), "everforest medium");
        press(&mut editor, KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(editor.appearance.colorscheme_label(), "everforest hard");
        press(&mut editor, KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(
            editor.appearance.colorscheme_label(),
            "gruvbox material original"
        );
    }

    #[test]
    fn enter_closes_style_and_keeps_changes() {
        let mut editor = Editor::new(String::new());
        let original = editor.appearance;

        open_style(&mut editor);
        press(&mut editor, KeyCode::Up, KeyModifiers::NONE);
        press(&mut editor, KeyCode::Right, KeyModifiers::NONE);
        assert_ne!(editor.appearance, original);

        press(&mut editor, KeyCode::Enter, KeyModifiers::NONE);
        assert_ne!(editor.appearance, original);
        assert_eq!(picker_title(&editor), Some("commands"));
    }

    #[test]
    fn style_brightness_preserves_the_variant_when_available() {
        let mut editor = Editor::new(String::new());
        open_style(&mut editor);
        press(&mut editor, KeyCode::Down, KeyModifiers::NONE);

        press(&mut editor, KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(
            editor.appearance.variant().brightness().to_string(),
            "light"
        );
        assert_eq!(editor.appearance.variant().label(), "medium");
        press(&mut editor, KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(editor.appearance.variant().brightness().to_string(), "dark");
        press(&mut editor, KeyCode::Left, KeyModifiers::NONE);
        assert_eq!(
            editor.appearance.variant().brightness().to_string(),
            "light"
        );
    }

    #[test]
    fn style_background_row_toggles_transparency_from_either_direction() {
        let mut editor = Editor::new(String::new());
        open_style(&mut editor);
        press(&mut editor, KeyCode::Up, KeyModifiers::NONE);

        press(&mut editor, KeyCode::Right, KeyModifiers::NONE);
        assert!(editor.appearance.transparent_background);
        press(&mut editor, KeyCode::Left, KeyModifiers::NONE);
        assert!(!editor.appearance.transparent_background);
    }

    #[test]
    fn style_rows_expose_three_cyclic_axes() {
        let rows = Editor::appearance_items(Appearance::default());
        let rows: Vec<_> = rows.iter().map(|row| item_text(row.as_ref())).collect();

        assert_eq!(rows.len(), 3);
        assert!(rows[0].contains("colorscheme") && rows[0].contains("everforest medium"));
        assert!(rows[1].contains("brightness") && rows[1].contains("dark"));
        assert!(rows[2].contains("background") && rows[2].contains("opaque"));
    }

    #[test]
    fn settings_style_row_opens_the_separate_picker_and_returns() {
        let mut editor = Editor::new(String::new());

        open_settings(&mut editor);
        press(&mut editor, KeyCode::Down, KeyModifiers::NONE);
        press(&mut editor, KeyCode::Down, KeyModifiers::NONE);
        press(&mut editor, KeyCode::Down, KeyModifiers::NONE);
        press(&mut editor, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(picker_title(&editor), Some("style"));
        press(&mut editor, KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(picker_title(&editor), Some("settings"));
        assert_eq!(editor.pickers.last().unwrap().picker.selected(), 3);
    }

    #[test]
    fn keymap_picker_remaps_every_exposed_command() {
        let mut editor = Editor::new(String::new());
        let labels: Vec<_> = editor
            .keymap_items()
            .iter()
            .map(|row| item_text(row.as_ref()))
            .collect();
        for action in Action::REMAPPABLE {
            assert!(
                labels
                    .iter()
                    .any(|label| label.contains(action.description())),
                "missing {}",
                action.description()
            );
        }

        open_keymaps(&mut editor);

        assert_eq!(
            editor.pickers.last().unwrap().picker.height(&editor.groups),
            Action::REMAPPABLE.len() as u16 + 2
        );
        press(&mut editor, KeyCode::Char('x'), KeyModifiers::CONTROL);
        press(&mut editor, KeyCode::Esc, KeyModifiers::NONE);
        press(&mut editor, KeyCode::Esc, KeyModifiers::NONE);
        press(&mut editor, KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert_eq!(picker_title(&editor), Some("commands"));
    }

    // Quitting must not require dismissing a picker first.
    #[test]
    fn quit_works_with_a_picker_open() {
        let mut editor = Editor::new(String::new());
        press(&mut editor, KeyCode::Char('/'), KeyModifiers::NONE);

        let action = editor.handle_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(matches!(action, Reaction::Quit));
    }

    #[test]
    fn plain_q_leaves_a_preview() {
        let mut editor = Editor::new(String::new());
        editor.set_context(false, true, true);

        assert!(editor.resolve_action(KeyCode::Char('q').into()) == Some(Action::Quit));
    }

    #[test]
    fn typing_closes_settings_and_style_and_reaches_the_input() {
        let mut editor = Editor::new(String::new());
        editor.input.field_mut().insert_char('x');
        editor.open_settings_from_commands();
        press(&mut editor, KeyCode::Char('y'), KeyModifiers::NONE);
        assert!(editor.pickers.is_empty());
        assert_eq!(editor.input.text(), "xy");

        editor.open_style();
        press(&mut editor, KeyCode::Char('z'), KeyModifiers::NONE);
        assert!(editor.pickers.is_empty());
        assert_eq!(editor.input.text(), "xyz");
    }

    #[test]
    fn slash_opens_the_command_picker_and_filters_its_rows() {
        let mut editor = Editor::new(String::new());

        press(&mut editor, KeyCode::Char('/'), KeyModifiers::NONE);
        press(&mut editor, KeyCode::Char('s'), KeyModifiers::NONE);
        press(&mut editor, KeyCode::Char('e'), KeyModifiers::NONE);
        press(&mut editor, KeyCode::Char('t'), KeyModifiers::NONE);

        let picker = &editor.pickers.last().unwrap().picker;
        assert_eq!(picker.title(), "commands");
        assert_eq!(picker.len(), 1);
        assert_eq!(picker.height(&Groups::default()), UI.picker_min_rows + 2);
        assert_eq!(editor.input.text(), "/set");

        press(&mut editor, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(picker_title(&editor), Some("settings"));
        assert!(editor.input.text().is_empty());
    }

    #[test]
    fn command_shortcut_opens_the_full_picker_until_typing_resumes() {
        let mut editor = Editor::new(String::new());
        editor.input.field_mut().insert_str("ab");

        press(&mut editor, KeyCode::Char('/'), KeyModifiers::CONTROL);
        assert_eq!(picker_title(&editor), Some("commands"));
        assert_eq!(
            editor.pickers.last().unwrap().picker.len(),
            commands::matching("", false).len()
        );
        assert_eq!(editor.input.text(), "ab");

        press(&mut editor, KeyCode::Char('c'), KeyModifiers::NONE);
        assert!(editor.pickers.is_empty());
        assert_eq!(editor.input.text(), "abc");

        press(&mut editor, KeyCode::Char('/'), KeyModifiers::CONTROL);
        press(&mut editor, KeyCode::Esc, KeyModifiers::NONE);
        assert!(editor.pickers.is_empty());
        assert_eq!(editor.input.text(), "abc");
    }

    #[test]
    fn opener_shortcuts_toggle_and_switch_pickers() {
        let mut editor = Editor::new(String::new());
        let commands = key(KeyCode::Char('/'), KeyModifiers::CONTROL);
        let settings = key(KeyCode::Char('o'), KeyModifiers::CONTROL);

        editor.handle_key(commands);
        assert_eq!(picker_title(&editor), Some("commands"));
        editor.handle_key(commands);
        assert!(editor.pickers.is_empty());

        editor.handle_key(settings);
        assert_eq!(picker_title(&editor), Some("settings"));
        editor.handle_key(commands);
        assert_eq!(picker_title(&editor), Some("commands"));
        editor.handle_key(commands);
        assert!(editor.pickers.is_empty());

        editor.handle_key(settings);
        editor.handle_key(settings);
        assert!(editor.pickers.is_empty());
    }

    #[test]
    fn command_keymaps_run_callbacks_and_toggle_their_pickers() {
        let mut editor = Editor::new(String::new());

        let action = editor.handle_key(key(KeyCode::Char('c'), KeyModifiers::ALT));
        assert!(matches!(
            action,
            Reaction::Open(ViewKind::Code, OpenTarget::Current)
        ));

        for (key, title) in [('s', "style"), ('k', "keymaps")] {
            press(&mut editor, KeyCode::Char(key), KeyModifiers::ALT);
            assert_eq!(picker_title(&editor), Some(title));
            press(&mut editor, KeyCode::Char(key), KeyModifiers::ALT);
            assert!(editor.pickers.is_empty());
        }
    }

    #[test]
    fn command_shortcut_preserves_input_through_nested_pickers_and_commands() {
        let mut editor = Editor::new(String::new());
        editor.input = InputCode::new_in_memory();
        editor.input.field_mut().insert_str("x");

        press(&mut editor, KeyCode::Char('/'), KeyModifiers::CONTROL);
        press(&mut editor, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(picker_title(&editor), Some("settings"));
        assert_eq!(editor.input.text(), "x");
        press(&mut editor, KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(picker_title(&editor), Some("commands"));

        for _ in 0..4 {
            press(&mut editor, KeyCode::Down, KeyModifiers::NONE);
        }
        let action = editor.handle_key(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(editor.pickers.is_empty());
        assert!(matches!(
            action,
            Reaction::Open(ViewKind::Code, OpenTarget::Current)
        ));
        assert_eq!(editor.input.text(), "x");

        editor.input.clear();
        press(&mut editor, KeyCode::Up, KeyModifiers::NONE);
        assert!(editor.input.text().is_empty());
    }

    #[test]
    fn slash_command_history_records_the_completed_command() {
        let mut editor = Editor::new(String::new());
        editor.input = InputCode::new_in_memory();

        for character in "/as".chars() {
            press(&mut editor, KeyCode::Char(character), KeyModifiers::NONE);
        }
        let action = editor.handle_key(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            action,
            Reaction::Open(ViewKind::SyntaxTree, OpenTarget::Current)
        ));

        press(&mut editor, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(editor.input.text(), "/ast");

        let action = editor.handle_key(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            action,
            Reaction::Open(ViewKind::SyntaxTree, OpenTarget::Current)
        ));
        assert!(editor.input.text().is_empty());
    }

    #[test]
    fn debug_command_posts_through_the_session_and_enters_history() {
        let mut editor = Editor::new(String::new());
        editor.input = InputCode::new_in_memory();
        let ordinary = editor.feed.len();

        for character in "/de".chars() {
            press(&mut editor, KeyCode::Char(character), KeyModifiers::NONE);
        }
        let action = editor.handle_key(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(editor.highlight_debug);
        assert!(editor.pickers.is_empty());
        assert!(matches!(action, Reaction::Post(DEBUG_POST)));

        editor.post(EditorUpdate::Pending {
            id: 7,
            cells: [(DEBUG_POST.to_string(), Vec::new())].into(),
        });
        assert_eq!(editor.feed.len(), ordinary + 1);

        press(&mut editor, KeyCode::Esc, KeyModifiers::NONE);
        assert!(!editor.highlight_debug);
        assert_eq!(editor.feed.len(), ordinary + 1);
        press(&mut editor, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(editor.input.text(), "/debug");
    }

    #[test]
    fn command_filter_matches_only_visible_word_prefixes() {
        let mut editor = Editor::new(String::new());

        for character in "/co".chars() {
            press(&mut editor, KeyCode::Char(character), KeyModifiers::NONE);
        }

        assert_eq!(editor.pickers.last().unwrap().picker.len(), 1);
        let action = editor.handle_key(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            action,
            Reaction::Open(ViewKind::Code, OpenTarget::Current)
        ));
    }

    #[test]
    fn clear_command_empties_the_feed() {
        let mut editor = Editor::new("welcome".to_string());

        for character in "/cl".chars() {
            press(&mut editor, KeyCode::Char(character), KeyModifiers::NONE);
        }
        press(&mut editor, KeyCode::Enter, KeyModifiers::NONE);

        assert!(editor.feed.is_empty());
        assert!(editor.input.text().is_empty());
        assert!(editor.pickers.is_empty());
    }

    #[test]
    fn command_picker_uses_literal_and_aligned_columns_when_they_fit() {
        let mut editor = Editor::new(String::new());
        press(&mut editor, KeyCode::Char('/'), KeyModifiers::NONE);

        let rows = rendered_editor(&mut editor, 120, 16);
        let settings = rows
            .iter()
            .find(|row| row.contains("settings") && row.contains("ctrl+o"))
            .unwrap_or_else(|| panic!("settings command missing from {rows:#?}"));
        let quit = rows
            .iter()
            .find(|row| row.contains("quit") && row.contains("ctrl+c"))
            .unwrap_or_else(|| panic!("quit command missing from {rows:#?}"));

        assert!(settings.contains(Action::OpenSettings.description()));
        assert!(quit.contains(Action::Quit.description()));
        assert!(settings.contains("/settings"));
        assert!(quit.contains("/quit"));
        assert_eq!(
            settings.find(Action::OpenSettings.description()),
            quit.find(Action::Quit.description())
        );
        assert_eq!(settings.find("ctrl+o"), quit.find("ctrl+c"));
    }

    #[test]
    fn an_empty_or_unmatched_command_returns_to_code_input() {
        let mut editor = Editor::new(String::new());

        press(&mut editor, KeyCode::Char('/'), KeyModifiers::NONE);
        press(&mut editor, KeyCode::Backspace, KeyModifiers::NONE);
        assert!(editor.pickers.is_empty());
        assert!(editor.input.text().is_empty());

        press(&mut editor, KeyCode::Char('/'), KeyModifiers::NONE);
        press(&mut editor, KeyCode::Char('z'), KeyModifiers::NONE);
        assert!(editor.pickers.is_empty());
        assert_eq!(editor.input.text(), "/z");

        press(&mut editor, KeyCode::Char('$'), KeyModifiers::NONE);
        assert_eq!(editor.input.text(), "/z$");
    }

    #[test]
    fn settings_has_no_tabs_and_style_has_its_own_title() {
        let mut editor = Editor::new(String::new());
        editor.open_settings_from_commands();

        let rows = rendered_editor(&mut editor, 70, 12);
        let screen = rows.join("\n");

        assert!(screen.contains(" settings "));
        assert!(!screen.contains("[shell]") && !screen.contains("[theme]"));

        editor.open_style();
        assert!(rendered_editor(&mut editor, 70, 12)
            .join("\n")
            .contains(" style "));
    }

    #[test]
    fn statusline_setting_cycles_in_both_directions() {
        let mut settings = Settings::default();

        settings.statusline = settings.statusline.cycle(1);
        assert_eq!(settings.statusline, StatusLinePosition::AboveInput);
        settings.statusline = settings.statusline.cycle(1);
        assert_eq!(settings.statusline, StatusLinePosition::Off);
        settings.statusline = settings.statusline.cycle(1);
        assert_eq!(settings.statusline, StatusLinePosition::BelowInput);
        settings.statusline = settings.statusline.cycle(-1);
        assert_eq!(settings.statusline, StatusLinePosition::Off);
    }

    #[test]
    fn statusline_can_move_above_input_or_release_its_row() {
        let mut editor = Editor::new(String::new());
        editor.feed.clear();
        editor.settings.statusline = StatusLinePosition::AboveInput;
        let rows = rendered_editor(&mut editor, 40, 10);

        assert!(rows[editor.input.area().y as usize].trim().is_empty());
        assert!(rows[editor.input.area().y.saturating_sub(1) as usize].contains("ctrl+/ commands"));

        editor.settings.statusline = StatusLinePosition::Off;
        let rows = rendered_editor(&mut editor, 40, 10);

        assert_eq!(editor.input.area().bottom(), 10);
        assert!(rows.iter().all(|row| !row.contains("ctrl+/ commands")));
    }

    #[test]
    fn status_line_is_right_aligned_below_empty_input() {
        let mut editor = Editor::new(String::new());
        editor.feed.clear();

        let rows = rendered_editor(&mut editor, 40, 10);
        let status = rows.last().unwrap();
        let top_margin = &rows[editor.input.area().y as usize];
        let bottom_margin = &rows[editor.input.area().bottom().saturating_sub(1) as usize];

        assert!(status
            .trim_end()
            .ends_with("ctrl+/ commands · ctrl+o settings"));
        assert!(status.starts_with("  "));
        assert!(status.ends_with("  "));
        assert!(top_margin.trim().is_empty());
        assert!(
            bottom_margin.trim().is_empty(),
            "bottom margin {bottom_margin:?} in {:?}",
            editor.input.area()
        );
    }

    #[test]
    fn highlight_debug_lists_only_the_complete_highlight_group_space() {
        let mut editor = Editor::new(String::new());
        press(&mut editor, KeyCode::Char('d'), KeyModifiers::CONTROL);

        let rows = rendered_editor(&mut editor, 120, 120);
        let screen = rows.join("\n");
        let title_row = rows.iter().position(|row| row.contains("debug")).unwrap();
        assert!(screen.contains("FPS"));
        assert!(screen.contains("highlight groups"));
        assert!(screen.contains("Conditional"));
        assert!(screen.contains("LspTypeClass"));
        assert!(screen.contains("Accent1"));
        assert!(screen.contains("BgColor1"));
        assert!(screen.contains("CodeBlock"));
        assert!(screen.contains("Normal"));
        assert!(screen.contains("Delimiter"));
        let entries = editor.highlight_debug_entries();
        assert_eq!(entries.len(), editor.groups.highlight_groups().len() + 2);
        assert!(!entries.iter().any(|(name, _)| name == "bg_dim"));
        assert!(entries
            .iter()
            .skip(2)
            .all(|(name, _)| screen.contains(name)));
        assert!(rows[title_row].ends_with('┐'));
        assert_eq!(
            rows.iter().find_map(|row| row.find("CodeBlock")).unwrap(),
            rows.iter().find_map(|row| row.find("Normal")).unwrap(),
        );
        assert_eq!(
            rows.iter().find_map(|row| row.find("Normal")).unwrap(),
            rows.iter().find_map(|row| row.find("Delimiter")).unwrap()
        );

        press(&mut editor, KeyCode::Esc, KeyModifiers::NONE);
        assert!(!rendered_editor(&mut editor, 120, 120)
            .join("\n")
            .contains("Normal"));
    }

    #[test]
    fn upper_input_margin_keeps_the_input_band_background() {
        let mut editor = Editor::new(String::new());
        let backend = TestBackend::new(70, 20);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut highlighter = EmptyHighlighter;

        terminal
            .draw(|frame| editor.draw(frame, frame.area(), &mut highlighter, false))
            .expect("draw editor");

        let margin = &terminal.backend().buffer()[(0, editor.input.area().y)];
        assert_eq!(margin.symbol(), " ");
        assert_eq!(margin.bg, editor.groups.code().bg.unwrap());
        assert_ne!(margin.bg, editor.groups.transcript().bg.unwrap());
    }

    #[test]
    fn transparent_background_only_clears_unpainted_feed_cells() {
        let mut editor = Editor::new("visible".to_string());
        editor.apply_appearance(editor.appearance.cycle_background());
        let buffer = rendered_buffer(&mut editor, 70, 20);
        let content = (0..20)
            .find_map(|y| {
                let row = (0..70).map(|x| buffer[(x, y)].symbol()).collect::<String>();
                row.find("visible").map(|x| (x as u16, y))
            })
            .expect("welcome text");

        assert_eq!(buffer[(0, 0)].bg, ratatui::style::Color::Reset);
        assert_eq!(buffer[content].bg, ratatui::style::Color::Reset);
        assert_eq!(
            buffer[(0, editor.input.area().y)].bg,
            editor.groups.code().bg.unwrap()
        );
        assert_eq!(
            buffer[(0, buffer.area().bottom() - 1)].bg,
            editor.groups.statusline_bg().bg.unwrap()
        );
    }

    #[test]
    fn slash_commands_use_the_prompt_highlight_in_the_input() {
        let mut editor = Editor::new(String::new());
        editor.input.field_mut().insert_str("/style");
        let buffer = rendered_buffer(&mut editor, 70, 20);
        let foreground = editor.groups.prompt().fg.unwrap();

        let text_area = editor.input.text_area();
        for x in text_area.x..text_area.x + 6 {
            assert_eq!(buffer[(x, text_area.y)].fg, foreground);
        }
    }

    #[test]
    fn statusline_reports_ready_pending_and_processing() {
        let mut editor = Editor::new(String::new());
        let rows = rendered_editor(&mut editor, 70, 10);
        assert!(rows.last().unwrap().contains("Ready"));

        press(&mut editor, KeyCode::Char('('), KeyModifiers::NONE);
        let rows = rendered_editor(&mut editor, 70, 10);
        assert!(rows.last().unwrap().contains("Ready"));

        editor.apply(EditorUpdate::ContinueSubmission(
            "unclosed parenthesized expression".into(),
        ));
        let buffer = rendered_buffer(&mut editor, 70, 10);
        let status = (0..70).map(|x| buffer[(x, 9)].symbol()).collect::<String>();
        assert!(status.contains("Input pending (unclosed"));
        assert_eq!(
            buffer[(UI.statusline_margin, 9)].fg,
            editor.groups.style(HLGroup::StatuslineError).fg.unwrap()
        );

        editor.set_processing(true);
        let rows = rendered_editor(&mut editor, 70, 10);
        assert!(rows.last().unwrap().contains("Processing"));
        assert!(!rows.last().unwrap().contains("Macaulay2"));
    }

    #[test]
    fn modified_enter_inserts_a_newline_without_submitting() {
        for modifier in [KeyModifiers::SHIFT, KeyModifiers::ALT] {
            let mut editor = Editor::new(String::new());
            editor.input.field_mut().insert_str("x");

            assert!(matches!(
                editor.handle_key(key(KeyCode::Enter, modifier)),
                Reaction::Continue
            ));
            assert_eq!(editor.submission(), "x\n");
        }
    }

    #[test]
    fn pickers_keep_the_regular_statusline() {
        let mut editor = Editor::new(String::new());
        editor.refresh_status_line();
        editor.open_settings_from_commands();
        let rows = rendered_editor(&mut editor, 70, 12);

        assert!(rows.last().unwrap().contains("ctrl+o settings"));
    }

    #[test]
    fn nonempty_input_keeps_only_the_settings_shortcut() {
        let mut editor = Editor::new(String::new());
        editor.input.field_mut().insert_str("x");
        editor.refresh_status_line();

        let rows = rendered_editor(&mut editor, 40, 10);
        let status = rows.last().unwrap();
        assert!(status.trim_end().ends_with("ctrl+o settings"));
        assert!(!status.contains("ctrl+/ commands"));

        press(&mut editor, KeyCode::Char('o'), KeyModifiers::CONTROL);
        assert_eq!(picker_title(&editor), Some("settings"));
        press(&mut editor, KeyCode::Esc, KeyModifiers::NONE);
        assert!(editor.pickers.is_empty());
        assert_eq!(editor.input.text(), "x");
    }

    #[test]
    fn status_line_keeps_settings_available_below_a_full_feed() {
        let mut editor = Editor::new(String::new());
        editor.feed = Feed::new(Box::new(OccupiedFeed));

        let rows = rendered_editor(&mut editor, 40, 10);
        let status = rows.last().unwrap();

        assert!(status.trim_end().ends_with("ctrl+o settings"));
    }

    #[test]
    fn code_picker_command_opens_the_view_and_clears_input() {
        let mut editor = Editor::new(String::new());
        press(&mut editor, KeyCode::Char('/'), KeyModifiers::NONE);
        press(&mut editor, KeyCode::Char('c'), KeyModifiers::NONE);
        press(&mut editor, KeyCode::Char('o'), KeyModifiers::NONE);
        let action = editor.handle_key(key(KeyCode::Enter, KeyModifiers::NONE));

        assert!(matches!(
            action,
            Reaction::Open(ViewKind::Code, OpenTarget::Current)
        ));
        assert!(editor.submission().is_empty());
    }

    #[test]
    fn syntax_preview_uses_committed_code_and_the_m2_syn_tree_without_input() {
        let mut editor = Editor::new(String::new());
        editor.input.field_mut().insert_str("UNCOMMITTED_DRAFT");
        let committed = [CommittedCell {
            number: 3,
            state: CellState::Value,
            code: "committedCode = 1".to_string(),
            tree: PrettyNode::syntax("CommittedTree", None),
        }];

        let rows = rendered_preview(
            &mut editor,
            &committed,
            ViewKind::SyntaxTree,
            &mut Viewport::default(),
        );
        let screen = rows.join("\n");
        assert!(screen.contains("committedCode = 1"));
        assert!(screen.contains("CommittedTree"));
        assert!(!screen.contains("UNCOMMITTED_DRAFT"));
        assert!(rows.last().unwrap().contains("q leave view"));
    }

    #[test]
    fn unchanged_preview_reuses_committed_highlights() {
        /// A semantic highlighter counting preview rebuilds.
        struct CountingHighlighter(usize);

        impl Highlighter for CountingHighlighter {
            fn highlight(&mut self, _source: &str) -> Vec<HLSpan> {
                self.0 += 1;
                Vec::new()
            }
        }

        let committed = [CommittedCell {
            number: 1,
            state: CellState::Value,
            code: "x".into(),
            tree: PrettyNode::syntax("Symbol", None),
        }];
        let mut editor = Editor::new(String::new());
        let mut viewport = Viewport::default();
        let mut highlighter = CountingHighlighter(0);
        let backend = TestBackend::new(70, 18);
        let mut terminal = Terminal::new(backend).unwrap();
        for revision in [1, 1, 2] {
            terminal
                .draw(|frame| {
                    editor.draw_preview(
                        frame,
                        frame.area(),
                        &mut highlighter,
                        CommittedCells {
                            cells: &committed,
                            revision,
                        },
                        ViewKind::Code,
                        &mut viewport,
                    )
                })
                .unwrap();
        }
        assert_eq!(highlighter.0, 2);
    }

    #[test]
    fn syntax_preview_centers_its_yellow_mode_banner_with_status_margins() {
        let mut editor = Editor::new(String::new());
        let committed = [CommittedCell {
            number: 1,
            state: CellState::Value,
            code: "x".to_string(),
            tree: PrettyNode::syntax("Symbol", None),
        }];
        let backend = TestBackend::new(70, 18);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut highlighter = EmptyHighlighter;
        let mut viewport = Viewport::default();
        terminal
            .draw(|frame| {
                editor.draw_preview(
                    frame,
                    frame.area(),
                    &mut highlighter,
                    CommittedCells {
                        cells: &committed,
                        revision: 0,
                    },
                    ViewKind::SyntaxTree,
                    &mut viewport,
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let y = buffer.area().bottom() - 1;
        let row: String = (0..buffer.area().width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        let banner = row.find("Syntax Tree Preview Mode").unwrap();
        let key = row.find("q leave view").unwrap() as u16;

        assert_eq!(banner, (usize::from(buffer.area().width) - 24) / 2);
        assert_eq!(
            buffer[(banner as u16, y)].fg,
            editor.groups.style(HLGroup::Statusline2).fg.unwrap()
        );
        assert!(!row.contains("syntax tree"));
        assert_eq!(buffer[(key, y)].fg, editor.groups.prompt().fg.unwrap());
        assert!(row.starts_with("  "));
        assert!(row.ends_with("  "));
    }

    #[test]
    fn syntax_preview_opens_at_the_latest_tree() {
        let committed: Vec<_> = (1..=8)
            .map(|number| CommittedCell {
                number,
                state: CellState::Value,
                code: format!("CELL_{number}"),
                tree: PrettyNode::syntax(format!("TREE_{number}"), None),
            })
            .collect();
        let mut editor = Editor::new(String::new());
        let mut viewport = Viewport::for_view(ViewKind::SyntaxTree);
        let screen = rendered_preview(&mut editor, &committed, ViewKind::SyntaxTree, &mut viewport)
            .join("\n");

        assert!(viewport.scroll > 0);
        assert!(screen.contains("TREE_8"), "{screen}");
        assert!(!screen.contains("TREE_1"), "{screen}");
    }

    #[test]
    fn syntax_preview_selects_code_and_tree_text() {
        let committed = [CommittedCell {
            number: 3,
            state: CellState::Value,
            code: "CODE_TEXT".to_string(),
            tree: PrettyNode::syntax("TREE_TEXT", None),
        }];
        let mut editor = Editor::new(String::new());
        let rows = rendered_preview(
            &mut editor,
            &committed,
            ViewKind::SyntaxTree,
            &mut Viewport::default(),
        );

        for text in ["CODE_TEXT", "TREE_TEXT"] {
            let row = rows.iter().position(|line| line.contains(text)).unwrap() as u16;
            let start = rows[row as usize].find(text).unwrap() as u16;
            let end = start + text.len() as u16 - 1;
            editor.handle_preview_mouse(mouse(MouseEventKind::Down(MouseButton::Left), start, row));
            editor.handle_preview_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), end, row));
            let copied =
                editor.handle_preview_mouse(mouse(MouseEventKind::Up(MouseButton::Left), end, row));
            let Reaction::Copy(copied) = copied else {
                panic!("preview selection did not copy {text}");
            };
            assert_eq!(copied, text);
        }
    }

    #[test]
    fn preview_selection_copies_code_and_output_without_chrome() {
        let select = |editor: &mut Editor, row| {
            editor.handle_preview_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 0, row));
            editor.handle_preview_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 69, row));
            let Reaction::Copy(text) =
                editor.handle_preview_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 69, row))
            else {
                panic!("preview selection was not copied");
            };
            text
        };
        let committed = [CommittedCell {
            number: 12,
            state: CellState::Value,
            code: "CODE_TEXT".to_string(),
            tree: PrettyNode::syntax("tree", None),
        }];
        let mut editor = Editor::new(String::new());
        let rows = rendered_preview(
            &mut editor,
            &committed,
            ViewKind::Code,
            &mut Viewport::default(),
        );
        let row = rows
            .iter()
            .position(|line| line.contains("CODE_TEXT"))
            .unwrap() as u16;
        assert_eq!(select(&mut editor, row), "CODE_TEXT");

        editor.feed = Feed::new(Box::new(crate::ui::block::OutputBlock::value(
            "VALUE_TEXT".to_string(),
            None,
        )));
        let mut viewport = Viewport::default();
        viewport.cycle_output(-1);
        let rows = rendered_preview(&mut editor, &[], ViewKind::Output, &mut viewport);
        let row = rows
            .iter()
            .position(|line| line.contains("VALUE_TEXT"))
            .unwrap() as u16;
        assert_eq!(select(&mut editor, row), "VALUE_TEXT");
    }

    #[test]
    fn output_preview_cycles_the_three_structured_filters() {
        use crate::ui::block::{OutputBlock, SubmissionBlock};

        let mut submission = SubmissionBlock::new(0, [("x".to_string(), Vec::new())]);
        submission.succeed(
            0,
            1,
            CellState::Value,
            vec![
                Box::new(TextBlock::printed("PRINTED_ONLY")),
                Box::new(
                    OutputBlock::value("VALUE_ONLY".to_string(), Some("TYPE_ONLY".to_string()))
                        .numbered(1),
                ),
            ],
        );
        let mut editor = Editor::new(String::new());
        editor.feed = Feed::new(Box::new(submission));
        let mut viewport = Viewport::default();

        let evaluation =
            rendered_preview(&mut editor, &[], ViewKind::Output, &mut viewport).join("\n");
        assert!(evaluation.contains("PRINTED_ONLY"));
        assert!(!evaluation.contains("VALUE_ONLY"));
        viewport.cycle_output(1);
        let values = rendered_preview(&mut editor, &[], ViewKind::Output, &mut viewport).join("\n");
        assert!(!values.contains("PRINTED_ONLY"));
        assert!(values.contains("VALUE_ONLY"));
        assert!(values.contains("TYPE_ONLY"));
        viewport.cycle_output(1);
        let all = rendered_preview(&mut editor, &[], ViewKind::Output, &mut viewport).join("\n");
        assert!(all.contains("PRINTED_ONLY"));
        assert!(all.contains("VALUE_ONLY"));
    }
}
