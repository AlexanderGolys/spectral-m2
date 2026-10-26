//! One independently evaluated Macaulay2 workspace.

use std::collections::{HashSet, VecDeque};
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use m2_syn::{NativeCellParser, NativeCellState, PrettyNode, PrettyTree};

use super::examples::{ExamplePage, ExampleRunner};
use crate::backend::transcript::captured_error;
use crate::backend::{M2Backend, M2Response, M2Value};
use crate::code::highlight::Highlighter;
use crate::code::semantic::{Document, SourceId};
use crate::errors::Result;
use crate::ui::cell::{response_blocks, CellState, TextBlock};
use crate::ui::controller::EditorUpdate;
use crate::ui::render::markup;

/// One successfully evaluated input cell retained by its workspace.
#[derive(Clone)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct CommittedCell {
    /// Stable source identity shared by previews and the transcript.
    pub source_id: SourceId,
    pub number: u32,
    pub state: CellState,
    pub code: String,
    pub tree: PrettyNode,
    /// The submission it was entered with, which cells of one share.
    pub block: u64,
}

/// A parsed cell waiting for its worker to evaluate it.
#[derive(Clone)]
struct SubmittedCell {
    code: String,
    tree: PrettyNode,
}

/// What divides the submissions in code kept as a file.
const BLOCK_SEPARATOR: char = '\u{c}';

/// One submitted evaluation, active at the queue front and queued otherwise.
struct Evaluation {
    id: u64,
    cells: Vec<SubmittedCell>,
    cancelled: Arc<AtomicBool>,
}

/// Work sent to the thread that exclusively owns an M2 process.
enum WorkerRequest {
    Evaluate {
        id: u64,
        first_index: usize,
        first_number: u32,
        cells: Vec<String>,
        cancelled: Arc<AtomicBool>,
    },
    Restart,
    Stop,
}

/// Incremental results returned by one workspace worker.
enum WorkerEvent {
    Exited,
    Cell {
        id: u64,
        index: usize,
        number: u32,
        result: std::result::Result<M2Response, String>,
    },
    /// The fresh process's version and every name it started with.
    Restarted(std::result::Result<(String, Vec<String>), String>),
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
    generation: u64,
    source_revision: u64,
    leading_whitespace: String,
    parser: NativeCellParser,
    evaluations: VecDeque<Evaluation>,
    restarting: bool,
    /// Code to run in the fresh session once the restart it waits for succeeds.
    after_restart: Option<String>,
    /// Documentation examples computed again, apart from the session.
    examples: ExampleRunner,
    /// How long each example may take; zero computes none.
    example_timeout: Duration,
    /// The pages whose examples were asked for, so each is computed once.
    computed_pages: HashSet<u64>,
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
            generation: 0,
            source_revision: 0,
            leading_whitespace: String::new(),
            parser: NativeCellParser::new(),
            evaluations: VecDeque::new(),
            restarting: false,
            after_restart: None,
            examples: ExampleRunner::new(),
            example_timeout: Duration::ZERO,
            computed_pages: HashSet::new(),
        }
    }

    /// How long each documentation example may take to be computed again;
    /// zero computes none.
    pub fn set_example_timeout(&mut self, timeout: Duration) {
        self.example_timeout = timeout;
    }

    /// Compute the examples of the documentation `value` shows, once per page.
    fn compute_examples(&mut self, value: Option<&M2Value>) {
        let Some(M2Value::Markup {
            html,
            example_width,
            ..
        }) = value
        else {
            return;
        };
        let page = markup::page(html);
        if self.example_timeout.is_zero() || !self.computed_pages.insert(page) {
            return;
        }
        let examples = markup::examples(html, *example_width);
        if !examples.is_empty() {
            let package = markup::source_package(html);
            let page = ExamplePage {
                page,
                package,
                examples,
            };
            self.examples.run(page, self.example_timeout);
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

    /// Revision of the source assembled for static session analysis.
    pub fn source_revision(&self) -> u64 {
        self.source_revision
    }

    /// Successful cells, unresolved evaluations, and the current editable continuation.
    pub fn semantic_document(&self, pending: Option<&str>, draft: &str) -> Document {
        let mut document = Document::new(self.generation);
        let mut included = HashSet::new();
        for cell in &self.committed {
            document.push(cell.source_id, &cell.code);
            included.insert(cell.source_id);
        }
        for evaluation in &self.evaluations {
            for (index, cell) in evaluation.cells.iter().enumerate() {
                let id = SourceId::Cell {
                    generation: self.generation,
                    submission: evaluation.id,
                    index,
                };
                if !included.contains(&id) {
                    document.push(id, &cell.code);
                }
            }
        }
        if let Some(pending) = pending {
            document.push(SourceId::Pending(self.generation), pending);
        }
        document.push(SourceId::Draft(self.generation), draft);
        document
    }

    /// The code of every committed cell in evaluation order, as a file of it
    /// reads: a form feed line, which is whitespace to Macaulay2, divides the
    /// cells entered together from those entered after them.
    pub fn code(&self) -> String {
        let mut code = String::new();
        let mut previous = None;
        for cell in &self.committed {
            if previous.is_some_and(|block| block != cell.block) {
                code.push(BLOCK_SEPARATOR);
                code.push('\n');
            }
            code.push_str(&cell.code);
            code.push('\n');
            previous = Some(cell.block);
        }
        code
    }

    /// The submissions `code` was entered as: what lies between its form feeds,
    /// each to be submitted whole. Code with none, such as a hand-written file,
    /// is read as a submission per cell.
    pub fn blocks(code: &str) -> Vec<String> {
        if !code.contains(BLOCK_SEPARATOR) {
            return Self::cells(code);
        }
        code.split(BLOCK_SEPARATOR)
            .map(|block| block.trim().to_string())
            .filter(|block| !block.is_empty())
            .collect()
    }

    /// The top-level cells `code` divides into, each as it would be typed, so
    /// code read back can be submitted cell by cell as it was first entered.
    pub fn cells(code: &str) -> Vec<String> {
        let mut parser = NativeCellParser::new();
        parser.push_line(code);
        let mut cells: Vec<String> = parser
            .take_completed()
            .iter()
            .map(|cell| cell.source().trim().to_string())
            .collect();
        cells.push(parser.active_source().trim().to_string());
        cells.retain(|cell| !cell.is_empty());
        cells
    }

    /// Revision of the immutable committed-cell sequence.
    pub const fn committed_revision(&self) -> u64 {
        self.committed_revision
    }

    pub fn busy(&self) -> bool {
        !self.evaluations.is_empty() || self.restarting
    }

    /// Interrupt the active submission and discard work queued behind it.
    pub fn interrupt(&mut self) -> EditorUpdate {
        let Some(active) = self.evaluations.front() else {
            return EditorUpdate::Ignore;
        };
        active.cancelled.store(true, Ordering::Release);
        let queued = self
            .evaluations
            .drain(1..)
            .map(|evaluation| EditorUpdate::CellFailed {
                id: evaluation.id,
                index: 0,
                number: self.next_input_number,
                printed: None,
                message: "Computation cancelled before execution".to_string(),
            })
            .collect();
        Self::batch(queued)
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

    /// Restart, keeping `code` to run in the fresh session once the restart
    /// succeeds; a restart refused or failed drops it.
    pub fn restart_with(&mut self, code: String) -> EditorUpdate {
        let update = self.restart();
        if self.restarting {
            self.after_restart = Some(code);
        }
        update
    }

    /// The code kept by [`Self::restart_with`], once its restart has succeeded.
    pub fn take_after_restart(&mut self) -> Option<String> {
        match self.restarting {
            true => None,
            false => self.after_restart.take(),
        }
    }

    pub fn submit(&mut self, source: &str, highlighter: &mut dyn Highlighter) -> EditorUpdate {
        self.source_revision = self.source_revision.wrapping_add(1);
        if source.is_empty() {
            return EditorUpdate::Ignore;
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
                    _ => separated(cell.source().trim_end()),
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
            NativeCellState::Incomplete | NativeCellState::Empty
                if !self.parser.active_source().is_empty() =>
            {
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
            NativeCellState::Empty | NativeCellState::Complete | NativeCellState::Incomplete => {}
        }
        Self::batch(updates)
    }

    /// Discard parser state after the feed transfers its unfinished block back.
    pub fn reset_incomplete(&mut self) {
        self.source_revision = self.source_revision.wrapping_add(1);
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
        let evaluation = Evaluation {
            id,
            cells,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
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

    /// `updates` as one, a batch among them spliced in so batches never nest.
    fn batch(updates: Vec<EditorUpdate>) -> EditorUpdate {
        let mut updates: Vec<EditorUpdate> = updates
            .into_iter()
            .flat_map(|update| match update {
                EditorUpdate::Batch(updates) => updates,
                update => vec![update],
            })
            .collect();
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
                cancelled: Arc::clone(&evaluation.cancelled),
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

    pub fn poll(&mut self) -> Vec<EditorUpdate> {
        let mut updates = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            self.source_revision = self.source_revision.wrapping_add(1);
            match event {
                WorkerEvent::Exited => {
                    self.evaluations.clear();
                    self.restarting = false;
                    updates.push(EditorUpdate::Exit);
                }
                WorkerEvent::Cell {
                    id,
                    index,
                    number,
                    result,
                } => updates.push(self.resolve_cell(id, index, number, result)),
                WorkerEvent::Restarted(result) => updates.push(self.resolve_restart(result)),
            }
        }
        let computed = self.examples.answered();
        if !computed.is_empty() {
            for example in computed {
                markup::remember_example(example.page, example.number, example.response);
            }
            updates.push(EditorUpdate::ExamplesComputed);
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
        let mut result = result;
        let bound = match &mut result {
            Ok(response) if !response.names.is_empty() => {
                Some(EditorUpdate::BoundNames(mem::take(&mut response.names)))
            }
            _ => None,
        };
        let resolved = self.resolve_response(id, index, number, cell, result);
        match bound {
            Some(bound) => Self::batch(vec![bound, resolved]),
            None => resolved,
        }
    }

    fn resolve_response(
        &mut self,
        id: u64,
        index: usize,
        number: u32,
        cell: SubmittedCell,
        result: std::result::Result<M2Response, String>,
    ) -> EditorUpdate {
        match result {
            Ok(response) if !response.is_error => {
                self.in_debugger = response.depth > 1;
                self.next_input_number = number + 1;
                let state = CellState::of(&response);
                self.compute_examples(response.value.as_ref());
                self.committed.push(CommittedCell {
                    source_id: SourceId::Cell {
                        generation: self.generation,
                        submission: id,
                        index,
                    },
                    number,
                    state,
                    code: cell.code.clone(),
                    tree: cell.tree,
                    block: id,
                });
                self.committed_revision = self.committed_revision.wrapping_add(1);
                let finished = self
                    .evaluations
                    .front()
                    .is_some_and(|evaluation| index + 1 == evaluation.cells.len());
                let (outputs, printed, value) = response_blocks(&response, &cell.code);
                let value = value.map(|value| value.numbered(number));
                let resolved = EditorUpdate::CellSucceeded {
                    id,
                    index,
                    number,
                    state,
                    outputs,
                    printed,
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
                self.next_input_number = number + u32::from(response.has_value);
                self.in_debugger = response.depth > 1;
                let error = captured_error(&response.transcript, &cell.code);
                let printed =
                    (!error.printed.is_empty()).then(|| TextBlock::printed(error.printed));
                let failed = EditorUpdate::CellFailed {
                    id,
                    index,
                    number,
                    printed,
                    message: error.diagnostic,
                };
                Self::batch(
                    [Some(failed), self.abandon(id)]
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
                    printed: None,
                    message: if message == crate::errors::Error::Cancelled.to_string() {
                        message
                    } else {
                        format!("Communication error: {message}")
                    },
                };
                Self::batch(
                    [Some(failed), self.abandon(id)]
                        .into_iter()
                        .flatten()
                        .collect(),
                )
            }
        }
    }

    /// Drop whatever remains of a submission whose cell failed.
    ///
    /// Cells queued behind a failure are never sent. A statement that threw may have
    /// left the session part-way through what it was doing, and the cells after it
    /// were written assuming it finished -- running them anyway compounds the error
    /// instead of reporting it. The next *submission* still starts, having been typed
    /// separately.
    fn abandon(&mut self, id: u64) -> Option<EditorUpdate> {
        self.evaluations
            .front()
            .filter(|evaluation| evaluation.id == id)?;
        self.evaluations.pop_front();
        self.start_next()
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

    fn resolve_restart(
        &mut self,
        result: std::result::Result<(String, Vec<String>), String>,
    ) -> EditorUpdate {
        self.restarting = false;
        match result {
            Ok((version, names)) => {
                self.version = version;
                self.in_debugger = false;
                self.next_input_number = 1;
                self.generation = self.generation.wrapping_add(1);
                self.committed.clear();
                self.committed_revision = self.committed_revision.wrapping_add(1);
                self.leading_whitespace.clear();
                self.parser.reset_active();
                EditorUpdate::Batch(vec![
                    EditorUpdate::Message(Box::new(TextBlock::system(
                        "Macaulay2 restarted -- the session is empty and numbering begins again",
                    ))),
                    EditorUpdate::KnownNames(names),
                ])
            }
            Err(error) => {
                self.after_restart = None;
                EditorUpdate::Diagnostic(format!("could not restart M2: {error}"))
            }
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
                cancelled,
            } => {
                for (offset, code) in cells.iter().enumerate() {
                    let index = first_index + offset;
                    let number = first_number + offset as u32;
                    let result =
                        backend
                            .evaluate_cancellable(code, &cancelled)
                            .map(|mut response| {
                                if cancelled.load(Ordering::Acquire) {
                                    response.is_error = true;
                                    response.transcript.push_str("\nComputation cancelled\n");
                                }
                                response
                            });
                    if matches!(result, Err(crate::errors::Error::Exited(_))) {
                        let _ = events.send(WorkerEvent::Exited);
                        return;
                    }
                    let result = result.map_err(|error| error.to_string());
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
                let result = M2Backend::spawn().and_then(|mut replacement| {
                    let started = (replacement.version()?, replacement.take_names());
                    backend = replacement;
                    Ok(started)
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

/// A later cell's code without what parted it from the cell before: the spaces
/// left on that cell's line and the break ending it. A cell sharing a line is not
/// indented by them, while one on a line of its own keeps its indentation.
fn separated(source: &str) -> &str {
    let rest = source.trim_start_matches([' ', '\t']);
    rest.strip_prefix('\n').unwrap_or(rest)
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
    fn process_exit_closes_the_shell_without_running_later_blocks() {
        for code in [
            "quit",
            "exit 0",
            "exit 7",
            "end",
            "(() -> quit)()",
            "(() -> exit 3)()",
            "(() -> end)()",
        ] {
            let mut session = Session::new(M2Backend::spawn().expect("spawn M2"));
            let mut highlighter = SyntaxHighlighter::new();
            session.submit(&format!("{code}\nerror \"must not run\""), &mut highlighter);
            session.submit("error \"queued code must not run\"", &mut highlighter);
            let updates = updates_until_idle(&mut session);
            assert!(
                updates
                    .iter()
                    .any(|update| matches!(update, EditorUpdate::Exit)),
                "{code} did not close the shell"
            );
            assert!(session.committed().is_empty());
        }
    }

    #[test]
    fn returning_command_names_does_not_close_the_shell() {
        let mut session = Session::new(M2Backend::spawn().expect("spawn M2"));
        let mut highlighter = SyntaxHighlighter::new();
        session.submit(
            "(() -> {\"quit\", \"exit\", \"restart\", \"end\"})()",
            &mut highlighter,
        );
        let updates = updates_until_idle(&mut session);
        assert!(!updates
            .iter()
            .any(|update| matches!(update, EditorUpdate::Exit)));
        assert_eq!(session.committed().len(), 1);
    }

    #[test]
    fn interrupt_stops_active_and_queued_work_and_preserves_the_session() {
        let mut session = Session::new(M2Backend::spawn().expect("spawn M2"));
        let mut highlighter = SyntaxHighlighter::new();
        session.submit("savedBeforeInterrupt = 41", &mut highlighter);
        updates_until_idle(&mut session);
        session.submit(
            "while true do null\nsavedBeforeInterrupt = 0",
            &mut highlighter,
        );
        let queued = session.submit("savedBeforeInterrupt = 99", &mut highlighter);
        let EditorUpdate::Queued { id: queued_id, .. } = queued else {
            panic!("expected queued submission");
        };
        thread::sleep(Duration::from_millis(100));
        let cancelled = session.interrupt();
        assert!(matches!(cancelled, EditorUpdate::CellFailed { id, .. } if id == queued_id));
        // Repeated presses while the interruption is in flight remain harmless.
        session.interrupt();
        let updates = updates_until_idle(&mut session);
        assert!(updates.iter().any(|update| matches!(update,
            EditorUpdate::CellFailed { message, .. } if message.contains("cancelled"))));
        assert_eq!(session.committed().len(), 1);
        assert_eq!(session.next_input_number(), 2);
        session.submit("assert(savedBeforeInterrupt == 41);", &mut highlighter);
        updates_until_idle(&mut session);
        assert_eq!(session.committed().len(), 2);
        assert_eq!(session.next_input_number(), 3);
    }

    #[test]
    fn immediate_interrupt_does_not_run_the_submission() {
        let mut session = Session::new(M2Backend::spawn().expect("spawn M2"));
        let mut highlighter = SyntaxHighlighter::new();
        session.submit("while true do null", &mut highlighter);
        session.interrupt();
        updates_until_idle(&mut session);
        session.submit("1+1", &mut highlighter);
        updates_until_idle(&mut session);
        assert_eq!(session.committed().len(), 1);
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
                type_parts: Vec::new(),
                has_value,
                depth: 1,
                names: Vec::new(),
            };
            let (outputs, printed, value) = response_blocks(&response, "x");
            assert!(outputs.is_empty());
            assert!(printed.is_none());
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
    fn code_kept_across_a_restart_comes_back_only_once_the_session_is_fresh() {
        let mut highlighter = SyntaxHighlighter::new();
        let mut session = Session::new(M2Backend::spawn().expect("spawn M2"));
        session.submit("a = 1", &mut highlighter);
        updates_until_idle(&mut session);

        session.restart_with("b = 2".to_string());
        assert!(session.take_after_restart().is_none());
        updates_until_idle(&mut session);

        assert_eq!(session.take_after_restart().as_deref(), Some("b = 2"));
        assert!(session.committed().is_empty());
        assert!(session.take_after_restart().is_none());
    }

    #[test]
    fn saved_code_runs_back_as_the_same_cells() {
        let mut highlighter = SyntaxHighlighter::new();
        let cells = |session: &Session| -> Vec<String> {
            session
                .committed()
                .iter()
                .map(|cell| cell.code.clone())
                .collect()
        };
        let mut first = Session::new(M2Backend::spawn().expect("spawn M2"));
        first.submit("a = 1\nb = a + 1", &mut highlighter);
        updates_until_idle(&mut first);

        let mut second = Session::new(M2Backend::spawn().expect("spawn M2"));
        second.submit(&first.code(), &mut highlighter);
        updates_until_idle(&mut second);

        assert_eq!(cells(&first).len(), 2);
        assert_eq!(cells(&second), cells(&first));
    }

    #[test]
    fn code_read_back_divides_into_the_submissions_it_was_typed_as() {
        let separator = format!("\n{BLOCK_SEPARATOR}\n");
        let code = format!("a = 1;\nb = 2;{separator}c = 3\n{separator}d");

        assert_eq!(Session::blocks(&code), ["a = 1;\nb = 2;", "c = 3", "d"]);
        assert_eq!(Session::blocks("a = 1\nb = 2\n"), ["a = 1", "b = 2"]);
        for comments in ["-- comment", "-* comment 2\n   comment 3\n*-"] {
            assert_eq!(Session::blocks(comments), [comments]);
        }
    }

    #[test]
    fn a_submissions_cells_stay_together_in_the_code_kept() {
        let mut highlighter = SyntaxHighlighter::new();
        let mut session = Session::new(M2Backend::spawn().expect("spawn M2"));
        session.submit("a = 1;\nb = 2;", &mut highlighter);
        updates_until_idle(&mut session);
        session.submit("c = 3", &mut highlighter);
        updates_until_idle(&mut session);

        assert_eq!(
            Session::blocks(&session.code()),
            ["a = 1;\nb = 2;", "c = 3"]
        );
    }

    #[test]
    fn a_cell_sharing_a_line_is_not_indented_but_one_on_its_own_keeps_its_indent() {
        assert_eq!(separated(" b = 2"), "b = 2");
        assert_eq!(separated("  \nb = 2"), "b = 2");
        assert_eq!(separated("\n    b = 2"), "    b = 2");
    }

    #[test]
    fn code_read_back_divides_into_the_cells_it_was_typed_as() {
        let code = "R = QQ[x]\nf = n -> (\n    n + 1\n)\n-- a comment\nf 2\n";

        assert_eq!(
            Session::cells(code),
            ["R = QQ[x]", "f = n -> (\n    n + 1\n)", "-- a comment\nf 2"]
        );
    }

    #[test]
    fn comment_only_submissions_wait_for_code() {
        for comments in [
            vec!["-- comment"],
            vec!["-* comment 2\n   comment 3\n*-"],
            vec!["-- comment", "-* comment 2", "   comment 3", "*-"],
        ] {
            let mut session = Session::new(M2Backend::spawn().expect("spawn M2"));
            let mut highlighter = SyntaxHighlighter::new();
            let mut source = String::new();
            for comment in comments {
                source.push_str(comment);
                assert!(matches!(
                    session.submit(comment, &mut highlighter),
                    EditorUpdate::Incomplete { source: pending, .. }
                        if pending == source.trim_end()
                ));
                source.push('\n');
                assert!(!session.busy());
                assert_eq!(session.next_input_number(), 1);
            }

            source.push_str("1 + 1");
            assert!(matches!(
                session.submit("1 + 1", &mut highlighter),
                EditorUpdate::Pending { cells, .. } if cells[0].0 == source
            ));
            updates_until_idle(&mut session);
            assert_eq!(session.committed().len(), 1);
            assert_eq!(session.committed()[0].code, source);
            assert_eq!(session.next_input_number(), 2);
        }
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
    fn a_runtime_failure_leaves_its_number_to_the_next_submission() {
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
        assert!(
            session.committed().is_empty(),
            "nothing behind the failure should have committed"
        );
        assert_eq!(session.next_input_number(), 1);

        session.submit("3 + 3", &mut highlighter);
        updates_until_idle(&mut session);
        assert_eq!(session.committed()[0].number, 1);
    }

    #[test]
    fn a_runtime_failure_abandons_the_cells_queued_behind_it() {
        let backend = M2Backend::spawn().expect("spawn M2");
        let mut session = Session::new(backend);
        let mut highlighter = SyntaxHighlighter::new();

        // `2 + 2` is queued behind a statement that throws, and must never be sent:
        // it was written assuming the one before it finished.
        session.submit("1/0\n2 + 2", &mut highlighter);
        assert!(matches!(
            session.submit("3 + 3; (4", &mut highlighter),
            EditorUpdate::Batch(_)
        ));
        updates_until_idle(&mut session);

        assert_eq!(
            session
                .committed()
                .iter()
                .map(|cell| cell.code.as_str())
                .collect::<Vec<_>>(),
            ["3 + 3;"],
            "only the separately typed submission should have run"
        );
        // The failure bound no number, so the next submission takes the one it left.
        assert_eq!(session.committed()[0].number, 1);
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
            printed, message, ..
        }) = updates.into_iter().next()
        else {
            panic!("expected a failed cell");
        };

        assert!(printed.is_some());
        assert!(!message.lines().any(|line| line == "1"));
        assert!(message.contains("division by zero"));
    }
    #[test]
    fn analysis_source_tracks_queued_cells_without_duplicating_committed_cells() {
        let mut session = Session::new(M2Backend::spawn().expect("M2 starts"));
        let mut highlighter = SyntaxHighlighter::new();
        session.submit(
            "sleep 1;
a = 1",
            &mut highlighter,
        );
        session.submit("b = a + 1", &mut highlighter);
        let queued = session.semantic_document(Some("f("), "b)");
        assert_eq!(queued.segments.len(), 5);
        assert_eq!(queued.text.matches("a = 1").count(), 1);
        assert_eq!(queued.text.matches("b = a + 1").count(), 1);
        updates_until_idle(&mut session);
        let committed = session.semantic_document(Some("f("), "b)");
        assert_eq!(committed.segments, queued.segments);
        assert_eq!(committed.text, queued.text);
        session.restart();
        updates_until_idle(&mut session);
        let restarted = session.semantic_document(None, "b");
        assert_eq!(restarted.generation, queued.generation + 1);
        assert_eq!(restarted.text, "b\n");
    }

    #[test]
    fn failed_and_abandoned_source_is_removed_from_analysis() {
        let mut session = Session::new(M2Backend::spawn().expect("M2 starts"));
        let mut highlighter = SyntaxHighlighter::new();
        session.submit("a = 1;\n1/0\nb = 2", &mut highlighter);
        updates_until_idle(&mut session);
        let document = session.semantic_document(None, "a");
        assert_eq!(document.segments.len(), 2);
        assert!(document.text.contains("a = 1"));
        assert!(!document.text.contains("1/0"));
        assert!(!document.text.contains("b = 2"));
    }
}
