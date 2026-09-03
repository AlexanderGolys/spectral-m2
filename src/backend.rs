//! Macaulay2 process management, protocol types, and transcript decoding.

pub mod transcript;

use std::io::Read;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, fs, thread};

use crate::errors::{Error, Result};
use jsonrpc::simple_tcp::TcpTransport;
use jsonrpc::Client;
use serde::{Deserialize, Serialize};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// Code submitted for evaluation, tagged with the input number it was entered
/// under. `number` is `None` for code the session runs on its own behalf, whose
/// results must not claim an `oN` binding.
#[derive(Serialize)]
struct EvaluateParams<'a> {
    code: &'a str,
    number: Option<u32>,
}

/// The output and status returned by one M2 evaluation.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct M2Response {
    #[serde(rename = "error")]
    pub is_error: bool,
    pub transcript: String,
    /// The rendered result, present only when M2 displayed it. A `;`-suppressed
    /// result is `None` here -- deliberately never rendered -- yet still bound.
    pub value: Option<String>,
    pub type_info: Option<String>,
    /// Whether M2 bound an `oN` for this input. Not implied by `value`: a
    /// suppressed result binds one without producing rendered text.
    pub has_value: bool,
    /// M2's interpreter depth after the evaluation. Anything above 1 means a
    /// nested interpreter, which is what being in the debugger looks like.
    pub depth: u32,
}

/// The live Macaulay2 child and its JSON-RPC client.
pub struct M2Backend {
    process: Child,
    client: Client,
}

impl M2Backend {
    pub fn spawn() -> Result<Self> {
        Self::spawn_with_init_file(Self::init_file_path())
    }

    fn spawn_with_init_file(init_file: Option<PathBuf>) -> Result<Self> {
        let startup_script = include_str!("backend/m2_startup.m2");

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        drop(listener);

        let script_path = env::temp_dir().join(format!("spectral-m2_startup-{port}.m2"));
        fs::write(&script_path, startup_script)?;

        // The port is passed as the script's sole argument (M2 exposes it as
        // `scriptCommandLine#1`), so the two processes can never disagree on a
        // shared file path the way a `/tmp` rendezvous file could.
        let process = Command::new("M2")
            .arg("--script")
            .arg(&script_path)
            .arg(port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let client = Client::with_transport(TcpTransport {
            addr: SocketAddrV4::new(Ipv4Addr::LOCALHOST, port).into(),
            timeout: Some(Duration::from_secs(300)),
        });
        let mut backend = Self { process, client };
        let ready = backend.wait_for_ready();
        let _ = fs::remove_file(script_path);
        ready?;
        backend.drain_output();
        backend.load_init_file(init_file.as_deref())?;
        Ok(backend)
    }

    /// The optional user init file, evaluated once at startup so personal
    /// definitions are in scope before the first prompt:
    /// `$XDG_CONFIG_HOME/spectral-m2/init.m2` (falling back to
    /// `~/.config/spectral-m2/init.m2`). `None` when neither var is set.
    fn init_file_path() -> Option<PathBuf> {
        let config_dir = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
        Some(config_dir.join("spectral-m2").join("init.m2"))
    }

    /// Evaluate the user init file if one exists. A missing or unreadable file is
    /// the common case and not an error; only a broken socket propagates. The
    /// file's own output is discarded -- it runs before the transcript exists.
    fn load_init_file(&self, path: Option<&Path>) -> Result<()> {
        let Some(path) = path else {
            return Ok(());
        };
        let Ok(code) = fs::read_to_string(path) else {
            return Ok(());
        };
        if !code.trim().is_empty() {
            self.evaluate(&code, None)?;
        }
        Ok(())
    }

    /// Read and discard whatever the child writes to its own stdout and stderr.
    ///
    /// Everything the shell displays arrives over the socket, so these streams
    /// only ever carry output that escaped `capture` -- notably from programs M2
    /// launches, such as the browser `viewHelp` opens, which inherit these pipes
    /// and are happy to chatter on them. Left unread, a full pipe buffer blocks
    /// the writer, and M2 stops answering for good.
    fn drain_output(&mut self) {
        fn discard(stream: Option<impl Read + Send + 'static>) {
            let Some(mut stream) = stream else {
                return;
            };
            thread::spawn(move || {
                let mut scratch = [0_u8; 4096];
                while matches!(stream.read(&mut scratch), Ok(read) if read > 0) {}
            });
        }
        discard(self.process.stdout.take());
        discard(self.process.stderr.take());
    }

    fn wait_for_ready(&mut self) -> Result<()> {
        let start = Instant::now();
        loop {
            if let Some(status) = self.process.try_wait()? {
                let mut stderr = String::new();
                if let Some(mut err) = self.process.stderr.take() {
                    let _ = err.read_to_string(&mut stderr);
                }
                let status = status.to_string();
                let stderr = stderr.trim().to_string();
                let details = if stderr.is_empty() {
                    status
                } else {
                    format!("{status} ({stderr})")
                };
                return Err(Error::Process(details));
            }
            if self.client.call::<String>("version", None).is_ok() {
                return Ok(());
            }
            if start.elapsed() > STARTUP_TIMEOUT {
                return Err(Error::Timeout(STARTUP_TIMEOUT.as_secs()));
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// Evaluate `code` (executes it) as input `number`, under which M2 binds the
    /// result's `oN`. Synchronous and blocking (300s read timeout).
    pub fn evaluate(&self, code: &str, number: Option<u32>) -> Result<M2Response> {
        let params = jsonrpc::try_arg(EvaluateParams { code, number })?;
        Ok(self.client.call("evaluate", Some(&params))?)
    }

    /// Query the M2 version string.
    pub fn version(&self) -> Result<String> {
        Ok(self.client.call("version", None)?)
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

    #[test]
    fn evaluation_distinguishes_printed_null_from_returned_value() {
        let backend = M2Backend::spawn().expect("spawn M2");

        let printed = backend
            .evaluate("print \"hello\"", Some(1))
            .expect("evaluate null result");
        assert!(!printed.is_error);
        assert!(printed.transcript.contains("print \"hello\""));
        assert!(printed.transcript.contains("hello"));
        // An unterminated statement reports its result even when that result is
        // null -- but null binds no oN, so there is no symbol to name it by.
        assert_eq!(printed.value.as_deref(), Some("null"));
        assert_eq!(printed.type_info.as_deref(), Some("Nothing"));
        assert!(!printed.has_value, "a null result binds no oN");

        let returned = backend
            .evaluate("2 + 2", Some(2))
            .expect("evaluate returned value");
        assert!(!returned.is_error);
        assert!(returned.transcript.contains("2 + 2"));
        assert_eq!(returned.value.as_deref(), Some("4"));
        assert_eq!(returned.type_info.as_deref(), Some("ZZ"));

        let string = backend
            .evaluate("\"a\"", Some(6))
            .expect("evaluate string literal");
        assert!(!string.is_error, "string errored: {:?}", string.transcript);
        assert_eq!(string.value.as_deref(), Some("a"));
        assert_eq!(string.type_info.as_deref(), Some("String"));

        let boolean = backend
            .evaluate("true", Some(7))
            .expect("evaluate boolean literal");
        assert_eq!(boolean.value.as_deref(), Some("true"));
        assert_eq!(boolean.type_info.as_deref(), Some("Boolean"));

        let spoofed_code = "print \"\\no184 = 11\"";
        let spoofed = backend
            .evaluate(spoofed_code, Some(3))
            .expect("evaluate spoofed output marker");
        assert!(!spoofed.is_error);
        assert!(spoofed.transcript.contains("o184 = 11"));
        // The spoofed marker stays printed text: the reported value is the
        // statement's actual result, which for `print` is null.
        assert_eq!(spoofed.value.as_deref(), Some("null"));
        let spoofed_output = crate::backend::transcript::captured_output(
            &spoofed.transcript,
            spoofed_code,
            spoofed.value.as_deref(),
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
        assert_eq!(identity.value.as_deref(), Some("true"));
    }

    // `capture` restarts its inner session at lineNumber 1, so M2's own AfterEval
    // would bind every result to o1. The shell's input number has to survive that.
    #[test]
    fn results_bind_the_output_symbol_of_their_input_number() {
        let backend = M2Backend::spawn().expect("spawn M2");

        backend.evaluate("41 + 1", Some(42)).expect("evaluate");
        let recalled = backend.evaluate("o42", Some(43)).expect("recall o42");
        assert_eq!(recalled.value.as_deref(), Some("42"));

        // A `;` suppresses rendering, so no value comes back -- but the binding
        // is still made, which is what has_value reports.
        let suppressed = backend
            .evaluate("spectralSuppressed = 7;", Some(44))
            .expect("evaluate suppressed");
        assert!(suppressed.value.is_none(), "`;` suppresses the rendering");
        assert!(suppressed.has_value, "a suppressed result still binds oN");
        assert_eq!(suppressed.type_info.as_deref(), Some("ZZ"));
        let recalled = backend.evaluate("o44", Some(45)).expect("recall o44");
        assert_eq!(recalled.value.as_deref(), Some("7"));
    }

    // The transport loads JSON and JSONRPC, whose dictionaries would otherwise
    // sit at the front of dictionaryPath and reserve their exports: `json = 3`
    // would fail with "assignment to protected global variable 'toJSON'".
    #[test]
    fn transport_packages_do_not_reserve_user_names() {
        let backend = M2Backend::spawn().expect("spawn M2");

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
        let backend = M2Backend::spawn().expect("spawn M2");

        let failed = backend.evaluate("1/0", Some(1)).expect("evaluate");
        assert!(failed.is_error);
        assert!(
            failed.transcript.contains("division by zero"),
            "diagnostic missing from transcript: {:?}",
            failed.transcript
        );

        let message = crate::backend::transcript::captured_error(&failed.transcript, "1/0");
        assert!(
            message.contains("division by zero"),
            "diagnostic lost while cleaning: {message:?}"
        );
    }

    #[test]
    fn init_file_is_loaded() {
        let base = env::temp_dir().join(format!("spectral-m2-test-{}", std::process::id()));
        let config = base.join("spectral-m2");
        fs::create_dir_all(&config).expect("create temp config dir");
        let init_file = config.join("init.m2");
        fs::write(&init_file, "spectralInitProbe = 1729\n").expect("write init.m2");

        let be = M2Backend::spawn_with_init_file(Some(init_file)).expect("spawn M2");
        let out = be
            .evaluate("spectralInitProbe", Some(1))
            .expect("evaluate probe");

        let _ = fs::remove_dir_all(&base);

        assert!(!out.is_error, "probe errored: {:?}", out.transcript);
        assert!(
            out.value.as_deref() == Some("1729"),
            "expected 1729 in {:?}",
            out.value
        );
    }
}
