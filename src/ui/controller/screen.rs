//! Mouse selection over the drawn screen: what each pane drew, which of its
//! cells may be copied, and the text a drag selects. A selection belongs to the
//! pane it starts in and never leaves it.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::Frame;

use super::settings::theme::groups::scheme;
use super::{Editor, Reaction};

/// A selectable cell position: a screen column and a content row, so a
/// selection stays on its text while the pane scrolls.
#[derive(Clone, Copy)]
pub struct ScreenPosition {
    column: u16,
    row: i64,
}

/// A mouse selection spanning captured cells of one pane.
pub struct ScreenSelection {
    /// The area of the pane the selection started in.
    area: Rect,
    anchor: ScreenPosition,
    focus: ScreenPosition,
    dragged: bool,
    text: Option<String>,
}

/// Captured terminal content and whether it may be copied.
pub struct ScreenCell {
    symbol: String,
    /// Whether the cell holds text a selection copies.
    selectable: bool,
    /// Whether the cell lies outside every selection exclusion, such as a frame,
    /// so a row of such cells with no text still copies as an empty line.
    copyable: bool,
}

/// What one pane drew in its last frame, kept to select and copy from.
pub struct ScreenCapture {
    area: Rect,
    /// The content row shown on the area's first line.
    top: i64,
    rows: Vec<Vec<ScreenCell>>,
}

impl ScreenCapture {
    /// The position under `event`, held inside this pane's area.
    fn position(&self, event: MouseEvent) -> ScreenPosition {
        let area = self.area;
        let column = event.column.clamp(area.x, area.right().saturating_sub(1));
        let line = event.row.clamp(area.y, area.bottom().saturating_sub(1));
        ScreenPosition {
            column,
            row: self.top + i64::from(line - area.y),
        }
    }

    /// The screen line showing `content_row`, if it is on screen.
    fn line(&self, content_row: i64) -> Option<u16> {
        let offset = u16::try_from(content_row - self.top).ok()?;
        (offset < self.area.height).then(|| self.area.y + offset)
    }

    fn cell(&self, x: u16, y: u16) -> Option<&ScreenCell> {
        self.rows
            .get(usize::from(y.checked_sub(self.area.y)?))?
            .get(usize::from(x.checked_sub(self.area.x)?))
    }

    fn selectable(&self, x: u16, y: u16) -> Option<&ScreenCell> {
        self.cell(x, y).filter(|cell| cell.selectable)
    }

    /// The columns a selection from `start` to `end` covers on `content_row`.
    fn columns(
        &self,
        content_row: i64,
        start: ScreenPosition,
        end: ScreenPosition,
    ) -> impl Iterator<Item = u16> {
        let from = match content_row == start.row {
            true => start.column,
            false => self.area.x,
        };
        let through = match content_row == end.row {
            true => end.column,
            false => self.area.right().saturating_sub(1),
        };
        from..=through
    }
}

impl Editor {
    pub fn handle_screen_mouse(&mut self, event: MouseEvent) -> Option<Reaction> {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) if !self.input.contains(event) => {
                self.input.field_mut().clear_selection();
                let pointer = Position::new(event.column, event.row);
                let capture = self
                    .captures
                    .iter()
                    .find(|capture| capture.area.contains(pointer))?;
                let position = capture.position(event);
                self.screen_selection = Some(ScreenSelection {
                    area: capture.area,
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
                if let Some(selection) = self.screen_selection.as_mut() {
                    if let Some(capture) = self
                        .captures
                        .iter()
                        .find(|capture| capture.area == selection.area)
                    {
                        selection.focus = capture.position(event);
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

    /// The capture of the pane `selection` belongs to.
    fn capture_of(&self, selection: &ScreenSelection) -> Option<&ScreenCapture> {
        self.captures
            .iter()
            .find(|capture| capture.area == selection.area)
    }

    pub fn selected_screen_text(&self) -> Option<String> {
        let selection = self.screen_selection.as_ref()?;
        if let Some(text) = selection.text.as_ref() {
            return Some(text.clone());
        }
        if !selection.dragged {
            return None;
        }
        let capture = self.capture_of(selection)?;
        let (start, end) = ordered_screen_positions(selection.anchor, selection.focus);
        let lines: Vec<String> = (start.row..=end.row)
            .filter_map(|content_row| {
                let y = capture.line(content_row)?;
                let cells: Vec<&ScreenCell> = capture
                    .columns(content_row, start, end)
                    .filter_map(|x| capture.cell(x, y))
                    .filter(|cell| cell.copyable)
                    .collect();
                (!cells.is_empty()).then(|| {
                    let line: String = cells
                        .iter()
                        .filter(|cell| cell.selectable)
                        .map(|cell| cell.symbol.as_str())
                        .collect();
                    line.trim_end().to_string()
                })
            })
            .collect();
        let text = lines.join("\n");
        let text = text.trim_matches('\n');
        (!text.is_empty()).then(|| text.to_string())
    }

    /// Keep what the pane just drawn into the feed area shows, replacing whatever
    /// was captured where it now lies.
    pub fn capture_screen(&mut self, frame: &mut Frame) {
        let area = self.feed_area;
        let buffer = frame.buffer_mut();
        let rows = (area.y..area.bottom())
            .map(|y| {
                let symbols: Vec<String> = (area.x..area.right())
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect();
                let allowed: Vec<bool> = (area.x..area.right())
                    .map(|x| {
                        let position = Position::new(x, y);
                        !self.selection_exclusions.contains(position)
                    })
                    .collect();
                let selectable = selectable_columns(&symbols, &allowed);
                symbols
                    .into_iter()
                    .zip(selectable)
                    .zip(allowed)
                    .map(|((symbol, selectable), copyable)| ScreenCell {
                        symbol,
                        selectable,
                        copyable,
                    })
                    .collect()
            })
            .collect();
        self.captures
            .retain(|capture| !capture.area.intersects(area));
        self.captures.push(ScreenCapture {
            area,
            top: self.feed_top,
            rows,
        });
    }

    /// Paint the selection over the pane just drawn, if the selection is its own.
    pub fn paint_screen_selection(&self, frame: &mut Frame) {
        let Some(selection) = self
            .screen_selection
            .as_ref()
            .filter(|selection| selection.dragged && selection.area == self.feed_area)
        else {
            return;
        };
        let Some(capture) = self.capture_of(selection) else {
            return;
        };
        let (start, end) = ordered_screen_positions(selection.anchor, selection.focus);
        let buffer = frame.buffer_mut();
        for content_row in start.row..=end.row {
            let Some(y) = capture.line(content_row) else {
                continue;
            };
            for x in capture.columns(content_row, start, end) {
                if capture.selectable(x, y).is_some() {
                    buffer[(x, y)].set_style(scheme().selection());
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
        while segment_start < symbols.len() && !allowed[segment_start] {
            segment_start += 1;
        }
        let mut segment_end = segment_start;
        while segment_end < symbols.len() && allowed[segment_end] {
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
    if segment_start >= segment_end {
        return;
    }
    let Some(last_content) = symbols[segment_start..segment_end]
        .iter()
        .rposition(|symbol| !symbol.trim().is_empty())
        .map(|index| segment_start + index)
    else {
        return;
    };
    selectable[segment_start..=last_content].fill(true);
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyModifiers;

    use super::*;

    /// A pane at `area` that drew `text` on every line, every cell selectable.
    fn pane(area: Rect, text: &str) -> ScreenCapture {
        let line = || {
            text.chars()
                .cycle()
                .take(usize::from(area.width))
                .map(|character| ScreenCell {
                    symbol: character.to_string(),
                    selectable: true,
                    copyable: true,
                })
                .collect()
        };
        ScreenCapture {
            area,
            top: 0,
            rows: (0..area.height).map(|_| line()).collect(),
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

    /// A pane drawing `lines` from its first column: blank cells copy as
    /// nothing, and a `None` line is a frame no selection copies.
    fn printed(area: Rect, lines: &[Option<&str>]) -> ScreenCapture {
        let row = |line: Option<&str>| {
            (0..area.width)
                .map(|column| {
                    let symbol = line
                        .and_then(|text| text.chars().nth(usize::from(column)))
                        .unwrap_or(' ');
                    ScreenCell {
                        symbol: symbol.to_string(),
                        selectable: symbol != ' ',
                        copyable: line.is_some(),
                    }
                })
                .collect()
        };
        ScreenCapture {
            area,
            top: 0,
            rows: lines.iter().map(|&line| row(line)).collect(),
        }
    }

    #[test]
    fn blank_lines_inside_a_selection_are_copied_and_frames_are_not() {
        let area = Rect::new(0, 0, 4, 6);
        let lines = [Some(""), Some("1"), Some(""), None, Some("1"), Some("")];
        let mut editor = Editor::new(String::new());
        editor.captures = vec![printed(area, &lines)];

        let left = MouseButton::Left;
        editor.handle_screen_mouse(mouse(MouseEventKind::Down(left), 0, 0));
        editor.handle_screen_mouse(mouse(MouseEventKind::Drag(left), 3, 5));
        let copied = editor.handle_screen_mouse(mouse(MouseEventKind::Up(left), 3, 5));

        let Some(Reaction::Copy(text)) = copied else {
            panic!("a drag copies its text");
        };
        assert_eq!(text, "1\n\n1");
    }

    #[test]
    fn a_selection_dragged_out_of_its_pane_stays_inside_it() {
        let shell = Rect::new(0, 0, 4, 2);
        let view = Rect::new(5, 0, 4, 2);
        let mut editor = Editor::new(String::new());
        editor.captures = vec![pane(shell, "s"), pane(view, "v")];

        let left = MouseButton::Left;
        editor.handle_screen_mouse(mouse(MouseEventKind::Down(left), view.x + 1, 0));
        editor.handle_screen_mouse(mouse(MouseEventKind::Drag(left), shell.x, 1));
        let copied = editor.handle_screen_mouse(mouse(MouseEventKind::Up(left), shell.x, 1));

        let Some(Reaction::Copy(text)) = copied else {
            panic!("a drag copies its text");
        };
        assert!(!text.contains('s'), "{text:?} reached into the shell");
        assert_eq!(text, "vvv\nv");
    }
}
