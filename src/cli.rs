//! The command line the shell starts from.

use std::fs;

use clap::Parser;

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
