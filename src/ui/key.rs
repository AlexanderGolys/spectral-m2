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
        let desktop_chord =
            cfg!(not(target_os = "macos")) && event.modifiers.contains(CrosstermModifiers::SUPER);
        (event.kind != KeyEventKind::Release && !desktop_chord).then(|| {
            let mut key = KeyEvent::from(event);
            key.modifiers |= self.modifiers;
            key
        })
    }

    /// Whether the platform's primary command modifier is held on its own.
    pub const fn ctrl(&self) -> bool {
        self.modifiers & KeyMod::Ctrl as u8 != 0
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
        if self.code == KeyCode::BackTab {
            // The key is the shifted Tab; terminals differ on whether they say so.
            self.modifiers &= !(KeyMod::Shift as u8);
        }
        if self.code == KeyCode::Char('7') && self.ctrl() {
            self.code = KeyCode::Char('/');
        }
        if let KeyCode::Char(character) = self.code {
            if character.is_ascii_alphabetic() && !self.is_text() {
                // A letter's case is its Shift: terminals report Ctrl+Shift+D as `D`,
                // as `d` with Shift, or as `D` with Shift.
                if character.is_ascii_uppercase() {
                    self.modifiers |= KeyMod::Shift as u8;
                }
                self.code = KeyCode::Char(character.to_ascii_lowercase());
            } else if character != ' ' && !self.is_text() {
                // A symbol already names its shifted key.
                self.modifiers &= !(KeyMod::Shift as u8);
            }
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
                    @@"cmd"
                } else {
                    @@"ctrl"
                });
            }
            if self.alt() {
                parts.push(if cfg!(target_os = "macos") {
                    @@"option"
                } else {
                    @@"alt"
                });
            }
            if self.shift() {
                parts.push(@@"shift");
            }
            parts.push(match self.code {
                Char(character) => character.into(),
                Enter => @@"enter",
                Tab => @@"tab",
                Backspace => @@"backspace",
                Delete => @@"delete",
                Insert => @@"insert",
                Left => @@"left",
                Right => @@"right",
                Up => @@"up",
                Down => @@"down",
                Home => @@"home",
                End => @@"end",
                PageUp => @@"pg up",
                PageDown => @@"pg down",
                Esc => @@"esc",
                F(number) => format!("f{number}"),
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
            CrosstermModifiers::SUPER
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

