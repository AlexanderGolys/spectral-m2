//! Stateful text pickers rendered above the input.

use crossterm::event::KeyEvent;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line as TuiLine, Span};
use ratatui::widgets::{Block as TuiBlock, BorderType, Borders, Paragraph, Widget};

use super::settings::Groups;
use super::EditorEvent;
use crate::ui::statusline::StatusLineEvent;

const TAB_WIDTH: usize = 4;

/// Text that generates styled spans within local rendering coordinates.
pub trait TextComponent {
    fn spans(&self, context: &LineContext<'_>) -> Vec<Span<'static>>;
}

/// A single unwrapped text component.
pub trait Line: TextComponent {
    fn alignment_width(&self) -> Option<usize> {
        None
    }
}

/// Local layout and selection state supplied while a picker line renders.
pub struct LineContext<'a> {
    pub width: usize,
    pub value_column: usize,
    pub selected: bool,
    pub groups: &'a Groups,
}

/// A line that owns its behavior and may request an editor state change.
pub trait PickerItem: Line {
    fn handle_key(&mut self, key: KeyEvent) -> Option<EditorEvent>;
}

/// Heterogeneous picker rows in display order.
pub type PickerItems = Vec<Box<dyn PickerItem>>;

/// Finish an item's spans with its selected or ordinary full-row background.
pub fn finish_spans(
    mut spans: Vec<Span<'static>>,
    context: &LineContext<'_>,
) -> Vec<Span<'static>> {
    let used = spans.iter().map(Span::width).sum::<usize>();
    spans.push(Span::raw(" ".repeat(context.width.saturating_sub(used))));
    let background = if context.selected {
        context.groups.picker_selected()
    } else {
        context.groups.picker()
    };
    for span in &mut spans {
        span.style = background.patch(span.style);
    }
    spans
}

/// Generate the shared label/value presentation used by ordinary setting rows.
pub fn value_spans(
    label: &str,
    value: &str,
    trailing: bool,
    context: &LineContext<'_>,
) -> Vec<Span<'static>> {
    let marker = if context.selected { " > " } else { "   " };
    let label = format!("{marker}{label}");
    let gap = if trailing {
        context
            .width
            .saturating_sub(label.chars().count() + value.chars().count())
    } else {
        context
            .value_column
            .saturating_sub(label.chars().count() - marker.len())
    };
    finish_spans(
        vec![
            Span::styled(
                label,
                if context.selected {
                    context.groups.prompt()
                } else {
                    context.groups.muted()
                },
            ),
            Span::raw(" ".repeat(gap)),
            Span::styled(value.to_string(), context.groups.value()),
        ],
        context,
    )
}

/// Status text associated with a picker.
pub struct PickerStatus {
    label: &'static str,
    message: &'static str,
}

impl PickerStatus {
    pub const fn new(label: &'static str, message: &'static str) -> Self {
        Self { label, message }
    }

    fn event(&self, groups: &Groups) -> StatusLineEvent {
        fn keymap_spans(text: &'static str, groups: &Groups) -> Vec<ratatui::text::Span<'static>> {
            text.split_inclusive(' ')
                .map(|part| {
                    let style =
                        if matches!(part.trim(), "/" | "↑↓" | "←→" | "enter" | "esc" | "tab")
                        {
                            groups.prompt()
                        } else {
                            groups.muted()
                        };
                    ratatui::text::Span::styled(part, style)
                })
                .collect()
        }

        let mut spans = keymap_spans(self.label, groups);
        spans.extend(keymap_spans(self.message, groups));
        StatusLineEvent::Show {
            alignment: Alignment::Left,
            spans,
        }
    }
}

/// Presentation and selection state for a list of picker items.
pub struct Picker {
    title: &'static str,
    items: PickerItems,
    selected: usize,
    status: PickerStatus,
}

impl Picker {
    pub fn new(title: &'static str, items: PickerItems, status: PickerStatus) -> Self {
        let mut picker = Self {
            title,
            items,
            selected: 0,
            status,
        };
        picker.select(0);
        picker
    }

    #[cfg(test)]
    pub const fn title(&self) -> &'static str {
        self.title
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn selected(&self) -> usize {
        self.selected
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
    pub fn set_items(&mut self, items: PickerItems) {
        self.items = items;
        self.select(self.selected);
    }

    pub fn selected_item_mut(&mut self) -> Option<&mut (dyn PickerItem + '_)> {
        match self.items.get_mut(self.selected) {
            Some(item) => Some(item.as_mut()),
            None => None,
        }
    }

    pub fn status_event(&self, groups: &Groups) -> StatusLineEvent {
        self.status.event(groups)
    }

    pub fn height(&self) -> u16 {
        self.items.len() as u16 + 2
    }

    fn text(&self, width: usize, groups: &Groups) -> Vec<TuiLine<'static>> {
        let label_width = self
            .items
            .iter()
            .filter_map(|item| item.alignment_width())
            .max()
            .unwrap_or_default();
        let value_column = label_width / TAB_WIDTH * TAB_WIDTH + TAB_WIDTH;
        self.items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                TuiLine::from(item.spans(&LineContext {
                    width,
                    value_column,
                    selected: index == self.selected,
                    groups,
                }))
            })
            .collect()
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, groups: &Groups) {
        let title = TuiLine::from(Span::styled(format!(" {} ", self.title), groups.prompt()));
        let block = TuiBlock::default()
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_type(BorderType::Rounded)
            .border_style(groups.border())
            .style(groups.picker())
            .title(title)
            .title_alignment(Alignment::Center);
        let inner = block.inner(area);
        block.render(area, buf);
        Paragraph::new(self.text(usize::from(inner.width), groups)).render(inner, buf);
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

    impl TextComponent for TestItem {
        fn spans(&self, context: &LineContext<'_>) -> Vec<Span<'static>> {
            value_spans(self.label, self.value, self.trailing, context)
        }
    }

    impl Line for TestItem {
        fn alignment_width(&self) -> Option<usize> {
            (!self.trailing).then(|| self.label.chars().count())
        }
    }

    impl PickerItem for TestItem {
        fn handle_key(&mut self, key: KeyEvent) -> Option<EditorEvent> {
            if key.code != KeyCode::Right {
                return None;
            }
            self.value = "two";
            Some(EditorEvent::FinishCommand)
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
        Picker::new("test", items, PickerStatus::new("test ", "help"))
    }

    fn rendered(picker: &Picker, width: u16) -> Buffer {
        let mut buffer = Buffer::empty(Rect::new(0, 0, width, picker.height()));
        picker.render(*buffer.area(), &mut buffer, &Groups::default());
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
    fn selected_item_can_change_itself_and_fire_an_event() {
        let mut picker = picker(vec![item("value", "one")]);

        let event = picker
            .selected_item_mut()
            .unwrap()
            .handle_key(KeyEvent::from(KeyCode::Right));

        assert!(row(&rendered(&picker, 32), 1).contains("two"));
        assert!(matches!(event, Some(EditorEvent::FinishCommand)));
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
    fn picker_has_horizontal_borders_and_distinct_row_backgrounds() {
        let picker = picker(vec![item("one", "1"), item("two", "2")]);
        let buffer = rendered(&picker, 32);
        let groups = Groups::default();

        assert_eq!(column(&row(&buffer, 0), " test "), Some(13));
        assert_ne!(buffer[(0, 1)].symbol(), "│");
        assert_ne!(buffer[(31, 1)].symbol(), "│");
        assert_eq!(buffer[(0, 1)].symbol(), " ");
        assert_eq!(buffer[(1, 1)].symbol(), ">");
        assert_eq!(buffer[(16, 1)].bg, groups.picker_selected().bg.unwrap());
        assert_eq!(buffer[(16, 2)].bg, groups.picker().bg.unwrap());
        assert_ne!(groups.picker().bg, groups.code().bg);
    }
}
