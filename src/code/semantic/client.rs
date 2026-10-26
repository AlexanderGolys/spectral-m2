//! Optional stdio language-server process, isolated from the terminal thread.

use std::io::BufReader;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use lsp_server::{Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Exit, Initialized,
    Notification as NotificationMethod,
};
use lsp_types::request::{
    Initialize, Request as RequestMethod, SemanticTokensFullRequest, SemanticTokensRefresh,
    Shutdown,
};
use lsp_types::{
    InitializeResult, SemanticTokens, SemanticTokensLegend, SemanticTokensServerCapabilities,
    ServerInfo,
};
use serde_json::{json, Value};

use super::{Document, TOKEN_MODIFIERS, TOKEN_TYPES};

/// The language server Spectral runs, looked up on `PATH`.
pub const SERVER_EXECUTABLE: &str = "m2-ls";
const DEBOUNCE: Duration = Duration::from_millis(100);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const WORKER_INTERVAL: Duration = Duration::from_millis(10);

/// A completed computation or a terminal client failure.
pub(super) enum Event {
    /// The server finished initializing, named as it names itself or else by its executable.
    Started(ServerInfo),
    Tokens {
        document: Arc<Document>,
        legend: SemanticTokensLegend,
        tokens: Vec<lsp_types::SemanticToken>,
    },
    Failed(String),
}

#[derive(Default)]
struct Mailbox {
    latest: Option<(Arc<Document>, Instant)>,
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

struct OpenDocument {
    generation: u64,
    version: i32,
    uri: String,
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

fn serve(
    child: Child,
    mailbox: &Mutex<Mailbox>,
    events: &Sender<Event>,
    timeout: Duration,
) -> Result<(), String> {
    let mut process = Process::new(child);
    let root = std::env::current_dir().map_err(|error| error.to_string())?;
    let root_uri = url::Url::from_directory_path(&root)
        .map_err(|_| "cannot form workspace URI".to_string())?;
    process.request(
        0,
        Initialize::METHOD,
        json!({
            "processId": std::process::id(),
            "rootUri": root_uri.as_str(),
            "capabilities": {
                "general": { "positionEncodings": ["utf-16"] },
                "workspace": { "semanticTokens": { "refreshSupport": true } },
                "textDocument": { "semanticTokens": {
                    "requests": { "full": true },
                    "tokenTypes": TOKEN_TYPES,
                    "tokenModifiers": TOKEN_MODIFIERS,
                    "formats": ["relative"],
                    "augmentsSyntaxTokens": true,
                    "overlappingTokenSupport": false,
                    "multilineTokenSupport": false
                }}
            },
            "initializationOptions": { "diagnostics": { "enabled": false } }
        }),
    )?;
    let started = Instant::now();
    let mut initialized = false;
    let mut legend = None;
    let mut pending = None;
    let mut latest = None;
    let mut open: Option<OpenDocument> = None;
    let mut next_id = 1i32;
    let mut in_flight: Option<(RequestId, Arc<Document>, Instant)> = None;
    loop {
        {
            let mut mailbox = mailbox.lock().unwrap();
            if mailbox.stop {
                drop(mailbox);
                if let Some(opened) = &open {
                    let _ = process.notify(
                        DidCloseTextDocument::METHOD,
                        json!({ "textDocument": { "uri": opened.uri } }),
                    );
                }
                process.shutdown(initialized);
                return Ok(());
            }
            if let Some(document) = mailbox.latest.take() {
                latest = Some(document.0.clone());
                pending = Some(document);
            }
        }
        if !initialized && started.elapsed() >= timeout {
            return Err("language-server initialization timed out".to_string());
        }
        if in_flight
            .as_ref()
            .is_some_and(|(_, _, sent)| sent.elapsed() >= timeout)
        {
            return Err("semantic-token request timed out".to_string());
        }
        if initialized && legend.is_none() && pending.is_some() {
            return Err("language server does not support full semantic tokens".to_string());
        }
        if initialized
            && in_flight.is_none()
            && pending
                .as_ref()
                .is_some_and(|(_, changed): &(Arc<Document>, Instant)| {
                    changed.elapsed() >= DEBOUNCE
                })
        {
            let (document, _) = pending.take().unwrap();
            if open
                .as_ref()
                .is_none_or(|opened| opened.generation != document.generation)
            {
                if let Some(opened) = open.take() {
                    process.notify(
                        DidCloseTextDocument::METHOD,
                        json!({ "textDocument": { "uri": opened.uri } }),
                    )?;
                }
                let path = root.join(format!(
                    ".spectral-session-{}-{}.m2",
                    std::process::id(),
                    document.generation
                ));
                let uri = url::Url::from_file_path(path)
                    .map_err(|_| "cannot form session URI".to_string())?
                    .to_string();
                process.notify(DidOpenTextDocument::METHOD, json!({ "textDocument": {
                    "uri": uri, "languageId": "macaulay2", "version": document.version, "text": document.text
                }}))?;
                open = Some(OpenDocument {
                    generation: document.generation,
                    version: document.version,
                    uri,
                });
            } else if open.as_ref().unwrap().version != document.version {
                process.notify(DidChangeTextDocument::METHOD, json!({
                    "textDocument": { "uri": open.as_ref().unwrap().uri, "version": document.version },
                    "contentChanges": [{ "text": document.text }]
                }))?;
                open.as_mut().unwrap().version = document.version;
            }
            let id = next_id;
            next_id = next_id
                .checked_add(1)
                .filter(|id| *id < i32::MAX)
                .ok_or("LSP request identifiers exhausted")?;
            process.request(
                id,
                SemanticTokensFullRequest::METHOD,
                json!({ "textDocument": { "uri": open.as_ref().unwrap().uri } }),
            )?;
            in_flight = Some((id.into(), document, Instant::now()));
        }
        match process.incoming.recv_timeout(WORKER_INTERVAL) {
            Ok(WireEvent::Message(Message::Response(response)))
                if response.id == RequestId::from(0) =>
            {
                let result = response.response_result.map_err(|error| {
                    format!("language-server initialization failed: {}", error.message)
                })?;
                let result: InitializeResult =
                    serde_json::from_value(result).map_err(|error| error.to_string())?;
                if result
                    .capabilities
                    .position_encoding
                    .as_ref()
                    .is_some_and(|encoding| *encoding != lsp_types::PositionEncodingKind::UTF16)
                {
                    return Err(
                        "language server selected unsupported position encoding".to_string()
                    );
                }
                legend = match result.capabilities.semantic_tokens_provider {
                    Some(SemanticTokensServerCapabilities::SemanticTokensOptions(options))
                        if supports_full(&options.full) =>
                    {
                        Some(options.legend)
                    }
                    Some(SemanticTokensServerCapabilities::SemanticTokensRegistrationOptions(
                        options,
                    )) if supports_full(&options.semantic_tokens_options.full) => {
                        Some(options.semantic_tokens_options.legend)
                    }
                    _ => None,
                };
                process.notify(Initialized::METHOD, json!({}))?;
                initialized = true;
                let _ = events.send(Event::Started(result.server_info.unwrap_or_else(|| {
                    ServerInfo {
                        name: SERVER_EXECUTABLE.to_string(),
                        version: None,
                    }
                })));
            }
            Ok(WireEvent::Message(Message::Response(response))) => {
                if in_flight
                    .as_ref()
                    .is_none_or(|(id, _, _)| *id != response.id)
                {
                    continue;
                }
                let (_, document, _) = in_flight.take().unwrap();
                if latest
                    .as_ref()
                    .is_some_and(|latest| latest.version != document.version)
                {
                    continue;
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
                let _ = events.send(Event::Tokens {
                    document,
                    legend: legend.as_ref().unwrap().clone(),
                    tokens,
                });
            }
            Ok(WireEvent::Message(Message::Request(request))) => {
                let response = if request.method == SemanticTokensRefresh::METHOD {
                    if pending.is_none() {
                        pending = latest
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
                process.send(response)?;
            }
            Ok(WireEvent::Message(Message::Notification(_))) => {}
            Ok(WireEvent::Closed(error)) => return Err(error),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("language-server reader stopped".to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::highlight::Highlighter;
    use crate::code::semantic::{SemanticHighlighter, SourceId};
    use crate::ui::controller::settings::theme::groups::HLGroup;

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
refreshed = False
while True:
    request = read()
    if request is None: break
    method = request.get('method')
    if method == 'textDocument/didOpen':
        opened = request['params']['textDocument']
        assert opened['languageId'] == 'macaulay2'
    elif method == 'textDocument/didChange':
        assert opened is not None
        assert request['params']['textDocument']['version'] > opened['version']
        opened['version'] = request['params']['textDocument']['version']
        opened['text'] = request['params']['contentChanges'][0]['text']
    elif method == 'textDocument/didClose':
        opened = None
    elif method == 'textDocument/semanticTokens/full':
        assert opened is not None
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

    /// The next event after the server's startup announcement.
    fn event(client: &Client) -> Event {
        match next(client) {
            Event::Started(_) => next(client),
            event => event,
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
                Event::Started(_) => panic!("the server starts once"),
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
            Event::Started(_) => panic!("the server starts once"),
        }
        assert!(client.events.recv_timeout(DEBOUNCE).is_err());
    }

    #[test]
    fn initialization_and_requests_have_bounded_timeouts() {
        for mode in ["initialize-timeout", "request-timeout"] {
            let client = fixture(mode, Duration::from_millis(200));
            client.update(document(0, 1, "f"));
            match event(&client) {
                Event::Failed(error) => assert!(error.contains("timed out"), "{error}"),
                Event::Tokens { .. } | Event::Started(_) => panic!("fixture must time out"),
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

    #[test]
    fn missing_executable_reports_a_failure_without_blocking_startup() {
        let client = Client::spawn(
            Command::new("/nonexistent/spectral-m2-language-server"),
            REQUEST_TIMEOUT,
        );
        assert!(matches!(event(&client), Event::Failed(_)));
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
