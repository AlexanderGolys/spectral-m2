//! One independently evaluated Macaulay2 workspace.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use m2_syn::PrettyNode;

use crate::backend::transcript::{captured_error, captured_output};
use crate::backend::{M2Backend, M2Response};
use crate::code::highlight::Highlighter;
use crate::code::parser::{self, ParseVerdict};
use crate::errors::Result;
use crate::ui::block::{Block, CellState, OutputBlock, TextBlock, WarningBlock};
use crate::ui::controller::commands::{submitted_command, SubmittedCommand};
use crate::ui::controller::EditorUpdate;

/// One successfully evaluated input cell retained by its workspace.
#[derive(Clone)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct CommittedCell {
    pub number: u32,
    pub state: CellState,
    pub code: String,
    pub tree: PrettyNode,
}

/// A parsed cell waiting for its worker to evaluate it.
#[derive(Clone)]
struct SubmittedCell {
    code: String,
    tree: PrettyNode,
}

/// The ordered cells belonging to the workspace's active submission.
struct PendingSubmission {
    id: u64,
    cells: Vec<SubmittedCell>,
}

/// Work sent to the thread that exclusively owns an M2 process.
enum WorkerRequest {
    Evaluate {
        id: u64,
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
pub struct Session {
    requests: Sender<WorkerRequest>,
    events: Receiver<WorkerEvent>,
    version: String,
    in_debugger: bool,
    next_input_number: u32,
    next_submission_id: u64,
    committed: Vec<CommittedCell>,
    committed_revision: u64,
    pending: Option<PendingSubmission>,
    restarting: bool,
}

impl Session {
    pub fn new(backend: M2Backend) -> Self {
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
            pending: None,
            restarting: false,
        }
    }

    pub fn version(&self) -> Result<String> {
        Ok(self.version.clone())
    }

    #[cfg(test)]
    pub fn next_input_number(&self) -> u32 {
        self.next_input_number
    }

    pub fn in_debugger(&self) -> bool {
        self.in_debugger
    }

    pub fn committed(&self) -> &[CommittedCell] {
        &self.committed
    }

    /// Revision of the immutable committed-cell sequence.
    pub const fn committed_revision(&self) -> u64 {
        self.committed_revision
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.restarting
    }

    pub fn restart(&mut self) -> EditorUpdate {
        if self.busy() {
            return EditorUpdate::Diagnostic("workspace is still evaluating".to_string());
        }
        if self.requests.send(WorkerRequest::Restart).is_err() {
            return EditorUpdate::Diagnostic("Macaulay2 worker stopped".to_string());
        }
        self.restarting = true;
        EditorUpdate::Ignore
    }

    pub fn submit(&mut self, source: &str, highlighter: &mut dyn Highlighter) -> EditorUpdate {
        if source.is_empty() {
            return EditorUpdate::Ignore;
        }
        if let Some(command) = submitted_command(source) {
            return match command {
                SubmittedCommand::Quit => EditorUpdate::Quit,
                SubmittedCommand::Restart => self.restart(),
            };
        }
        if self.busy() {
            return EditorUpdate::Diagnostic("workspace is still evaluating".to_string());
        }

        let source = source.trim();
        let parsed = match parser::check(source) {
            ParseVerdict::Incomplete(reason) => {
                return EditorUpdate::ContinueSubmission(reason);
            }
            ParseVerdict::SyntaxError(diagnostic) => {
                return EditorUpdate::SyntaxDiagnostic {
                    message: diagnostic.message,
                    range: diagnostic.range,
                };
            }
            ParseVerdict::Complete(cells) if cells.is_empty() => return EditorUpdate::Ignore,
            ParseVerdict::Complete(cells) => cells,
        };
        let cells: Vec<_> = parsed
            .into_iter()
            .map(|cell| SubmittedCell {
                code: source[cell.range].to_string(),
                tree: cell.tree,
            })
            .collect();
        let displayed = cells
            .iter()
            .map(|cell| (cell.code.clone(), highlighter.highlight(&cell.code)))
            .collect();
        let id = self.next_submission_id;
        self.next_submission_id += 1;
        let request = WorkerRequest::Evaluate {
            id,
            first_number: self.next_input_number,
            cells: cells.iter().map(|cell| cell.code.clone()).collect(),
        };
        if self.requests.send(request).is_err() {
            return EditorUpdate::Diagnostic("Macaulay2 worker stopped".to_string());
        }
        self.pending = Some(PendingSubmission { id, cells });
        EditorUpdate::Pending {
            id,
            cells: displayed,
        }
    }

    pub fn poll(&mut self) -> Vec<EditorUpdate> {
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
            .pending
            .as_ref()
            .filter(|pending| pending.id == id)
            .and_then(|pending| pending.cells.get(index))
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
                    .pending
                    .as_ref()
                    .is_some_and(|pending| index + 1 == pending.cells.len());
                if finished {
                    self.pending = None;
                }
                EditorUpdate::CellSucceeded {
                    id,
                    index,
                    number,
                    state,
                    outputs: response_blocks(&response, &cell.code, number),
                }
            }
            Ok(response) => {
                self.in_debugger = response.depth > 1;
                self.pending = None;
                EditorUpdate::CellFailed {
                    id,
                    index,
                    number,
                    message: captured_error(&response.transcript, &cell.code),
                }
            }
            Err(message) => {
                self.pending = None;
                EditorUpdate::CellFailed {
                    id,
                    index,
                    number,
                    message: format!("Communication error: {message}"),
                }
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
                first_number,
                cells,
            } => {
                for (index, code) in cells.iter().enumerate() {
                    let number = first_number + index as u32;
                    let result = backend
                        .evaluate(code, Some(number))
                        .map_err(|error| error.to_string());
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

fn response_blocks(response: &M2Response, code: &str, number: u32) -> Vec<Box<dyn Block>> {
    let output = captured_output(
        &response.transcript,
        code,
        response.value.as_deref(),
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
    if let (Some(value), true) = (output.value, response.has_value) {
        blocks.push(Box::new(
            OutputBlock::value(value.text, value.type_info).numbered(number),
        ));
    }
    blocks
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
                is_error: false,
                transcript: "x\n\n".into(),
                value,
                type_info,
                has_value,
                depth: 1,
            };
            assert!(response_blocks(&response, "x", 1).is_empty());
        }
    }

    #[test]
    fn submission_resolves_incrementally_and_preserves_numbering() {
        let backend = M2Backend::spawn().expect("spawn M2");
        let mut session = Session::new(backend);
        let mut highlighter = SyntaxHighlighter::new();

        assert!(matches!(
            session.submit("1 + 1\n2 + 2", &mut highlighter),
            EditorUpdate::Pending { .. }
        ));
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
            ["1 + 1", "2 + 2"]
        );
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
    fn a_runtime_failure_reuses_its_number() {
        let backend = M2Backend::spawn().expect("spawn M2");
        let mut session = Session::new(backend);
        let mut highlighter = SyntaxHighlighter::new();

        session.submit("1/0\n2 + 2", &mut highlighter);
        let updates = updates_until_idle(&mut session);
        assert!(updates
            .iter()
            .any(|update| matches!(update, EditorUpdate::CellFailed { index: 0, .. })));
        assert_eq!(session.next_input_number(), 1);
        assert!(session.committed().is_empty());

        session.submit("2 + 2", &mut highlighter);
        updates_until_idle(&mut session);
        assert_eq!(session.committed()[0].number, 1);
    }
}
