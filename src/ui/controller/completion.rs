//! What the input completes, chosen by what the cursor is in the middle of: a
//! command's name, the file a command takes, a file named in a string, or the
//! name of something M2 knows. Names are the ones M2's own readline completes,
//! reported by M2 as it starts and as each cell binds more.

mod usage;

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use m2_syn::{lex_str_recovering, CommentKind, LexError, LexErrorKind, Spanned, TokenTree};

use self::usage::Usage;
use super::commands::{self, Command, Operand};
use crate::backend::installed::InstalledSymbols;
use crate::code::file::{self, Entries};
use crate::input::Completion;
use crate::ui::settings::editor::CompletionSort;

/// What opens a raw string literal, which a path typed inside one follows.
const RAW_STRING_OPENER: &str = "///";

/// Every name M2 has reported, those the session bound kept apart from those
/// it started with.
pub struct Names {
    started: BTreeSet<String>,
    bound: BTreeSet<String>,
    /// Names the language server reads as bound in the session's source,
    /// the input included, before M2 has run them.
    analysed: BTreeSet<String>,
    locals: BTreeSet<String>,
    usage: Usage,
    /// Frequencies for names introduced by this session, never saved to disk.
    session_usage: BTreeMap<String, u64>,
}

impl Default for Names {
    fn default() -> Self {
        Self {
            started: BTreeSet::new(),
            bound: BTreeSet::new(),
            analysed: BTreeSet::new(),
            locals: BTreeSet::new(),
            usage: Usage::load(),
            session_usage: BTreeMap::new(),
        }
    }
}

impl Names {
    /// Forget every name for those a freshly started M2 knows.
    pub fn reset(&mut self, names: Vec<String>) {
        self.started = names.into_iter().collect();
        self.bound.clear();
        self.locals.clear();
        self.session_usage.clear();
    }

    /// Add names the session has bound since.
    pub fn bind(&mut self, names: Vec<String>) {
        self.bound.extend(names);
    }

    /// Replace the names the language server last read as bound.
    pub fn analyse(&mut self, names: impl IntoIterator<Item = String>) {
        self.analysed = names.into_iter().collect();
    }

    /// Save library-symbol frequencies and retain session-name frequencies only in memory.
    pub fn record(&mut self, source: &str) {
        let used = usage::references(source, &mut self.locals);
        let mut library = Vec::new();
        for name in used {
            if self.started.contains(&name) {
                library.push(name);
            } else if self.bound.contains(&name) {
                let count = self.session_usage.entry(name).or_default();
                *count = count.saturating_add(1);
            }
        }
        self.usage.record(library);
    }

    /// The saved library count or the current session's count for a candidate.
    fn frequency(&self, name: &str) -> u64 {
        if self.started.contains(name) {
            self.usage.count(name)
        } else {
            self.session_usage.get(name).copied().unwrap_or_default()
        }
    }

    /// Whether M2 knows `name`, so there is nothing left to complete.
    fn knows(&self, name: &str) -> bool {
        self.started.contains(name) || self.bound.contains(name)
    }

    /// Prefix matches ranked by usage, then by length and name for stable ties.
    fn starting_with(&self, prefix: &str, sort: CompletionSort) -> Vec<String> {
        let matching = |names: &BTreeSet<String>| {
            names
                .range(prefix.to_string()..)
                .take_while(|name| name.starts_with(prefix))
                .cloned()
                .collect::<Vec<String>>()
        };
        let mut found = matching(&self.bound);
        found.extend(
            matching(&self.analysed)
                .into_iter()
                .filter(|name| name != prefix && !self.knows(name)),
        );
        found.extend(
            matching(&self.started)
                .into_iter()
                .filter(|name| !self.bound.contains(name)),
        );
        match sort {
            CompletionSort::Usage => found.sort_by(|left, right| {
                (Reverse(self.frequency(left)), left.len(), left).cmp(&(
                    Reverse(self.frequency(right)),
                    right.len(),
                    right,
                ))
            }),
            CompletionSort::Alphabetical => found.sort(),
            CompletionSort::Length => {
                found.sort_by(|left, right| (left.len(), left).cmp(&(right.len(), right)))
            }
        }
        found
    }
}

/// What the source before the cursor ends in the middle of, as M2 reads it.
#[derive(Debug, PartialEq, Eq)]
enum Typing {
    /// An identifier, spanning this range.
    Name(Range<usize>),
    /// A string literal, whose text so far spans this range.
    Text(Range<usize>),
    /// Anything else, a comment included.
    Other,
}

/// What `source` ends in the middle of.
fn typing(source: &str) -> Typing {
    let end = source.len();
    let recovered = match lex_str_recovering(source) {
        Ok(recovered) => recovered,
        Err(error) => return unterminated_text(&error, end).unwrap_or(Typing::Other),
    };
    // A string left open is recovered, and reported among the diagnostics.
    if let Some(text) = recovered
        .diagnostics
        .iter()
        .find_map(|error| unterminated_text(error, end))
    {
        return text;
    }
    // The innermost last token: an unclosed group's last token is inside it.
    let mut chain: Vec<&TokenTree> = Vec::new();
    let mut stream = &recovered.value;
    while let Some(last) = stream.iter().next_back() {
        chain.push(last);
        match last {
            TokenTree::Group(group) => stream = group.stream(),
            _ => break,
        }
    }
    let commented = chain.iter().any(|tree| {
        tree.span().comments_after().iter().any(|comment| {
            comment.kind() == CommentKind::Line
                && comment.span().range().is_some_and(|range| range.end == end)
        })
    });
    match chain.last() {
        Some(TokenTree::Ident(ident)) if !commented => ident
            .span()
            .range()
            .filter(|range| range.end == end)
            .map_or(Typing::Other, Typing::Name),
        _ => Typing::Other,
    }
}

/// The text typed into a string `error` reports left open, running to `end`.
fn unterminated_text(error: &LexError, end: usize) -> Option<Typing> {
    let opener = match error.kind() {
        LexErrorKind::UnterminatedString => "\"".len(),
        LexErrorKind::UnterminatedRawString => RAW_STRING_OPENER.len(),
        _ => return None,
    };
    let range = error.span().range()?;
    Some(Typing::Text(range.start + opener..end))
}

/// Whether `character` can continue an M2 identifier.
fn continues_name(character: char) -> bool {
    character.is_alphanumeric() || character == '\''
}

/// What the input needs to know to complete its text.
pub struct Request<'a> {
    pub text: &'a str,
    pub cursor: usize,
    /// Incomplete source the input continues.
    pub pending: Option<&'a str>,
    /// The command picked while its name is being typed.
    pub picked: Option<Command>,
    pub names: &'a Names,
    /// The installed packages a command naming one completes from.
    pub installed: &'a InstalledSymbols,
    /// Characters of a name or a string typed before it is completed.
    pub complete_after: usize,
    pub sort: CompletionSort,
}

/// The completion for the text before the cursor, if anything completes it.
pub fn complete(request: Request<'_>) -> Option<Completion> {
    let Request {
        text,
        cursor,
        pending,
        picked,
        names,
        installed,
        complete_after,
        sort,
    } = request;
    if let Some(command) = picked.filter(|_| cursor == text.len()) {
        let mut candidates = vec![commands::completed(command)];
        candidates.extend(
            commands::matching(text.strip_prefix(commands::PREFIX)?)
                .into_iter()
                .filter(|candidate| candidate.action != command.action)
                .map(commands::completed),
        );
        return Completion::new(text, 0..text.len(), candidates);
    }
    if text.starts_with(commands::PREFIX) {
        let (operand, range) = commands::operand(text).filter(|_| cursor == text.len())?;
        let typed = &text[range.clone()];
        let candidates = match operand {
            Operand::File => file::completions(typed, Entries::Code),
            Operand::Package => installed.packages_starting_with(typed),
        };
        return Completion::new(text, range, candidates);
    }
    let pending = pending.map_or(String::new(), |pending| format!("{pending}\n"));
    let in_text = |range: Range<usize>| range.start.max(pending.len()) - pending.len()..cursor;
    match typing(&format!("{pending}{}", &text[..cursor])) {
        Typing::Name(range) if !text[cursor..].starts_with(continues_name) => {
            let range = in_text(range);
            let typed = &text[range.clone()];
            (typed.chars().count() >= complete_after && !names.knows(typed))
                .then(|| Completion::new(text, range, names.starting_with(typed, sort)))?
        }
        Typing::Text(range) => {
            let range = in_text(range);
            let typed = &text[range.clone()];
            (typed.chars().count() >= complete_after && !typed.contains(char::is_whitespace))
                .then(|| Completion::new(text, range, file::completions(typed, Entries::All)))?
        }
        _ => None,
    }
}
