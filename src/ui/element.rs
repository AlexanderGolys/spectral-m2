//! Time-driven elements: the three things an animation must supply, and the machinery
//! that turns them into a working one.
//!
//! An implementor provides only what is specific to it — how large it is, what it draws,
//! and what one tick does to it. Everything shared lives in [`Animated`]: accumulating
//! elapsed time, deciding when a step is due, redrawing exactly then and not otherwise,
//! and holding the cells in between. Two implementations differing only in `tick` are
//! two different animations with no other code written.
//!
//! Placement stays the parent's business. Cells are local with their origin at zero and
//! are [`paste`]d at a shift the owner alone chose, so a coordinate is translated once.

use std::num::NonZeroUsize;
use std::ops::Range;

use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::widgets::Widget;

type Milliseconds = usize;

/// Time elapsed since the previous tick.
pub struct TickEvent {
    pub dt: Milliseconds,
}

/// A component that folds one kind of ambient change into whatever state it retains.
///
/// The event belongs to the update, never to the drawing: it is the input that makes a
/// presentation settle, and once settled, redrawing needs none of it. The kind is a
/// parameter rather than an associated type so one component can listen for several
/// unrelated changes without folding them into an enum most implementors would ignore.
pub trait EventListener<Event> {
    fn on_event(&mut self, context: &Event);
}

/// Everything an animation must supply, and nothing it shares with other animations.
pub(crate) trait Animation {
    /// Extent required to draw everything.
    fn size(&self) -> Size;

    /// Draw into a zero-origin buffer of exactly [`Animation::size`].
    fn paint(&self, buffer: &mut Buffer);

    /// Advance by exactly one step.
    fn tick(&mut self);
}

/// Any [`Animation`] driven by elapsed time, holding the cells it last drew.
///
/// Time acts on the content as the additive monoid of durations: no time is no change,
/// and time delivered in pieces does what its sum would have done. Keeping the
/// remainder below one period is what makes that hold — discarding it would let one
/// long tick differ from the several short ticks composing it, so a stutter in the
/// event loop would quietly slow the animation instead of catching it up.
pub(crate) struct Animated<T: Animation> {
    content: T,
    buffer: Buffer,
    period: NonZeroUsize,
    accumulator: Milliseconds,
}

impl<T: Animation> Animated<T> {
    pub(crate) fn new(content: T, period: NonZeroUsize) -> Self {
        let mut animated = Self {
            content,
            buffer: Buffer::empty(Rect::ZERO),
            period,
            accumulator: 0,
        };
        animated.redraw();
        animated
    }

    /// The cells, unchanged until the next step falls due.
    pub(crate) fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    pub(crate) fn size(&self) -> Size {
        self.buffer.area().as_size()
    }

    pub(crate) fn height(&self) -> u16 {
        self.size().height
    }

    /// Rebuild the cells from the content's current state.
    fn redraw(&mut self) {
        let size = self.content.size();
        self.buffer = Buffer::empty(Rect::new(0, 0, size.width, size.height));
        self.content.paint(&mut self.buffer);
    }
}

impl<T: Animation> EventListener<TickEvent> for Animated<T> {
    fn on_event(&mut self, tick: &TickEvent) {
        self.accumulator += tick.dt;
        while self.accumulator >= self.period.get() {
            self.accumulator -= self.period.get();
            self.content.tick();
        }
        self.redraw();
    }
}

/// Copy a child's local `rows` into `into`, clipped to that region and to `parent`.
///
/// This is the one place a coordinate is translated: a child settles at a zero origin
/// and never learns where it sits, so the shift is applied once, by the owner that
/// chose it. Anything falling outside either the destination or the parent is dropped
/// rather than wrapped, so a viewport can paste a slice of a child larger than itself.
pub(crate) fn paste(child: &Buffer, rows: Range<u16>, into: Rect, parent: &mut Buffer) {
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

/// Placing a settled element is a copy, never a redraw.
///
/// Implemented for a reference because [`Widget::render`] consumes `self`; taking the
/// element by reference keeps it alive across frames, which is the whole point of
/// holding its cells.
impl<T: Animation> Widget for &Animated<T> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        paste(self.buffer(), 0..self.height(), area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One bar filling left to right; supplies only size, paint, and tick.
    struct Filling {
        width: u16,
        filled: u16,
    }

    impl Animation for Filling {
        fn size(&self) -> Size {
            Size::new(self.width, 1)
        }

        fn paint(&self, buffer: &mut Buffer) {
            for x in 0..self.width {
                buffer[(x, 0)].set_char(if x < self.filled { 'X' } else { '.' });
            }
        }

        fn tick(&mut self) {
            self.filled = self.filled.saturating_add(1).min(self.width);
        }
    }

    /// The same three methods with a different action: a cycling spinner.
    struct Spinning {
        frames: Vec<char>,
        at: usize,
    }

    impl Animation for Spinning {
        fn size(&self) -> Size {
            Size::new(1, 1)
        }

        fn paint(&self, buffer: &mut Buffer) {
            buffer[(0, 0)].set_char(self.frames[self.at]);
        }

        fn tick(&mut self) {
            self.at = (self.at + 1) % self.frames.len();
        }
    }

    fn period(milliseconds: usize) -> NonZeroUsize {
        NonZeroUsize::new(milliseconds).expect("test periods are positive")
    }

    fn filling() -> Animated<Filling> {
        Animated::new(
            Filling {
                width: 4,
                filled: 0,
            },
            period(100),
        )
    }

    fn row(animated: &Animated<impl Animation>, width: u16) -> String {
        (0..width)
            .map(|x| animated.buffer()[(x, 0)].symbol())
            .collect()
    }

    #[test]
    fn one_action_and_another_give_two_animations_and_no_other_code() {
        let mut bar = filling();
        let mut spinner = Animated::new(
            Spinning {
                frames: vec!['|', '/', '-'],
                at: 0,
            },
            period(100),
        );

        bar.on_event(&TickEvent { dt: 200 });
        spinner.on_event(&TickEvent { dt: 200 });

        assert_eq!(row(&bar, 4), "XX..");
        assert_eq!(row(&spinner, 1), "-");
    }

    #[test]
    fn elapsed_time_acts_the_same_split_as_whole() {
        let mut split = filling();
        let mut whole = filling();

        for _ in 0..3 {
            split.on_event(&TickEvent { dt: 100 });
        }
        whole.on_event(&TickEvent { dt: 300 });

        assert_eq!(row(&split, 4), row(&whole, 4));
        assert_eq!(row(&whole, 4), "XXX.");
    }

    #[test]
    fn a_remainder_below_the_period_carries_rather_than_vanishing() {
        let mut carried = filling();

        // Three 60ms ticks are 180ms: one whole step with 80ms still owed.
        for _ in 0..3 {
            carried.on_event(&TickEvent { dt: 60 });
        }
        assert_eq!(row(&carried, 4), "X...");

        // The owed 80ms plus 20ms completes the second step.
        carried.on_event(&TickEvent { dt: 20 });
        assert_eq!(row(&carried, 4), "XX..");
    }

    #[test]
    fn time_below_one_period_changes_nothing() {
        let mut idle = filling();

        idle.on_event(&TickEvent { dt: 0 });
        idle.on_event(&TickEvent { dt: 99 });

        assert_eq!(row(&idle, 4), "....");
    }

    fn marked(width: u16, height: u16) -> Buffer {
        let mut cells = Buffer::empty(Rect::new(0, 0, width, height));
        for y in 0..height {
            cells[(0, y)].set_symbol(&y.to_string());
        }
        cells
    }

    #[test]
    fn a_settled_element_renders_as_a_widget_clipped_to_its_area() {
        let mut bar = filling();
        bar.on_event(&TickEvent { dt: 400 });
        let mut screen = Buffer::empty(Rect::new(0, 0, 8, 2));

        // Placed at (1,1) but allowed only 2 columns, so half the bar is withheld.
        (&bar).render(Rect::new(1, 1, 2, 1), &mut screen);

        let placed: String = (0..8).map(|x| screen[(x, 1)].symbol()).collect();
        assert_eq!(placed, " XX     ");
        // The row it was not given stays untouched.
        assert!((0..8).all(|x| screen[(x, 0)].symbol() == " "));
    }

    #[test]
    fn pasting_shifts_a_child_slice_without_wrapping_past_the_parent() {
        let child = marked(2, 6);
        let mut parent = Buffer::empty(Rect::new(0, 0, 4, 3));

        paste(&child, 2..5, Rect::new(1, 0, 2, 3), &mut parent);

        assert_eq!(parent[(1, 0)].symbol(), "2");
        assert_eq!(parent[(1, 2)].symbol(), "4");
        assert_eq!(parent[(0, 0)].symbol(), " ");
    }

    #[test]
    fn pasting_drops_rows_and_columns_beyond_the_parent() {
        let child = marked(6, 4);
        let mut parent = Buffer::empty(Rect::new(0, 0, 4, 2));

        paste(&child, 0..4, Rect::new(2, 1, 6, 4), &mut parent);

        assert_eq!(parent[(2, 1)].symbol(), "0");
        assert_eq!(parent[(2, 0)].symbol(), " ");
    }
}
