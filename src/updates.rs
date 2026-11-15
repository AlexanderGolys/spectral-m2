//! Whether a newer sp2 or m2-ls than the one running exists: installed since it
//! started, or published on crates.io. Purely a courtesy: whatever cannot be
//! asked (no network, no cargo) is skipped without a word.

use std::fmt::{self, Display, Formatter};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use semver::Version;

use crate::code::semantic::SERVER_EXECUTABLE;

/// `cargo search <crate> --limit 1`: the crate itself is the first hit.
const CARGO_SEARCH: [&str; 3] = ["search", "--limit", "1"];

/// A program whose updates are announced.
#[derive(Clone, Copy, Debug)]
pub struct Program {
    /// What it is run as, and so found on `PATH` as.
    executable: &'static str,
    /// What it is published on crates.io as.
    published_as: &'static str,
}

pub const SHELL: Program = Program {
    executable: env!("CARGO_BIN_NAME"),
    published_as: env!("CARGO_PKG_NAME"),
};

pub const LANGUAGE_SERVER: Program = Program {
    executable: SERVER_EXECUTABLE,
    published_as: SERVER_EXECUTABLE,
};

/// Where a newer version was found, which decides how to get it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
    Published,
    Installed,
}

/// A version of `program` newer than the one running, e.g. displayed as
/// `m2-ls 2.8.0 is out (running 2.7.0): cargo install m2-ls`.
#[derive(Debug)]
pub struct Update {
    program: Program,
    running: Version,
    newer: Version,
    source: Source,
}

impl Program {
    /// The version `PATH`'s binary reports as the last word of `--version`.
    fn installed(self) -> Option<Version> {
        let output = Command::new(self.executable)
            .arg("--version")
            .output()
            .ok()?;
        Version::parse(
            String::from_utf8(output.stdout)
                .ok()?
                .split_whitespace()
                .last()?,
        )
        .ok()
    }

    /// The newest version on crates.io, from the `name = "version"` line of its
    /// search hit.
    fn published(self) -> Option<Version> {
        let output = Command::new("cargo")
            .args(CARGO_SEARCH)
            .arg(self.published_as)
            .output()
            .ok()?;
        let hit = String::from_utf8(output.stdout).ok()?;
        let quoted = hit
            .lines()
            .next()?
            .strip_prefix(self.published_as)?
            .trim_start()
            .strip_prefix("= \"")?;
        Version::parse(&quoted[..quoted.find('"')?]).ok()
    }

    /// The newest version this machine could run instead of `running`, if any is
    /// newer; installed wins a tie, being one relaunch away.
    fn update(self, running: Version) -> Option<Update> {
        let found = [
            (self.published(), Source::Published),
            (self.installed(), Source::Installed),
        ];
        let (newer, source) = found
            .into_iter()
            .filter_map(|(version, source)| Some((version?, source)))
            .max()?;
        (newer > running).then_some(Update {
            program: self,
            running,
            newer,
            source,
        })
    }
}

impl Display for Update {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let Update {
            program,
            running,
            newer,
            source,
        } = self;
        write!(
            f,
            "{} {newer} is out (running {running}): ",
            program.executable
        )?;
        match source {
            Source::Installed => write!(f, "quit and start sp2 again to use it"),
            Source::Published => write!(f, "cargo install {}", program.published_as),
        }
    }
}

/// Updates found by checks running in the background, collected as they finish.
#[derive(Debug)]
pub struct UpdateChecks {
    found: Receiver<Update>,
    report: Sender<Update>,
}

impl UpdateChecks {
    pub fn new() -> Self {
        let (report, found) = channel();
        Self { found, report }
    }

    /// Look for an update of `program`, running `version`; a version that is not
    /// semver (or unknown) has nothing to compare against.
    pub fn check(&self, program: Program, version: Option<&str>) {
        let Some(running) = version.and_then(|version| Version::parse(version).ok()) else {
            return;
        };
        let report = self.report.clone();
        thread::spawn(move || program.update(running).map(|update| report.send(update)));
    }

    /// Updates found since last asked.
    pub fn found(&self) -> impl Iterator<Item = Update> + '_ {
        self.found.try_iter()
    }
}
