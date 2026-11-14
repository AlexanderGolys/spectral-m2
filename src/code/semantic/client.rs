//! Optional stdio language-server process, isolated from the terminal thread.

use std::collections::{HashMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use lsp_server::{Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    Cancel, DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Exit, Initialized,
    Notification as NotificationMethod, PublishDiagnostics,
};
use lsp_types::request::{
    DocumentHighlightRequest, DocumentSymbolRequest, FoldingRangeRequest, Formatting,
    GotoDefinition, HoverRequest, Initialize, Request as RequestMethod, SemanticTokensFullRequest,
    SemanticTokensRefresh, Shutdown, SignatureHelpRequest,
};
use lsp_types::{
    DocumentHighlight, DocumentSymbolResponse, FoldingRange, GotoDefinitionResponse, Hover,
    HoverContents, InitializeResult, Location, MarkedString, NumberOrString, ParameterLabel,
    Position, PublishDiagnosticsParams, SemanticTokens, SemanticTokensLegend,
    SemanticTokensServerCapabilities, ServerInfo, SignatureHelp, TextEdit,
};
use serde_json::{json, Value};

use super::query::{
    offset_of, position_of, Answer, Binding, Finding, Place, Query, Question, Signature,
};
use super::{Document, SourceId, TOKEN_MODIFIERS, TOKEN_TYPES};
use crate::env::ENV;

/// The language server Spectral runs, looked up on `PATH`.
pub const SERVER_EXECUTABLE: &str = "m2-ls";
const STDERR_CAPACITY: usize = 16 * 1024;
const FAILURE_LOG: &str = "language-server.log";
const DEBOUNCE: Duration = Duration::from_millis(100);
/// How long a request may wait for its answer before it is cancelled and,
/// when it was a question, answered with nothing.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a server may say nothing at all while asked something before it
/// counts as hung: a busy server still answers or cancels within this.
const SILENCE_LIMIT: Duration = Duration::from_secs(30);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const WORKER_INTERVAL: Duration = Duration::from_millis(10);
/// The longest a question about a document apart waits for the server to have
/// read the document, which it tells by publishing its diagnostics.
const APART_READING_TIMEOUT: Duration = Duration::from_secs(1);

/// A completed computation or a terminal client failure.
pub(super) enum Event {
    /// The server finished initializing, named as it names itself or else by its executable.
    Started(ServerInfo),
    Tokens {
        document: Arc<Document>,
        legend: SemanticTokensLegend,
        tokens: Vec<lsp_types::SemanticToken>,
    },
    /// The tokens of a documentation page, analysed apart from the session.
    PageTokens {
        document: Arc<Document>,
        legend: SemanticTokensLegend,
        tokens: Vec<lsp_types::SemanticToken>,
    },
    /// The answer to a question put with [`Client::ask`].
    Answered {
        query: Query,
        answer: Answer,
    },
    /// What the server found wrong in the session's `document`, by segment.
    Findings {
        document: Arc<Document>,
        findings: Vec<(SourceId, Finding)>,
    },
    /// The names the session's `document` binds.
    Bindings {
        document: Arc<Document>,
        bindings: Vec<Binding>,
    },
    Failed(String),
}

#[derive(Default)]
struct Mailbox {
    latest: Option<(Arc<Document>, Instant)>,
    pages: VecDeque<Arc<Document>>,
    queries: VecDeque<Query>,
    stop: bool,
}

/// Nonblocking handle to one optional language-server worker.
pub(super) struct Client {
    mailbox: Arc<Mutex<Mailbox>>,
    events: Receiver<Event>,
    worker: Option<JoinHandle<()>>,
}

impl Client {
    pub(super) fn start() -> Self {
        Self::spawn(Command::new(SERVER_EXECUTABLE), REQUEST_TIMEOUT)
    }

    fn spawn(mut command: Command, timeout: Duration) -> Self {
        let mailbox = Arc::new(Mutex::new(Mailbox::default()));
        let shared = mailbox.clone();
        let (sender, events) = mpsc::channel();
        let worker = thread::spawn(move || {
            let result = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|error| format!("cannot start {SERVER_EXECUTABLE}: {error}"))
                .and_then(|child| serve(child, &shared, &sender, timeout));
            if let Err(error) = result {
                let _ = sender.send(Event::Failed(error));
            }
        });
        Self {
            mailbox,
            events,
            worker: Some(worker),
        }
    }

    #[cfg(test)]
    pub(super) fn test_channel() -> (Self, Sender<Event>) {
        let (sender, events) = mpsc::channel();
        (
            Self {
                mailbox: Arc::new(Mutex::new(Mailbox::default())),
                events,
                worker: None,
            },
            sender,
        )
    }

    pub(super) fn update(&self, document: Arc<Document>) {
        self.mailbox.lock().unwrap().latest = Some((document, Instant::now()));
    }

    /// Queue a documentation page. Unlike the session's edits, none supersedes another.
    pub(super) fn analyse_page(&self, page: Arc<Document>) {
        self.mailbox.lock().unwrap().pages.push_back(page);
    }

    /// Put a question; its answer arrives as [`Event::Answered`].
    pub(super) fn ask(&self, query: Query) {
        self.mailbox.lock().unwrap().queries.push_back(query);
    }

    pub(super) fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.mailbox.lock().unwrap().stop = true;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

enum WireEvent {
    Message(Message),
    Closed(String),
}

/// Owns the child and reaps it even when initialization or protocol handling fails.
struct Process {
    child: Child,
    outgoing: Sender<Message>,
    incoming: Receiver<WireEvent>,
    stderr: Arc<Mutex<VecDeque<u8>>>,
    stderr_done: Receiver<()>,
}

impl Process {
    fn new(mut child: Child) -> Self {
        let stdout = child.stdout.take().expect("piped server stdout");
        let mut stderr = child.stderr.take().expect("piped server stderr");
        let recent_stderr = Arc::new(Mutex::new(VecDeque::new()));
        let captured = recent_stderr.clone();
        let (stderr_finished, stderr_done) = mpsc::channel();
        thread::spawn(move || {
            let mut buffer = [0; STDERR_CAPACITY];
            while let Ok(length) = stderr.read(&mut buffer) {
                if length == 0 {
                    break;
                }
                let mut captured = captured.lock().unwrap();
                captured.extend(&buffer[..length]);
                let excess = captured.len().saturating_sub(STDERR_CAPACITY);
                captured.drain(..excess);
            }
            let _ = stderr_finished.send(());
        });
        let mut stdin = child.stdin.take().expect("piped server stdin");
        let (incoming_tx, incoming) = mpsc::channel();
        let read_events = incoming_tx.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                match Message::read(&mut reader) {
                    Ok(Some(message)) => {
                        if read_events.send(WireEvent::Message(message)).is_err() {
                            break;
                        }
                    }
                    result => {
                        let error = match result {
                            Err(error) => format!("language-server read failed: {error}"),
                            _ => "language server exited".to_string(),
                        };
                        let _ = read_events.send(WireEvent::Closed(error));
                        break;
                    }
                }
            }
        });
        let (outgoing, messages) = mpsc::channel::<Message>();
        thread::spawn(move || {
            for message in messages {
                if let Err(error) = message.write(&mut stdin) {
                    let _ = incoming_tx.send(WireEvent::Closed(format!(
                        "language-server write failed: {error}"
                    )));
                    break;
                }
            }
        });
        Self {
            child,
            outgoing,
            incoming,
            stderr: recent_stderr,
            stderr_done,
        }
    }

    fn failure(&mut self, error: String) -> String {
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        let status = loop {
            let status = self.child.try_wait().ok().flatten();
            if status.is_some() || Instant::now() >= deadline {
                break status;
            }
            thread::sleep(WORKER_INTERVAL);
        };
        if status.is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        let _ = self.stderr_done.recv_timeout(SHUTDOWN_TIMEOUT);
        let bytes: Vec<_> = self.stderr.lock().unwrap().iter().copied().collect();
        let stderr = String::from_utf8_lossy(&bytes);
        let mut report = error;
        if let Some(status) = status {
            report.push_str(&format!(" ({status})"));
        }
        let mut summary = report.clone();
        if let Some(line) = stderr.lines().rev().find(|line| !line.trim().is_empty()) {
            summary.push_str(&format!(": {line}"));
            report.push_str(&format!("\n{}", stderr.trim()));
        }
        if let Some(path) = ENV
            .state_home()
            .map(|root| root.join("spectral-m2").join(FAILURE_LOG))
        {
            let saved = path
                .parent()
                .map(fs::create_dir_all)
                .transpose()
                .and_then(|_| {
                    let mut log = OpenOptions::new().create(true).append(true).open(&path)?;
                    writeln!(log, "{report}\n")
                });
            if saved.is_ok() {
                summary.push_str(&format!("\nDetails: {}", path.display()));
            }
        }
        summary
    }

    fn send(&self, message: impl Into<Message>) -> Result<(), String> {
        self.outgoing
            .send(message.into())
            .map_err(|_| "language-server writer stopped".to_string())
    }

    fn request(&self, id: i32, method: &str, params: Value) -> Result<(), String> {
        self.send(Request {
            id: id.into(),
            method: method.to_string(),
            params,
        })
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.send(Notification::new(method.to_string(), params))
    }

    fn shutdown(&mut self, initialized: bool) {
        if !initialized {
            return;
        }
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        let shutdown_id = RequestId::from(i32::MAX);
        let _ = self.request(i32::MAX, Shutdown::METHOD, Value::Null);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            if let Ok(WireEvent::Message(Message::Response(response))) =
                self.incoming.recv_timeout(WORKER_INTERVAL)
            {
                if response.id == shutdown_id {
                    let _ = self.notify(Exit::METHOD, Value::Null);
                }
            }
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

/// The session document as the server last saw it.
struct OpenDocument {
    document: Arc<Document>,
    uri: String,
}

/// A request awaiting its response.
enum Pending {
    /// Semantic tokens of the session, or of a page open only for this request.
    Tokens {
        document: Arc<Document>,
        page_uri: Option<String>,
    },
    /// The names the session binds.
    Bindings(Arc<Document>),
    /// A question, about a document open only for it when it is apart.
    Query {
        query: Query,
        apart_uri: Option<String>,
    },
}

/// A URI for source no file holds, beside the workspace so the server resolves
/// packages as it would for the workspace's own files.
fn scratch_uri(root: &Path, kind: &str, generation: u64) -> Result<String, String> {
    let path = root.join(format!(
        ".spectral-{kind}-{}-{generation}.m2",
        std::process::id()
    ));
    url::Url::from_file_path(path)
        .map(|uri| uri.to_string())
        .map_err(|_| "cannot form source URI".to_string())
}

fn supports_full(full: &Option<lsp_types::SemanticTokensFullOptions>) -> bool {
    matches!(
        full,
        Some(
            lsp_types::SemanticTokensFullOptions::Bool(true)
                | lsp_types::SemanticTokensFullOptions::Delta { .. }
        )
    )
}

/// What the client tells the server it can take, beyond the defaults.
fn client_capabilities() -> Value {
    json!({
        "general": { "positionEncodings": ["utf-16"] },
        "workspace": { "semanticTokens": { "refreshSupport": true } },
        "textDocument": {
            "semanticTokens": {
                "requests": { "full": true },
                "tokenTypes": TOKEN_TYPES,
                "tokenModifiers": TOKEN_MODIFIERS,
                "formats": ["relative"],
                "augmentsSyntaxTokens": true,
                "overlappingTokenSupport": false,
                "multilineTokenSupport": false
            },
            "hover": { "contentFormat": ["markdown", "plaintext"] },
            "signatureHelp": { "signatureInformation": {
                "parameterInformation": { "labelOffsetSupport": true }
            }},
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": false },
            "publishDiagnostics": { "versionSupport": true },
            "foldingRange": { "lineFoldingOnly": true }
        }
    })
}

/// The language-server conversation on the worker thread.
struct Worker<'a> {
    process: Process,
    root: PathBuf,
    events: &'a Sender<Event>,
    timeout: Duration,
    initialized: bool,
    legend: Option<SemanticTokensLegend>,
    /// The newest session document and when it arrived, until it is sent.
    pending: Option<(Arc<Document>, Instant)>,
    latest: Option<Arc<Document>>,
    open: Option<OpenDocument>,
    pages: VecDeque<Arc<Document>>,
    queries: VecDeque<Query>,
    /// Questions about documents apart, by URI, waiting until the server has
    /// read their document: asked sooner, it answers as if it were empty.
    reading: HashMap<String, (Query, Instant)>,
    next_id: i32,
    in_flight: HashMap<RequestId, (Pending, Instant)>,
    /// Since when the server has said nothing while asked something.
    silent_since: Option<Instant>,
}

fn serve(
    child: Child,
    mailbox: &Mutex<Mailbox>,
    events: &Sender<Event>,
    timeout: Duration,
) -> Result<(), String> {
    let root = ENV.current_dir().map_err(|error| error.to_string())?;
    let root_uri = url::Url::from_directory_path(&root)
        .map_err(|_| "cannot form workspace URI".to_string())?;
    let mut worker = Worker {
        process: Process::new(child),
        root,
        events,
        timeout,
        initialized: false,
        legend: None,
        pending: None,
        latest: None,
        open: None,
        pages: VecDeque::new(),
        queries: VecDeque::new(),
        reading: HashMap::new(),
        next_id: 1,
        in_flight: HashMap::new(),
        silent_since: None,
    };
    let result = (|| {
        worker.process.request(
            0,
            Initialize::METHOD,
            json!({
                "processId": std::process::id(),
                "rootUri": root_uri.as_str(),
                "capabilities": client_capabilities(),
            }),
        )?;
        let started = Instant::now();
        loop {
            {
                let mut mailbox = mailbox.lock().unwrap();
                if mailbox.stop {
                    drop(mailbox);
                    worker.stop();
                    return Ok(());
                }
                if let Some(document) = mailbox.latest.take() {
                    worker.latest = Some(document.0.clone());
                    worker.pending = Some(document);
                }
                worker.pages.append(&mut mailbox.pages);
                worker.queries.append(&mut mailbox.queries);
            }
            if !worker.initialized && started.elapsed() >= timeout {
                return Err("language-server initialization timed out".to_string());
            }
            worker.step()?;
        }
    })();
    result.map_err(|error| worker.process.failure(error))
}

impl Worker<'_> {
    fn stop(&mut self) {
        if let Some(opened) = &self.open {
            let _ = self.process.notify(
                DidCloseTextDocument::METHOD,
                json!({ "textDocument": { "uri": opened.uri } }),
            );
        }
        self.process.shutdown(self.initialized);
    }

    /// Send what is due, then handle at most one message from the server.
    fn step(&mut self) -> Result<(), String> {
        if self
            .silent_since
            .is_some_and(|since| since.elapsed() >= SILENCE_LIMIT)
        {
            return Err("language server stopped answering".to_string());
        }
        let overdue: Vec<RequestId> = self
            .in_flight
            .iter()
            .filter(|(_, (_, sent))| sent.elapsed() >= self.timeout)
            .map(|(id, _)| id.clone())
            .collect();
        for id in overdue {
            self.abandon(id)?;
        }
        if self.initialized {
            if self.legend.is_none() && (self.pending.is_some() || !self.pages.is_empty()) {
                return Err("language server does not support full semantic tokens".to_string());
            }
            self.send_due()?;
        }
        match self.process.incoming.recv_timeout(WORKER_INTERVAL) {
            Ok(WireEvent::Message(message)) => {
                self.silent_since = None;
                match message {
                    Message::Response(response) => self.receive(response),
                    Message::Request(request) => self.answer(request),
                    Message::Notification(notification) => self.notified(notification),
                }
            }
            Ok(WireEvent::Closed(error)) => Err(error),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(()),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("language-server reader stopped".to_string())
            }
        }
    }

    fn tokens_in_flight(&self) -> bool {
        self.in_flight
            .values()
            .any(|(pending, _)| matches!(pending, Pending::Tokens { .. }))
    }

    fn send_due(&mut self) -> Result<(), String> {
        if !self.tokens_in_flight()
            && self
                .pending
                .as_ref()
                .is_some_and(|(_, changed)| changed.elapsed() >= DEBOUNCE)
        {
            let (document, _) = self.pending.take().unwrap();
            let uri = self.sync(&document)?;
            self.send(
                SemanticTokensFullRequest::METHOD,
                json!({ "textDocument": { "uri": uri } }),
                Pending::Tokens {
                    document: document.clone(),
                    page_uri: None,
                },
            )?;
            self.send(
                DocumentSymbolRequest::METHOD,
                json!({ "textDocument": { "uri": uri } }),
                Pending::Bindings(document),
            )?;
        }
        if !self.tokens_in_flight() {
            if let Some(document) = self.pages.pop_front() {
                let uri = scratch_uri(&self.root, "page", document.generation)?;
                self.open_apart(&uri, &document)?;
                self.send(
                    SemanticTokensFullRequest::METHOD,
                    json!({ "textDocument": { "uri": uri } }),
                    Pending::Tokens {
                        document,
                        page_uri: Some(uri),
                    },
                )?;
            }
        }
        while let Some(query) = self.queries.pop_front() {
            self.ask(query)?;
        }
        let overdue: Vec<String> = self
            .reading
            .iter()
            .filter(|(_, (_, opened))| opened.elapsed() >= APART_READING_TIMEOUT)
            .map(|(uri, _)| uri.clone())
            .collect();
        for uri in overdue {
            self.read_apart(&uri)?;
        }
        Ok(())
    }

    fn ask(&mut self, query: Query) -> Result<(), String> {
        let uri = if query.question.apart() {
            let uri = scratch_uri(&self.root, "query", query.id.0)?;
            self.open_apart(&uri, &query.document)?;
            self.reading.insert(uri, (query, Instant::now()));
            return Ok(());
        } else if self
            .open
            .as_ref()
            .is_some_and(|open| open.document.version > query.document.version)
        {
            // The session moved on since the question was asked.
            return Ok(());
        } else {
            self.sync(&query.document)?
        };
        self.request(query, uri, false)
    }

    /// Ask the question waiting on the document apart at `uri`, now read.
    fn read_apart(&mut self, uri: &str) -> Result<(), String> {
        match self.reading.remove(uri) {
            Some((query, _)) => self.request(query, uri.to_string(), true),
            None => Ok(()),
        }
    }

    /// Send `query` about the document the server knows by `uri`, closed once
    /// answered when it is `apart`.
    fn request(&mut self, query: Query, uri: String, apart: bool) -> Result<(), String> {
        let text = &query.document.text;
        let point = |offset: usize| json!(position_of(text, offset));
        let document = json!({ "uri": uri });
        let apart_uri = apart.then_some(uri);
        let (method, params) = match query.question {
            Question::Hover(offset) => (
                HoverRequest::METHOD,
                json!({ "textDocument": document, "position": point(offset) }),
            ),
            Question::Definition(offset) => (
                GotoDefinition::METHOD,
                json!({ "textDocument": document, "position": point(offset) }),
            ),
            Question::Highlights(offset) => (
                DocumentHighlightRequest::METHOD,
                json!({ "textDocument": document, "position": point(offset) }),
            ),
            Question::Signature(offset) => (
                SignatureHelpRequest::METHOD,
                json!({ "textDocument": document, "position": point(offset) }),
            ),
            Question::Folds => (
                FoldingRangeRequest::METHOD,
                json!({ "textDocument": document }),
            ),
            Question::Format => (
                Formatting::METHOD,
                json!({ "textDocument": document, "options": { "tabSize": 4, "insertSpaces": true } }),
            ),
        };
        self.send(method, params, Pending::Query { query, apart_uri })
    }

    /// Bring the server's session document to `document`; its URI.
    fn sync(&mut self, document: &Arc<Document>) -> Result<String, String> {
        match &mut self.open {
            Some(open) if open.document.generation == document.generation => {
                if open.document.version < document.version {
                    let uri = open.uri.clone();
                    self.process.notify(
                        DidChangeTextDocument::METHOD,
                        json!({
                            "textDocument": { "uri": uri, "version": document.version },
                            "contentChanges": [{ "text": document.text }]
                        }),
                    )?;
                    open.document = document.clone();
                }
                Ok(open.uri.clone())
            }
            _ => {
                if let Some(opened) = self.open.take() {
                    self.process.notify(
                        DidCloseTextDocument::METHOD,
                        json!({ "textDocument": { "uri": opened.uri } }),
                    )?;
                }
                let uri = scratch_uri(&self.root, "session", document.generation)?;
                self.open_apart(&uri, document)?;
                self.open = Some(OpenDocument {
                    document: document.clone(),
                    uri: uri.clone(),
                });
                Ok(uri)
            }
        }
    }

    fn open_apart(&self, uri: &str, document: &Document) -> Result<(), String> {
        self.process.notify(DidOpenTextDocument::METHOD, json!({ "textDocument": {
            "uri": uri, "languageId": "macaulay2", "version": document.version, "text": document.text
        }}))
    }

    fn close(&self, uri: &str) -> Result<(), String> {
        self.process.notify(
            DidCloseTextDocument::METHOD,
            json!({ "textDocument": { "uri": uri } }),
        )
    }

    fn send(&mut self, method: &str, params: Value, pending: Pending) -> Result<(), String> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .filter(|id| *id < i32::MAX)
            .ok_or("LSP request identifiers exhausted")?;
        self.process.request(id, method, params)?;
        self.in_flight.insert(id.into(), (pending, Instant::now()));
        self.silent_since.get_or_insert_with(Instant::now);
        Ok(())
    }

    /// Give up waiting for request `id`: the server is told to drop it, and a
    /// question is answered with nothing, as one it cannot answer is.
    fn abandon(&mut self, id: RequestId) -> Result<(), String> {
        let Some((pending, _)) = self.in_flight.remove(&id) else {
            return Ok(());
        };
        self.process.notify(Cancel::METHOD, json!({ "id": id }))?;
        match pending {
            Pending::Tokens {
                page_uri: Some(uri),
                ..
            } => self.close(&uri),
            Pending::Tokens { page_uri: None, .. } | Pending::Bindings(_) => Ok(()),
            Pending::Query { query, apart_uri } => self.settle(query, apart_uri, Value::Null),
        }
    }

    /// Deliver what the server answered `query`, closing its document apart.
    fn settle(
        &mut self,
        query: Query,
        apart_uri: Option<String>,
        result: Value,
    ) -> Result<(), String> {
        if let Some(uri) = &apart_uri {
            self.close(uri)?;
        }
        let answer = answer(&query, apart_uri.as_deref(), self.open_uri(), result);
        let _ = self.events.send(Event::Answered { query, answer });
        Ok(())
    }

    fn receive(&mut self, response: Response) -> Result<(), String> {
        if response.id == RequestId::from(0) {
            return self.initialize(response);
        }
        let Some((pending, _)) = self.in_flight.remove(&response.id) else {
            return Ok(());
        };
        match pending {
            Pending::Tokens { document, page_uri } => {
                if let Some(uri) = &page_uri {
                    self.close(uri)?;
                } else if self
                    .latest
                    .as_ref()
                    .is_some_and(|latest| latest.version != document.version)
                {
                    return Ok(());
                }
                let result = response
                    .response_result
                    .map_err(|error| format!("semantic-token request failed: {}", error.message))?;
                let tokens = if result.is_null() {
                    Vec::new()
                } else {
                    serde_json::from_value::<SemanticTokens>(result)
                        .map_err(|error| format!("invalid semantic tokens: {error}"))?
                        .data
                };
                let legend = self.legend.clone().unwrap();
                let _ = self.events.send(match page_uri {
                    Some(_) => Event::PageTokens {
                        document,
                        legend,
                        tokens,
                    },
                    None => Event::Tokens {
                        document,
                        legend,
                        tokens,
                    },
                });
            }
            Pending::Bindings(document) => {
                // A server that cannot list symbols leaves completion as it was.
                let symbols = response
                    .response_result
                    .ok()
                    .and_then(|result| serde_json::from_value(result).ok());
                if let Some(symbols) = symbols {
                    let bindings = bindings(symbols);
                    let _ = self.events.send(Event::Bindings { document, bindings });
                }
            }
            Pending::Query { query, apart_uri } => {
                // A question the server cannot answer is answered with nothing,
                // never with a failure of the whole server.
                let result = response.response_result.unwrap_or(Value::Null);
                self.settle(query, apart_uri, result)?;
            }
        }
        Ok(())
    }

    fn open_uri(&self) -> Option<&str> {
        self.open.as_ref().map(|open| open.uri.as_str())
    }

    fn initialize(&mut self, response: Response) -> Result<(), String> {
        let result = response
            .response_result
            .map_err(|error| format!("language-server initialization failed: {}", error.message))?;
        let result: InitializeResult =
            serde_json::from_value(result).map_err(|error| error.to_string())?;
        if result
            .capabilities
            .position_encoding
            .as_ref()
            .is_some_and(|encoding| *encoding != lsp_types::PositionEncodingKind::UTF16)
        {
            return Err("language server selected unsupported position encoding".to_string());
        }
        self.legend = match result.capabilities.semantic_tokens_provider {
            Some(SemanticTokensServerCapabilities::SemanticTokensOptions(options))
                if supports_full(&options.full) =>
            {
                Some(options.legend)
            }
            Some(SemanticTokensServerCapabilities::SemanticTokensRegistrationOptions(options))
                if supports_full(&options.semantic_tokens_options.full) =>
            {
                Some(options.semantic_tokens_options.legend)
            }
            _ => None,
        };
        self.process.notify(Initialized::METHOD, json!({}))?;
        self.initialized = true;
        let _ = self
            .events
            .send(Event::Started(result.server_info.unwrap_or_else(|| {
                ServerInfo {
                    name: SERVER_EXECUTABLE.to_string(),
                    version: None,
                }
            })));
        Ok(())
    }

    fn answer(&mut self, request: Request) -> Result<(), String> {
        let response = if request.method == SemanticTokensRefresh::METHOD {
            if self.pending.is_none() {
                self.pending = self
                    .latest
                    .clone()
                    .map(|document| (document, Instant::now() - DEBOUNCE));
            }
            Response::new_ok(request.id, Value::Null)
        } else {
            Response::new_err(
                request.id,
                lsp_server::ErrorCode::MethodNotFound as i32,
                "unsupported client request".to_string(),
            )
        };
        self.process.send(response)
    }

    /// Findings on the session as it stands; those on pages and older versions
    /// are left unseen.
    fn notified(&mut self, notification: Notification) -> Result<(), String> {
        if notification.method != PublishDiagnostics::METHOD {
            return Ok(());
        }
        let Ok(params) = serde_json::from_value::<PublishDiagnosticsParams>(notification.params)
        else {
            return Ok(());
        };
        if self.reading.contains_key(params.uri.as_str()) {
            return self.read_apart(params.uri.as_str());
        }
        let Some(open) = self
            .open
            .as_ref()
            .filter(|open| open.uri == params.uri.as_str())
        else {
            return Ok(());
        };
        if params
            .version
            .is_some_and(|version| version != open.document.version)
        {
            return Ok(());
        }
        let document = &open.document;
        let findings = params
            .diagnostics
            .into_iter()
            .filter_map(|diagnostic| {
                let start = offset_of(&document.text, diagnostic.range.start)?;
                let end = offset_of(&document.text, diagnostic.range.end)?.max(start);
                let (id, range) = document.place(start..end)?;
                Some((
                    id,
                    Finding {
                        range,
                        severity: diagnostic.severity.into(),
                        message: diagnostic.message,
                        code: diagnostic.code.map(|code| match code {
                            NumberOrString::Number(number) => number.to_string(),
                            NumberOrString::String(code) => code,
                        }),
                    },
                ))
            })
            .collect();
        let _ = self.events.send(Event::Findings {
            document: document.clone(),
            findings,
        });
        Ok(())
    }
}

fn bindings(symbols: DocumentSymbolResponse) -> Vec<Binding> {
    match symbols {
        DocumentSymbolResponse::Flat(symbols) => symbols
            .into_iter()
            .map(|symbol| Binding {
                name: symbol.name,
                detail: None,
            })
            .collect(),
        DocumentSymbolResponse::Nested(symbols) => symbols
            .into_iter()
            .map(|symbol| Binding {
                name: symbol.name,
                detail: symbol.detail,
            })
            .collect(),
    }
}

/// `result` read as the answer to `query`, its positions in `query`'s document,
/// which the server knows by `apart_uri` or, for the session, `session_uri`.
fn answer(
    query: &Query,
    apart_uri: Option<&str>,
    session_uri: Option<&str>,
    result: Value,
) -> Answer {
    let document = &query.document;
    let text = &document.text;
    let range = |range: lsp_types::Range| -> Option<Range<usize>> {
        let start = offset_of(text, range.start)?;
        Some(start..offset_of(text, range.end)?.max(start))
    };
    let here = |uri: &str| Some(uri) == apart_uri.or(session_uri);
    match query.question {
        Question::Hover(_) => Answer::Hover(
            serde_json::from_value::<Option<Hover>>(result)
                .ok()
                .flatten()
                .map(|hover| hover_text(hover.contents))
                .filter(|text| !text.trim().is_empty()),
        ),
        Question::Definition(_) => {
            let locations = match serde_json::from_value::<Option<GotoDefinitionResponse>>(result)
                .ok()
                .flatten()
            {
                Some(GotoDefinitionResponse::Scalar(location)) => vec![location],
                Some(GotoDefinitionResponse::Array(locations)) => locations,
                Some(GotoDefinitionResponse::Link(links)) => links
                    .into_iter()
                    .map(|link| Location::new(link.target_uri, link.target_selection_range))
                    .collect(),
                None => Vec::new(),
            };
            Answer::Definition(
                locations
                    .into_iter()
                    .filter_map(|location| {
                        if here(location.uri.as_str()) {
                            let (id, range) = document.place(range(location.range)?)?;
                            Some(Place::Source(id, range))
                        } else {
                            let path = url::Url::parse(location.uri.as_str())
                                .ok()?
                                .to_file_path()
                                .ok()?;
                            Some(Place::File {
                                path,
                                line: location.range.start.line,
                                column: location.range.start.character,
                            })
                        }
                    })
                    .collect(),
            )
        }
        Question::Highlights(_) => Answer::Highlights(
            serde_json::from_value::<Option<Vec<DocumentHighlight>>>(result)
                .ok()
                .flatten()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|highlight| document.place(range(highlight.range)?))
                .collect(),
        ),
        Question::Signature(_) => Answer::Signature(
            serde_json::from_value::<Option<SignatureHelp>>(result)
                .ok()
                .flatten()
                .map(signatures)
                .unwrap_or_default(),
        ),
        Question::Format => {
            let edits = serde_json::from_value::<Option<Vec<TextEdit>>>(result)
                .ok()
                .flatten()
                .unwrap_or_default();
            Answer::Formatted(edited(text, edits).filter(|formatted| formatted != text))
        }
        Question::Folds => Answer::Folds(
            serde_json::from_value::<Option<Vec<FoldingRange>>>(result)
                .ok()
                .flatten()
                .unwrap_or_default()
                .into_iter()
                .filter(|fold| fold.start_line < fold.end_line)
                .map(|fold| fold.start_line..=fold.end_line)
                .collect(),
        ),
    }
}

fn hover_text(contents: HoverContents) -> String {
    let marked = |marked: MarkedString| match marked {
        MarkedString::String(text) => text,
        MarkedString::LanguageString(code) => {
            format!("```{}\n{}\n```", code.language, code.value)
        }
    };
    match contents {
        HoverContents::Scalar(scalar) => marked(scalar),
        HoverContents::Array(parts) => parts
            .into_iter()
            .map(marked)
            .collect::<Vec<_>>()
            .join("\n\n"),
        HoverContents::Markup(markup) => markup.value,
    }
}

/// The signatures of `help`, the active one first, each with its active parameter.
fn signatures(help: SignatureHelp) -> Vec<Signature> {
    let active = help.active_signature.unwrap_or(0) as usize;
    let mut signatures: Vec<_> = help
        .signatures
        .into_iter()
        .map(|signature| {
            let parameter = signature.active_parameter.or(help.active_parameter);
            let active = parameter
                .and_then(|index| signature.parameters.as_ref()?.get(index as usize).cloned())
                .and_then(|parameter| match parameter.label {
                    ParameterLabel::Simple(label) => {
                        // Among the parameters, never in the name before them.
                        let open = signature.label.find('(').unwrap_or(0);
                        let start = open + signature.label[open..].find(&label)?;
                        Some(start..start + label.len())
                    }
                    ParameterLabel::LabelOffsets([start, end]) => {
                        let byte =
                            |units: u32| offset_of(&signature.label, Position::new(0, units));
                        Some(byte(start)?..byte(end)?)
                    }
                });
            Signature {
                label: signature.label,
                active,
            }
        })
        .collect();
    if active < signatures.len() {
        signatures[..=active].rotate_right(1);
    }
    signatures
}

/// `text` with `edits` applied, or nothing when an edit falls outside it.
fn edited(text: &str, edits: Vec<TextEdit>) -> Option<String> {
    let mut edits = edits
        .into_iter()
        .map(|edit| {
            let start = offset_of(text, edit.range.start)?;
            let end = offset_of(text, edit.range.end)?;
            (start <= end).then_some((start..end, edit.new_text))
        })
        .collect::<Option<Vec<_>>>()?;
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut result = text.to_string();
    for (range, new_text) in edits {
        result.replace_range(range, &new_text);
    }
    Some(result)
}
