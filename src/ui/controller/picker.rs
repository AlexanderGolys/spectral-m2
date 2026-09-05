//! Stateful text pickers rendered above the input.

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line as TuiLine, Span};
use ratatui::widgets::{Block as TuiBlock, BorderType, Borders, Paragraph, Widget};

use super::settings::theme::groups::{Groups, HLGroup};
use super::EditorEvent;
use crate::ui::action::Action;
use crate::ui::block::{Element, Text};
use crate::ui::config::UI;
use crate::ui::key::KeyEvent;

/// A single unwrapped text component.
pub(crate) trait Line: Text<Context = LineContext> {
    fn column_widths(&self) -> [usize; 3] {
        [0; 3]
    }
}

/// Local layout and selection state supplied while a picker line renders.
pub(crate) struct LineContext {
    pub(crate) width: usize,
    pub(crate) value_column: usize,
    pub(crate) columns: Option<[usize; 2]>,
    pub(crate) selected: bool,
    pub(crate) groups: Groups,
}

impl LineContext {
    /// Finish spans with the selected or ordinary full-row background.
    pub(crate) fn finish(&self, mut spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
        let used = spans.iter().map(Span::width).sum::<usize>();
        spans.push(Span::raw(" ".repeat(self.width.saturating_sub(used))));
        let background = self.groups.picker();
        for span in &mut spans {
            span.style = background.patch(span.style);
            if self.selected {
                span.style = span.style.patch(self.groups.picker_selected());
            }
        }
        spans
    }

    /// Generate the shared label/value presentation for one picker row.
    pub(crate) fn value(
        &self,
        label: &str,
        value: &str,
        trailing: bool,
        value_group: HLGroup,
    ) -> Vec<Span<'static>> {
        let marker = if self.selected { " > " } else { "   " };
        let label = format!("{marker}{label}");
        let gap = if trailing {
            self.width
                .saturating_sub(label.chars().count() + value.chars().count())
        } else {
            self.value_column
                .saturating_sub(label.chars().count() - marker.len())
        };
        self.finish(vec![
            Span::styled(
                label,
                if self.selected {
                    self.groups.prompt()
                } else {
                    self.groups.muted()
                },
            ),
            Span::raw(" ".repeat(gap)),
            Span::styled(value.to_string(), self.groups.style(value_group)),
        ])
    }

    /// Render three left-aligned fields at shared tab stops when they fit.
    pub(crate) fn columns(&self, columns: [Vec<Span<'static>>; 3]) -> Vec<Span<'static>> {
        let marker = if self.selected { " > " } else { "   " };
        let widths = columns
            .each_ref()
            .map(|column| column.iter().map(Span::width).sum::<usize>());
        let starts = self
            .columns
            .unwrap_or([widths[0] + 2, widths[0] + widths[1] + 4]);
        let mut used = 0;
        let mut spans = vec![Span::styled(
            marker,
            if self.selected {
                self.groups.prompt()
            } else {
                self.groups.muted()
            },
        )];
        for (index, column) in columns.into_iter().enumerate() {
            if index > 0 {
                let gap = starts[index - 1].saturating_sub(used).max(1);
                spans.push(Span::raw(" ".repeat(gap)));
                used += gap;
            }
            spans.extend(column);
            used += widths[index];
        }
        self.finish(spans)
    }
}

/// Input delivered to the selected picker row after command lookup.
#[derive(Clone, Copy)]
pub(crate) enum PickerInput {
    /// A mapped application action whose physical key was discarded.
    Action(Action),
    /// Unmapped input interpreted privately by the picker.
    Key(KeyEvent),
}

/// A line that owns its behavior and may request an editor state change.
pub(crate) trait PickerItem: Line {
    fn handle(&mut self, input: PickerInput) -> Option<EditorEvent>;
}

/// Label/value picker row with component-owned key behavior.
pub(crate) struct ValueItem {
    label: String,
    value: String,
    trailing: bool,
    value_group: HLGroup,
    handle: Box<dyn FnMut(PickerInput) -> Option<EditorEvent> + Send>,
}

impl ValueItem {
    /// Create a boxed row with arbitrary key handling.
    pub(crate) fn custom(
        label: impl Into<String>,
        value: impl Into<String>,
        trailing: bool,
        handle: impl FnMut(PickerInput) -> Option<EditorEvent> + Send + 'static,
    ) -> Box<dyn PickerItem> {
        Self::styled(label, value, trailing, HLGroup::Value, handle)
    }

    /// Create a boxed row whose value is a keymap hint.
    pub(crate) fn keymap(
        label: impl Into<String>,
        value: impl Into<String>,
        handle: impl FnMut(PickerInput) -> Option<EditorEvent> + Send + 'static,
    ) -> Box<dyn PickerItem> {
        Self::styled(label, value, false, HLGroup::KeymapHint, handle)
    }

    fn styled(
        label: impl Into<String>,
        value: impl Into<String>,
        trailing: bool,
        value_group: HLGroup,
        handle: impl FnMut(PickerInput) -> Option<EditorEvent> + Send + 'static,
    ) -> Box<dyn PickerItem> {
        Box::new(Self {
            label: label.into(),
            value: value.into(),
            trailing,
            value_group,
            handle: Box::new(handle),
        })
    }

    /// Create a row adjusted by horizontal arrow keys.
    pub(crate) fn adjust(
        label: impl Into<String>,
        value: impl Into<String>,
        mut adjust: impl FnMut(i8) -> EditorEvent + Send + 'static,
    ) -> Box<dyn PickerItem> {
        Self::custom(label, value, false, move |input| match input {
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
    pub(crate) fn enter(
        label: impl Into<String>,
        value: impl Into<String>,
        trailing: bool,
        event: impl Fn() -> EditorEvent + Send + 'static,
    ) -> Box<dyn PickerItem> {
        Self::custom(label, value, trailing, move |input| {
            matches!(input, PickerInput::Action(Action::Submit)).then(&event)
        })
    }
}

impl Text for ValueItem {
    type Context = LineContext;

    fn spans(&self, context: &LineContext) -> Vec<Span<'static>> {
        context.value(&self.label, &self.value, self.trailing, self.value_group)
    }
}

impl Line for ValueItem {
    fn column_widths(&self) -> [usize; 3] {
        [
            if self.trailing {
                0
            } else {
                self.label.chars().count()
            },
            0,
            0,
        ]
    }
}

impl PickerItem for ValueItem {
    fn handle(&mut self, input: PickerInput) -> Option<EditorEvent> {
        (self.handle)(input)
    }
}

/// Heterogeneous picker rows in display order.
pub(crate) type PickerItems = Vec<Box<dyn PickerItem>>;

/// Keyboard behavior attached to a picker presentation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerMode {
    /// Slash-prefixed live command filtering.
    FilteredCommands,
    /// Complete command list.
    Commands,
    /// Shell settings.
    Settings,
    /// Color appearance.
    Style,
    /// Remappable keys.
    Keymaps,
}

/// A picker paired with its component-owned unhandled-key policy.
pub(crate) struct PickerController {
    /// Active picker behavior.
    pub(crate) mode: PickerMode,
    /// Rendered and selected rows.
    pub(crate) picker: Picker,
}

impl PickerController {
    /// Handle navigation, row behavior, or mode-specific fallback behavior.
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Option<EditorEvent> {
        self.handle(PickerInput::Key(key))
    }

    /// Handle a mapped action without retaining its originating key.
    pub(crate) fn handle_action(&mut self, action: Action) -> Option<EditorEvent> {
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
            (PickerMode::FilteredCommands, KeyCode::Backspace | KeyCode::Delete) => {
                Some(EditorEvent::EditCommand(key))
            }
            (_, KeyCode::Char(character)) if printable => {
                Some(EditorEvent::ResumeTyping(character))
            }
            _ => None,
        }
    }
}

/// Presentation and selection state for a list of picker items.
pub(crate) struct Picker {
    title: &'static str,
    items: PickerItems,
    selected: usize,
}

impl Picker {
    pub(crate) fn new(title: &'static str, items: PickerItems) -> Self {
        let mut picker = Self {
            title,
            items,
            selected: 0,
        };
        picker.select(0);
        picker
    }

    #[cfg(test)]
    pub(crate) const fn title(&self) -> &'static str {
        self.title
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    pub(crate) fn height(&self) -> u16 {
        (self.items.len() as u16).max(UI.picker_min_rows) + 2
    }

    /// Select an item, or zero when the picker has no items.
    pub(crate) fn select(&mut self, index: usize) {
        self.selected = index.min(self.items.len().saturating_sub(1));
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        if !self.items.is_empty() {
            let selected =
                (self.selected as isize + delta).rem_euclid(self.items.len() as isize) as usize;
            self.select(selected);
        }
    }

    /// Replace the items while retaining the selection where possible.
    pub(crate) fn set_items(&mut self, items: PickerItems) {
        self.items = items;
        self.select(self.selected);
    }

    pub(crate) fn selected_item_mut(&mut self) -> Option<&mut (dyn PickerItem + '_)> {
        match self.items.get_mut(self.selected) {
            Some(item) => Some(item.as_mut()),
            None => None,
        }
    }

    /// Handle picker navigation or forward input to the selected row.
    pub(crate) fn handle(&mut self, input: PickerInput) -> (bool, Option<EditorEvent>) {
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

    fn text(&self, width: usize, height: usize, groups: &Groups) -> Vec<TuiLine<'static>> {
        let widths =
            self.items
                .iter()
                .map(|item| item.column_widths())
                .fold([0; 3], |mut widths, item| {
                    for index in 0..3 {
                        widths[index] = widths[index].max(item[index]);
                    }
                    widths
                });
        let label_width = widths[0];
        let value_column =
            label_width / UI.picker_tab_width * UI.picker_tab_width + UI.picker_tab_width;
        let hint_column = value_column
            + widths[1] / UI.picker_tab_width * UI.picker_tab_width
            + UI.picker_tab_width;
        let columns = (3 + hint_column + widths[2] + UI.picker_right_margin <= width)
            .then_some([value_column, hint_column]);
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
                TuiLine::from(item.spans(&LineContext {
                    width,
                    value_column,
                    columns,
                    selected: index == self.selected,
                    groups: *groups,
                }))
            })
            .collect()
    }
}

impl Element for Picker {
    type Context = Groups;

    fn render(&self, groups: &Groups, area: Rect, buf: &mut Buffer) {
        let title = TuiLine::from(Span::styled(format!(" {} ", self.title), groups.prompt()));
        let block = TuiBlock::default()
            .style(groups.picker())
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_type(BorderType::Rounded)
            .border_style(groups.border())
            .title(title)
            .title_alignment(Alignment::Center);
        let inner = block.inner(area);
        block.render(area, buf);
        Paragraph::new(self.text(usize::from(inner.width), usize::from(inner.height), groups))
            .render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    /// A mutable line used to exercise the picker trait boundary.
    struct TestItem {
        label: &'static str,
        value: &'static str,
        trailing: bool,
    }

    impl Text for TestItem {
        type Context = LineContext;

        fn spans(&self, context: &LineContext) -> Vec<Span<'static>> {
            context.value(self.label, self.value, self.trailing, HLGroup::Value)
        }
    }

    impl Line for TestItem {
        fn column_widths(&self) -> [usize; 3] {
            [
                if self.trailing {
                    0
                } else {
                    self.label.chars().count()
                },
                0,
                0,
            ]
        }
    }

    impl PickerItem for TestItem {
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
        Box::new(TestItem {
            label,
            value,
            trailing: false,
        })
    }

    fn picker(items: PickerItems) -> Picker {
        Picker::new("test", items)
    }

    fn rendered(picker: &Picker, width: u16) -> Buffer {
        let groups = Groups::default();
        rendered_in(picker, width, picker.height(), &groups)
    }

    fn rendered_in(picker: &Picker, width: u16, height: u16, groups: &Groups) -> Buffer {
        let mut buffer = Buffer::empty(Rect::new(0, 0, width, height));
        picker.render(groups, *buffer.area(), &mut buffer);
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

        let buffer = rendered_in(&picker, 32, 4, &Groups::default());
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
        let picker = picker(vec![
            item("scheme", "everforest"),
            item("brightness", "dark"),
        ]);
        let buffer = rendered(&picker, 32);

        assert_eq!(column(&row(&buffer, 1), "everforest"), Some(15));
        assert_eq!(column(&row(&buffer, 2), "dark"), Some(15));
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
        assert_eq!(buffer.area().height, UI.picker_min_rows + 2);
    }

    #[test]
    fn keymap_values_keep_the_statusline_accent_when_selected() {
        let picker = Picker::new(
            "keymaps",
            vec![ValueItem::keymap("submit", "enter", |_| None)],
        );
        let buffer = rendered(&picker, 32);
        let groups = Groups::default();
        let key = column(&row(&buffer, 1), "enter").unwrap() as u16;

        assert_eq!(buffer[(key, 1)].fg, groups.prompt().fg.unwrap());
        assert_eq!(buffer[(key, 1)].bg, groups.picker_selected().bg.unwrap());
    }
}
