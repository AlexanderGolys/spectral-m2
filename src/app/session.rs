//! One independently evaluated Macaulay2 workspace.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use m2_syn::{NativeCellParser, NativeCellState, PrettyNode, PrettyTree};

use crate::backend::transcript::{captured_error, captured_output};
#[cfg(test)]
use crate::backend::M2Value;
use crate::backend::{M2Backend, M2Response};
use crate::code::highlight::Highlighter;
use crate::errors::Result;
use crate::ui::block::Block;
use crate::ui::cell::{CellState, OutputBlock, TextBlock, WarningBlock};
use crate::ui::controller::commands::{submitted_command, SubmittedCommand};
use crate::ui::controller::EditorUpdate;

/// One successfully evaluated input cell retained by its workspace.
#[derive(Clone)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct CommittedCell {
    pub(crate) number: u32,
    pub(crate) state: CellState,
    pub(crate) code: String,
    pub(crate) tree: PrettyNode,
}

/// A parsed cell waiting for its worker to evaluate it.
#[derive(Clone)]
struct SubmittedCell {
    code: String,
    tree: PrettyNode,
}

/// One submitted evaluation, active at the queue front and queued otherwise.
struct Evaluation {
    id: u64,
    cells: Vec<SubmittedCell>,
}

/// Work sent to the thread that exclusively owns an M2 process.
enum WorkerRequest {
    Evaluate {
        id: u64,
        first_index: usize,
        first_number: u32,
        cells: Vec<String>,
    },
    Restart,
    Stop,
}

/// Incremental results returned by one workspace worker.
enum WorkerEvent {
    Cell {
        id: u64,
        index: usize,
        number: u32,
        result: std::result::Result<M2Response, String>,
    },
    Restarted(std::result::Result<String, String>),
}

/// A live M2 worker and the source state committed through it.
pub(crate) struct Session {
    requests: Sender<WorkerRequest>,
    events: Receiver<WorkerEvent>,
    version: String,
    in_debugger: bool,
    next_input_number: u32,
    next_submission_id: u64,
    committed: Vec<CommittedCell>,
    committed_revision: u64,
    leading_whitespace: String,
    parser: NativeCellParser,
    evaluations: VecDeque<Evaluation>,
    restarting: bool,
}

impl Session {
    pub(crate) fn new(backend: M2Backend) -> Self {
        let version = backend.version().unwrap_or_else(|_| "unknown".to_string());
        let (request_tx, request_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        thread::spawn(move || worker(backend, request_rx, event_tx));
        Self {
            requests: request_tx,
            events: event_rx,
            version,
            in_debugger: false,
            next_input_number: 1,
            next_submission_id: 0,
            committed: Vec::new(),
            committed_revision: 0,
            leading_whitespace: String::new(),
            parser: NativeCellParser::new(),
            evaluations: VecDeque::new(),
            restarting: false,
        }
    }

    pub(crate) fn version(&self) -> Result<String> {
        Ok(self.version.clone())
    }

    #[cfg(test)]
    pub(crate) fn next_input_number(&self) -> u32 {
        self.next_input_number
    }

    pub(crate) fn in_debugger(&self) -> bool {
        self.in_debugger
    }

    pub(crate) fn committed(&self) -> &[CommittedCell] {
        &self.committed
    }

    /// Revision of the immutable committed-cell sequence.
    pub(crate) const fn committed_revision(&self) -> u64 {
        self.committed_revision
    }

    pub(crate) fn busy(&self) -> bool {
        !self.evaluations.is_empty() || self.restarting
    }

    pub(crate) fn restart(&mut self) -> EditorUpdate {
        if self.busy() {
            return EditorUpdate::Diagnostic("workspace is still evaluating".to_string());
        }
        if self.requests.send(WorkerRequest::Restart).is_err() {
            return EditorUpdate::Diagnostic("Macaulay2 worker stopped".to_string());
        }
        self.restarting = true;
        EditorUpdate::Ignore
    }

    pub(crate) fn submit(
        &mut self,
        source: &str,
        highlighter: &mut dyn Highlighter,
    ) -> EditorUpdate {
        if source.is_empty() {
            return EditorUpdate::Ignore;
        }
        if self.parser.active_source().is_empty() {
            if let Some(command) = submitted_command(source) {
                return match command {
                    SubmittedCommand::Quit => EditorUpdate::Quit,
                    SubmittedCommand::Restart => self.restart(),
                };
            }
        }
        if self.restarting {
            return EditorUpdate::Diagnostic("workspace is restarting".to_string());
        }
        if self.parser.active_source().is_empty() && source.chars().all(char::is_whitespace) {
            return self.append_whitespace(source);
        }

        let state = self.parser.push_line(source);
        let mut cells: Vec<_> = self
            .parser
            .take_completed()
            .into_iter()
            .enumerate()
            .map(|(index, cell)| SubmittedCell {
                code: match index {
                    0 => cell.source().trim_end(),
                    _ => cell
                        .source()
                        .trim_end()
                        .strip_prefix('\n')
                        .unwrap_or_else(|| cell.source().trim_end()),
                }
                .to_string(),
                tree: cell.syntax().pretty_tree(),
            })
            .collect();
        if let Some(first) = cells.first_mut() {
            first.code.insert_str(0, &self.leading_whitespace);
        }
        let mut updates = Vec::new();
        if !cells.is_empty() {
            updates.push(self.enqueue(cells, highlighter));
            self.leading_whitespace.clear();
        }
        match state {
            NativeCellState::Incomplete => {
                let mut source = self.leading_whitespace.clone();
                source.push_str(self.parser.active_source().trim_end());
                let reason = self.incomplete_reason();
                updates.push(EditorUpdate::Incomplete {
                    spans: highlighter.highlight(&source),
                    source,
                    reason,
                });
            }
            NativeCellState::Invalid => {
                let mut source = self.parser.active_source().to_string();
                source.insert_str(0, &self.leading_whitespace);
                source.truncate(source.trim_end().len());
                self.leading_whitespace.clear();
                let message = self.invalid_diagnostic();
                let spans = highlighter.highlight(&source);
                let id = self.next_id();
                self.parser.reset_active();
                updates.push(EditorUpdate::SyntaxFailed {
                    id,
                    source,
                    spans,
                    message,
                });
            }
            NativeCellState::Empty if !self.parser.active_source().is_empty() => {
                let source = self.parser.active_source().to_string();
                self.parser.reset_active();
                updates.push(self.append_whitespace(&source));
            }
            NativeCellState::Empty | NativeCellState::Complete => {}
        }
        Self::batch(updates)
    }

    /// Discard parser state after the feed transfers its unfinished block back.
    pub(crate) fn reset_incomplete(&mut self) {
        self.parser.reset_active();
        self.leading_whitespace.clear();
    }

    fn enqueue(
        &mut self,
        cells: Vec<SubmittedCell>,
        highlighter: &mut dyn Highlighter,
    ) -> EditorUpdate {
        let displayed = cells
            .iter()
            .map(|cell| {
                let code = cell.code.trim_end().to_string();
                let spans = highlighter.highlight(&code);
                (code, spans)
            })
            .collect();
        let id = self.next_id();
        let evaluation = Evaluation { id, cells };
        let pending = self.evaluations.is_empty();
        if pending && !self.dispatch(&evaluation, 0) {
            return EditorUpdate::Diagnostic("Macaulay2 worker stopped".to_string());
        }
        self.evaluations.push_back(evaluation);
        match pending {
            true => EditorUpdate::Pending {
                id,
                cells: displayed,
            },
            false => EditorUpdate::Queued {
                id,
                cells: displayed,
            },
        }
    }

    fn incomplete_reason(&self) -> String {
        self.parser.parse_error().map_or_else(
            || {
                self.parser
                    .lexer_diagnostics()
                    .last()
                    .map_or_else(|| "more input".to_string(), ToString::to_string)
                    .replace("unterminated", "unclosed")
            },
            |error| {
                let message = error.to_string().replace("unterminated", "unclosed");
                message
                    .split_once(": expected")
                    .or_else(|| message.split_once(", found"))
                    .map_or(message.as_str(), |(reason, _)| reason)
                    .to_string()
            },
        )
    }

    fn invalid_diagnostic(&self) -> String {
        match self.parser.parse_error() {
            Some(error) => error.to_string(),
            None => self
                .parser
                .lexer_diagnostics()
                .last()
                .map_or_else(|| "invalid syntax".to_string(), ToString::to_string),
        }
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_submission_id;
        self.next_submission_id += 1;
        id
    }

    fn batch(mut updates: Vec<EditorUpdate>) -> EditorUpdate {
        match updates.len() {
            0 => EditorUpdate::Ignore,
            1 => updates.pop().unwrap(),
            _ => EditorUpdate::Batch(updates),
        }
    }

    fn dispatch(&self, evaluation: &Evaluation, first_index: usize) -> bool {
        self.requests
            .send(WorkerRequest::Evaluate {
                id: evaluation.id,
                first_index,
                first_number: self.next_input_number,
                cells: evaluation
                    .cells
                    .iter()
                    .skip(first_index)
                    .map(|cell| cell.code.clone())
                    .collect(),
            })
            .is_ok()
    }

    fn append_whitespace(&mut self, source: &str) -> EditorUpdate {
        self.leading_whitespace.push_str(source);
        EditorUpdate::WhitespaceAccepted
    }

    pub(crate) fn poll(&mut self) -> Vec<EditorUpdate> {
        let mut updates = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            match event {
                WorkerEvent::Cell {
                    id,
                    index,
                    number,
                    result,
                } => updates.push(self.resolve_cell(id, index, number, result)),
                WorkerEvent::Restarted(result) => updates.push(self.resolve_restart(result)),
            }
        }
        updates
    }

    fn resolve_cell(
        &mut self,
        id: u64,
        index: usize,
        number: u32,
        result: std::result::Result<M2Response, String>,
    ) -> EditorUpdate {
        let Some(cell) = self
            .evaluations
            .front()
            .filter(|evaluation| evaluation.id == id)
            .and_then(|evaluation| evaluation.cells.get(index))
            .cloned()
        else {
            return EditorUpdate::Ignore;
        };
        match result {
            Ok(response) if !response.is_error => {
                self.in_debugger = response.depth > 1;
                self.next_input_number = number + 1;
                let state = match (response.has_value, response.value.is_some()) {
                    (true, true) => CellState::Value,
                    (true, false) => CellState::Muted,
                    _ => CellState::Null,
                };
                self.committed.push(CommittedCell {
                    number,
                    state,
                    code: cell.code.clone(),
                    tree: cell.tree,
                });
                self.committed_revision = self.committed_revision.wrapping_add(1);
                let finished = self
                    .evaluations
                    .front()
                    .is_some_and(|evaluation| index + 1 == evaluation.cells.len());
                let (outputs, value) = response_blocks(&response, &cell.code, number);
                let resolved = EditorUpdate::CellSucceeded {
                    id,
                    index,
                    number,
                    state,
                    outputs,
                    value,
                };
                match finished {
                    true => {
                        self.evaluations.pop_front();
                        Self::batch(
                            [Some(resolved), self.start_next()]
                                .into_iter()
                                .flatten()
                                .collect(),
                        )
                    }
                    false => resolved,
                }
            }
            Ok(response) => {
                self.in_debugger = response.depth > 1;
                let error = captured_error(&response.transcript, &cell.code);
                let outputs = (!error.printed.is_empty())
                    .then(|| Box::new(TextBlock::printed(error.printed)) as Box<dyn Block>)
                    .into_iter()
                    .collect();
                let failed = EditorUpdate::CellFailed {
                    id,
                    index,
                    number,
                    outputs,
                    message: error.diagnostic,
                };
                Self::batch(
                    [Some(failed), self.continue_after(id, index)]
                        .into_iter()
                        .flatten()
                        .collect(),
                )
            }
            Err(message) => {
                let failed = EditorUpdate::CellFailed {
                    id,
                    index,
                    number,
                    outputs: Vec::new(),
                    message: format!("Communication error: {message}"),
                };
                Self::batch(
                    [Some(failed), self.continue_after(id, index)]
                        .into_iter()
                        .flatten()
                        .collect(),
                )
            }
        }
    }

    fn continue_after(&mut self, id: u64, index: usize) -> Option<EditorUpdate> {
        let evaluation = self.evaluations.front()?;
        match index + 1 < evaluation.cells.len() {
            true => match self.dispatch(evaluation, index + 1) {
                true => Some(EditorUpdate::Started { id }),
                false => Some(EditorUpdate::Diagnostic(
                    "Macaulay2 worker stopped".to_string(),
                )),
            },
            false => {
                self.evaluations.pop_front();
                self.start_next()
            }
        }
    }

    fn start_next(&mut self) -> Option<EditorUpdate> {
        let evaluation = self.evaluations.front()?;
        let id = evaluation.id;
        match self.dispatch(evaluation, 0) {
            true => Some(EditorUpdate::Started { id }),
            false => {
                self.evaluations.clear();
                Some(EditorUpdate::Diagnostic(
                    "Macaulay2 worker stopped".to_string(),
                ))
            }
        }
    }

    fn resolve_restart(&mut self, result: std::result::Result<String, String>) -> EditorUpdate {
        self.restarting = false;
        match result {
            Ok(version) => {
                self.version = version;
                self.in_debugger = false;
                self.next_input_number = 1;
                self.committed.clear();
                self.committed_revision = self.committed_revision.wrapping_add(1);
                self.leading_whitespace.clear();
                self.parser.reset_active();
                EditorUpdate::Message(Box::new(TextBlock::system(
                    "Macaulay2 restarted -- the session is empty and numbering begins again",
                )))
            }
            Err(error) => EditorUpdate::Diagnostic(format!("could not restart M2: {error}")),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.requests.send(WorkerRequest::Stop);
    }
}

fn worker(mut backend: M2Backend, requests: Receiver<WorkerRequest>, events: Sender<WorkerEvent>) {
    while let Ok(request) = requests.recv() {
        match request {
            WorkerRequest::Evaluate {
                id,
                first_index,
                first_number,
                cells,
            } => {
                for (offset, code) in cells.iter().enumerate() {
                    let index = first_index + offset;
                    let number = first_number + offset as u32;
                    let result = backend
                        .evaluate(code, Some(number))
                        .map_err(|error| error.to_string());
                    // M2 numbers its own inputs, so the number a cell is labelled
                    // and recalled by is the one it reports back, not the one the
                    // shell guessed before submitting.
                    let number = result.as_ref().map_or(number, |response| response.number);
                    let stop = result.as_ref().map_or(true, |response| response.is_error);
                    if events
                        .send(WorkerEvent::Cell {
                            id,
                            index,
                            number,
                            result,
                        })
                        .is_err()
                    {
                        return;
                    }
                    if stop {
                        break;
                    }
                }
            }
            WorkerRequest::Restart => {
                let result = M2Backend::spawn().and_then(|replacement| {
                    let version = replacement.version()?;
                    backend = replacement;
                    Ok(version)
                });
                if events
                    .send(WorkerEvent::Restarted(
                        result.map_err(|error| error.to_string()),
                    ))
                    .is_err()
                {
                    return;
                }
            }
            WorkerRequest::Stop => return,
        }
    }
}

fn response_blocks(
    response: &M2Response,
    code: &str,
    number: u32,
) -> (Vec<Box<dyn Block>>, Option<OutputBlock>) {
    let value = response.rendered_value();
    let output = captured_output(
        &response.transcript,
        code,
        value.as_deref(),
        response.type_info.as_deref(),
    );
    let mut blocks: Vec<Box<dyn Block>> = output
        .warnings
        .into_iter()
        .map(|text| Box::new(WarningBlock { text }) as Box<dyn Block>)
        .collect();
    if !output.printed.is_empty() {
        blocks.push(Box::new(TextBlock::printed(output.printed)));
    }
    let value = match (response.value.clone(), output.value, response.has_value) {
        (Some(value), Some(rendered), true) => {
            Some(OutputBlock::rendered(value, rendered.type_info).numbered(number))
        }
        _ => None,
    };
    (blocks, value)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::code::highlight::SyntaxHighlighter;

    fn updates_until_idle(session: &mut Session) -> Vec<EditorUpdate> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut updates = Vec::new();
        while session.busy() && Instant::now() < deadline {
            updates.extend(session.poll());
            thread::sleep(Duration::from_millis(5));
        }
        updates.extend(session.poll());
        assert!(!session.busy(), "evaluation timed out");
        updates
    }

    #[test]
    fn null_and_muted_results_create_no_output_blocks() {
        for (value, type_info, has_value) in [
            (Some("null".to_string()), Some("Nothing".to_string()), false),
            (None, Some("ZZ".to_string()), true),
        ] {
            let response = M2Response {
                number: 1,
                is_error: false,
                transcript: "x\n\n".into(),
                value: value.map(M2Value::Net),
                type_info,
                has_value,
                depth: 1,
            };
            let (outputs, value) = response_blocks(&response, "x", 1);
            assert!(outputs.is_empty());
            assert!(value.is_none());
        }
    }

    #[test]
    fn submission_resolves_incrementally_and_preserves_numbering() {
        let backend = M2Backend::spawn().expect("spawn M2");
        let mut session = Session::new(backend);
        let mut highlighter = SyntaxHighlighter::new();

        assert!(matches!(
            session.submit(" \n\t", &mut highlighter),
            EditorUpdate::WhitespaceAccepted
        ));
        assert!(!session.busy());
        assert_eq!(session.next_input_number(), 1);

        let submitted = "  1 + 1  \n\n2 + 2  ";
        let pending = session.submit(submitted, &mut highlighter);
        let EditorUpdate::Pending { cells, .. } = &pending else {
            panic!("expected pending submission");
        };
        assert_eq!(cells[0].0, " \n\t  1 + 1");
        assert_eq!(cells[1].0, "2 + 2");
        let updates = updates_until_idle(&mut session);
        assert_eq!(
            updates
                .iter()
                .filter(|update| matches!(update, EditorUpdate::CellSucceeded { .. }))
                .count(),
            2
        );
        assert_eq!(session.next_input_number(), 3);
        assert_eq!(
            session
                .committed()
                .iter()
                .map(|cell| cell.code.as_str())
                .collect::<Vec<_>>(),
            [" \n\t  1 + 1", "2 + 2"]
        );

        assert!(matches!(
            session.submit("\n  \t", &mut highlighter),
            EditorUpdate::WhitespaceAccepted
        ));
        assert!(!session.busy());
        assert_eq!(session.next_input_number(), 3);
        assert_eq!(session.committed()[1].code, "2 + 2");
    }

    #[test]
    fn complete_prefix_runs_while_its_incomplete_tail_waits() {
        let backend = M2Backend::spawn().expect("spawn M2");
        let mut session = Session::new(backend);
        let mut highlighter = SyntaxHighlighter::new();

        let EditorUpdate::Batch(updates) = session.submit("a = 1; (1", &mut highlighter) else {
            panic!("expected a complete prefix and incomplete tail");
        };
        assert!(matches!(updates[0], EditorUpdate::Pending { .. }));
        assert!(matches!(
            &updates[1],
            EditorUpdate::Incomplete { source, .. } if source == " (1"
        ));

        assert!(matches!(
            session.submit(")", &mut highlighter),
            EditorUpdate::Queued { .. }
        ));
        let updates = updates_until_idle(&mut session);
        assert!(updates.iter().any(|update| matches!(
            update,
            EditorUpdate::Batch(updates)
                if updates.iter().any(|update| matches!(update, EditorUpdate::Started { .. }))
        )));
        assert_eq!(session.committed().len(), 2);
        assert_eq!(session.committed()[0].number, 1);
        assert_eq!(session.committed()[1].number, 2);
    }

    #[test]
    fn illegal_continuation_becomes_a_local_feed_error() {
        let backend = M2Backend::spawn().expect("spawn M2");
        let mut session = Session::new(backend);
        let mut highlighter = SyntaxHighlighter::new();

        assert!(matches!(
            session.submit("f(", &mut highlighter),
            EditorUpdate::Incomplete { .. }
        ));
        assert!(matches!(
            session.submit("]", &mut highlighter),
            EditorUpdate::SyntaxFailed { source, .. } if source == "f(\n]"
        ));
        assert_eq!(session.next_input_number(), 1);
        assert!(session.committed().is_empty());
        assert!(!session.busy());
    }

    #[test]
    fn debug_post_runs_as_value_muted_and_null_cells() {
        let backend = M2Backend::spawn().expect("spawn M2");
        let mut session = Session::new(backend);
        let mut highlighter = SyntaxHighlighter::new();

        assert!(matches!(
            session.submit(crate::ui::DEBUG_POST, &mut highlighter),
            EditorUpdate::Pending { .. }
        ));
        updates_until_idle(&mut session);
        let states = session
            .committed()
            .iter()
            .map(|cell| cell.state)
            .collect::<Vec<_>>();

        assert_eq!(
            states,
            [CellState::Value, CellState::Muted, CellState::Null]
        );
    }

    #[test]
    fn a_runtime_failure_reuses_its_number_for_the_next_cell() {
        let backend = M2Backend::spawn().expect("spawn M2");
        let mut session = Session::new(backend);
        let mut highlighter = SyntaxHighlighter::new();

        session.submit("1/0\n2 + 2", &mut highlighter);
        let updates = updates_until_idle(&mut session);
        assert!(updates.iter().any(|update| match update {
            EditorUpdate::CellFailed { index: 0, .. } => true,
            EditorUpdate::Batch(updates) => updates
                .iter()
                .any(|update| matches!(update, EditorUpdate::CellFailed { index: 0, .. })),
            _ => false,
        }));
        assert_eq!(session.next_input_number(), 2);
        assert_eq!(session.committed()[0].number, 1);

        session.submit("3 + 3", &mut highlighter);
        updates_until_idle(&mut session);
        assert_eq!(session.committed()[1].number, 2);
    }

    #[test]
    fn a_runtime_failure_continues_with_queued_cells() {
        let backend = M2Backend::spawn().expect("spawn M2");
        let mut session = Session::new(backend);
        let mut highlighter = SyntaxHighlighter::new();

        session.submit("1/0\n2 + 2", &mut highlighter);
        assert!(matches!(
            session.submit("3 + 3; (4", &mut highlighter),
            EditorUpdate::Batch(_)
        ));
        updates_until_idle(&mut session);

        assert_eq!(session.committed().len(), 2);
        assert_eq!(session.committed()[0].code, "2 + 2");
        assert_eq!(session.committed()[0].number, 1);
        assert_eq!(session.committed()[1].code, "3 + 3;");
        assert_eq!(session.committed()[1].number, 2);
        assert_eq!(session.parser.active_source(), " (4\n");
        assert!(!session.busy());
    }

    #[test]
    fn output_before_a_runtime_error_remains_an_ordinary_block() {
        let backend = M2Backend::spawn().expect("spawn M2");
        let mut session = Session::new(backend);
        let mut highlighter = SyntaxHighlighter::new();

        session.submit("(print 1; 1/0)", &mut highlighter);
        let updates = updates_until_idle(&mut session);
        let Some(EditorUpdate::CellFailed {
            outputs, message, ..
        }) = updates.into_iter().next()
        else {
            panic!("expected a failed cell");
        };

        assert_eq!(outputs.len(), 1);
        assert!(!message.lines().any(|line| line == "1"));
        assert!(message.contains("division by zero"));
    }
}
