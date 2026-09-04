//! Terminal UI controllers, widgets, rendering, and styling.

pub mod action;
pub mod block;
pub mod config;
pub mod controller;
mod debug;
pub mod element;
mod feed;
pub mod key;
mod statusline;
pub mod terminal;

pub const DEBUG_POST: &str = "2 + 2\n7;\nprint \"debug post\"";
