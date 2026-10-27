//! Macaulay2 process management, protocol types, and transcript decoding.

pub mod transcript;

use std::fmt::Display;
use std::io::{BufRead, BufReader, PipeReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use std::{env, fs, mem, thread};

use crate::cli::M2Options;
use crate::errors::{Error, Result};
use regex::Regex;
use serde::Deserialize;
use term_maths::RenderedBlock;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const EVALUATION_TIMEOUT: Duration = Duration::from_secs(300);
const STARTUP_SCRIPT: &str = include_str!("backend/m2_startup.m2");
const STARTUP_FILE: &str = "backend.m2";
/// Introduces a result frame on the child's output stream. ASCII group
/// separator: no diagnostic, prompt or ordinary value begins with it, and the
/// M2 side writes it immediately before the frame's JSON.
const FRAME_MARK: u8 = 0x1D;

/// Appended to every submission. The statement that closes an input -- see the
/// M2 side for why the hooks cannot do it alone for a statement that errored. It
/// hands over the top level's local dictionaries, which only a statement typed
/// at the top level can reach, so the names bound there are reported too.
const CELL_TERMINATOR: &str = r#"(User#"spectral end of cell")(localDictionaries());"#;

static M2_ARRAY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\begin\{array\}\{[clr]+\}").expect("valid M2 array pattern"));

static M2_MATH_FONT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{\\(mathbb|mathbf|mathcal|mathrm|mathfrak|mathsf|mathtt)\s+([^{}]+)\}")
        .expect("valid M2 math font pattern")
});
/// `\mathit{…}`, which M2 puts around multi-character names only -- `x2` gets it,
/// `x` does not. It is there to stop `x2` being read as *x^2*, so it says nothing
/// the renderer needs and the braces come off with it.
static M2_MATHIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\mathit\{([^{}]*)\}").expect("valid M2 mathit pattern"));

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

/// The frame M2 sends to close one input, or to announce that it is up.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Frame {
    /// The M2 version, present only on the startup frame.
    ready: Option<String>,
    /// The input number M2 gave this statement, absent on the startup frame.
    #[serde(default)]
    number: u32,
    #[serde(default)]
    error: bool,
    value: Option<M2Value>,
    type_info: Option<String>,
    type_parts: Option<Vec<M2Value>>,
    #[serde(default)]
    has_value: bool,
    #[serde(default)]
    depth: u32,
    /// Names M2 had not reported before: all of them on the startup frame.
    #[serde(default)]
    names: Vec<String>,
}

/// A displayed M2 value in its preferred TeX form or its original Net form.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum M2Value {
    /// TeX source accompanied by the Net spelling used when it is unsupported,
    /// and the one-line code M2 reads back, which copying its picture gives.
    Tex {
        source: String,
        fallback: String,
        code: String,
    },
    /// A value for which M2 could not produce TeX.
    Net(String),
    /// Key/value parts composed into one brace-delimited row grid.
    HashTable(Vec<[M2Value; 2]>),
    /// Entries M2 rendered one by one, set inside the delimiters of their kind.
    Collection {
        kind: CollectionKind,
        items: Vec<M2Value>,
    },
    /// Documentation retained as semantic HTML for width-aware rendering, with
    /// the print width its archived examples were wrapped to.
    Markup {
        html: String,
        fallback: String,
        #[serde(rename = "exampleWidth")]
        example_width: usize,
    },
    /// A value that is one word with a role in code: null, a Boolean, a string, a
    /// function's name.
    Token { text: String, role: Role },
}

/// What a token is in code, which decides how it is coloured.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Null,
    Boolean,
    String,
    Function,
    Type,
    Package,
    Keyword,
    Operator,
    Command,
}

/// The characters a value's text layout may draw with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spelling {
    /// Two-dimensional maths and tall brackets.
    Unicode,
    /// Macaulay2's own plain-text spelling, between plain delimiters.
    Ascii,
}

impl Spelling {
    const fn arrow(self) -> &'static str {
        match self {
            Self::Unicode => " ⇒ ",
            Self::Ascii => " => ",
        }
    }
}

/// The M2 list class a collection was drawn as, named exactly as M2 names it.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub enum CollectionKind {
    List,
    Array,
    Sequence,
    AngleBarList,
    /// A List set one entry per row rather than along one line.
    VerticalList,
}

impl CollectionKind {
    /// The opening and closing delimiters M2 draws this kind between.
    pub const fn delimiters(self) -> (&'static str, &'static str) {
        match self {
            Self::List | Self::VerticalList => ("{", "}"),
            Self::Array => ("[", "]"),
            Self::Sequence => ("(", ")"),
            Self::AngleBarList => ("<|", "|>"),
        }
    }
}

impl M2Value {
    fn normalize(source: &str) -> String {
        let source = M2_MATH_FONT.replace_all(source, r"\$1{$2}");
        let source = M2_MATHIT.replace_all(&source, "$1");
        M2_ARRAY
            .replace_all(&source, r"\begin{matrix}")
            .replace(r"\end{array}", r"\end{matrix}")
            .replace(r"\mathopen{}", "")
            .replace(r"\texttt", r"\text")
            .replace(r"\left\{", r"\left{")
            .replace(r"\right\}", r"\right}")
    }

    fn text_block(text: &str) -> RenderedBlock {
        let block = Self::stack(text.lines().map(RenderedBlock::from_text));
        RenderedBlock::new(block.cells().to_vec(), block.height() / 2)
    }

    fn stack(rows: impl IntoIterator<Item = RenderedBlock>) -> RenderedBlock {
        let block = rows.into_iter().fold(RenderedBlock::empty(), |block, row| {
            match block.is_empty() {
                true => row,
                false => RenderedBlock::above(&block, &row, 0),
            }
        });
        RenderedBlock::new(block.cells().to_vec(), block.height() / 2)
    }

    /// A curly brace `height` rows tall. An odd one has its cusp on the middle
    /// row; an even one, having no middle row, splits the cusp across the two
    /// rows about its middle, which at height two is the whole brace.
    fn curly(height: usize, left: bool, baseline: usize) -> RenderedBlock {
        let (single, top, cusp, bottom, upper_cusp, lower_cusp) = match left {
            true => ("{", "⎧", "⎨", "⎩", "⎰", "⎱"),
            false => ("}", "⎫", "⎬", "⎭", "⎱", "⎰"),
        };
        let cells = (0..height)
            .map(|row| {
                vec![match (height % 2, row) {
                    _ if height == 1 => single,
                    (0, row) if row + 1 == height / 2 => upper_cusp,
                    (0, row) if row == height / 2 => lower_cusp,
                    (_, 0) => top,
                    (_, row) if row + 1 == height => bottom,
                    (1, row) if row == height / 2 => cusp,
                    _ => "⎪",
                }
                .to_string()]
            })
            .collect();
        RenderedBlock::new(cells, baseline)
    }

    fn brace(block: RenderedBlock, spelling: Spelling) -> RenderedBlock {
        match (block.is_empty(), spelling) {
            (true, _) => RenderedBlock::from_text("{}"),
            (false, Spelling::Ascii) => RenderedBlock::from_text("{")
                .beside(&block)
                .beside(&RenderedBlock::from_text("}")),
            (false, Spelling::Unicode) => {
                let space = RenderedBlock::from_text(" ");
                let body = space.beside(&block).beside(&space);
                let left = Self::curly(body.height(), true, body.baseline());
                let right = Self::curly(body.height(), false, body.baseline());
                left.beside(&body).beside(&right)
            }
        }
    }

    fn block(&self, spelling: Spelling) -> RenderedBlock {
        match self {
            Self::Tex { fallback, .. } if spelling == Spelling::Ascii => Self::text_block(fallback),
            Self::Tex {
                source, fallback, ..
            } => {
                let source = match source
                    .strip_prefix('$')
                    .and_then(|tex| tex.strip_suffix('$'))
                {
                    Some(source) => source,
                    None => return Self::text_block(fallback),
                };
                let rendered = term_maths::render(&Self::normalize(source));
                match rendered.to_string().contains('\\') {
                    true => Self::text_block(fallback),
                    false => rendered,
                }
            }
            Self::Net(text) | Self::Markup { fallback: text, .. } | Self::Token { text, .. } => {
                Self::text_block(text)
            }
            Self::HashTable(rows) => Self::brace(
                Self::stack(rows.iter().map(|row| {
                    row[0]
                        .block(spelling)
                        .beside(&RenderedBlock::from_text(spelling.arrow()))
                        .beside(&row[1].block(spelling))
                })),
                spelling,
            ),
            Self::Collection {
                kind: CollectionKind::VerticalList,
                items,
            } => Self::brace(
                Self::stack(items.iter().map(|item| item.block(spelling))),
                spelling,
            ),
            Self::Collection { kind, items } => {
                let (open, close) = kind.delimiters();
                let separator = RenderedBlock::from_text(", ");
                items
                    .iter()
                    .map(|item| item.block(spelling))
                    .reduce(|entries, item| entries.beside(&separator).beside(&item))
                    .map_or_else(
                        || RenderedBlock::from_text(&format!("{open}{close}")),
                        |entries| {
                            RenderedBlock::from_text(open)
                                .beside(&entries)
                                .beside(&RenderedBlock::from_text(close))
                        },
                    )
            }
        }
    }

    /// Render this value at a requested terminal width.
    pub fn render_at(&self, width: usize) -> String {
        match self {
            Self::Markup { html, fallback, .. } => html2text::config::plain()
                .link_footnotes(false)
                .string_from_read(html.as_bytes(), width.max(1))
                .unwrap_or_else(|_| fallback.clone())
                .trim_end()
                .to_string(),
            _ => self.render(),
        }
    }

    /// Render this value as copyable terminal text.
    pub fn render(&self) -> String {
        // Flat text already has its layout. Stacking it into a character grid
        // repeatedly copies all earlier rows and is quadratic for long Nets.
        if let Some(text) = self.plain_text() {
            return text
                .lines()
                .map(str::trim_end)
                .collect::<Vec<_>>()
                .join("\n");
        }
        self.block(Spelling::Unicode)
            .to_string()
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
    }
    /// Borrow flat output without parsing TeX or allocating a character grid.
    /// The value in plain ASCII: Macaulay2's own spelling of every piece, set
    /// between plain delimiters.
    pub fn ascii(&self) -> String {
        self.block(Spelling::Ascii)
            .to_string()
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn plain_text(&self) -> Option<&str> {
        match self {
            Self::Net(text) | Self::Markup { fallback: text, .. } | Self::Token { text, .. } => {
                Some(text.as_str())
            }
            Self::Tex {
                source, fallback, ..
            } if !(source.starts_with('$') && source.ends_with('$')) => Some(fallback.as_str()),
            _ => None,
        }
    }
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
    pub value: Option<M2Value>,
    pub type_info: Option<String>,
    /// The pieces of the type annotation, each rendered as a value is.
    pub type_parts: Vec<M2Value>,
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
        self.value.as_ref().map(M2Value::render)
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
        let script_path = Self::sync_startup_script(config)?;
        let (reader, writer) = std::io::pipe()?;

        let mut command = Command::new("M2");
        command.arg("--silent").arg("--no-readline");
        command.args(options.arguments());
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
        };
        let ready = backend.wait_for_ready()?;
        backend.version = ready.ready.unwrap_or_default();
        backend.names = ready.names;
        Ok(backend)
    }

    /// Spectral's persistent configuration directory.
    fn config_dir() -> Result<PathBuf> {
        #[cfg(test)]
        return Ok(env::temp_dir().join(format!("spectral-m2-test-{}", std::process::id())));

        #[cfg(not(test))]
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|config| config.join("spectral-m2"))
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "no configuration directory")
                    .into()
            })
    }

    /// Materialize the embedded M2 backend at a stable source path. Replacement
    /// through a same-directory staging file keeps parallel launches atomic.
    fn sync_startup_script(config: &Path) -> Result<PathBuf> {
        fs::create_dir_all(config)?;
        let path = config.join(STARTUP_FILE);
        match fs::read(&path) {
            Ok(contents) if contents == STARTUP_SCRIPT.as_bytes() => return Ok(path),
            _ => {}
        }
        // Unique per call, not merely per process: several backends may be
        // starting at once (the test suite does exactly that), and a shared
        // staging name means one launch renames the file out from under another.
        static STAGE: AtomicU32 = AtomicU32::new(0);
        let staged = config.join(format!(
            ".{STARTUP_FILE}-{}-{}",
            std::process::id(),
            STAGE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&staged, STARTUP_SCRIPT)?;
        match fs::rename(&staged, &path) {
            Ok(()) => Ok(path),
            Err(error) => {
                let _ = fs::remove_file(staged);
                Err(error.into())
            }
        }
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
        let written = writeln!(self.input, "{}\n{CELL_TERMINATOR}", code.trim_end())
            .and_then(|()| self.input.flush());
        if let Err(error) = written {
            return match error.kind() {
                std::io::ErrorKind::BrokenPipe => Err(Error::Exited(self.process.wait()?)),
                _ => Err(error.into()),
            };
        }

        let mut transcript = String::new();
        let deadline = Instant::now() + timeout;
        let mut interrupted = false;
        loop {
            if !interrupted && cancelled.load(Ordering::Acquire) {
                nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(self.process.id() as i32),
                    nix::sys::signal::Signal::SIGINT,
                )
                .map_err(std::io::Error::from)?;
                interrupted = true;
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
                    return Ok(M2Response {
                        number: frame.number,
                        is_error: frame.error,
                        transcript,
                        value: frame.value,
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
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

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
            assert!(matches!(response, Ok(_) | Err(Error::Cancelled)));
            let next = backend
                .evaluate_within("1+1", Duration::from_secs(2))
                .expect("evaluate after cancellation");
            assert!(!next.is_error, "{}", next.transcript);
            assert_eq!(next.rendered_value().as_deref(), Some("2"));
        }
    }

    #[test]
    fn an_even_brace_splits_its_cusp_over_the_two_middle_rows() {
        let column = |height, left| {
            let brace = M2Value::curly(height, left, 0);
            brace
                .cells()
                .iter()
                .map(|row| row[0].clone())
                .collect::<String>()
        };
        assert_eq!(column(2, true), "⎰⎱");
        assert_eq!(column(2, false), "⎱⎰");
        assert_eq!(column(4, true), "⎧⎰⎱⎩");
        assert_eq!(column(5, true), "⎧⎪⎨⎪⎩");
    }

    #[test]
    fn plain_values_preserve_net_rows_without_building_a_character_grid() {
        let text = "  x²  \n\n y\t\n";
        let expected = "  x²\n\n y";
        for value in [
            M2Value::Net(text.into()),
            M2Value::Tex {
                source: "\\begin{tabular}...".into(),
                fallback: text.into(),
                code: String::new(),
            },
            M2Value::Markup {
                html: String::new(),
                fallback: text.into(),
                example_width: 77,
            },
            M2Value::Token {
                text: text.into(),
                role: Role::String,
            },
        ] {
            assert_eq!(value.render(), expected);
        }
    }

    #[test]
    fn tex_values_render_as_plain_unicode_blocks() {
        let fraction = M2Value::Tex {
            source: r"$\frac{1}{2}$".into(),
            fallback: "1\n-\n2".into(),
            code: String::new(),
        };
        assert_eq!(fraction.render(), " 1\n───\n 2");

        let matrix = M2Value::Tex {
            source: concat!(
                r"$\left(\!\begin{array}{cc}",
                "\n",
                r"x^{2}&y\\",
                "\n",
                r"1&x-y",
                "\n",
                r"\end{array}\!\right)$"
            )
            .into(),
            fallback: "matrix fallback".into(),
            code: String::new(),
        };
        assert_eq!(matrix.render(), "⎛ x²    y   ⎞\n⎝ 1   x - y ⎠");
    }

    #[test]
    fn unsupported_tex_uses_its_net_fallback() {
        let value = M2Value::Tex {
            source: r"$\spectralUnsupported{true}$".into(),
            fallback: "true".into(),
            code: String::new(),
        };
        assert_eq!(value.render(), "true");

        let document = M2Value::Tex {
            source: r"\par\textbf{Description}".into(),
            fallback: "Description".into(),
            code: String::new(),
        };
        assert_eq!(document.render(), "Description");
    }

    #[test]
    fn m2_tex_dialect_renders_rings_and_ideals() {
        for (source, expected) in [
            (r"${\mathbb Z}$", "ℤ"),
            (
                r"$\frac{{\mathbb Z}\mathopen{}\left[x\right]}{5}$",
                " ℤ[x]\n──────\n  5",
            ),
            (
                r"$\frac{{\mathbb Z}}{5}\mathopen{}\left[x\right]$",
                " ℤ\n───[x]\n 5",
            ),
            (
                r"$\texttt{ideal}\left(x^{3}-10\,y-2\right)$",
                "ideal(x³ - 10 y - 2)",
            ),
            (
                r"$\frac{R}{x^{3}-10\,y-2}$",
                "       R\n───────────────\n x³ - 10 y - 2",
            ),
        ] {
            let value = M2Value::Tex {
                source: source.into(),
                fallback: "NET FALLBACK".into(),
                code: String::new(),
            };
            assert_eq!(value.render(), expected);
        }
    }

    #[test]
    fn m2_tex_dialect_renders_collections() {
        for (source, expected) in [
            (r"$\left\{1,\:2,\:3\right\}$", "{1, 2, 3}"),
            (r"$\texttt{print}$", "print"),
            (
                r"$\texttt{HashTable}\left\{\texttt{a}\ \Rightarrow \ 1,\:\texttt{b}\ \Rightarrow \ 2\right\}$",
                "HashTable{a ⇒ 1, b ⇒ 2}",
            ),
        ] {
            let value = M2Value::Tex {
                source: source.into(),
                fallback: "NET FALLBACK".into(),
                code: String::new(),
            };
            assert_eq!(value.render(), expected);
        }
    }

    // In QQ[xx,yy,xxyy] M2's net collapses four distinct matrix entries into two
    // -- `xx*yy` loses its `*` and reads as the variable `xxyy` -- so both columns
    // print identically. The TeX keeps them apart, which is the whole reason the
    // shell prefers it.
    #[test]
    fn colliding_monomial_names_stay_distinct_through_tex() {
        let value = M2Value::Tex {
            source: r"$\left(\!\begin{array}{cc}
\mathit{xxyy}&\mathit{xx}\,\mathit{yy}\\
\mathit{xxyy}^{2}&\mathit{xx}\,\mathit{yy}^{2}
\end{array}\!\right)$"
                .into(),
            fallback: "| xxyy   xxyy   |\n| xxyy^2 xxyy^2 |".into(),
            code: String::new(),
        };
        let rendered = value.render();

        // The product keeps its separation, which the net cannot express at all;
        // without it `xx*yy` and the variable `xxyy` are the same eight glyphs.
        assert!(
            rendered.contains("xx yy"),
            "the product collapsed into the variable: {rendered}"
        );
        assert!(
            !rendered.contains("xxyy   xxyy"),
            "fell back to the ambiguous net: {rendered}"
        );
    }

    // M2's own net for this matrix is `| x2+1 1/3x2 |`, which reads as the ring
    // variable `x2` plus one -- a different element entirely. Its TeX keeps the
    // two apart (`x^{2}+1` against `\mathit{x2}`), so rendering the TeX is what
    // makes the value readable and falling back to the net is what loses it.
    #[test]
    fn a_matrix_renders_from_tex_rather_than_the_ambiguous_net() {
        let value = M2Value::Tex {
            source: r"$\left(\!\begin{array}{cc}
1&x\\
x^{2}+1&\frac{1}{3}\mathit{x2}
\end{array}\!\right)$"
                .into(),
            fallback: "| 1    x     |\n| x2+1 1/3x2 |".into(),
            code: String::new(),
        };
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

    fn tex(source: &str, fallback: &str) -> M2Value {
        M2Value::Tex {
            source: format!("${source}$"),
            fallback: fallback.into(),
            code: String::new(),
        }
    }

    #[test]
    fn ascii_spells_every_piece_as_macaulay2_does() {
        let table = M2Value::HashTable(vec![[tex("1", "1"), tex(r"\frac{1}{2}", "1/2")]]);

        let ascii = table.ascii();

        assert!(ascii.is_ascii(), "{ascii}");
        assert!(ascii.contains("1 => 1/2"), "{ascii}");
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

        let kind = backend.evaluate("Type", Some(5)).expect("evaluate a type");
        assert!(matches!(
            kind.value,
            Some(M2Value::Token {
                role: Role::Type,
                ..
            })
        ));
        let wrapper = backend
            .evaluate("WrapperType", Some(8))
            .expect("evaluate a type of types");
        assert!(matches!(
            wrapper.value,
            Some(M2Value::Token {
                role: Role::Type,
                ..
            })
        ));
        let contained = backend
            .evaluate("{Type, WrapperType, Core}", Some(9))
            .expect("evaluate types in a list");
        let Some(M2Value::Collection { items, .. }) = &contained.value else {
            panic!("a list arrives as a collection");
        };
        assert!(matches!(
            items.as_slice(),
            [
                M2Value::Token {
                    role: Role::Type,
                    ..
                },
                M2Value::Token {
                    role: Role::Type,
                    ..
                },
                M2Value::Token {
                    role: Role::Package,
                    ..
                },
            ]
        ));
        let ring = backend.evaluate("ZZ", Some(6)).expect("evaluate a ring");
        assert!(!matches!(ring.value, Some(M2Value::Token { .. })));

        let function = backend
            .evaluate("Thing#{SpectralTopLevelMode, Print}", Some(13))
            .expect("evaluate function closure");
        assert!(matches!(
            function.value,
            Some(M2Value::Token {
                role: Role::Function,
                ..
            })
        ));
        // Anonymous, so drawn by its class: its Net would only say where it was
        // written.
        assert_eq!(
            function.rendered_value().as_deref(),
            Some("FunctionClosure")
        );

        let command = backend
            .evaluate("print", Some(14))
            .expect("evaluate command");
        assert_eq!(command.rendered_value().as_deref(), Some("print"));

        for (code, word, role) in [
            ("symbol ^", "^", Role::Operator),
            ("symbol SPACE", "SPACE", Role::Operator),
            ("symbol if", "if", Role::Keyword),
            ("Ext", "Ext", Role::Function),
        ] {
            let evaluated = backend.evaluate(code, Some(15)).expect("evaluate a word");
            assert!(
                matches!(&evaluated.value, Some(M2Value::Token { text, role: found }) if text == word && *found == role),
                "{code} should be the {role:?} token {word}"
            );
        }
        // A functor with a TeX spelling of its own is set in it, not as a name.
        let structure_sheaf = backend
            .evaluate("OO", Some(15))
            .expect("evaluate a functor with TeX");
        assert!(
            matches!(&structure_sheaf.value, Some(M2Value::Tex { source, .. }) if source.contains(r"\mathcal O")),
            "OO should arrive as its calligraphic TeX: {:?}",
            structure_sheaf.value
        );

        // Typed bare, a Command runs; held in a list it is only drawn.
        let held = backend
            .evaluate("{help}", Some(16))
            .expect("evaluate a list holding a command");
        assert!(matches!(
            held.value,
            Some(M2Value::Collection { ref items, .. })
                if matches!(items.as_slice(), [M2Value::Token { role: Role::Command, .. }])
        ));

        let fraction = backend.evaluate("1/2", Some(8)).expect("evaluate fraction");
        assert_eq!(fraction.rendered_value().as_deref(), Some(" 1\n───\n 2"));

        let matrix = backend
            .evaluate("matrix{{1,2},{3,4}}", Some(9))
            .expect("evaluate matrix");
        assert_eq!(
            matrix.rendered_value().as_deref(),
            Some("⎛ 1  2 ⎞\n⎝ 3  4 ⎠")
        );

        let list = backend
            .evaluate("{1,2,3}", Some(10))
            .expect("evaluate list");
        assert_eq!(list.rendered_value().as_deref(), Some("{1, 2, 3}"));

        let table = backend
            .evaluate(r#"hashTable {"a" => 1, "b" => 2}"#, Some(11))
            .expect("evaluate hash table");
        assert_eq!(
            table.rendered_value().as_deref(),
            Some("⎰ a ⇒ 1 ⎱\n⎱ b ⇒ 2 ⎰")
        );

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
        let Some(M2Value::Markup { .. }) = response.value.as_ref() else {
            panic!("help did not return structured markup");
        };
        let width = 60;
        let rendered = response.value.as_ref().unwrap().render_at(width);

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
    fn documentation_links_carry_the_key_help_is_asked_for() {
        let mut backend = M2Backend::spawn().expect("spawn M2");
        let response = backend
            .evaluate("help ancestors", None)
            .expect("read real documentation");
        let Some(M2Value::Markup { html, .. }) = response.value else {
            panic!("documentation did not return markup");
        };
        assert!(
            html.contains(r#"href="&quot;ancestor(Type,Type)&quot;""#),
            "{html}"
        );
        assert!(!html.contains(".html\""), "{html}");
    }

    #[test]
    fn text_documentation_examples_render_as_native_session_blocks() {
        let mut backend = M2Backend::spawn().expect("spawn M2");
        let response = backend
            .evaluate("help det", None)
            .expect("read real documentation");
        let Some(M2Value::Markup {
            html,
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
    fn containers_arrive_entry_by_entry() {
        let mut backend = M2Backend::spawn().expect("spawn M2");

        let list = backend
            .evaluate("{1, [2], (3, 4)}", Some(1))
            .expect("evaluate a nested list");
        let Some(M2Value::Collection {
            kind: CollectionKind::List,
            items,
        }) = &list.value
        else {
            panic!("a list arrived whole: {:?}", list.value);
        };
        assert!(matches!(items[0], M2Value::Tex { .. }));
        assert!(matches!(
            items[1],
            M2Value::Collection {
                kind: CollectionKind::Array,
                ..
            }
        ));
        assert!(matches!(
            items[2],
            M2Value::Collection {
                kind: CollectionKind::Sequence,
                ..
            }
        ));
        assert_eq!(list.rendered_value().as_deref(), Some("{1, [2], (3, 4)}"));

        // A table holding itself opens once and folds where it repeats, rather
        // than recursing forever.
        let looped = backend
            .evaluate("m = new MutableHashTable; m#1 = m; m", Some(2))
            .expect("evaluate a self-referencing table");
        let Some(M2Value::HashTable(entries)) = &looped.value else {
            panic!("the outer table was not opened: {:?}", looped.value);
        };
        assert!(matches!(&entries[0][1], M2Value::Tex { fallback, .. }
            if fallback.starts_with("MutableHashTable{")));
    }

    // A container's inherited TeX only spells out its structure, `Shout{...}`,
    // where M2 prints what its class's own net draws. A real TeX form, such as
    // the one a ring family carries, is still preferred.
    #[test]
    fn a_class_drawing_itself_arrives_as_its_net_not_its_structure() {
        let mut backend = M2Backend::spawn().expect("spawn M2");

        let drawn = backend
            .evaluate(
                r#"Shout = new Type of BasicList; net Shout := x -> net "LOUD"; new Shout from {1}"#,
                Some(1),
            )
            .expect("evaluate a list drawing itself");
        assert!(
            matches!(&drawn.value, Some(M2Value::Net(text)) if text == "LOUD"),
            "arrived as its structure: {:?}",
            drawn.value
        );

        let field = backend.evaluate("RR", Some(2)).expect("evaluate RR");
        assert!(
            matches!(&field.value, Some(M2Value::Tex { source, .. }) if source.contains(r"\mathbb")),
            "RR lost its TeX: {:?}",
            field.value
        );
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
        assert_eq!(recalled.rendered_value().as_deref(), Some("1 729"));

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
        let text: Vec<String> = rows
            .iter()
            .map(|row| row.iter().map(|span| span.content.as_str()).collect())
            .collect();
        assert_eq!(text, ["Matrix QQ\u{00B2} \u{27F5} QQ\u{00B2}"]);
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
        backend.process.kill().unwrap();
        backend.process.wait().unwrap();
        assert!(matches!(
            backend.evaluate("1 + 1", None),
            Err(Error::Exited(_))
        ));
    }

    #[test]
    fn init_file_is_loaded() {
        let base = env::temp_dir().join(format!("spectral-m2-init-test-{}", std::process::id()));
        let config = base.join("spectral-m2");
        fs::create_dir_all(&config).expect("create temp config dir");
        let init_file = config.join("init.m2");
        fs::write(
            &init_file,
            "spectralInitProbe = 1729\nprint \"initialization probe loaded\"\n",
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

        let _ = fs::remove_dir_all(&base);

        assert!(!out.is_error, "probe errored: {:?}", out.transcript);
        assert!(
            out.rendered_value().as_deref() == Some("1 729"),
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
        let path = M2Backend::config_dir()
            .expect("config directory")
            .join(STARTUP_FILE);
        assert_eq!(
            fs::read_to_string(path).expect("read backend.m2"),
            STARTUP_SCRIPT
        );

        let inspected = backend
            .evaluate("code Thing#{SpectralTopLevelMode, BeforePrint}", None)
            .expect("inspect hook source");
        assert!(!inspected.is_error, "{}", inspected.transcript);
    }
}
