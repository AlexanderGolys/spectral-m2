//! What a double click picks out of text: a run of word characters, or a block
//! from one bracket to its partner. The input reads these from its source, the
//! transcript from the cells it drew, tall fences included.

use crate::ui::render::maths::{SHORT_BRACES, TALL_FENCES};

/// Brackets that enclose a block, as opening and closing characters.
const BRACKET_PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

/// What word-wise motions and a double click treat alike: a word is a run of one
/// class.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CharClass {
    Word,
    Punctuation,
    Space,
}

impl CharClass {
    pub fn of(character: char) -> Self {
        if character.is_whitespace() {
            Self::Space
        } else if character.is_alphanumeric() || character == '_' || character == '\'' {
            Self::Word
        } else {
            Self::Punctuation
        }
    }
}

/// One side of a bracket pair.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bracket {
    pub open: char,
    pub close: char,
    pub opening: bool,
}

impl Bracket {
    pub fn of(character: char) -> Option<Self> {
        BRACKET_PAIRS.iter().find_map(|&(open, close)| {
            let opening = match character {
                _ if character == open => true,
                _ if character == close => false,
                _ => return None,
            };
            Some(Self {
                open,
                close,
                opening,
            })
        })
    }

    /// The bracket a fence grown taller than a row begins with, read from its
    /// top piece; none for any other glyph.
    pub fn of_fence_top(glyph: &str) -> Option<Self> {
        SHORT_BRACES
            .iter()
            .chain(&TALL_FENCES)
            .find(|(_, [top, ..])| *top == glyph)
            .and_then(|(fence, _)| Self::of(fence.chars().next()?))
    }

    /// Whether `glyph` is any piece of a fence grown taller than a row.
    pub fn is_fence_piece(glyph: &str) -> bool {
        !glyph.is_empty()
            && SHORT_BRACES
                .iter()
                .chain(&TALL_FENCES)
                .any(|(_, pieces)| pieces.contains(&glyph))
    }

    /// The character this side is written as.
    pub fn character(self) -> char {
        match self.opening {
            true => self.open,
            false => self.close,
        }
    }

    /// The position of this bracket's partner among `beyond`: the characters
    /// after it in reading order when it opens, those before it nearest first
    /// when it closes. Brackets of the same pair nested between are skipped.
    pub fn partner<P>(self, beyond: impl IntoIterator<Item = (P, char)>) -> Option<P> {
        let (same, partner) = match self.opening {
            true => (self.open, self.close),
            false => (self.close, self.open),
        };
        let mut depth = 0usize;
        for (position, character) in beyond {
            if character == same {
                depth += 1;
            } else if character == partner {
                match depth.checked_sub(1) {
                    Some(outer) => depth = outer,
                    None => return Some(position),
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bracket_finds_its_partner_past_nested_pairs() {
        let text = "f((a), [b]) + c";
        let at = |index: usize| text[index..].chars().next().unwrap();
        let open = Bracket::of(at(1)).unwrap();
        let close = open.partner(text.char_indices().skip(2));
        assert_eq!(close, Some(10));

        let back = Bracket::of(at(10)).unwrap();
        assert_eq!(back.partner(text[..10].char_indices().rev()), Some(1));
    }

    #[test]
    fn every_tall_fence_reads_back_as_its_bracket() {
        for (fence, pieces) in TALL_FENCES.iter().chain(&SHORT_BRACES) {
            let Some(bracket) = Bracket::of(fence.chars().next().unwrap()) else {
                continue;
            };
            assert_eq!(Bracket::of_fence_top(pieces[0]), Some(bracket), "{fence}");
            assert!(pieces
                .iter()
                .filter(|piece| !piece.is_empty())
                .all(|piece| Bracket::is_fence_piece(piece)));
        }
    }
}
