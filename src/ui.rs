//! Terminal UI controllers, widgets, rendering, and styling.

pub(crate) mod action;
pub(crate) mod block;
pub(crate) mod cell;
pub(crate) mod config;
pub(crate) mod controller;
mod debug;
// Groundwork for the event-driven element hierarchy, not yet wired into the running
// UI. Drop these allows as each module gains its first caller.
#[allow(dead_code)]
pub(crate) mod element;
mod feed;
#[allow(dead_code)]
pub(crate) mod geometry;
pub(crate) mod key;
#[allow(dead_code)]
pub(crate) mod present;
pub(crate) mod render;
mod statusline;
mod syntax_tree;
pub(crate) mod terminal;
pub(crate) mod wrap;

pub(crate) const DEBUG_POST: &str = "2 + 2\n7;\nprint \"debug post\"";
