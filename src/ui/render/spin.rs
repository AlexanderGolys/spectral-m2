//! The spinner of a cell that is still evaluating.
//!
//! The frame is a function of the clock and nothing else: there is no accumulator and no
//! remainder to carry, so however irregularly the event loop redraws, a redraw only
//! decides *when* the spinner is asked, never *what* it answers, and it cannot drift.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

/// The first frame's instant, from which every later frame is counted.
///
/// Deliberately not wall-clock: `SystemTime` moves when NTP or a timezone says so, and
/// an animation reading it would stall or leap when the clock is corrected.
static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

/// A looping animation of glyphs, each held for the same span.
///
/// Spinners are stateless: every spinner shares one epoch, so two of them with the same
/// frames turn in step, and asking twice within a span answers the same frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Spinner {
    frames: &'static [&'static str],
    span: Duration,
}

impl Spinner {
    /// Needs at least one frame and a nonzero span.
    pub const fn new(frames: &'static [&'static str], span: Duration) -> Self {
        assert!(!frames.is_empty(), "a spinner needs at least one frame");
        assert!(!span.is_zero(), "a spinner frame must last some time");
        Self { frames, span }
    }

    pub fn frames(self) -> &'static [&'static str] {
        self.frames
    }

    pub fn span(self) -> Duration {
        self.span
    }

    pub fn current_frame(self) -> &'static str {
        self.frame_at(EPOCH.elapsed())
    }

    /// The frame shown `elapsed` after the first, the frames repeating in order.
    pub fn frame_at(self, elapsed: Duration) -> &'static str {
        let turns = elapsed.as_nanos() / self.span.as_nanos();
        let frame_count = self.frames.len() as u128;
        self.frames[(turns % frame_count) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPAN: Duration = Duration::from_millis(10);
    const SPINNER: Spinner = Spinner::new(&["a", "b", "c"], SPAN);

    #[test]
    fn the_frame_advances_with_the_clock_and_wraps() {
        let frames = SPINNER.frames();
        let at = |turn: u32| SPINNER.frame_at(SPAN * turn);
        assert_eq!(at(0), frames[0]);
        assert_eq!(at(1), frames[1]);
        assert_eq!(at(frames.len() as u32), frames[0]);
        assert_eq!(at(frames.len() as u32 + 1), frames[1]);
    }

    #[test]
    fn a_frame_holds_for_its_whole_span() {
        let just_before = SPAN - Duration::from_nanos(1);
        assert_eq!(
            SPINNER.frame_at(Duration::ZERO),
            SPINNER.frame_at(just_before)
        );
        assert_ne!(SPINNER.frame_at(just_before), SPINNER.frame_at(SPAN));
    }
}
