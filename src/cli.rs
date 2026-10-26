//! The command line the shell starts from.

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use clap::{Args, Parser};

use crate::code::file as code_file;

/// Where the rest of the shell is configured, shown beneath the options.
const INSIDE_THE_SHELL: &str = "Settings, keymaps and themes live inside the shell:\n\
Ctrl+O opens the settings and Ctrl+/ lists every command.";

/// What the shell is started with.
#[derive(Debug, Parser)]
#[command(name = env!("CARGO_BIN_NAME"), version, about, after_help = INSIDE_THE_SHELL)]
pub struct Options {
    /// Run the Macaulay2 code in FILE as the session starts, cell by cell
    #[arg(value_name = "FILE", conflicts_with = "resume")]
    file: Option<String>,
    /// Run the code of the previous session as the session starts
    #[arg(short, long)]
    resume: bool,
    /// M2 configuration applied to the initial process and each restart.
    #[command(flatten)]
    pub m2: M2Options,
}

/// Macaulay2 initialization and runtime options retained across restarts.
#[derive(Args, Clone, Debug, Default)]
#[group(skip)]
pub struct M2Options {
    /// Skip user initialization and packages in the home directory
    #[arg(short = 'q', help_heading = "Macaulay2 startup")]
    no_user_init: bool,
    /// Evaluate an expression before M2 initialization (repeatable)
    #[arg(
        short = 'E',
        value_name = "EXPR",
        allow_hyphen_values = true,
        help_heading = "Macaulay2 startup"
    )]
    before_init: Vec<String>,
    /// Evaluate an expression after M2 initialization (repeatable)
    #[arg(
        short = 'e',
        value_name = "EXPR",
        allow_hyphen_values = true,
        help_heading = "Macaulay2 startup"
    )]
    after_init: Vec<String>,
    /// Skip preloaded packages
    #[arg(long, help_heading = "Macaulay2 startup")]
    no_preload: bool,
    /// Set M2's prefix directory
    #[arg(long, value_name = "DIR", help_heading = "Macaulay2 startup")]
    prefix: Option<PathBuf>,
    /// Add an M2 source or build tree to the initial path
    #[arg(long, value_name = "DIR", help_heading = "Macaulay2 startup")]
    srcdir: Option<PathBuf>,
    /// Suppress error backtraces
    #[arg(long, help_heading = "Macaulay2 startup")]
    no_backtrace: bool,
    /// Do not enter the debugger on errors
    #[arg(long, help_heading = "Macaulay2 startup")]
    no_debug: bool,
    /// Turn M2 warning N into an error
    #[arg(long, value_name = "N", help_heading = "Macaulay2 startup")]
    debug: Option<u32>,
    /// Report files loaded and command-line expressions evaluated
    #[arg(long, help_heading = "Macaulay2 startup")]
    notify: bool,
    /// Set M2's output width
    #[arg(long, value_name = "N", help_heading = "Macaulay2 startup")]
    print_width: Option<u32>,
    /// Use M2's fixed initial random seed
    #[arg(long, help_heading = "Macaulay2 startup")]
    no_randomize: bool,
    /// Disable additional M2 threads and tasks
    #[arg(long, help_heading = "Macaulay2 startup")]
    no_threads: bool,
    /// Make M2's current time always zero
    #[arg(long, help_heading = "Macaulay2 startup")]
    no_time: bool,
}

impl M2Options {
    /// Whether user initialization files should be loaded.
    pub fn loads_init(&self) -> bool {
        !self.no_user_init
    }

    /// Arguments passed directly to M2 before the backend hooks are installed.
    pub fn arguments(&self) -> Vec<OsString> {
        let mut args = Vec::new();
        for (enabled, flag) in [
            (self.no_user_init, "-q"),
            (self.no_preload, "--no-preload"),
            (self.no_backtrace, "--no-backtrace"),
            (self.no_debug, "--no-debug"),
            (self.notify, "--notify"),
            (self.no_randomize, "--no-randomize"),
            (self.no_threads, "--no-threads"),
            (self.no_time, "--no-time"),
        ] {
            if enabled {
                args.push(OsString::from(flag));
            }
        }
        for (flag, path) in [("--prefix", &self.prefix), ("--srcdir", &self.srcdir)] {
            if let Some(path) = path {
                args.extend([OsString::from(flag), path.as_os_str().to_owned()]);
            }
        }
        for (flag, number) in [("--debug", self.debug), ("--print-width", self.print_width)] {
            if let Some(number) = number {
                args.extend([OsString::from(flag), number.to_string().into()]);
            }
        }
        for (flag, expressions) in [("-E", &self.before_init), ("-e", &self.after_init)] {
            for expression in expressions {
                args.extend([OsString::from(flag), expression.into()]);
            }
        }
        args
    }
}

impl Options {
    /// The code to run as the session starts, read now so a file that cannot be
    /// read is reported before the shell takes over the terminal.
    pub fn startup_code(&self) -> Result<Option<String>, String> {
        let path = match (&self.file, self.resume) {
            (Some(file), _) => code_file::resolve(file),
            (None, true) => code_file::previous_session()
                .filter(|path| path.exists())
                .ok_or("there is no previous session to resume")?,
            (None, false) => return Ok(None),
        };
        fs::read_to_string(&path)
            .map(Some)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m2_startup_options_preserve_values_and_repeated_expressions() {
        let options = Options::try_parse_from([
            "sp2",
            "-q",
            "--no-preload",
            "--no-backtrace",
            "--no-debug",
            "--notify",
            "--no-randomize",
            "--no-threads",
            "--no-time",
            "--prefix",
            "/some prefix",
            "--srcdir",
            "/source tree",
            "--debug",
            "3",
            "--print-width",
            "72",
            "-E",
            "print 1",
            "-E",
            "print 2",
            "-e",
            "x = 3",
            "-e",
            "-1",
            "ring.m2",
        ])
        .unwrap();
        assert_eq!(
            options.m2.arguments(),
            [
                "-q",
                "--no-preload",
                "--no-backtrace",
                "--no-debug",
                "--notify",
                "--no-randomize",
                "--no-threads",
                "--no-time",
                "--prefix",
                "/some prefix",
                "--srcdir",
                "/source tree",
                "--debug",
                "3",
                "--print-width",
                "72",
                "-E",
                "print 1",
                "-E",
                "print 2",
                "-e",
                "x = 3",
                "-e",
                "-1",
            ]
            .map(OsString::from)
        );
        assert!(!options.m2.loads_init());
        assert_eq!(options.file.as_deref(), Some("ring.m2"));
    }

    #[test]
    fn invalid_startup_numbers_and_missing_values_are_rejected() {
        for args in [
            vec!["sp2", "--print-width", "invalid"],
            vec!["sp2", "--debug", "-1"],
            vec!["sp2", "--prefix"],
            vec!["sp2", "-e"],
        ] {
            assert!(Options::try_parse_from(args).is_err());
        }
    }

    #[test]
    fn a_file_and_resuming_cannot_both_open_the_session() {
        assert!(Options::try_parse_from(["sp2", "ring.m2", "--resume"]).is_err());
    }

    #[test]
    fn a_file_missing_is_reported_before_the_shell_starts() -> Result<(), clap::Error> {
        let options = Options::try_parse_from(["sp2", "/nonexistent/ring.m2"])?;
        let error = options.startup_code().unwrap_err();
        assert!(error.contains("/nonexistent/ring.m2"), "{error}");
        Ok(())
    }

    #[test]
    fn no_arguments_start_an_empty_session() -> Result<(), clap::Error> {
        assert_eq!(Options::try_parse_from(["sp2"])?.startup_code(), Ok(None));
        Ok(())
    }
}
