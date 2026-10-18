//! Single-cell spinners, in the two flavours time comes in.
//!
//! One evaluates: given the clock, it computes what to show and remembers nothing, so
//! it cannot drift however irregularly the ticks arrive. The other integrates: it walks
//! a queue of frames, each holding the screen for a stated span, which is what you want
//! when the frames are a script rather than a function.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;

use crate::ui::config::UI;
use crate::ui::element::EventListener;

type Ms = u64;

/// Process start, so a tick's `t` is monotonic.
///
/// Deliberately not wall-clock: `SystemTime` moves when NTP or a timezone says so, and
/// an animation reading it would stall or leap when the clock is corrected.
static START: LazyLock<Instant> = LazyLock::new(Instant::now);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub struct Tick {
    t: Ms,
    dt: Ms,
}

impl Tick {
    pub fn new(dt: Ms) -> Self {
        Self {
            t: Ms::try_from(START.elapsed().as_millis()).unwrap_or(Ms::MAX),
            dt,
        }
    }

    /// Build a tick at an explicit instant, for tests and for replaying a recording.
    pub fn at(t: Ms, dt: Ms) -> Self {
        Self { t, dt }
    }

    pub const fn t(self) -> Ms {
        self.t
    }

    pub const fn dt(self) -> Ms {
        self.dt
    }
}

/// A steady grid of tick instants, held in phase however long the work between them takes.
///
/// Sleeping for the interval *after* doing the work drifts by the work's duration every
/// pass. Advancing a deadline instead keeps beats on the original grid, and when a pass
/// overruns, the deadline jumps by whole intervals — dropping the beats that were
/// missed rather than sliding the phase to wherever lateness left it.
pub struct Beat {
    interval: Duration,
    origin: Instant,
    next: Instant,
    last: Instant,
}

impl Beat {
    pub fn new(interval: Duration) -> Self {
        assert!(
            !interval.is_zero(),
            "a beat needs a positive interval; zero is infinite frequency"
        );
        let now = Instant::now();
        Self {
            interval,
            origin: now,
            next: now + interval,
            last: now,
        }
    }

    /// How long remains before the next beat falls due; the poll timeout to use.
    pub fn wait(&self) -> Duration {
        self.wait_at(Instant::now())
    }

    /// Take the beat if it is due, reporting the time actually elapsed since the last.
    pub fn strike(&mut self) -> Option<Tick> {
        self.strike_at(Instant::now())
    }

    fn wait_at(&self, now: Instant) -> Duration {
        self.next.saturating_duration_since(now)
    }

    fn strike_at(&mut self, now: Instant) -> Option<Tick> {
        if now < self.next {
            return None;
        }
        let dt = now.duration_since(self.last);
        self.last = now;
        // Land on the first grid instant after `now`, counting only whole intervals.
        let late = now.duration_since(self.next).as_nanos();
        let skipped = late / self.interval.as_nanos() + 1;
        self.next += self.interval * u32::try_from(skipped).unwrap_or(u32::MAX);
        Some(Tick::at(
            millis(now.duration_since(self.origin)),
            millis(dt),
        ))
    }
}

fn millis(span: Duration) -> Ms {
    Ms::try_from(span.as_millis()).unwrap_or(Ms::MAX)
}

/// The frame of the marker of a cell still evaluating at `tick`.
pub fn busy_frame(tick: &Tick) -> &'static str {
    let frames = UI.spinner.active_frames();
    let index = tick.t() / UI.spinner.frame_ms;
    frames[usize::try_from(index).unwrap_or_default() % frames.len()]
}

pub trait SpinningRod<Output = Cell> {
    fn call(&self, tick: &Tick) -> Output;
}

/// Remembers nothing but always on time.
///
/// The cell is a function of the clock, so an irregular event loop changes when the
/// spinner is asked, never what it answers. No accumulator, no remainder, no drift.
pub struct ExpSpinner<R: SpinningRod> {
    fish: Buffer,
    rod: R,
}

impl<R: SpinningRod> ExpSpinner<R> {
    pub fn new(rod: R) -> Self {
        Self {
            fish: Buffer::empty(Rect::new(0, 0, 1, 1)),
            rod,
        }
    }

    pub fn buffer(&self) -> &Buffer {
        &self.fish
    }
}

impl<R: SpinningRod> EventListener<Tick> for ExpSpinner<R> {
    fn on_event(&mut self, tick: &Tick) {
        let stock_fish = self.rod.call(tick);
        self.fish[(0, 0)] = stock_fish;
    }
}

/// Walks a scripted queue of frames, each holding the screen for its own span.
///
/// A coroutine, not a clock: each frame hands back how long to wait, every tick counts
/// that down, and reaching zero draws the next one. At most one frame per tick, so a
/// stall in the event loop *delays* the animation rather than skipping through it —
/// every frame is shown, and the rhythm is what gives way. That is the whole difference
/// from [`ExpSpinner`], which keeps the rhythm and drops frames instead.
pub struct BufferedSpinner<R: Iterator<Item = (Cell, Ms)>> {
    fish: Buffer,
    queue: R,
    buffered: Option<Ms>,
}

impl<R: Iterator<Item = (Cell, Ms)>> BufferedSpinner<R> {
    pub fn new(queue: R) -> Self {
        Self {
            fish: Buffer::empty(Rect::new(0, 0, 1, 1)),
            queue,
            buffered: Some(0),
        }
    }

    pub fn buffer(&self) -> &Buffer {
        &self.fish
    }

    /// Whether any frame remains to be shown.
    pub fn spinning(&self) -> bool {
        self.buffered.is_some()
    }

    pub fn next(&mut self) {
        match self.queue.next() {
            Some((stock_fish, dt)) => {
                self.buffered = Some(dt);
                self.fish[(0, 0)] = stock_fish;
            }
            None => self.buffered = None,
        }
    }
}

impl<R: Iterator<Item = (Cell, Ms)>> EventListener<Tick> for BufferedSpinner<R> {
    fn on_event(&mut self, tick: &Tick) {
        if let Some(held) = self.buffered {
            if tick.dt >= held {
                self.next();
            } else {
                self.buffered = Some(held - tick.dt);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAMES: [char; 4] = ['|', '/', '-', '\\'];

    #[test]
    fn the_busy_marker_advances_with_the_clock_and_wraps() {
        let frames = UI.spinner.active_frames();
        let at = |frame: usize| busy_frame(&Tick::at(frame as Ms * UI.spinner.frame_ms, 0));
        assert_ne!(at(0), at(1));
        assert_eq!(at(0), frames[0]);
        assert_eq!(at(frames.len()), frames[0]);
        assert_eq!(at(frames.len() + 1), frames[1]);
    }

    /// Reads the clock and answers; keeps nothing between calls.
    struct Clockwise {
        period: Ms,
    }

    impl SpinningRod for Clockwise {
        fn call(&self, tick: &Tick) -> Cell {
            let mut cell = Cell::EMPTY;
            cell.set_char(FRAMES[(tick.t() / self.period) as usize % FRAMES.len()]);
            cell
        }
    }

    fn frame(buffer: &Buffer) -> char {
        buffer[(0, 0)].symbol().chars().next().unwrap_or(' ')
    }

    fn scripted() -> BufferedSpinner<impl Iterator<Item = (Cell, Ms)>> {
        BufferedSpinner::new(FRAMES.into_iter().map(|glyph| {
            let mut cell = Cell::EMPTY;
            cell.set_char(glyph);
            (cell, 100)
        }))
    }

    /// A regular beat, one stall of five beats' length, then regular again.
    const STALLED: [(Ms, Ms); 4] = [(100, 100), (200, 100), (700, 500), (800, 100)];

    #[test]
    fn evaluating_the_clock_cannot_drift_however_the_ticks_arrive() {
        let mut steady = ExpSpinner::new(Clockwise { period: 100 });
        let mut stuttering = ExpSpinner::new(Clockwise { period: 100 });

        // Same destination on the clock, wildly different delivery.
        for step in 1..=7 {
            steady.on_event(&Tick::at(step * 100, 100));
        }
        stuttering.on_event(&Tick::at(50, 50));
        stuttering.on_event(&Tick::at(700, 650));

        assert_eq!(frame(steady.buffer()), frame(stuttering.buffer()));
        assert_eq!(frame(steady.buffer()), '\\');
    }

    /// Through the same stall, one keeps the rhythm and the other keeps the song.
    #[test]
    fn a_stall_costs_the_clock_its_frames_and_the_queue_its_timing() {
        let mut clockwise = ExpSpinner::new(Clockwise { period: 100 });
        let mut queued = scripted();
        let (mut heard_clockwise, mut heard_queued) = (Vec::new(), Vec::new());

        for (t, dt) in STALLED {
            let tick = Tick::at(t, dt);
            clockwise.on_event(&tick);
            queued.on_event(&tick);
            heard_clockwise.push(frame(clockwise.buffer()));
            heard_queued.push(frame(queued.buffer()));
        }

        // Skipped '|' and '-' outright, but every frame matches its own instant.
        assert_eq!(heard_clockwise, ['/', '-', '\\', '|']);
        for (t, _) in STALLED {
            let due = FRAMES[(t / 100) as usize % FRAMES.len()];
            assert!(heard_clockwise.contains(&due), "{due} was never on screen");
        }

        // Dropped nothing: the whole script, in order, arriving one tick late.
        assert_eq!(heard_queued, ['|', '/', '-', '\\']);
    }

    #[test]
    fn a_stalled_queue_waits_rather_than_skipping_ahead() {
        let mut spinner = scripted();

        // Five frames' worth of time in one tick still advances exactly one frame.
        spinner.on_event(&Tick::at(500, 500));

        assert_eq!(frame(spinner.buffer()), '|');
        assert!(spinner.spinning());
    }

    #[test]
    fn a_scripted_queue_stops_when_its_frames_run_out() {
        let mut spinner = scripted();

        for step in 1..=5 {
            spinner.on_event(&Tick::at(step * 100, 100));
        }

        assert!(!spinner.spinning());
        assert_eq!(frame(spinner.buffer()), '\\');
    }

    #[test]
    fn time_shorter_than_the_held_frame_changes_nothing() {
        let mut spinner = scripted();
        spinner.on_event(&Tick::at(100, 100));

        spinner.on_event(&Tick::at(199, 99));

        assert_eq!(frame(spinner.buffer()), '|');
        assert!(spinner.spinning());
    }

    /// A frame too short for the tick interval is still guaranteed the screen.
    ///
    /// Sampling a clock can only see what a tick lands on, so a one-millisecond accent
    /// between ordinary frames is invisible to [`ExpSpinner`] roughly fifteen times in
    /// sixteen. Counting down a queue cannot miss it: every frame is drawn when its
    /// predecessor's span runs out, so the shortest one still gets a whole tick.
    #[test]
    fn a_frame_shorter_than_a_tick_still_gets_the_screen() {
        let script = [('|', 100), ('!', 1), ('-', 100)];
        let mut spinner = BufferedSpinner::new(script.into_iter().map(|(glyph, span)| {
            let mut cell = Cell::EMPTY;
            cell.set_char(glyph);
            (cell, span as Ms)
        }));
        let mut heard = Vec::new();

        // A 16ms event loop: the accent is a sixteenth of one interval wide.
        for step in 1..=12 {
            spinner.on_event(&Tick::at(step * 16, 16));
            heard.push(frame(spinner.buffer()));
        }

        assert!(heard.contains(&'!'), "the accent was skipped: {heard:?}");
        assert_eq!(heard.iter().filter(|glyph| **glyph == '!').count(), 1);
    }

    #[test]
    fn a_beat_waits_out_its_interval_before_striking() {
        let mut beat = Beat::new(Duration::from_millis(50));
        let start = beat.next - Duration::from_millis(50);

        assert!(beat.strike_at(start + Duration::from_millis(49)).is_none());
        assert_eq!(
            beat.wait_at(start + Duration::from_millis(20)),
            Duration::from_millis(30)
        );
        assert!(beat.strike_at(start + Duration::from_millis(50)).is_some());
    }

    /// An overrun drops the beats it missed; it never slides the grid.
    #[test]
    fn an_overrun_skips_whole_beats_and_keeps_the_phase() {
        let mut beat = Beat::new(Duration::from_millis(50));
        let start = beat.next - Duration::from_millis(50);

        // Two and a half intervals late: three beats were due, one is delivered.
        let tick = beat
            .strike_at(start + Duration::from_millis(175))
            .expect("a beat was due");

        assert_eq!(tick.dt(), 175, "the delta is measured, not nominal");
        // Next beat sits on the original grid at 200ms, not at 175 + 50.
        assert_eq!(beat.next.duration_since(start), Duration::from_millis(200));
    }

    #[test]
    fn successive_beats_stay_on_the_grid_however_late_each_arrives() {
        let mut beat = Beat::new(Duration::from_millis(50));
        let start = beat.next - Duration::from_millis(50);

        for (arrival, grid) in [(52, 100), (137, 150), (151, 200)] {
            beat.strike_at(start + Duration::from_millis(arrival))
                .expect("a beat was due");
            assert_eq!(
                beat.next.duration_since(start),
                Duration::from_millis(grid),
                "arriving at {arrival}ms should leave the grid at {grid}ms"
            );
        }
    }
}
