//! A record of why the program stopped, kept where it survives the screen.
//!
//! While the screen is drawn stderr points at a sink, so the message a panic prints
//! is lost with it. The hook here writes the report to a file instead, and the note
//! naming that file is printed once the terminal has been restored.

use crate::env::ENV;
use std::backtrace::Backtrace;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic::{self, PanicHookInfo};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const LOG_DIRECTORY: &str = "spectral-m2";
const LOG_FILE: &str = "crash.log";

/// What went wrong, in one line, and where the whole report was written.
struct Recorded {
    summary: String,
    log: Option<PathBuf>,
}

static RECORDED: Mutex<Option<Recorded>> = Mutex::new(None);

/// Start recording panics from every thread.
pub fn install() {
    panic::set_hook(Box::new(|info| {
        let summary = summary(info);
        let report = format!(
            "{} {summary}\n{}\n",
            unix_seconds(),
            Backtrace::force_capture()
        );
        let log = log_path().filter(|path| record(path, &report).is_ok());
        if let Ok(mut recorded) = RECORDED.lock() {
            *recorded = Some(Recorded { summary, log });
        }
    }));
}

/// Say what stopped the program, if anything was recorded. Meant for after the
/// terminal is restored, when stderr shows again.
pub fn report() {
    let Some(recorded) = RECORDED
        .lock()
        .ok()
        .and_then(|mut recorded| recorded.take())
    else {
        return;
    };
    eprintln!("sp2 stopped: {}", recorded.summary);
    match recorded.log {
        Some(path) => eprintln!("The full report is in {}", path.display()),
        None => eprintln!("The report could not be written to a file."),
    }
}

fn summary(info: &PanicHookInfo) -> String {
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|message| message.to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic without a message".into());
    let place = info
        .location()
        .map_or_else(String::new, |location| format!(" at {location}"));
    let thread = thread::current();
    format!(
        "thread '{}' panicked: {message}{place}",
        thread.name().unwrap_or("<unnamed>")
    )
}

fn log_path() -> Option<PathBuf> {
    let state = ENV.state_home()?;
    Some(state.join(LOG_DIRECTORY).join(LOG_FILE))
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn record(path: &Path, report: &str) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(report.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_are_appended_and_the_directory_is_made() {
        let directory = ENV.temp_dir().join(format!("sp2-crash-{}", unix_seconds()));
        let path = directory.join("nested").join(LOG_FILE);

        record(&path, "first\n").unwrap();
        record(&path, "second\n").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "first\nsecond\n");
        fs::remove_dir_all(directory).unwrap();
    }
}
