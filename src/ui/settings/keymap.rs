//! Key bindings as data.
//!
//! An action binding consumes its terminal key and emits an [`Action`]. Keys
//! without a binding continue to the focused component as ordinary input.

use std::collections::{BTreeSet, HashMap};

use crossterm::event::KeyCode;
use KeyCode::*;

use ratatui::text::{Line, Span};
use serde::{Deserialize, Serialize};

use crate::input::{InputAction, Traversal};
use crate::ui::action::Action;
use crate::ui::controller::picker::{PickerInput, PickerItem, PickerMode, PickerTab, Row};
use crate::ui::controller::EditorEvent;
use crate::ui::key::{KeyEvent, KeyMod};
use crate::ui::theme::groups::{scheme, HLGroup};
use KeyMod::{Alt, Ctrl, Shift};

/// The part of the editor a shortcut works in; each context is a tab of the keymaps
/// picker.
#[non_exhaustive]
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
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Shortcut {
    Command(Action),
    Edit(InputAction),
}

impl Shortcut {
    pub fn context(self) -> KeyContext {
        use InputAction::*;
        let Self::Edit(edit) = self else {
            return KeyContext::Global;
        };
        match (edit, edit.traversal()) {
            (_, Some((Traversal::Move, ..))) | (MoveUp | MoveDown | MoveMatchingBracket, None) => {
                KeyContext::Movement
            }
            (_, Some((Traversal::Select, ..)))
            | (SelectUp | SelectDown | SelectAll | GrowSelection | ShrinkSelection, None) => {
                KeyContext::Selection
            }
            (NextCompletion | PreviousCompletion | HistoryPrevious | HistoryNext, None) => {
                KeyContext::Recall
            }
            _ => KeyContext::Editing,
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
        label: "keymap",
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
                keymap!(A::SelectPreviousBlock => Up + Alt),
                keymap!(A::SelectNextBlock => Down + Alt),
                keymap!(A::Describe => Char('k') + Alt),
                keymap!(A::DescribeBeside => Char('k') + Alt + Shift),
                keymap!(A::GoToDefinition => Char(']') + Ctrl),
                // What a terminal without keyboard enhancement sends for Ctrl+].
                keymap!(alias A::GoToDefinition => Char('5') + Ctrl),
                keymap!(A::FormatInput => Char('f') + Alt + Shift),
                keymap!(fixed A::Submit => Enter),
                keymap!(fixed A::ScrollFeedUp => PageUp),
                keymap!(fixed A::ScrollFeedDown => PageDown),
                keymap!(fixed A::CutSelection => Char('x') + Ctrl),
                keymap!(A::ToggleViewCursor => Char('e') + Ctrl),
                keymap!(A::ScrollBlockStart => PageUp + Alt),
                keymap!(A::ScrollBlockEnd => PageDown + Alt),
                // Keys every editor gives the same meaning hold it here too, and
                // stay out of the keymaps list.
                keymap!(fixed I::MoveLeft => Left),
                keymap!(fixed I::MoveRight => Right),
                keymap!(fixed I::MoveUp => Up),
                keymap!(fixed I::MoveDown => Down),
                keymap!(fixed I::MoveWordLeft => Left + Ctrl),
                keymap!(fixed I::MoveWordRight => Right + Ctrl),
                keymap!(fixed I::MoveLineStart => Home),
                keymap!(fixed I::MoveLineEnd => End),
                keymap!(fixed I::MoveTextStart => Home + Ctrl),
                keymap!(fixed I::MoveTextEnd => End + Ctrl),
                keymap!(fixed I::SelectLeft => Left + Shift),
                keymap!(fixed I::SelectRight => Right + Shift),
                keymap!(fixed I::SelectUp => Up + Shift),
                keymap!(fixed I::SelectDown => Down + Shift),
                keymap!(fixed I::SelectWordLeft => Left + Ctrl + Shift),
                keymap!(fixed I::SelectWordRight => Right + Ctrl + Shift),
                keymap!(fixed I::SelectLineStart => Home + Shift),
                keymap!(fixed I::SelectLineEnd => End + Shift),
                keymap!(fixed I::SelectTextStart => Home + Ctrl + Shift),
                keymap!(fixed I::SelectTextEnd => End + Ctrl + Shift),
                keymap!(fixed I::SelectAll => Char('a') + Ctrl),
                keymap!(fixed I::DeleteBack => Backspace),
                keymap!(fixed I::DeleteForward => Delete),
                keymap!(fixed I::DeleteWordBack => Backspace + Ctrl),
                keymap!(alias I::DeleteWordBack => Backspace + Alt),
                // The backspace control character, which is what a terminal sends
                // for Ctrl+Backspace when it cannot report the modifier.
                keymap!(alias I::DeleteWordBack => Char('h') + Ctrl),
                keymap!(fixed I::DeleteWordForward => Delete + Ctrl),
                keymap!(alias I::DeleteWordForward => Delete + Alt),
                keymap!(fixed I::Undo => Char('z') + Ctrl),
                keymap!(fixed I::Redo => Char('z') + Ctrl + Shift),
                keymap!(fixed I::NextCompletion => Tab),
                keymap!(fixed I::PreviousCompletion => BackTab),
                keymap!(alias I::PreviousCompletion => Tab + Ctrl),
                keymap!(I::MoveSpacedWordLeft => Left + Alt),
                keymap!(I::MoveSpacedWordRight => Right + Alt),
                keymap!(I::SelectSpacedWordLeft => Left + Alt + Shift),
                keymap!(I::SelectSpacedWordRight => Right + Alt + Shift),
                keymap!(I::MoveParagraphUp => Up + Ctrl + Alt),
                keymap!(I::MoveParagraphDown => Down + Ctrl + Alt),
                keymap!(I::SelectParagraphUp => Up + Ctrl + Alt + Shift),
                keymap!(I::SelectParagraphDown => Down + Ctrl + Alt + Shift),
                keymap!(I::MoveBlockStart => Left + Ctrl + Alt),
                keymap!(I::MoveBlockEnd => Right + Ctrl + Alt),
                keymap!(I::MoveMatchingBracket => Char('m') + Alt),
                keymap!(I::GrowSelection => Up + Alt + Shift),
                keymap!(I::ShrinkSelection => Down + Alt + Shift),
                keymap!(I::InsertNewline => Enter + Shift),
                // Enter that a terminal cannot report with Shift arrives as the
                // newline control character or as a Shifted space.
                keymap!(alias I::InsertNewline => Enter + Alt),
                keymap!(alias I::InsertNewline => Char(' ') + Shift),
                keymap!(alias I::InsertNewline => Char('j') + Ctrl),
                keymap!(I::InsertTab => Tab + Alt),
                keymap!(I::DeleteLine => Char('u') + Ctrl),
                keymap!(I::DeleteSpacedWordBack => Char('w') + Ctrl),
                keymap!(I::Clear => Char('l') + Ctrl),
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
            key!(Up + Ctrl + Shift),
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
        let original = keymaps.key(InputAction::Clear).expect("clear binding");
        let replacement = keymaps.key(InputAction::DeleteLine).expect("line deletion binding");
        assert!(keymaps.rebind(InputAction::Clear, replacement));
        assert_eq!(keymaps.edit(replacement), Some(InputAction::Clear));
        assert_eq!(keymaps.edit(original), Some(InputAction::DeleteLine));

        assert!(!keymaps.rebind(InputAction::Clear, key!(Char('j') + Ctrl)));
        // A plain arrow is fine for editing, but a global command could not hold it.
        assert!(!keymaps.rebind(Action::ClearFeed, key!(Left)));
        // Swapping with a command would strand it on a key it cannot hold.
        let moved_from = keymaps.key(InputAction::MoveSpacedWordLeft);
        assert!(
            !keymaps.rebind(InputAction::MoveSpacedWordLeft, key!(Char('l') + Alt))
                || keymaps.key(Action::ClearFeed) == moved_from
        );
    }

    #[test]
    fn edit_rebinds_survive_saving_next_to_command_rebinds() {
        let mut keymaps = Keymaps::default();
        assert!(keymaps.rebind(InputAction::DeleteLine, key!(Char('y') + Alt)));
        assert!(keymaps.rebind(Action::ClearFeed, key!(F(7))));
        let saved = serde_json::to_string(keymaps.rebinds()).unwrap();
        let loaded: Vec<(Shortcut, KeyEvent)> = serde_json::from_str(&saved).unwrap();

        assert_eq!(
            Keymaps::replaying(&loaded).key(InputAction::DeleteLine),
            Some(key!(Char('y') + Alt))
        );
    }

    #[test]
    fn running_the_viewed_file_avoids_the_enter_chord_the_system_reserves() {
        let keymaps = Keymaps::default();
        assert!(keymaps.action(key!(Enter + Ctrl), false).is_none());
        assert!(keymaps.action(key!(F(8)), false) == Some(Action::RunViewedFile));
    }

    /// Keys every editor gives one meaning keep it, and the keymaps list leaves
    /// them out so the shortcuts worth learning are easy to find.
    #[test]
    fn everyday_editing_keys_are_fixed_and_unlisted() {
        let mut keymaps = Keymaps::default();
        for edit in [
            InputAction::MoveLeft,
            InputAction::SelectUp,
            InputAction::DeleteBack,
            InputAction::Undo,
            InputAction::SelectAll,
        ] {
            let key = keymaps.key(edit).expect("everyday edits are bound");
            assert_eq!(keymaps.edit(key), Some(edit));
            assert!(!keymaps.rebind(edit, key!(F(24))));
            assert!(keymaps
                .remappable()
                .iter()
                .all(|(listed, _)| *listed != Shortcut::Edit(edit)));
        }
        for edit in [InputAction::GrowSelection, InputAction::MoveParagraphDown] {
            assert!(keymaps
                .remappable()
                .iter()
                .any(|(listed, _)| *listed == Shortcut::Edit(edit)));
        }
    }
}
