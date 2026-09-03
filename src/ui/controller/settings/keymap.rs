//! Key bindings as data.
//!
//! Every key the editor responds to resolves to an [`Action`] through a
//! [`Keymap`], so the same table drives dispatch, the status line's hints, and
//! the bindings picker. Nothing reads a raw [`KeyCode`] outside this module.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use these_macros_should_be_illegal::discriminated_str;

use crate::ui::controller::{icons::Icon, OpenTarget};
/// A finite keymap target. Real commands keep their callbacks and properties in
/// [`Command`](crate::ui::controller::commands::Command); input actions stay here.
#[discriminated_str(description)]
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit = "quit spectral-m2",
    Submit = "evaluate the input",
    InsertNewline = "insert a line break",
    ScrollFeedUp = "scroll the feed up",
    ScrollFeedDown = "scroll the feed down",
    OpenCommands = "open commands",
    OpenSettings = "open settings",
    OpenStyle = "open style",
    OpenKeymaps = "open keymaps",
    ClearFeed = "clear the feed",
    OpenCode = "open code",
    NewWorkspace = "open a new workspace",
    OpenOutput = "open output",
    OpenSyntaxTree = "open syntax tree",
    Restart = "restart Macaulay2",
    ClosePane = "close the focused pane",
    ToggleHighlightDebug = "toggle highlight groups",
    MoveLeft = "move left",
    MoveRight = "move right",
    MoveUp = "move up, or recall older input",
    MoveDown = "move down, or recall newer input",
    MoveLineStart = "move to line start",
    MoveLineEnd = "move to line end",
    SelectLeft = "extend selection left",
    SelectRight = "extend selection right",
    SelectLineStart = "extend selection to line start",
    SelectLineEnd = "extend selection to line end",
    CopySelection = "copy the selection",
    ClearInput = "clear input text",
    DeleteBack = "delete before the cursor",
    DeleteForward = "delete under the cursor",
    InsertTab = "insert a tab",
}

impl Action {
    pub const REMAPPABLE: &'static [Self] = &[
        Self::OpenCommands,
        Self::OpenSettings,
        Self::OpenStyle,
        Self::OpenKeymaps,
        Self::ClearFeed,
        Self::OpenCode,
        Self::NewWorkspace,
        Self::OpenOutput,
        Self::OpenSyntaxTree,
        Self::Restart,
        Self::ClosePane,
        Self::Quit,
        Self::ToggleHighlightDebug,
        Self::ClearInput,
        Self::CopySelection,
    ];

    fn is_remappable(self) -> bool {
        Self::REMAPPABLE.contains(&self)
    }
}

/// One key, its modifiers, and what it does.
#[derive(Clone)]
pub struct Binding {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
    pub action: Action,
}

impl Binding {
    pub fn new(code: KeyCode, modifiers: KeyModifiers, action: Action) -> Self {
        Self {
            code,
            modifiers,
            action,
        }
    }

    /// How this key reads to a user: `ctrl+c`, `shift+enter`, `left`.
    pub fn label(&self) -> String {
        use KeyCode::*;
        let mut parts = Vec::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push("ctrl".to_string());
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push("alt".to_string());
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            parts.push("shift".to_string());
        }
        parts.push(match self.code {
            Char(' ') => Icon::Space.symbol().into(),
            Char(character) => character.into(),
            Enter => Icon::Enter.symbol().into(),
            Tab => Icon::Tab.symbol().into(),
            Backspace => Icon::Backspace.symbol().into(),
            Delete => "del".into(),
            Left => Icon::Left.symbol().into(),
            Right => Icon::Right.symbol().into(),
            Up => Icon::Up.symbol().into(),
            Down => Icon::Down.symbol().into(),
            Home => "home".into(),
            End => "end".into(),
            PageUp => "pgup".into(),
            PageDown => "pgdn".into(),
            Esc => "esc".into(),
            other => format!("{other:?}").to_lowercase(),
        });
        parts.join("+")
    }

    /// Does this binding claim `key`? Shift is ignored for printable characters
    /// other than space, since the terminal reports the shifted character itself.
    fn matches(&self, key: KeyEvent) -> bool {
        if self.code != key.code
            && !(self.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(
                    (self.code, key.code),
                    (KeyCode::Char('/'), KeyCode::Char('7'))
                ))
        {
            return false;
        }
        let mask = if matches!(self.code, KeyCode::Char(character) if character != ' ')
            && !self.modifiers.is_empty()
        {
            KeyModifiers::CONTROL | KeyModifiers::ALT
        } else {
            KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT
        };
        key.modifiers & mask == self.modifiers & mask
    }
}

/// The editor's bindings, in the order the bindings picker lists them.
#[derive(Clone)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Keymap {
    /// The action `key` triggers, if any. The first matching binding wins, so
    /// more specific combinations are listed before their bare keys.
    pub fn action(&self, key: KeyEvent) -> Option<Action> {
        self.bindings
            .iter()
            .find(|binding| binding.matches(key))
            .map(|binding| binding.action)
    }

    /// The key that triggers `action`, for hints and help.
    pub fn key_for(&self, action: Action) -> Option<String> {
        self.bindings
            .iter()
            .find(|binding| binding.action == action)
            .map(Binding::label)
    }

    pub fn rebind(&mut self, action: Action, key: KeyEvent) -> bool {
        if !action.is_remappable()
            || OpenTarget::from_key(key, true) == Some(OpenTarget::Split)
            || matches!(key.code, KeyCode::Char(_))
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        let Some(target) = self
            .bindings
            .iter()
            .position(|binding| binding.action == action)
        else {
            return false;
        };
        let conflict = self
            .bindings
            .iter()
            .position(|binding| binding.matches(key));
        if conflict.is_some_and(|index| !self.bindings[index].action.is_remappable()) {
            return false;
        }

        if let Some(conflict) = conflict.filter(|index| *index != target) {
            let old_code = self.bindings[target].code;
            let old_modifiers = self.bindings[target].modifiers;
            self.bindings[conflict].code = old_code;
            self.bindings[conflict].modifiers = old_modifiers;
        }
        self.bindings[target].code = key.code;
        self.bindings[target].modifiers = key.modifiers;
        true
    }
}

macro_rules! mod_key {
    (Ctrl) => {
        KeyModifiers::CONTROL
    };
    (Shift) => {
        KeyModifiers::SHIFT
    };
    (Alt) => {
        KeyModifiers::ALT
    };
    ($a:literal) => {
        KeyCode::Char($a)
    };
    ($a:ident) => {
        KeyCode::$a
    };
    () => {
        KeyModifiers::NONE
    };
}

macro_rules! binding {
    ($mod:ident - $key:tt, $action:ident) => {
        Binding::new(mod_key! {$key}, mod_key!($mod), Action::$action)
    };
    ($key:tt, $action:ident) => {
        Binding::new(mod_key! {$key}, mod_key!(), Action::$action)
    };
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            bindings: vec![
                binding!(Ctrl - 'c', Quit),
                binding!(Ctrl - '/', OpenCommands),
                binding!(Ctrl - 'o', OpenSettings),
                binding!(Alt - 's', OpenStyle),
                binding!(Alt - 'k', OpenKeymaps),
                binding!(Alt - 'l', ClearFeed),
                binding!(Alt - 'c', OpenCode),
                binding!(Alt - 'n', NewWorkspace),
                binding!(Alt - 'o', OpenOutput),
                binding!(Alt - 't', OpenSyntaxTree),
                binding!(Alt - 'r', Restart),
                binding!(Alt - 'w', ClosePane),
                binding!(Ctrl - 'd', ToggleHighlightDebug),
                binding!(Ctrl - 'y', CopySelection),
                binding!(Ctrl - 'l', ClearInput),
                binding!(Shift - ' ', InsertNewline),
                binding!(Shift - Enter, InsertNewline),
                binding!(Alt - Enter, InsertNewline),
                binding!(Enter, Submit),
                binding!(PageUp, ScrollFeedUp),
                binding!(PageDown, ScrollFeedDown),
                binding!(Shift - Left, SelectLeft),
                binding!(Shift - Right, SelectRight),
                binding!(Shift - Home, SelectLineStart),
                binding!(Shift - End, SelectLineEnd),
                binding!(Left, MoveLeft),
                binding!(Right, MoveRight),
                binding!(Up, MoveUp),
                binding!(Down, MoveDown),
                binding!(Home, MoveLineStart),
                binding!(End, MoveLineEnd),
                binding!(Backspace, DeleteBack),
                binding!(Delete, DeleteForward),
                binding!(Tab, InsertTab),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modified_bindings_win_over_their_bare_key() {
        let keymap = Keymap::default();
        let plain = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let shifted = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);

        assert!(keymap.action(plain) == Some(Action::Submit));
        assert!(keymap.action(shifted) == Some(Action::InsertNewline));
    }

    #[test]
    fn shift_space_inserts_newline_without_claiming_plain_space() {
        let keymap = Keymap::default();

        assert!(
            keymap.action(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::SHIFT))
                == Some(Action::InsertNewline)
        );
        assert!(keymap
            .action(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
            .is_none());
    }

    // The terminal reports a shifted letter as the letter itself, so a binding on
    // a character must not demand the shift state match.
    #[test]
    fn character_bindings_ignore_shift() {
        let keymap = Keymap::default();
        let shifted = KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );

        assert!(keymap.action(shifted) == Some(Action::Quit));
    }

    #[test]
    fn control_slash_accepts_legacy_terminal_encoding() {
        let keymap = Keymap::default();
        let legacy = KeyEvent::new(KeyCode::Char('7'), KeyModifiers::CONTROL);

        assert!(keymap.action(legacy) == Some(Action::OpenCommands));
    }

    #[test]
    fn keys_are_labelled_for_display() {
        let keymap = Keymap::default();
        assert_eq!(
            keymap.key_for(Action::OpenSettings).as_deref(),
            Some("ctrl+o")
        );
        assert_eq!(
            keymap.key_for(Action::OpenCommands).as_deref(),
            Some("ctrl+/")
        );
        assert_eq!(
            keymap.key_for(Action::InsertNewline),
            Some(format!("shift+{}", Icon::Space.symbol()))
        );
        assert_eq!(
            keymap.key_for(Action::MoveLeft),
            Some(Icon::Left.symbol().into())
        );
    }

    #[test]
    fn remapping_swaps_remappable_conflicts_and_preserves_fixed_keys() {
        let mut keymap = Keymap::default();
        let copy = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL);
        let left = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);

        assert!(keymap.rebind(Action::ClearInput, copy));
        assert!(keymap.action(copy) == Some(Action::ClearInput));
        assert!(
            keymap.action(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL))
                == Some(Action::CopySelection)
        );
        assert!(!keymap.rebind(Action::ClearInput, left));
        assert!(keymap.action(left) == Some(Action::MoveLeft));
    }

    #[test]
    fn plain_typing_cannot_be_claimed_by_a_remapping() {
        let mut keymap = Keymap::default();
        let letter = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);

        assert!(!keymap.rebind(Action::ClearInput, letter));
        assert!(keymap.action(letter).is_none());
    }

    #[test]
    fn alternative_view_cannot_be_claimed_by_a_remapping() {
        let mut keymap = Keymap::default();
        let alternative = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT);

        assert!(!keymap.rebind(Action::ClearInput, alternative));
        assert!(keymap.action(alternative).is_none());
    }
}
