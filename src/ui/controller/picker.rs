//! Stateful text pickers rendered above the input.

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as TuiBlock, BorderType, Borders, Paragraph, Widget};

use super::settings::theme::groups::scheme;
use super::EditorEvent;
use crate::ui::action::Action;
use crate::ui::block::Element;
use crate::ui::config::UI;
use crate::ui::key::KeyEvent;

/// Gutter in front of the selected row.
const SELECTED_MARKER: &str = " > ";
/// Gutter in front of every other row, as wide as [`SELECTED_MARKER`].
const MARKER: &str = "   ";

/// Input delivered to the selected picker row after command lookup.
#[derive(Clone, Copy)]
pub enum PickerInput {
    /// A mapped application action whose physical key was discarded.
    Action(Action),
    /// Unmapped input interpreted privately by the picker.
    Key(KeyEvent),
}

/// A picker row: the cells it shows and the input it answers.
///
/// The picker aligns the n-th cell of every row into one column, so a row says only
/// what it shows, never where.
pub trait PickerItem: Send {
    /// Cell contents from left to right.
    fn cells(&self) -> Vec<Line<'static>>;

    fn handle(&mut self, input: PickerInput) -> Option<EditorEvent>;
}

/// A row of caller-styled cells with caller-supplied key behavior.
pub struct Row {
    cells: Vec<Line<'static>>,
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
}

impl PickerItem for Row {
    fn cells(&self) -> Vec<Line<'static>> {
        self.cells.clone()
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

/// Presentation and selection state for a list of picker items.
pub struct Picker {
    title: &'static str,
    items: Vec<Box<dyn PickerItem>>,
    selected: usize,
    /// Widest cell seen in each column while open, so tab stops stay put as values change.
    column_widths: Vec<usize>,
}

impl Picker {
    pub fn new(title: &'static str, items: Vec<Box<dyn PickerItem>>) -> Self {
        let mut picker = Self {
            title,
            items: Vec::new(),
            selected: 0,
            column_widths: Vec::new(),
        };
        picker.set_items(items);
        picker
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn height(&self) -> u16 {
        // A row per item, so a filtered list leaves no empty ones; an empty list keeps one.
        (self.items.len() as u16).max(1) + 2
    }

    /// Select an item, or zero when the picker has no items.
    pub fn select(&mut self, index: usize) {
        self.selected = index.min(self.items.len().saturating_sub(1));
    }

    pub fn move_selection(&mut self, delta: isize) {
        if !self.items.is_empty() {
            let selected =
                (self.selected as isize + delta).rem_euclid(self.items.len() as isize) as usize;
            self.select(selected);
        }
    }

    /// Replace the items while retaining the selection where possible.
    pub fn set_items(&mut self, items: Vec<Box<dyn PickerItem>>) {
        for cells in items.iter().map(|item| item.cells()) {
            if self.column_widths.len() < cells.len() {
                self.column_widths.resize(cells.len(), 0);
            }
            for (widest, cell) in self.column_widths.iter_mut().zip(&cells) {
                *widest = (*widest).max(cell.width());
            }
        }
        self.items = items;
        self.select(self.selected);
    }

    pub fn selected_item_mut(&mut self) -> Option<&mut (dyn PickerItem + '_)> {
        match self.items.get_mut(self.selected) {
            Some(item) => Some(item.as_mut()),
            None => None,
        }
    }

    /// Handle picker navigation or forward input to the selected row.
    pub fn handle(&mut self, input: PickerInput) -> (bool, Option<EditorEvent>) {
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
            _ => {
                let event = self.selected_item_mut().and_then(|item| item.handle(input));
                (event.is_some(), event)
            }
        }
    }

    /// Column starts on shared tab stops, or `None` when they would overflow `width`.
    fn column_starts(&self, width: usize) -> Option<Vec<usize>> {
        let starts = self
            .column_widths
            .iter()
            .scan(0, |start, &column| {
                let current = *start;
                *start += UI.picker_tab_stop(column);
                Some(current)
            })
            .collect::<Vec<_>>();
        let end = starts.last().zip(self.column_widths.last());
        end.is_none_or(|(start, column)| {
            MARKER.len() + start + column + UI.picker_right_margin <= width
        })
        .then_some(starts)
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
            true => (SELECTED_MARKER, groups.prompt()),
            false => (MARKER, groups.muted()),
        };
        let mut spans = vec![Span::styled(marker, emphasis)];
        let mut used = 0;
        for (index, mut cell) in item.cells().into_iter().enumerate() {
            if index == 0 && selected {
                for span in &mut cell.spans {
                    span.style = span.style.patch(emphasis);
                }
            }
            let start = match starts {
                Some(starts) => starts[index],
                None if index == 0 => 0,
                None => used + UI.picker_column_gap,
            };
            spans.push(Span::raw(" ".repeat(start.saturating_sub(used))));
            used = start.max(used) + cell.width();
            spans.extend(cell.spans);
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

    fn text(&self, width: usize, height: usize) -> Vec<Line<'static>> {
        let starts = self.column_starts(width);
        let start = self
            .selected
            .saturating_sub(height / 2)
            .min(self.items.len().saturating_sub(height));
        self.items
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
            .collect()
    }
}

impl Element for Picker {
    type Context = ();

    fn render(&self, _: &(), area: Rect, buf: &mut Buffer) {
        let groups = scheme();
        let title = Line::from(Span::styled(format!(" {} ", self.title), groups.prompt()));
        let block = TuiBlock::default()
            .style(groups.picker())
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_type(BorderType::Rounded)
            .border_style(groups.border())
            .title(title)
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
    /// Handle navigation, row behavior, or mode-specific fallback behavior.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<EditorEvent> {
        self.handle(PickerInput::Key(key))
    }

    /// Handle a mapped action without retaining its originating key.
    pub fn handle_action(&mut self, action: Action) -> Option<EditorEvent> {
        self.handle(PickerInput::Action(action))
    }

    fn handle(&mut self, input: PickerInput) -> Option<EditorEvent> {
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
        rendered_in(picker, width, picker.height())
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
    fn picker_uses_printed_and_previous_picker_row_backgrounds() {
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
        assert_eq!(groups.picker().bg, groups.printed().bg);
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
}
