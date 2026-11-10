//! Questions put to the language server about analysed source, and its answers in
//! the source's own terms: byte offsets and the segments holding them.

use std::ops::{Range, RangeInclusive};
use std::path::PathBuf;
use std::sync::Arc;

use lsp_types::{DiagnosticSeverity, Position};

use super::{Document, SourceId};
use crate::ui::theme::groups::HLGroup;

/// What is asked. A point is a byte offset into the asked document's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Question {
    /// What the name at the point is: its signature, kind and documentation.
    Hover(usize),
    /// Where the name at the point is defined.
    Definition(usize),
    /// Every reference to the name at the point within the document.
    Highlights(usize),
    /// The signatures of the call enclosing the point.
    Signature(usize),
    /// The document laid out as the server formats Macaulay2.
    Format,
    /// The stretches of lines the document can fold away.
    Folds,
}

impl Question {
    /// Whether the question is about a document of its own rather than the
    /// session, which is opened only to be asked and closed again.
    pub fn apart(self) -> bool {
        matches!(self, Self::Format | Self::Folds)
    }
}

/// Identifies a question, to match its answer arriving later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QueryId(pub u64);

/// A question about `document`.
#[derive(Clone, Debug)]
pub struct Query {
    pub id: QueryId,
    pub document: Arc<Document>,
    pub question: Question,
}

/// Where a definition lives.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Place {
    /// In the asked document, within one of its segments.
    Source(SourceId, Range<usize>),
    /// In a file, at a zero-based line and UTF-16 column, as the server counts.
    File {
        path: PathBuf,
        line: u32,
        column: u32,
    },
}

/// One signature of a call, with the parameter the point is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub label: String,
    /// The byte range of the active parameter within `label`.
    pub active: Option<Range<usize>>,
}

/// The server's answer to a [`Query`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Answer {
    /// Markdown describing the name, or nothing to say.
    Hover(Option<String>),
    Definition(Vec<Place>),
    Highlights(Vec<(SourceId, Range<usize>)>),
    /// Every candidate signature, the active one first.
    Signature(Vec<Signature>),
    /// The formatted document, or nothing when it is already formatted or cannot be.
    Formatted(Option<String>),
    /// Foldable stretches of zero-based lines, each kept to its first line when folded.
    Folds(Vec<RangeInclusive<u32>>),
}

/// How serious a finding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

impl Severity {
    /// How a finding of this severity marks the source it is about.
    pub fn group(self) -> HLGroup {
        match self {
            Self::Error => HLGroup::DiagnosticError,
            Self::Warning => HLGroup::DiagnosticWarning,
            Self::Information => HLGroup::DiagnosticInfo,
            Self::Hint => HLGroup::DiagnosticHint,
        }
    }

    /// The colour a finding of this severity is told in.
    pub fn message_group(self) -> HLGroup {
        match self {
            Self::Error => HLGroup::Error,
            Self::Warning => HLGroup::Warning,
            Self::Information => HLGroup::Info,
            Self::Hint => HLGroup::Hint,
        }
    }
}

impl From<Option<DiagnosticSeverity>> for Severity {
    fn from(severity: Option<DiagnosticSeverity>) -> Self {
        match severity {
            Some(DiagnosticSeverity::WARNING) => Self::Warning,
            Some(DiagnosticSeverity::INFORMATION) => Self::Information,
            Some(DiagnosticSeverity::HINT) => Self::Hint,
            _ => Self::Error,
        }
    }
}

/// A problem the server found in a segment, at a byte range within it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub range: Range<usize>,
    pub severity: Severity,
    pub message: String,
    /// The server's code for the kind of problem, such as `E13`.
    pub code: Option<String>,
}

/// A name the analysed source binds, with what the server knows it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub name: String,
    pub detail: Option<String>,
}

impl Document {
    /// The byte offset of `local` within the segment `id`.
    pub fn offset_in(&self, id: SourceId, local: usize) -> Option<usize> {
        self.segments
            .iter()
            .find(|segment| segment.id == id)
            .map(|segment| segment.range.start + local)
            .filter(|offset| *offset <= self.text.len())
    }

    /// `range` within the segment holding its start, clipped to that segment.
    pub fn place(&self, range: Range<usize>) -> Option<(SourceId, Range<usize>)> {
        let segment = self
            .segments
            .iter()
            .find(|segment| segment.range.contains(&range.start))?;
        let end = range.end.min(segment.range.end);
        Some((
            segment.id,
            range.start - segment.range.start..end - segment.range.start,
        ))
    }
}

/// The byte offset of an LSP position in `text`, counting columns in UTF-16 units.
pub fn offset_of(text: &str, position: Position) -> Option<usize> {
    let start = if position.line == 0 {
        0
    } else {
        text.match_indices('\n')
            .nth(position.line as usize - 1)
            .map(|(offset, _)| offset + 1)?
    };
    let line = text[start..].split('\n').next().unwrap_or_default();
    let mut units = 0;
    for (offset, character) in line.char_indices() {
        if units >= position.character {
            return Some(start + offset);
        }
        units += character.len_utf16() as u32;
    }
    Some(start + line.len())
}

/// The LSP position of byte `offset` in `text`, counting columns in UTF-16 units.
pub fn position_of(text: &str, offset: usize) -> Position {
    let before = &text[..offset.min(text.len())];
    let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
    Position::new(
        before.matches('\n').count() as u32,
        before[line_start..].encode_utf16().count() as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_and_offsets_agree_across_lines_and_wide_characters() {
        let text = "ab\nℤ→c\n";
        for offset in [0, 1, 3, 6, 9, 10] {
            assert_eq!(offset_of(text, position_of(text, offset)), Some(offset));
        }
        assert_eq!(position_of(text, 9), Position::new(1, 2));
    }

    #[test]
    fn an_offset_is_placed_in_the_segment_holding_it() {
        let mut document = Document::new(0);
        document.push(SourceId::Pending(0), "f = (");
        document.push(SourceId::Draft(0), "x)");
        let draft = document.offset_in(SourceId::Draft(0), 1).unwrap();
        assert_eq!(
            document.place(draft..draft + 1),
            Some((SourceId::Draft(0), 1..2))
        );
    }
}
