//! One independently evaluated Macaulay2 workspace.

use std::collections::{HashSet, VecDeque};
use std::mem;
use std::result::Result;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;

use m2_syn::{NativeCellParser, NativeCellState, PrettyNode, PrettyTree};

use super::examples::{ExamplePage, ExampleRunner};
use crate::backend::{Announcement, M2Backend, M2Response, Rod, Versions};
use crate::code::highlight::Highlighter;
use crate::code::semantic::{Document, SourceId};
use crate::errors::Error;
use crate::ui::cell::{error_blocks, response_blocks, CellState, TextBlock};
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
// @##magic
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
        result: Box<Result<M2Response, String>>,
    },
    /// What the fresh process announced.
    Restarted(Result<Announcement, String>),
}

/// A live M2 worker and the source state committed through it.
pub struct Session {
    requests: Sender<WorkerRequest>,
    events: Receiver<WorkerEvent>,
    /// The id of the M2 process the worker runs, replaced when it restarts.
    m2_process: Arc<AtomicU32>,
    restart_requested: Arc<AtomicBool>,
    versions: Versions,
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
    /// A restart has completed since [`Self::take_restarted`] last asked.
    restarted: bool,
    /// Documentation examples computed again, apart from the session.
    examples: ExampleRunner,
    /// How long each example may take; zero computes none.
    example_timeout: Duration,
    /// The pages whose examples were asked for, so each is computed once.
    computed_pages: HashSet<u64>,
    /// The pages whose code was handed over for analysis, so each is analysed once.
    analysed_pages: HashSet<u64>,
    pages_to_analyse: Vec<(u64, Vec<String>)>,
}

impl Session {
    pub fn new(backend: M2Backend) -> Self {
        let versions = backend.versions().clone();
        let (request_tx, request_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let m2_process = Arc::new(AtomicU32::new(backend.process_id()));
        let worker_process = m2_process.clone();
        let restart_requested = Arc::new(AtomicBool::new(false));
        let worker_restart = Arc::clone(&restart_requested);
        thread::spawn(move || {
            worker(
                backend,
                request_rx,
                event_tx,
                worker_process,
                worker_restart,
            )
        });
        Self {
            requests: request_tx,
            events: event_rx,
            m2_process,
            restart_requested,
            versions,
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
            restarted: false,
            examples: ExampleRunner::new(),
            example_timeout: Duration::ZERO,
            computed_pages: HashSet::new(),
            analysed_pages: HashSet::new(),
            pages_to_analyse: Vec::new(),
        }
    }

    /// The code of every documentation page first shown since last asked, by page.
    pub fn take_pages_to_analyse(&mut self) -> Vec<(u64, Vec<String>)> {
        mem::take(&mut self.pages_to_analyse)
    }

    /// Prepare a page for the same example rendering and semantic analysis as session output.
    pub fn prepare_documentation(&mut self, value: Option<&Rod>) {
        self.compute_examples(value);
        self.analyse_page(value);
    }

    fn analyse_page(&mut self, value: Option<&Rod>) {
        let Some((html, example_width)) = value.and_then(Rod::markup) else {
            return;
        };
        let page = markup::page(html);
        if self.analysed_pages.insert(page) {
            let sources = markup::code_sources(html, *example_width);
            if !sources.is_empty() {
                self.pages_to_analyse.push((page, sources));
            }
        }
    }

    /// How long each documentation example may take to be computed again;
    /// zero computes none.
    pub fn set_example_timeout(&mut self, timeout: Duration) {
        self.example_timeout = timeout;
    }

    /// Compute the examples of the documentation `value` shows, once per page.
    // @##issue should take either a code block or a sequence of blocks
    fn compute_examples(&mut self, value: Option<&Rod>) {
        let Some((html, example_width)) = value.and_then(Rod::markup) else {
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

    pub fn versions(&self) -> &Versions {
        &self.versions
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

    /// The id of the M2 process behind this session.
    pub fn m2_process(&self) -> u32 {
        self.m2_process.load(Ordering::Relaxed)
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
                outputs: Vec::new(),
                printed: None,
                message: "Computation cancelled before execution".to_string(),
            })
            .collect();
        Self::batch(queued)
    }

    pub fn restart(&mut self) -> EditorUpdate {
        if self.restarting {
            return EditorUpdate::Ignore;
        }
        self.restart_requested.store(true, Ordering::Release);
        let pending = self.stop_for_restart();
        // Wake the worker even when the interpreter no longer handles interrupts.
        let _ = killpg(Pid::from_raw(self.m2_process() as i32), Signal::SIGKILL);
        if self.requests.send(WorkerRequest::Restart).is_err() {
            // @##magic hardcoded diagnostic 3 separate times
            return EditorUpdate::Diagnostic("Macaulay2 worker stopped".to_string());
        }
        self.restarting = true;
        Self::batch(pending)
    }

    /// Cancel every evaluation still pending, as a restart ends them.
    fn stop_for_restart(&mut self) -> Vec<EditorUpdate> {
        let number = self.next_input_number;
        self.evaluations
            .drain(..)
            .map(|evaluation| {
                evaluation.cancelled.store(true, Ordering::Release);
                EditorUpdate::CellFailed {
                    id: evaluation.id,
                    index: 0,
                    number,
                    outputs: Vec::new(),
                    printed: None,
                    message: "Computation stopped for restart".to_string(),
                }
            })
            .collect()
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

    pub fn take_restarted(&mut self) -> bool {
        mem::take(&mut self.restarted)
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

    // @##wtf
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
                WorkerEvent::Exited if self.restarting => {}
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
                } => updates.push(self.resolve_cell(id, index, number, *result)),
                WorkerEvent::Restarted(result) => updates.push(self.resolve_restart(result)),
            }
        }
        let computed = self.examples.answered();
        if !computed.is_empty() {
            for example in computed {
                self.analyse_page(example.response.value.as_ref());
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
        result: Result<M2Response, String>,
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
        result: Result<M2Response, String>,
    ) -> EditorUpdate {
        match result {
            Ok(response) if !response.is_error => {
                self.in_debugger = response.depth > 1;
                self.next_input_number = number + 1;
                let state = CellState::of(&response);
                self.prepare_documentation(response.value.as_ref());
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
                let (outputs, printed, message) = error_blocks(&response, &cell.code);
                let failed = EditorUpdate::CellFailed {
                    id,
                    index,
                    number,
                    outputs,
                    printed,
                    message,
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
                    outputs: Vec::new(),
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
        result: Result<Announcement, String>,
    ) -> EditorUpdate {
        self.restarting = false;
        match result {
            Ok(Announcement {
                versions,
                names,
                startup_output,
            }) => {
                // Empty unless M2 restarted itself, from a cell still pending.
                let mut updates = self.stop_for_restart();
                self.restarted = true;
                self.versions = versions;
                self.in_debugger = false;
                self.next_input_number = 1;
                self.generation = self.generation.wrapping_add(1);
                self.committed.clear();
                self.committed_revision = self.committed_revision.wrapping_add(1);
                self.leading_whitespace.clear();
                self.parser.reset_active();
                updates.extend([
                    EditorUpdate::Message(Box::new(TextBlock::system(
                        "Macaulay2 restarted -- the session is empty and numbering begins again",
                    ))),
                    EditorUpdate::KnownNames(names),
                ]);
                if !startup_output.is_empty() {
                    updates.push(EditorUpdate::Message(Box::new(TextBlock::printed(
                        startup_output,
                    ))));
                }
                EditorUpdate::Batch(updates)
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

fn worker(
    mut backend: M2Backend,
    requests: Receiver<WorkerRequest>,
    events: Sender<WorkerEvent>,
    process: Arc<AtomicU32>,
    restart_requested: Arc<AtomicBool>,
) {
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
                    if restart_requested.load(Ordering::Acquire) {
                        break;
                    }
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
                    if restart_requested.load(Ordering::Acquire) {
                        break;
                    }
                    match result {
                        Err(Error::Exited(_)) => {
                            let _ = events.send(WorkerEvent::Exited);
                            return;
                        }
                        Err(Error::Restarted) => {
                            let announced = Ok(backend.take_announcement());
                            if events.send(WorkerEvent::Restarted(announced)).is_err() {
                                return;
                            }
                            break;
                        }
                        _ => {}
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
                            result: Box::new(result),
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
                let result = backend.restarted().map(|replacement| {
                    backend = replacement;
                    process.store(backend.process_id(), Ordering::Relaxed);
                    backend.take_announcement()
                });
                restart_requested.store(false, Ordering::Release);
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
