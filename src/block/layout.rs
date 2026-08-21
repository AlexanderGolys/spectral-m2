//! Range-based projections from owned text into visual lines.

use std::iter;
use std::ops::Range;

pub trait WrapPolicy {
    fn wrap_logical_line(line: &str, width: u16) -> Vec<Range<usize>>;
}

pub trait TextLayout {
    type Wrapping: WrapPolicy;

    fn text(&self) -> &str;

    fn lines(&self, width: u16) -> Vec<VisualLine> {
        wrap::<Self::Wrapping>(self.text(), width)
    }
}

/// A visual row represented as a byte range into its source text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualLine {
    pub range: Range<usize>,
}

pub struct PreserveLines;

impl WrapPolicy for PreserveLines {
    fn wrap_logical_line(line: &str, _width: u16) -> Vec<Range<usize>> {
        iter::once(0..line.len()).collect()
    }
}

pub struct WordWrap;

impl WrapPolicy for WordWrap {
    fn wrap_logical_line(line: &str, width: u16) -> Vec<Range<usize>> {
        let width = width as usize;
        if width == 0 || line.is_empty() {
            return iter::once(0..line.len()).collect();
        }

        let characters: Vec<(usize, char)> = line.char_indices().collect();
        let count = characters.len();
        let byte_at = |index: usize| characters.get(index).map_or(line.len(), |&(byte, _)| byte);
        let mut rows = Vec::new();
        let mut start = 0;

        while start < count {
            if count - start <= width {
                rows.push(byte_at(start)..line.len());
                break;
            }
            let hard_end = start + width;
            let (end, next) = if let Some(index) = (start + 1..hard_end)
                .rev()
                .find(|&index| characters[index].1 == ' ')
            {
                (index, index + 1)
            } else if let Some(index) = (start + 1..hard_end)
                .rev()
                .find(|&index| !is_word_character(characters[index].1))
            {
                (index, index)
            } else {
                (hard_end, hard_end)
            };
            rows.push(byte_at(start)..byte_at(end));
            start = next;
        }
        rows
    }
}

pub fn wrap<Policy: WrapPolicy>(text: &str, width: u16) -> Vec<VisualLine> {
    let mut lines = Vec::new();
    let mut logical_start = 0;
    for logical in text.split('\n') {
        lines.extend(
            Policy::wrap_logical_line(logical, width)
                .into_iter()
                .map(|range| VisualLine {
                    range: logical_start + range.start..logical_start + range.end,
                }),
        );
        logical_start += logical.len() + 1;
    }
    lines
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '\'' || character == '$'
}
