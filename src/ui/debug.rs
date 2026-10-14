//! Runtime rendering diagnostics.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(10);

/// Recent rendered-frame times used by the highlight debug view.
#[derive(Default)]
pub struct FrameStats(VecDeque<Instant>);

impl FrameStats {
    /// Record one rendered application frame.
    pub fn tick(&mut self) {
        let now = Instant::now();
        if self
            .0
            .back()
            .is_some_and(|last| now.duration_since(*last) < Duration::from_millis(1))
        {
            return;
        }
        self.0.push_back(now);
        while self
            .0
            .front()
            .is_some_and(|time| now.duration_since(*time) > WINDOW)
        {
            self.0.pop_front();
        }
    }

    /// Format current, minimum, and average draw rates over the last ten seconds.
    pub fn label(&self) -> String {
        let intervals = self
            .0
            .iter()
            .zip(self.0.iter().skip(1))
            .map(|(left, right)| right.duration_since(*left).as_secs_f64())
            .collect::<Vec<_>>();
        let current = intervals.last().map_or(0.0, |seconds| 1.0 / seconds);
        let min = intervals
            .iter()
            .map(|seconds| 1.0 / seconds)
            .reduce(f64::min)
            .unwrap_or(0.0);
        let elapsed = self
            .0
            .front()
            .zip(self.0.back())
            .map_or(0.0, |(first, last)| {
                last.duration_since(*first).as_secs_f64()
            });
        let average = if elapsed > 0.0 {
            intervals.len() as f64 / elapsed
        } else {
            0.0
        };
        Self::format(current, min, average)
    }

    fn format(current: f64, min: f64, average: f64) -> String {
        let [current, min, average] = [current, min, average].map(|fps| fps.clamp(0.0, 9999.9));
        format!("FPS {current:6.1} current · {min:6.1} min · {average:6.1} avg / 10s")
    }
}

#[cfg(test)]
mod tests {
    use super::FrameStats;

    #[test]
    fn label_width_does_not_follow_fps_digits() {
        assert_eq!(
            FrameStats::format(0.0, 9.9, 99.9).len(),
            FrameStats::format(999.9, 9999.9, 99_999.0).len()
        );
    }
}
