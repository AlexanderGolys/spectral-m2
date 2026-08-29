//! The interactive editor: submitted feed, live input and history, navigation,
//! scrolling, and rendering.

mod commands;
mod picker;
mod settings;

pub use commands::{submitted_command, SubmittedCommand};
pub use settings::Groups;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as TuiBlock, Clear, Paragraph, Widget};
use ratatui::Frame;
use std::ops::Range;
use std::time::Instant;

use self::commands::Command;
use self::picker::{
    finish_spans, value_spans, Line as PickerLine, LineContext, Picker, PickerItem, PickerItems,
    PickerStatus, TextComponent,
};
use self::settings::{Action, Appearance, Keymap, SettingField, Settings, StatusLinePosition};
use super::block::render::{gutter_span, gutter_width, prompt_gutter_width, RenderContext};
use super::block::{
    Block, BlockKind, Cell, CodeBlock, ErrorMessageBlock, SyntaxTreeBlock, TextBlock,
};
use super::statusline::{StatusLine, StatusLineEvent};
use crate::app::CommittedCell;
use crate::code::highlight::Highlighter;
use crate::input::InputCode;

/// The smallest input pane worth drawing: one code row between its two margins.
const MIN_INPUT_HEIGHT: u16 = 3;

/// Control flow requested by one editor key event.
pub enum EditorAction {
    Continue,
    Submit,
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

/// The projection selected inside the output preview.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputMode {
    #[default]
    Evaluation,
    Values,
    Everything,
}

impl OutputMode {
    pub fn cycle(self, delta: i8) -> Self {
        match (self, delta > 0) {
            (Self::Evaluation, true) | (Self::Everything, false) => Self::Values,
            (Self::Values, true) => Self::Everything,
            (Self::Values, false) => Self::Evaluation,
            (Self::Everything, true) => Self::Evaluation,
            (Self::Evaluation, false) => Self::Everything,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Evaluation => "evaluation output",
            Self::Values => "values and types",
            Self::Everything => "everything except input",
        }
    }
}

/// Independent scrolling and output-filter state for one pane.
#[derive(Default)]
pub struct Viewport {
    pub scroll: usize,
    output: OutputMode,
}

impl Viewport {
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
}

/// A session result to apply to the editor model.
pub enum EditorUpdate {
    Ignore,
    /// Leave the shell; the backend shuts down with it.
    Quit,
    ContinueSubmission,
    Diagnostic(String),
    SyntaxDiagnostic {
        message: String,
        range: Option<Range<usize>>,
    },
    Commit(Vec<Box<dyn Block>>),
    Pending {
        id: u64,
        cells: Vec<(String, Vec<crate::code::highlight::HLSpan>)>,
    },
    CellSucceeded {
        id: u64,
        index: usize,
        number: u32,
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
enum EditorEvent {
    EditCommand(KeyEvent),
    ResumeTyping(char),
    FinishCommand,
    RunCommand(Command, OpenTarget),
    ClosePicker,
    OpenStyle,
    RestoreDefaults,
    SetSettings(Settings),
    SetAppearance(Appearance),
    Rebind(Action, KeyEvent),
}

/// One independently cyclic axis of a colour appearance.
#[derive(Clone, Copy)]
enum AppearanceField {
    Colorscheme,
    Brightness,
    Background,
}

impl AppearanceField {
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
    row: usize,
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

/// Region in which the current mouse selection began.
#[derive(Clone, Copy)]
enum MouseSelection {
    Input,
    Screen,
}

const SHELL_ITEMS: &[ShellItem] = &[
    ShellItem::Setting(SettingField::MaxInputHeight),
    ShellItem::Setting(SettingField::InputScrolloff),
    ShellItem::Setting(SettingField::StatusLine),
    ShellItem::Style,
    ShellItem::RestoreDefault,
];

/// One item listed in the settings picker.
#[derive(Clone, Copy)]
enum ShellItem {
    Setting(SettingField),
    Style,
    RestoreDefault,
}

/// A shell setting line that updates its own displayed state on arrow keys.
struct SettingItem {
    field: SettingField,
    settings: Settings,
}

impl TextComponent for SettingItem {
    fn spans(&self, context: &LineContext<'_>) -> Vec<Span<'static>> {
        value_spans(
            self.field.label(),
            &self.field.value(&self.settings),
            false,
            context,
        )
    }
}

impl PickerItem for SettingItem {
    fn handle_key(&mut self, key: KeyEvent) -> Option<EditorEvent> {
        let delta = match key.code {
            KeyCode::Left => -1,
            KeyCode::Right => 1,
            _ => return None,
        };
        self.field.adjust(&mut self.settings, delta);
        Some(EditorEvent::SetSettings(self.settings))
    }
}

/// The settings entry opening the dedicated style picker.
struct StyleItem {
    appearance: Appearance,
}

impl TextComponent for StyleItem {
    fn spans(&self, context: &LineContext<'_>) -> Vec<Span<'static>> {
        value_spans(
            "style",
            &self.appearance.colorscheme_label(),
            false,
            context,
        )
    }
}

impl PickerItem for StyleItem {
    fn handle_key(&mut self, key: KeyEvent) -> Option<EditorEvent> {
        (key.code == KeyCode::Enter).then_some(EditorEvent::OpenStyle)
    }
}

/// A single-span picker line that restores every setting on Enter.
struct RestoreDefaultsItem;

impl TextComponent for RestoreDefaultsItem {
    fn spans(&self, context: &LineContext<'_>) -> Vec<Span<'static>> {
        finish_spans(
            vec![Span::styled(
                if context.selected {
                    " > restore default"
                } else {
                    "   restore default"
                },
                if context.selected {
                    context.groups.prompt()
                } else {
                    context.groups.muted()
                },
            )],
            context,
        )
    }
}

impl PickerItem for RestoreDefaultsItem {
    fn handle_key(&mut self, key: KeyEvent) -> Option<EditorEvent> {
        (key.code == KeyCode::Enter).then_some(EditorEvent::RestoreDefaults)
    }
}

/// A keymap line that accepts a replacement modified key.
struct KeymapItem {
    action: Action,
    key: String,
}

impl TextComponent for KeymapItem {
    fn spans(&self, context: &LineContext<'_>) -> Vec<Span<'static>> {
        value_spans(self.action.description(), &self.key, false, context)
    }
}

impl PickerItem for KeymapItem {
    fn handle_key(&mut self, key: KeyEvent) -> Option<EditorEvent> {
        if matches!(key.code, KeyCode::Char(_))
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            || matches!(
                key.code,
                KeyCode::Esc
                    | KeyCode::Enter
                    | KeyCode::Left
                    | KeyCode::Right
                    | KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Tab
                    | KeyCode::BackTab
            )
        {
            return None;
        }
        Some(EditorEvent::Rebind(self.action, key))
    }
}

/// A theme line that cycles one appearance axis on arrow keys.
struct AppearanceItem {
    field: AppearanceField,
    appearance: Appearance,
}

impl AppearanceItem {
    fn label(&self) -> &'static str {
        match self.field {
            AppearanceField::Colorscheme => "colorscheme",
            AppearanceField::Brightness => "brightness",
            AppearanceField::Background => "background",
        }
    }

    fn value(&self) -> String {
        match self.field {
            AppearanceField::Colorscheme => self.appearance.colorscheme_label(),
            AppearanceField::Brightness => self.appearance.variant().brightness().to_string(),
            AppearanceField::Background => self.appearance.background_label().to_string(),
        }
    }
}

impl TextComponent for AppearanceItem {
    fn spans(&self, context: &LineContext<'_>) -> Vec<Span<'static>> {
        value_spans(self.label(), &self.value(), false, context)
    }
}

impl PickerItem for AppearanceItem {
    fn handle_key(&mut self, key: KeyEvent) -> Option<EditorEvent> {
        let delta = match key.code {
            KeyCode::Left => -1,
            KeyCode::Right => 1,
            _ => return None,
        };
        self.appearance = self.field.adjust(self.appearance, delta);
        Some(EditorEvent::SetAppearance(self.appearance))
    }
}

macro_rules! picker_item {
    ($item:ty) => {
        picker_item!($item, |_| None);
    };
    ($item:ty, $width:expr) => {
        impl PickerLine for $item {
            fn alignment_width(&self) -> Option<usize> {
                $width(self)
            }
        }
    };
}

picker_item!(SettingItem, |item: &SettingItem| Some(
    item.field.label().chars().count()
));
picker_item!(StyleItem, |_| Some("style".len()));
picker_item!(RestoreDefaultsItem);
picker_item!(KeymapItem, |item: &KeymapItem| Some(
    item.action.description().chars().count()
));
picker_item!(AppearanceItem, |item: &AppearanceItem| Some(
    item.label().chars().count()
));

/// Keyboard behavior attached to an otherwise presentation-only picker.
#[derive(Clone, Copy)]
enum PickerMode {
    FilteredCommands,
    Commands,
    Settings,
    Style,
    Keymaps,
}

/// A picker paired with the controller that interprets its unhandled keys.
struct PickerController {
    mode: PickerMode,
    picker: Picker,
}

/// Presentation settings shared by every workspace.
#[derive(Clone)]
pub(crate) struct EditorConfig {
    settings: Settings,
    keymap: Keymap,
    appearance: Appearance,
}

/// The transcript feed, live input, and viewport state.
pub struct Editor {
    feed: Vec<Box<dyn Block>>,
    input: InputCode,
    scroll_offset: usize,
    /// First visual row of the input drawn, once the source outgrows its pane.
    input_scroll: usize,
    settings: Settings,
    keymap: Keymap,
    appearance: Appearance,
    groups: Groups,
    status_line: StatusLine,
    macaulay_version: Option<String>,
    pickers: Vec<PickerController>,
    /// The input pane's area from the last frame, so wheel events can tell
    /// whether the pointer is over the input or the feed.
    input_area: Rect,
    input_text_area: Rect,
    feed_area: Rect,
    feed_top: usize,
    mouse_selection: Option<MouseSelection>,
    screen_selection: Option<ScreenSelection>,
    screen: Vec<Vec<ScreenCell>>,
    screen_area: Rect,
    selection_exclusions: Vec<Rect>,
    animation_started: Instant,
    split: bool,
    previewing: bool,
    focused: bool,
    highlight_debug: bool,
    highlight_debug_scroll: usize,
}

impl Editor {
    pub fn new(welcome: String) -> Self {
        let appearance = Appearance::default();
        let mut editor = Self {
            feed: vec![Box::new(TextBlock::system(welcome))],
            input: InputCode::new(),
            scroll_offset: 0,
            input_scroll: 0,
            settings: Settings::default(),
            keymap: Keymap::default(),
            appearance,
            groups: appearance.groups(),
            status_line: StatusLine::default(),
            macaulay_version: None,
            pickers: Vec::new(),
            input_area: Rect::ZERO,
            input_text_area: Rect::ZERO,
            feed_area: Rect::ZERO,
            feed_top: 0,
            mouse_selection: None,
            screen_selection: None,
            screen: Vec::new(),
            screen_area: Rect::ZERO,
            selection_exclusions: Vec::new(),
            animation_started: Instant::now(),
            split: false,
            previewing: false,
            focused: true,
            highlight_debug: false,
            highlight_debug_scroll: 0,
        };
        editor.refresh_status_line();
        editor
    }

    pub fn with_macaulay_version(mut self, version: String) -> Self {
        self.macaulay_version = Some(format!("Macaulay2 v{version}"));
        self
    }

    pub fn submission(&self) -> &str {
        self.input.text()
    }

    pub(crate) fn config(&self) -> EditorConfig {
        EditorConfig {
            settings: self.settings,
            keymap: self.keymap.clone(),
            appearance: self.appearance,
        }
    }

    pub(crate) fn set_config(&mut self, config: &EditorConfig) {
        self.settings = config.settings;
        self.keymap.clone_from(&config.keymap);
        self.apply_appearance(config.appearance);
        self.refresh_status_line();
    }

    pub(crate) fn append_error(&mut self, message: impl Into<String>) {
        self.feed
            .push(Box::new(ErrorMessageBlock::new(message.into())));
        self.scroll_offset = 0;
    }

    pub fn set_context(&mut self, split: bool, previewing: bool, focused: bool) {
        self.split = split;
        self.previewing = previewing;
        self.focused = focused;
    }

    pub fn picker_open(&self) -> bool {
        !self.pickers.is_empty()
    }

    pub fn handle_preview_key(&mut self, key: KeyEvent) -> EditorAction {
        if self.picker_open() {
            return self.handle_picker_key(key);
        }
        if self.handle_debug_key(key) {
            return EditorAction::Continue;
        }
        match self.keymap.action(key) {
            Some(Action::Quit) => EditorAction::Quit,
            Some(Action::CopySelection) => self
                .selected_screen_text()
                .map_or(EditorAction::Continue, EditorAction::Copy),
            Some(Action::OpenCommands) => {
                self.open_commands();
                EditorAction::Continue
            }
            Some(Action::OpenSettings) => {
                self.open_settings();
                EditorAction::Continue
            }
            _ => EditorAction::Continue,
        }
    }

    /// Apply a session result. Returns whether the shell should keep running.
    pub fn apply(&mut self, update: EditorUpdate) -> bool {
        match update {
            EditorUpdate::Quit => return false,
            EditorUpdate::Ignore => {}
            EditorUpdate::ContinueSubmission => self.input.field_mut().insert_char('\n'),
            EditorUpdate::Diagnostic(diagnostic) => self.input.set_diagnostic(diagnostic, None),
            EditorUpdate::SyntaxDiagnostic { message, range } => {
                self.input.set_diagnostic(message, range);
            }
            EditorUpdate::Commit(blocks) => {
                self.input.accept_submission();
                self.feed.extend(blocks);
                self.scroll_offset = 0;
            }
            EditorUpdate::Pending { id, cells } => {
                self.input.accept_submission();
                self.feed
                    .push(Box::new(super::block::SubmissionBlock::new(id, cells)));
                self.scroll_offset = 0;
            }
            EditorUpdate::CellSucceeded {
                id,
                index,
                number,
                outputs,
            } => {
                self.input.clear_diagnostic();
                if let Some(submission) = self.feed.iter_mut().find_map(|block| {
                    block
                        .submission_mut()
                        .filter(|submission| submission.id() == id)
                }) {
                    submission.succeed(index, number, outputs);
                    self.scroll_offset = 0;
                }
            }
            EditorUpdate::CellFailed {
                id,
                index,
                number,
                message,
            } => {
                self.input.clear_diagnostic();
                if let Some(submission) = self.feed.iter_mut().find_map(|block| {
                    block
                        .submission_mut()
                        .filter(|submission| submission.id() == id)
                }) {
                    submission.fail(index, number, message);
                    self.scroll_offset = 0;
                }
            }
        }
        self.refresh_status_line();
        true
    }

    pub fn paste(&mut self, text: &str) {
        self.input.clear_diagnostic();
        self.input.field_mut().insert_str(text);
        self.refresh_status_line();
    }

    /// Route the wheel by what it is pointing at: over the input pane it resizes
    /// the pane, everywhere else it scrolls the feed.
    pub fn handle_mouse(&mut self, event: MouseEvent) -> EditorAction {
        if let Some(action) = self.handle_screen_mouse(event) {
            return action;
        }
        let completes_selection = matches!(event.kind, MouseEventKind::Up(MouseButton::Left));
        match event.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let delta = if event.kind == MouseEventKind::ScrollUp {
                    1
                } else {
                    -1
                };
                if self.pointer_over_input(&event) {
                    SettingField::MaxInputHeight.adjust(&mut self.settings, delta);
                } else if delta > 0 {
                    self.scroll_offset = self.scroll_offset.saturating_add(1);
                } else {
                    self.scroll_offset = self.scroll_offset.saturating_sub(1);
                }
            }
            MouseEventKind::Down(MouseButton::Left) if self.pointer_over_input(&event) => {
                self.screen_selection = None;
                self.mouse_selection = Some(MouseSelection::Input);
                if let Some(cursor) = self.input_cursor_at_mouse(event) {
                    self.input.field_mut().start_selection(cursor);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => {
                if matches!(self.mouse_selection, Some(MouseSelection::Input)) {
                    if let Some(cursor) = self.input_cursor_at_mouse(event) {
                        self.input.field_mut().continue_selection(cursor);
                    }
                }
                if matches!(event.kind, MouseEventKind::Up(MouseButton::Left)) {
                    self.mouse_selection = None;
                }
            }
            _ => {}
        }
        if completes_selection {
            if let Some(text) = self.input.field().selected_text().map(str::to_string) {
                return EditorAction::Copy(text);
            }
        }
        EditorAction::Continue
    }

    pub fn handle_preview_mouse(&mut self, event: MouseEvent) -> EditorAction {
        self.handle_screen_mouse(event)
            .unwrap_or(EditorAction::Continue)
    }

    fn handle_screen_mouse(&mut self, event: MouseEvent) -> Option<EditorAction> {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) if !self.pointer_over_input(&event) => {
                self.input.field_mut().clear_selection();
                let position = self.screen_position(event)?;
                self.screen_selection = Some(ScreenSelection {
                    anchor: position,
                    focus: position,
                    dragged: false,
                    text: None,
                });
                self.mouse_selection = Some(MouseSelection::Screen);
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
                if matches!(self.mouse_selection, Some(MouseSelection::Screen)) =>
            {
                if let Some(position) = self.screen_position(event) {
                    if let Some(selection) = self.screen_selection.as_mut() {
                        selection.focus = position;
                        selection.dragged |=
                            matches!(event.kind, MouseEventKind::Drag(MouseButton::Left));
                    }
                }
                if matches!(event.kind, MouseEventKind::Up(MouseButton::Left)) {
                    self.mouse_selection = None;
                    if let Some(text) = self.selected_screen_text() {
                        self.screen_selection.as_mut().unwrap().text = Some(text.clone());
                        return Some(EditorAction::Copy(text));
                    }
                }
            }
            _ => return None,
        }
        Some(EditorAction::Continue)
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
            row: self.feed_top + usize::from(screen_row.saturating_sub(area.y)),
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
            let screen_row = content_row - self.feed_top;
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

    fn input_cursor_at_mouse(&self, event: MouseEvent) -> Option<usize> {
        let area = self.input_text_area;
        if area.width == 0 || area.height == 0 {
            return None;
        }
        let row = event
            .row
            .saturating_sub(area.y)
            .min(area.height.saturating_sub(1));
        let column = event.column.saturating_sub(area.x).min(area.width) as usize;
        Some(
            self.input
                .cursor_at_visual(area.width, self.input_scroll + row as usize, column),
        )
    }

    fn pointer_over_input(&self, event: &MouseEvent) -> bool {
        let area = self.input_area;
        event.row >= area.y
            && event.row < area.y.saturating_add(area.height)
            && event.column >= area.x
            && event.column < area.x.saturating_add(area.width)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> EditorAction {
        if !self.pickers.is_empty() {
            return self.handle_picker_key(key);
        }
        if self.handle_debug_key(key) {
            return EditorAction::Continue;
        }

        let Some(action) = self.keymap.action(key) else {
            self.screen_selection = None;
            // Anything the table does not claim is ordinary typing.
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            {
                if let KeyCode::Char(character) = key.code {
                    self.input.clear_diagnostic();
                    self.input.field_mut().insert_char(character);
                    if character == '/' && self.input.text() == "/" {
                        self.open_filtered_commands();
                    }
                }
            }
            self.refresh_status_line();
            return EditorAction::Continue;
        };
        if action != Action::CopySelection {
            self.screen_selection = None;
        }

        if matches!(
            action,
            Action::Submit
                | Action::InsertNewline
                | Action::InsertTab
                | Action::DeleteBack
                | Action::DeleteForward
        ) {
            self.input.clear_diagnostic();
        }
        if action == Action::ClearInput {
            self.input.clear();
            self.refresh_status_line();
            return EditorAction::Continue;
        }
        if action == Action::CopySelection {
            if let Some(text) = self
                .input
                .field()
                .selected_text()
                .map(str::to_string)
                .or_else(|| self.selected_screen_text())
            {
                return EditorAction::Copy(text);
            }
            return EditorAction::Continue;
        }
        if action == Action::Submit {
            if let Some(command) = commands::exact(self.input.text(), self.split) {
                self.input.accept_submission();
                return command.run(self, OpenTarget::Current);
            }
        }

        let field = self.input.field_mut();
        match action {
            Action::Quit => return EditorAction::Quit,
            // Submitting is not an edit, so it must not consume a live selection:
            // an incomplete buffer comes back as `ContinueSubmission`, whose
            // inserted newline would otherwise delete the selected text.
            Action::Submit => {
                field.clear_selection();
                return EditorAction::Submit;
            }
            Action::InsertNewline => field.insert_char('\n'),
            Action::InsertTab => field.insert_char('\t'),
            Action::DeleteBack => field.backspace(),
            Action::DeleteForward => field.delete_forward(),
            Action::MoveLeft => field.move_left(),
            Action::MoveRight => field.move_right(),
            Action::MoveLineStart => field.move_home(),
            Action::MoveLineEnd => field.move_end(),
            Action::MoveUp if field.cursor_on_first_line() => field.history_prev(),
            Action::MoveUp => field.move_up(),
            Action::MoveDown if field.cursor_on_last_line() => field.history_next(),
            Action::MoveDown => field.move_down(),
            Action::SelectLeft => field.select_left(),
            Action::SelectRight => field.select_right(),
            Action::SelectLineStart => field.select_home(),
            Action::SelectLineEnd => field.select_end(),
            Action::CopySelection => unreachable!("copy selection is handled before editing"),
            Action::ClearInput => unreachable!("clear input is handled before borrowing the field"),
            Action::ScrollFeedUp => self.scroll_offset = self.scroll_offset.saturating_add(1),
            Action::ScrollFeedDown => self.scroll_offset = self.scroll_offset.saturating_sub(1),
            Action::OpenCommands => self.open_commands(),
            Action::OpenSettings => self.open_settings(),
            Action::ToggleHighlightDebug => unreachable!("debug toggles before input handling"),
        }
        self.refresh_status_line();
        EditorAction::Continue
    }

    fn open_commands(&mut self) {
        let commands = commands::matching("", self.split);
        self.pickers = vec![PickerController {
            mode: PickerMode::Commands,
            picker: commands::picker(&commands, &self.keymap, self.split),
        }];
        self.refresh_status_line();
    }

    fn open_filtered_commands(&mut self) {
        let commands = commands::matching("", self.split);
        self.pickers = vec![PickerController {
            mode: PickerMode::FilteredCommands,
            picker: commands::filtered_picker(&commands, &self.keymap, self.split),
        }];
        self.refresh_status_line();
    }

    fn refresh_commands(&mut self) {
        let Some(query) = self.input.text().strip_prefix('/') else {
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
        let mut picker = commands::filtered_picker(&commands, &self.keymap, self.split);
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

    fn run_command(&mut self, command: Command, target: OpenTarget) -> EditorAction {
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
        self.scroll_offset = 0;
        self.screen_selection = None;
    }

    fn settings_picker(&self) -> Picker {
        Picker::new(
            "settings",
            self.shell_items(),
            PickerStatus::new("settings ", "↑↓ select · ←→ adjust · enter open · esc back"),
        )
    }

    fn style_picker(&self) -> Picker {
        Picker::new(
            "style",
            Self::appearance_items(self.appearance),
            PickerStatus::new("style ", "↑↓ select · ←→ cycle · esc back"),
        )
    }

    fn keymaps_picker(&self) -> Picker {
        Picker::new(
            "keymaps",
            self.keymap_items(),
            PickerStatus::new("keymaps ", "↑↓ select · press a modified key · esc back"),
        )
    }

    fn shell_items(&self) -> PickerItems {
        SHELL_ITEMS
            .iter()
            .copied()
            .map(|item| match item {
                ShellItem::Setting(field) => Box::new(SettingItem {
                    field,
                    settings: self.settings,
                }) as Box<dyn PickerItem>,
                ShellItem::Style => Box::new(StyleItem {
                    appearance: self.appearance,
                }) as Box<dyn PickerItem>,
                ShellItem::RestoreDefault => Box::new(RestoreDefaultsItem) as Box<dyn PickerItem>,
            })
            .collect()
    }

    fn keymap_items(&self) -> PickerItems {
        Action::REMAPPABLE
            .iter()
            .map(|&action| {
                Box::new(KeymapItem {
                    action,
                    key: self
                        .keymap
                        .key_for(action)
                        .unwrap_or_else(|| "unbound".to_string()),
                }) as Box<dyn PickerItem>
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
        .map(|field| Box::new(AppearanceItem { field, appearance }) as Box<dyn PickerItem>)
        .collect()
    }

    fn handle_picker_key(&mut self, key: KeyEvent) -> EditorAction {
        if self.handle_debug_key(key) {
            return EditorAction::Continue;
        }
        if self.keymap.action(key) == Some(Action::Quit) {
            return EditorAction::Quit;
        }
        let controller = self.pickers.last_mut().unwrap();
        let mut events = Vec::new();
        let handled = match key.code {
            KeyCode::Up => {
                controller.picker.move_selection(-1);
                true
            }
            KeyCode::Down => {
                controller.picker.move_selection(1);
                true
            }
            _ => controller
                .picker
                .selected_item_mut()
                .and_then(|item| item.handle_key(key))
                .is_some_and(|event| {
                    events.push(event);
                    true
                }),
        };
        if !handled {
            match controller.mode {
                PickerMode::FilteredCommands => match key.code {
                    KeyCode::Char(_)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        events.push(EditorEvent::EditCommand(key));
                    }
                    KeyCode::Backspace | KeyCode::Delete => {
                        events.push(EditorEvent::EditCommand(key));
                    }
                    KeyCode::Esc => events.push(EditorEvent::FinishCommand),
                    _ => {}
                },
                PickerMode::Commands => match key.code {
                    KeyCode::Esc => events.push(EditorEvent::ClosePicker),
                    KeyCode::Char(character)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        events.push(EditorEvent::ResumeTyping(character));
                    }
                    _ => {}
                },
                PickerMode::Settings | PickerMode::Style => match key.code {
                    KeyCode::Esc => events.push(EditorEvent::ClosePicker),
                    KeyCode::Char(character)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        events.push(EditorEvent::ResumeTyping(character));
                    }
                    _ => {}
                },
                PickerMode::Keymaps => match key.code {
                    KeyCode::Esc => events.push(EditorEvent::ClosePicker),
                    KeyCode::Char(character)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        events.push(EditorEvent::ResumeTyping(character));
                    }
                    _ => {}
                },
            }
        }
        for event in events {
            let action = self.handle_picker_event(event);
            if !matches!(action, EditorAction::Continue) {
                return action;
            }
        }
        self.refresh_status_line();
        EditorAction::Continue
    }

    fn open_settings_from_commands(&mut self) {
        if matches!(
            self.pickers.last().map(|controller| controller.mode),
            Some(PickerMode::FilteredCommands)
        ) {
            self.input.clear();
        }
        self.pickers.push(PickerController {
            mode: PickerMode::Settings,
            picker: self.settings_picker(),
        });
        self.refresh_status_line();
    }

    fn open_style_from_commands(&mut self) {
        if matches!(
            self.pickers.last().map(|controller| controller.mode),
            Some(PickerMode::FilteredCommands)
        ) {
            self.input.clear();
        }
        self.open_style();
    }

    fn open_keymaps_from_commands(&mut self) {
        if matches!(
            self.pickers.last().map(|controller| controller.mode),
            Some(PickerMode::FilteredCommands)
        ) {
            self.input.clear();
        }
        self.pickers.push(PickerController {
            mode: PickerMode::Keymaps,
            picker: self.keymaps_picker(),
        });
        self.refresh_status_line();
    }

    fn open_settings(&mut self) {
        self.pickers.clear();
        self.pickers.push(PickerController {
            mode: PickerMode::Settings,
            picker: self.settings_picker(),
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

    fn handle_picker_event(&mut self, event: EditorEvent) -> EditorAction {
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
                self.keymap = Keymap::default();
                self.apply_appearance(Appearance::default());
                let items = self.shell_items();
                self.pickers.last_mut().unwrap().picker.set_items(items);
            }
            EditorEvent::SetSettings(settings) => {
                self.settings = settings;
                let items = self.shell_items();
                self.pickers.last_mut().unwrap().picker.set_items(items);
            }
            EditorEvent::SetAppearance(appearance) => {
                self.apply_appearance(appearance);
                let items = Self::appearance_items(appearance);
                self.pickers.last_mut().unwrap().picker.set_items(items);
            }
            EditorEvent::Rebind(action, key) => {
                if self.keymap.rebind(action, key) {
                    let items = self.keymap_items();
                    self.pickers.last_mut().unwrap().picker.set_items(items);
                }
            }
        }
        EditorAction::Continue
    }

    fn apply_appearance(&mut self, appearance: Appearance) {
        self.appearance = appearance;
        self.groups = appearance.groups();
    }

    fn handle_debug_key(&mut self, key: KeyEvent) -> bool {
        if self.keymap.action(key) == Some(Action::ToggleHighlightDebug) {
            self.toggle_highlight_debug();
            return true;
        }
        if self.highlight_debug && key.code == KeyCode::Esc {
            self.highlight_debug = false;
            return true;
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
                _ => return false,
            }
            return true;
        }
        false
    }

    fn toggle_highlight_debug(&mut self) {
        self.highlight_debug = !self.highlight_debug;
        self.highlight_debug_scroll = 0;
    }

    fn draw_highlight_debug(&mut self, frame: &mut Frame) {
        let available = self.feed_area;
        if !self.highlight_debug || available.width < 4 || available.height < 3 {
            return;
        }
        let mut entries = vec![("palette".to_string(), self.groups.prompt().bold())];
        entries.extend(
            self.appearance
                .variant()
                .native_colors()
                .iter()
                .map(|color| {
                    (
                        color.name.to_string(),
                        ratatui::style::Style::new().fg(color.color),
                    )
                }),
        );
        entries.push(("highlight groups".to_string(), self.groups.prompt().bold()));
        entries.extend(
            self.groups
                .highlight_groups()
                .into_iter()
                .map(|(name, style)| (name.to_string(), style)),
        );
        let title = format!(
            " {} {} · debug ",
            self.appearance.scheme().label(),
            self.appearance.variant().label()
        );
        let width = entries
            .iter()
            .map(|(name, _)| name.chars().count())
            .chain([title.chars().count()])
            .max()
            .unwrap_or(1)
            .saturating_add(2)
            .min(usize::from(available.width)) as u16;
        let content_width = usize::from(width - 2);
        let visible = usize::from(available.height - 2).min(entries.len());
        self.highlight_debug_scroll = self
            .highlight_debug_scroll
            .min(entries.len().saturating_sub(visible));
        let lines = entries
            .into_iter()
            .skip(self.highlight_debug_scroll)
            .take(visible)
            .map(|(label, style)| {
                let truncated = label.chars().count() > content_width;
                let mut label: String = label.chars().take(content_width).collect();
                if truncated && content_width > 1 {
                    label.pop();
                    label.push('…');
                }
                Line::from(Span::styled(format!("{label:<content_width$}"), style))
            })
            .collect::<Vec<_>>();
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
            .keymap
            .key_for(Action::OpenCommands)
            .unwrap_or_default();
        let settings_key = self
            .keymap
            .key_for(Action::OpenSettings)
            .unwrap_or_default();
        let event = match self.pickers.last() {
            None if self.input.text().is_empty() => StatusLineEvent::Show {
                alignment: Alignment::Right,
                spans: vec![
                    Span::styled(format!("{commands_key} "), self.groups.prompt()),
                    Span::styled("commands · ", self.groups.muted()),
                    Span::styled(format!("{settings_key} "), self.groups.prompt()),
                    Span::styled("settings", self.groups.muted()),
                ],
            },
            None => StatusLineEvent::Show {
                alignment: Alignment::Right,
                spans: vec![
                    Span::styled(format!("{settings_key} "), self.groups.prompt()),
                    Span::styled("settings", self.groups.muted()),
                ],
            },
            Some(controller) => controller.picker.status_event(&self.groups),
        };
        self.status_line.handle(event);
    }

    pub fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        highlighter: &mut dyn Highlighter,
        next_input_number: u32,
        in_debugger: bool,
    ) {
        self.selection_exclusions.clear();
        let theme = self.groups;
        let feed_gutter = gutter_width(next_input_number);
        let input_gutter = prompt_gutter_width();
        frame.render_widget(Clear, area);

        let editor_area = area;

        let input_height = self.input_height(editor_area.width, input_gutter, editor_area.height);
        let status_height = u16::from(self.settings.statusline != StatusLinePosition::Off);
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
        let animation_frame = (self.animation_started.elapsed().as_millis() / 80) as usize;
        let context = RenderContext::new(editor_area.width, feed_gutter, &theme)
            .debugging(in_debugger)
            .animation_frame(animation_frame);
        let input_context =
            RenderContext::new(editor_area.width, input_gutter, &theme).debugging(in_debugger);

        self.feed_area = sections[0];
        self.draw_feed(frame, sections[0], &context);
        if let Some(controller) = self.pickers.last() {
            controller
                .picker
                .render(sections[1], frame.buffer_mut(), &theme);
            self.selection_exclusions.push(sections[1]);
        }
        self.input_area = input_area;
        self.selection_exclusions.push(input_area);
        self.draw_input(frame, input_area, &input_context, highlighter, self.focused);
        TuiBlock::default()
            .style(theme.transcript())
            .render(status_area, frame.buffer_mut());
        self.status_line.render(
            status_area,
            frame.buffer_mut(),
            self.macaulay_version
                .clone()
                .map(|version| Span::styled(version, theme.muted())),
            None,
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
        committed: &[CommittedCell],
        view: ViewKind,
        viewport: &mut Viewport,
    ) {
        self.selection_exclusions.clear();
        self.input_area = Rect::ZERO;
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
            .map_or(0, |controller| controller.picker.height())
            .min(area.height.saturating_sub(1));
        let sections = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(picker_height),
            Constraint::Length(area.height.min(1)),
        ])
        .split(area);
        let gutter = gutter_width(committed.last().map_or(1, |cell| cell.number));
        let context = RenderContext::new(sections[0].width, gutter, &theme)
            .animation_frame((self.animation_started.elapsed().as_millis() / 80) as usize);
        self.feed_area = sections[0];

        match view {
            ViewKind::Shell => unreachable!("shells use Editor::draw"),
            ViewKind::Code => {
                let code = (!committed.is_empty()).then(|| {
                    CodeBlock::new(committed.iter().map(|cell| {
                        Cell::input(
                            cell.code.clone(),
                            cell.number,
                            highlighter.highlight(&cell.code),
                        )
                    }))
                });
                let blocks: Vec<&dyn Block> = code
                    .as_ref()
                    .map(|block| vec![block as &dyn Block])
                    .unwrap_or_default();
                render_preview_blocks(
                    frame,
                    sections[0],
                    &context,
                    &blocks,
                    &mut viewport.scroll,
                    "no committed code",
                );
            }
            ViewKind::SyntaxTree => {
                let blocks: Vec<Box<dyn Block>> = committed
                    .iter()
                    .map(|cell| {
                        Box::new(SyntaxTreeBlock::new(
                            cell.code.clone(),
                            cell.number,
                            highlighter.highlight(&cell.code),
                            cell.tree.clone(),
                        )) as Box<dyn Block>
                    })
                    .collect();
                let blocks: Vec<&dyn Block> = blocks.iter().map(Box::as_ref).collect();
                render_preview_blocks(
                    frame,
                    sections[0],
                    &context,
                    &blocks,
                    &mut viewport.scroll,
                    "no committed syntax trees",
                );
            }
            ViewKind::Output => {
                let blocks: Vec<&dyn Block> = self
                    .feed
                    .iter()
                    .flat_map(|block| {
                        let top = (viewport.output == OutputMode::Everything
                            && block.kind() != BlockKind::Input)
                            .then_some(block.as_ref());
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
                render_preview_blocks(
                    frame,
                    sections[0],
                    &context,
                    &blocks,
                    &mut viewport.scroll,
                    "no matching output",
                );
            }
        }
        self.feed_top = viewport.scroll;
        self.selection_exclusions.extend([sections[1], sections[2]]);
        TuiBlock::default()
            .style(theme.transcript())
            .render(sections[2], frame.buffer_mut());
        let center = (view == ViewKind::SyntaxTree).then(|| {
            Line::from(Span::styled(
                "Syntax Tree Preview Mode",
                theme.preview_mode(),
            ))
        });
        if let Some(controller) = self.pickers.last() {
            controller
                .picker
                .render(sections[1], frame.buffer_mut(), &theme);
            self.status_line
                .render(sections[2], frame.buffer_mut(), None, center);
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
                    Span::styled("←", theme.prompt()),
                    Span::styled(format!(" {} ", viewport.output.label()), theme.muted()),
                    Span::styled("→", theme.prompt()),
                    Span::styled(" · ", theme.muted()),
                ]);
            }
            right.extend([
                Span::styled("ctrl+q ", theme.prompt()),
                Span::styled(format!("{action} view"), theme.muted()),
            ]);
            self.status_line.render_parts(
                sections[2],
                frame.buffer_mut(),
                title
                    .map(|title| {
                        Span::styled(
                            title,
                            if self.focused {
                                theme.value()
                            } else {
                                theme.muted()
                            },
                        )
                    })
                    .into_iter()
                    .collect(),
                center,
                right,
            );
        }
        self.draw_highlight_debug(frame);
        self.capture_screen(frame);
        self.paint_screen_selection(frame);
    }

    fn capture_screen(&mut self, frame: &mut Frame) {
        let area = frame.area();
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
            let content_row = self.feed_top + usize::from(y - area.y);
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

    /// The input pane's height: enough for the wrapped source and its margins,
    /// but never more than the configured share of the editor, past which the
    /// input scrolls internally rather than crowding out the feed.
    fn input_height(&self, width: u16, gutter: u16, editor_height: u16) -> u16 {
        let rows = self.input.visual_rows(width.saturating_sub(gutter)) as u16;
        let chrome = 2 + u16::from(self.input.diagnostic().is_some());
        let cap = editor_height
            .saturating_mul(u16::from(self.settings.max_input_height_percent))
            .saturating_div(100)
            .max(MIN_INPUT_HEIGHT);
        rows.saturating_add(chrome).clamp(MIN_INPUT_HEIGHT, cap)
    }

    /// The first visual row to draw, chosen so the cursor keeps `input_scrolloff`
    /// rows of context from both edges. With a viewport too short to honour both
    /// margins the cursor is centred instead, which degrades smoothly rather than
    /// fighting itself between the two clamps.
    fn input_scroll(&self, cursor_row: usize, rows: usize, viewport: usize) -> usize {
        let last_start = rows.saturating_sub(viewport);
        let scrolloff = usize::from(self.settings.input_scrolloff);
        if viewport <= 2 * scrolloff + 1 {
            return cursor_row.saturating_sub(viewport / 2).min(last_start);
        }
        let highest = cursor_row.saturating_sub(scrolloff);
        let lowest = (cursor_row + scrolloff + 1).saturating_sub(viewport);
        self.input_scroll.clamp(lowest, highest).min(last_start)
    }

    fn draw_feed(&mut self, frame: &mut Frame, area: Rect, context: &RenderContext<'_>) -> bool {
        if area.width == 0 || area.height == 0 {
            self.feed_top = 0;
            return false;
        }

        let heights: Vec<u16> = self
            .feed
            .iter()
            .map(|block| block.height(context))
            .collect();
        let total: u16 = heights.iter().sum();
        let buffer_height = total.max(area.height).max(1);
        let mut composition = Buffer::empty(Rect::new(0, 0, area.width, buffer_height));
        let composition_area = *composition.area();
        TuiBlock::default()
            .style(self.appearance.canvas(context.groups().transcript()))
            .render(composition_area, &mut composition);

        // An underfilled transcript grows upward from the input. Once full, the
        // leading offset is zero and the existing scrolling path is unchanged.
        let mut y = buffer_height - total;
        let mut feed_exclusions = Vec::new();
        for (block, height) in self.feed.iter().zip(&heights) {
            let block_area = Rect::new(0, y, area.width, *height);
            block.render(context, block_area, &mut composition);
            block.selection_exclusions(context, block_area, &mut feed_exclusions);
            y += height;
        }

        let visible = area.height as usize;
        let max_scroll = (total as usize).saturating_sub(visible);
        // Park the offset at the top of the feed rather than letting it run past
        // it, so scrolling back down moves on the first notch instead of after as
        // many notches as were spent overshooting.
        self.scroll_offset = self.scroll_offset.min(max_scroll);
        let top = max_scroll - self.scroll_offset;
        self.feed_top = top;
        let destination = frame.buffer_mut();
        for row in 0..area.height {
            let source_y = top + row as usize;
            if source_y >= buffer_height as usize {
                break;
            }
            for column in 0..area.width {
                if let Some(cell) = composition.cell((column, source_y as u16)) {
                    if let Some(slot) = destination.cell_mut((area.x + column, area.y + row)) {
                        *slot = cell.clone();
                    }
                }
            }
        }
        let visible_bottom = top + usize::from(area.height);
        self.selection_exclusions
            .extend(feed_exclusions.into_iter().filter_map(|excluded| {
                let source_start = usize::from(excluded.y).max(top);
                let source_end = usize::from(excluded.bottom()).min(visible_bottom);
                (source_start < source_end).then(|| {
                    Rect::new(
                        area.x + excluded.x,
                        area.y + (source_start - top) as u16,
                        excluded.width.min(area.width.saturating_sub(excluded.x)),
                        (source_end - source_start) as u16,
                    )
                })
            }));
        total < area.height
    }

    fn draw_input(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        context: &RenderContext<'_>,
        highlighter: &mut dyn Highlighter,
        focused: bool,
    ) {
        let diagnostic = self.input.diagnostic().map(str::to_string);
        let code = context.groups().code();
        let text_width = area.width.saturating_sub(context.gutter());
        let mut rows = self.input.highlighted_lines(context, highlighter);
        if self
            .input
            .text()
            .strip_prefix('/')
            .is_some_and(|query| !commands::matching(query, self.split).is_empty())
        {
            for span in rows
                .iter_mut()
                .flat_map(|line| line.spans.iter_mut().skip(1))
            {
                span.style = span.style.patch(context.groups().prompt());
            }
        }

        let chrome = 2 + u16::from(diagnostic.is_some());
        let viewport = area.height.saturating_sub(chrome).max(1) as usize;
        let (cursor_row, cursor_column) = self.input.visual_cursor(text_width);
        self.input_scroll = self.input_scroll(cursor_row, rows.len(), viewport);
        self.input_text_area = Rect::new(
            area.x + context.gutter(),
            area.y + 1,
            text_width,
            viewport as u16,
        );

        let mut lines = if context.is_debugging() {
            let warning = context.groups().warning().bold().patch(code);
            vec![Line::from(vec![
                gutter_span("", context.gutter(), warning),
                Span::styled("DEBUGGER", warning),
            ])]
        } else {
            vec![Line::default()]
        };
        lines.extend(rows.into_iter().skip(self.input_scroll).take(viewport));
        if let Some(message) = diagnostic {
            let error = context.groups().error().patch(code);
            lines.push(Line::from(vec![
                gutter_span("", context.gutter(), error),
                Span::styled(message, error),
            ]));
        }
        lines.push(Line::default());
        frame.render_widget(Paragraph::new(lines).style(code), area);

        if focused {
            frame.set_cursor_position(Position::new(
                area.x + context.gutter() + cursor_column as u16,
                area.y + 1 + (cursor_row - self.input_scroll) as u16,
            ));
        }
    }
}

fn render_preview_blocks(
    frame: &mut Frame,
    area: Rect,
    context: &RenderContext<'_>,
    blocks: &[&dyn Block],
    scroll: &mut usize,
    empty: &str,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if blocks.is_empty() {
        frame.render_widget(
            Paragraph::new(empty)
                .style(context.groups().muted())
                .alignment(Alignment::Center),
            area,
        );
        return;
    }
    let heights: Vec<_> = blocks.iter().map(|block| block.height(context)).collect();
    let total: u16 = heights.iter().sum();
    let mut composition = Buffer::empty(Rect::new(0, 0, area.width, total.max(1)));
    TuiBlock::default()
        .style(context.groups().transcript())
        .render(*composition.area(), &mut composition);
    let mut y = 0;
    for (block, height) in blocks.iter().zip(heights) {
        block.render(
            context,
            Rect::new(0, y, area.width, height),
            &mut composition,
        );
        y += height;
    }
    let max_scroll = usize::from(total.saturating_sub(area.height));
    *scroll = (*scroll).min(max_scroll);
    for row in 0..area.height {
        let source_y = *scroll + usize::from(row);
        if source_y >= usize::from(total) {
            break;
        }
        for column in 0..area.width {
            if let Some(cell) = composition.cell((column, source_y as u16)) {
                if let Some(slot) = frame.buffer_mut().cell_mut((area.x + column, area.y + row)) {
                    *slot = cell.clone();
                }
            }
        }
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
    let content_start =
        marker_content_start(symbols, segment_start, segment_end).unwrap_or_else(|| {
            symbols[segment_start..=last_content]
                .iter()
                .position(|symbol| !symbol.trim().is_empty())
                .map_or(segment_start, |index| segment_start + index)
        });
    if content_start <= last_content {
        selectable[content_start..=last_content].fill(true);
    }
}

fn marker_content_start(
    symbols: &[String],
    segment_start: usize,
    segment_end: usize,
) -> Option<usize> {
    let marker_start = segment_start + 2;
    let marker = symbols.get(marker_start)?.as_str();
    if !matches!(marker, "i" | "o") {
        return None;
    }
    let mut cursor = marker_start + 1;
    let digits_start = cursor;
    while cursor < segment_end
        && symbols[cursor]
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        cursor += 1;
    }
    if cursor == digits_start {
        return None;
    }
    for _ in 0..2 {
        if cursor >= segment_end || !symbols[cursor].trim().is_empty() {
            return None;
        }
        cursor += 1;
    }
    Some(cursor)
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
    use m2_syn::PrettyNode;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    struct EmptyHighlighter;

    impl Highlighter for EmptyHighlighter {
        fn highlight(&mut self, _source: &str) -> Vec<crate::code::highlight::HLSpan> {
            Vec::new()
        }
    }

    struct OccupiedFeed;

    impl Block for OccupiedFeed {
        fn height(&self, _context: &RenderContext<'_>) -> u16 {
            7
        }

        fn render(&self, _context: &RenderContext<'_>, area: Rect, buf: &mut Buffer) {
            Paragraph::new("feed").render(
                Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
                buf,
            );
        }
    }

    fn rendered_buffer(editor: &mut Editor, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut highlighter = EmptyHighlighter;
        terminal
            .draw(|frame| editor.draw(frame, frame.area(), &mut highlighter, 1, false))
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
                    committed,
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
        assert_eq!(editor.scroll_offset, 1);
        editor.handle_mouse(wheel(MouseEventKind::ScrollDown, 0));
        assert_eq!(editor.scroll_offset, 0);
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

        let copied = editor.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
        let EditorAction::Copy(text) = copied else {
            panic!("copy binding did not receive the selection");
        };
        assert_eq!(text, "cd");
    }

    #[test]
    fn mouse_drag_selects_input_text() {
        let mut editor = Editor::new(String::new());
        editor.input.field_mut().insert_str("abcd");
        editor.input_area = Rect::new(0, 5, 20, 3);
        editor.input_text_area = Rect::new(5, 6, 15, 1);

        editor.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 6));
        editor.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 8, 6));
        let copied = editor.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 8, 6));

        assert_eq!(editor.input.field().selected_text(), Some("abc"));
        let EditorAction::Copy(text) = copied else {
            panic!("mouse release did not copy the input selection");
        };
        assert_eq!(text, "abc");
    }

    #[test]
    fn mouse_drag_selects_rendered_feed_text_for_copying() {
        let mut editor = Editor::new("hello".to_string());
        let rows = rendered_editor(&mut editor, 40, 10);
        let row = rows.iter().position(|row| row.contains("hello")).unwrap() as u16;

        editor.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 6, row));
        editor.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 10, row));
        let copied = editor.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 10, row));
        let EditorAction::Copy(text) = copied else {
            panic!("mouse release did not copy the screen selection");
        };
        assert_eq!(text, "hello");
    }

    #[test]
    fn mouse_selection_skips_output_markers_and_types() {
        let mut editor = Editor::new(String::new());
        editor.feed = vec![Box::new(crate::ui::block::OutputBlock {
            number: Some(1),
            text: "value".to_string(),
            ty: Some("ZZ".to_string()),
        })];
        let rows = rendered_editor(&mut editor, 40, 10);
        let row = rows.iter().position(|row| row.contains("value")).unwrap() as u16;

        editor.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 0, row));
        editor.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 39, row));
        let copied = editor.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 39, row));

        let EditorAction::Copy(text) = copied else {
            panic!("mouse release did not copy the output selection");
        };
        assert_eq!(text, "value");
    }

    #[test]
    fn underfilled_feed_grows_upward_from_the_input() {
        let mut editor = Editor::new("first".to_string());
        let rows = rendered_editor(&mut editor, 40, 12);
        let first = rows.iter().position(|row| row.contains("first")).unwrap();

        assert!(rows[..first].iter().all(|row| row.trim().is_empty()));
        editor.feed.push(Box::new(TextBlock::system("second")));
        let rows = rendered_editor(&mut editor, 40, 12);
        let moved = rows.iter().position(|row| row.contains("first")).unwrap();
        let appended = rows.iter().position(|row| row.contains("second")).unwrap();

        assert!(moved < first);
        assert!(appended > moved && appended < editor.input_area.y as usize);
    }

    #[test]
    fn scrolling_preserves_the_feed_selection() {
        let mut editor = Editor::new(
            (0..10)
                .map(|line| format!("line{line}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        rendered_editor(&mut editor, 40, 10);
        let original_top = editor.feed_top;
        editor.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 6, 1));
        editor.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 10, 1));
        editor.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 10, 1));
        let selected_row = editor.screen_selection.as_ref().unwrap().anchor.row;

        editor.handle_mouse(wheel(MouseEventKind::ScrollUp, 0));
        rendered_editor(&mut editor, 40, 10);

        assert_eq!(editor.feed_top, original_top - 1);
        assert_eq!(
            editor.screen_selection.as_ref().unwrap().anchor.row,
            selected_row
        );
        let copied = editor.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
        let EditorAction::Copy(text) = copied else {
            panic!("scrolling cleared the feed selection");
        };
        assert_eq!(text, format!("line{selected_row}"));
    }

    #[test]
    fn selectable_cells_exclude_markers_and_box_drawing() {
        let symbols: Vec<String> = "  o12  value │"
            .chars()
            .map(|character| character.to_string())
            .collect();
        let mask = selectable_columns(&symbols, &vec![true; symbols.len()]);
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
        editor.input_area = Rect::new(0, 10, 40, 4);
        let before = editor.settings.max_input_height_percent;

        editor.handle_mouse(wheel(MouseEventKind::ScrollUp, 11));
        assert_eq!(editor.settings.max_input_height_percent, before + 5);
        assert_eq!(editor.scroll_offset, 0, "the feed must not move");

        editor.handle_mouse(wheel(MouseEventKind::ScrollDown, 11));
        assert_eq!(editor.settings.max_input_height_percent, before);
    }

    #[test]
    fn settings_picker_adjusts_the_selected_field() {
        let mut editor = Editor::new(String::new());
        let before = editor.settings.input_scrolloff;

        open_settings(&mut editor);
        editor.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        editor.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));

        assert_eq!(editor.settings.input_scrolloff, before + 1);
        assert_eq!(editor.input.text(), "", "the setting consumes the key");

        editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        editor.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(editor.pickers.is_empty());
    }

    #[test]
    fn max_input_height_changes_by_five_percentage_points() {
        let mut editor = Editor::new(String::new());
        let before = editor.settings.max_input_height_percent;

        open_settings(&mut editor);
        press(&mut editor, KeyCode::Right, KeyModifiers::NONE);

        assert_eq!(editor.settings.max_input_height_percent, before + 5);
    }

    #[test]
    fn restore_default_resets_shell_keymaps_and_style_on_enter() {
        let mut editor = Editor::new(String::new());
        let rebound = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        editor.settings.max_input_height_percent = 40;
        assert!(editor.keymap.rebind(Action::ClearInput, rebound));
        editor.apply_appearance(editor.appearance.cycle_colorscheme(1));
        editor.open_settings();

        press(&mut editor, KeyCode::Down, KeyModifiers::NONE);
        press(&mut editor, KeyCode::Down, KeyModifiers::NONE);
        press(&mut editor, KeyCode::Down, KeyModifiers::NONE);
        press(&mut editor, KeyCode::Down, KeyModifiers::NONE);
        press(&mut editor, KeyCode::Enter, KeyModifiers::NONE);

        let defaults = Settings::default();
        assert_eq!(
            editor.settings.max_input_height_percent,
            defaults.max_input_height_percent
        );
        assert_eq!(editor.settings.input_scrolloff, defaults.input_scrolloff);
        assert!(editor.keymap.action(rebound).is_none());
        assert_eq!(editor.appearance, Appearance::default());
    }

    fn press(editor: &mut Editor, code: KeyCode, modifiers: KeyModifiers) {
        editor.handle_key(KeyEvent::new(code, modifiers));
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
            selected: false,
            groups: &groups,
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
    fn closing_style_keeps_changes() {
        let mut editor = Editor::new(String::new());
        let original = editor.appearance;

        open_style(&mut editor);
        press(&mut editor, KeyCode::Up, KeyModifiers::NONE);
        press(&mut editor, KeyCode::Right, KeyModifiers::NONE);
        assert_ne!(editor.appearance, original);

        press(&mut editor, KeyCode::Esc, KeyModifiers::NONE);
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
    fn keymap_picker_only_remaps_explicit_commands() {
        let mut editor = Editor::new(String::new());
        let labels: Vec<_> = editor
            .keymap_items()
            .iter()
            .map(|row| item_text(row.as_ref()))
            .collect();
        assert!(labels[0].contains("clear input text"));
        assert!(labels[1].contains("copy the selection"));

        open_keymaps(&mut editor);

        assert_eq!(
            editor.pickers.last().unwrap().picker.height(),
            Action::REMAPPABLE.len() as u16 + 2
        );
        press(&mut editor, KeyCode::Char('x'), KeyModifiers::CONTROL);
        press(&mut editor, KeyCode::Esc, KeyModifiers::NONE);
        press(&mut editor, KeyCode::Esc, KeyModifiers::NONE);
        editor.input.field_mut().insert_str("something");

        press(&mut editor, KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert!(editor.input.text().is_empty());
    }

    // Quitting must not require dismissing a picker first.
    #[test]
    fn quit_works_with_a_picker_open() {
        let mut editor = Editor::new(String::new());
        press(&mut editor, KeyCode::Char('/'), KeyModifiers::NONE);

        let action = editor.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(matches!(action, EditorAction::Quit));
    }

    #[test]
    fn typing_closes_settings_and_style_and_reaches_the_input() {
        let mut editor = Editor::new(String::new());
        editor.input.field_mut().insert_char('x');
        editor.open_settings();
        press(&mut editor, KeyCode::Char('y'), KeyModifiers::NONE);
        assert!(editor.pickers.is_empty());
        assert_eq!(editor.input.text(), "xy");

        editor.open_style();
        press(&mut editor, KeyCode::Char('z'), KeyModifiers::NONE);
        assert!(editor.pickers.is_empty());
        assert_eq!(editor.input.text(), "xyz");

        let mut editor = Editor::new(String::new());
        open_keymaps(&mut editor);
        press(&mut editor, KeyCode::Char('y'), KeyModifiers::NONE);
        assert!(editor.pickers.is_empty());
        assert_eq!(editor.input.text(), "y");
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
        assert_eq!(picker.height(), 3);
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
        let action = editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(editor.pickers.is_empty());
        assert!(matches!(
            action,
            EditorAction::Open(ViewKind::Code, OpenTarget::Current)
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

        for character in "/sy".chars() {
            press(&mut editor, KeyCode::Char(character), KeyModifiers::NONE);
        }
        let action = editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            action,
            EditorAction::Open(ViewKind::SyntaxTree, OpenTarget::Current)
        ));

        press(&mut editor, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(editor.input.text(), "/syntax tree");

        let action = editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            action,
            EditorAction::Open(ViewKind::SyntaxTree, OpenTarget::Current)
        ));
        assert!(editor.input.text().is_empty());
    }

    #[test]
    fn debug_command_opens_the_overlay_and_enters_history() {
        let mut editor = Editor::new(String::new());
        editor.input = InputCode::new_in_memory();

        for character in "/de".chars() {
            press(&mut editor, KeyCode::Char(character), KeyModifiers::NONE);
        }
        press(&mut editor, KeyCode::Enter, KeyModifiers::NONE);
        assert!(editor.highlight_debug);
        assert!(editor.pickers.is_empty());

        press(&mut editor, KeyCode::Esc, KeyModifiers::NONE);
        assert!(!editor.highlight_debug);
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
        let action = editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            action,
            EditorAction::Open(ViewKind::Code, OpenTarget::Current)
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
    fn command_shortcuts_are_aligned_at_the_right_edge() {
        let mut editor = Editor::new(String::new());
        press(&mut editor, KeyCode::Char('/'), KeyModifiers::NONE);

        let rows = rendered_editor(&mut editor, 70, 16);
        let settings = rows
            .iter()
            .find(|row| row.contains("settings") && row.contains("ctrl+o"))
            .unwrap_or_else(|| panic!("settings command missing from {rows:#?}"));
        let quit = rows
            .iter()
            .find(|row| row.contains("quit") && row.contains("ctrl+c"))
            .unwrap_or_else(|| panic!("quit command missing from {rows:#?}"));

        assert!(!settings.contains("configure spectral-m2"));
        assert!(!quit.contains("leave spectral-m2"));
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
        editor.open_settings();

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

        SettingField::StatusLine.adjust(&mut settings, 1);
        assert_eq!(settings.statusline, StatusLinePosition::AboveInput);
        SettingField::StatusLine.adjust(&mut settings, 1);
        assert_eq!(settings.statusline, StatusLinePosition::Off);
        SettingField::StatusLine.adjust(&mut settings, 1);
        assert_eq!(settings.statusline, StatusLinePosition::BelowInput);
        SettingField::StatusLine.adjust(&mut settings, -1);
        assert_eq!(settings.statusline, StatusLinePosition::Off);
    }

    #[test]
    fn statusline_can_move_above_input_or_release_its_row() {
        let mut editor = Editor::new(String::new());
        editor.feed.clear();
        editor.settings.statusline = StatusLinePosition::AboveInput;
        let rows = rendered_editor(&mut editor, 40, 10);

        assert!(rows[editor.input_area.y as usize].trim().is_empty());
        assert!(rows[editor.input_area.y.saturating_sub(1) as usize].contains("ctrl+/ commands"));

        editor.settings.statusline = StatusLinePosition::Off;
        let rows = rendered_editor(&mut editor, 40, 10);

        assert_eq!(editor.input_area.bottom(), 10);
        assert!(rows.iter().all(|row| !row.contains("ctrl+/ commands")));
    }

    #[test]
    fn status_line_is_right_aligned_below_empty_input() {
        let mut editor = Editor::new(String::new());
        editor.feed.clear();

        let rows = rendered_editor(&mut editor, 40, 10);
        let status = rows.last().unwrap();
        let top_margin = &rows[editor.input_area.y as usize];
        let bottom_margin = &rows[editor.input_area.bottom().saturating_sub(1) as usize];

        assert!(status
            .trim_end()
            .ends_with("ctrl+/ commands · ctrl+o settings"));
        assert!(status.starts_with("  "));
        assert!(status.ends_with("  "));
        assert!(top_margin.trim().is_empty());
        assert!(
            bottom_margin.trim().is_empty(),
            "bottom margin {bottom_margin:?} in {:?}",
            editor.input_area
        );
    }

    #[test]
    fn highlight_debug_lists_ui_and_code_groups_at_the_lower_right() {
        let mut editor = Editor::new(String::new());
        press(&mut editor, KeyCode::Char('d'), KeyModifiers::CONTROL);

        let rows = rendered_editor(&mut editor, 120, 120);
        let screen = rows.join("\n");
        let title_row = rows.iter().position(|row| row.contains("debug")).unwrap();
        assert!(screen.contains("bg_dim"));
        assert!(screen.contains("ui.transcript"));
        assert!(screen.contains("m2.PunctuationDelimiter"));
        assert!(title_row > 0);
        assert!(rows[title_row].ends_with('┐'));
        assert_eq!(
            rows.iter().find_map(|row| row.find("bg_dim")).unwrap(),
            rows.iter()
                .find_map(|row| row.find("ui.transcript"))
                .unwrap(),
        );
        assert_eq!(
            rows.iter()
                .find_map(|row| row.find("ui.transcript"))
                .unwrap(),
            rows.iter()
                .find_map(|row| row.find("m2.PunctuationDelimiter"))
                .unwrap()
        );

        press(&mut editor, KeyCode::Esc, KeyModifiers::NONE);
        assert!(!rendered_editor(&mut editor, 120, 120)
            .join("\n")
            .contains("ui.transcript"));
    }

    #[test]
    fn upper_input_margin_keeps_the_input_band_background() {
        let mut editor = Editor::new(String::new());
        let backend = TestBackend::new(70, 20);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut highlighter = EmptyHighlighter;

        terminal
            .draw(|frame| editor.draw(frame, frame.area(), &mut highlighter, 1, false))
            .expect("draw editor");

        let margin = &terminal.backend().buffer()[(0, editor.input_area.y)];
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
        assert_eq!(buffer[content].bg, editor.groups.transcript().bg.unwrap());
        assert_eq!(
            buffer[(0, editor.input_area.y)].bg,
            editor.groups.code().bg.unwrap()
        );
        assert_eq!(
            buffer[(0, buffer.area().bottom() - 1)].bg,
            editor.groups.transcript().bg.unwrap()
        );
    }

    #[test]
    fn slash_commands_use_the_prompt_highlight_in_the_input() {
        let mut editor = Editor::new(String::new());
        editor.input.field_mut().insert_str("/style");
        let buffer = rendered_buffer(&mut editor, 70, 20);
        let foreground = editor.groups.prompt().fg.unwrap();

        for x in editor.input_text_area.x..editor.input_text_area.x + 6 {
            assert_eq!(buffer[(x, editor.input_text_area.y)].fg, foreground);
        }
    }

    #[test]
    fn idle_left_status_shows_the_live_macaulay_version() {
        let mut editor = Editor::new(String::new()).with_macaulay_version("1.26.06".to_string());
        let rows = rendered_editor(&mut editor, 70, 10);

        assert_eq!(rows.last().unwrap().find("Macaulay2 v1.26.06"), Some(2));
        press(&mut editor, KeyCode::Char('/'), KeyModifiers::NONE);
        let rows = rendered_editor(&mut editor, 70, 10);
        assert!(!rows.last().unwrap().contains("Macaulay2"));
        press(&mut editor, KeyCode::Esc, KeyModifiers::NONE);
        let rows = rendered_editor(&mut editor, 70, 10);
        assert_eq!(rows.last().unwrap().find("Macaulay2 v1.26.06"), Some(2));
    }

    #[test]
    fn statusline_keymaps_use_accent_only_for_keys() {
        let mut editor = Editor::new(String::new());
        editor.open_settings();
        let backend = TestBackend::new(70, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut highlighter = EmptyHighlighter;
        terminal
            .draw(|frame| editor.draw(frame, frame.area(), &mut highlighter, 1, false))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let y = buffer.area().bottom() - 1;
        let row: String = (0..buffer.area().width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        let column = |text| {
            let byte = row.find(text).unwrap_or_else(|| panic!("{row:?}"));
            row[..byte].chars().count() as u16
        };
        let key = column("enter");
        let description = column("open");

        assert_eq!(buffer[(key, y)].fg, editor.groups.prompt().fg.unwrap());
        assert_eq!(
            buffer[(description, y)].fg,
            editor.groups.muted().fg.unwrap()
        );
        assert!(row.starts_with("  "));
        assert!(row.ends_with("  "));
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
        editor.feed = vec![Box::new(OccupiedFeed)];

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
        let action = editor.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(matches!(
            action,
            EditorAction::Open(ViewKind::Code, OpenTarget::Current)
        ));
        assert!(editor.submission().is_empty());
    }

    #[test]
    fn syntax_preview_uses_committed_code_and_the_m2_syn_tree_without_input() {
        let mut editor = Editor::new(String::new());
        editor.input.field_mut().insert_str("UNCOMMITTED_DRAFT");
        let committed = [CommittedCell {
            number: 3,
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
        assert!(rows.last().unwrap().contains("ctrl+q leave view"));
    }

    #[test]
    fn syntax_preview_centers_its_yellow_mode_banner_with_status_margins() {
        let mut editor = Editor::new(String::new());
        let committed = [CommittedCell {
            number: 1,
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
                    &committed,
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
        let key = row.find("ctrl+q").unwrap() as u16;

        assert_eq!(banner, (usize::from(buffer.area().width) - 24) / 2);
        assert_eq!(
            buffer[(banner as u16, y)].fg,
            editor.groups.preview_mode().fg.unwrap()
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
            let EditorAction::Copy(copied) = copied else {
                panic!("preview selection did not copy {text}");
            };
            assert_eq!(copied, text);
        }
    }

    #[test]
    fn output_preview_cycles_the_three_structured_filters() {
        use crate::ui::block::{OutputBlock, SubmissionBlock};

        let mut submission = SubmissionBlock::new(0, [("x".to_string(), Vec::new())]);
        submission.succeed(
            0,
            1,
            vec![
                Box::new(TextBlock::printed("PRINTED_ONLY")),
                Box::new(OutputBlock {
                    number: Some(1),
                    text: "VALUE_ONLY".to_string(),
                    ty: Some("TYPE_ONLY".to_string()),
                }),
            ],
        );
        let mut editor = Editor::new(String::new());
        editor.feed = vec![Box::new(submission)];
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
