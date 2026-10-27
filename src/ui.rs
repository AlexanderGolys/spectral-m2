//! Terminal UI controllers, widgets, rendering, and styling.

pub mod action;
pub mod block;
pub mod cell;
pub mod config;
pub mod controller;
pub mod cycle;
mod debug;
// Groundwork for the event-driven element hierarchy, not yet wired into the running
// UI. Drop these allows as each module gains its first caller.
pub mod element;
mod feed;
pub mod geometry;
pub mod graphics;
pub mod key;
pub mod present;
pub mod render;
mod statusline;
mod syntax_tree;
pub mod terminal;
pub mod text_object;
pub mod wrap;
