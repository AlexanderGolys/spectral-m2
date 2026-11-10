//! Optional stdio language-server process, isolated from the terminal thread.

use std::collections::{HashMap, VecDeque};
use std::io::BufReader;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use lsp_server::{Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Exit, Initialized,
    Notification as NotificationMethod, PublishDiagnostics,
};
use lsp_types::request::{
    DocumentHighlightRequest, FoldingRangeRequest, DocumentSymbolRequest, Formatting, GotoDefinition, HoverRequest,
    Initialize, Request as RequestMethod, SemanticTokensFullRequest, SemanticTokensRefresh,
    Shutdown, SignatureHelpRequest,
};
use lsp_types::{
    DocumentHighlight, FoldingRange, DocumentSymbolResponse, GotoDefinitionResponse, Hover, HoverContents,
    InitializeResult, Location, MarkedString, NumberOrString, ParameterLabel, Position,
    PublishDiagnosticsParams, SemanticTokens, SemanticTokensLegend,
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
const DEBOUNCE: Duration = Duration::from_millis(100);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
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
                .stderr(Stdio::null())
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
}

impl Process {
    fn new(mut child: Child) -> Self {
        let stdout = child.stdout.take().expect("piped server stdout");
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
        }
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
    };
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
            .in_flight
            .values()
            .any(|(_, sent)| sent.elapsed() >= self.timeout)
        {
            return Err("language-server request timed out".to_string());
        }
        if self.initialized {
            if self.legend.is_none() && (self.pending.is_some() || !self.pages.is_empty()) {
                return Err("language server does not support full semantic tokens".to_string());
            }
            self.send_due()?;
        }
        match self.process.incoming.recv_timeout(WORKER_INTERVAL) {
            Ok(WireEvent::Message(Message::Response(response))) => self.receive(response),
            Ok(WireEvent::Message(Message::Request(request))) => self.answer(request),
            Ok(WireEvent::Message(Message::Notification(notification))) => {
                self.notified(notification)
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
            Question::Folds => (FoldingRangeRequest::METHOD, json!({ "textDocument": document })),
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
                if let Some(uri) = &apart_uri {
                    self.close(uri)?;
                }
                // A question the server cannot answer is answered with nothing,
                // never with a failure of the whole server.
                let result = response.response_result.unwrap_or(Value::Null);
                let answer = answer(&query, apart_uri.as_deref(), self.open_uri(), result);
                let _ = self.events.send(Event::Answered { query, answer });
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::RangeInclusive;
    use crate::code::highlight::Highlighter;
    use crate::code::semantic::query::{QueryId, Severity};
    use crate::code::semantic::{SemanticHighlighter, SourceId};
    use crate::ui::theme::groups::HLGroup;

    // The fixture checks wire ordering independently of the client implementation.
    const SERVER: &str = r#"
import json, sys, time
mode = sys.argv[1]
def read():
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line: return None
        if line == b'\r\n': break
        key, value = line.decode().split(':', 1)
        headers[key.lower()] = value.strip()
    return json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
def send(message):
    body = json.dumps(message).encode()
    sys.stdout.buffer.write(b'Content-Length: ' + str(len(body)).encode() + b'\r\n\r\n' + body)
    sys.stdout.buffer.flush()
def reply(request, value):
    send({'jsonrpc':'2.0', 'id':request['id'], 'result':value})
request = read()
assert request['method'] == 'initialize'
assert request['params']['capabilities']['textDocument']['semanticTokens']['augmentsSyntaxTokens']
if mode == 'initialize-timeout':
    time.sleep(5)
    sys.exit(0)
reply(request, {'serverInfo':{'name':'fixture-ls','version':'0.1'}, 'capabilities':{'semanticTokensProvider': {'full':True, 'legend': {'tokenTypes':['function'], 'tokenModifiers':[]}}}})
assert read()['method'] == 'initialized'
opened = None
documents = set()
refreshed = False
while True:
    request = read()
    if request is None: break
    method = request.get('method')
    if method == 'textDocument/didOpen':
        document = request['params']['textDocument']
        assert document['languageId'] == 'macaulay2'
        documents.add(document['uri'])
        if '-session-' in document['uri']: opened = document
        else: send({'jsonrpc':'2.0', 'method':'textDocument/publishDiagnostics', 'params':{'uri':document['uri'], 'diagnostics':[]}})
    elif method == 'textDocument/didChange':
        assert opened is not None
        assert request['params']['textDocument']['version'] > opened['version']
        opened['version'] = request['params']['textDocument']['version']
        opened['text'] = request['params']['contentChanges'][0]['text']
    elif method == 'textDocument/didClose':
        closed = request['params']['textDocument']['uri']
        documents.discard(closed)
        if opened and opened['uri'] == closed: opened = None
    elif method == 'textDocument/semanticTokens/full':
        assert request['params']['textDocument']['uri'] in documents
        if mode == 'request-timeout':
            time.sleep(5)
            sys.exit(0)
        if mode == 'malformed':
            reply(request, {'data':[0,0]})
            continue
        if mode == 'exit': sys.exit(0)
        reply(request, {'data':[0,0,1,0,0]})
        if mode == 'refresh' and not refreshed:
            refreshed = True
            send({'jsonrpc':'2.0', 'id':'refresh', 'method':'workspace/semanticTokens/refresh', 'params':None})
    elif method == 'textDocument/documentSymbol':
        reply(request, [{'name':'f', 'kind':12, 'detail':'FunctionClosure', 'range':{'start':{'line':0,'character':0},'end':{'line':0,'character':1}}, 'selectionRange':{'start':{'line':0,'character':0},'end':{'line':0,'character':1}}}])
        if mode == 'diagnostics':
            send({'jsonrpc':'2.0', 'method':'textDocument/publishDiagnostics', 'params':{'uri':opened['uri'], 'version':opened['version'], 'diagnostics':[{'range':{'start':{'line':0,'character':0},'end':{'line':0,'character':1}}, 'severity':2, 'code':'W1', 'message':'careful'}]}})
    elif method == 'textDocument/hover':
        assert request['params']['textDocument']['uri'] == opened['uri']
        reply(request, {'contents':{'kind':'markdown', 'value':'**f** at ' + str(request['params']['position']['character'])}})
    elif method == 'textDocument/definition':
        reply(request, [{'uri':opened['uri'], 'range':{'start':{'line':0,'character':0},'end':{'line':0,'character':1}}}, {'uri':'file:///library/core.m2', 'range':{'start':{'line':4,'character':2},'end':{'line':4,'character':3}}}])
    elif method == 'textDocument/formatting':
        apart = request['params']['textDocument']['uri']
        assert apart != (opened or {}).get('uri') and apart in documents
        reply(request, [{'range':{'start':{'line':0,'character':0},'end':{'line':0,'character':0}}, 'newText':'-- formatted\n'}])
    elif method == 'shutdown':
        reply(request, None)
    elif method == 'exit': break
    elif method is None:
        assert request['id'] == 'refresh' and request['result'] is None
    else: raise AssertionError(method)
"#;

    fn fixture(mode: &str, timeout: Duration) -> Client {
        let mut command = Command::new("python3");
        command.args(["-c", SERVER, mode]);
        Client::spawn(command, timeout)
    }

    fn document(generation: u64, version: i32, text: &str) -> Arc<Document> {
        let mut document = Document::new(generation);
        document.version = version;
        document.push(SourceId::Draft(generation), text);
        Arc::new(document)
    }

    fn next(client: &Client) -> Event {
        client
            .events
            .recv_timeout(Duration::from_secs(5))
            .expect("server fixture response")
    }

    /// The next event other than the server's startup announcement and the
    /// names and findings every analysis brings.
    fn event(client: &Client) -> Event {
        loop {
            match next(client) {
                Event::Started(_) | Event::Bindings { .. } | Event::Findings { .. } => {}
                event => return event,
            }
        }
    }

    #[test]
    fn initialization_announces_the_server_by_name_and_version() {
        let client = fixture("normal", REQUEST_TIMEOUT);
        match next(&client) {
            Event::Started(info) => {
                assert_eq!(info.name, "fixture-ls");
                assert_eq!(info.version.as_deref(), Some("0.1"));
            }
            _ => panic!("the server announces itself first"),
        }
    }

    #[test]
    fn stdio_lifecycle_updates_versions_and_reopens_after_restart() {
        let client = fixture("normal", REQUEST_TIMEOUT);
        for document in [
            document(0, 1, "f"),
            document(0, 2, "g"),
            document(1, 3, "h"),
        ] {
            client.update(document.clone());
            match event(&client) {
                Event::Tokens {
                    document: answered,
                    tokens,
                    ..
                } => {
                    assert_eq!(answered, document);
                    assert_eq!(tokens.len(), 1);
                }
                Event::Failed(error) => panic!("{error}"),
                _ => panic!("only the session was analysed"),
            }
        }
        let before = Instant::now();
        drop(client);
        assert!(before.elapsed() < SHUTDOWN_TIMEOUT + Duration::from_millis(500));
    }

    #[test]
    fn refresh_is_answered_and_requests_tokens_again() {
        let client = fixture("refresh", REQUEST_TIMEOUT);
        client.update(document(0, 1, "f"));
        assert!(matches!(event(&client), Event::Tokens { .. }));
        assert!(matches!(event(&client), Event::Tokens { .. }));
    }

    #[test]
    fn edits_are_coalesced_to_the_latest_document() {
        let client = fixture("normal", REQUEST_TIMEOUT);
        for version in 1..20 {
            client.update(document(0, version, "f"));
        }
        match event(&client) {
            Event::Tokens { document, .. } => assert_eq!(document.version, 19),
            Event::Failed(error) => panic!("{error}"),
            _ => panic!("only the session was analysed"),
        }
        assert!(!matches!(
            client.events.recv_timeout(DEBOUNCE),
            Ok(Event::Tokens { .. })
        ));
    }

    #[test]
    fn every_queued_page_is_analysed_on_its_own() {
        let client = fixture("normal", REQUEST_TIMEOUT);
        for page in [7, 8] {
            client.analyse_page(document(page, 1, "f"));
        }
        for page in [7, 8] {
            match event(&client) {
                Event::PageTokens {
                    document, tokens, ..
                } => {
                    assert_eq!(document.generation, page);
                    assert_eq!(tokens.len(), 1);
                }
                _ => panic!("page {page} is answered in turn"),
            }
        }
    }

    #[test]
    fn initialization_and_requests_have_bounded_timeouts() {
        for mode in ["initialize-timeout", "request-timeout"] {
            let client = fixture(mode, Duration::from_millis(200));
            client.update(document(0, 1, "f"));
            match event(&client) {
                Event::Failed(error) => assert!(error.contains("timed out"), "{error}"),
                _ => panic!("fixture must time out"),
            }
        }
    }

    #[test]
    fn malformed_tokens_and_unexpected_exit_report_one_failure() {
        for mode in ["malformed", "exit"] {
            let client = fixture(mode, REQUEST_TIMEOUT);
            client.update(document(0, 1, "f"));
            assert!(matches!(event(&client), Event::Failed(_)));
            assert!(client.events.recv_timeout(DEBOUNCE).is_err());
        }
    }

    fn answered(client: &Client) -> (Query, Answer) {
        loop {
            match event(client) {
                Event::Answered { query, answer } => return (query, answer),
                Event::Failed(error) => panic!("{error}"),
                _ => {}
            }
        }
    }

    fn query(id: u64, document: Arc<Document>, question: Question) -> Query {
        Query {
            id: QueryId(id),
            document,
            question,
        }
    }

    /// A session of one cell and the input after it.
    fn session() -> Arc<Document> {
        let mut document = Document::new(0);
        document.version = 1;
        document.push(
            SourceId::Cell {
                generation: 0,
                submission: 1,
                index: 0,
            },
            "f = x -> x",
        );
        document.push(SourceId::Draft(0), "f 1");
        Arc::new(document)
    }

    #[test]
    fn a_question_about_the_session_is_asked_where_its_point_lies() {
        let client = fixture("normal", REQUEST_TIMEOUT);
        let document = session();
        let offset = document.offset_in(SourceId::Draft(0), 2).unwrap();
        client.ask(query(1, document, Question::Hover(offset)));
        let (query, answer) = answered(&client);
        assert_eq!(query.id, QueryId(1));
        assert_eq!(answer, Answer::Hover(Some("**f** at 2".to_string())));
    }

    #[test]
    fn definitions_are_placed_in_the_session_or_in_files() {
        let client = fixture("normal", REQUEST_TIMEOUT);
        let document = session();
        let offset = document.offset_in(SourceId::Draft(0), 0).unwrap();
        client.ask(query(1, document, Question::Definition(offset)));
        let (_, answer) = answered(&client);
        let cell = SourceId::Cell {
            generation: 0,
            submission: 1,
            index: 0,
        };
        assert_eq!(
            answer,
            Answer::Definition(vec![
                Place::Source(cell, 0..1),
                Place::File {
                    path: PathBuf::from("/library/core.m2"),
                    line: 4,
                    column: 2
                },
            ])
        );
    }

    #[test]
    fn a_document_apart_is_opened_for_its_question_alone() {
        let client = fixture("normal", REQUEST_TIMEOUT);
        let mut apart = Document::new(9);
        apart.push(SourceId::Apart(QueryId(9)), "f=x->x");
        client.ask(query(9, Arc::new(apart), Question::Format));
        let (_, answer) = answered(&client);
        assert_eq!(
            answer,
            Answer::Formatted(Some("-- formatted\nf=x->x\n".to_string()))
        );
    }

    #[test]
    fn findings_and_bindings_arrive_for_the_session_as_it_stands() {
        let client = fixture("diagnostics", REQUEST_TIMEOUT);
        client.update(session());
        let (mut findings, mut bindings) = (None, None);
        while findings.is_none() || bindings.is_none() {
            match next(&client) {
                Event::Findings {
                    findings: found, ..
                } => findings = Some(found),
                Event::Bindings {
                    bindings: bound, ..
                } => bindings = Some(bound),
                Event::Failed(error) => panic!("{error}"),
                _ => {}
            }
        }
        let (source, finding) = &findings.unwrap()[0];
        assert!(matches!(source, SourceId::Cell { .. }));
        assert_eq!(finding.range, 0..1);
        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(finding.code.as_deref(), Some("W1"));
        assert_eq!(bindings.unwrap()[0].name, "f");
    }

    #[test]
    fn missing_executable_reports_a_failure_without_blocking_startup() {
        let client = Client::spawn(
            Command::new("/nonexistent/spectral-m2-language-server"),
            REQUEST_TIMEOUT,
        );
        assert!(matches!(event(&client), Event::Failed(_)));
    }

    /// Poll `highlighter` until `found` takes something from what it reports.
    fn polled_until<T>(
        highlighter: &mut SemanticHighlighter,
        mut found: impl FnMut(crate::code::semantic::Polled) -> Option<T>,
    ) -> T {
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        loop {
            let polled = highlighter.poll();
            assert!(polled.failure.is_none(), "{:?}", polled.failure);
            if let Some(value) = found(polled) {
                return value;
            }
            assert!(Instant::now() < deadline, "the real server never answered");
            thread::sleep(WORKER_INTERVAL);
        }
    }

    #[test]
    #[ignore = "requires an installed m2-ls executable"]
    fn installed_server_finds_problems_and_bindings_in_the_input() {
        let mut highlighter = SemanticHighlighter::new();
        highlighter.configure(true, true);
        let mut source = Document::new(0);
        source.push(SourceId::Draft(0), "ZZ = 7\nmyFunction = x -> x");
        highlighter.update(source);
        let (mut findings, mut bindings) = (Vec::new(), Vec::new());
        polled_until(&mut highlighter, |polled| {
            findings.extend(polled.findings.unwrap_or_default());
            bindings.extend(polled.bindings.unwrap_or_default());
            (!findings.is_empty() && !bindings.is_empty()).then_some(())
        });
        assert!(
            findings
                .iter()
                .any(|(source, finding)| *source == SourceId::Draft(0) && finding.range == (0..2)),
            "{findings:?}"
        );
        assert!(bindings.iter().any(|binding| binding.name == "myFunction"));
    }

    #[test]
    #[ignore = "requires an installed m2-ls executable"]
    fn installed_server_answers_hover_and_definition_in_the_input() {
        let mut highlighter = SemanticHighlighter::new();
        highlighter.configure(true, true);
        let cell = SourceId::Cell {
            generation: 0,
            submission: 1,
            index: 0,
        };
        let mut source = Document::new(0);
        source.push(cell, "f = x -> x");
        source.push(SourceId::Draft(0), "f ideal 1");
        highlighter.update(source);
        let draft = SourceId::Draft(0);
        let hover = highlighter.ask(draft, 3, Question::Hover).unwrap();
        let definition = highlighter.ask(draft, 0, Question::Definition).unwrap();
        let mut answers = std::collections::HashMap::new();
        while answers.len() < 2 {
            answers.extend(polled_until(&mut highlighter, |polled| {
                (!polled.answers.is_empty()).then_some(polled.answers)
            }));
        }
        match &answers[&hover] {
            Answer::Hover(Some(text)) => assert!(text.contains("ideal"), "{text}"),
            answer => panic!("{answer:?}"),
        }
        assert_eq!(
            answers[&definition],
            Answer::Definition(vec![Place::Source(cell, 0..1)])
        );
    }

    #[test]
    #[ignore = "requires an installed m2-ls executable"]
    fn installed_server_answers_occurrences_signatures_formatting_and_folds() {
        let mut highlighter = SemanticHighlighter::new();
        highlighter.configure(true, true);
        let draft = SourceId::Draft(0);
        let code = "g = x -> x + 1\nideal(g 2, 3";
        let mut source = Document::new(0);
        source.push(draft, code);
        highlighter.update(source);
        let inner = code.rfind("g 2").unwrap();
        let occurrences = highlighter.ask(draft, 0, Question::Highlights).unwrap();
        let signature = highlighter
            .ask(draft, code.len(), Question::Signature)
            .unwrap();
        let format = highlighter
            .ask_apart("f=x->x\nideal(1,2)", Question::Format)
            .unwrap();
        let folds = highlighter
            .ask_apart("f = x -> (\n    y := x;\n    y\n    )\n", Question::Folds)
            .unwrap();
        let mut answers = std::collections::HashMap::new();
        while answers.len() < 4 {
            answers.extend(polled_until(&mut highlighter, |polled| {
                (!polled.answers.is_empty()).then_some(polled.answers)
            }));
        }
        let Answer::Highlights(places) = &answers[&occurrences] else {
            panic!("{:?}", answers[&occurrences]);
        };
        let starts: Vec<usize> = places.iter().map(|(_, range)| range.start).collect();
        assert!(starts.contains(&0) && starts.contains(&inner), "{places:?}");
        let Answer::Signature(signatures) = &answers[&signature] else {
            panic!("{:?}", answers[&signature]);
        };
        let first = signatures.first().expect("ideal has signatures");
        assert!(first.label.starts_with("ideal("), "{first:?}");
        let Answer::Formatted(Some(formatted)) = &answers[&format] else {
            panic!("{:?}", answers[&format]);
        };
        assert_ne!(formatted, "f=x->x\nideal(1,2)");
        assert_eq!(answers[&folds], Answer::Folds(vec![RangeInclusive::new(0, 3)]));
    }

    #[test]
    #[ignore = "requires an installed m2-ls executable"]
    fn installed_server_classifies_a_documentation_page() {
        let mut highlighter = SemanticHighlighter::new();
        highlighter.configure(true, true);
        let (ring, generators) = ("R = QQ[x,y,z];", "matrix{{x,0,2},{1,2,y}}");
        highlighter.analyse_page(7, vec![ring.to_string(), generators.to_string()]);
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        let page = loop {
            let mut polled = highlighter.poll();
            assert!(polled.failure.is_none(), "{:?}", polled.failure);
            if let Some(page) = polled.pages.pop() {
                break page;
            }
            assert!(Instant::now() < deadline, "no real-server tokens");
            thread::sleep(WORKER_INTERVAL);
        };
        assert_eq!(page.page, 7);
        let kind_of = |source: &str, at: usize| {
            page.sources
                .iter()
                .find(|(text, _)| text == source)
                .and_then(|(_, spans)| spans.iter().find(|span| span.content.contains(&at)))
                .map(|span| span.kind)
        };
        assert_eq!(kind_of(ring, ring.find("QQ").unwrap()), Some(HLGroup::Type));
        assert_eq!(
            kind_of(generators, generators.find("{x").unwrap() + 1),
            Some(HLGroup::RingGenerator),
            "generators are known from the earlier block"
        );
    }

    #[test]
    #[ignore = "requires an installed m2-ls executable"]
    fn installed_server_classifies_session_definitions_and_ring_generators() {
        let mut highlighter = SemanticHighlighter::new();
        highlighter.configure(true, true);
        let mut source = Document::new(0);
        let cell = SourceId::Cell {
            generation: 0,
            submission: 1,
            index: 0,
        };
        source.push(cell, "R = QQ[x, y]\nf = a -> a + x");
        source.push(SourceId::Draft(0), "f(x)");
        highlighter.update(source);
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        loop {
            let polled = highlighter.poll();
            assert!(polled.failure.is_none(), "{:?}", polled.failure);
            if polled.recolored {
                break;
            }
            assert!(Instant::now() < deadline, "no real-server tokens");
            thread::sleep(WORKER_INTERVAL);
        }
        let spans = highlighter.highlight_source(cell, "R = QQ[x, y]\nf = a -> a + x");
        assert!(spans
            .iter()
            .any(|span| span.mark.unwrap_or(span.kind) == HLGroup::Type));
        assert!(spans
            .iter()
            .any(|span| span.mark.unwrap_or(span.kind) == HLGroup::RingGenerator));
        assert!(spans
            .iter()
            .any(|span| span.mark.unwrap_or(span.kind) == HLGroup::Parameter));
        let spans = highlighter.highlight_continuation(None, "f(x)");
        assert_eq!(
            spans
                .iter()
                .find(|span| span.content.contains(&0))
                .map(|span| span.mark.unwrap_or(span.kind))
                .unwrap(),
            HLGroup::Function
        );
        assert_eq!(
            spans
                .iter()
                .find(|span| span.content.contains(&2))
                .map(|span| span.mark.unwrap_or(span.kind))
                .unwrap(),
            HLGroup::RingGenerator
        );
    }
}
