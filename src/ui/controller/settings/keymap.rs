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

/// One action attached to a normalized key, and whether the user may move it.
#[derive(Clone, Copy)]
struct Keymap {
    action: Action,
    remappable: bool,
}

/// The editor's exact normalized-key bindings.
#[derive(Clone)]
pub struct Keymaps {
    bindings: HashMap<KeyEvent, Keymap>,
    reserved: HashSet<KeyEvent>,
    rebinds: Vec<(Action, KeyEvent)>,
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

    /// The key that triggers `action`.
    pub fn key(&self, action: Action) -> Option<KeyEvent> {
        self.bindings
            .iter()
            .find_map(|(key, binding)| (binding.action == action).then_some(*key))
    }

    /// The key that triggers `action`, labelled for hints and help.
    pub fn key_for(&self, action: Action) -> Option<String> {
        self.key(action).map(|key| key.to_string())
    }

    /// The default keymaps with `rebinds` made over them, in order.
    pub fn replaying(rebinds: &[(Action, KeyEvent)]) -> Self {
        let mut keymaps = Self::default();
        for &(action, key) in rebinds {
            keymaps.rebind(action, key);
        }
        keymaps
    }

    /// Every rebind that took effect, in the order made.
    pub fn rebinds(&self) -> &[(Action, KeyEvent)] {
        &self.rebinds
    }

    /// The bindings the user may move, with their current keys, in action order.
    pub fn remappable(&self) -> Vec<(Action, KeyEvent)> {
        let mut remappable: Vec<_> = self
            .bindings
            .iter()
            .filter(|(_, binding)| binding.remappable)
            .map(|(key, binding)| (binding.action, *key))
            .collect();
        remappable.sort_by_key(|(action, _)| *action);
        remappable
    }

    /// Move `action`'s remappable binding onto `key`, swapping with a remappable
    /// binding already there. Fixed bindings and reserved keys never move.
    pub fn rebind(&mut self, action: Action, key: KeyEvent) -> bool {
        let key = key.binding();
        let Some(target) = self.bindings.iter().find_map(|(bound, binding)| {
            (binding.action == action && binding.remappable).then_some(*bound)
        }) else {
            return false;
        };
        if self.reserved.contains(&key)
            || self
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
        self.rebinds.push((action, key));
        true
    }
}

/// A default binding: `fixed` ones hold their key, the rest may be remapped.
macro_rules! keymap {
    (fixed $action:ident <- $key:expr) => {
        keymap!(@ $action, $key, false)
    };
    ($action:ident <- $key:expr) => {
        keymap!(@ $action, $key, true)
    };
    (@ $action:ident, $key:expr, $remappable:expr) => {
        (
            key!($key),
            Keymap {
                action: Action::$action,
                remappable: $remappable,
            },
        )
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
                keymap!(fixed FocusPaneLeft <- Left + Alt),
                keymap!(fixed FocusPaneRight <- Right + Alt),
                keymap!(fixed ResizePaneLeft <- Left + Alt + Shift),
                keymap!(fixed ResizePaneRight <- Right + Alt + Shift),
                keymap!(Quit <- Char('c') + Ctrl),
                keymap!(OpenCommands <- Char('/') + Ctrl),
                keymap!(OpenSettings <- Char('o') + Ctrl),
                keymap!(OpenKeymaps <- Char('k') + Alt),
                keymap!(ClearFeed <- Char('l') + Alt),
                keymap!(OpenCode <- Char('c') + Alt),
                keymap!(OpenValues <- Char('v') + Alt),
                keymap!(OpenStdOut <- Char('p') + Alt),
                keymap!(OpenOutput <- Char('o') + Alt),
                keymap!(OpenSyntaxTree <- Char('t') + Alt),
                keymap!(Restart <- Char('r') + Alt),
                keymap!(ClosePane <- Char('q') + Ctrl),
                keymap!(ToggleHighlightDebug <- Char('d') + Ctrl),
                keymap!(CopySelection <- Char('y') + Ctrl),
                keymap!(fixed Submit <- Enter),
                keymap!(fixed ScrollFeedUp <- PageUp),
                keymap!(fixed ScrollFeedDown <- PageDown),
            ]),
            reserved: HashSet::from([
                key!(Char('l') + Ctrl),
                key!(Char(' ') + Shift),
                key!(Enter + Shift),
                key!(Enter + Alt),
                key!(Char('j') + Ctrl),
                key!(Char('z') + Ctrl),
                key!(Char('h') + Ctrl),
                key!(Backspace + Ctrl),
                key!(Backspace + Alt),
                key!(Delete + Ctrl),
                key!(Delete + Alt),
                key!(Left + Ctrl),
                key!(Right + Ctrl),
                key!(Left + Ctrl + Shift),
                key!(Right + Ctrl + Shift),
                key!(Home + Ctrl),
                key!(End + Ctrl),
                key!(Home + Ctrl + Shift),
                key!(End + Ctrl + Shift),
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
                key!(BackTab),
            ]),
            rebinds: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(keymaps: &Keymaps, key: KeyEvent) -> Option<Action> {
        keymaps.action(key, false)
    }

    /// The key that submits, held with Shift, inserts a newline instead.
    #[test]
    fn editing_keys_are_not_command_bindings() {
        let keymaps = Keymaps::default();
        let submit = keymaps.key(Action::Submit).expect("submitting is bound");

        assert!(action(&keymaps, submit + Shift).is_none());
    }

    #[test]
    fn spaces_are_forwarded_to_the_editor() {
        let keymaps = Keymaps::default();

        for key in [key!(Char(' ')), key!(Char(' ') + Shift)] {
            assert!(action(&keymaps, key).is_none());
        }
    }

    /// Every default shortcut on a character holds a modifier, so typing that
    /// character stays text.
    #[test]
    fn shortcuts_leave_their_plain_characters_to_typing() {
        let keymaps = Keymaps::default();

        for (_, bound) in keymaps.remappable() {
            if let Char(character) = bound.code {
                assert!(
                    action(&keymaps, key!(Char(character))).is_none(),
                    "{bound} leaves no plain {character:?}"
                );
            }
        }
    }

    // The terminal reports a shifted letter as the letter itself, so a binding on
    // a character must not demand the shift state match.
    #[test]
    fn character_bindings_ignore_shift() {
        let mut keymaps = Keymaps::default();
        let bound = key!(Char('c') + Ctrl);
        assert!(keymaps.rebind(Action::ClearFeed, bound));

        assert!(action(&keymaps, bound + Shift) == Some(Action::ClearFeed));
    }

    #[test]
    fn control_slash_accepts_legacy_terminal_encoding() {
        let mut keymaps = Keymaps::default();
        assert!(keymaps.rebind(Action::OpenCommands, key!(Char('/') + Ctrl)));
        let legacy = key!(Char('7') + Ctrl);

        assert!(action(&keymaps, legacy) == Some(Action::OpenCommands));
    }

    #[test]
    fn keys_are_labelled_for_display() {
        let ctrl = if cfg!(target_os = "macos") {
            "⌘"
        } else {
            "ctrl"
        };
        assert_eq!(key!(Char('o') + Ctrl).to_string(), format!("{ctrl}+o"));
        assert_eq!(key!(Char('/') + Ctrl).to_string(), format!("{ctrl}+/"));
        assert_eq!(key!(Left + Ctrl + Shift).to_string(), format!("{ctrl}+⇧+←"));

        let keymaps = Keymaps::default();
        for (command, _) in keymaps.remappable() {
            assert_eq!(
                keymaps.key_for(command),
                keymaps.key(command).map(|key| key.to_string())
            );
        }
    }

    #[test]
    fn remapping_swaps_remappable_conflicts_and_preserves_fixed_keys() {
        let mut keymaps = Keymaps::default();
        let clear = Action::ClearFeed;
        let copy = keymaps
            .key(Action::CopySelection)
            .expect("copying is bound");
        let cleared_by = keymaps.key(clear).expect("clearing is bound");
        let submit = keymaps.key(Action::Submit).expect("submitting is bound");

        assert!(keymaps.rebind(clear, copy));
        assert!(action(&keymaps, copy) == Some(clear));
        assert!(action(&keymaps, cleared_by) == Some(Action::CopySelection));
        assert!(!keymaps.rebind(clear, submit));
        assert!(action(&keymaps, submit) == Some(Action::Submit));
    }

    #[test]
    fn a_fixed_binding_keeps_its_key_and_stays_out_of_the_remappable_list() {
        let mut keymaps = Keymaps::default();
        let submit = keymaps.key(Action::Submit).expect("submitting is bound");

        assert!(!keymaps.rebind(Action::Submit, key!(Char('a'))));
        assert!(action(&keymaps, submit) == Some(Action::Submit));
        assert!(keymaps
            .remappable()
            .iter()
            .all(|(remappable, _)| *remappable != Action::Submit));
    }

    #[test]
    fn plain_typing_can_be_claimed_by_a_remapping() {
        let mut keymaps = Keymaps::default();
        let letter = key!(Char('a'));

        assert!(keymaps.rebind(Action::ClearFeed, letter));
        assert!(action(&keymaps, letter) == Some(Action::ClearFeed));
    }
}
