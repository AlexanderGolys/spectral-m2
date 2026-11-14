//! The spinner of a cell that is still evaluating.
//!
//! The frame is a function of the clock and nothing else: there is no accumulator and no
//! remainder to carry, so however irregularly the event loop redraws, a redraw only
//! decides *when* the spinner is asked, never *what* it answers, and it cannot drift.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use crate::ui::block::HLSpan;

/// One spinner frame: text whose style comes from the active scheme's group.
pub type SpinnerFrame = HLSpan<&'static str>;

/// The first frame's instant, from which every later frame is counted.
///
/// Deliberately not wall-clock: `SystemTime` moves when NTP or a timezone says so, and
/// an animation reading it would stall or leap when the clock is corrected.
static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Frames sharing one highlight group, as a `'static` slice for [`Spinner::new`]:
/// `spinner_frames![PendingCellMarker: "<", "^", ">", "v"]`.
#[macro_export]
macro_rules! spinner_frames {
    ($group:ident: $($frame:expr),+ $(,)?) => {
        &[$($crate::ui::block::HLSpan {
            content: $frame,
            kind: $crate::ui::theme::groups::HLGroup::$group,
            mark: None,
            target: None,
        }),+]
    };
}

/// A looping animation of styled frames, each held for the same duration.
///
/// Spinners are stateless: every spinner shares one epoch, so two of them with the same
/// frames turn in step, and asking twice within one frame answers the same frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spinner {
    frames: &'static [SpinnerFrame],
    frame_duration: Duration,
}

impl Spinner {
    /// Needs at least one frame and a nonzero frame duration.
    pub const fn new(frames: &'static [SpinnerFrame], frame_duration: Duration) -> Self {
        assert!(!frames.is_empty(), "a spinner needs at least one frame");
        assert!(
            !frame_duration.is_zero(),
            "a spinner frame must last some time"
        );
        Self {
            frames,
            frame_duration,
        }
    }

    pub fn frames(self) -> &'static [SpinnerFrame] {
        self.frames
    }

    pub fn frame_duration(self) -> Duration {
        self.frame_duration
    }

    pub fn current_frame(self) -> &'static SpinnerFrame {
        self.frame_at(EPOCH.elapsed())
    }

    /// The frame shown `elapsed` after the first, the frames repeating in order.
    pub fn frame_at(self, elapsed: Duration) -> &'static SpinnerFrame {
        let turns = elapsed.as_nanos() / self.frame_duration.as_nanos();
        let frame_count = self.frames.len() as u128;
        &self.frames[(turns % frame_count) as usize]
    }
}

