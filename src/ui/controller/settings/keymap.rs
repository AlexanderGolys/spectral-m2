//! Key bindings as data.
//!
//! An action binding consumes its terminal key and emits an [`Action`]. Keys
//! without a binding continue to the focused component as ordinary input.

use std::collections::{BTreeSet, HashMap};

use crossterm::event::KeyCode;
use KeyCode::*;

use ratatui::text::{Line, Span};
use serde::{Deserialize, Serialize};

use crate::input::InputAction;
use crate::ui::action::Action;
use crate::ui::controller::picker::{PickerInput, PickerItem, PickerMode, PickerTab, Row};
use crate::ui::controller::settings::theme::groups::{scheme, HLGroup};
use crate::ui::controller::EditorEvent;
use crate::ui::key::{KeyEvent, KeyMod};
use KeyMod::{Alt, Ctrl, Shift};

/// The part of the editor a shortcut works in; each context is a tab of the keymaps
/// picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyContext {
    Global,
    Movement,
    Selection,
    Editing,
    Recall,
}

impl KeyContext {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Movement => "movement",
            Self::Selection => "selection",
            Self::Editing => "editing",
            Self::Recall => "completion & history",
        }
    }
}

/// Something a key can be bound to: an editor-wide command or a text edit.
///
/// Serialized as the bare action name, so shortcuts saved before edits were
/// bindable still load.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Shortcut {
    Command(Action),
    Edit(InputAction),
}

impl Shortcut {
    pub fn context(self) -> KeyContext {
        use InputAction::*;
        match self {
            Self::Command(_) => KeyContext::Global,
            Self::Edit(edit) => match edit {
                MoveLeft | MoveRight | MoveUp | MoveDown | MoveWordLeft | MoveWordRight
                | MoveLineStart | MoveLineEnd | MoveTextStart | MoveTextEnd => KeyContext::Movement,
                SelectLeft | SelectRight | SelectWordLeft | SelectWordRight | SelectLineStart
                | SelectLineEnd | SelectTextStart | SelectTextEnd | SelectAll => {
                    KeyContext::Selection
                }
                InsertNewline | InsertTab | DeleteBack | DeleteForward | DeleteWordBack
                | DeleteWordForward | DeleteLine | Undo | Redo | Clear => KeyContext::Editing,
                NextCompletion | PreviousCompletion | HistoryPrevious | HistoryNext => {
                    KeyContext::Recall
                }
            },
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Command(action) => action.description(),
            Self::Edit(edit) => edit.description(),
        }
    }
}

impl From<Action> for Shortcut {
    fn from(action: Action) -> Self {
        Self::Command(action)
    }
}

impl From<InputAction> for Shortcut {
    fn from(edit: InputAction) -> Self {
        Self::Edit(edit)
    }
}

/// One shortcut attached to a normalized key, and whether the user may move it.
///
/// An `alias` is another key for a shortcut that already has a primary one, such
/// as the control character a terminal sends when it cannot report a modifier.
#[derive(Clone, Copy)]
struct Keymap {
    shortcut: Shortcut,
    remappable: bool,
    alias: bool,
}

/// The editor's exact normalized-key bindings.
#[derive(Clone)]
pub struct Keymaps {
    bindings: HashMap<KeyEvent, Keymap>,
    rebinds: Vec<(Shortcut, KeyEvent)>,
}

impl Keymaps {
    /// Resolve a terminal key to a command, after which the key is discarded.
    pub fn action(&self, key: KeyEvent, remapping: bool) -> Option<Action> {
        let binding = self.bindings.get(&key.binding())?;
        match binding.shortcut {
            Shortcut::Command(action)
                if !remapping
                    || !binding.remappable
                    || matches!(action, Action::OpenKeymaps | Action::OpenSettings) =>
            {
                Some(action)
            }
            _ => None,
        }
    }

    /// Resolve a terminal key to the text edit it performs in the input.
    pub fn edit(&self, key: KeyEvent) -> Option<InputAction> {
        match self.bindings.get(&key.binding())?.shortcut {
            Shortcut::Edit(edit) => Some(edit),
            Shortcut::Command(_) => None,
        }
    }

    /// The key that triggers `shortcut`.
    pub fn key(&self, shortcut: impl Into<Shortcut>) -> Option<KeyEvent> {
        let shortcut = shortcut.into();
        self.bindings.iter().find_map(|(key, binding)| {
            (binding.shortcut == shortcut && !binding.alias).then_some(*key)
        })
    }

    /// The key that triggers `shortcut`, labelled for hints and help.
    pub fn key_for(&self, shortcut: impl Into<Shortcut>) -> Option<String> {
        self.key(shortcut).map(|key| key.to_string())
    }

    /// The default keymaps with `rebinds` made over them, in order.
    pub fn replaying(rebinds: &[(Shortcut, KeyEvent)]) -> Self {
        let mut keymaps = Self::default();
        for &(shortcut, key) in rebinds {
            keymaps.rebind(shortcut, key);
        }
        keymaps
    }

    /// Every rebind that took effect, in the order made.
    pub fn rebinds(&self) -> &[(Shortcut, KeyEvent)] {
        &self.rebinds
    }

    /// The bindings the user may move, with their current keys, in shortcut order.
    pub fn remappable(&self) -> Vec<(Shortcut, KeyEvent)> {
        let mut remappable: Vec<_> = self
            .bindings
            .iter()
            .filter(|(_, binding)| binding.remappable)
            .map(|(key, binding)| (binding.shortcut, *key))
            .collect();
        remappable.sort_by_key(|(shortcut, _)| *shortcut);
        remappable
    }

    /// The contexts that have a binding the user may move.
    pub fn contexts(&self) -> BTreeSet<KeyContext> {
        self.remappable()
            .into_iter()
            .map(|(shortcut, _)| shortcut.context())
            .collect()
    }

    /// Whether a key can be assigned to a shortcut working in `context`, leaving
    /// text entry and terminal control keys available.
    fn valid_shortcut(context: KeyContext, key: KeyEvent) -> bool {
        match key.code {
            F(2..=24) => return true,
            F(_) | Esc | Null | Modifier(_) => return false,
            Char(character) => {
                if character.is_control() || key.is_text() {
                    return false;
                }
                // The terminal sends these as Tab, Enter and Esc.
                if key.ctrl() && !key.alt() && !key.shift() && matches!(character, 'i' | 'm' | '[')
                {
                    return false;
                }
            }
            _ => {}
        }
        match context {
            KeyContext::Global => {
                !matches!(key.code, Tab | BackTab) && (key.ctrl() || key.alt() || key.shift())
            }
            _ => true,
        }
    }

    /// Move `shortcut`'s remappable binding onto `key`, swapping with a remappable
    /// binding already there. Fixed bindings never move, and neither shortcut may
    /// land on a key invalid in its context.
    pub fn rebind(&mut self, shortcut: impl Into<Shortcut>, key: KeyEvent) -> bool {
        let shortcut = shortcut.into();
        let key = key.binding();
        let Some(target) = self.bindings.iter().find_map(|(bound, binding)| {
            (binding.shortcut == shortcut && binding.remappable && !binding.alias).then_some(*bound)
        }) else {
            return false;
        };
        if !Self::valid_shortcut(shortcut.context(), key) {
            return false;
        }
        if let Some(conflict) = self.bindings.get(&key) {
            if !conflict.remappable || !Self::valid_shortcut(conflict.shortcut.context(), target) {
                return false;
            }
        }
        if key == target {
            return true;
        }
        let binding = self.bindings.remove(&target).unwrap();
        if let Some(conflict) = self.bindings.remove(&key) {
            self.bindings.insert(target, conflict);
        }
        self.bindings.insert(key, binding);
        self.rebinds.push((shortcut, key));
        true
    }
}

/// The keymaps page of the settings picker, grouped by shortcut context.
pub fn settings_tab(keymaps: &Keymaps) -> PickerTab {
    let remappable = keymaps.remappable();
    let mut items = keymaps
        .contexts()
        .into_iter()
        .flat_map(|context| {
            remappable
                .iter()
                .filter(move |(shortcut, _)| shortcut.context() == context)
                .map(|&(shortcut, key)| {
                    Box::new(KeymapRow::new(shortcut, key)) as Box<dyn PickerItem>
                })
        })
        .collect::<Vec<_>>();
    items.push(restore_defaults());
    PickerTab {
        mode: Some(PickerMode::Keymaps),
        label: "keymaps",
        items,
    }
}

/// The button putting every shortcut back on its default key.
pub fn restore_defaults() -> Box<dyn PickerItem> {
    let label = Span::styled("restore keymaps", scheme().settings_button());
    Row::enter(vec![label.into()], || EditorEvent::RestoreKeymaps)
        .described("put every shortcut back on its default key")
        .into()
}

/// A shortcut row that captures a replacement only after Enter.
pub struct KeymapRow {
    shortcut: Shortcut,
    key: KeyEvent,
    capturing: bool,
}

impl KeymapRow {
    /// Show a shortcut and its current key.
    pub fn new(shortcut: Shortcut, key: KeyEvent) -> Self {
        Self {
            shortcut,
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
            Span::styled(self.shortcut.description(), groups.keymap_action_label()).into(),
            Span::styled(shortcut, groups.style(HLGroup::KeymapHint)).into(),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some(self.shortcut.context().label())
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
                    Some(EditorEvent::Rebind(self.shortcut, key))
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

/// A default binding: `fixed` ones hold their key, `alias` ones are further fixed
/// keys for a shortcut, the rest may be remapped.
macro_rules! keymap {
    (fixed $shortcut:path => $key:expr) => {
        keymap!(@ $shortcut, $key, false, false)
    };
    (alias $shortcut:path => $key:expr) => {
        keymap!(@ $shortcut, $key, false, true)
    };
    ($shortcut:path => $key:expr) => {
        keymap!(@ $shortcut, $key, true, false)
    };
    (@ $shortcut:path, $key:expr, $remappable:expr, $alias:expr) => {
        (
            key!($key),
            Keymap {
                shortcut: Shortcut::from($shortcut),
                remappable: $remappable,
                alias: $alias,
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
        use Action as A;
        use InputAction as I;
        Self {
            bindings: HashMap::from([
                keymap!(A::CancelExecution => Char('c') + Ctrl),
                keymap!(A::OpenCommands => Char('/') + Ctrl),
                keymap!(A::OpenSettings => Char('o') + Ctrl),
                keymap!(A::OpenKeymaps => Char('k') + Ctrl),
                keymap!(A::ClearFeed => Char('l') + Alt),
                keymap!(A::OpenCode => F(2)),
                keymap!(A::OpenValues => F(3)),
                keymap!(A::OpenStdOut => F(4)),
                keymap!(A::OpenOutput => F(5)),
                keymap!(A::OpenSyntaxTree => F(6)),
                keymap!(A::Restart => Char('n') + Ctrl),
                keymap!(A::ResumeSession => Char('r') + Ctrl),
                keymap!(A::ClosePane => Char('q') + Ctrl),
                keymap!(A::ReferenceBack => Char('b') + Ctrl + Shift),
                keymap!(fixed A::ToggleHighlightDebug => Char('d') + Ctrl + Shift),
                keymap!(A::SaveSelection => Char('s') + Ctrl + Shift),
                keymap!(A::BrowseSessions => F(7)),
                keymap!(A::RunViewedFile => F(8)),
                keymap!(A::CopySelection => Char('y') + Ctrl),
                keymap!(fixed A::Submit => Enter),
                keymap!(fixed A::ScrollFeedUp => PageUp),
                keymap!(fixed A::ScrollFeedDown => PageDown),
                keymap!(I::MoveLeft => Left),
                keymap!(I::MoveRight => Right),
                keymap!(I::MoveUp => Up),
                keymap!(I::MoveDown => Down),
                keymap!(I::MoveWordLeft => Left + Ctrl),
                keymap!(I::MoveWordRight => Right + Ctrl),
                keymap!(I::MoveLineStart => Home),
                keymap!(I::MoveLineEnd => End),
                keymap!(I::MoveTextStart => Home + Ctrl),
                keymap!(I::MoveTextEnd => End + Ctrl),
                keymap!(I::SelectLeft => Left + Shift),
                keymap!(I::SelectRight => Right + Shift),
                keymap!(I::SelectWordLeft => Left + Ctrl + Shift),
                keymap!(I::SelectWordRight => Right + Ctrl + Shift),
                keymap!(I::SelectLineStart => Home + Shift),
                keymap!(I::SelectLineEnd => End + Shift),
                keymap!(I::SelectTextStart => Home + Ctrl + Shift),
                keymap!(I::SelectTextEnd => End + Ctrl + Shift),
                keymap!(I::SelectAll => Char('a') + Ctrl),
                keymap!(I::InsertNewline => Enter + Shift),
                // Enter that a terminal cannot report with Shift arrives as the
                // newline control character or as a Shifted space.
                keymap!(alias I::InsertNewline => Enter + Alt),
                keymap!(alias I::InsertNewline => Char(' ') + Shift),
                keymap!(alias I::InsertNewline => Char('j') + Ctrl),
                keymap!(I::InsertTab => Tab + Alt),
                keymap!(I::DeleteBack => Backspace),
                keymap!(I::DeleteForward => Delete),
                keymap!(I::DeleteLine => Char('u') + Ctrl),
                keymap!(I::DeleteWordBack => Backspace + Ctrl),
                keymap!(alias I::DeleteWordBack => Backspace + Alt),
                // The backspace control character, which is what a terminal sends
                // for Ctrl+Backspace when it cannot report the modifier.
                keymap!(alias I::DeleteWordBack => Char('h') + Ctrl),
                keymap!(I::DeleteWordForward => Delete + Ctrl),
                keymap!(alias I::DeleteWordForward => Delete + Alt),
                keymap!(I::Undo => Char('z') + Ctrl),
                keymap!(I::Redo => Char('z') + Ctrl + Shift),
                keymap!(I::Clear => Char('l') + Ctrl),
                keymap!(I::NextCompletion => Tab),
                keymap!(I::PreviousCompletion => BackTab),
                keymap!(alias I::PreviousCompletion => Tab + Ctrl),
                keymap!(I::HistoryPrevious => Up + Ctrl),
                keymap!(I::HistoryNext => Down + Ctrl),
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
            .all(|(action, _)| *action != Shortcut::Command(Action::ToggleHighlightDebug)));
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
            "cmd"
        } else {
            "ctrl"
        };
        assert_eq!(key!(Char('o') + Ctrl).to_string(), format!("{ctrl}+o"));
        assert_eq!(key!(Char('/') + Ctrl).to_string(), format!("{ctrl}+/"));
        assert_eq!(
            key!(Left + Ctrl + Shift).to_string(),
            format!("{ctrl}+shift+left")
        );

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
            .all(|(remappable, _)| *remappable != Shortcut::Command(Action::Submit)));
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
            let loaded = Keymaps::replaying(&[(Shortcut::Command(Action::ClearFeed), key)]);
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
            .any(|(action, _)| *action == Shortcut::Command(Action::CancelExecution)));
        assert!(keymaps.rebind(Action::CancelExecution, key!(F(24))));
        assert!(keymaps.action(key!(F(24)), false) == Some(Action::CancelExecution));
    }

    #[test]
    fn shortcut_capture_requires_enter_and_escape_restores_the_label() {
        let mut row = KeymapRow::new(Action::ClearFeed.into(), key!(Char('l') + Alt));
        assert!(row.handle(PickerInput::Key(key!(F(24)))).is_none());
        assert!(!row.capturing());
        row.handle(PickerInput::Action(Action::Submit));
        assert!(row.capturing());
        assert!(row.cells()[1].to_string().contains("Type keybind"));
        assert!(matches!(row.handle(PickerInput::Key(key!(F(24)))),
            Some(EditorEvent::Rebind(Shortcut::Command(Action::ClearFeed), key)) if key == key!(F(24))));
        row.handle(PickerInput::Key(key!(Esc)));
        assert!(!row.capturing());
        assert_eq!(
            row.cells()[1].to_string(),
            key!(Char('l') + Alt).to_string()
        );
    }

    #[test]
    fn plain_typing_cannot_be_claimed_by_a_remapping() {
        let mut keymaps = Keymaps::default();
        let letter = key!(Char('a'));

        assert!(!keymaps.rebind(Action::ClearFeed, letter));
        assert!(action(&keymaps, letter).is_none());
    }

    #[test]
    fn every_edit_has_a_binding_in_the_unified_keymaps_tab() {
        let keymaps = Keymaps::default();
        for edit in [
            InputAction::MoveLeft,
            InputAction::SelectAll,
            InputAction::Undo,
            InputAction::HistoryNext,
        ] {
            let key = keymaps.key(edit).expect("edits are bound");
            assert_eq!(keymaps.edit(key), Some(edit));
            assert!(keymaps.action(key, false).is_none());
        }
        let tab = settings_tab(&keymaps);
        assert_eq!(tab.mode, Some(PickerMode::Keymaps));
        assert_eq!(tab.items.len(), keymaps.remappable().len() + 1);
    }

    #[test]
    fn aliases_trigger_their_shortcut_without_being_its_key() {
        let keymaps = Keymaps::default();
        for alias in [
            key!(Char('j') + Ctrl),
            key!(Char(' ') + Shift),
            key!(Enter + Alt),
        ] {
            assert_eq!(keymaps.edit(alias), Some(InputAction::InsertNewline));
            assert_ne!(keymaps.key(InputAction::InsertNewline), Some(alias));
        }
        assert_eq!(
            keymaps.edit(key!(BackTab + Shift)),
            Some(InputAction::PreviousCompletion)
        );
    }

    #[test]
    fn an_edit_can_move_but_not_onto_an_alias_or_a_key_its_swap_partner_cannot_hold() {
        let mut keymaps = Keymaps::default();
        let original = keymaps.key(InputAction::Undo).expect("undo binding");
        let replacement = key!(Char('u') + Ctrl);
        let displaced = keymaps.edit(replacement);
        assert!(keymaps.rebind(InputAction::Undo, replacement));
        assert_eq!(keymaps.edit(replacement), Some(InputAction::Undo));
        assert_eq!(keymaps.edit(original), displaced);

        assert!(!keymaps.rebind(InputAction::Undo, key!(Char('j') + Ctrl)));
        // A plain arrow is fine for editing, but a global command could not hold it.
        assert!(!keymaps.rebind(Action::ClearFeed, key!(Left)));
        // Swapping with a command would strand it on a key it cannot hold.
        assert!(
            !keymaps.rebind(InputAction::MoveLeft, key!(Char('l') + Alt)) || {
                keymaps.key(Action::ClearFeed) == Some(key!(Left))
            }
        );
    }

    #[test]
    fn edit_rebinds_survive_saving_next_to_command_rebinds() {
        let mut keymaps = Keymaps::default();
        assert!(keymaps.rebind(InputAction::Redo, key!(Char('y') + Alt)));
        assert!(keymaps.rebind(Action::ClearFeed, key!(F(7))));
        let saved = serde_json::to_string(keymaps.rebinds()).unwrap();
        let loaded: Vec<(Shortcut, KeyEvent)> = serde_json::from_str(&saved).unwrap();

        assert_eq!(
            Keymaps::replaying(&loaded).key(InputAction::Redo),
            Some(key!(Char('y') + Alt))
        );
    }

    #[test]
    fn running_the_viewed_file_avoids_the_enter_chord_the_system_reserves() {
        let keymaps = Keymaps::default();
        assert!(keymaps.action(key!(Enter + Ctrl), false).is_none());
        assert!(keymaps.action(key!(F(8)), false) == Some(Action::RunViewedFile));
    }
}
