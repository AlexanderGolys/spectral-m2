//! The process environment: every variable the shell reads, and the directories
//! it derives from them, asked for through [`ENV`] by a method named after what
//! is read.

use std::env::{current_dir, var_os};
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

use crate::updates::SHELL;

// @##magic
/// Where configuration lives under the home directory without `XDG_CONFIG_HOME`.
const CONFIG_UNDER_HOME: &str = ".config";
/// Where state lives under the home directory without `XDG_STATE_HOME`.
const STATE_UNDER_HOME: &str = ".local/state";

/// The one way into the process environment.
pub static ENV: Environment = Environment;

/// Reads the process environment on behalf of the whole shell: each variable by
/// the method named after it, and what the shell derives from them.
///
/// Every program starts with a table of named texts handed down from whatever
/// started it, its environment: the shell that launched it, and before that the
/// terminal and the login. Programs leave each other notes there by convention —
/// where the user's files live, what kind of terminal is drawing — so reading a
/// variable is asking that convention a question; an unset one has no answer.
///
/// ```ignore
/// let config = ENV.config_home();       // XDG_CONFIG_HOME, else ~/.config
/// let term = ENV.term();                // TERM, as text
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct Environment;

/// What a variable's value is read as; `None` when it does not read as one.
trait Value: Sized {
    fn read(value: OsString) -> Option<Self>;
}

impl Value for OsString {
    fn read(value: OsString) -> Option<Self> {
        Some(value)
    }
}

impl Value for PathBuf {
    fn read(value: OsString) -> Option<Self> {
        Some(value.into())
    }
}

impl Value for String {
    fn read(value: OsString) -> Option<Self> {
        value.into_string().ok()
    }
}

/// Gives [`Environment`] a reader for each variable, named after it in lowercase:
/// `xdg_config_home -> PathBuf` reads `XDG_CONFIG_HOME` as a path.
macro_rules! variables {
    ($($(#[$attribute:meta])* $name:ident -> $value:ty;)*) => {
        impl Environment {
            $(
                $(#[$attribute])*
                pub fn $name(&self) -> Option<$value> {
                    var_os(stringify!($name).to_uppercase()).and_then(<$value>::read)
                }
            )*
        }
    };
}

variables! {
    /// The user's home directory, the `~` of paths.
    home -> PathBuf;
    /// Where the user keeps programs' configuration, by the XDG base-directory
    /// convention most Linux programs follow. Usually unset, meaning `~/.config`.
    xdg_config_home -> PathBuf;
    /// Where programs keep what builds up as they run and is wanted next time,
    /// such as history and logs, by the same convention. Usually unset, meaning
    /// `~/.local/state`.
    xdg_state_home -> PathBuf;
    /// Set by tmux, which runs several terminal sessions inside one terminal window
    /// and keeps them alive when the window closes, in every program it starts.
    /// Only whether it is set matters: the value locates tmux's own server.
    tmux -> OsString;
    /// The kind of terminal, as the terminal declares it to the programs it runs:
    /// which control sequences it understands, as in `xterm-256color`. Many
    /// terminals claim a generic kind, a few their own, as in `xterm-ghostty`.
    term -> String;
    /// The terminal application's own name, as in `ghostty` or `WezTerm`, set by
    /// some terminals only. More telling than `TERM` when present.
    term_program -> String;
}

impl Environment {
    /// `XDG_CONFIG_HOME`, else `~/.config`.
    pub fn config_home(&self) -> Option<PathBuf> {
        self.under_home_unless(self.xdg_config_home(), CONFIG_UNDER_HOME)
    }

    /// `XDG_STATE_HOME`, else `~/.local/state`.
    pub fn state_home(&self) -> Option<PathBuf> {
        self.under_home_unless(self.xdg_state_home(), STATE_UNDER_HOME)
    }

    /// The version of sp2 itself, as every view shows it: marked once a newer one
    /// is known.
    pub fn shell_version(&self) -> String {
        SHELL.version(env!("CARGO_PKG_VERSION"))
    }

    pub fn current_dir(&self) -> io::Result<PathBuf> {
        current_dir()
    }

    pub fn in_tmux(&self) -> bool {
        self.tmux().is_some()
    }

    /// The names the terminal goes by outside tmux, most specific first.
    pub fn terminal_names(&self) -> Vec<String> {
        [self.term_program(), self.term()]
            .into_iter()
            .flatten()
            .collect()
    }

    /// `set` when a variable gave it, else `under_home` in the home directory.
    fn under_home_unless(&self, set: Option<PathBuf>, under_home: &str) -> Option<PathBuf> {
        set.or_else(|| self.home().map(|home| home.join(under_home)))
    }
}
