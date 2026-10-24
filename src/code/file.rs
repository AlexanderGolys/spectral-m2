//! The session's code as `.m2` files: saved on request, read back to run, and
//! kept on every exit and restart so the next session can resume it. Typed
//! file names complete to the entries they can name.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The extension a file of Macaulay2 code carries.
const EXTENSION: &str = "m2";

/// The file a typed name refers to: a leading `~` is the home directory, and a
/// name without an extension gets `.m2`.
pub fn resolve(typed: &str) -> PathBuf {
    let path = expand_home(typed);
    match path.extension() {
        Some(_) => path,
        None => path.with_extension(EXTENSION),
    }
}

/// `typed` with a leading `~` read as the home directory.
fn expand_home(typed: &str) -> PathBuf {
    match (typed.strip_prefix("~/"), env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(typed),
    }
}

/// `path` spelled with the home directory as a leading `~`.
pub fn abbreviate_home(path: &Path) -> String {
    match env::var_os("HOME").and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Which directory entries a typed path may complete to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Entries {
    /// Directories and files of Macaulay2 code.
    Code,
    /// Every directory and file.
    All,
}

/// The paths `typed` completes to, spelled as it is typed: its directory as
/// written, then an entry whose name starts with the rest, a directory's with
/// a trailing `/`. Hidden entries are offered only to a name starting with `.`.
/// Directories come first, then files, each alphabetically.
pub fn completions(typed: &str, entries: Entries) -> Vec<String> {
    let (directory, name) = typed
        .rfind('/')
        .map_or(("", typed), |slash| typed.split_at(slash + 1));
    let listed = match directory {
        "" => PathBuf::from("."),
        directory => expand_home(directory),
    };
    let Ok(listing) = fs::read_dir(listed) else {
        return Vec::new();
    };
    let mut found: Vec<(bool, String)> = listing
        .flatten()
        .filter_map(|entry| {
            let entry_name = entry.file_name().into_string().ok()?;
            let is_directory = fs::metadata(entry.path()).is_ok_and(|meta| meta.is_dir());
            let shown = entry_name.starts_with(name)
                && (name.starts_with('.') || !entry_name.starts_with('.'))
                && (is_directory
                    || entries == Entries::All
                    || Path::new(&entry_name)
                        .extension()
                        .is_some_and(|ext| ext == EXTENSION));
            shown.then(|| {
                let slash = if is_directory { "/" } else { "" };
                (!is_directory, format!("{directory}{entry_name}{slash}"))
            })
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, path)| path).collect()
}

/// Write `code` to `path`, creating the directories it needs.
pub fn write(path: &Path, code: &str) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, code)
}

/// Where the code of the previous session is kept.
#[cfg(not(test))]
pub fn previous_session() -> Option<PathBuf> {
    let state = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))?;
    Some(
        state
            .join(env!("CARGO_PKG_NAME"))
            .join("previous-session")
            .with_extension(EXTENSION),
    )
}

/// Tests keep no previous session, so they never overwrite a developer's.
#[cfg(test)]
pub fn previous_session() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_name_becomes_an_m2_file_and_an_explicit_extension_stays() {
        assert_eq!(resolve("notes"), PathBuf::from("notes.m2"));
        assert_eq!(resolve("ideal.m2"), PathBuf::from("ideal.m2"));
        assert_eq!(resolve("ring.txt"), PathBuf::from("ring.txt"));
    }

    #[test]
    fn a_typed_path_completes_to_the_entries_it_can_name() {
        let root = env::temp_dir().join(format!("spectral-m2-completion-{}", std::process::id()));
        for directory in ["ring", "rational"] {
            fs::create_dir_all(root.join(directory)).unwrap();
        }
        for file in ["resolution.m2", "readme.txt", ".recent.m2"] {
            fs::write(root.join(file), "").unwrap();
        }
        let typed = format!("{}/r", root.display());
        let named = |entries| -> Vec<String> {
            completions(&typed, entries)
                .into_iter()
                .map(|path| path[typed.len() - 1..].to_string())
                .collect()
        };

        assert_eq!(
            named(Entries::Code),
            ["rational/", "ring/", "resolution.m2"]
        );
        assert_eq!(
            named(Entries::All),
            ["rational/", "ring/", "readme.txt", "resolution.m2"]
        );
        let hidden = completions(&format!("{}/.r", root.display()), Entries::Code);
        assert_eq!(hidden.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_leading_tilde_is_the_home_directory() {
        let Some(home) = env::var_os("HOME") else {
            return;
        };
        assert_eq!(
            resolve("~/work/ring"),
            PathBuf::from(home).join("work/ring.m2")
        );
    }

    #[test]
    fn the_home_directory_is_spelled_as_a_tilde() {
        let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
            return;
        };
        assert_eq!(abbreviate_home(&home), "~");
        assert_eq!(abbreviate_home(&home.join("work/ring")), "~/work/ring");
        assert_eq!(abbreviate_home(Path::new("/srv/m2")), "/srv/m2");
    }
}
