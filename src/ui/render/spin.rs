//! The spinner of a cell that is still evaluating.
//!
//! The frame is a function of the clock and nothing else: there is no accumulator and no
//! remainder to carry, so however irregularly the event loop redraws, a redraw only
//! decides *when* the spinner is asked, never *what* it answers, and it cannot drift.

use std::sync::LazyLock;
use std::time::Instant;

use crate::ui::config::UI;

type Milliseconds = u64;

/// The first frame's instant, from which every later frame is counted.
///
/// Deliberately not wall-clock: `SystemTime` moves when NTP or a timezone says so, and
/// an animation reading it would stall or leap when the clock is corrected.
static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

/// The frame of the marker of a cell still evaluating, now.
pub fn busy_frame() -> &'static str {
    frame_at(Milliseconds::try_from(EPOCH.elapsed().as_millis()).unwrap_or(Milliseconds::MAX))
}

/// The frame shown `elapsed` milliseconds after the first, the frames repeating in order.
fn frame_at(elapsed: Milliseconds) -> &'static str {
    let frames = UI.spinner.active_frames();
    let index = elapsed / UI.spinner.frame_ms;
    frames[usize::try_from(index).unwrap_or_default() % frames.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_busy_marker_advances_with_the_clock_and_wraps() {
        let frames = UI.spinner.active_frames();
        let at = |frame: usize| frame_at(frame as Milliseconds * UI.spinner.frame_ms);
        assert_ne!(at(0), at(1));
        assert_eq!(at(0), frames[0]);
        assert_eq!(at(frames.len()), frames[0]);
        assert_eq!(at(frames.len() + 1), frames[1]);
    }

    #[test]
    fn a_frame_holds_for_its_whole_span() {
        let span = UI.spinner.frame_ms;
        assert_eq!(frame_at(0), frame_at(span - 1));
        assert_ne!(frame_at(span - 1), frame_at(span));
    }
}
