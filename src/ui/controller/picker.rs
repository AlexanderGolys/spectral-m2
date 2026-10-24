//! Stateful text pickers rendered above the input.

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as TuiBlock, BorderType, Borders, Paragraph, Widget};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::settings::theme::groups::{scheme, HLGroup};
use super::EditorEvent;
use crate::ui::action::Action;
use crate::ui::block::{Element, TextSpan};
use crate::ui::config::UI;
use crate::ui::key::KeyEvent;
use crate::ui::wrap::{WordWrap, WrapPolicy};

/// Gutter in front of the selected row.
const SELECTED_MARKER: &str = " > ";
/// Gutter in front of every other row, as wide as [`SELECTED_MARKER`].
const MARKER: &str = "   ";
/// Between the key hints a picker gives the status line.
const HINT_SEPARATOR: &str = " · ";

/// Input delivered to the selected picker row after command lookup.
#[derive(Clone, Copy)]
pub enum PickerInput {
    /// A mapped application action whose physical key was discarded.
    Action(Action),
    /// Unmapped input interpreted privately by the picker.
    Key(KeyEvent),
}

/// One option a [`PickerValue::Choice`] offers.
pub struct PickerOption {
    pub label: String,
    /// Why choosing it takes no effect here, written after it; an option with
    /// one is drawn struck through.
    pub unavailable: Option<String>,
}

impl From<String> for PickerOption {
    fn from(label: String) -> Self {
        Self {
            label,
            unavailable: None,
        }
    }
}

impl From<&str> for PickerOption {
    fn from(label: &str) -> Self {
        label.to_string().into()
    }
}

/// A value closing its row, adjusted with the horizontal arrows.
pub enum PickerValue {
    /// One of a few named options, the chosen one shown between step arrows.
    Choice {
        options: Vec<PickerOption>,
        chosen: usize,
    },
    /// A value stepped through a range too long to list, between step arrows.
    Stepped(String),
}

impl PickerValue {
    /// The choice of `chosen` among `options`, each shown as `option` draws it.
    pub fn among<T: PartialEq>(
        options: &[T],
        chosen: &T,
        option: impl Fn(&T) -> PickerOption,
    ) -> Self {
        Self::Choice {
            options: options.iter().map(option).collect(),
            chosen: options
                .iter()
                .position(|option| option == chosen)
                .expect("the chosen option is among the options"),
        }
    }

    fn arrows() -> (String, String) {
        (
            format!("{} ", UI.glyphs.step_back.current()),
            format!(" {}", UI.glyphs.step_forward.current()),
        )
    }

    /// The value between step arrows lit on the selected row, within `room`
    /// columns; a choice that takes no effect here is struck through and followed
    /// by why.
    fn line(&self, room: usize, selected: bool) -> Line<'static> {
        let groups = scheme();
        let (shown, style, note) = match self {
            Self::Choice { options, chosen } => {
                let option = &options[*chosen];
                match &option.unavailable {
                    None => (&option.label, groups.settings_item_value(), None),
                    Some(note) => (
                        &option.label,
                        groups.settings_item_unavailable(),
                        Some(note),
                    ),
                }
            }
            Self::Stepped(value) => (value, groups.settings_item_value(), None),
        };
        let arrow = match selected {
            true => groups.picker_selected_marker(),
            false => groups.settings_step_arrow(),
        };
        let (back, forward) = Self::arrows();
        let note = note.map(|note| format!("{}{note}", " ".repeat(UI.picker_column_gap)));
        let room = room.saturating_sub(back.width() + forward.width());
        let note = note.filter(|note| shown.width() + note.width() <= room);
        let text = trimmed(shown, room);
        let mut spans = vec![
            Span::styled(back, arrow),
            Span::styled(text, style),
            Span::styled(forward, arrow),
        ];
        spans.extend(note.map(|note| Span::styled(note, groups.settings_unavailable_note())));
        Line::from(spans)
    }
}

/// `text` within `width` columns, its end giving way to the trimmed glyph.
fn trimmed(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let glyph = UI.glyphs.trimmed.current();
    let mut used = glyph.width();
    let kept = text
        .chars()
        .take_while(|character| {
            used += character.width().unwrap_or_default();
            used <= width
        })
        .collect::<String>();
    kept + glyph
}

/// A picker row: the cells it shows and the input it answers.
///
/// The picker aligns the n-th cell of every row into one column, so a row says only
/// what it shows, never where.
pub trait PickerItem: Send {
    /// Cell contents from left to right.
    fn cells(&self) -> Vec<Line<'static>>;

    /// The value after the cells, drawn to fit the room the row leaves it.
    fn value(&self) -> Option<&PickerValue> {
        None
    }

    /// What the item does, shown beneath the rows while it is selected.
    fn description(&self) -> Option<&str> {
        None
    }

    /// Whether this row is waiting for a shortcut, before picker navigation.
    fn capturing(&self) -> bool {
        false
    }

    fn handle(&mut self, input: PickerInput) -> Option<EditorEvent>;
}

/// A row of caller-styled cells with caller-supplied key behavior.
pub struct Row {
    cells: Vec<Line<'static>>,
    value: Option<PickerValue>,
    description: Option<&'static str>,
    handle: Box<dyn FnMut(PickerInput) -> Option<EditorEvent> + Send>,
}

impl Row {
    /// Create a row with arbitrary key handling.
    pub fn new(
        cells: Vec<Line<'static>>,
        handle: impl FnMut(PickerInput) -> Option<EditorEvent> + Send + 'static,
    ) -> Self {
        Self {
            cells,
            value: None,
            description: None,
            handle: Box::new(handle),
        }
    }

    /// Create a row adjusted by horizontal arrow keys and closed by Enter.
    pub fn adjust(
        cells: Vec<Line<'static>>,
        mut adjust: impl FnMut(i8) -> EditorEvent + Send + 'static,
    ) -> Self {
        Self::new(cells, move |input| match input {
            PickerInput::Key(KeyEvent {
                code: KeyCode::Left,
                ..
            }) => Some(adjust(-1)),
            PickerInput::Key(KeyEvent {
                code: KeyCode::Right,
                ..
            }) => Some(adjust(1)),
            PickerInput::Action(Action::Submit) => Some(EditorEvent::ClosePicker),
            _ => None,
        })
    }

    /// Create a row activated by Enter.
    pub fn enter(
        cells: Vec<Line<'static>>,
        event: impl Fn() -> EditorEvent + Send + 'static,
    ) -> Self {
        Self::new(cells, move |input| {
            matches!(input, PickerInput::Action(Action::Submit)).then(&event)
        })
    }

    pub fn with_value(self, value: PickerValue) -> Self {
        Self {
            value: Some(value),
            ..self
        }
    }

    pub fn described(self, description: &'static str) -> Self {
        Self {
            description: Some(description),
            ..self
        }
    }
}

impl PickerItem for Row {
    fn cells(&self) -> Vec<Line<'static>> {
        self.cells.clone()
    }

    fn value(&self) -> Option<&PickerValue> {
        self.value.as_ref()
    }

    fn description(&self) -> Option<&str> {
        self.description
    }

    fn handle(&mut self, input: PickerInput) -> Option<EditorEvent> {
        (self.handle)(input)
    }
}

impl From<Row> for Box<dyn PickerItem> {
    fn from(row: Row) -> Self {
        Box::new(row)
    }
}

/// Keyboard behavior attached to a picker presentation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PickerMode {
    FilteredCommands,
    Commands,
    Settings,
    Keymaps,
}

/// A named page of picker rows.
pub struct PickerTab {
    pub label: &'static str,
    pub items: Vec<Box<dyn PickerItem>>,
}

/// Presentation and selection state for pages of picker items.
///
/// A picker with several tabs names them along its top edge and switches them with
/// Tab; one with a single tab is titled by it.
pub struct Picker {
    tabs: Vec<PickerTab>,
    tab: usize,
    /// An item beneath every tab's rows, reached by moving past the last row.
    button: Option<Box<dyn PickerItem>>,
    /// The selected row of the current tab, or the button one past its rows.
    selected: usize,
    /// Widest cell seen in each column while open, so tab stops stay put as values change.
    column_widths: Vec<usize>,
}

impl Picker {
    pub fn new(title: &'static str, items: Vec<Box<dyn PickerItem>>) -> Self {
        Self::tabbed(
            vec![PickerTab {
                label: title,
                items,
            }],
            None,
        )
    }

    pub fn tabbed(tabs: Vec<PickerTab>, button: Option<Box<dyn PickerItem>>) -> Self {
        let mut picker = Self {
            tabs: Vec::new(),
            tab: 0,
            button,
            selected: 0,
            column_widths: Vec::new(),
        };
        picker.set_tabs(tabs);
        picker
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    fn items(&self) -> &[Box<dyn PickerItem>] {
        &self.tabs[self.tab].items
    }

    /// Selectable positions: the current tab's rows, then the button.
    fn positions(&self) -> usize {
        self.items().len() + usize::from(self.button.is_some())
    }

    /// Rows needed at `width`: as many as the longest tab has, so switching tabs
    /// never resizes the picker, an empty list keeping one; then the button and the
    /// longest description beneath them.
    pub fn height(&self, width: u16) -> u16 {
        let rows = self.tabs.iter().map(|tab| tab.items.len()).max();
        let rows = rows.unwrap_or_default().max(1) + self.footer_height(usize::from(width));
        rows as u16 + 2
    }

    fn footer_height(&self, width: usize) -> usize {
        let button = match self.button {
            Some(_) => 2,
            None => 0,
        };
        button + self.description_height(width)
    }

    fn description_height(&self, width: usize) -> usize {
        self.tabs
            .iter()
            .flat_map(|tab| &tab.items)
            .chain(&self.button)
            .filter_map(|item| item.description())
            .map(|description| Self::wrapped(description, width).len())
            .max()
            .unwrap_or_default()
    }

    /// `description` broken into the rows it takes beneath the gutter at `width`.
    fn wrapped(description: &str, width: usize) -> Vec<String> {
        let room = width
            .saturating_sub(MARKER.len() + UI.picker_right_margin)
            .max(1);
        WordWrap
            .wrap(description, room as u16)
            .into_iter()
            .map(|line| description[line.range].trim().to_string())
            .collect()
    }

    /// Select an item, or zero when the picker has no items.
    pub fn select(&mut self, index: usize) {
        self.selected = index.min(self.positions().saturating_sub(1));
    }

    pub fn move_selection(&mut self, delta: isize) {
        let positions = self.positions() as isize;
        if positions > 0 {
            self.select((self.selected as isize + delta).rem_euclid(positions) as usize);
        }
    }

    /// Show a neighbouring tab, keeping the selected row, or the button, where possible.
    fn switch_tab(&mut self, delta: isize) {
        let on_button = self.selected >= self.items().len();
        self.tab = (self.tab as isize + delta).rem_euclid(self.tabs.len() as isize) as usize;
        let rows = self.items().len();
        self.selected = match on_button {
            true => rows,
            false => self.selected.min(rows.saturating_sub(1)),
        };
    }

    /// Replace the current tab's items while retaining the selection where possible.
    pub fn set_items(&mut self, items: Vec<Box<dyn PickerItem>>) {
        self.tabs[self.tab].items = items;
        self.measure();
    }

    /// Replace every tab while retaining the tab and selection shown where possible.
    pub fn set_tabs(&mut self, tabs: Vec<PickerTab>) {
        assert!(!tabs.is_empty(), "a picker shows at least one tab");
        self.tab = self.tab.min(tabs.len() - 1);
        self.tabs = tabs;
        self.measure();
    }

    /// Widen the columns to every tab's cells, so a value lines up across tabs.
    fn measure(&mut self) {
        for cells in self
            .tabs
            .iter()
            .flat_map(|tab| &tab.items)
            .map(|item| item.cells())
        {
            if self.column_widths.len() < cells.len() {
                self.column_widths.resize(cells.len(), 0);
            }
            for (widest, width) in self
                .column_widths
                .iter_mut()
                .zip(cells.iter().map(Line::width))
            {
                *widest = (*widest).max(width);
            }
        }
        self.select(self.selected);
    }

    fn selected_item(&self) -> Option<&dyn PickerItem> {
        match self.items().get(self.selected) {
            Some(item) => Some(item.as_ref()),
            None => self.button.as_deref(),
        }
    }

    pub fn selected_item_mut(&mut self) -> Option<&mut (dyn PickerItem + '_)> {
        let tab = &mut self.tabs[self.tab];
        match tab.items.get_mut(self.selected) {
            Some(item) => Some(item.as_mut()),
            None => match &mut self.button {
                Some(button) => Some(button.as_mut()),
                None => None,
            },
        }
    }

    /// Whether there are several tabs to switch between.
    fn has_tabs(&self) -> bool {
        self.tabs.len() > 1
    }

    /// Handle picker navigation or forward input to the selected row.
    pub fn handle(&mut self, input: PickerInput) -> (bool, Option<EditorEvent>) {
        let tabbed = self.has_tabs();
        match input {
            PickerInput::Key(KeyEvent {
                code: KeyCode::Up, ..
            }) => {
                self.move_selection(-1);
                (true, None)
            }
            PickerInput::Key(KeyEvent {
                code: KeyCode::Down,
                ..
            }) => {
                self.move_selection(1);
                (true, None)
            }
            PickerInput::Key(KeyEvent {
                code: KeyCode::Tab, ..
            }) if tabbed => {
                self.switch_tab(1);
                (true, None)
            }
            PickerInput::Key(KeyEvent {
                code: KeyCode::BackTab,
                ..
            }) if tabbed => {
                self.switch_tab(-1);
                (true, None)
            }
            _ => {
                let event = self.selected_item_mut().and_then(|item| item.handle(input));
                (event.is_some(), event)
            }
        }
    }

    /// Column starts on shared tab stops, the last where a value follows the cells,
    /// or `None` when the cells, or the least room a value keeps after them, would
    /// overflow `width`.
    fn column_starts(&self, width: usize) -> Option<Vec<usize>> {
        let starts = self
            .column_widths
            .iter()
            .scan(0, |start, &column| {
                *start += UI.picker_tab_stop(column);
                Some(*start)
            })
            .collect::<Vec<_>>();
        let starts = [0].into_iter().chain(starts).collect::<Vec<_>>();
        let room = width.saturating_sub(MARKER.len() + UI.picker_right_margin);
        let cells_fit = (starts.len() < 2)
            || starts[starts.len() - 2] + self.column_widths.last().unwrap() <= room;
        let values = self
            .tabs
            .iter()
            .flat_map(|tab| &tab.items)
            .any(|item| item.value().is_some());
        let value_fits = !values || starts.last().unwrap() + UI.picker_min_value_width <= room;
        (cells_fit && value_fits).then_some(starts)
    }

    /// One row laid out at `starts`, or with packed cells when there is no room for them.
    fn line(
        item: &dyn PickerItem,
        selected: bool,
        starts: Option<&[usize]>,
        width: usize,
    ) -> Line<'static> {
        let groups = scheme();
        let (marker, emphasis) = match selected {
            true => (SELECTED_MARKER, groups.picker_selected_marker()),
            false => (MARKER, groups.picker_marker()),
        };
        let mut spans = vec![Span::styled(marker, emphasis)];
        let mut used = 0;
        let cells = item.cells();
        let columns = cells.len();
        let start = |index: usize, used: usize| match starts {
            Some(starts) => starts[index],
            None if index == 0 => 0,
            None => used + UI.picker_column_gap,
        };
        for (index, mut cell) in cells.into_iter().enumerate() {
            if index == 0 && selected {
                for span in &mut cell.spans {
                    span.style = span.style.patch(emphasis);
                }
            }
            let start = start(index, used);
            spans.push(Span::raw(" ".repeat(start.saturating_sub(used))));
            used = start.max(used) + cell.width();
            spans.extend(cell.spans);
        }
        if let Some(value) = item.value() {
            let start = start(columns, used).max(used);
            let room = width.saturating_sub(marker.len() + start + UI.picker_right_margin);
            let value = value.line(room, selected);
            spans.push(Span::raw(" ".repeat(start - used)));
            used = start + value.width();
            spans.extend(value.spans);
        }
        spans.push(Span::raw(
            " ".repeat(width.saturating_sub(marker.len() + used)),
        ));
        for span in &mut spans {
            span.style = groups.picker().patch(span.style);
            if selected {
                span.style = span.style.patch(groups.picker_selected());
            }
        }
        Line::from(spans)
    }

    /// The button, a rule across the picker, and the selected item's description
    /// beneath the rows, padded to the most any description takes.
    fn footer(&self, width: usize) -> Vec<Line<'static>> {
        let groups = scheme();
        let mut lines = Vec::new();
        if let Some(button) = &self.button {
            let selected = self.selected == self.items().len();
            lines.push(Self::line(button.as_ref(), selected, None, width));
            let rule = UI.glyphs.picker_rule.current().repeat(width);
            lines.push(Span::styled(rule, groups.picker_border()).into());
        }
        let mut description = self
            .selected_item()
            .and_then(|item| item.description())
            .map(|description| Self::wrapped(description, width))
            .unwrap_or_default();
        description.resize(self.description_height(width), String::new());
        lines.extend(description.into_iter().map(|row| {
            Line::from(vec![
                Span::raw(MARKER),
                Span::styled(row, groups.settings_item_description()),
            ])
        }));
        lines
    }

    fn text(&self, width: usize, height: usize) -> Vec<Line<'static>> {
        let footer = self.footer(width);
        let height = height.saturating_sub(footer.len());
        let starts = self.column_starts(width);
        let items = self.items();
        let start = self
            .selected
            .min(items.len().saturating_sub(1))
            .saturating_sub(height / 2)
            .min(items.len().saturating_sub(height));
        let mut lines = items
            .iter()
            .enumerate()
            .skip(start)
            .take(height)
            .map(|(index, item)| {
                Self::line(
                    item.as_ref(),
                    index == self.selected,
                    starts.as_deref(),
                    width,
                )
            })
            .collect::<Vec<_>>();
        lines.resize(height, Line::default());
        lines.extend(footer);
        lines
    }

    /// The single tab's label, or every tab named with the shown one lit.
    fn title(&self) -> Line<'static> {
        let groups = scheme();
        if let [tab] = self.tabs.as_slice() {
            return Span::styled(format!(" {} ", tab.label), groups.picker_title()).into();
        }
        Line::from(
            self.tabs
                .iter()
                .enumerate()
                .map(|(index, tab)| {
                    let style = match index == self.tab {
                        true => groups.picker_tab_selected(),
                        false => groups.picker_tab(),
                    };
                    Span::styled(format!(" {} ", tab.label), style)
                })
                .collect::<Vec<_>>(),
        )
    }
}

impl Element for Picker {
    type Context = ();

    fn render(&self, _: &(), area: Rect, buf: &mut Buffer) {
        let groups = scheme();
        let block = TuiBlock::default()
            .style(groups.picker())
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_type(BorderType::Rounded)
            .border_style(groups.picker_border())
            .title(self.title())
            .title_alignment(Alignment::Center);
        let inner = block.inner(area);
        block.render(area, buf);
        Paragraph::new(self.text(usize::from(inner.width), usize::from(inner.height)))
            .render(inner, buf);
    }
}

/// A picker paired with its component-owned unhandled-key policy.
pub struct PickerController {
    /// Active picker behavior.
    pub mode: PickerMode,
    /// Rendered and selected rows.
    pub picker: Picker,
}

impl PickerController {
    /// The keys this picker answers, each followed by what it does.
    pub fn hints(&self) -> Vec<TextSpan> {
        let keys = |codes: &[KeyCode]| {
            codes
                .iter()
                .map(|&code| KeyEvent::from(code).to_string())
                .collect::<String>()
        };
        let mut hints = vec![(keys(&[KeyCode::Up, KeyCode::Down]), "select")];
        hints.push(match self.mode {
            PickerMode::Settings => (keys(&[KeyCode::Left, KeyCode::Right]), "change"),
            PickerMode::Commands | PickerMode::FilteredCommands => (keys(&[KeyCode::Enter]), "run"),
            PickerMode::Keymaps => (keys(&[KeyCode::Enter]), "rebind"),
        });
        if self.picker.has_tabs() {
            hints.push((keys(&[KeyCode::Tab]), "switch tabs"));
        }
        hints.push((keys(&[KeyCode::Esc]), "close"));
        hints
            .into_iter()
            .enumerate()
            .flat_map(|(index, (keys, label))| {
                let separator = (index > 0)
                    .then(|| TextSpan::new(HINT_SEPARATOR, HLGroup::StatuslineSeparator));
                separator.into_iter().chain([
                    TextSpan::new(format!("{keys} "), HLGroup::KeymapHint),
                    TextSpan::new(label, HLGroup::StatuslineHintLabel),
                ])
            })
            .collect()
    }

    /// Handle navigation, row behavior, or mode-specific fallback behavior.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<EditorEvent> {
        self.handle(PickerInput::Key(key))
    }

    /// Handle a mapped action without retaining its originating key.
    pub fn handle_action(&mut self, action: Action) -> Option<EditorEvent> {
        self.handle(PickerInput::Action(action))
    }

    fn handle(&mut self, input: PickerInput) -> Option<EditorEvent> {
        if self
            .picker
            .selected_item()
            .is_some_and(|item| item.capturing())
        {
            return self
                .picker
                .selected_item_mut()
                .and_then(|item| item.handle(input));
        }
        if let PickerInput::Key(KeyEvent { code, .. }) = input {
            match (self.mode, code) {
                (_, KeyCode::Esc) => return Some(EditorEvent::ClosePickers),
                (PickerMode::FilteredCommands, _) => {}
                (_, KeyCode::Backspace) => return Some(EditorEvent::ClosePicker),
                _ => {}
            }
        }
        let (handled, event) = self.picker.handle(input);
        if handled {
            return event;
        }
        let PickerInput::Key(key) = input else {
            return None;
        };
        let printable = key.is_text();
        match (self.mode, key.code) {
            (PickerMode::FilteredCommands, KeyCode::Char(_)) if printable => {
                Some(EditorEvent::EditCommand(key))
            }
            (
                PickerMode::FilteredCommands,
                KeyCode::Backspace
                | KeyCode::Delete
                | KeyCode::Tab
                | KeyCode::BackTab
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Home
                | KeyCode::End,
            ) => Some(EditorEvent::EditCommand(key)),
            (_, KeyCode::Char(character)) if printable => {
                Some(EditorEvent::ResumeTyping(character))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::controller::settings::theme::groups::{Groups, HLGroup};
    use crossterm::event::KeyCode;

    #[test]
    fn keymap_capture_precedes_navigation_and_escape_cancels_only_capture() {
        use crate::ui::controller::settings::keymap::KeymapRow;

        let row = KeymapRow::new(Action::ClearFeed, KeyCode::F(2).into());
        let mut controller = PickerController {
            mode: PickerMode::Keymaps,
            picker: Picker::new("keymaps", vec![Box::new(row)]),
        };
        controller.handle_action(Action::Submit);
        assert!(controller.picker.selected_item().unwrap().capturing());
        assert!(matches!(controller.handle_key(KeyCode::Up.into()),
            Some(EditorEvent::Rebind(Action::ClearFeed, key)) if key.code == KeyCode::Up));
        assert!(controller.handle_key(KeyCode::Esc.into()).is_none());
        assert!(!controller.picker.selected_item().unwrap().capturing());
        assert!(matches!(
            controller.handle_key(KeyCode::Esc.into()),
            Some(EditorEvent::ClosePickers)
        ));
    }

    /// A mutable row used to exercise the picker trait boundary.
    struct TestItem {
        label: &'static str,
        value: &'static str,
    }

    impl PickerItem for TestItem {
        fn cells(&self) -> Vec<Line<'static>> {
            vec![
                Span::styled(self.label, scheme().muted()).into(),
                Span::styled(self.value, scheme().value()).into(),
            ]
        }

        fn handle(&mut self, input: PickerInput) -> Option<EditorEvent> {
            if !matches!(
                input,
                PickerInput::Key(KeyEvent {
                    code: KeyCode::Right,
                    ..
                })
            ) {
                return None;
            }
            self.value = "two";
            Some(EditorEvent::ClosePickers)
        }
    }

    fn item(label: &'static str, value: &'static str) -> Box<dyn PickerItem> {
        Box::new(TestItem { label, value })
    }

    fn picker(items: Vec<Box<dyn PickerItem>>) -> Picker {
        Picker::new("test", items)
    }

    fn rendered(picker: &Picker, width: u16) -> Buffer {
        rendered_in(picker, width, picker.height(width))
    }

    fn rendered_in(picker: &Picker, width: u16, height: u16) -> Buffer {
        let mut buffer = Buffer::empty(Rect::new(0, 0, width, height));
        picker.render(&(), *buffer.area(), &mut buffer);
        buffer
    }

    fn row(buffer: &Buffer, y: u16) -> String {
        (buffer.area().left()..buffer.area().right())
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    fn column(line: &str, text: &str) -> Option<usize> {
        line.find(text).map(|byte| line[..byte].chars().count())
    }

    #[test]
    fn selection_wraps_in_both_directions() {
        let mut picker = picker(vec![item("one", ""), item("two", "")]);

        picker.move_selection(-1);
        assert_eq!(picker.selected(), 1);
        picker.move_selection(1);
        assert_eq!(picker.selected(), 0);
    }

    #[test]
    fn constrained_picker_scrolls_to_its_selected_row() {
        let mut picker = picker(vec![
            item("one", ""),
            item("two", ""),
            item("three", ""),
            item("four", ""),
        ]);
        picker.select(3);

        let buffer = rendered_in(&picker, 32, 4);
        assert!(row(&buffer, 2).contains("> four"));
    }

    #[test]
    fn selected_item_can_change_itself_and_fire_an_event() {
        let mut picker = picker(vec![item("value", "one")]);

        let event = picker
            .selected_item_mut()
            .unwrap()
            .handle(PickerInput::Key(KeyEvent::from(KeyCode::Right)));

        assert!(row(&rendered(&picker, 32), 1).contains("two"));
        assert!(matches!(event, Some(EditorEvent::ClosePickers)));
    }

    #[test]
    fn values_share_the_next_tab_stop_after_the_longest_label() {
        let picker = picker(vec![item("scheme", "everforest"), item("contrast", "dark")]);
        let buffer = rendered(&picker, 32);

        assert_eq!(column(&row(&buffer, 1), "everforest"), Some(15));
        assert_eq!(column(&row(&buffer, 2), "dark"), Some(15));
    }

    #[test]
    fn every_column_aligns_across_rows_and_packs_when_narrow() {
        let cells = |texts: [&'static str; 3]| texts.map(Line::from).to_vec();
        let wide = picker(vec![
            Row::new(cells(["a", "one", "first"]), |_| None).into(),
            Row::new(cells(["longer", "1", "second"]), |_| None).into(),
        ]);
        let buffer = rendered(&wide, 40);
        assert_eq!(column(&row(&buffer, 1), "first"), Some(19));
        assert_eq!(column(&row(&buffer, 2), "second"), Some(19));

        let buffer = rendered(&wide, 20);
        assert_eq!(column(&row(&buffer, 1), "one"), Some(6));
        assert_eq!(column(&row(&buffer, 2), "1"), Some(11));
    }

    #[test]
    fn picker_uses_its_own_surface_and_selected_row_backgrounds() {
        let picker = picker(vec![item("one", "1"), item("two", "2")]);
        let buffer = rendered(&picker, 32);
        let groups = Groups::default();

        assert_eq!(column(&row(&buffer, 0), " test "), Some(13));
        assert_ne!(buffer[(0, 1)].symbol(), "│");
        assert_ne!(buffer[(31, 1)].symbol(), "│");
        assert_eq!(buffer[(0, 1)].symbol(), " ");
        assert_eq!(buffer[(1, 1)].symbol(), ">");
        assert_eq!(buffer[(16, 1)].bg, groups.picker_selected().bg.unwrap());
        let value = column(&row(&buffer, 1), "1").unwrap() as u16;
        assert_eq!(buffer[(value, 1)].fg, groups.value().fg.unwrap());
        assert_eq!(buffer[(16, 2)].bg, groups.picker().bg.unwrap());
        assert!(groups.picker().bg.is_some());
        assert!(groups.picker_selected().fg.is_none());
        assert_eq!(buffer.area().height, 2 + 2);
    }

    #[test]
    fn keymap_values_keep_the_statusline_accent_when_selected() {
        let picker = Picker::new(
            "keymaps",
            vec![Row::new(
                vec![
                    Line::from("submit"),
                    Span::styled("enter", scheme().style(HLGroup::KeymapHint)).into(),
                ],
                |_| None,
            )
            .into()],
        );
        let buffer = rendered(&picker, 32);
        let groups = Groups::default();
        let key = column(&row(&buffer, 1), "enter").unwrap() as u16;

        assert_eq!(buffer[(key, 1)].fg, groups.prompt().fg.unwrap());
        assert_eq!(buffer[(key, 1)].bg, groups.picker_selected().bg.unwrap());
    }

    fn setting(label: &'static str, value: PickerValue) -> Box<dyn PickerItem> {
        Row::new(vec![Line::from(label)], |_| None)
            .with_value(value)
            .described("what this setting does")
            .into()
    }

    fn mode(chosen: usize) -> PickerValue {
        PickerValue::Choice {
            options: vec!["dark".into(), "light".into()],
            chosen,
        }
    }

    fn tabbed(button: bool) -> Picker {
        let button = button.then(|| {
            Row::new(vec![Line::from("restore")], |_| None)
                .described("reset everything")
                .into()
        });
        Picker::tabbed(
            vec![
                PickerTab {
                    label: "first",
                    items: vec![setting("mode", mode(0)), setting("rows", mode(1))],
                },
                PickerTab {
                    label: "second",
                    items: vec![setting(
                        "a much longer label",
                        PickerValue::Stepped("3".into()),
                    )],
                },
            ],
            button,
        )
    }

    fn key(code: KeyCode) -> PickerInput {
        PickerInput::Key(KeyEvent::from(code))
    }

    #[test]
    fn tabs_are_named_along_the_top_and_switched_with_tab() {
        let mut picker = tabbed(false);
        let buffer = rendered(&picker, 60);
        assert!(row(&buffer, 0).contains(" first "));
        assert!(row(&buffer, 0).contains(" second "));
        assert!(row(&buffer, 1).contains("> mode"));

        picker.handle(key(KeyCode::Tab));
        assert!(row(&rendered(&picker, 60), 1).contains("> a much longer label"));
        picker.handle(key(KeyCode::BackTab));
        assert!(row(&rendered(&picker, 60), 1).contains("> mode"));
    }

    #[test]
    fn values_share_one_column_across_tabs() {
        let mut picker = tabbed(false);
        let first = column(&row(&rendered(&picker, 60), 1), "dark").unwrap();
        picker.handle(key(KeyCode::Tab));
        let second = column(&row(&rendered(&picker, 60), 1), "3").unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn the_button_follows_the_rows_and_keeps_the_picker_one_height() {
        let mut picker = tabbed(true);
        let height = picker.height(60);
        picker.move_selection(2);
        let buffer = rendered(&picker, 60);
        let button = (0..height)
            .find(|&y| row(&buffer, y).contains("> restore"))
            .expect("moving past the last row selects the button");
        let rule = UI.glyphs.picker_rule.current();
        assert_eq!(
            row(&buffer, button + 1),
            rule.repeat(60),
            "a rule spans the picker"
        );
        assert!(row(&buffer, button + 2).contains("reset everything"));

        picker.handle(key(KeyCode::Tab));
        assert_eq!(picker.height(60), height, "a shorter tab keeps the height");
        assert!(row(&rendered(&picker, 60), button).contains("> restore"));
    }

    #[test]
    fn a_description_wraps_beneath_its_selected_row() {
        let picker = tabbed(true);
        let wide = picker.height(60);
        let narrow = picker.height(16);
        assert!(narrow > wide, "the description takes more rows when narrow");

        let buffer = rendered(&picker, 16);
        let text = (0..narrow)
            .map(|y| row(&buffer, y))
            .collect::<Vec<_>>()
            .join(" ");
        for word in ["what", "this", "setting", "does"] {
            assert!(text.contains(word));
        }
    }

    #[test]
    fn a_choice_shows_only_the_chosen_option_between_arrows() {
        let back = UI.glyphs.step_back.current();
        let forward = UI.glyphs.step_forward.current();

        assert_eq!(
            mode(1).line(usize::MAX, false).to_string(),
            format!("{back} light {forward}")
        );
    }

    #[test]
    fn the_status_line_names_the_keys_a_picker_answers() {
        let hints = |picker| {
            PickerController {
                mode: PickerMode::Settings,
                picker,
            }
            .hints()
            .iter()
            .map(|span| span.content.clone())
            .collect::<String>()
        };
        let tab = KeyEvent::from(KeyCode::Tab).to_string();
        assert!(hints(tabbed(true)).contains(&format!("{tab} ")));
        assert!(!hints(picker(vec![item("one", "")])).contains(&tab));
    }

    #[test]
    fn a_value_too_wide_for_its_room_is_trimmed_between_its_arrows() {
        let value = PickerValue::Stepped("gruvbox material".into());
        let line = value.line(10, true);
        assert!(line.width() <= 10);
        assert!(line.to_string().contains(UI.glyphs.trimmed.current()));
    }

    #[test]
    fn an_unavailable_option_is_drawn_marked() {
        let value = PickerValue::Choice {
            options: vec![
                "unicode".into(),
                PickerOption {
                    label: "typeset".into(),
                    unavailable: Some("unavailable here".into()),
                },
            ],
            chosen: 1,
        };
        let line = value.line(usize::MAX, false);
        let typeset = line
            .spans
            .iter()
            .find(|span| span.content == "typeset")
            .unwrap();
        assert_eq!(typeset.style, scheme().settings_item_unavailable());
        assert!(line.to_string().ends_with("unavailable here"));
    }
}
