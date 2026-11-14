//! Units of text: what a double click picks out, how far a cursor motion goes,
//! and what growing a selection takes in next. The input reads these from its
//! source, the transcript from the cells it drew, tall fences included.

use std::ops::Range;

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

/// Which way a motion goes through text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDirection {
    Backward,
    Forward,
}

/// A unit of text, from the smallest up: a cursor steps over one at a time, and a
/// growing selection takes in the smallest one around what it already holds.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TextMotion {
    Character,
    /// A run of word characters, or of punctuation.
    WordAlpha,
    /// A run of anything but whitespace.
    WordAny,
    /// What lies between a bracket and its partner, then the brackets with it.
    DelimitedBlock,
    Line,
    /// The lines between blank ones.
    Paragraph,
    Full,
}

impl TextMotion {
    /// The units a selection grows through, smallest first. Characters are too
    /// small to be worth a step, and whitespace-delimited words reach across
    /// brackets, so neither is one.
    const GROWTH: [Self; 5] = [
        Self::WordAlpha,
        Self::DelimitedBlock,
        Self::Line,
        Self::Paragraph,
        Self::Full,
    ];

    /// Where one step from `from` in `direction` lands in `text`.
    ///
    /// Word steps cross whitespace first; a paragraph step lands on the blank line
    /// beside the next paragraph; a block step backward lands on the bracket
    /// opening the block the cursor is in, and forward just past the one closing
    /// it, so repeating it climbs outward.
    pub fn reach(self, text: &str, from: usize, direction: TextDirection) -> usize {
        let (before, after) = text.split_at(from);
        let forward = direction == TextDirection::Forward;
        match self {
            Self::Character if forward => from + after.chars().next().map_or(0, char::len_utf8),
            Self::Character => from - before.chars().next_back().map_or(0, char::len_utf8),
            Self::WordAlpha | Self::WordAny => {
                let same = |class: Option<CharClass>, character: char| match self {
                    Self::WordAny => !character.is_whitespace(),
                    _ => Some(CharClass::of(character)) == class,
                };
                if forward {
                    let trimmed = after.trim_start();
                    let class = trimmed.chars().next().map(CharClass::of);
                    let run: usize = trimmed
                        .chars()
                        .take_while(|&character| same(class, character))
                        .map(char::len_utf8)
                        .sum();
                    text.len() - trimmed.len() + run
                } else {
                    let trimmed = before.trim_end();
                    let class = trimmed.chars().next_back().map(CharClass::of);
                    let run: usize = trimmed
                        .chars()
                        .rev()
                        .take_while(|&character| same(class, character))
                        .map(char::len_utf8)
                        .sum();
                    trimmed.len() - run
                }
            }
            Self::DelimitedBlock if forward => unmatched(
                after
                    .char_indices()
                    .map(|(index, character)| (from + index, character)),
                false,
            )
            .map_or(text.len(), |close| close + 1),
            Self::DelimitedBlock => unmatched(before.char_indices().rev(), true).unwrap_or(0),
            Self::Line if forward => after.find('\n').map_or(text.len(), |index| from + index),
            Self::Line => before.rfind('\n').map_or(0, |index| index + 1),
            Self::Paragraph => {
                let lines = lines(text);
                let at = line_at(&lines, from);
                let blank = |index: usize| text[lines[index].clone()].trim().is_empty();
                let edge = |index: usize, beside: usize| blank(index) && !blank(beside);
                match direction {
                    TextDirection::Forward => (at + 1..lines.len())
                        .find(|&index| edge(index, index - 1))
                        .map_or(text.len(), |index| lines[index].start),
                    TextDirection::Backward => (0..at)
                        .rev()
                        .find(|&index| edge(index, index + 1))
                        .map_or(0, |index| lines[index].start),
                }
            }
            Self::Full if forward => text.len(),
            Self::Full => 0,
        }
    }

    /// The units of this kind holding all of `range`, innermost first; a block
    /// counts once without its brackets and once with them.
    pub fn around(self, text: &str, range: Range<usize>) -> Vec<Range<usize>> {
        let extend = |admits: fn(char) -> bool| {
            let start = range.start
                - text[..range.start]
                    .chars()
                    .rev()
                    .take_while(|&character| admits(character))
                    .map(char::len_utf8)
                    .sum::<usize>();
            let end = range.end
                + text[range.end..]
                    .chars()
                    .take_while(|&character| admits(character))
                    .map(char::len_utf8)
                    .sum::<usize>();
            let inside = text[range.clone()].chars().all(admits);
            (inside && start < end).then_some(start..end)
        };
        match self {
            Self::Character => text[range.start..]
                .chars()
                .next()
                .filter(|_| range.is_empty())
                .map(|character| range.start..range.start + character.len_utf8())
                .into_iter()
                .collect(),
            Self::WordAlpha => extend(|character| CharClass::of(character) == CharClass::Word)
                .into_iter()
                .collect(),
            Self::WordAny => extend(|character| !character.is_whitespace())
                .into_iter()
                .collect(),
            Self::DelimitedBlock => enclosing_blocks(text, range),
            Self::Line => {
                let start = Self::Line.reach(text, range.start, TextDirection::Backward);
                let end = Self::Line.reach(text, range.end, TextDirection::Forward);
                let line = start..end;
                vec![line]
            }
            Self::Paragraph => {
                let lines = lines(text);
                let blank = |line: &Range<usize>| text[line.clone()].trim().is_empty();
                let (first, last) = (line_at(&lines, range.start), line_at(&lines, range.end));
                if lines[first..=last].iter().all(blank) {
                    return Vec::new();
                }
                let first = lines[..first]
                    .iter()
                    .rposition(blank)
                    .map_or(0, |gap| gap + 1);
                let last = lines[last..]
                    .iter()
                    .position(blank)
                    .map_or(lines.len(), |gap| last + gap)
                    - 1;
                let paragraph = lines[first].start..lines[last].end;
                vec![paragraph]
            }
            Self::Full => {
                let whole = 0..text.len();
                vec![whole]
            }
        }
    }

    /// The smallest unit of any kind strictly holding `range`, what one step of
    /// growing a selection takes in.
    pub fn grow(text: &str, range: Range<usize>) -> Option<Range<usize>> {
        Self::GROWTH
            .iter()
            .flat_map(|motion| motion.around(text, range.clone()))
            .filter(|unit| unit.start <= range.start && range.end <= unit.end)
            .filter(|unit| unit.len() > range.len())
            .min_by_key(Range::len)
    }

    /// Where the bracket at `cursor`, or else the one just before it, has its
    /// partner: on the opening one, or just past the closing one.
    pub fn matching_bracket(text: &str, cursor: usize) -> Option<usize> {
        let (at, partner) = bracket_pair_at(text, cursor)?;
        Some(match at < partner {
            true => partner + 1,
            false => partner,
        })
    }
}

/// The bracket at `cursor`, or else the one just before it, and its partner's
/// position.
pub fn bracket_pair_at(text: &str, cursor: usize) -> Option<(usize, usize)> {
    let under = text[cursor..]
        .chars()
        .next()
        .map(|character| (cursor, character));
    let before = text[..cursor]
        .char_indices()
        .next_back()
        .filter(|&(_, character)| Bracket::of(character).is_some_and(|bracket| !bracket.opening));
    [under, before]
        .into_iter()
        .flatten()
        .find_map(|(at, character)| {
            let bracket = Bracket::of(character)?;
            let after = at + character.len_utf8();
            let partner = match bracket.opening {
                true => bracket.partner(
                    text[after..]
                        .char_indices()
                        .map(|(index, character)| (after + index, character)),
                ),
                false => bracket.partner(text[..at].char_indices().rev()),
            }?;
            Some((at, partner))
        })
}

/// The first bracket among `characters` left without a partner among those
/// before it: an opening one when reading backward, a closing one forward.
fn unmatched(characters: impl Iterator<Item = (usize, char)>, opening: bool) -> Option<usize> {
    let mut depth = 0usize;
    for (position, character) in characters {
        match Bracket::of(character) {
            Some(bracket) if bracket.opening == opening => match depth.checked_sub(1) {
                Some(outer) => depth = outer,
                None => return Some(position),
            },
            Some(_) => depth += 1,
            None => {}
        }
    }
    None
}

/// Every block holding `range`, innermost first, each without and then with its
/// brackets.
fn enclosing_blocks(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let mut blocks = Vec::new();
    let mut from = range.start;
    while let Some(open) = unmatched(text[..from].char_indices().rev(), true) {
        let close = unmatched(
            text[open + 1..]
                .char_indices()
                .map(|(index, character)| (open + 1 + index, character)),
            false,
        );
        let Some(close) = close else { break };
        if close >= range.end {
            blocks.extend([open + 1..close, open..close + 1]);
        }
        from = open;
    }
    blocks
}

/// The byte range of every line, without its line break.
fn lines(text: &str) -> Vec<Range<usize>> {
    let mut offset = 0;
    text.split('\n')
        .map(|line| {
            let range = offset..offset + line.len();
            offset = range.end + 1;
            range
        })
        .collect()
}

fn line_at(lines: &[Range<usize>], offset: usize) -> usize {
    lines
        .iter()
        .position(|line| line.start <= offset && offset <= line.end)
        .unwrap_or(lines.len() - 1)
}
