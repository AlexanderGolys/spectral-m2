//! Source-file URLs and the Macaulay2 positions they identify.

use std::ops::Range;
use std::path::PathBuf;

use url::Url;

/// M2 source positions have one-based lines and zero-based columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourcePoint {
    pub line: usize,
    pub column: usize,
}

impl SourcePoint {
    fn parse(point: &str) -> Option<Self> {
        let (line, column) = point.strip_prefix('L')?.split_once(":C")?;
        Some(Self {
            line: line.parse::<usize>().ok()?.max(1),
            column: column.parse().ok()?,
        })
    }

    fn offset(self, source: &str) -> usize {
        let mut offset = 0;
        for (index, line) in source.split_inclusive('\n').enumerate() {
            if index + 1 == self.line {
                return offset
                    + line
                        .trim_end_matches(['\r', '\n'])
                        .char_indices()
                        .nth(self.column)
                        .map_or(line.trim_end_matches(['\r', '\n']).len(), |(index, _)| {
                            index
                        });
            }
            offset += line.len();
        }
        source.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceRange {
    pub start: SourcePoint,
    pub end: Option<SourcePoint>,
}

impl SourceRange {
    pub fn parse(fragment: &str) -> Option<Self> {
        let (start, end) = fragment
            .split_once('-')
            .map_or((fragment, None), |(start, end)| (start, Some(end)));
        Some(Self {
            start: SourcePoint::parse(start)?,
            end: match end {
                Some(end) => Some(SourcePoint::parse(end)?),
                None => None,
            },
        })
    }

    /// M2 ranges include their final column; a point highlights the rest of its line.
    pub fn bytes(self, source: &str) -> Range<usize> {
        let start = self.start.offset(source);
        let end = match self.end {
            Some(point) => {
                let offset = point.offset(source);
                offset + source[offset..].chars().next().map_or(0, char::len_utf8)
            }
            None => start + source[start..].find('\n').unwrap_or(source.len() - start),
        };
        start..end.max(start)
    }
}

/// A file target retained independently of its display text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLink {
    pub path: PathBuf,
    pub range: Option<SourceRange>,
}

impl SourceLink {
    pub fn parse(target: &str) -> Option<Self> {
        let url = Url::parse(target).ok()?;
        let path = url.to_file_path().ok()?;
        let range = match url.fragment() {
            Some(fragment) => Some(SourceRange::parse(fragment)?),
            None => None,
        };
        Some(Self { path, range })
    }

    /// Ask M2 for source markup rather than the file's contents as a String.
    pub fn command(&self) -> String {
        let file =
            serde_json::to_string(&self.path.to_string_lossy()).expect("file path serializes");
        let coordinates = match self.range {
            Some(range) => {
                let end = range.end.unwrap_or(range.start);
                format!(
                    "{}, {}, {}, {}",
                    range.start.line, range.start.column, end.line, end.column
                )
            }
            None => format!("1, 0, # lines get {file}, 0"),
        };
        format!("code new FilePosition from {{{file}, {coordinates}}}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_file_targets_keep_their_source_range() {
        let link = SourceLink::parse("file:///tmp/source%20file.m2#L2:C1-L3:C2").unwrap();
        assert_eq!(link.path, PathBuf::from("/tmp/source file.m2"));
        let source = "header\nabcdef\nuvwxyz\ntail";
        assert_eq!(&source[link.range.unwrap().bytes(source)], "bcdef\nuvw");
        assert!(link.command().contains("2, 1, 3, 2"));
        assert!(link.command().starts_with("code new FilePosition"));
    }

    #[test]
    fn source_columns_never_split_a_unicode_character() {
        let source = "aλ界z\n";
        let range = SourceRange::parse("L1:C1-L1:C2").unwrap();
        assert_eq!(&source[range.bytes(source)], "λ界");
        assert!(SourceRange::parse("L1:C2-Loops").is_none());
    }
}
