//! Macaulay2 process management, protocol types, and transcript decoding.

pub mod preview;
pub mod rod;
pub mod transcript;

pub use preview::{PreviewMode, ValuePreview};
pub use rod::Rod;
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
/// Where the shell writes the `Spectral` package it carries, under its config,
/// and the package's main file there.
const PACKAGE_DIRECTORY: &str = "package";
const PACKAGE_MAIN: &str = "Spectral.m2";
/// Loads the `Spectral` package, which defines `Rod` and `rod` that every result
/// is drawn from: the user's installed copy when it is as recent as the carried
/// one, else the carried one, installing it for the user's M2 meanwhile.
const PACKAGE_LOADER: EmbeddedSource = EmbeddedSource {
    file_name: "load.m2",
    contents: include_str!("backend/m2_spectral_package.m2"),
};
/// The pinned `Spectral` submodule's source files, as `build.rs` lists them.
const SPECTRAL_SOURCES: &[EmbeddedSource] =
    &include!(concat!(env!("OUT_DIR"), "/spectral_package.rs"));
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

/// An M2 source file the shell carries inside itself and hands M2 by path,
/// named by its path relative to the directory it is written into.
#[derive(Clone, Copy, Debug)]
struct EmbeddedSource {
    file_name: &'static str,
    contents: &'static str,
}

impl EmbeddedSource {
    /// Materialize the file under `config` at a stable path, so source positions
    /// in it stay readable. Replacement through a same-directory staging file
    /// keeps parallel launches atomic.
    fn sync_into(self, directory: &Path) -> Result<PathBuf> {
        let path = directory.join(self.file_name);
        let parent = path.parent().unwrap_or(directory);
        fs::create_dir_all(parent)?;
        match fs::read(&path) {
            Ok(contents) if contents == self.contents.as_bytes() => return Ok(path),
            _ => {}
        }
        // Unique per call, not merely per process: several backends may be
        // starting at once (the test suite does exactly that), and a shared
        // staging name means one launch renames the file out from under another.
        static STAGE: AtomicU32 = AtomicU32::new(0);
        let staged = parent.join(format!(
            ".{}-{}-{}",
            path.file_name().unwrap_or_default().display(),
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
    /// The version of the Spectral package loaded, on the startup frame.
    #[serde(default)]
    spectral: String,
    /// The input number M2 gave this statement, absent on the startup frame.
    #[serde(default)]
    number: u32,
    /// The submission this frame answers, absent on the startup frame.
    #[serde(default)]
    submission: u64,

    #[serde(default)]
    error: bool,

    value: Option<Rod>,
    inspection: Option<Rod>,
    #[serde(default)]
    previews: Vec<ValuePreview>,
    type_info: Option<String>,
    type_drawing: Option<Rod>,
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
    pub value: Option<Rod>,
    /// The value inspected as `peek` inspects it, as Spectral's `speek` draws it.
    pub inspection: Option<Rod>,
    /// Independent M2 formatter results available to the preview selector.
    pub previews: Vec<ValuePreview>,
    pub type_info: Option<String>,
    /// The type annotation as Spectral draws it.
    pub type_drawing: Option<Rod>,
    /// Whether M2 bound an `oN` for this input. Not implied by `value`: a
    /// suppressed result binds one without producing rendered text.
    pub has_value: bool,
    /// M2's interpreter depth after the evaluation. Anything above 1 means a
    /// nested interpreter, which is what being in the debugger looks like.
    pub depth: u32,
    /// Names the statement bound that M2 had not reported before.
    pub names: Vec<String>,
}

impl M2Response {}

/// The versions of what a session runs, as its startup frame reports them.
#[derive(Clone, Debug, Default)]
pub struct Versions {
    pub m2: String,
    /// The `Spectral` package loaded, the user's own installed copy.
    pub spectral: String,
}

/// What M2 announces on becoming ready: its versions, every name it started
/// with, and what it printed on the way.
#[derive(Clone, Debug, Default)]
pub struct Announcement {
    pub versions: Versions,
    pub names: Vec<String>,
    pub startup_output: String,
}

/// The live Macaulay2 child, the stdin it reads its session from, and the
/// demultiplexed stream it answers on.
pub struct M2Backend {
    process: Child,
    input: ChildStdin,
    chunks: Receiver<Chunk>,
    /// What the startup frame announced; its names and printed text until taken.
    announcement: Announcement,
    startup_options: M2Options,
    /// Number of the latest submission; its frame is the only one that closes it.
    submission: u64,
}

impl M2Backend {
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

    /// Start M2 and let its own top level read the session from stdin.
    ///
    /// The startup script is loaded with `-e` *before* the top level begins, so
    /// the hooks are installed by the time the first cell can arrive. Deliberately
    /// not `--script`, which would evaluate the file and exit rather than going on
    /// to read stdin. Normal M2 initialization is preserved unless `-q` is
    /// requested, which also removes home-directory package paths. Spectral's
    /// own `init.m2` runs after M2's initialization and after the backend hooks.
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
        let carried = config.join(PACKAGE_DIRECTORY);
        for source in SPECTRAL_SOURCES {
            source.sync_into(&carried)?;
        }
        let package_path = PACKAGE_LOADER.sync_into(&carried)?;
        let script_path = STARTUP_SCRIPT.sync_into(config)?;
        let (reader, writer) = std::io::pipe()?;

        let mut command = Command::new("M2");
        command.arg("--silent").arg("--no-readline");
        command.args(options.arguments());
        // The display package comes first, so the init file and a requested
        // package can already install `rod` for classes of their own.
        command.arg("-e").arg(format!(
            "load {}",
            m2_string_literal(package_path.display())
        ));
        // Spectral's init file is loaded by the backend itself, once the hooks it
        // may edit are installed; it is only told where the file is.
        if let Some(init_file) = init_file.as_deref().filter(|path| path.is_file()) {
            command.arg("-e").arg(format!(
                r#"User#"spectral init file" = {}"#,
                m2_string_literal(init_file.display())
            ));
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
        // The command still holds the pipe's writing ends; only once they are gone
        // does a child dying during startup close the pipe instead of hanging it.
        drop(command);

        let input = process.stdin.take().ok_or_else(|| {
            Error::Process("Macaulay2 was started without a writable stdin".to_string())
        })?;
        let mut backend = Self {
            process,
            input,
            chunks: split_stream(reader),
            announcement: Announcement::default(),
            startup_options: options.clone(),
            submission: 0,
        };
        backend.wait_for_ready()?;
        Ok(backend)
    }

    /// The main file of the `Spectral` package the shell carries, written out.
    pub fn carried_package() -> Option<PathBuf> {
        let directory = Self::config_dir().ok()?.join(PACKAGE_DIRECTORY);
        Some(directory.join(PACKAGE_MAIN))
    }

    /// Spectral's persistent configuration directory.
    fn config_dir() -> Result<PathBuf> {
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
    fn wait_for_ready(&mut self) -> Result<()> {
        let mut said = String::new();
        loop {
            match self.chunks.recv_timeout(STARTUP_TIMEOUT) {
                Ok(Chunk::Text(text)) => said.push_str(&text),
                Ok(Chunk::Frame(frame)) => {
                    let frame: Frame = serde_json::from_str(&frame)?;
                    return match self.adopt_ready(frame, &said) {
                        true => Ok(()),
                        false => Err(Error::Process(
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

    /// Take over what a startup frame announces, with the text printed before it;
    /// false when `frame` is not a startup frame.
    fn adopt_ready(&mut self, frame: Frame, said: &str) -> bool {
        let Some(m2) = frame.ready else {
            return false;
        };
        self.announcement = Announcement {
            versions: Versions {
                m2,
                spectral: frame.spectral,
            },
            names: frame.names,
            startup_output: said.trim().to_string(),
        };
        true
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
                    // A startup frame mid-cell: M2 ran its own `restart`, which
                    // re-executes it in place, on the same process and pipes.
                    if frame.ready.is_some() {
                        self.adopt_ready(frame, &transcript);
                        return Err(Error::Restarted);
                    }
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
                        inspection: frame.inspection,
                        previews: frame.previews,
                        type_info: frame.type_info,
                        type_drawing: frame.type_drawing,
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

    pub fn versions(&self) -> &Versions {
        &self.announcement.versions
    }

    /// What M2 announced on becoming ready, its names and printed text taken:
    /// later names arrive with each response.
    pub fn take_announcement(&mut self) -> Announcement {
        let versions = self.announcement.versions.clone();
        mem::replace(
            &mut self.announcement,
            Announcement {
                versions,
                ..Announcement::default()
            },
        )
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
pub fn m2_string_literal(text: impl Display) -> String {
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
