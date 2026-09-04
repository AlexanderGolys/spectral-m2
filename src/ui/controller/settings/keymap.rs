//! Key bindings as data.
//!
//! An action binding consumes its terminal key and emits an [`Action`]. Keys
//! without a binding continue to the focused component as ordinary input.

use std::collections::{HashMap, HashSet};

use crossterm::event::KeyCode;
use KeyCode::*;

use crate::ui::action::Action;
use crate::ui::key::{KeyEvent, KeyMod};
use KeyMod::{Alt, Ctrl, Shift};

/// One action attached to a normalized key.
#[derive(Clone, Copy)]
struct Keymap {
    action: Action,
    remappable: bool,
}

impl Keymap {
    fn new(action: Action) -> Self {
        Self {
            action,
            remappable: action.is_remappable(),
        }
    }

    fn fixed(action: Action) -> Self {
        Self {
            action,
            remappable: false,
        }
    }
}

/// The editor's exact normalized-key bindings.
#[derive(Clone)]
pub struct Keymaps {
    bindings: HashMap<KeyEvent, Keymap>,
    preview: HashMap<KeyEvent, Keymap>,
    reserved: HashSet<KeyEvent>,
}

impl Keymaps {
    /// Resolve a terminal key to an action, after which the key is discarded.
    pub fn action(&self, key: KeyEvent, remapping: bool) -> Option<Action> {
        self.bindings
            .get(&key.binding())
            .filter(|binding| {
                !remapping || !binding.remappable || binding.action == Action::OpenKeymaps
            })
            .map(|binding| binding.action)
    }

    /// Resolve a fixed key owned by preview panes.
    pub fn preview_action(&self, key: KeyEvent) -> Option<Action> {
        self.preview
            .get(&key.binding())
            .map(|binding| binding.action)
    }

    /// The key that triggers `action`, for hints and help.
    pub fn key_for(&self, action: Action) -> Option<String> {
        self.bindings
            .iter()
            .find_map(|(key, binding)| (binding.action == action).then(|| key.to_string()))
    }

    pub fn rebind(&mut self, action: Action, key: KeyEvent) -> bool {
        let key = key.binding();
        if !action.is_remappable()
            || self.reserved.contains(&key)
            || self
                .bindings
                .get(&key)
                .is_some_and(|binding| !binding.remappable)
        {
            return false;
        }
        let Some(target) = self
            .bindings
            .iter()
            .find_map(|(key, binding)| (binding.action == action).then_some(*key))
        else {
            return false;
        };
        if self
            .bindings
            .get(&key)
            .is_some_and(|binding| !binding.remappable)
        {
            return false;
        }
        if key == target {
            return true;
        }
        let binding = self.bindings.remove(&target).unwrap();
        if let Some(conflict) = self.bindings.remove(&key) {
            self.bindings.insert(target, conflict);
        }
        self.bindings.insert(key, binding);
        true
    }
}

macro_rules! keymap {
    ($action:ident <- $key:expr) => {
        (key!($key), Keymap::new(Action::$action))
    };
}

macro_rules! key {
    ($key:expr) => {
        Into::<KeyEvent>::into($key)
    };
}

impl Default for Keymaps {
    fn default() -> Self {
        Self {
            bindings: HashMap::from([
                keymap!(FocusPaneLeft <- Left + Ctrl),
                keymap!(FocusPaneRight <- Right + Ctrl),
                keymap!(ResizePaneLeft <- Left + Ctrl + Shift),
                keymap!(ResizePaneRight <- Right + Ctrl + Shift),
                keymap!(Quit <- Char('c') + Ctrl),
                keymap!(OpenCommands <- Char('/') + Ctrl),
                keymap!(OpenSettings <- Char('o') + Ctrl),
                keymap!(OpenStyle <- Char('s') + Alt),
                keymap!(OpenKeymaps <- Char('k') + Alt),
                keymap!(ClearFeed <- Char('l') + Alt),
                keymap!(OpenCode <- Char('c') + Alt),
                keymap!(NewWorkspace <- Char('n') + Alt),
                keymap!(OpenOutput <- Char('o') + Alt),
                keymap!(OpenSyntaxTree <- Char('t') + Alt),
                keymap!(Restart <- Char('r') + Alt),
                keymap!(ClosePane <- Char('w') + Alt),
                keymap!(ToggleHighlightDebug <- Char('d') + Ctrl),
                keymap!(CopySelection <- Char('y') + Ctrl),
                keymap!(Submit <- Enter),
                keymap!(ScrollFeedUp <- PageUp),
                keymap!(ScrollFeedDown <- PageDown),
            ]),
            preview: HashMap::from([(key!(Char('q')), Keymap::fixed(Action::Quit))]),
            reserved: HashSet::from([
                key!(Char('v') + Alt),
                key!(Char('l') + Ctrl),
                key!(Char(' ') + Shift),
                key!(Enter + Shift),
                key!(Enter + Alt),
                key!(Left + Shift),
                key!(Right + Shift),
                key!(Home + Shift),
                key!(End + Shift),
                key!(Left),
                key!(Right),
                key!(Up),
                key!(Down),
                key!(Home),
                key!(End),
                key!(Backspace),
                key!(Delete),
                key!(Tab),
            ]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(keymaps: &Keymaps, key: KeyEvent) -> Option<Action> {
        keymaps.action(key, false)
    }

    #[test]
    fn editing_keys_are_not_command_bindings() {
        let keymaps = Keymaps::default();
        let plain = key!(Enter);
        let shifted = key!(Enter + Shift);

        assert!(action(&keymaps, plain) == Some(Action::Submit));
        assert!(action(&keymaps, shifted).is_none());
    }

    #[test]
    fn spaces_are_forwarded_to_the_editor() {
        let keymaps = Keymaps::default();

        for key in [key!(Char(' ')), key!(Char(' ') + Shift)] {
            assert!(action(&keymaps, key).is_none());
        }
    }

    #[test]
    fn preview_exit_is_fixed_to_plain_q() {
        let keymaps = Keymaps::default();

        assert!(keymaps.preview_action(key!(Char('q'))) == Some(Action::Quit));
        assert!(keymaps.preview_action(key!(Char('q') + Ctrl)).is_none());
    }

    // The terminal reports a shifted letter as the letter itself, so a binding on
    // a character must not demand the shift state match.
    #[test]
    fn character_bindings_ignore_shift() {
        let keymaps = Keymaps::default();
        let shifted = key!(Char('c') + Ctrl + Shift);

        assert!(action(&keymaps, shifted) == Some(Action::Quit));
    }

    #[test]
    fn control_slash_accepts_legacy_terminal_encoding() {
        let keymaps = Keymaps::default();
        let legacy = key!(Char('7') + Ctrl);

        assert!(action(&keymaps, legacy) == Some(Action::OpenCommands));
    }

    #[test]
    fn keys_are_labelled_for_display() {
        let keymaps = Keymaps::default();
        let ctrl = if cfg!(target_os = "macos") {
            "⌘"
        } else {
            "ctrl"
        };
        assert_eq!(
            keymaps.key_for(Action::OpenSettings),
            Some(format!("{ctrl}+o"))
        );
        assert_eq!(
            keymaps.key_for(Action::OpenCommands),
            Some(format!("{ctrl}+/"))
        );
        assert_eq!(
            keymaps.key_for(Action::ResizePaneLeft),
            Some(format!("{ctrl}+⇧+←"))
        );
    }

    #[test]
    fn remapping_swaps_remappable_conflicts_and_preserves_fixed_keys() {
        let mut keymaps = Keymaps::default();
        let copy = key!(Char('y') + Ctrl);
        let submit = key!(Enter);

        let clear = Action::ClearFeed;
        assert!(keymaps.rebind(clear, copy));
        assert!(action(&keymaps, copy) == Some(clear));
        assert!(action(&keymaps, key!(Char('l') + Alt)) == Some(Action::CopySelection));
        assert!(!keymaps.rebind(clear, submit));
        assert!(action(&keymaps, submit) == Some(Action::Submit));
    }

    #[test]
    fn plain_typing_can_be_claimed_by_a_remapping() {
        let mut keymaps = Keymaps::default();
        let letter = key!(Char('a'));

        assert!(keymaps.rebind(Action::ClearFeed, letter));
        assert!(action(&keymaps, letter) == Some(Action::ClearFeed));
    }

    #[test]
    fn alternative_view_cannot_be_claimed_by_a_remapping() {
        let mut keymaps = Keymaps::default();
        let alternative = key!(Char('v') + Alt);

        assert!(!keymaps.rebind(Action::ClearFeed, alternative));
        assert!(action(&keymaps, alternative).is_none());
    }
}
