//! Terminal UI controllers, widgets, rendering, and styling.

pub(crate) mod action;
pub(crate) mod block;
pub(crate) mod cell;
pub(crate) mod config;
pub(crate) mod controller;
mod debug;
mod feed;
pub(crate) mod key;
pub(crate) mod render;
mod statusline;
mod syntax_tree;
pub(crate) mod terminal;
pub(crate) mod wrap;

pub(crate) const DEBUG_POST: &str = "2 + 2\n7;\nprint \"debug post\"";
