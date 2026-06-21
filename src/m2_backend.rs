use std::error::Error;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, fs, thread};

use serde::{Deserialize, Serialize};

// Length-prefixed JSON framing -- mirrored in src/m2_startup.m2. Every frame is a
// marker line, a decimal byte count line, then exactly that many payload bytes.
// Length-prefixing keeps the payload opaque, so a JSON document may contain the
// marker string, newlines, quotes -- anything -- without breaking the frame.

/// The loopback interface the backend listens on and the TUI connects to. The M2
/// side builds the same address from this host (`serverHost` in src/m2_startup.m2).
const LOOPBACK_HOST: &str = "127.0.0.1";

/// The marker line that opens every request and reply frame. Mirrored as `M2MSG`
/// in src/m2_startup.m2.
const MSG_MARKER: &str = "<<<M2_MSG>>>";

/// The one-time handshake line M2 sends once its listener is up.
const READY_SIGNAL: &str = "<<<M2_READY>>>";

/// A request to the M2 backend. Serializes to `{"method": ..., "code": ...}`; the
/// `method` tag drives dispatch in src/m2_startup.m2's `handleMessage`.
#[derive(Serialize)]
#[serde(tag = "method", rename_all = "lowercase")]
enum Request<'a> {
    /// Execute `code` in the live session and capture its output.
    Evaluate { code: &'a str },
    /// Classify `code`'s completeness without executing it.
    Check { code: &'a str },
}

/// The reply to a [`Request::Evaluate`]: the captured session output and whether
/// evaluation raised an error.
#[derive(Deserialize)]
struct EvalReply {
    output: String,
    error: bool,
}

/// The reply to a [`Request::Check`]: the completeness verdict and a diagnostic
/// message (empty unless the verdict is `ERROR`).
#[derive(Deserialize)]
struct CheckReply {
    verdict: String,
    message: String,
}

pub struct M2Result {
    pub is_error: bool,
    pub output: String,
}

/// The grammar-oracle verdict for an input buffer, decided by the live M2 process
/// via a non-executing `parse` (see `parseCheck` in src/m2_startup.m2).
pub enum ParseVerdict {
    Complete,
    Incomplete,
    SyntaxError(String),
}

pub struct M2Backend {
    process: Child,
    stream: TcpStream,
}

impl M2Backend {
    pub fn spawn() -> Result<Self, Box<dyn Error>> {
        let startup_script = include_str!("m2_startup.m2");

        // Bind an ephemeral port, then hand it to M2 to re-open as its listener.
        let listener = TcpListener::bind(format!("{LOOPBACK_HOST}:0"))?;
        let port = listener.local_addr()?.port();
        drop(listener);

        let script_path = env::temp_dir().join("spectral-m2_startup.m2");
        fs::write(&script_path, startup_script)?;

        // The port is passed as the script's sole argument (M2 exposes it as
        // `scriptCommandLine#1`), so the two processes can never disagree on a
        // shared file path the way a `/tmp` rendezvous file could.
        let mut process = Command::new("M2")
            .arg("--script")
            .arg(script_path.to_str().unwrap())
            .arg(port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stream = Self::connect_to_m2(port, &mut process)?;

        Self::wait_for_ready(&stream, &mut process)?;

        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;

        let mut backend = Self { process, stream };
        backend.load_init_file()?;
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
    fn load_init_file(&mut self) -> Result<(), Box<dyn Error>> {
        let Some(path) = Self::init_file_path() else {
            return Ok(());
        };
        let Ok(code) = fs::read_to_string(&path) else {
            return Ok(());
        };
        if !code.trim().is_empty() {
            self.evaluate(&code)?;
        }
        Ok(())
    }

    fn connect_to_m2(port: u16, process: &mut Child) -> Result<TcpStream, Box<dyn Error>> {
        let addr = format!("{LOOPBACK_HOST}:{port}");
        let start = Instant::now();
        loop {
            if let Some(status) = process.try_wait()? {
                let mut stderr = String::new();
                if let Some(mut err) = process.stderr.take() {
                    let _ = err.read_to_string(&mut stderr);
                }
                return Err(
                    format!("M2 process exited (status: {})\n{}", status, stderr.trim()).into(),
                );
            }
            if let Ok(stream) = TcpStream::connect(&addr) {
                return Ok(stream);
            }
            if start.elapsed() > Duration::from_secs(30) {
                return Err("Timed out connecting to M2".into());
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn wait_for_ready(stream: &TcpStream, process: &mut Child) -> Result<(), Box<dyn Error>> {
        let mut buf = String::new();
        let mut reader = BufReader::new(stream);
        loop {
            buf.clear();
            match reader.read_line(&mut buf) {
                Ok(0) => {
                    let mut stderr = String::new();
                    if let Some(mut err) = process.stderr.take() {
                        let _ = err.read_to_string(&mut stderr);
                    }
                    return Err(format!(
                        "M2 closed before sending ready signal\n{}",
                        stderr.trim()
                    )
                    .into());
                }
                Err(e) => return Err(format!("Error reading from M2: {}", e).into()),
                _ => {
                    if buf.contains(READY_SIGNAL) {
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Send a length-prefixed frame: marker line, byte count, payload bytes.
    fn send_frame(&mut self, payload: &str) -> Result<(), Box<dyn Error>> {
        let header = format!("{MSG_MARKER}\n{}\n", payload.len());
        self.stream.write_all(header.as_bytes())?;
        self.stream.write_all(payload.as_bytes())?;
        self.stream.flush()?;
        Ok(())
    }

    /// Read a length-prefixed frame: the marker line, a byte count line, then
    /// exactly that many payload bytes. Returns the payload (a JSON document).
    fn read_frame(&mut self) -> Result<String, Box<dyn Error>> {
        let mut reader = BufReader::new(&self.stream);

        let mut marker = String::new();
        if reader.read_line(&mut marker)? == 0 {
            return Err("Connection to M2 closed".into());
        }
        if marker.trim() != MSG_MARKER {
            return Err(format!("expected {MSG_MARKER}, got {:?}", marker.trim()).into());
        }

        let mut count = String::new();
        reader.read_line(&mut count)?;
        let n: usize = count.trim().parse()?;

        let mut buf = vec![0u8; n];
        reader.read_exact(&mut buf)?;

        Ok(String::from_utf8_lossy(&buf).into_owned())
    }

    /// Round-trip one request through the backend, returning the reply payload.
    fn request(&mut self, request: &Request) -> Result<String, Box<dyn Error>> {
        self.send_frame(&serde_json::to_string(request)?)?;
        self.read_frame()
    }

    /// Evaluate `code` (executes it). Synchronous and blocking (30s read timeout).
    pub fn evaluate(&mut self, code: &str) -> Result<M2Result, Box<dyn Error>> {
        let reply: EvalReply = serde_json::from_str(&self.request(&Request::Evaluate { code })?)?;
        Ok(M2Result {
            is_error: reply.error,
            output: reply.output.trim_end_matches('\n').to_string(),
        })
    }

    /// Ask the live M2 process to classify `code` without executing it.
    pub fn check(&mut self, code: &str) -> Result<ParseVerdict, Box<dyn Error>> {
        let reply: CheckReply = serde_json::from_str(&self.request(&Request::Check { code })?)?;
        parse_verdict(&reply.verdict, reply.message)
    }

    #[allow(dead_code)]
    pub fn is_running(&mut self) -> bool {
        matches!(self.process.try_wait(), Ok(None))
    }
}

/// Map an M2 verdict line + message into a [`ParseVerdict`].
fn parse_verdict(verdict: &str, msg: String) -> Result<ParseVerdict, Box<dyn Error>> {
    Ok(match verdict {
        "COMPLETE" => ParseVerdict::Complete,
        "INCOMPLETE" => ParseVerdict::Incomplete,
        "ERROR" => ParseVerdict::SyntaxError(msg),
        other => return Err(format!("unexpected verdict from M2: {other:?}").into()),
    })
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
    fn verdict_maps_known_lines() {
        assert!(matches!(
            parse_verdict("COMPLETE", String::new()).unwrap(),
            ParseVerdict::Complete
        ));
        assert!(matches!(
            parse_verdict("INCOMPLETE", String::new()).unwrap(),
            ParseVerdict::Incomplete
        ));
        match parse_verdict("ERROR", "syntax error: unmatched )".to_string()).unwrap() {
            ParseVerdict::SyntaxError(m) => assert_eq!(m, "syntax error: unmatched )"),
            _ => panic!("expected SyntaxError"),
        }
    }

    #[test]
    fn verdict_rejects_garbage() {
        assert!(parse_verdict("???", String::new()).is_err());
    }

    // End-to-end over a real M2 process and the length-prefixed protocol.
    #[test]
    #[ignore = "spawns a real M2 process; run with `cargo test -- --ignored`"]
    fn check_and_evaluate_round_trip() {
        let mut be = M2Backend::spawn().expect("spawn M2");

        assert!(matches!(be.check("1+1").unwrap(), ParseVerdict::Complete));
        assert!(matches!(
            be.check("a = 5\nb = 6").unwrap(),
            ParseVerdict::Complete
        ));
        assert!(matches!(be.check("a*").unwrap(), ParseVerdict::Incomplete));
        assert!(matches!(
            be.check("if true").unwrap(),
            ParseVerdict::Incomplete
        ));
        assert!(matches!(
            be.check("symbol").unwrap(),
            ParseVerdict::Incomplete
        ));
        assert!(matches!(
            be.check(")").unwrap(),
            ParseVerdict::SyntaxError(_)
        ));

        // A payload containing the frame marker must NOT break framing.
        assert!(matches!(
            be.check("s = \"<<<M2_MSG>>>\nmore\"").unwrap(),
            ParseVerdict::Complete
        ));

        let ok = be.evaluate("2 + 3").unwrap();
        assert!(!ok.is_error, "2+3 errored: {:?}", ok.output);
        assert!(ok.output.contains('5'), "expected 5 in {:?}", ok.output);

        let bad = be.evaluate("1/0").unwrap();
        assert!(bad.is_error, "1/0 should error: {:?}", bad.output);
    }

    // A user init file is evaluated at spawn, so its definitions are in scope at
    // the first prompt. Points `XDG_CONFIG_HOME` at a temp config holding an
    // init.m2 and checks the variable it defines resolves.
    #[test]
    #[ignore = "spawns a real M2 process; run with `cargo test -- --ignored`"]
    fn init_file_is_loaded() {
        let base = env::temp_dir().join(format!("spectral-m2-test-{}", std::process::id()));
        let config = base.join("spectral-m2");
        fs::create_dir_all(&config).expect("create temp config dir");
        fs::write(config.join("init.m2"), "spectralInitProbe = 1729\n").expect("write init.m2");
        env::set_var("XDG_CONFIG_HOME", &base);

        let mut be = M2Backend::spawn().expect("spawn M2");
        let out = be.evaluate("spectralInitProbe").expect("evaluate probe");

        env::remove_var("XDG_CONFIG_HOME");
        let _ = fs::remove_dir_all(&base);

        assert!(!out.is_error, "probe errored: {:?}", out.output);
        assert!(
            out.output.contains("1729"),
            "expected 1729 in {:?}",
            out.output
        );
    }
}
