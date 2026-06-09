use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};

const CODE_START: &str = "<<<M2_CODE>>>";
const CODE_END: &str = "<<<M2_END>>>";
const RESULT_START: &str = "<<<M2_RESULT>>>";
const RESULT_END: &str = "<<<M2_END>>>";
const READY_SIGNAL: &str = "<<<M2_READY>>>";

pub struct M2Result {
    pub is_error: bool,
    pub output: String,
}

pub struct M2Backend {
    process: Child,
    stream: TcpStream,
    input_counter: usize,
    history: Vec<String>,
}

impl M2Backend {
    pub fn spawn() -> Result<Self, Box<dyn std::error::Error>> {
        let startup_script = include_str!("m2_startup.m2");

        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);

        let tmp_dir = std::env::temp_dir();
        let port_file = tmp_dir.join("m2sh_port");
        std::fs::write(&port_file, port.to_string())?;

        let script_path = tmp_dir.join("m2sh_startup.m2");
        std::fs::write(&script_path, startup_script)?;

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
        stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(10)))?;

        let _ = std::fs::remove_file(&port_file);

        Ok(Self {
            process,
            stream,
            input_counter: 0,
            history: Vec::new(),
        })
    }

    fn connect_to_m2(
        port: u16,
        process: &mut Child,
    ) -> Result<TcpStream, Box<dyn std::error::Error>> {
        let addr = format!("127.0.0.1:{}", port);
        let start = std::time::Instant::now();
        loop {
            match process.try_wait()? {
                Some(status) => {
                    let mut stderr = String::new();
                    if let Some(mut err) = process.stderr.take() {
                        let _ = err.read_to_string(&mut stderr);
                    }
                    return Err(format!(
                        "M2 process exited (status: {})\n{}",
                        status,
                        stderr.trim()
                    )
                    .into());
                }
                None => {}
            }
            if let Ok(stream) = TcpStream::connect(&addr) {
                return Ok(stream);
            }
            if start.elapsed() > std::time::Duration::from_secs(30) {
                return Err("Timed out connecting to M2".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    fn wait_for_ready(
        stream: &TcpStream,
        process: &mut Child,
    ) -> Result<(), Box<dyn std::error::Error>> {
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
                    return Err(format!("M2 closed before sending ready signal\n{}", stderr.trim()).into());
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

    pub fn evaluate(&mut self, code: &str) -> Result<M2Result, Box<dyn std::error::Error>> {
        self.input_counter += 1;
        self.history.push(code.to_string());

        let msg = format!("{}\n{}\n{}\n", CODE_START, code, CODE_END);
        self.stream.write_all(msg.as_bytes())?;
        self.stream.flush()?;

        let mut reader = BufReader::new(&self.stream);
        let mut line = String::new();

        loop {
            line.clear();
            let bytes = reader.read_line(&mut line)?;
            if bytes == 0 {
                return Err("Connection to M2 closed".into());
            }
            if line.trim() == RESULT_START {
                break;
            }
        }

        let mut status_line = String::new();
        reader.read_line(&mut status_line)?;
        let is_error = status_line.trim() == "error";

        let mut output = String::new();
        loop {
            line.clear();
            let bytes = reader.read_line(&mut line)?;
            if bytes == 0 {
                break;
            }
            if line.trim() == RESULT_END {
                break;
            }
            output.push_str(&line);
        }

        let output = output.trim_end_matches('\n').to_string();

        Ok(M2Result { is_error, output })
    }

    pub fn next_input_number(&self) -> usize {
        self.input_counter + 1
    }

    #[allow(dead_code)]
    pub fn history(&self) -> &[String] {
        &self.history
    }

    #[allow(dead_code)]
    pub fn is_running(&mut self) -> bool {
        matches!(self.process.try_wait(), Ok(None))
    }
}

impl Drop for M2Backend {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}