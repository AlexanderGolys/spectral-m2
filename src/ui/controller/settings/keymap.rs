//! Key bindings as data.
//!
//! An action binding consumes its terminal key and emits an [`Action`]. Keys
//! without a binding continue to the focused component as ordinary input.

use std::collections::{HashMap, HashSet};

use crossterm::event::KeyCode;
use KeyCode::*;

use ratatui::text::{Line, Span};

use crate::ui::action::Action;
use crate::ui::controller::picker::{PickerInput, PickerItem, Row};
use crate::ui::controller::settings::theme::groups::{scheme, HLGroup};
use crate::ui::controller::EditorEvent;
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

    /// The bindings that hold their key, with that key, in action order.
    pub fn fixed(&self) -> Vec<(Action, KeyEvent)> {
        let mut fixed: Vec<_> = self
            .bindings
            .iter()
            .filter(|(_, binding)| !binding.remappable)
            .map(|(key, binding)| (binding.action, *key))
            .collect();
        fixed.sort_by_key(|(action, _)| *action);
        fixed
    }

    /// Whether a shortcut leaves text entry and terminal control keys available.
    fn valid_shortcut(key: KeyEvent) -> bool {
        if matches!(key.code, F(2..=24)) {
            return true;
        }
        if matches!(key.code, F(_) | Esc | Tab | BackTab | Null | Modifier(_)) {
            return false;
        }
        if let Char(character) = key.code {
            if character.is_control() || key.is_text() {
                return false;
            }
            // The terminal sends these as Tab, Enter and Esc.
            if key.ctrl() && !key.alt() && !key.shift() && matches!(character, 'i' | 'm' | '[') {
                return false;
            }
        }
        key.ctrl() || key.alt() || key.shift()
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
        if !Self::valid_shortcut(key)
            || self.reserved.contains(&key)
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

/// The button putting every shortcut back on its default key.
pub fn restore_defaults() -> Box<dyn PickerItem> {
    let label = Span::styled("restore defaults", scheme().settings_button());
    Row::enter(vec![label.into()], || EditorEvent::RestoreKeymaps)
        .described("put every shortcut back on its default key")
        .into()
}

/// A shortcut row that captures a replacement only after Enter.
pub struct KeymapRow {
    action: Action,
    key: KeyEvent,
    capturing: bool,
}

impl KeymapRow {
    /// Show an action and its current shortcut.
    pub fn new(action: Action, key: KeyEvent) -> Self {
        Self {
            action,
            key,
            capturing: false,
        }
    }
}

impl PickerItem for KeymapRow {
    fn cells(&self) -> Vec<Line<'static>> {
        let groups = scheme();
        let shortcut = if self.capturing {
            "Type keybind… (Esc cancels)".to_string()
        } else {
            self.key.to_string()
        };
        vec![
            Span::styled(self.action.description(), groups.keymap_action_label()).into(),
            Span::styled(shortcut, groups.style(HLGroup::KeymapHint)).into(),
        ]
    }

    fn capturing(&self) -> bool {
        self.capturing
    }

    fn handle(&mut self, input: PickerInput) -> Option<EditorEvent> {
        match input {
            PickerInput::Key(key) if self.capturing => {
                if key.code == Esc {
                    self.capturing = false;
                    None
                } else {
                    Some(EditorEvent::Rebind(self.action, key))
                }
            }
            PickerInput::Action(Action::Submit) => {
                self.capturing = true;
                None
            }
            _ => None,
        }
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

/// The editing and navigation keys the input, the views and the pickers keep for
/// themselves, each group with what it does. No shortcut may take one of them.
pub fn editing_keys() -> Vec<(Vec<KeyEvent>, &'static str)> {
    vec![
        (
            vec![key!(Left), key!(Right), key!(Up), key!(Down)],
            "move the cursor; in a view, change view and scroll",
        ),
        (
            vec![key!(Left + Ctrl), key!(Right + Ctrl)],
            "move by word; beside a split, step its view",
        ),
        (
            vec![key!(Home), key!(End)],
            "move to the start or end of the line",
        ),
        (
            vec![key!(Home + Ctrl), key!(End + Ctrl)],
            "move to the start or end of the input",
        ),
        (
            vec![key!(Left + Shift), key!(Right + Shift)],
            "select a character",
        ),
        (
            vec![key!(Left + Ctrl + Shift), key!(Right + Ctrl + Shift)],
            "select a word",
        ),
        (
            vec![key!(Home + Shift), key!(End + Shift)],
            "select to the start or end of the line",
        ),
        (
            vec![key!(Home + Ctrl + Shift), key!(End + Ctrl + Shift)],
            "select to the start or end of the input",
        ),
        (vec![key!(Char('a') + Ctrl)], "select the whole input"),
        (
            vec![key!(Up + Ctrl), key!(Down + Ctrl)],
            "recall earlier or later input",
        ),
        (vec![key!(Backspace), key!(Delete)], "delete a character"),
        (
            vec![
                key!(Backspace + Ctrl),
                key!(Backspace + Alt),
                key!(Char('h') + Ctrl),
            ],
            "delete the word before the cursor",
        ),
        (
            vec![key!(Delete + Ctrl), key!(Delete + Alt)],
            "delete the word after the cursor",
        ),
        (vec![key!(Char('l') + Ctrl)], "clear the input"),
        (vec![key!(Char('z') + Ctrl)], "undo"),
        (vec![key!(Char('z') + Ctrl + Shift)], "redo"),
        (
            vec![
                key!(Enter + Shift),
                key!(Enter + Alt),
                key!(Char(' ') + Shift),
                key!(Char('j') + Ctrl),
            ],
            "insert a newline",
        ),
        (
            vec![key!(Tab), key!(BackTab)],
            "next or previous completion; in a picker, switch tabs",
        ),
    ]
}

/// A binding the user cannot move, listed so every key the shell answers is shown.
pub struct FixedRow {
    label: &'static str,
    keys: String,
}

impl FixedRow {
    /// Show what `keys` do.
    pub fn new(label: &'static str, keys: &[KeyEvent]) -> Self {
        let keys = keys
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        Self { label, keys }
    }
}

impl PickerItem for FixedRow {
    fn cells(&self) -> Vec<Line<'static>> {
        let groups = scheme();
        vec![
            Span::styled(self.label, groups.keymap_action_label()).into(),
            Span::styled(self.keys.clone(), groups.style(HLGroup::KeymapHint)).into(),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some("built in; this key cannot be rebound")
    }

    fn handle(&mut self, _: PickerInput) -> Option<EditorEvent> {
        None
    }
}

impl Default for Keymaps {
    fn default() -> Self {
        Self {
            bindings: HashMap::from([
                keymap!(CancelExecution <- Char('c') + Ctrl),
                keymap!(OpenCommands <- Char('/') + Ctrl),
                keymap!(OpenSettings <- Char('o') + Ctrl),
                keymap!(OpenKeymaps <- Char('k') + Ctrl),
                keymap!(ClearFeed <- Char('l') + Alt),
                keymap!(OpenCode <- F(2)),
                keymap!(OpenValues <- F(3)),
                keymap!(OpenStdOut <- F(4)),
                keymap!(OpenOutput <- F(5)),
                keymap!(OpenSyntaxTree <- F(6)),
                keymap!(Restart <- Char('n') + Ctrl),
                keymap!(ResumeSession <- Char('r') + Ctrl),
                keymap!(ClosePane <- Char('q') + Ctrl),
                keymap!(fixed ToggleHighlightDebug <- Char('d') + Ctrl + Shift),
                keymap!(CopySelection <- Char('y') + Ctrl),
                keymap!(fixed Submit <- Enter),
                keymap!(fixed ScrollFeedUp <- PageUp),
                keymap!(fixed ScrollFeedDown <- PageDown),
            ]),
            reserved: editing_keys()
                .into_iter()
                .flat_map(|(keys, _)| keys)
                .collect(),
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

    // Terminals report a shifted letter either as the capital or with Shift held, so
    // both reach the same binding, apart from the unshifted letter's.
    #[test]
    fn shifted_letter_bindings_are_their_own_keys() {
        let mut keymaps = Keymaps::default();
        let bound = key!(Char('c') + Ctrl + Shift);
        assert!(keymaps.rebind(Action::ClearFeed, bound));

        assert!(action(&keymaps, key!(Char('C') + Ctrl)) == Some(Action::ClearFeed));
        assert!(action(&keymaps, key!(Char('c') + Ctrl)) != Some(Action::ClearFeed));
    }

    #[test]
    fn highlight_debug_is_fixed_and_leaves_control_d_free() {
        let keymaps = Keymaps::default();
        let debug = keymaps
            .key(Action::ToggleHighlightDebug)
            .expect("highlight debug is bound");

        assert!(debug.shift());
        assert!(action(&keymaps, debug.code + Ctrl).is_none());
        assert!(keymaps
            .remappable()
            .iter()
            .all(|(action, _)| *action != Action::ToggleHighlightDebug));
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
    fn unsafe_shortcuts_are_rejected_when_rebound_or_loaded() {
        let rejected = [
            key!(Char('a')),
            key!(Char('A') + Shift),
            key!(Char('!') + Shift),
            key!(F(1)),
            key!(F(25)),
            key!(Char('i') + Ctrl),
            key!(Char('m') + Ctrl),
            key!(Char('[') + Ctrl),
            key!(Tab + Alt),
            key!(Esc + Ctrl),
            key!(Char('\t') + Alt),
        ];
        for key in rejected {
            let mut keymaps = Keymaps::default();
            let original = keymaps.key(Action::ClearFeed);
            assert!(!keymaps.rebind(Action::ClearFeed, key), "accepted {key}");
            let loaded = Keymaps::replaying(&[(Action::ClearFeed, key)]);
            assert_eq!(loaded.key(Action::ClearFeed), original);
        }
    }

    #[test]
    fn function_keys_and_modified_shortcuts_can_be_assigned() {
        for key in (2..=24).map(|number| key!(F(number))).chain([
            key!(Char('g') + Ctrl),
            key!(Char('c') + Ctrl),
            key!(Char('s') + Ctrl),
            key!(Char('q') + Ctrl),
            key!(Char('a') + Alt),
            key!(Up + Shift),
        ]) {
            let mut keymaps = Keymaps::default();
            assert!(keymaps.rebind(Action::ClearFeed, key), "rejected {key}");
            assert!(action(&keymaps, key) == Some(Action::ClearFeed));
        }
    }

    #[test]
    fn a_moved_binding_can_return_to_its_default_key() {
        let mut keymaps = Keymaps::default();
        let default = keymaps.key(Action::ClosePane).expect("closing is bound");
        assert!(keymaps.rebind(Action::ClosePane, key!(Char('e') + Ctrl)));
        assert!(keymaps.rebind(Action::ClosePane, default));
        assert_eq!(keymaps.key(Action::ClosePane), Some(default));
    }

    #[test]
    fn cancel_execution_has_a_remappable_ctrl_c_binding() {
        let mut keymaps = Keymaps::default();
        assert!(keymaps.action(key!(Char('c') + Ctrl), false) == Some(Action::CancelExecution));
        assert!(keymaps
            .remappable()
            .iter()
            .any(|(action, _)| *action == Action::CancelExecution));
        assert!(keymaps.rebind(Action::CancelExecution, key!(F(24))));
        assert!(keymaps.action(key!(F(24)), false) == Some(Action::CancelExecution));
    }

    #[test]
    fn shortcut_capture_requires_enter_and_escape_restores_the_label() {
        let mut row = KeymapRow::new(Action::ClearFeed, key!(Char('l') + Alt));
        assert!(row.handle(PickerInput::Key(key!(F(24)))).is_none());
        assert!(!row.capturing());
        row.handle(PickerInput::Action(Action::Submit));
        assert!(row.capturing());
        assert!(row.cells()[1].to_string().contains("Type keybind"));
        assert!(matches!(row.handle(PickerInput::Key(key!(F(24)))),
            Some(EditorEvent::Rebind(Action::ClearFeed, key)) if key == key!(F(24))));
        row.handle(PickerInput::Key(key!(Esc)));
        assert!(!row.capturing());
        assert_eq!(
            row.cells()[1].to_string(),
            key!(Char('l') + Alt).to_string()
        );
    }

    /// The keymaps overlay lists the movable shortcuts, the fixed ones and the
    /// editing keys; together they must be every key the shell answers.
    #[test]
    fn the_overlay_lists_every_binding_and_every_reserved_key() {
        let keymaps = Keymaps::default();
        let listed: HashSet<KeyEvent> = keymaps
            .remappable()
            .into_iter()
            .chain(keymaps.fixed())
            .map(|(_, key)| key)
            .collect();
        assert!(keymaps.bindings.keys().all(|key| listed.contains(key)));
        assert_eq!(listed.len(), keymaps.bindings.len());
        assert!(keymaps
            .fixed()
            .iter()
            .any(|(action, _)| *action == Action::Submit));
        let editing: HashSet<KeyEvent> = editing_keys()
            .into_iter()
            .flat_map(|(keys, _)| keys)
            .collect();
        assert_eq!(editing, keymaps.reserved);
    }

    #[test]
    fn plain_typing_cannot_be_claimed_by_a_remapping() {
        let mut keymaps = Keymaps::default();
        let letter = key!(Char('a'));

        assert!(!keymaps.rebind(Action::ClearFeed, letter));
        assert!(action(&keymaps, letter).is_none());
    }
}
