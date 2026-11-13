//! Macaulay2 process management, protocol types, and transcript decoding.

pub mod preview;
pub mod spec_net;
pub mod transcript;

pub use preview::{PreviewMode, ValuePreview};
pub use spec_net::{CollectionKind, SpecNet};
use std::fmt::Display;
use std::io::{BufRead, BufReader, PipeReader, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use std::{fs, mem, thread};

use crate::cli::M2Options;
use crate::env::ENV;
use crate::errors::{Error, Result};
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use serde::Deserialize;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

const EVALUATION_TIMEOUT: Duration = Duration::from_secs(300);
/// How long after a submission is written before it may be interrupted: a signal
/// reaching M2 just as its read of the submission wakes leaves it blocked on stdin for
/// good, answering neither later signals nor later input.
const INTERRUPT_TIMEOUT: Duration = Duration::from_secs(2);
const INTERRUPT_GRACE: Duration = Duration::from_millis(100);
/// The shell's top-level mode: the hooks that record each result and send its frame.
const STARTUP_SCRIPT: EmbeddedSource = EmbeddedSource {
    file_name: "backend.m2",
    contents: include_str!("backend/m2_startup.m2"),
};
/// The package that defines `SpecNet` and `specNet`, which every result is drawn from.
const DISPLAY_PACKAGE: EmbeddedSource = EmbeddedSource {
    file_name: "Spectral.m2",
    contents: include_str!("backend/Spectral.m2"),
};
/// Introduces a result frame on the child's output stream. ASCII group
/// separator: no diagnostic, prompt or ordinary value begins with it, and the
/// M2 side writes it immediately before the frame's JSON.
const FRAME_MARK: u8 = 0x1D;
/// The arrow of `key => value`.
pub const OPTION_ARROW: &str = "=>";

/// Appended to every submission, with the submission's number filled in. The statement that closes an input -- see the
/// M2 side for why the hooks cannot do it alone for a statement that errored. It
/// hands over the top level's local dictionaries, which only a statement typed
/// at the top level can reach, so the names bound there are reported too.
fn cell_terminator(submission: u64) -> String {
    format!(r#"(User#"spectral end of cell")({submission}, localDictionaries());"#)
}

/// One piece of the child's merged output stream, split on the frame mark.
///
/// Both of the child's streams are handed the same pipe, so printed text,
/// interpreter diagnostics and result frames arrive in the order M2 wrote them
/// and cannot race each other into the wrong cell.
enum Chunk {
    /// Anything M2 said for itself: printed output and diagnostics alike.
    Text(String),
    /// The JSON of a result frame, closing one input.
    Frame(String),
}

/// An M2 source file the shell carries inside itself and hands M2 by path.
#[derive(Clone, Copy, Debug)]
struct EmbeddedSource {
    file_name: &'static str,
    contents: &'static str,
}

impl EmbeddedSource {
    /// The name M2 knows the file by when it is a package: its name, less `.m2`.
    fn package_name(self) -> &'static str {
        self.file_name
            .strip_suffix(".m2")
            .expect("an M2 source file is named *.m2")
    }

    /// Materialize the file under `config` at a stable path, so source positions
    /// in it stay readable. Replacement through a same-directory staging file
    /// keeps parallel launches atomic.
    fn sync_into(self, config: &Path) -> Result<PathBuf> {
        fs::create_dir_all(config)?;
        let path = config.join(self.file_name);
        match fs::read(&path) {
            Ok(contents) if contents == self.contents.as_bytes() => return Ok(path),
            _ => {}
        }
        // Unique per call, not merely per process: several backends may be
        // starting at once (the test suite does exactly that), and a shared
        // staging name means one launch renames the file out from under another.
        static STAGE: AtomicU32 = AtomicU32::new(0);
        let staged = config.join(format!(
            ".{}-{}-{}",
            self.file_name,
            std::process::id(),
            STAGE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&staged, self.contents)?;
        match fs::rename(&staged, &path) {
            Ok(()) => Ok(path),
            Err(error) => {
                let _ = fs::remove_file(staged);
                Err(error.into())
            }
        }
    }
}

/// The frame M2 sends to close one input, or to announce that it is up.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Frame {
    /// The M2 version, present only on the startup frame.
    ready: Option<String>,
    /// The input number M2 gave this statement, absent on the startup frame.
    #[serde(default)]
    number: u32,
    /// The submission this frame answers, absent on the startup frame.
    #[serde(default)]
    submission: u64,

    #[serde(default)]
    error: bool,

    value: Option<SpecNet>,
    #[serde(default)]
    previews: Vec<ValuePreview>,
    type_info: Option<String>,
    type_parts: Option<Vec<SpecNet>>,
    #[serde(default)]
    has_value: bool,
    #[serde(default)]
    depth: u32,
    /// Names M2 had not reported before: all of them on the startup frame.
    #[serde(default)]
    names: Vec<String>,
}

/// The output and status returned by one M2 evaluation.
pub struct M2Response {
    /// The input number M2 evaluated this under, and so the `N` of its `oN`.
    ///
    /// M2 keeps the count itself now that one reader serves the whole session,
    /// which makes this the authority rather than anything the shell tracks: work
    /// the session runs on its own behalf, such as the init file, consumes input
    /// numbers exactly as typed cells do.
    pub number: u32,
    pub is_error: bool,
    /// Everything M2 said while the statement ran -- printed output and
    /// diagnostics, in order. With the input echo off and the mode installing no
    /// prompt, this carries no session scaffolding to unpick.
    pub transcript: String,
    /// The rendered result, present only when M2 displayed it. A `;`-suppressed
    /// result is `None` here -- deliberately never rendered -- yet still bound.
    pub value: Option<SpecNet>,
    /// Independent M2 formatter results available to the preview selector.
    pub previews: Vec<ValuePreview>,
    pub type_info: Option<String>,
    /// The pieces of the type annotation, each rendered as a value is.
    pub type_parts: Vec<SpecNet>,
    /// Whether M2 bound an `oN` for this input. Not implied by `value`: a
    /// suppressed result binds one without producing rendered text.
    pub has_value: bool,
    /// M2's interpreter depth after the evaluation. Anything above 1 means a
    /// nested interpreter, which is what being in the debugger looks like.
    pub depth: u32,
    /// Names the statement bound that M2 had not reported before.
    pub names: Vec<String>,
}

impl M2Response {
    /// Render the displayed value, preserving its Net fallback.
    #[cfg(test)]
    pub fn rendered_value(&self) -> Option<String> {
        self.value.as_ref().map(SpecNet::render)
    }
}

/// The live Macaulay2 child, the stdin it reads its session from, and the
/// demultiplexed stream it answers on.
pub struct M2Backend {
    process: Child,
    input: ChildStdin,
    chunks: Receiver<Chunk>,
    version: String,
    /// Every name M2 started with, until it is taken.
    names: Vec<String>,
    startup_output: String,
    startup_options: M2Options,
    /// Number of the latest submission; its frame is the only one that closes it.
    submission: u64,
}

impl M2Backend {
    #[cfg(test)]
    pub fn spawn() -> Result<Self> {
        Self::spawn_with_options(&M2Options::default())
    }

    /// Start an interactive session with the requested M2 configuration.
    pub fn spawn_with_options(options: &M2Options) -> Result<Self> {
        let config = Self::config_dir()?;
        let init = options.loads_init().then(|| config.join("init.m2"));
        Self::spawn_in(&config, init, None, options)
    }

    /// Start a fresh process with this session's original M2 configuration.
    pub fn restarted(&self) -> Result<Self> {
        Self::spawn_with_options(&self.startup_options)
    }

    /// A Macaulay2 as documentation examples run in: without Spectral's init
    /// file, and with the package they document already loaded, as M2 loads it
    /// before running them. Loading it on the command line costs no input
    /// number, so the examples keep the numbers they were recorded under.
    pub fn spawn_for_examples(package: Option<&str>) -> Result<Self> {
        Self::spawn_in(&Self::config_dir()?, None, package, &M2Options::default())
    }

    #[cfg(test)]
    pub(crate) fn spawn_with_init_file(init_file: Option<PathBuf>) -> Result<Self> {
        let config = match init_file.as_deref().and_then(Path::parent) {
            Some(config) => config.to_path_buf(),
            None => Self::config_dir()?,
        };
        Self::spawn_in(&config, init_file, None, &M2Options::default())
    }

    /// Start M2 and let its own top level read the session from stdin.
    ///
    /// The startup script is loaded with `-e` *before* the top level begins, so
    /// the hooks are installed by the time the first cell can arrive. Deliberately
    /// not `--script`, which would evaluate the file and exit rather than going on
    /// to read stdin. Normal M2 initialization is preserved unless `-q` is
    /// requested, which also removes home-directory package paths. Spectral's
    /// own `init.m2` runs after M2's initialization and before the backend hooks.
    ///
    /// Both output streams are handed the same pipe. Printed text, diagnostics
    /// and result frames then share one ordering, which is what lets a cell's
    /// output be attributed by position rather than by guesswork.
    fn spawn_in(
        config: &Path,
        init_file: Option<PathBuf>,
        package: Option<&str>,
        options: &M2Options,
    ) -> Result<Self> {
        let package_path = DISPLAY_PACKAGE.sync_into(config)?;
        let script_path = STARTUP_SCRIPT.sync_into(config)?;
        let (reader, writer) = std::io::pipe()?;

        let mut command = Command::new("M2");
        command.arg("--silent").arg("--no-readline");
        command.args(options.arguments());
        // The display package comes first, so the init file and a requested
        // package can already install `specNet` for classes of their own.
        command.arg("-e").arg(format!(
            "loadPackage({}, FileName => {}, LoadDocumentation => true)",
            m2_string_literal(DISPLAY_PACKAGE.package_name()),
            m2_string_literal(package_path.display())
        ));
        // Load Spectral's init file before the hooks, so the startup frame is the
        // last thing to arrive before the session opens. `load` rather than a
        // submitted cell: it gives the file its own dictionary, the way every
        // other loaded file gets one, and costs it no input number -- numbering
        // starts at the user's first cell, as it does in any M2.
        if let Some(init_file) = init_file.as_deref().filter(|path| path.is_file()) {
            command
                .arg("-e")
                .arg(format!("load {}", m2_string_literal(init_file.display())));
        }
        if let Some(package) = package {
            command
                .arg("-e")
                .arg(format!("needsPackage {}", m2_string_literal(package)));
        }
        let mut process = command
            .process_group(0)
            .arg("-e")
            .arg(format!("load {}", m2_string_literal(script_path.display())))
            .stdin(Stdio::piped())
            .stdout(writer.try_clone()?)
            .stderr(writer)
            .spawn()?;

        let input = process.stdin.take().ok_or_else(|| {
            Error::Process("Macaulay2 was started without a writable stdin".to_string())
        })?;
        let mut backend = Self {
            process,
            input,
            chunks: split_stream(reader),
            version: String::new(),
            names: Vec::new(),
            startup_output: String::new(),
            startup_options: options.clone(),
            submission: 0,
        };
        let ready = backend.wait_for_ready()?;
        backend.version = ready.ready.unwrap_or_default();
        backend.names = ready.names;
        Ok(backend)
    }

    /// Spectral's persistent configuration directory.
    fn config_dir() -> Result<PathBuf> {
        #[cfg(test)]
        return Ok(ENV
            .temp_dir()
            .join(format!("spectral-m2-test-{}", std::process::id())));

        #[cfg(not(test))]
        ENV.config_home()
            .map(|config| config.join("spectral-m2"))
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "no configuration directory")
                    .into()
            })
    }

    /// Wait for the startup frame, which carries the M2 version and its names.
    ///
    /// M2 sends it once the hooks are installed and before it can read a cell, so
    /// receiving it is the readiness signal. A child that dies during startup
    /// closes the pipe instead, which arrives here as a disconnect.
    fn wait_for_ready(&mut self) -> Result<Frame> {
        let mut said = String::new();
        loop {
            match self.chunks.recv_timeout(STARTUP_TIMEOUT) {
                Ok(Chunk::Text(text)) => said.push_str(&text),
                Ok(Chunk::Frame(frame)) => {
                    let frame: Frame = serde_json::from_str(&frame)?;
                    self.startup_output = said.trim().to_string();
                    return match frame.ready {
                        Some(_) => Ok(frame),
                        None => Err(Error::Process(
                            "Macaulay2 answered before it was ready".to_string(),
                        )),
                    };
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(Error::Timeout(STARTUP_TIMEOUT.as_secs()))
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Error::Process(self.exit_details(said.trim())))
                }
            }
        }
    }

    /// How the child ended, with whatever it said on the way out.
    fn exit_details(&mut self, said: &str) -> String {
        let status = match self.process.wait() {
            Ok(status) => status.to_string(),
            Err(error) => error.to_string(),
        };
        match said.is_empty() {
            true => status,
            false => format!("{status} ({said})"),
        }
    }

    /// Evaluate `code` (executes it), returning once M2 has closed the input.
    ///
    /// `number` is the input number the shell believes this cell has. M2 keeps
    /// its own count now -- one reader means `lineNumber` is the session's real
    /// input number -- so it is no longer sent; it stays in the signature because
    /// the caller still names its cells by it.
    ///
    /// Synchronous and blocking. The terminator statement is appended so that a
    /// statement which errors, and therefore runs none of the hooks, is still
    /// closed by a frame rather than leaving this loop waiting forever.
    #[cfg(test)]
    pub fn evaluate(&mut self, code: &str, number: Option<u32>) -> Result<M2Response> {
        let _ = number;
        self.evaluate_within(code, EVALUATION_TIMEOUT)
    }

    /// Evaluate `code`, giving up once `timeout` passes without an answer. M2 is
    /// then still busy with it, so the process can no longer be trusted with
    /// anything else.
    pub fn evaluate_within(&mut self, code: &str, timeout: Duration) -> Result<M2Response> {
        self.evaluate_with_cancellation(code, timeout, &AtomicBool::new(false))
    }

    /// Evaluate a cell, interrupting M2 once cancellation is requested.
    pub fn evaluate_cancellable(
        &mut self,
        code: &str,
        cancelled: &AtomicBool,
    ) -> Result<M2Response> {
        self.evaluate_with_cancellation(code, EVALUATION_TIMEOUT, cancelled)
    }

    fn evaluate_with_cancellation(
        &mut self,
        code: &str,
        timeout: Duration,
        cancelled: &AtomicBool,
    ) -> Result<M2Response> {
        if cancelled.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        self.submission += 1;
        let terminator = cell_terminator(self.submission);
        let written = writeln!(self.input, "{}\n{terminator}", code.trim_end())
            .and_then(|()| self.input.flush());
        if let Err(error) = written {
            return match error.kind() {
                std::io::ErrorKind::BrokenPipe => Err(Error::Exited(self.process.wait()?)),
                _ => Err(error.into()),
            };
        }

        let mut transcript = String::new();
        let deadline = Instant::now() + timeout;
        let written_at = Instant::now();
        let mut interrupted_at = None;
        loop {
            if interrupted_at.is_none()
                && cancelled.load(Ordering::Acquire)
                && written_at.elapsed() >= INTERRUPT_GRACE
            {
                // External programs may own the computation while M2 waits.
                // The child's private group keeps the signal away from the UI.
                self.signal_group(Signal::SIGINT)?;
                interrupted_at = Some(Instant::now());
            }
            if interrupted_at.is_some_and(|started: Instant| started.elapsed() >= INTERRUPT_TIMEOUT)
            {
                return Err(Error::Process(
                    "Macaulay2 did not respond to cancellation; restart the session".to_string(),
                ));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout(timeout.as_secs()));
            }
            match self
                .chunks
                .recv_timeout(remaining.min(Duration::from_millis(20)))
            {
                Ok(Chunk::Text(text)) => transcript.push_str(&text),
                Ok(Chunk::Frame(frame)) => {
                    let frame: Frame = serde_json::from_str(&frame)?;
                    if frame.submission != self.submission {
                        // Closes an earlier submission: its output is not ours.
                        transcript.clear();
                        continue;
                    }
                    return Ok(M2Response {
                        number: frame.number,
                        is_error: frame.error,
                        transcript,
                        value: frame.value,
                        previews: frame.previews,
                        type_info: frame.type_info,
                        type_parts: frame.type_parts.unwrap_or_default(),
                        has_value: frame.has_value,
                        depth: frame.depth,
                        names: frame.names,
                    });
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Error::Exited(self.process.wait()?));
                }
            }
        }
    }

    /// The M2 version, as reported by the startup frame.
    pub fn version(&self) -> Result<String> {
        Ok(self.version.clone())
    }

    /// Every name M2 started with; later names arrive with each response.
    pub fn take_names(&mut self) -> Vec<String> {
        mem::take(&mut self.names)
    }

    /// Text printed during initialization before the backend became ready.
    pub fn take_startup_output(&mut self) -> String {
        mem::take(&mut self.startup_output)
    }

    /// The operating-system id of the M2 process.
    pub fn process_id(&self) -> u32 {
        self.process.id()
    }

    /// Signal everything in the child's private group, not just the child.
    ///
    /// `M2` on the `PATH` may be a launcher script rather than M2 itself, and one
    /// that does not `exec` leaves M2 as its own child: a signal to the direct
    /// child alone would reach the script and leave M2 running.
    fn signal_group(&self, signal: Signal) -> Result<()> {
        killpg(Pid::from_raw(self.process.id() as i32), signal).map_err(std::io::Error::from)?;
        Ok(())
    }
}

/// Spell `text` as an M2 string literal, for the `-e` arguments M2 starts with.
fn m2_string_literal(text: impl Display) -> String {
    format!(
        "\"{}\"",
        text.to_string().replace('\\', r"\\").replace('"', "\\\"")
    )
}

/// Split the child's output into printed text and result frames on a thread.
///
/// A frame runs from the mark to the end of its line, so `read_until` gives the
/// text before it and `read_line` the frame itself. Reading rather than blocking
/// also keeps the pipe drained, which matters because programs M2 launches --
/// the browser `viewHelp` opens, say -- inherit it and are happy to chatter;
/// left unread, a full buffer stops M2 answering for good.
fn split_stream(stream: PipeReader) -> Receiver<Chunk> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || relay_chunks(BufReader::new(stream), &sender));
    receiver
}

fn relay_chunks(mut stream: impl BufRead, sender: &Sender<Chunk>) {
    loop {
        let mut text = Vec::new();
        match stream.read_until(FRAME_MARK, &mut text) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let framed = text.last() == Some(&FRAME_MARK);
        if framed {
            text.pop();
        }
        if !text.is_empty()
            && sender
                .send(Chunk::Text(String::from_utf8_lossy(&text).into_owned()))
                .is_err()
        {
            return;
        }
        if !framed {
            return;
        }
        let mut frame = String::new();
        match stream.read_line(&mut frame) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if sender.send(Chunk::Frame(frame)).is_err() {
            return;
        }
    }
}

impl Drop for M2Backend {
    fn drop(&mut self) {
        let _ = self.signal_group(Signal::SIGKILL);
        let _ = self.process.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    use crate::ui::theme::groups::HLGroup;

    /// A hash table's rows, each its key, the arrow and its value, aligned as one.
    fn table_rows(table: &SpecNet) -> &[SpecNet] {
        let [aligned] = table.children.as_slice() else {
            panic!("a table holds its rows aligned together: {table:?}");
        };
        assert!(aligned.is("AlignedExpression"));
        assert!(aligned
            .children
            .iter()
            .all(|row| row.children.len() == 3 && row.children[1].group() == HLGroup::Operator));
        &aligned.children
    }

    #[test]
    fn debugger_commands_return_to_a_usable_session() {
        for command in ["break", "end", "continue"] {
            let mut backend = M2Backend::spawn().expect("spawn M2");
            let baseline = backend
                .evaluate_within("debuggingMode = true;", Duration::from_secs(2))
                .expect("enable debugger");
            let paused = backend
                .evaluate_within("breakpoint \"pause\"", Duration::from_secs(2))
                .expect("enter debugger");
            assert!(paused.depth > baseline.depth, "{}", paused.transcript);
            let resumed = backend
                .evaluate_within(command, Duration::from_secs(2))
                .expect("leave debugger");
            assert_eq!(
                resumed.depth, baseline.depth,
                "{command}: {}",
                resumed.transcript
            );
            let recovered = backend
                .evaluate_within("6 * 7", Duration::from_secs(2))
                .expect("evaluate after debugger");
            assert!(!recovered.is_error, "{}", recovered.transcript);
            assert_eq!(recovered.rendered_value().as_deref(), Some("42"));
        }
    }

    #[test]
    fn debugger_can_step_through_a_loaded_function_and_exit() {
        let mut backend = M2Backend::spawn().expect("spawn M2");
        let path = M2Backend::config_dir()
            .expect("test directory")
            .join("debugger-stepping.m2");
        fs::write(
            &path,
            "debuggingMode = true;\ndebuggerExample = t -> (\n breakpoint t;\n t + 1\n);\n",
        )
        .expect("write function source");
        let baseline = backend
            .evaluate_within(
                &format!("load {}", m2_string_literal(path.display())),
                Duration::from_secs(2),
            )
            .expect("load function");
        assert!(!baseline.is_error, "{}", baseline.transcript);
        let paused = backend
            .evaluate_within("debuggerExample 41", Duration::from_secs(2))
            .expect("enter debugger");
        assert!(paused.depth > baseline.depth, "{}", paused.transcript);
        let stepped = backend
            .evaluate_within("step", Duration::from_secs(2))
            .expect("step to next expression");
        assert_eq!(stepped.depth, paused.depth, "{}", stepped.transcript);
        let inspected = backend
            .evaluate_within("t", Duration::from_secs(2))
            .expect("inspect function argument");
        assert_eq!(inspected.rendered_value().as_deref(), Some("41"));
        let resumed = backend
            .evaluate_within("continue", Duration::from_secs(2))
            .expect("resume function");
        assert_eq!(resumed.depth, baseline.depth, "{}", resumed.transcript);
        assert_eq!(resumed.rendered_value().as_deref(), Some("42"));
        let recovered = backend
            .evaluate_within("6 * 7", Duration::from_secs(2))
            .expect("evaluate after stepping");
        assert!(!recovered.is_error, "{}", recovered.transcript);
        assert_eq!(recovered.rendered_value().as_deref(), Some("42"));
    }

    #[test]
    fn cancellation_stops_external_commands_and_preserves_session_state() {
        let mut backend = M2Backend::spawn().expect("spawn M2");
        backend
            .evaluate("savedBeforeExternalCommand = 41;", None)
            .expect("save session state");
        let cancelled = AtomicBool::new(false);
        let response = thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(100));
                cancelled.store(true, Ordering::Release);
            });
            backend.evaluate_with_cancellation(
                "run \"sleep 10\"",
                Duration::from_secs(2),
                &cancelled,
            )
        });
        response.expect("external command stopped before its sleep completed");
        let next = backend
            .evaluate_within("savedBeforeExternalCommand + 1", Duration::from_secs(2))
            .expect("evaluate after cancellation");
        assert!(!next.is_error, "{}", next.transcript);
        assert_eq!(next.rendered_value().as_deref(), Some("42"));
    }

    #[test]
    fn cancellation_near_completion_keeps_response_frames_in_sync() {
        let mut backend = M2Backend::spawn().expect("spawn M2");
        for delay in [0, 1, 5, 20, 50] {
            let cancelled = AtomicBool::new(false);
            let response = thread::scope(|scope| {
                scope.spawn(|| {
                    thread::sleep(Duration::from_millis(delay));
                    cancelled.store(true, Ordering::Release);
                });
                backend.evaluate_with_cancellation(
                    "sum(1..1000, identity)",
                    Duration::from_secs(2),
                    &cancelled,
                )
            });
            assert!(
                matches!(response, Ok(_) | Err(Error::Cancelled)),
                "{:?}",
                response.as_ref().err()
            );
            let next = backend
                .evaluate_within("1+1", Duration::from_secs(2))
                .expect("evaluate after cancellation");
            assert!(!next.is_error, "{}", next.transcript);
            assert_eq!(next.rendered_value().as_deref(), Some("2"));
        }
    }

    #[test]
    fn a_frame_for_an_earlier_submission_is_not_filed_under_the_current_cell() {
        let mut backend = M2Backend::spawn().expect("spawn M2");
        let stray = cell_terminator(0);
        let response = backend
            .evaluate(&format!("{stray}\n7 + 1"), None)
            .expect("evaluate with a stray frame ahead of the value");
        assert_eq!(response.rendered_value().as_deref(), Some("8"));
        let next = backend.evaluate("2", None).expect("evaluate after");
        assert_eq!(next.rendered_value().as_deref(), Some("2"));
    }

    #[test]
    fn plain_values_preserve_net_rows_without_building_a_character_grid() {
        let text = "  x²  \n\n y\t\n";
        let expected = "  x²\n\n y";
        for value in [
            SpecNet::plain(text.into()),
            SpecNet::tex_fixture("\\begin{tabular}...".into(), text.into(), String::new()),
            SpecNet::document(String::new(), text.into(), 77),
            SpecNet::token_fixture(text.into(), "String"),
        ] {
            assert_eq!(value.render(), expected);
        }
    }

    #[test]
    fn structured_values_render_as_unicode_blocks() {
        let fraction = SpecNet::branch(
            "Divide",
            vec![SpecNet::leaf("ZZ", "1"), SpecNet::leaf("ZZ", "2")],
        );
        assert_eq!(fraction.render(), "1\n─\n2");
        let matrix = SpecNet::branch(
            "MatrixExpression",
            vec![
                SpecNet::branch(
                    "List",
                    vec![SpecNet::leaf("Symbol", "x"), SpecNet::leaf("ZZ", "1")],
                ),
                SpecNet::branch(
                    "List",
                    vec![SpecNet::leaf("ZZ", "0"), SpecNet::leaf("Symbol", "y")],
                ),
            ],
        );
        assert_eq!(matrix.render(), "⎛x  1⎞\n⎝0  y⎠");
    }

    #[test]
    fn unsupported_tex_uses_its_net_fallback() {
        let value = SpecNet::tex_fixture(
            r"$\spectralUnsupported{true}$".into(),
            "true".into(),
            String::new(),
        );
        assert_eq!(value.render(), "true");

        let document = SpecNet::tex_fixture(
            r"\par\textbf{Description}".into(),
            "Description".into(),
            String::new(),
        );
        assert_eq!(document.render(), "Description");
    }

    #[test]
    fn rings_and_ideals_keep_structured_or_opaque_representations() {
        let mut backend = M2Backend::spawn().unwrap();
        for source in ["ZZ", "QQ", "QQ[x,y]", "R = QQ[x,y]; ideal(x,y)"] {
            let result = backend.evaluate(source, None).unwrap();
            assert!(!result.is_error, "{source}: {}", result.transcript);
            let value = result.value.unwrap();
            assert!(!value.classes.is_empty());
            assert!(!value.render().is_empty());
            assert!(!value.ascii().is_empty());
        }
    }

    #[test]
    fn m2_tex_dialect_renders_collections() {
        for (source, expected) in [
            (r"$\left\{1,\:2,\:3\right\}$", "{1, 2, 3}"),
            (r"$\texttt{print}$", "print"),
        ] {
            let value = SpecNet::tex_fixture(source.into(), "NET FALLBACK".into(), String::new());
            assert_eq!(value.render(), expected);
        }
    }

    // In QQ[xx,yy,xxyy] M2's net collapses four distinct matrix entries into two
    // -- `xx*yy` loses its `*` and reads as the variable `xxyy` -- so both columns
    // print identically. The TeX keeps them apart, which is the whole reason the
    // shell prefers it.
    #[test]
    fn colliding_monomial_names_keep_distinct_expression_structure() {
        let mut backend = M2Backend::spawn().unwrap();
        let result = backend
            .evaluate(
                "R = QQ[xx,yy,xxyy]; matrix{{xx*yy, xxyy}, {xx*yy^2, xxyy^2}}",
                None,
            )
            .unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        let value = result.value.unwrap();
        let entries = &value.children[0].children;
        assert!(entries[0].is("Product"), "{:?}", entries[0]);
        assert!(entries[1].text.is_some());
        assert_ne!(entries[0], entries[1]);
    }

    // M2's own net for this matrix is `| x2+1 1/3x2 |`, which reads as the ring
    // variable `x2` plus one -- a different element entirely. Its TeX keeps the
    // two apart (`x^{2}+1` against `\mathit{x2}`), so rendering the TeX is what
    // makes the value readable and falling back to the net is what loses it.
    #[test]
    fn a_matrix_renders_from_tex_rather_than_the_ambiguous_net() {
        let value = SpecNet::tex_fixture(
            r"$\left(\!\begin{array}{cc}
1&x\\
x^{2}+1&\frac{1}{3}\mathit{x2}
\end{array}\!\right)$"
                .into(),
            "| 1    x     |\n| x2+1 1/3x2 |".into(),
            String::new(),
        );
        let rendered = value.render();

        assert!(
            !rendered.contains("x2+1"),
            "fell back to the ambiguous net: {rendered}"
        );
        assert!(
            rendered.contains("x² + 1") && rendered.contains("x2"),
            "the exponent and the variable must stay distinguishable: {rendered}"
        );
    }

    #[test]
    fn ascii_spells_every_piece_as_macaulay2_does() {
        let mut backend = M2Backend::spawn().unwrap();
        let result = backend.evaluate("hashTable{1 => 1/2}", None).unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        let ascii = result.value.unwrap().ascii();
        assert!(ascii.is_ascii(), "{ascii}");
        assert!(ascii.contains("1 =>"), "{ascii}");
    }

    #[test]
    fn serre_twists_preserve_parentheses_around_the_degree() {
        let mut backend = M2Backend::spawn().unwrap();
        let setup = backend.evaluate("R = QQ[x,y,z];", None).unwrap();
        assert!(!setup.is_error, "{}", setup.transcript);
        for degree in [-4, 3] {
            let source = format!("sheaf(R^{{{degree}}})");
            let result = backend.evaluate(&source, None).unwrap();
            assert!(!result.is_error, "{}", result.transcript);
            let value = result.value.unwrap();
            let twist = value.children.last().unwrap();
            assert!(twist.is("Parenthesize"), "{value:?}");
            assert!(value.ascii().contains(&format!("({degree})")));
        }
    }

    #[test]
    fn sheaves_varieties_and_scripted_functors_have_structured_previews() {
        use crate::ui::render::maths::{Math, MathStyle};
        use crate::ui::render::typeset;
        use crate::ui::theme::groups::Groups;

        let mut backend = M2Backend::spawn().unwrap();
        let setup = backend
            .evaluate("R = QQ[x,y,z]; X = Proj R; F = sheaf image vars R;", None)
            .unwrap();
        assert!(!setup.is_error, "{}", setup.transcript);
        for source in [
            "F",
            "expression F",
            "sheaf(R^2 ++ R^{-1})",
            "OO",
            "OO_X",
            "X",
            "Proj(R/ideal(x*y))",
            "sheaf map(R^1,R^1,x)",
            "Spec R",
        ] {
            let result = backend.evaluate(source, None).unwrap();
            assert!(!result.is_error, "{source}: {}", result.transcript);
            let value = result.value.unwrap();
            assert!(Math::supports_spec_net(&value), "{source}: {value:?}");
            let rows = typeset::drawn(&value, 120, &Groups::default(), MathStyle::default());
            assert!(!rows.lines.is_empty(), "{source}");
        }
    }

    #[test]
    fn frames_survive_the_user_redefining_to_string() {
        let mut backend = M2Backend::spawn().expect("spawn M2");
        backend
            .evaluate("toString ZZ := integer -> \"redefined\"", Some(1))
            .expect("redefine toString");

        let after = backend
            .evaluate("6 * 7", Some(2))
            .expect("frame arrives after the redefinition");

        assert!(!after.is_error, "{}", after.transcript);
        assert_eq!(after.number, 2);
        assert_eq!(after.rendered_value().as_deref(), Some("redefined"));
        assert!(after.value.as_ref().unwrap().is("ZZ"));
        assert!(after.has_value);
    }

    #[test]
    fn evaluation_distinguishes_printed_null_from_returned_value() {
        let mut backend = M2Backend::spawn().expect("spawn M2");

        let printed = backend
            .evaluate("print \"hello\"", Some(1))
            .expect("evaluate null result");
        assert!(!printed.is_error);
        // The transcript is printed text and nothing else: M2 reads the session
        // with its input echo off, so the submitted code is not read back.
        assert_eq!(printed.transcript, "hello\n");
        // An unterminated statement reports its result even when that result is
        // null -- but null binds no oN, so there is no symbol to name it by.
        assert_eq!(printed.rendered_value().as_deref(), Some("null"));
        assert_eq!(printed.type_info.as_deref(), Some("Nothing"));
        assert!(!printed.has_value, "a null result binds no oN");
        assert_eq!(
            transcript::captured_output(
                &printed.transcript,
                "print \"hello\"",
                printed.rendered_value().as_deref(),
                printed.type_info.as_deref(),
            )
            .printed,
            "hello\n"
        );

        let returned = backend
            .evaluate("2 + 2", Some(2))
            .expect("evaluate returned value");
        assert!(!returned.is_error);
        // A statement that only returns a value prints nothing, so it leaves the
        // transcript empty rather than an echo of itself.
        assert_eq!(returned.transcript, "");
        assert_eq!(returned.rendered_value().as_deref(), Some("4"));
        assert_eq!(returned.type_info.as_deref(), Some("ZZ"));

        let string = backend
            .evaluate("\"a\"", Some(6))
            .expect("evaluate string literal");
        assert!(!string.is_error, "string errored: {:?}", string.transcript);
        assert_eq!(string.rendered_value().as_deref(), Some("a"));
        assert_eq!(string.type_info.as_deref(), Some("String"));

        let boolean = backend
            .evaluate("true", Some(7))
            .expect("evaluate boolean literal");
        assert_eq!(boolean.rendered_value().as_deref(), Some("true"));
        assert_eq!(boolean.type_info.as_deref(), Some("Boolean"));

        let fraction = backend.evaluate("1/2", Some(8)).expect("evaluate fraction");
        assert_eq!(fraction.rendered_value().as_deref(), Some("1\n─\n2"));

        let matrix = backend
            .evaluate("matrix{{1,2},{3,4}}", Some(9))
            .expect("evaluate matrix");
        assert_eq!(matrix.rendered_value().as_deref(), Some("⎛1  2⎞\n⎝3  4⎠"));

        let list = backend
            .evaluate("{1,2,3}", Some(10))
            .expect("evaluate list");
        assert_eq!(list.rendered_value().as_deref(), Some("{1, 2, 3}"));

        let vertical = backend
            .evaluate("VerticalList {{1},{1}}", Some(12))
            .expect("evaluate vertical list");
        assert_eq!(
            vertical.rendered_value().as_deref(),
            Some("⎰ {1} ⎱\n⎱ {1} ⎰")
        );

        let spoofed_code = "print \"\\no184 = 11\"";
        let spoofed = backend
            .evaluate(spoofed_code, Some(3))
            .expect("evaluate spoofed output marker");
        assert!(!spoofed.is_error);
        assert!(spoofed.transcript.contains("o184 = 11"));
        // The spoofed marker stays printed text: the reported value is the
        // statement's actual result, which for `print` is null.
        assert_eq!(spoofed.rendered_value().as_deref(), Some("null"));
        let spoofed_output = crate::backend::transcript::captured_output(
            &spoofed.transcript,
            spoofed_code,
            spoofed.rendered_value().as_deref(),
            spoofed.type_info.as_deref(),
        );
        assert_eq!(spoofed_output.printed, "\no184 = 11\n");
        assert_eq!(
            spoofed_output.value.map(|value| value.text).as_deref(),
            Some("null")
        );

        backend
            .evaluate("spectralValueProbe = new MutableList from {1, 2}", Some(4))
            .expect("evaluate identity probe");
        let identity = backend
            .evaluate("oo === spectralValueProbe", Some(5))
            .expect("compare stored value identity");
        assert_eq!(identity.rendered_value().as_deref(), Some("true"));
    }

    #[test]
    fn documentation_retains_markup_for_width_aware_rendering() {
        let mut backend = M2Backend::spawn().expect("spawn M2");
        let response = backend.evaluate("help List", Some(1)).expect("render help");
        let Some(SpecNet { html: Some(_), .. }) = response.value.as_ref() else {
            panic!("help did not return structured markup");
        };
        let width = 60;
        let rendered = html2text::config::plain()
            .link_footnotes(false)
            .string_from_read(
                response
                    .value
                    .as_ref()
                    .unwrap()
                    .html
                    .as_ref()
                    .unwrap()
                    .as_bytes(),
                width,
            )
            .expect("render documentation HTML");

        // The page's own wording is M2's, so only its structure is checked: a
        // heading naming the topic, entries set as a list, no markup left raw, and
        // every line within the width it was rendered for.
        let lines: Vec<&str> = rendered.lines().collect();
        assert!(
            lines.iter().any(|line| line.starts_with("# List")),
            "{rendered}"
        );
        assert!(
            lines.iter().any(|line| line.starts_with("* ")),
            "{rendered}"
        );
        assert!(!rendered.contains("<div"), "{rendered}");
        assert!(
            lines
                .iter()
                .all(|line| UnicodeWidthStr::width(*line) <= width),
            "{rendered}"
        );
    }

    #[test]
    fn documentation_source_locations_become_resolved_file_links() {
        let path = ENV.temp_dir().join(format!(
            "spectral documentation source {}.m2",
            std::process::id()
        ));
        std::fs::write(&path, "sourceLocationProbe = 1\n").unwrap();
        let location = serde_json::to_string(&format!("{}:12:3", path.display())).unwrap();
        let mut backend = M2Backend::spawn().unwrap();
        let response = backend
            .evaluate(&format!("needsPackage \"Text\"; TT {{{location}}}"), None)
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        let Some(SpecNet {
            html: Some(html), ..
        }) = response.value
        else {
            panic!("source location did not produce markup")
        };
        let mut target = url::Url::from_file_path(&path).unwrap();
        target.set_fragment(Some("L12:C3"));
        assert!(html.contains(&format!("href=\"{target}\"")), "{html}");
    }

    #[test]
    fn functions_written_in_source_files_link_to_where_they_were_written() {
        let mut backend = M2Backend::spawn().unwrap();
        let file = ENV
            .temp_dir()
            .join(format!("sp2-function-{}.m2", std::process::id()));
        fs::write(&file, "linkedProbe = x -> x+1;\n").unwrap();
        backend
            .evaluate(&format!("load {}", m2_string_literal(file.display())), None)
            .unwrap();
        let result = backend.evaluate("linkedProbe", None).unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        let value = result.value.unwrap();
        assert!(value.is("Function"));
        let target = value.target.unwrap();
        assert!(target.starts_with("file://"));
        assert!(target.contains("#L"));
    }

    #[test]
    fn documentation_links_carry_the_key_help_is_asked_for() {
        let mut backend = M2Backend::spawn().expect("spawn M2");
        let response = backend
            .evaluate("help ancestors", None)
            .expect("read real documentation");
        let Some(SpecNet {
            html: Some(html), ..
        }) = response.value
        else {
            panic!("documentation did not return markup");
        };
        assert!(html.contains(r#"href="&quot;ancestor&quot;""#), "{html}");
        assert!(!html.contains(".html\""), "{html}");
    }

    #[test]
    fn text_documentation_examples_render_as_native_session_blocks() {
        let mut backend = M2Backend::spawn().expect("spawn M2");
        let response = backend
            .evaluate("help det", None)
            .expect("read real documentation");
        let Some(SpecNet {
            html: Some(html),
            example_width,
            ..
        }) = response.value
        else {
            panic!("documentation did not return markup");
        };
        for width in [28, 60, 100] {
            let context = crate::ui::render::RenderContext::new(width, 0);
            let lines = crate::ui::render::markup::lines(&html, example_width, &context)
                .expect("render documentation");
            let text = lines
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains("Description"), "{text}");
            let unwrapped: String = text.split_whitespace().collect();
            assert!(
                unwrapped.contains("genericMatrix(R,2,2)"),
                "the examples are drawn: {text}"
            );
            assert!(!text.contains("i1 :"), "{text}");
            assert!(text.contains("o1"), "{text}");
            assert!(!text.contains("o1 ="), "{text}");
            assert!(
                lines.iter().all(|line| line.width() <= usize::from(width)),
                "{text}"
            );
        }
    }

    // One reader means M2's own `lineNumber` is the session's input number, so
    // results are bound by M2 rather than under a number the shell supplies. The
    // number a cell reports back is therefore the one its `oN` answers to.
    #[test]
    fn results_bind_the_output_symbol_m2_reports() {
        let mut backend = M2Backend::spawn().expect("spawn M2");

        let bound = backend.evaluate("41 + 1", Some(42)).expect("evaluate");
        let recalled = backend
            .evaluate(&format!("o{}", bound.number), Some(43))
            .expect("recall the bound output");
        assert_eq!(recalled.rendered_value().as_deref(), Some("42"));

        // Numbering runs on unbroken: the terminator statement appended to every
        // submission must not consume an input number of its own.
        assert_eq!(recalled.number, bound.number + 1);

        // A `;` suppresses rendering, so no value comes back -- but the binding
        // is still made, which is what has_value reports.
        let suppressed = backend
            .evaluate("spectralSuppressed = 7;", Some(44))
            .expect("evaluate suppressed");
        assert!(suppressed.value.is_none(), "`;` suppresses the rendering");
        assert!(suppressed.has_value, "a suppressed result still binds oN");
        assert_eq!(suppressed.type_info.as_deref(), Some("ZZ"));
        let recalled = backend
            .evaluate(&format!("o{}", suppressed.number), Some(45))
            .expect("recall the suppressed binding");
        assert_eq!(recalled.rendered_value().as_deref(), Some("7"));
    }

    // M2 rotates `oo`/`ooo`/`oooo` after every statement, a null-valued one
    // included, so the terminator appended to each submission must opt out or
    // every cell's result lands in the history twice.
    #[test]
    fn output_history_advances_once_per_cell() {
        let mut backend = M2Backend::spawn().expect("spawn M2");

        backend.evaluate("a = 2", Some(1)).expect("evaluate");
        backend.evaluate("a = 3", Some(2)).expect("evaluate");
        let history = backend
            .evaluate("(oo, ooo)", Some(3))
            .expect("recall the output history");
        assert_eq!(history.rendered_value().as_deref(), Some("(3, 2)"));

        let shifted = backend
            .evaluate("(oo, ooo, oooo)", Some(4))
            .expect("recall the shifted history");
        assert_eq!(shifted.rendered_value().as_deref(), Some("((3, 2), 3, 2)"));
    }

    // Containers arrive entry by entry, so the shell can colour each entry by its
    // own role. A list's own TeX is one string, which every entry's colour would
    // be lost in.
    #[test]
    fn output_previews_use_each_m2_formatter_without_repeating_evaluation() {
        let mut backend = M2Backend::spawn().unwrap();
        let response = backend
            .evaluate(
                r#"PreviewValue = new Type of HashTable;
            net PreviewValue := value -> net "net preview";
            toString PreviewValue := value -> "string preview";
            tex PreviewValue := value -> "$p$";
            toExternalString PreviewValue := value -> "external preview";
            expression PreviewValue := value -> symbol p;
            previewEvaluations = 0;
            (previewEvaluations = previewEvaluations + 1; new PreviewValue from {})"#,
                None,
            )
            .unwrap();
        assert!(!response.is_error, "{}", response.transcript);
        for (mode, text) in [
            (PreviewMode::Net, "net preview"),
            (PreviewMode::String, "string preview"),
            (PreviewMode::Tex, "$p$"),
            (PreviewMode::External, "external preview"),
        ] {
            assert!(response.previews.contains(&ValuePreview {
                mode,
                text: text.into()
            }));
        }
        assert_eq!(response.value.unwrap().render(), "p");
        assert_eq!(
            backend
                .evaluate("previewEvaluations", None)
                .unwrap()
                .rendered_value()
                .as_deref(),
            Some("1")
        );
    }

    #[test]
    fn a_failed_preview_formatter_does_not_hide_other_representations() {
        let mut backend = M2Backend::spawn().unwrap();
        let response = backend
            .evaluate(
                r#"PreviewValue = new Type of HashTable;
            tex PreviewValue := value -> error "unsupported TeX";
            new PreviewValue from {}"#,
                None,
            )
            .unwrap();
        assert!(!response.is_error, "{}", response.transcript);
        assert!(response.value.is_some());
        assert!(!response
            .previews
            .iter()
            .any(|preview| preview.mode == PreviewMode::Tex));
        assert!(response
            .previews
            .iter()
            .any(|preview| preview.mode == PreviewMode::Net));
    }

    #[test]
    fn containers_arrive_entry_by_entry() {
        let mut backend = M2Backend::spawn().unwrap();
        let result = backend
            .evaluate("{1, [2, 3], (4, 5), a => 1/2}", None)
            .unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        let value = result.value.unwrap();
        assert!(value.is("List"));
        assert_eq!(value.children.len(), 4);
        assert!(value.children[1].is("Array"));
        assert!(value.children[2].is("Sequence"));
        assert!(value.children[3].is("Option"));
        let looped = backend
            .evaluate(
                "looped = new MutableHashTable; looped#self = looped; looped",
                None,
            )
            .unwrap();
        assert!(!looped.is_error, "{}", looped.transcript);
        let looped = looped.value.unwrap();
        assert!(table_rows(&looped)[0].children[2].text.is_some());
        let after = backend.evaluate("{1, 2}", None).unwrap();
        assert!(!after.is_error);
        assert_eq!(after.value.unwrap().children.len(), 2);
    }

    // A container's inherited TeX only spells out its structure, `Shout{...}`,
    // where M2 prints what its class's own net draws. A real TeX form, such as
    // the one a ring family carries, is still preferred.
    #[test]
    fn an_opaque_class_keeps_its_identity_and_text() {
        let mut backend = M2Backend::spawn().unwrap();
        let result = backend.evaluate("Opaque = new Type of HashTable; toString Opaque := x -> \"opaque\"; expression Opaque := x -> hold x; new Opaque from {}", None).unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        let value = result.value.unwrap();
        assert_eq!(value.class(), "Opaque");
        assert_eq!(value.text.as_deref(), Some("opaque"));
        assert!(value.children.is_empty());
    }

    // One reader holds one local dictionary for the whole session, exactly as an
    // interactive M2 does, so a top-level `:=` is still bound at the next input.
    // Evaluating each cell through `capture` gave every cell a dictionary of its
    // own and the binding never survived.
    #[test]
    fn names_arrive_once_as_m2_comes_to_know_them() {
        let mut backend = M2Backend::spawn().expect("spawn M2");
        let started = backend.take_names();
        assert!(started.iter().any(|name| name == "ideal"));
        assert!(
            started.iter().all(|name| name != "toJSON"),
            "the transport's names are given back"
        );

        let bound = backend
            .evaluate("spectralGlobal = 1; spectralLocal := 2", Some(1))
            .expect("bind names");
        for name in ["spectralGlobal", "spectralLocal"] {
            assert!(
                bound.names.iter().any(|bound| bound == name),
                "{name} is reported"
            );
        }
        assert!(!bound.names.iter().any(|name| name == "ideal"));

        let unchanged = backend
            .evaluate("spectralGlobal + 1", Some(2))
            .expect("use names");
        assert!(unchanged.names.is_empty());
    }

    #[test]
    fn local_assignments_outlive_their_cell() {
        let mut backend = M2Backend::spawn().expect("spawn M2");

        backend
            .evaluate("spectralLocal := 1729", Some(1))
            .expect("bind local");
        let recalled = backend.evaluate("spectralLocal", Some(2)).expect("recall");
        assert_eq!(recalled.rendered_value().as_deref(), Some("1729"));

        // A promoted name is an ordinary session name: plain `=` reassigns it.
        backend
            .evaluate("spectralLocal = 7", Some(3))
            .expect("reassign");
        let reassigned = backend.evaluate("spectralLocal", Some(4)).expect("recall");
        assert_eq!(reassigned.rendered_value().as_deref(), Some("7"));

        // The statement that carries the bindings out must leave no trace: it is
        // neither echoed into the transcript nor allowed to claim the cell's
        // result, type, or output number.
        let bound = backend
            .evaluate("spectralQuiet := 5", Some(5))
            .expect("bind local");
        assert_eq!(bound.rendered_value().as_deref(), Some("5"));
        assert_eq!(bound.type_info.as_deref(), Some("ZZ"));
        assert!(
            !bound.transcript.contains("localDictionaries"),
            "promotion statement leaked into {:?}",
            bound.transcript
        );
        let printed = crate::backend::transcript::captured_output(
            &bound.transcript,
            "spectralQuiet := 5",
            None,
            None,
        );
        assert_eq!(printed.printed, "");
        let recalled = backend.evaluate("o5", Some(6)).expect("recall o5");
        assert_eq!(recalled.rendered_value().as_deref(), Some("5"));
    }

    // A `:=` inside a function body is that function's own local. Promoting it
    // would leak it into the session and, worse, shadow whatever the user has.
    #[test]
    fn locals_inside_a_function_body_stay_there() {
        let mut backend = M2Backend::spawn().expect("spawn M2");

        backend
            .evaluate(
                "spectralNested = a -> (spectralHidden := a; spectralHidden)",
                Some(1),
            )
            .expect("define function");
        backend.evaluate("spectralNested 3", Some(2)).expect("call");
        let leaked = backend.evaluate("spectralHidden", Some(3)).expect("recall");
        assert_eq!(leaked.type_info.as_deref(), Some("Symbol"));
    }

    // The transport loads JSON and JSONRPC, whose dictionaries would otherwise
    // sit at the front of dictionaryPath and reserve their exports: `json = 3`
    // would fail with "assignment to protected global variable 'toJSON'".
    #[test]
    fn transport_packages_do_not_reserve_user_names() {
        let mut backend = M2Backend::spawn().expect("spawn M2");

        for (number, code, name) in [(1, "json = 3", "json"), (2, "Indent = 4", "Indent")] {
            let assigned = backend.evaluate(code, Some(number)).expect("assign");
            assert!(
                !assigned.is_error,
                "`{name}` must stay available to the user: {:?}",
                assigned.transcript
            );
        }
    }

    // JSONRPC traps each method call, and that trap swallows M2's diagnostic
    // before `capture` can write it to the transcript -- an errored evaluation
    // came back carrying only the echoed input. The capture runs in its own task
    // to keep the message.
    #[test]
    fn errors_carry_their_diagnostic() {
        let mut backend = M2Backend::spawn().expect("spawn M2");

        let failed = backend.evaluate("1/0", Some(1)).expect("evaluate");
        assert!(failed.is_error);
        assert!(
            failed.transcript.contains("division by zero"),
            "diagnostic missing from transcript: {:?}",
            failed.transcript
        );

        let message = crate::backend::transcript::captured_error(&failed.transcript, "1/0");
        assert!(
            message.diagnostic.contains("division by zero"),
            "diagnostic lost while cleaning: {:?}",
            message.diagnostic
        );
        assert!(!message.diagnostic.contains("back trace"));
    }

    #[test]
    fn type_info_preserves_multiline_nets() {
        let mut backend = M2Backend::spawn().expect("spawn M2");

        backend
            .evaluate("ZZ[x]/ideal(x-1)", Some(1))
            .expect("create quotient ring");
        backend
            .evaluate("use oo", Some(2))
            .expect("use quotient ring");
        let result = backend.evaluate("x+1", Some(3)).expect("evaluate element");

        assert_eq!(result.rendered_value().as_deref(), Some("2"));
        assert!(
            result
                .type_info
                .as_deref()
                .is_some_and(|ty| ty.lines().count() > 1),
            "expected multiline type, got {:?}",
            result.type_info
        );
    }

    /// A matrix's annotation raises its ranks over the rings: as text the nets join
    /// along their baselines, and as pieces it sets as `Matrix QQ² ⟵ QQ²`.
    #[test]
    fn matrix_types_keep_their_shape() {
        let mut backend = M2Backend::spawn().expect("spawn M2");
        let result = backend
            .evaluate("matrix{{1,2},{3,4/5}}", Some(1))
            .expect("evaluate matrix");

        assert_eq!(
            result.type_info.as_deref(),
            Some("         2       2\nMatrix QQ  <-- QQ")
        );
        let rows = crate::ui::render::maths::annotation(
            &result.type_parts,
            80,
            crate::ui::render::maths::FractionStyle::Stacked,
        );
        let text: String = rows
            .iter()
            .flat_map(|row| row.iter().map(|span| span.content.as_str()))
            .collect();
        let (target, source) = text
            .strip_prefix("Matrix")
            .and_then(|rest| rest.split_once('⟵'))
            .unwrap_or_else(|| panic!("a matrix is typed by its target and source: {text}"));
        assert!(target.contains("QQ") && source.contains("QQ"), "{text}");
    }

    #[test]
    fn ring_values_keep_their_expression_data() {
        let mut backend = M2Backend::spawn().unwrap();
        for code in ["ZZ", "QQ", "RR", "R = QQ[x,y]; R"] {
            let result = backend.evaluate(code, None).unwrap();
            assert!(!result.is_error, "{code}: {}", result.transcript);
            assert!(!result.value.unwrap().classes.is_empty());
        }
    }

    #[test]
    fn polynomial_components_keep_their_expression_and_ring_classes() {
        let mut backend = M2Backend::spawn().unwrap();
        let result = backend.evaluate("R = QQ[x,y]; x^2 + 2*y", None).unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        let value = result.value.unwrap();
        assert!(value.is("Sum"), "{value:?}");
        assert!(value.children[0].is("Power"));
        assert!(value.children[0].children[0].is("Symbol"));
        assert_eq!(
            value.children[0].children[0].group(),
            crate::ui::theme::groups::HLGroup::Variable
        );
    }

    #[test]
    fn precision_fields_retain_their_component_classes() {
        let mut backend = M2Backend::spawn().unwrap();
        for code in ["RR_100", "CC_100"] {
            let result = backend.evaluate(code, None).unwrap();
            assert!(!result.is_error, "{code}: {}", result.transcript);
            assert!(!result.value.unwrap().classes.is_empty());
        }
    }

    #[test]
    fn fields_of_arbitrary_precision_are_drawn_as_the_types_they_are() {
        let mut backend = M2Backend::spawn().unwrap();
        for source in ["RR_*", "CC_*"] {
            let result = backend.evaluate(source, None).unwrap();
            assert!(!result.is_error, "{source}: {}", result.transcript);
            let value = result.value.unwrap();
            assert_eq!(value.group(), crate::ui::theme::groups::HLGroup::Type);
            assert_eq!(value.text.as_deref(), Some(source));
        }
    }

    #[test]
    fn inexact_values_render_without_private_core_helpers() {
        let mut backend = M2Backend::spawn().expect("spawn M2");
        let result = backend.evaluate("1.3", Some(1)).expect("evaluate real");

        assert!(!result.is_error, "{}", result.transcript);
        assert_eq!(result.rendered_value().as_deref(), Some("1.3"));
        assert!(result.has_value);
    }

    #[test]
    fn inexact_values_respect_printing_accuracy() {
        let mut backend = M2Backend::spawn().expect("spawn M2");
        backend
            .evaluate("printingAccuracy = 3;", Some(1))
            .expect("set accuracy");
        let result = backend
            .evaluate("1.23456789", Some(2))
            .expect("evaluate real");

        assert!(!result.is_error, "{}", result.transcript);
        assert_eq!(result.rendered_value().as_deref(), Some("1.235"));
    }

    #[test]
    fn user_package_paths_are_available() {
        let mut backend = M2Backend::spawn().expect("spawn M2");
        let response = backend
            .evaluate(
                r#"assert isMember(toAbsolutePath(applicationDirectory() | "code/"), apply(path, toAbsolutePath)); assert isMember(toAbsolutePath(applicationDirectory() | "local/"), apply(prefixPath, toAbsolutePath));"#,
                None,
            )
            .expect("check user package paths");

        assert!(!response.is_error, "{}", response.transcript);
    }

    #[test]
    fn configured_startup_expressions_and_runtime_flags_survive_restart() {
        use clap::Parser;
        let options = crate::cli::Options::try_parse_from([
            "sp2",
            "-q",
            "--no-preload",
            "--no-debug",
            "--no-backtrace",
            "--no-randomize",
            "--no-threads",
            "--no-time",
            "--print-width",
            "73",
            "-E",
            "print \"before initialization probe\"",
            "-e",
            "spectralStartupProbe = 41",
            "-e",
            "spectralStartupProbe = spectralStartupProbe + 1",
        ])
        .unwrap();
        let mut backend = M2Backend::spawn_with_options(&options.m2).expect("configured M2");
        for generation in 0..2 {
            let output = backend.take_startup_output();
            assert!(output.contains("before initialization probe"), "{output}");
            assert!(!output.contains("error:"), "{output}");
            let response = backend
                .evaluate(
                    "assert(spectralStartupProbe == 42); assert(printWidth == 73);",
                    None,
                )
                .unwrap();
            assert!(!response.is_error, "{}", response.transcript);
            if generation == 0 {
                backend.evaluate("spectralStartupProbe = 0;", None).unwrap();
                backend = backend.restarted().unwrap();
            }
        }
    }

    #[test]
    fn writing_after_process_exit_reports_exit_instead_of_broken_pipe() {
        let mut backend = M2Backend::spawn().unwrap();
        backend.signal_group(Signal::SIGKILL).unwrap();
        backend.process.wait().unwrap();
        assert!(matches!(
            backend.evaluate("1 + 1", None),
            Err(Error::Exited(_))
        ));
    }

    #[test]
    fn init_file_is_loaded() {
        let base = ENV
            .temp_dir()
            .join(format!("spectral-m2-init-test-{}", std::process::id()));
        let config = base.join("spectral-m2");
        fs::create_dir_all(&config).expect("create temp config dir");
        let init_file = config.join("init.m2");
        fs::write(
            &init_file,
            "spectralInitProbe = 1729\nArray.typeDepth = 2\nprint \"initialization probe loaded\"\n",
        )
        .expect("write init.m2");

        let mut be = M2Backend::spawn_with_init_file(Some(init_file)).expect("spawn M2");
        assert!(be
            .take_startup_output()
            .contains("initialization probe loaded"));
        assert!(be.take_startup_output().is_empty());
        let out = be
            .evaluate("spectralInitProbe", Some(1))
            .expect("evaluate probe");

        let configured = be
            .evaluate("[1,2]", Some(2))
            .expect("evaluate configured type");
        assert!(!configured.is_error, "{}", configured.transcript);
        assert_eq!(configured.type_parts[0].render(), "Array[ZZ, ZZ]");

        let _ = fs::remove_dir_all(&base);

        assert!(!out.is_error, "probe errored: {:?}", out.transcript);
        assert!(
            out.rendered_value().as_deref() == Some("1729"),
            "expected TeX-formatted 1 729 in {:?}",
            out.value
        );
    }

    /// A statement that throws must report itself as an error, because that flag is
    /// the only thing stopping the cells queued behind it from running.
    #[test]
    fn a_throwing_statement_reports_an_error() {
        let mut backend = M2Backend::spawn().expect("spawn M2");

        let suppressed = backend.evaluate("1/0;", None).expect("evaluate");
        let bare = backend.evaluate("1/0", None).expect("evaluate");
        let fine = backend.evaluate("1", None).expect("evaluate");

        assert!(
            suppressed.is_error,
            "`1/0;` reported success: number {}, transcript {:?}",
            suppressed.number, suppressed.transcript
        );
        assert!(
            bare.is_error,
            "`1/0` reported success: number {}, transcript {:?}",
            bare.number, bare.transcript
        );
        assert!(!fine.is_error, "`1` reported an error");
    }

    #[test]
    fn errors_in_user_functions_return_to_the_same_session() {
        let mut backend = M2Backend::spawn().expect("spawn M2");
        let setup = backend
            .evaluate(
                "savedBeforeFailure := 41; failingFunction = () -> error \"expected failure\";",
                None,
            )
            .expect("define function");
        assert!(!setup.is_error);
        for _ in 0..2 {
            let failed = backend
                .evaluate("failingFunction()", None)
                .expect("evaluate error");
            assert!(failed.is_error, "{}", failed.transcript);
            let recovered = backend
                .evaluate("savedBeforeFailure + 1", None)
                .expect("evaluate after error");
            assert!(!recovered.is_error, "{}", recovered.transcript);
            assert_eq!(recovered.depth, setup.depth);
            assert!(recovered
                .rendered_value()
                .is_some_and(|value| value.contains("42")));
        }
    }

    /// `capture`, which `installPackage` uses to run examples, evaluates through
    /// the shell's mode hooks; a statement it completes must neither stand in
    /// for the cell that called it nor lose the values M2 prints into its output.
    #[test]
    fn nested_capture_is_not_the_cell() {
        let mut backend = M2Backend::spawn().expect("spawn M2");

        let thrown = backend
            .evaluate("(capture \"1+1\"; 1/0)", None)
            .expect("evaluate");
        assert!(
            thrown.is_error,
            "a cell throwing after `capture` reported success: {:?}",
            thrown.transcript
        );

        let captured = backend
            .evaluate("last capture \"1+1\"", None)
            .expect("evaluate");
        assert!(!captured.is_error, "{}", captured.transcript);
        let value = captured.value.as_ref().map(|value| format!("{value:?}"));
        assert!(
            value.as_deref().is_some_and(|value| value.contains("= 2")),
            "captured output lost its value: {value:?}"
        );
    }

    #[test]
    fn startup_source_remains_inspectable() {
        let mut backend = M2Backend::spawn().expect("spawn M2");
        let config = M2Backend::config_dir().expect("config directory");
        for source in [STARTUP_SCRIPT, DISPLAY_PACKAGE] {
            assert_eq!(
                fs::read_to_string(config.join(source.file_name)).expect("read staged source"),
                source.contents
            );
        }

        for code in [
            "code Thing#{SpectralTopLevelMode, BeforePrint}",
            "code(specNet, Thing)",
        ] {
            let inspected = backend.evaluate(code, None).expect("inspect source");
            assert!(!inspected.is_error, "{code}: {}", inspected.transcript);
        }
    }

    #[test]
    fn a_custom_type_composes_expression_shapes_without_renderer_changes() {
        let mut backend = M2Backend::spawn().unwrap();
        let result = backend.evaluate("Pair = new Type of HashTable; expression Pair := pair -> RowExpression{pair#first, pair#last}; new Pair from {first => \"a\", last => symbol a}", None).unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        let value = result.value.unwrap();
        assert!(value.is("RowExpression"));
        assert!(value.children[0].is("String"));
        assert!(value.children[1].is("Symbol"));
        assert_eq!(value.render(), "aa");
    }

    #[test]
    fn expression_nodes_transmit_their_original_classes() {
        let mut backend = M2Backend::spawn().unwrap();
        for (source, class) in [
            ("expression(ZZ)", "Ring"),
            ("1/2", "Divide"),
            (
                "MatrixExpression{{symbol a,1/2},{[1],}}",
                "MatrixExpression",
            ),
        ] {
            let result = backend.evaluate(source, None).unwrap();
            assert!(!result.is_error, "{source}: {}", result.transcript);
            assert!(result.value.unwrap().is(class));
        }
    }

    #[test]
    fn a_failed_formatter_uses_the_opaque_fallback() {
        let mut backend = M2Backend::spawn().unwrap();
        let result = backend
            .evaluate(
                "Broken = new Type of HashTable; specNet Broken := x -> 1; new Broken from {}",
                None,
            )
            .unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        assert!(result.value.unwrap().is("Broken"));
        let after = backend.evaluate("1+1", None).unwrap();
        assert!(!after.is_error);
        assert_eq!(after.rendered_value().as_deref(), Some("2"));
    }

    #[test]
    fn custom_classes_keep_ancestry_and_unknown_expressions_keep_children() {
        let mut backend = M2Backend::spawn().unwrap();
        let result = backend
            .evaluate("Row = new Type of Array; new Row from {1,2}", None)
            .unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        let value = result.value.unwrap();
        assert_eq!(value.class(), "Row");
        assert!(value.is("Array"));
        assert_eq!(value.render(), "[1, 2]");
        let result = backend
            .evaluate(
                "Unfamiliar = new Type of Expression; new Unfamiliar from {\"a\", symbol a}",
                None,
            )
            .unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        let value = result.value.unwrap();
        assert_eq!(value.class(), "Unfamiliar");
        assert!(value.render().starts_with("Unfamiliar{"));
        assert!(value.children[0].is("String"));
    }

    #[test]
    fn speek_spells_out_what_a_value_is_drawn_as() {
        let mut backend = M2Backend::spawn().unwrap();
        let result = backend.evaluate("speek {1, symbol a}", None).unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        let value = result.value.unwrap();
        assert!(value.is("Option"));
        assert!(crate::ui::render::maths::Math::supports_spec_net(&value));
        assert!(value.render().contains("List"));
        let truncated = backend.evaluate("speek'(0, {1, symbol a})", None).unwrap();
        assert!(!truncated.is_error, "{}", truncated.transcript);
        let value = truncated.value.unwrap();
        assert!(value.is("Option"));
        assert_eq!(value.children[1].text.as_deref(), Some("..."));
    }

    #[test]
    fn explicit_type_depth_and_custom_annotations_are_independent() {
        let mut backend = M2Backend::spawn().unwrap();
        for (source, expected) in [
            ("[1, 1.]", "Array[ZZ, RR]"),
            ("[]", "Array[]"),
            ("[1, 2]", "Array[ZZ, ZZ]"),
        ] {
            let result = backend.evaluate(source, None).unwrap();
            assert!(!result.is_error, "{source}: {}", result.transcript);
            assert_eq!(result.type_parts[0].render(), "Array");
            let expanded = backend
                .evaluate(&format!("specType'(2, {source})"), None)
                .unwrap();
            assert!(!expanded.is_error, "{}", expanded.transcript);
            assert_eq!(expanded.value.unwrap().render(), expected);
        }
        let result = backend.evaluate("Sample = new Type of HashTable; expression Sample := x -> RowExpression{symbol u, \"u\"}; specType' (ZZ, Sample) := (depth, x) -> specNet symbol SampleType; new Sample from {}", None).unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        let value = result.value.unwrap();
        assert!(value.children[0].is("Symbol"));
        assert!(value.children[1].is("String"));
        assert_eq!(result.type_parts[0].text.as_deref(), Some("SampleType"));
    }

    #[test]
    fn nested_expressions_keep_string_and_symbol_classes() {
        let mut backend = M2Backend::spawn().unwrap();
        let result = backend
            .evaluate(
                r#"VectorExpression {(expression A)_A_A^3, (expression "a")_a^(a_"a")}"#,
                None,
            )
            .unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        let value = result.value.unwrap();
        assert!(value.is("VectorExpression"));
        let second = &value.children[1];
        assert!(second.is("Power"));
        assert!(second.children[0].is("Subscript"));
        let base = &second.children[0].children[0];
        assert!(base.is("String"), "{base:?}");
        assert_eq!(base.text.as_deref(), Some("a"));
        assert!(second.children[0].children[1].is("Symbol"));
        assert!(second.children[1].children[1].is("String"));
    }

    #[test]
    fn suppressed_values_only_run_the_type_formatter() {
        let mut backend = M2Backend::spawn().unwrap();
        let result = backend.evaluate("Suppressed = new Type of HashTable; specNet Suppressed := x -> error \"value formatter called\"; specType' (ZZ, Suppressed) := (depth, x) -> specNet symbol SuppressedType; new Suppressed from {};", None).unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        assert!(result.value.is_none());
        assert_eq!(result.type_parts[0].render(), "SuppressedType");
    }

    #[test]
    fn formatter_errors_clear_active_traversal_and_leave_the_session_usable() {
        let mut backend = M2Backend::spawn().unwrap();
        let result = backend.evaluate("Broken = new Type of HashTable; specNet Broken := x -> error \"broken formatting\"; {new Broken from {}}", None).unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        let result = backend.evaluate("{1,2}", None).unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        assert_eq!(result.value.unwrap().render(), "{1, 2}");
        let after = backend.evaluate("7", None).unwrap();
        assert_eq!(after.rendered_value().as_deref(), Some("7"));
    }

    #[test]
    fn a_container_subclass_can_customize_its_formatting() {
        let mut backend = M2Backend::spawn().unwrap();
        let result = backend.evaluate("Framed = new Type of Array; specNet Framed := x -> specNet Power{x#0,x#1}; new Framed from {symbol a,3}", None).unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        assert!(result.value.unwrap().is("Power"));
    }

    #[test]
    fn value_depth_elides_unnamed_structures_and_keeps_names() {
        let mut backend = M2Backend::spawn().unwrap();
        let mut drawn = |source: &str| {
            let result = backend.evaluate(source, None).unwrap();
            assert!(!result.is_error, "{source}: {}", result.transcript);
            result.value.unwrap().render()
        };
        let unnamed = "OO_(Proj(QQ[a,b]))^2";
        let shallow = drawn(&format!("specNet'(1, {unnamed})"));
        let deep = drawn(&format!("specNet'(2, {unnamed})"));
        assert!(!shallow.contains("Proj"), "{shallow}");
        assert!(deep.contains("Proj"), "{deep}");
        let named = drawn("Y = Proj(QQ[a,b]); specNet'(1, OO_Y^2)");
        assert!(named.contains('Y'), "{named}");
        let rank_one = drawn("OO_Y^1");
        assert_eq!(rank_one, drawn("OO_Y"), "rank one is an index, not shown");
        assert_ne!(
            drawn("specType'(2, OO_Y^1)"),
            drawn("specType'(2, OO_Y)"),
            "a sheaf of rings and a sheaf of modules are different types"
        );
        let elements = drawn("specNet'(0, {a^2 - 1})");
        assert!(elements.contains('a'), "{elements}");

        let sheaf = |depth: u32| format!("specType'({depth}, OO_(Proj(QQ[a,b]))^2)");
        let over_rings = drawn(&sheaf(2));
        let over_variety = drawn(&sheaf(3));
        assert!(
            over_rings.contains("OO") && !over_rings.contains("Proj"),
            "{over_rings}"
        );
        assert!(over_variety.contains("Proj"), "{over_variety}");

        let complex = |depth: u32| {
            format!("specNet'({depth}, res(coker vars (QQ[w,x]/(w^3)), LengthLimit => 1))")
        };
        let named = drawn(&complex(1));
        let modules = drawn(&complex(2));
        let quotient = drawn(&complex(3));
        let rings = drawn(&complex(4));
        assert!(named.contains("C") && named.contains('d'), "{named}");
        assert!(!named.contains('w'), "{named}");
        assert!(modules.contains('w') && !modules.contains("C"), "{modules}");
        assert!(!modules.contains("QQ"), "{modules}");
        assert!(
            quotient != modules && !quotient.contains("QQ"),
            "{quotient}"
        );
        assert!(rings.contains("QQ"), "{rings}");

        let typed = |depth: u32| format!("specType'({depth}, (ZZ/7[t])^2)");
        let shallow = drawn(&typed(2));
        let deep = drawn(&typed(3));
        assert!(!shallow.contains('t'), "{shallow}");
        assert!(deep.contains('t'), "{deep}");
    }

    #[test]
    fn type_depth_parameters_control_annotations_and_inherit_through_types() {
        let mut backend = M2Backend::spawn().unwrap();
        for (source, expected) in [
            ("Array.typeDepth = 1; [1,2]", "Array"),
            ("Array.typeDepth = 2; [1,2]", "Array[ZZ, ZZ]"),
            ("Row = new Type of Array; new Row from {1,2}", "Row[ZZ, ZZ]"),
            ("Row.typeDepth = 1; new Row from {1,2}", "Row"),
            ("Module.typeDepth = 2; R = QQ[x,y]; R^3", "Module[R]"),
            ("Module.typeDepth = 1; R^3", "Module"),
            (
                "X = Proj R; SheafOfRings.typeDepth = 2; OO_X",
                "SheafOfRings[X]",
            ),
            (
                "CoherentSheaf.typeDepth = 2; sheaf(R^{-4})",
                "CoherentSheaf[OO_X]",
            ),
            (
                "SheafMap.typeDepth = 3; sheaf map(R^1,R^1,x)",
                "SheafMap[OO_X ⟵ OO_X]",
            ),
            ("Matrix.typeDepth = 3; vars R", "Matrix[R ⟵ R²]"),
            ("Matrix.typeDepth = 2; vars R", "Matrix[M ⟵ M]"),
        ] {
            let result = backend.evaluate(source, None).unwrap();
            assert!(!result.is_error, "{source}: {}", result.transcript);
            assert_eq!(result.type_parts[0].render(), expected, "{source}");
        }
        let explicit = backend
            .evaluate("Array.typeDepth = 1; specType'(2, [1,2])", None)
            .unwrap();
        assert!(!explicit.is_error, "{}", explicit.transcript);
        assert_eq!(explicit.value.unwrap().render(), "Array[ZZ, ZZ]");
        for invalid in ["-1", "1/2", "\"deep\""] {
            let result = backend
                .evaluate(
                    &format!("Array.typeDepth = {invalid}; try (specType([1]); false) else true"),
                    None,
                )
                .unwrap();
            assert_eq!(
                result.value.unwrap().render(),
                "true",
                "invalid depth {invalid} was accepted"
            );
        }
    }

    #[test]
    fn inductive_types_fold_to_the_requested_depth() {
        let mut backend = M2Backend::spawn().unwrap();
        for (depth, expected) in [
            (0, "Array"),
            (1, "Array"),
            (2, "Array[Array, Array]"),
            (3, "Array[Array[ZZ, RR], Array[String]]"),
        ] {
            let source = format!(r#"specType'({depth}, [[1,1.],["a"]])"#);
            let result = backend.evaluate(&source, None).unwrap();
            assert!(!result.is_error, "{source}: {}", result.transcript);
            assert_eq!(result.value.unwrap().render(), expected);
        }
        let result = backend
            .evaluate("specType'(2, Power{symbol x,3})", None)
            .unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        assert_eq!(result.value.unwrap().render(), "Power[Symbol, ZZ]");
        for depth in ["1/2", "\"deep\""] {
            let result = backend
                .evaluate(&format!("specType'({depth}, [1])"), None)
                .unwrap();
            assert!(result.is_error, "invalid depth {depth} was accepted");
        }
        let result = backend.evaluate("[1,1.]", None).unwrap();
        assert!(!result.is_error);
        assert_eq!(result.type_parts[0].render(), "Array");
    }

    #[test]
    fn existing_renderers_accept_the_supported_expression_constructors() {
        let mut backend = M2Backend::spawn().unwrap();
        let mixed = backend.evaluate(r#"[1/2,"1"]"#, None).unwrap();
        assert!(!mixed.is_error, "{}", mixed.transcript);
        let mixed = mixed.value.unwrap();
        assert!(mixed.children[0].is("Divide"));
        assert!(mixed.children[1].is("String"));
        assert_eq!(mixed.children[1].text.as_deref(), Some("1"));
        for (source, class) in [
            (
                "MatrixExpression{{symbol A,11},{symbol B,13}}",
                "MatrixExpression",
            ),
            ("VectorExpression{symbol A,11}", "VectorExpression"),
            ("Table{{symbol A,11},{symbol B,13}}", "Table"),
            ("MapExpression{symbol A,symbol B,1/2}", "MapExpression"),
            ("BinaryOperation{symbol +,symbol A,11}", "BinaryOperation"),
            ("Adjacent{symbol A,symbol B}", "Adjacent"),
            ("Subscript{symbol A,11}", "Subscript"),
            ("Superscript{symbol A,11}", "Superscript"),
            ("symbol c_1", "Subscript"),
            ("c_1", "Subscript"),
        ] {
            let result = backend.evaluate(source, None).unwrap();
            assert!(!result.is_error, "{source}: {}", result.transcript);
            let value = result.value.unwrap();
            assert!(value.is(class), "{source}: {value:?}");
            let rows = crate::ui::render::maths::spans(
                &value,
                80,
                crate::ui::render::maths::MathStyle::default(),
            )
            .unwrap();
            assert!(!rows.is_empty(), "{source}");
            assert!(
                !value.render().contains("Expression{"),
                "{source}: {}",
                value.render()
            );
        }
    }

    #[test]
    fn complexes_compose_maps_with_differentials_and_degree_labels() {
        let mut backend = M2Backend::spawn().unwrap();
        // The chain of a complex, as its modules' degrees, between zeros.
        let degrees_of = |value: &SpecNet| {
            let is_zero = |drawn: &SpecNet| drawn.render() == "0";
            let [chain, end] = value.children.as_slice() else {
                panic!("a complex ends in zero: {value:?}");
            };
            assert!(value.is("MapExpression") && is_zero(end), "{value:?}");
            let mut target = chain;
            let mut degrees = Vec::new();
            loop {
                match target.children.as_slice() {
                    [next, module, differential] => {
                        assert!(differential.is("Array"));
                        assert!(differential.children[0].is("Table"));
                        degrees.push(module);
                        target = next;
                    }
                    [start, module] => {
                        assert!(is_zero(start), "{target:?}");
                        degrees.push(module);
                        break;
                    }
                    _ => panic!("incomplete differential: {target:?}"),
                }
            }
            degrees
                .into_iter()
                .map(|module| {
                    assert!(module.is("UnderExpression"), "{module:?}");
                    module.children[1].render().parse::<i32>().unwrap()
                })
                .collect::<Vec<_>>()
        };
        for source in ["S = QQ[a_1..a_4]; C = res coker vars S; C", "C[2]"] {
            let result = backend.evaluate(source, None).unwrap();
            assert!(!result.is_error, "{source}: {}", result.transcript);
            let value = result.value.unwrap();
            assert!(
                crate::ui::render::maths::Math::supports_spec_net(&value),
                "{value:?}"
            );
            let degrees = degrees_of(&value);
            assert_eq!(degrees.len(), 5);
            assert!(degrees.windows(2).all(|pair| pair[0] - pair[1] == 1));
        }
        let result = backend.evaluate("complex S^1", None).unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        assert_eq!(degrees_of(&result.value.unwrap()).len(), 1);
        let result = backend.evaluate("complex S^0", None).unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        assert_eq!(result.value.unwrap().render(), "0");
    }

    #[test]
    fn operator_attributes_keep_nested_tables_structured() {
        let mut backend = M2Backend::spawn().unwrap();
        let result = backend.evaluate("operatorAttributes", None).unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        let table = result.value.unwrap();
        assert!(table.is("HashTable"));
        assert!(crate::ui::render::maths::Math::supports_spec_net(&table));
        assert!(table.text.is_none());
        assert!(!table_rows(&table).is_empty());
        for entry in table_rows(&table) {
            let [key, _, attributes] = entry.children.as_slice() else {
                panic!("table entry lost its key or value: {entry:?}");
            };
            assert!(key.is("Keyword"));
            assert!(attributes.is("HashTable"));
            assert!(attributes.text.is_none());
            assert!(!table_rows(attributes).is_empty());
        }
        let rows = crate::ui::render::maths::spans(
            &table,
            120,
            crate::ui::render::maths::MathStyle::default(),
        )
        .unwrap();
        assert!(!rows.is_empty());
        assert!(rows
            .iter()
            .flatten()
            .any(|span| span.kind == table_rows(&table)[0].children[0].group()));
    }

    #[test]
    fn m2_marks_only_mathematical_parts_as_typeset() {
        let mut backend = M2Backend::spawn().unwrap();
        let mut value = |source: &str| {
            let result = backend.evaluate(source, None).unwrap();
            assert!(!result.is_error, "{source}: {}", result.transcript);
            result.value.unwrap()
        };
        for plain in ["loadedPackages", r#"{1, "a", symbol b}"#, r#""text""#] {
            assert!(!value(plain).wants_typesetting(), "{plain}");
        }
        for maths in ["matrix{{1, 2}}", "1/2", "QQ[t]; t^2 + 1"] {
            assert!(value(maths).wants_typesetting(), "{maths}");
        }
        let mixed = value(r#"{matrix{{1}}, "beside"}"#);
        assert!(mixed.children[0].typeset);
        assert!(mixed.children[0].children.iter().all(|row| row.typeset));
        assert!(!mixed.children[1].typeset);
    }

    #[test]
    fn a_named_key_before_an_arrow_is_coloured_as_an_option_name() {
        let mut backend = M2Backend::spawn().unwrap();
        let result = backend.evaluate("describe(ZZ[x,y])", None).unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        fn keys(node: &SpecNet, found: &mut Vec<HLGroup>) {
            if node.is("BinaryOperation")
                && node
                    .children
                    .first()
                    .and_then(|arrow| arrow.text.as_deref())
                    == Some("=>")
            {
                found.push(node.children[1].group());
            }
            node.children.iter().for_each(|child| keys(child, found));
        }
        let mut found = Vec::new();
        keys(&result.value.unwrap(), &mut found);
        assert!(!found.is_empty());
        assert!(
            found.iter().all(|group| *group == HLGroup::Option),
            "{found:?}"
        );
    }

    #[test]
    fn tables_keep_pairs_and_default_type_depth_keeps_annotations_short() {
        let mut backend = M2Backend::spawn().unwrap();
        for source in [
            r#"hashTable{a=>1/2,b=>"1"}"#,
            r#"new MutableHashTable from {a=>1/2,b=>"1"}"#,
        ] {
            let result = backend.evaluate(source, None).unwrap();
            assert!(!result.is_error, "{source}: {}", result.transcript);
            let table = result.value.unwrap();
            let rows = table_rows(&table);
            assert_eq!(rows.len(), 2);
            assert!(rows.iter().any(|entry| entry.children[2].is("Divide")));
            assert!(rows.iter().any(|entry| entry.children[2].is("String")));
            assert!(table.render().contains("=>"));
        }
        let result = backend.evaluate("toList(200:symbol if)", None).unwrap();
        assert!(!result.is_error, "{}", result.transcript);
        assert_eq!(result.value.unwrap().children.len(), 200);
        assert_eq!(result.type_parts[0].render(), "List");
    }
}
