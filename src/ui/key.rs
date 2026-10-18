//! Platform-normalized keyboard input.

use std::fmt::{self, Display, Formatter};
use std::ops::Add;

use crossterm::event::{
    KeyCode, KeyEvent as CrosstermKeyEvent, KeyEventKind, KeyModifiers as CrosstermModifiers,
    ModifierKeyCode,
};
use serde::{Deserialize, Serialize};
use these_macros_should_be_illegal::literally_literal_string;

/// One of the three logical modifiers supported by the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum KeyMod {
    Ctrl = 1,
    Alt = 2,
    Shift = 4,
}

/// A compact key event using the UI's three logical modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyEvent {
    pub code: KeyCode,
    modifiers: u8,
}

/// Held modifier state used when a terminal omits modifiers from another key.
#[derive(Default)]
pub struct Keyboard {
    modifiers: u8,
}

impl Keyboard {
    /// Normalize one raw event, consuming modifier and key-release notifications.
    pub fn event(&mut self, event: CrosstermKeyEvent) -> Option<KeyEvent> {
        if let KeyCode::Modifier(code) = event.code {
            if let Some(modifier) = KeyMod::from(code) {
                match event.kind {
                    KeyEventKind::Release => self.modifiers &= !(modifier as u8),
                    _ => self.modifiers |= modifier as u8,
                }
            }
            return None;
        }
        (event.kind != KeyEventKind::Release).then(|| {
            let mut key = KeyEvent::from(event);
            key.modifiers |= self.modifiers;
            key
        })
    }
}

impl KeyMod {
    fn from(code: ModifierKeyCode) -> Option<Self> {
        use ModifierKeyCode::*;
        match code {
            LeftShift | RightShift => Some(Self::Shift),
            LeftAlt | RightAlt => Some(Self::Alt),
            LeftControl | RightControl if cfg!(not(target_os = "macos")) => Some(Self::Ctrl),
            LeftSuper | RightSuper if cfg!(target_os = "macos") => Some(Self::Ctrl),
            _ => None,
        }
    }
}

impl KeyEvent {
    /// Create an unmodified key event.
    pub const fn new(code: KeyCode) -> Self {
        Self { code, modifiers: 0 }
    }

    /// Whether this event carries the platform's primary command modifier.
    pub const fn ctrl(self) -> bool {
        self.modifiers & KeyMod::Ctrl as u8 != 0
    }

    /// Whether this event carries Alt, called Option on macOS.
    pub const fn alt(self) -> bool {
        self.modifiers & KeyMod::Alt as u8 != 0
    }

    /// Whether this event carries Shift.
    pub const fn shift(self) -> bool {
        self.modifiers & KeyMod::Shift as u8 != 0
    }

    /// Whether this event can be inserted as ordinary text.
    pub const fn is_text(self) -> bool {
        !self.ctrl() && !self.alt()
    }

    /// This event with `modifier` held as well.
    pub const fn with(mut self, modifier: KeyMod) -> Self {
        self.modifiers |= modifier as u8;
        self
    }

    /// Normalize terminal encodings that should address the same binding.
    pub fn binding(mut self) -> Self {
        if self.code == KeyCode::Char('7') && self.ctrl() {
            self.code = KeyCode::Char('/');
        }
        if matches!(self.code, KeyCode::Char(character) if character != ' ') && !self.is_text() {
            self.modifiers &= !(KeyMod::Shift as u8);
        }
        self
    }
}

impl Add<KeyMod> for KeyCode {
    type Output = KeyEvent;

    fn add(self, modifier: KeyMod) -> Self::Output {
        KeyEvent::from(self) + modifier
    }
}

impl Add<KeyMod> for KeyEvent {
    type Output = Self;

    fn add(self, modifier: KeyMod) -> Self::Output {
        self.with(modifier)
    }
}

/// A key event written as its chord, usable in `const` items:
/// `key!(ctrl + alt + k)`, or `key!(ctrl + '/')` for a character that is not a
/// letter.
#[macro_export]
macro_rules! key {
    (alt + $($rest:tt)+) => {
        $crate::key!($($rest)+).with($crate::ui::key::KeyMod::Alt)
    };
    (ctrl + $($rest:tt)+) => {
        $crate::key!($($rest)+).with($crate::ui::key::KeyMod::Ctrl)
    };
    (shift + $($rest:tt)+) => {
        $crate::key!($($rest)+).with($crate::ui::key::KeyMod::Shift)
    };
    ($letter:ident) => {
        $crate::key!(stringify!($letter).as_bytes()[0] as char)
    };
    ($character:expr) => {
        $crate::ui::key::KeyEvent::new(::crossterm::event::KeyCode::Char($character))
    };
}

literally_literal_string! {
    impl Display for KeyEvent {
        fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
            use KeyCode::*;
            let mut parts = Vec::new();
            if self.ctrl() {
                parts.push(if cfg!(target_os = "macos") {
                    @@"⌘"
                } else {
                    @@"ctrl"
                });
            }
            if self.alt() {
                parts.push(if cfg!(target_os = "macos") {
                    @@"⌥"
                } else {
                    @@"alt"
                });
            }
            if self.shift() {
                parts.push(@@"⇧");
            }
            parts.push(match self.code {
                Char(character) => character.into(),
                Enter => @@"↵",
                Tab => @@"⇥",
                BackTab => @@"⇤",
                Backspace => @@"⌫",
                Delete => @@"delete",
                Insert => @@"insert",
                Left => @@"←",
                Right => @@"→",
                Up => @@"↑",
                Down => @@"↓",
                Home => @@"home",
                End => @@"end",
                PageUp => @@"page up",
                PageDown => @@"page down",
                Esc => @@"esc",
                other => format!("{other:?}").to_lowercase(),
            });
            formatter.write_str(&parts.join("+"))
        }
    }
}

impl From<KeyCode> for KeyEvent {
    fn from(code: KeyCode) -> Self {
        Self::new(code)
    }
}

impl From<CrosstermKeyEvent> for KeyEvent {
    fn from(event: CrosstermKeyEvent) -> Self {
        use KeyMod::*;
        let mut key = Self::new(event.code);
        let ctrl = if cfg!(target_os = "macos") {
            CrosstermModifiers::SUPER | CrosstermModifiers::CONTROL
        } else {
            CrosstermModifiers::CONTROL
        };
        if event.modifiers.intersects(ctrl) {
            key = key + Ctrl;
        }
        if event.modifiers.contains(CrosstermModifiers::ALT) {
            key = key + Alt;
        }
        if event.modifiers.contains(CrosstermModifiers::SHIFT) {
            key = key + Shift;
        }
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use KeyMod::*;

    #[test]
    fn a_written_chord_is_the_event_its_keys_make() {
        const CHORDS: [KeyEvent; 3] = [key!(ctrl + o), key!(ctrl + alt + k), key!(ctrl + '/')];

        assert_eq!(CHORDS[0], KeyCode::Char('o') + Ctrl);
        assert_eq!(CHORDS[1], KeyCode::Char('k') + Ctrl + Alt);
        assert_eq!(CHORDS[2], KeyCode::Char('/') + Ctrl);
    }

    #[test]
    fn crossterm_primary_modifier_becomes_logical_ctrl() {
        let modifier = if cfg!(target_os = "macos") {
            CrosstermModifiers::SUPER
        } else {
            CrosstermModifiers::CONTROL
        };
        let terminal = CrosstermKeyEvent::new(KeyCode::Char('x'), modifier);
        assert!(KeyEvent::from(terminal).ctrl());
    }

    #[test]
    fn bindings_normalize_modified_characters() {
        assert_eq!(
            (KeyCode::Char('c') + Ctrl + Shift).binding(),
            KeyCode::Char('c') + Ctrl
        );
        assert_eq!(
            (KeyCode::Char('7') + Ctrl).binding(),
            KeyCode::Char('/') + Ctrl
        );
    }

    #[test]
    fn modifier_addition_is_commutative() {
        assert_eq!(KeyCode::Left + Ctrl + Shift, KeyCode::Left + Shift + Ctrl);
    }

    #[test]
    fn held_shift_repairs_an_unmodified_enter_event() {
        let mut keyboard = Keyboard::default();
        let shift = KeyCode::Modifier(ModifierKeyCode::LeftShift);
        let modifier =
            |kind| CrosstermKeyEvent::new_with_kind(shift, CrosstermModifiers::SHIFT, kind);
        let release = CrosstermKeyEvent::new_with_kind(
            KeyCode::Enter,
            CrosstermModifiers::NONE,
            KeyEventKind::Release,
        );
        let enter = || {
            CrosstermKeyEvent::new_with_kind(
                KeyCode::Enter,
                CrosstermModifiers::NONE,
                KeyEventKind::Press,
            )
        };

        assert!(keyboard.event(modifier(KeyEventKind::Press)).is_none());
        assert_eq!(keyboard.event(enter()), Some(KeyCode::Enter + Shift));
        assert!(keyboard.event(release).is_none());
        assert!(keyboard.event(modifier(KeyEventKind::Release)).is_none());
        assert_eq!(keyboard.event(enter()), Some(KeyCode::Enter.into()));
    }
}
