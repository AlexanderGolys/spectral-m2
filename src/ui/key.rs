//! Platform-normalized keyboard input.

use std::fmt::{self, Display, Formatter};
use std::ops::Add;

use crossterm::event::{
    KeyCode, KeyEvent as CrosstermKeyEvent, KeyEventKind, KeyModifiers as CrosstermModifiers,
    ModifierKeyCode, MouseButton, MouseEvent, MouseEventKind,
};
use serde::{Deserialize, Serialize};

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

/// A mouse button pressed with modifiers held, such as Ctrl with the middle
/// button, written `ctrl+middle click`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MouseChord {
    pub button: MouseButton,
    modifiers: u8,
}

impl MouseChord {
    pub const fn new(button: MouseButton) -> Self {
        Self {
            button,
            modifiers: 0,
        }
    }

    /// The chord `event` presses; none unless it is a button going down.
    pub fn pressed(event: &MouseEvent) -> Option<Self> {
        let MouseEventKind::Down(button) = event.kind else {
            return None;
        };
        Some(Self {
            button,
            modifiers: KeyMod::held(event.modifiers),
        })
    }

    /// Whether no modifier is held with the button.
    pub const fn bare(self) -> bool {
        self.modifiers == 0
    }
}

impl Add<KeyMod> for MouseButton {
    type Output = MouseChord;

    fn add(self, modifier: KeyMod) -> Self::Output {
        MouseChord {
            button: self,
            modifiers: modifier as u8,
        }
    }
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
    /// The logical modifiers among those a terminal reports held, as bits.
    fn held(modifiers: CrosstermModifiers) -> u8 {
        let ctrl = if cfg!(target_os = "macos") {
            CrosstermModifiers::SUPER
        } else {
            CrosstermModifiers::CONTROL
        };
        [
            (ctrl, Self::Ctrl),
            (CrosstermModifiers::ALT, Self::Alt),
            (CrosstermModifiers::SHIFT, Self::Shift),
        ]
        .into_iter()
        .filter(|(reported, _)| modifiers.intersects(*reported))
        .fold(0, |held, (_, modifier)| held | modifier as u8)
    }

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
    impl KeyMod {
        /// The names of the modifiers in `modifiers`, in the order a chord is
        /// written.
        fn names(modifiers: u8) -> Vec<String> {
            let held = |modifier: Self| modifiers & modifier as u8 != 0;
            let mut names = Vec::new();
            if held(Self::Ctrl) {
                names.push(if cfg!(target_os = "macos") {
                    @@"cmd"
                } else {
                    @@"ctrl"
                });
            }
            if held(Self::Alt) {
                names.push(if cfg!(target_os = "macos") {
                    @@"option"
                } else {
                    @@"alt"
                });
            }
            if held(Self::Shift) {
                names.push(@@"shift");
            }
            names
        }
    }

    impl Display for MouseChord {
        fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
            let mut parts = KeyMod::names(self.modifiers);
            parts.push(match self.button {
                MouseButton::Left => @@"click",
                MouseButton::Middle => @@"middle click",
                MouseButton::Right => @@"right click",
            });
            formatter.write_str(&parts.join("+"))
        }
    }

    impl Display for KeyEvent {
        fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
            use KeyCode::*;
            let mut parts = KeyMod::names(self.modifiers);
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
        Self {
            code: event.code,
            modifiers: KeyMod::held(event.modifiers),
        }
    }
}
