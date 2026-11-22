//! Key bindings as data.
//!
//! An action binding consumes its terminal key and emits an [`Action`]. Keys
//! without a binding continue to the focused component as ordinary input.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Display;

use crossterm::event::{KeyCode, MouseButton};
use KeyCode::*;

use ratatui::text::{Line, Span};
use serde::{Deserialize, Serialize};

use crate::input::{InputAction, Traversal};
use crate::ui::action::{Action, Opening};
use crate::ui::controller::picker::{PickerInput, PickerItem, PickerMode, PickerTab, Row};
use crate::ui::controller::EditorEvent;
use crate::ui::glyphs::{Glyphs, Spelled};
use crate::ui::key::{KeyEvent, KeyMod, MouseChord};
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

/// A shortcut and the key it sits on, such as submitting on `Enter`: a rebind
/// the user made, or a binding as it stands.
pub type ShortcutKey = (Shortcut, KeyEvent);

/// A way of opening links and the mouse chord it sits on, such as a new tab on
/// the middle button.
pub type GestureChord = (Opening, MouseChord);

/// The editor's exact normalized-key bindings, and the mouse chords that open
/// links.
#[derive(Clone)]
pub struct Keymaps {
    bindings: HashMap<KeyEvent, Keymap>,
    rebinds: Vec<ShortcutKey>,
    gestures: HashMap<MouseChord, Opening>,
    gesture_rebinds: Vec<GestureChord>,
}

impl Keymaps {
    /// The *keymap* tab: every remappable shortcut, grouped by context, and the
    /// button putting them back.
    pub fn tab(&self) -> PickerTab {
        let remappable = self.remappable();
        let mut items = self
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
        let mut gestures = self.gestures.iter().collect::<Vec<_>>();
        gestures.sort_by_key(|(_, opening)| **opening);
        items.extend(gestures.into_iter().map(|(&chord, &opening)| {
            Box::new(KeymapRow::new(opening, chord)) as Box<dyn PickerItem>
        }));
        items.push(Self::restore_button());
        PickerTab {
            mode: Some(PickerMode::Keymaps),
            label: "keymap",
            items,
        }
    }

    /// The button putting every shortcut back on its default key.
    pub fn restore_button() -> Box<dyn PickerItem> {
        let label = Span::styled("restore keymaps", scheme().settings_button());
        Row::enter(vec![label.into()], || EditorEvent::RestoreKeymaps)
            .described("put every shortcut back on its default key")
            .into()
    }

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

    /// The default keymaps with `rebinds` and `gestures` made over them, in order.
    pub fn replaying(rebinds: &[ShortcutKey], gestures: &[GestureChord]) -> Self {
        let mut keymaps = Self::default();
        for &(shortcut, key) in rebinds {
            keymaps.rebind(shortcut, key);
        }
        for &(opening, chord) in gestures {
            keymaps.rebind_gesture(opening, chord);
        }
        keymaps
    }

    /// How the link under a press of `chord` opens, if the chord opens links.
    pub fn opening(&self, chord: MouseChord) -> Option<Opening> {
        self.gestures.get(&chord).copied()
    }

    /// Every gesture rebind that took effect, in the order made.
    pub fn gesture_rebinds(&self) -> &[GestureChord] {
        &self.gesture_rebinds
    }

    /// Move `opening` onto `chord`, swapping with the opening already there. A
    /// bare left click never opens links, as it places the cursor and selects.
    pub fn rebind_gesture(&mut self, opening: Opening, chord: MouseChord) -> bool {
        let selecting = chord.bare() && chord.button == MouseButton::Left;
        let target = self
            .gestures
            .iter()
            .find_map(|(bound, there)| (*there == opening).then_some(*bound));
        let Some(target) = target.filter(|_| !selecting) else {
            return false;
        };
        if chord != target {
            match self.gestures.insert(chord, opening) {
                Some(displaced) => self.gestures.insert(target, displaced),
                None => self.gestures.remove(&target),
            };
            self.gesture_rebinds.push((opening, chord));
        }
        true
    }

    /// Every rebind that took effect, in the order made.
    pub fn rebinds(&self) -> &[ShortcutKey] {
        &self.rebinds
    }

    /// The bindings the user may move, with their current keys, in shortcut order.
    pub fn remappable(&self) -> Vec<ShortcutKey> {
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
                // A terminal without keyboard enhancement sends these as Enter and
                // Esc; Ctrl+I, which it sends as Tab, is merely out of reach there.
                if key.ctrl() && !key.alt() && !key.shift() && matches!(character, 'm' | '[') {
                    return false;
                }
            }
            _ => {}
        }
        match context {
            KeyContext::Global => {
                key.code == BackTab || (key.code != Tab && (key.ctrl() || key.alt() || key.shift()))
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

/// Something the user binds to a chord of its own kind: a shortcut to a key, a
/// way of opening links to a mouse button.
pub trait Bindable: Copy + Send + 'static {
    type Chord: Copy + Display + Send;

    fn description(self) -> &'static str;

    /// The group the binding is listed in, named beneath the list.
    fn group(self) -> &'static str;

    /// What to do to give the binding a new chord.
    const PROMPT: &'static str;

    /// The chord `input` offers a binding of this kind, if any.
    fn offered(input: &PickerInput) -> Option<Self::Chord>;

    fn rebound(self, chord: Self::Chord) -> EditorEvent;
}

impl Bindable for Shortcut {
    type Chord = KeyEvent;
    const PROMPT: &'static str = "Type keybind";

    fn description(self) -> &'static str {
        Shortcut::description(self)
    }

    fn group(self) -> &'static str {
        self.context().label()
    }

    fn offered(input: &PickerInput) -> Option<KeyEvent> {
        match input {
            PickerInput::Key(key) => Some(*key),
            _ => None,
        }
    }

    fn rebound(self, key: KeyEvent) -> EditorEvent {
        EditorEvent::Rebind(self, key)
    }
}

impl Bindable for Opening {
    type Chord = MouseChord;
    const PROMPT: &'static str = "Click with the new button";

    fn description(self) -> &'static str {
        Opening::description(&self)
    }

    fn group(self) -> &'static str {
        "mouse"
    }

    fn offered(input: &PickerInput) -> Option<MouseChord> {
        match input {
            PickerInput::Mouse(chord) => Some(*chord),
            _ => None,
        }
    }

    fn rebound(self, chord: MouseChord) -> EditorEvent {
        EditorEvent::RebindGesture(self, chord)
    }
}

/// A binding's row, which captures a replacement chord only after Enter.
pub struct KeymapRow<B: Bindable = Shortcut> {
    bound: B,
    chord: B::Chord,
    capturing: bool,
}

impl<B: Bindable> KeymapRow<B> {
    /// Show a binding and its current chord.
    pub fn new(bound: B, chord: B::Chord) -> Self {
        Self {
            bound,
            chord,
            capturing: false,
        }
    }
}

impl<B: Bindable> PickerItem for KeymapRow<B> {
    fn cells(&self) -> Vec<Line<'static>> {
        let groups = scheme();
        let chord = if self.capturing {
            format!("{}{} (Esc cancels)", B::PROMPT, Glyphs::Ellipsis.current())
        } else {
            self.chord.to_string()
        };
        vec![
            Span::styled(self.bound.description(), groups.keymap_action_label()).into(),
            Span::styled(chord, groups.style(HLGroup::KeymapHint)).into(),
        ]
    }

    fn description(&self) -> Option<&str> {
        Some(self.bound.group())
    }

    fn capturing(&self) -> bool {
        self.capturing
    }

    fn handle(&mut self, input: PickerInput) -> Option<EditorEvent> {
        match input {
            PickerInput::Key(KeyEvent { code: Esc, .. }) if self.capturing => {
                self.capturing = false;
                None
            }
            PickerInput::Action(Action::Submit) if !self.capturing => {
                self.capturing = true;
                None
            }
            input if self.capturing => B::offered(&input).map(|chord| self.bound.rebound(chord)),
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
                keymap!(A::OpenSettings => Char(',') + Ctrl),
                keymap!(A::OpenKeymaps => Char('k') + Ctrl + Shift),
                keymap!(A::ClearFeed => Char('l') + Alt),
                keymap!(A::OpenCode => F(2)),
                keymap!(A::OpenOutput => F(5)),
                keymap!(A::OpenSyntaxTree => F(6)),
                keymap!(A::Restart => Char('n') + Ctrl),
                keymap!(A::ResumeSession => Char('r') + Ctrl),
                keymap!(A::CloseTab => Char('q') + Ctrl),
                keymap!(A::HistoryBack => Char('o') + Ctrl),
                keymap!(A::HistoryForward => Char('i') + Ctrl),
                // A terminal without keyboard enhancement sends Ctrl+I as Tab.
                keymap!(alias A::HistoryForward => Char('o') + Ctrl + Shift),
                keymap!(A::FocusPanel => BackTab),
                keymap!(A::TogglePanel => Char('b') + Ctrl),
                keymap!(fixed A::ToggleHighlightDebug => Char('d') + Ctrl + Shift),
                keymap!(A::SaveSelection => Char('s') + Ctrl + Shift),
                keymap!(A::BrowseSessions => F(7)),
                keymap!(A::RunViewedFile => F(8)),
                keymap!(A::CopySelection => Char('y') + Ctrl),
                keymap!(A::SelectPreviousBlock => Up + Alt),
                keymap!(A::SelectNextBlock => Down + Alt),
                keymap!(A::Describe => Char('k') + Alt),
                keymap!(A::DescribeBeside => Char(']') + Alt),
                keymap!(A::GoToDefinition => Char(']') + Ctrl),
                // What a terminal without keyboard enhancement sends for Ctrl+].
                keymap!(alias A::GoToDefinition => Char('5') + Ctrl),
                keymap!(A::GoToDocumentation => Char('k') + Ctrl),
                keymap!(A::FormatInput => Char('f') + Alt + Shift),
                keymap!(A::ImportPackage => Char('i') + Alt),
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
                // Shift+Tab steps back too while candidates are offered, as the
                // key moving between the session and the view beside it.
                keymap!(fixed I::PreviousCompletion => Tab + Ctrl),
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
                keymap!(I::InsertShuffle => Char('s') + Alt),
                keymap!(I::InsertDot => Char('.') + Alt),
                keymap!(I::InsertTensor => Char('t') + Alt),
                keymap!(I::DeleteLine => Char('u') + Ctrl),
                keymap!(I::DeleteSpacedWordBack => Char('w') + Ctrl),
                keymap!(I::Clear => Char('l') + Ctrl),
                keymap!(I::HistoryPrevious => Up + Ctrl),
                keymap!(I::HistoryNext => Down + Ctrl),
            ]),
            rebinds: Vec::new(),
            gestures: HashMap::from([
                (MouseButton::Left + Ctrl, Opening::Here),
                (MouseChord::new(MouseButton::Middle), Opening::BesideAndJump),
                (MouseButton::Middle + Ctrl, Opening::Beside),
            ]),
            gesture_rebinds: Vec::new(),
        }
    }
}
