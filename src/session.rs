//! A Macaulay2 session: command handling, submission validation, cell evaluation,
//! transcript conversion, and prompt numbering.

use std::ops::Range;

use crate::block::highlight::{HighlightSpan, Highlighter};
use crate::block::{
    Block, CodeBlock, OutputBlock, RejectedBlock, RuntimeErrorBlock, SyntaxErrorBlock, SystemBlock,
    WarningBlock,
};
use crate::commands::{dispatch, CommandResult};
use crate::editor::EditorUpdate;
use crate::errors::M2Result;
use crate::m2_backend::{M2Backend, M2Response};
use crate::parse_check::ParseVerdict;
use crate::parser::{has_syntax_error, split_cells};
use crate::transcript::{captured_error, captured_output, strip_m2_location};

struct EvaluatedCell {
    code: String,
    output: String,
}

struct FailedCell {
    code: String,
    output: String,
    rejected: Vec<String>,
}

struct Evaluation {
    successful: Vec<EvaluatedCell>,
    failed: Option<FailedCell>,
}

pub struct Session {
    backend: M2Backend,
    next_input_number: u32,
    committed: Vec<String>,
}

impl Session {
    pub fn new(backend: M2Backend) -> Self {
        Self {
            backend,
            next_input_number: 1,
            committed: Vec::new(),
        }
    }

    pub fn version(&self) -> M2Result<String> {
        self.backend.version()
    }

    pub fn committed(&self) -> &[String] {
        &self.committed
    }

    pub fn submit(&mut self, source: &str, highlighter: &mut dyn Highlighter) -> EditorUpdate {
        if source.is_empty() {
            return EditorUpdate::Ignore;
        }

        if let Some(command) = dispatch(source) {
            return match command {
                CommandResult::Message(message) => {
                    EditorUpdate::Commit(vec![Box::new(SystemBlock {
                        text: message.to_string(),
                    })])
                }
                CommandResult::ToggleCode => EditorUpdate::ToggleCode,
                CommandResult::Unrecognized => EditorUpdate::Ignore,
            };
        }

        match self.backend.check(source) {
            Ok(ParseVerdict::Incomplete) => EditorUpdate::ContinueSubmission,
            Ok(ParseVerdict::SyntaxError(message)) => {
                EditorUpdate::Diagnostic(strip_m2_location(&message).to_string())
            }
            Ok(ParseVerdict::Complete) => self
                .submit_complete(source.trim(), highlighter)
                .unwrap_or_else(|error| EditorUpdate::Diagnostic(error.to_string())),
            Err(error) => EditorUpdate::Diagnostic(format!("Communication error: {error}")),
        }
    }

    fn submit_complete(
        &mut self,
        source: &str,
        highlighter: &mut dyn Highlighter,
    ) -> M2Result<EditorUpdate> {
        if has_syntax_error(source)? {
            return Ok(EditorUpdate::Commit(vec![Box::new(SyntaxErrorBlock {
                text: source.to_string(),
                message: "syntax error".to_string(),
            })]));
        }

        let ranges = split_cells(source)?;
        let evaluation = self.evaluate(source, &ranges);
        let start = self.next_input_number;
        self.next_input_number += evaluation.successful.len() as u32;
        self.committed
            .extend(evaluation.successful.iter().map(|cell| cell.code.clone()));
        Ok(EditorUpdate::Commit(evaluation.into_blocks(
            start,
            self.next_input_number,
            highlighter,
        )))
    }

    fn evaluate(&self, source: &str, ranges: &[Range<usize>]) -> Evaluation {
        let mut successful = Vec::new();
        let mut failed = None;

        for (index, range) in ranges.iter().enumerate() {
            let code = source[range.clone()].to_string();
            match self.backend.evaluate(&code) {
                Ok(M2Response {
                    is_error: true,
                    output,
                }) => {
                    failed = Some(FailedCell {
                        code,
                        output,
                        rejected: cells(source, &ranges[index + 1..]),
                    });
                    break;
                }
                Ok(M2Response { output, .. }) => successful.push(EvaluatedCell { code, output }),
                Err(error) => {
                    failed = Some(FailedCell {
                        code,
                        output: format!("Communication error: {error}"),
                        rejected: cells(source, &ranges[index + 1..]),
                    });
                    break;
                }
            }
        }

        Evaluation { successful, failed }
    }
}

impl Evaluation {
    fn into_blocks(
        self,
        start_number: u32,
        failed_number: u32,
        highlighter: &mut dyn Highlighter,
    ) -> Vec<Box<dyn Block>> {
        let mut blocks: Vec<Box<dyn Block>> = Vec::new();

        if !self.successful.is_empty() {
            let mut outputs: Vec<Box<dyn Block>> = Vec::new();
            for (offset, cell) in self.successful.iter().enumerate() {
                let number = start_number + offset as u32;
                let output = captured_output(&cell.output, &cell.code);
                outputs.extend(
                    output
                        .warnings
                        .into_iter()
                        .map(|text| Box::new(WarningBlock { text }) as Box<dyn Block>),
                );
                if !output.value.is_empty() || output.type_info.is_some() {
                    outputs.push(Box::new(OutputBlock {
                        number,
                        text: output.value,
                        ty: output.type_info,
                    }));
                }
            }

            let codes: Vec<String> = self.successful.into_iter().map(|cell| cell.code).collect();
            let spans = highlight_cells(&codes, highlighter);
            blocks.push(Box::new(CodeBlock::new(codes, spans, start_number)));
            blocks.extend(outputs);
        }

        if let Some(failed) = self.failed {
            let spans = highlighter.highlight(&failed.code);
            let message = captured_error(&failed.output, &failed.code);
            blocks.push(Box::new(RuntimeErrorBlock {
                number: failed_number,
                text: failed.code,
                spans,
                message,
            }));

            if !failed.rejected.is_empty() {
                let spans = highlight_cells(&failed.rejected, highlighter);
                blocks.push(Box::new(RejectedBlock::new(failed.rejected, spans)));
            }
        }

        blocks
    }
}

fn cells(source: &str, ranges: &[Range<usize>]) -> Vec<String> {
    ranges
        .iter()
        .map(|range| source[range.clone()].to_string())
        .collect()
}

fn highlight_cells(cells: &[String], highlighter: &mut dyn Highlighter) -> Vec<HighlightSpan> {
    highlighter.highlight(&cells.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::SyntaxHighlighter;

    #[test]
    fn submission_pipeline_preserves_input_numbering_across_failures() {
        let backend = M2Backend::spawn().expect("spawn M2");
        let mut session = Session::new(backend);
        let mut highlighter = SyntaxHighlighter::new();

        assert!(matches!(
            session.submit("a*", &mut highlighter),
            EditorUpdate::ContinueSubmission
        ));
        assert_eq!(session.next_input_number, 1);

        let EditorUpdate::Commit(success) = session.submit("1 + 1", &mut highlighter) else {
            panic!("complete input should commit transcript blocks");
        };
        assert_eq!(success.len(), 2);
        assert_eq!(session.next_input_number, 2);
        assert_eq!(session.committed(), &["1 + 1"]);

        let EditorUpdate::Commit(failure) = session.submit("1 / 0\n2 + 2", &mut highlighter) else {
            panic!("runtime failure should commit its transcript blocks");
        };
        assert_eq!(failure.len(), 2);
        assert_eq!(session.next_input_number, 2);
        assert_eq!(session.committed(), &["1 + 1"]);

        assert!(matches!(
            session.submit("2 + 2", &mut highlighter),
            EditorUpdate::Commit(_)
        ));
        assert_eq!(session.next_input_number, 3);
        assert_eq!(session.committed(), &["1 + 1", "2 + 2"]);
    }
}
