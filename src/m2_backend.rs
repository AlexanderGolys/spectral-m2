use std::error::Error;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, fs, thread};

// Length-prefixed framing -- mirrored in src/m2_startup.m2. A request or reply is
// a marker line, a header line, a decimal byte count, then exactly that many
// payload bytes. Length-prefixing keeps the payload opaque, so user code or M2
// output may contain the marker strings, newlines, multi-line strings/comments --
// anything -- without breaking the frame.
const CODE_START: &str = "<<<M2_CODE>>>";
const CHECK_START: &str = "<<<M2_CHECK>>>";
const RESULT_START: &str = "<<<M2_RESULT>>>";
const VERDICT_START: &str = "<<<M2_VERDICT>>>";
const READY_SIGNAL: &str = "<<<M2_READY>>>";

pub struct M2Result {
    pub is_error: bool,
    pub output: String,
}

/// The grammar-oracle verdict for an input buffer, decided by the live M2 process
/// via a non-executing `parse` (see `parseCheck` in src/m2_startup.m2).
pub enum ParseVerdict {
    /// Parsed cleanly -- the buffer is ready to execute.
    Complete,
    /// Parsing reached end of input -- the user is still typing.
    Incomplete,
    /// A real syntax error at a concrete token; carries the M2 diagnostic line.
    SyntaxError(String),
}

pub struct M2Backend {
    process: Child,
    stream: TcpStream,
}

impl M2Backend {
    pub fn spawn() -> Result<Self, Box<dyn Error>> {
        let startup_script = include_str!("m2_startup.m2");

        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);

        let tmp_dir = env::temp_dir();
        let port_file = tmp_dir.join("spectral-m2_port");
        fs::write(&port_file, port.to_string())?;

        let script_path = tmp_dir.join("spectral-m2_startup.m2");
        fs::write(&script_path, startup_script)?;

        let mut process = Command::new("M2")
            .arg("--script")
            .arg(script_path.to_str().unwrap())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stream = Self::connect_to_m2(port, &mut process)?;

        Self::wait_for_ready(&stream, &mut process)?;

        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;

        let _ = fs::remove_file(&port_file);

        Ok(Self { process, stream })
    }

    fn connect_to_m2(port: u16, process: &mut Child) -> Result<TcpStream, Box<dyn Error>> {
        let addr = format!("127.0.0.1:{}", port);
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

    /// Send a length-prefixed request frame: marker line, byte count, payload bytes.
    fn send_frame(&mut self, marker: &str, payload: &str) -> Result<(), Box<dyn Error>> {
        let header = format!("{}\n{}\n", marker, payload.len());
        self.stream.write_all(header.as_bytes())?;
        self.stream.write_all(payload.as_bytes())?;
        self.stream.flush()?;
        Ok(())
    }

    /// Read a length-prefixed reply frame: the marker, then a header line, then a
    /// byte count, then exactly that many payload bytes. Returns (header, payload).
    fn read_frame(&mut self, expected: &str) -> Result<(String, String), Box<dyn Error>> {
        let mut reader = BufReader::new(&self.stream);

        let mut marker = String::new();
        if reader.read_line(&mut marker)? == 0 {
            return Err("Connection to M2 closed".into());
        }
        if marker.trim() != expected {
            return Err(format!("expected {expected}, got {:?}", marker.trim()).into());
        }

        let mut header = String::new();
        reader.read_line(&mut header)?;

        let mut count = String::new();
        reader.read_line(&mut count)?;
        let n: usize = count.trim().parse()?;

        let mut buf = vec![0u8; n];
        reader.read_exact(&mut buf)?;

        Ok((
            header.trim().to_string(),
            String::from_utf8_lossy(&buf).into_owned(),
        ))
    }

    /// Evaluate `code` (executes it). Synchronous and blocking (30s read timeout).
    pub fn evaluate(&mut self, code: &str) -> Result<M2Result, Box<dyn Error>> {
        self.send_frame(CODE_START, code)?;
        let (status, output) = self.read_frame(RESULT_START)?;
        Ok(M2Result {
            is_error: status == "error",
            output: output.trim_end_matches('\n').to_string(),
        })
    }

    /// Ask the live M2 process to classify `code` without executing it.
    pub fn check(&mut self, code: &str) -> Result<ParseVerdict, Box<dyn Error>> {
        self.send_frame(CHECK_START, code)?;
        let (verdict, msg) = self.read_frame(VERDICT_START)?;
        parse_verdict(&verdict, msg)
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

        // A payload containing a marker line must NOT break framing.
        assert!(matches!(
            be.check("s = \"<<<M2_END>>>\nmore\"").unwrap(),
            ParseVerdict::Complete
        ));

        let ok = be.evaluate("2 + 3").unwrap();
        assert!(!ok.is_error, "2+3 errored: {:?}", ok.output);
        assert!(ok.output.contains('5'), "expected 5 in {:?}", ok.output);

        let bad = be.evaluate("1/0").unwrap();
        assert!(bad.is_error, "1/0 should error: {:?}", bad.output);
    }
}
