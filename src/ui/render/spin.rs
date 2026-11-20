//! The spinner of a cell that is still evaluating.
//!
//! The frame is a function of the clock and nothing else: there is no accumulator and no
//! remainder to carry, so however irregularly the event loop redraws, a redraw only
//! decides *when* the spinner is asked, never *what* it answers, and it cannot drift.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::thread::{self, Thread};
use std::time::{Duration, Instant};

use crate::ui::block::HLSpan;
use crate::ui::config::UI;
use crate::ui::glyphs::{Spelled, Spellings};

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

    /// The frame shown `elapsed` after the first, the frames repeating in order.
    pub fn frame_at(self, elapsed: Duration) -> &'static SpinnerFrame {
        self.frame_of(Self::turns_at(self, elapsed))
    }

    fn frame_of(self, turns: u64) -> &'static SpinnerFrame {
        &self.frames[(turns % self.frames.len() as u64) as usize]
    }

    /// Frames shown `elapsed` after the first.
    fn turns_at(self, elapsed: Duration) -> u64 {
        u64::try_from(elapsed.as_nanos() / self.frame_duration.as_nanos())
            .expect("a spinner turns fewer than 2^64 times")
    }

    /// How long the current frame still shows.
    fn until_next_frame(self) -> Duration {
        let into = EPOCH.elapsed().as_nanos() % self.frame_duration.as_nanos();
        self.frame_duration - Duration::from_nanos(u64::try_from(into).expect("within one frame"))
    }
}

/// The pending cell spinner's clock, shared by every cell that spins.
pub fn clock() -> &'static SpinnerClock {
    static CLOCK: LazyLock<SpinnerClock> =
        LazyLock::new(|| SpinnerClock::new(UI::PENDING_CELL_SPINNER));
    &CLOCK
}

/// A spinner's own clock, spelled for the charset at each frame: while running, a thread of its own turns the frame as
/// each one's time is up. It only moves the spinner's state; drawing reads that
/// state at the drawing's own rate, so any number of spinners never makes more
/// frames than the loop does.
///
/// ```ignore
/// clock().run(true); // a cell started evaluating
/// let frame = clock().current_frame(); // drawn whenever the loop draws
/// clock().run(false); // the last one finished; the clock's thread parks
/// ```
#[derive(Debug)]
pub struct SpinnerClock {
    spinner: Spellings<Spinner>,
    turns: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
    thread: Thread,
}

impl SpinnerClock {
    pub fn new(spinner: Spellings<Spinner>) -> Self {
        let turns = Arc::new(AtomicU64::new(spinner.current().turns_at(EPOCH.elapsed())));
        let running = Arc::new(AtomicBool::new(false));
        let (turning, ticking) = (turns.clone(), running.clone());
        let thread = thread::spawn(move || loop {
            if !ticking.load(Ordering::Acquire) {
                thread::park();
                continue;
            }
            let spinner = spinner.current();
            thread::sleep(spinner.until_next_frame());
            turning.store(spinner.turns_at(EPOCH.elapsed()), Ordering::Release);
        })
        .thread()
        .clone();
        Self {
            spinner,
            turns,
            running,
            thread,
        }
    }

    /// Frames turned so far; the picture changes exactly when this does.
    pub fn turns(&self) -> u64 {
        self.turns.load(Ordering::Acquire)
    }

    pub fn current_frame(&self) -> &'static SpinnerFrame {
        self.spinner.current().frame_of(self.turns())
    }

    pub fn run(&self, running: bool) {
        self.running.store(running, Ordering::Release);
        if running {
            self.thread.unpark();
        }
    }
}
