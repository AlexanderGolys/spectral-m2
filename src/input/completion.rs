//! An inline completion: the candidates for the text just before the cursor,
//! the rest of the chosen one drawn after the cursor, ready to be taken, and
//! once taken cycled through in place.

use std::ops::Range;

/// What the typed text in `range` may be completed to, in order of preference.
pub struct Completion {
    range: Range<usize>,
    candidates: Vec<String>,
    chosen: usize,
    /// Whether the chosen candidate has replaced the typed text, so `range`
    /// spans it.
    taken: bool,
    /// Whether the suggestion is drawn as a muted hint rather than as text.
    hinted: bool,
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
            hinted: false,
        })
    }

    /// Draw the suggestion as a muted hint, as for a command, rather than as text.
    pub fn hinted(self) -> Self {
        Self {
            hinted: true,
            ..self
        }
    }

    pub fn is_hint(&self) -> bool {
        self.hinted
    }

    pub fn chosen(&self) -> &str {
        &self.candidates[self.chosen]
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
