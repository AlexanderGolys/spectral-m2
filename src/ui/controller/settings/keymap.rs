//! Key bindings as data.
//!
//! Every key the editor responds to resolves to an [`Action`] through a
//! [`Keymap`], so the same table drives dispatch, the status line's hints, and
//! the bindings picker. Nothing reads a raw [`KeyCode`] outside this module.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A command the editor can perform. Actions are what keys mean, so rebinding is
/// a change to the table rather than to the dispatcher.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    Submit,
    InsertNewline,
    ScrollFeedUp,
    ScrollFeedDown,
    OpenCommands,
    OpenSettings,
    ToggleHighlightDebug,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveLineStart,
    MoveLineEnd,
    SelectLeft,
    SelectRight,
    SelectLineStart,
    SelectLineEnd,
    CopySelection,
    ClearInput,
    DeleteBack,
    DeleteForward,
    InsertTab,
}

impl Action {
    /// What this action does, for the bindings picker.
    pub const fn description(&self) -> &'static str {
        use Action::*;
        match self {
            Quit => "quit spectral-m2",
            Submit => "evaluate the input",
            InsertNewline => "insert a line break",
            ScrollFeedUp => "scroll the feed up",
            ScrollFeedDown => "scroll the feed down",
            OpenCommands => "open commands",
            OpenSettings => "open settings",
            ToggleHighlightDebug => "toggle highlight groups",
            MoveLeft => "move left",
            MoveRight => "move right",
            MoveUp => "move up, or recall older input",
            MoveDown => "move down, or recall newer input",
            MoveLineStart => "move to line start",
            MoveLineEnd => "move to line end",
            SelectLeft => "extend selection left",
            SelectRight => "extend selection right",
            SelectLineStart => "extend selection to line start",
            SelectLineEnd => "extend selection to line end",
            CopySelection => "copy the selection",
            ClearInput => "clear input text",
            DeleteBack => "delete before the cursor",
            DeleteForward => "delete under the cursor",
            InsertTab => "insert a tab",
        }
    }

    pub const REMAPPABLE: [Self; 2] = [Self::ClearInput, Self::CopySelection];

    pub fn is_remappable(self) -> bool {
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
    fn new(code: KeyCode, modifiers: KeyModifiers, action: Action) -> Self {
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
            Char(' ') => "space".to_string(),
            Char(character) => character.to_string(),
            Enter => "enter".to_string(),
            Tab => "tab".to_string(),
            Backspace => "backspace".to_string(),
            Delete => "delete".to_string(),
            Left => "left".to_string(),
            Right => "right".to_string(),
            Up => "up".to_string(),
            Down => "down".to_string(),
            Home => "home".to_string(),
            End => "end".to_string(),
            PageUp => "page up".to_string(),
            PageDown => "page down".to_string(),
            Esc => "esc".to_string(),
            other => format!("{other:?}").to_lowercase(),
        });
        parts.join("+")
    }

    /// Does this binding claim `key`? Shift is ignored for printable characters,
    /// since the terminal already reports the shifted character itself.
    fn matches(&self, key: KeyEvent) -> bool {
        if self.code != key.code {
            return false;
        }
        let mask = if matches!(self.code, KeyCode::Char(_)) && !self.modifiers.is_empty() {
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
                binding!(Ctrl - 'd', ToggleHighlightDebug),
                binding!(Ctrl - 'y', CopySelection),
                binding!(Ctrl - 'l', ClearInput),
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
            keymap.key_for(Action::InsertNewline).as_deref(),
            Some("shift+enter")
        );
        assert_eq!(keymap.key_for(Action::MoveLeft).as_deref(), Some("left"));
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
}
