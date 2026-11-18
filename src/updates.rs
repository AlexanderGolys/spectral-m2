//! Whether a newer sp2, m2-ls or Spectral package than the one running exists:
//! installed since it started, published on crates.io, or for the package,
//! carried by this sp2. Purely a courtesy: whatever cannot be asked (no network,
//! no cargo) is skipped without a word, and nothing is ever updated for the user.

use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use semver::Version;

use crate::backend::{m2_string_literal, M2Backend};
use crate::code::semantic::SERVER_EXECUTABLE;
use crate::ui::config::UI;

/// `cargo search <crate> --limit 1`: the crate itself is the first hit.
const CARGO_SEARCH: [&str; 3] = ["search", "--limit", "1"];

/// Follows a running version wherever it is shown once a newer one is known.
const OUTDATED_MARK: &str = "*";

/// The newest version found of each program, by executable; what is already here
/// is not announced again.
static NEWEST: Mutex<BTreeMap<&str, Version>> = Mutex::new(BTreeMap::new());

/// A program whose updates are announced.
#[derive(Clone, Copy, Debug)]
pub struct Program {
    /// What it is called; for an executable, what it is run as, and so found on
    /// `PATH` as.
    name: &'static str,
    /// What it is published on crates.io as.
    published_as: &'static str,
    /// Where newer versions of it are looked for.
    sources: &'static [Source],
}

/// Where newer versions of a program published on crates.io are found.
const PUBLISHED_EXECUTABLE: &[Source] = &[Source::Published, Source::Installed];

pub const SHELL: Program = Program {
    name: env!("CARGO_BIN_NAME"),
    published_as: env!("CARGO_PKG_NAME"),
    sources: PUBLISHED_EXECUTABLE,
};

pub const LANGUAGE_SERVER: Program = Program {
    name: SERVER_EXECUTABLE,
    published_as: SERVER_EXECUTABLE,
    sources: PUBLISHED_EXECUTABLE,
};

/// The package results are drawn with: the user's own, which a newer sp2 only
/// tells them it carries a newer version of.
pub const SPECTRAL: Program = Program {
    name: "Spectral",
    published_as: "Spectral",
    sources: &[Source::Carried],
};

/// Where a newer version was found, which decides how to get it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
    Published,
    Installed,
    /// The `Spectral` this sp2 was built with.
    Carried,
}

impl Source {
    /// Every source, ordered so installed wins a tie, being one relaunch away.
    const EACH: [Self; 3] = [Self::Published, Self::Installed, Self::Carried];

    fn find(self, program: Program) -> Option<Version> {
        match self {
            Self::Published => program.published(),
            Self::Installed => program.installed(),
            Self::Carried => Version::parse(env!("SPECTRAL_VERSION")).ok(),
        }
    }

    /// How long one look holds: asking the installed binary is a local
    /// `--version`, asking crates.io a network search, and what sp2 carries
    /// cannot change while it runs.
    fn interval(self) -> Duration {
        match self {
            Self::Published => UI.published_check_interval,
            Self::Installed => UI.installed_check_interval,
            Self::Carried => Duration::MAX,
        }
    }
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
    /// `running` as every view shows it: `1.3.2*` once a newer version is known.
    pub fn version(self, running: &str) -> String {
        let outdated = Version::parse(running)
            .is_ok_and(|running| self.newest().is_some_and(|newest| newest > running));
        match outdated {
            true => format!("{running}{OUTDATED_MARK}"),
            false => running.to_string(),
        }
    }

    fn newest(self) -> Option<Version> {
        Self::known().get(self.name).cloned()
    }

    /// Record `found` as the newest version; whether it is news.
    fn learn(self, found: &Version) -> bool {
        let mut known = Self::known();
        let news = known.get(self.name).is_none_or(|newest| found > newest);
        if news {
            known.insert(self.name, found.clone());
        }
        news
    }

    fn known() -> MutexGuard<'static, BTreeMap<&'static str, Version>> {
        NEWEST
            .lock()
            .expect("the newest versions are never held across a panic")
    }

    /// The version `PATH`'s binary reports as the last word of `--version`.
    fn installed(self) -> Option<Version> {
        let output = Command::new(self.name).arg("--version").output().ok()?;
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

    /// The newest version those of `sources` it is looked for in offer instead
    /// of `running`, if any is newer.
    fn update(self, running: Version, sources: &[Source]) -> Option<Update> {
        let (newer, source) = sources
            .iter()
            .filter(|source| self.sources.contains(source))
            .filter_map(|&source| Some((source.find(self)?, source)))
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
        write!(f, "{} {newer} is out (running {running}): ", program.name)?;
        match source {
            Source::Installed => write!(f, "quit and start sp2 again to use it"),
            Source::Published => write!(f, "cargo install {}", program.published_as),
            Source::Carried => write!(
                f,
                "installPackage({}, FileName => {}), then restart",
                m2_string_literal(program.name),
                m2_string_literal(M2Backend::carried_package().unwrap_or_default().display())
            ),
        }
    }
}

/// Updates found by checks running in the background, collected as they finish;
/// each newer version is reported once, however often it is found again.
#[derive(Debug)]
pub struct UpdateChecks {
    found: Receiver<Update>,
    report: Sender<Update>,
    /// When each source was last looked at; one never looked at is due.
    checked: BTreeMap<Source, Instant>,
}

impl UpdateChecks {
    pub fn new() -> Self {
        let (report, found) = channel();
        Self {
            found,
            report,
            checked: BTreeMap::new(),
        }
    }

    /// Look for updates of each program, paired with the version of it running, in
    /// every source whose last look is older than its interval.
    pub fn poll(&mut self, programs: &[(Program, Option<&str>)]) {
        let due = Source::EACH
            .into_iter()
            .filter(|source| {
                self.checked
                    .get(source)
                    .is_none_or(|at| at.elapsed() >= source.interval())
            })
            .collect::<Vec<_>>();
        if due.is_empty() {
            return;
        }
        let now = Instant::now();
        self.checked.extend(due.iter().map(|&source| (source, now)));
        for &(program, version) in programs {
            self.check(program, version, due.clone());
        }
    }

    /// Make every source due at the next poll, as when what runs has changed.
    pub fn forget(&mut self) {
        self.checked.clear();
    }

    /// Look in `sources` for an update of `program`, running `version`; a version
    /// that is not semver (or unknown) has nothing to compare against.
    fn check(&self, program: Program, version: Option<&str>, sources: Vec<Source>) {
        let Some(running) = version.and_then(|version| Version::parse(version).ok()) else {
            return;
        };
        let report = self.report.clone();
        // A report nobody is left to receive is moot: the shell is closing.
        thread::spawn(move || {
            let news = program
                .update(running, &sources)
                .filter(|update| program.learn(&update.newer));
            news.map(|update| report.send(update).ok());
        });
    }

    /// Updates found since last asked.
    pub fn found(&self) -> impl Iterator<Item = Update> + '_ {
        self.found.try_iter()
    }
}
