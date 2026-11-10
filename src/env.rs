//! The process environment: every variable the shell reads, and the directories
//! it derives from them, asked for through [`ENV`].

#[cfg(test)]
use std::env::temp_dir;
use std::env::{current_dir, var, var_os};
use std::io;
use std::path::PathBuf;

const HOME: &str = "HOME";
const XDG_CONFIG_HOME: &str = "XDG_CONFIG_HOME";
const XDG_STATE_HOME: &str = "XDG_STATE_HOME";
/// Where configuration lives under the home directory without `XDG_CONFIG_HOME`.
const CONFIG_UNDER_HOME: &str = ".config";
/// Where state lives under the home directory without `XDG_STATE_HOME`.
const STATE_UNDER_HOME: &str = ".local/state";
/// Set in every process tmux runs.
const TMUX: &str = "TMUX";
const TERM: &str = "TERM";
/// Variables naming the terminal outside tmux, most specific first.
const TERMINAL_VARIABLES: [&str; 2] = ["TERM_PROGRAM", TERM];
/// The `TERM` of a terminal that draws nothing beyond plain text.
const DUMB_TERMINAL: &str = "dumb";
/// Set to [`ENABLED`] to draw with plain Unicode characters only.
const BASIC_GLYPHS: &str = "SPECTRAL_M2_BASIC_GLYPHS";
const ENABLED: &str = "1";

/// The one way into the process environment.
pub static ENV: Environment = Environment;

/// Reads the process environment on behalf of the whole shell.
#[derive(Clone, Copy, Debug, Default)]
pub struct Environment;

impl Environment {
    pub fn home(&self) -> Option<PathBuf> {
        var_os(HOME).map(PathBuf::from)
    }

    /// `XDG_CONFIG_HOME`, else `~/.config`.
    pub fn config_home(&self) -> Option<PathBuf> {
        self.xdg_home(XDG_CONFIG_HOME, CONFIG_UNDER_HOME)
    }

    /// `XDG_STATE_HOME`, else `~/.local/state`.
    pub fn state_home(&self) -> Option<PathBuf> {
        self.xdg_home(XDG_STATE_HOME, STATE_UNDER_HOME)
    }

    #[cfg(test)]
    pub fn temp_dir(&self) -> PathBuf {
        temp_dir()
    }

    pub fn current_dir(&self) -> io::Result<PathBuf> {
        current_dir()
    }

    pub fn in_tmux(&self) -> bool {
        var_os(TMUX).is_some()
    }

    /// The names the terminal goes by outside tmux, most specific first.
    pub fn terminal_names(&self) -> Vec<String> {
        TERMINAL_VARIABLES
            .iter()
            .filter_map(|variable| var(variable).ok())
            .collect()
    }

    /// Whether the terminal or the user asks for plain Unicode characters only.
    pub fn basic_glyphs(&self) -> bool {
        var(TERM).is_ok_and(|term| term == DUMB_TERMINAL)
            || var(BASIC_GLYPHS).is_ok_and(|value| value == ENABLED)
    }

    fn xdg_home(&self, variable: &str, under_home: &str) -> Option<PathBuf> {
        var_os(variable)
            .map(PathBuf::from)
            .or_else(|| self.home().map(|home| home.join(under_home)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_directories_fall_back_under_the_home_directory() {
        let Some(home) = ENV.home() else {
            return;
        };
        if var_os(XDG_STATE_HOME).is_none() {
            assert_eq!(ENV.state_home(), Some(home.join(STATE_UNDER_HOME)));
        }
        if var_os(XDG_CONFIG_HOME).is_none() {
            assert_eq!(ENV.config_home(), Some(home.join(CONFIG_UNDER_HOME)));
        }
    }
}
