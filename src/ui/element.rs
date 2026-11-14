//! What the UI's pieces share: the listener a component implements to fold an ambient
//! change into its own state, and the one place a child's cells are placed in a parent.
//!
//! Placement stays the parent's business. A child's cells are local with their origin at
//! zero and are [`paste`]d at a shift the owner alone chose, so a coordinate is
//! translated once.

use std::ops::Range;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// A component that folds one kind of ambient change into whatever state it retains.
///
/// The event belongs to the update, never to the drawing: it is the input that makes a
/// presentation settle, and once settled, redrawing needs none of it. The kind is a
/// parameter rather than an associated type so one component can listen for several
/// unrelated changes without folding them into an enum most implementors would ignore.
pub trait EventListener<Event> {
    fn on_event(&mut self, context: &Event);
}

/// Copy a child's local `rows` into `into`, clipped to that region and to `parent`.
///
/// This is the one place a coordinate is translated: a child settles at a zero origin
/// and never learns where it sits, so the shift is applied once, by the owner that
/// chose it. Anything falling outside either the destination or the parent is dropped
/// rather than wrapped, so a viewport can paste a slice of a child larger than itself.
pub fn paste(child: &Buffer, rows: Range<u16>, into: Rect, parent: &mut Buffer) {
    let source = *child.area();
    let clip = into.intersection(*parent.area());
    let width = usize::from(source.width.min(clip.width));
    if clip.is_empty() || width == 0 {
        return;
    }
    for (offset, row) in (rows.start..rows.end.min(source.height)).enumerate() {
        let Ok(offset) = u16::try_from(offset) else {
            break;
        };
        if offset >= clip.height {
            break;
        }
        let from = child.index_of(source.x, source.y + row);
        let to = parent.index_of(clip.x, clip.y + offset);
        parent.content[to..to + width].clone_from_slice(&child.content[from..from + width]);
    }
}
