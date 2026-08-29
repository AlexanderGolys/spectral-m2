//! Terminal user-interface controllers, widgets, rendering, and styling.

pub mod block;
mod controller;
mod statusline;
pub mod terminal;

pub(crate) use controller::EditorConfig;
pub use controller::{
    submitted_command, Editor, EditorAction, EditorUpdate, Groups, OpenTarget, SubmittedCommand,
    ViewKind, Viewport,
};
pub use terminal::ReplTerminal;
