use std::io::Read;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, fs, thread};

use crate::errors::{M2Error, M2Result};
use crate::parse_check::{ParseReply, ParseVerdict};
use jsonrpc::simple_tcp::TcpTransport;
use jsonrpc::Client;
use serde::{Deserialize, Serialize};

/// The loopback interface the backend listens on and the TUI connects to. The M2
/// side builds the same address from this host (`serverHost` in src/m2_startup.m2).
const LOOPBACK_HOST: &str = "127.0.0.1";

#[derive(Serialize)]
struct CodeParams<'a> {
    code: &'a str,
}

/// The reply to an `evaluate` request: the captured session output and whether
/// evaluation raised an error.
#[derive(Deserialize)]
struct EvalReply {
    output: String,
    error: bool,
}

pub struct M2Response {
    pub is_error: bool,
    pub output: String,
}

/// The live Macaulay2 child and its JSON-RPC client.
pub struct M2Backend {
    process: Child,
    client: Client,
}

impl M2Backend {
    pub fn spawn() -> M2Result<Self> {
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
            .arg(&script_path)
            .arg(port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let addr: SocketAddr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port).into();
        let client = Client::with_transport(TcpTransport {
            addr,
            timeout: Some(Duration::from_secs(300)),
        });
        Self::wait_for_ready(&client, &mut process)?;

        let mut backend = Self { process, client };
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
    fn load_init_file(&mut self) -> M2Result<()> {
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

    fn wait_for_ready(client: &Client, process: &mut Child) -> M2Result<()> {
        let start = Instant::now();
        const TIMEOUT_S: u64 = 30;
        loop {
            if let Some(status) = process.try_wait()? {
                let mut stderr = String::new();
                if let Some(mut err) = process.stderr.take() {
                    let _ = err.read_to_string(&mut stderr);
                }
                return Err(M2Error::M2ProcessError {
                    status: status.to_string(),
                    stderr: stderr.trim().to_string(),
                });
            }
            if client.call::<String>("version", None).is_ok() {
                return Ok(());
            }
            if start.elapsed() > Duration::from_secs(TIMEOUT_S) {
                return Err(M2Error::Timeout(TIMEOUT_S));
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// Evaluate `code` (executes it). Synchronous and blocking (300s read timeout).
    pub fn evaluate(&self, code: &str) -> M2Result<M2Response> {
        let params = jsonrpc::try_arg(CodeParams { code })?;
        let reply: EvalReply = self.client.call("evaluate", Some(&params))?;
        Ok(M2Response {
            is_error: reply.error,
            output: reply.output.trim_end_matches('\n').to_string(),
        })
    }

    /// Ask the live M2 process to classify `code` without executing it.
    pub fn check(&self, code: &str) -> M2Result<ParseVerdict> {
        let params = jsonrpc::try_arg(CodeParams { code })?;
        let reply: ParseReply = self.client.call("check", Some(&params))?;
        reply.into_verdict()
    }

    /// Query the M2 version string.
    pub fn version(&self) -> M2Result<String> {
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

    fn assert_complete(code: &str, backend: &M2Backend) {
        assert!(
            matches!(backend.check(code).unwrap(), ParseVerdict::Complete),
            "expected complete verdict, got {:?} for code \n {}",
            backend.check(code),
            code
        );
    }

    fn assert_incomplete(code: &str, backend: &M2Backend) {
        assert!(
            matches!(backend.check(code).unwrap(), ParseVerdict::Incomplete),
            "expected incomplete verdict, got {:?} for code \n {}",
            backend.check(code),
            code
        );
    }

    fn assert_error(code: &str, backend: &M2Backend) {
        assert!(
            matches!(backend.check(code).unwrap(), ParseVerdict::SyntaxError(_)),
            "expected syntax error, got {:?} for code \n {}",
            backend.check(code),
            code
        );
    }

    // End-to-end over a real M2 process and the JSON-RPC TCP transport.
    #[test]
    fn check_and_evaluate_round_trip() {
        let be = M2Backend::spawn().expect("spawn M2");

        assert_complete("1+1", &be);
        assert_complete("a = 5;\nb = 6\n\n", &be);
        assert_complete("local symbol", &be);
        assert_complete("if if 1\n\nthen\n\t1\nelse \n 1then 1", &be);

        assert_incomplete("a*", &be);
        assert_incomplete("*", &be);
        assert_incomplete("* *", &be);
        assert_incomplete("if", &be);
        assert_incomplete("(", &be);
        assert_incomplete("({{}", &be);
        assert_incomplete("symbol", &be);
        assert_incomplete("if if", &be);
        assert_incomplete("if 1", &be);
        assert_incomplete("if if 1 then 1", &be);
        assert_incomplete("if if 1\n\n", &be);
        assert_incomplete("if if 1\n\nthen ", &be);
        assert_incomplete("if if 1\n\nthen ", &be);
        assert_incomplete("if if 1\n\nthen\n\t1\nelse \n 1then", &be);
        assert_incomplete("if if 1\n\nthen\n\t1\nelse \n 1", &be);
        assert_incomplete("if if 1\n\nthen\n\t1\nelse", &be);
        assert_incomplete("\"", &be);
        assert_incomplete("///", &be);
        assert_incomplete("/////", &be);

        assert_error("if;", &be);
        assert_error("then", &be);
        assert_error(")", &be);
        assert_error("**", &be);
    }

    // A user init file is evaluated at spawn, so its definitions are in scope at
    // the first prompt. Points `XDG_CONFIG_HOME` at a temp config holding an
    // init.m2 and checks the variable it defines resolves.
    #[test]
    fn init_file_is_loaded() {
        let base = env::temp_dir().join(format!("spectral-m2-test-{}", std::process::id()));
        let config = base.join("spectral-m2");
        fs::create_dir_all(&config).expect("create temp config dir");
        fs::write(config.join("init.m2"), "spectralInitProbe = 1729\n").expect("write init.m2");
        env::set_var("XDG_CONFIG_HOME", &base);

        let be = M2Backend::spawn().expect("spawn M2");
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
