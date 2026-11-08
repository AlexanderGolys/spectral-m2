//! Completion candidates, their inline hint, and the list opened before insertion.

use std::ops::Range;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::ui::theme::groups::Groups;

/// What the typed text in `range` may be completed to, in order of preference.
pub struct Completion {
    range: Range<usize>,
    candidates: Vec<String>,
    chosen: usize,
    /// Whether the chosen candidate has replaced the typed text, so `range`
    /// spans it.
    taken: bool,
    /// Whether Tab has opened the candidate list.
    listed: bool,
}

impl Completion {
    /// The candidates in `text` for `range` that extend what is typed there, or
    /// `None` when none does.
    pub fn new(text: &str, range: Range<usize>, candidates: Vec<String>) -> Option<Self> {
        let typed = &text[range.clone()];
        let candidates: Vec<String> = candidates
            .into_iter()
            .filter(|candidate| candidate.len() > typed.len() && candidate.starts_with(typed))
            .collect();
        (!candidates.is_empty()).then_some(Self {
            range,
            candidates,
            chosen: 0,
            taken: false,
            listed: false,
        })
    }

    pub fn chosen(&self) -> &str {
        &self.candidates[self.chosen]
    }

    /// Whether the number of candidates permits an inline hint.
    pub fn hinted(&self, below: u16) -> bool {
        !self.taken && self.candidates.len() < usize::from(below)
    }

    /// Open the list before insertion when no inline hint was offered.
    /// Returns whether this key only opened the list.
    pub fn reveal(&mut self, below: u16) -> bool {
        if self.listed {
            return false;
        }
        let reveal_only = !self.taken && !self.hinted(below);
        self.listed = true;
        reveal_only
    }

    /// Rows needed by the open list, including its border.
    pub fn list_height(&self) -> u16 {
        if self.listed {
            u16::try_from(self.candidates.len())
                .unwrap_or(u16::MAX)
                .saturating_add(2)
        } else {
            0
        }
    }

    /// Draw the candidates with the chosen one visible even in a short pane.
    pub fn render_list(&self, area: Rect, buffer: &mut Buffer, groups: &Groups) {
        let height = usize::from(area.height.saturating_sub(2));
        let first = (self.chosen + 1).saturating_sub(height);
        let lines: Vec<_> = self
            .candidates
            .iter()
            .enumerate()
            .skip(first)
            .take(height)
            .map(|(index, name)| {
                let selected = index == self.chosen;
                let marker = if selected { "> " } else { "  " };
                Line::from(vec![
                    Span::styled(marker, groups.picker_selected_marker()),
                    Span::raw(name.clone()),
                ])
                .style(if selected {
                    groups.picker().patch(groups.picker_selected())
                } else {
                    groups.picker()
                })
            })
            .collect();
        Paragraph::new(lines)
            .style(groups.picker())
            .block(
                Block::default()
                    .borders(Borders::TOP | Borders::BOTTOM)
                    .border_style(groups.picker_border()),
            )
            .render(area, buffer);
    }

    /// The part of the chosen candidate not yet typed.
    pub fn ghost(&self) -> &str {
        &self.chosen()[self.range.len()..]
    }

    /// Put a candidate in place of the typed text and return the range it
    /// replaces and what replaces it: the chosen one the first time, then the
    /// next (`forward`) or previous one each time after, wrapping around.
    pub fn take(&mut self, forward: bool) -> (Range<usize>, &str) {
        let count = self.candidates.len();
        if self.taken || !forward {
            self.chosen = if forward {
                (self.chosen + 1) % count
            } else {
                (self.chosen + count - 1) % count
            };
        }
        self.taken = true;
        let replaced = self.range.clone();
        self.range = replaced.start..replaced.start + self.chosen().len();
        (replaced, &self.candidates[self.chosen])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hint_threshold_is_strict_and_a_hint_inserts_as_the_list_opens() {
        let mut completion =
            Completion::new("i", 0..1, vec!["ideal".into(), "image".into()]).unwrap();
        assert!(!completion.hinted(0));
        assert!(!completion.hinted(2));
        assert!(completion.hinted(3));
        assert!(!completion.reveal(3));
        assert_eq!(completion.list_height(), 4);
        assert_eq!(completion.take(true).1, "ideal");
        assert!(!completion.hinted(3));
        assert_eq!(completion.take(false).1, "image");
    }

    #[test]
    fn a_short_candidate_list_keeps_the_cycled_candidate_visible() {
        let mut completion = Completion::new(
            "i",
            0..1,
            vec!["ideal".into(), "image".into(), "identity".into()],
        )
        .unwrap();
        assert!(completion.reveal(0));
        completion.take(true);
        completion.take(false);
        let area = Rect::new(0, 0, 20, 3);
        let mut buffer = Buffer::empty(area);
        completion.render_list(area, &mut buffer, &Groups::default());
        let row: String = (0..area.width).map(|x| buffer[(x, 1)].symbol()).collect();
        assert!(row.starts_with("> identity"), "{row}");
    }

    #[test]
    fn only_candidates_extending_the_typed_text_are_offered() {
        let candidates = ["ideal", "id", "identity", "image"]
            .map(String::from)
            .to_vec();
        let mut completion = Completion::new("x = id", 4..6, candidates).unwrap();

        assert_eq!(completion.ghost(), "eal");
        let (replaced, taken) = completion.take(true);
        assert_eq!((replaced, taken), (4..6, "ideal"));
        let (replaced, taken) = completion.take(true);
        assert_eq!((replaced, taken), (4..9, "identity"));
        let (_, taken) = completion.take(false);
        assert_eq!(taken, "ideal");

        assert!(Completion::new("id", 0..2, vec!["id".to_string()]).is_none());
    }
}
