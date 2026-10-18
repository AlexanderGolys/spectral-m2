//! Terminal UI controllers, widgets, rendering, and styling.

pub mod action;
pub mod block;
pub mod cell;
pub mod config;
pub mod controller;
mod debug;
// Groundwork for the event-driven element hierarchy, not yet wired into the running
// UI. Drop these allows as each module gains its first caller.
#[allow(dead_code)]
pub mod element;
mod feed;
#[allow(dead_code)]
pub mod geometry;
pub mod key;
#[allow(dead_code)]
pub mod present;
pub mod render;
mod statusline;
mod syntax_tree;
pub mod terminal;
pub mod wrap;
