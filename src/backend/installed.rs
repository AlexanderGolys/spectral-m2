//! The symbols installed packages document, for suggesting the package that
//! binds a name an evaluation found unbound.

use std::collections::{BTreeSet, HashMap};
use std::fmt::{self, Display};
use std::sync::mpsc::{self, Receiver};
use std::sync::LazyLock;
use std::thread;
use std::time::Duration;

use regex::Regex;

use super::M2Backend;
use crate::code::file::without_comments;
use crate::errors::Result;

/// Prints a `name package` line for every symbol an installed package documents
/// that a fresh session leaves unbound.
const LISTING: &str = include_str!("m2_installed_symbols.m2");
/// How long reading every installed package's documentation may take; it is a
/// matter of a second.
const LISTING_TIMEOUT: Duration = Duration::from_secs(60);

/// A value an error names that is still a bare symbol, as M2 shows it beside
/// the error's other operands: `            groebnerFan (of class Symbol)`.
static UNBOUND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\S+) \(of class Symbol\)").expect("valid pattern"));

/// Which installed packages document each symbol a fresh session leaves
/// unbound, read from the documentation installed beside them: `Digraph` is
/// documented by `Graphs`, `gfan` by `gfanInterface`.
#[derive(Debug, Default)]
pub struct InstalledSymbols {
    packages: HashMap<String, BTreeSet<String>>,
}

impl InstalledSymbols {
    /// Read in an interpreter of its own, on a thread of its own; the receiver
    /// gets nothing should M2 fail to tell.
    pub fn read_in_background() -> Receiver<Self> {
        let (sender, read) = mpsc::channel();
        thread::spawn(move || {
            if let Ok(installed) = Self::read() {
                let _ = sender.send(installed);
            }
        });
        read
    }

    fn read() -> Result<Self> {
        let response = M2Backend::spawn_for_examples(None)?
            .evaluate_within(without_comments(LISTING).trim(), LISTING_TIMEOUT)?;
        Ok(response.transcript.lines().collect())
    }

    /// The packages documenting `name`, in alphabetical order.
    pub fn documenting(&self, name: &str) -> impl Iterator<Item = &str> {
        self.packages
            .get(name)
            .into_iter()
            .flatten()
            .map(String::as_str)
    }

    /// Every package documenting a symbol whose name starts with `prefix`.
    pub fn packages_starting_with(&self, prefix: &str) -> Vec<String> {
        let packages: BTreeSet<&String> = self.packages.values().flatten().collect();
        packages
            .into_iter()
            .filter(|package| package.starts_with(prefix))
            .cloned()
            .collect()
    }

    /// The packages to load for the first value `diagnostic` shows as a bare
    /// symbol that an installed package documents.
    pub fn suggestion(&self, diagnostic: &str) -> Option<ImportSuggestion> {
        UNBOUND.captures_iter(diagnostic).find_map(|unbound| {
            let name = &unbound[1];
            let packages: Vec<String> = self.documenting(name).map(str::to_string).collect();
            (!packages.is_empty()).then(|| ImportSuggestion {
                name: name.to_string(),
                packages,
            })
        })
    }
}

impl<'a> FromIterator<&'a str> for InstalledSymbols {
    /// Collected from `name package` lines; any other line is no listing's.
    fn from_iter<I: IntoIterator<Item = &'a str>>(lines: I) -> Self {
        let mut packages: HashMap<String, BTreeSet<String>> = HashMap::new();
        for (name, package) in lines.into_iter().filter_map(|line| line.split_once(' ')) {
            packages
                .entry(name.to_string())
                .or_default()
                .insert(package.to_string());
        }
        Self { packages }
    }
}

/// An unbound name an error showed and the installed packages documenting it,
/// as for `tropicalVariety` and `{"Tropical"}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportSuggestion {
    pub name: String,
    /// Never empty, in alphabetical order.
    pub packages: Vec<String>,
}

impl ImportSuggestion {
    /// The package loaded unless another is named.
    pub fn package(&self) -> &str {
        &self.packages[0]
    }
}

impl Display for ImportSuggestion {
    /// `tropicalVariety is documented in Tropical`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} is documented in {}",
            self.name,
            self.packages.join(", ")
        )
    }
}
