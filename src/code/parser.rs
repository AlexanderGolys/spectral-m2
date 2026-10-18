//! Local syntax analysis: is a buffer finished, and where do its cells begin?
//!
//! Both answers come from `m2-syn`'s native lexer and precedence parser, in
//! process. The rule for "not finished yet" is the one M2 itself uses: parsing
//! ran out of input rather than tripping over something. A lexer that ended
//! inside a string, raw string, block comment, or delimited group says so
//! directly; the parser's failures say so by their position, which sits at the
//! end of the source.

use std::ops::Range;

use m2_syn::native::{parse_native, NativeParseError};
use m2_syn::nodes::SourceFile;
use m2_syn::{LexErrorKind, PrettyNode, PrettyTree, Spanned};

/// One parsed evaluation unit and its rendered syntax tree.
#[derive(Debug, PartialEq, Eq)]
pub struct ParsedCell {
    pub range: Range<usize>,
    pub tree: PrettyNode,
}

/// A syntax error message paired with its exact source range when available.
#[derive(Debug, PartialEq, Eq)]
pub struct SyntaxDiagnostic {
    pub message: String,
    pub range: Option<Range<usize>>,
}

/// How a submitted buffer classifies.
///
/// A complete buffer carries its top-level cells, since finding them is what the
/// parse already did: splitting them out separately would parse the source a
/// second time and leave the two answers free to disagree.
#[derive(Debug, PartialEq)]
pub enum ParseVerdict {
    Complete(Vec<ParsedCell>),
    Incomplete(String),
    SyntaxError(SyntaxDiagnostic),
}

/// Did the lexer stop because the input ran out inside something it had opened?
fn lexer_ran_out(kind: LexErrorKind) -> bool {
    matches!(
        kind,
        LexErrorKind::UnterminatedGroup
            | LexErrorKind::UnterminatedString
            | LexErrorKind::UnterminatedRawString
            | LexErrorKind::UnterminatedBlockComment
    )
}

/// Did this failure happen because the input ran out, rather than because
/// something already typed is wrong? That is M2's own test for "keep typing",
/// and each error kind answers it differently:
///
/// * the lexer says so directly, by ending inside something it had opened;
/// * a missing operand means the operator is still waiting, provided nothing
///   follows it -- `a*` waits, `if;` does not;
/// * anything else reports an *empty* span when it hit the end of input, and a
///   span covering a real token when it tripped over one. `if 1` ends at an
///   empty span; `then` and `**` cover the token that cannot be there.
fn ran_out_of_input(error: &NativeParseError, source: &str) -> bool {
    let span = error.span();
    let reaches_end = span.end().is_some_and(|end| end >= source.trim_end().len());
    match error {
        NativeParseError::Lex(error) => lexer_ran_out(error.kind()),
        NativeParseError::MissingOperand { .. } => reaches_end,
        _ => reaches_end && span.start() == span.end(),
    }
}

pub fn incomplete_reason(error: &NativeParseError) -> String {
    let message = error.to_string().replace("unterminated", "unclosed");
    message
        .split_once(": expected")
        .or_else(|| message.split_once(", found"))
        .map_or(message.as_str(), |(reason, _)| reason)
        .to_string()
}

/// Classify a buffer without evaluating it.
pub fn check(source: &str) -> ParseVerdict {
    match parse_native(source) {
        Ok(file) => ParseVerdict::Complete(parsed_cells(&file, source.len())),
        Err(error) if ran_out_of_input(&error, source) => {
            ParseVerdict::Incomplete(incomplete_reason(&error))
        }
        Err(error) => {
            let range = error
                .span()
                .bounds()
                .map(|(start, end)| start.min(source.len())..end.min(source.len()));
            ParseVerdict::SyntaxError(SyntaxDiagnostic {
                message: error.to_string(),
                range,
            })
        }
    }
}

/// The byte ranges of a parsed file's top-level cells. Together they partition
/// the complete source: the first owns leading whitespace and each cell owns the
/// whitespace following it, while its syntax tree still describes its content.
fn parsed_cells(file: &SourceFile, source_len: usize) -> Vec<ParsedCell> {
    let mut cells = file
        .cells
        .iter()
        .map(|cell| {
            let start = cell.content.span().start()?;
            let end = match &cell.semicolon {
                Some(semicolon) => semicolon.span().end()?,
                None => cell.content.span().end()?,
            };
            Some(ParsedCell {
                range: start..end,
                tree: cell.pretty_tree(),
            })
        })
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();
    for index in 0..cells.len() {
        cells[index].range.end = cells
            .get(index + 1)
            .map_or(source_len, |next| next.range.start);
    }
    if let Some(first) = cells.first_mut() {
        first.range.start = 0;
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cells of a buffer, as text, for tests that care about the split.
    fn cells_of(source: &str) -> Vec<&str> {
        let ParseVerdict::Complete(ranges) = check(source) else {
            panic!("expected {source:?} to parse");
        };
        ranges.into_iter().map(|cell| &source[cell.range]).collect()
    }

    // The verdicts M2's own oracle gives, kept as the specification the local
    // parser has to reproduce.
    #[test]
    fn reproduces_the_oracle_verdicts() {
        let complete = [
            "1+1",
            "a",
            "\"a\"",
            "a = 5;\nb = 6\n\n",
            "local symbol",
            "if if 1\n\nthen\n\t1\nelse \n 1then 1",
        ];
        let incomplete = [
            "a*",
            "*",
            "* *",
            "if",
            "(",
            "({{}",
            "symbol",
            "if if",
            "if 1",
            "if if 1 then 1",
            "if if 1\n\n",
            "if if 1\n\nthen ",
            "if if 1\n\nthen\n\t1\nelse \n 1then",
            "if if 1\n\nthen\n\t1\nelse \n 1",
            "if if 1\n\nthen\n\t1\nelse",
            "\"",
            "///",
            "/////",
        ];
        let errors = ["if;", "then", ")", "**"];

        let mut wrong = Vec::new();
        for source in complete {
            let verdict = check(source);
            if !matches!(verdict, ParseVerdict::Complete(_)) {
                wrong.push(format!("{source:?} => {verdict:?}, wanted Complete"));
            }
        }
        for source in incomplete {
            let verdict = check(source);
            if !matches!(verdict, ParseVerdict::Incomplete(_)) {
                wrong.push(format!("{source:?} => {verdict:?}, wanted Incomplete"));
            }
        }
        for source in errors {
            let verdict = check(source);
            if !matches!(verdict, ParseVerdict::SyntaxError(_)) {
                wrong.push(format!("{source:?} => {verdict:?}, wanted SyntaxError"));
            }
        }
        assert!(
            wrong.is_empty(),
            "verdict mismatches:\n{}",
            wrong.join("\n")
        );
    }

    #[test]
    fn splits_newline_delimited_cells() {
        let source = "a = 5\nb = 6";
        let cells = cells_of(source);
        assert_eq!(cells, vec!["a = 5\n", "b = 6"]);
    }

    #[test]
    fn single_cell_is_one_range() {
        assert_eq!(cells_of("1 + 1").len(), 1);
    }

    #[test]
    fn literal_delimiters_travel_with_their_cell() {
        for source in ["help \"escape\"", "print ///a///"] {
            assert_eq!(cells_of(source), vec![source]);
        }
    }

    // Blank and comment-only input is valid M2 with nothing in it. The session
    // relies on the empty cell list to leave such a submission alone.
    #[test]
    fn input_without_statements_parses_to_no_cells() {
        for source in ["", "   ", "\n\n", "-- just a note"] {
            assert!(
                cells_of(source).is_empty(),
                "expected no cells in {source:?}"
            );
        }
    }

    #[test]
    fn a_semicolon_travels_with_its_cell() {
        let source = "z = 7;\n1 + 1";
        let cells = cells_of(source);
        assert_eq!(cells, vec!["z = 7;\n", "1 + 1"]);
    }

    #[test]
    fn cell_ranges_preserve_every_source_byte() {
        let source = "  a = 5  \n\n\tb = 6  ";
        let cells = cells_of(source);
        assert_eq!(cells, ["  a = 5  \n\n\t", "b = 6  "]);
        assert_eq!(cells.concat(), source);
    }

    #[test]
    fn clean_code_has_no_error() {
        assert!(matches!(check("a = 5\nb = 6"), ParseVerdict::Complete(_)));
        assert!(matches!(check(";;"), ParseVerdict::SyntaxError(_)));
    }
}
