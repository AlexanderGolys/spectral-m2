//! Runtime rendering diagnostics.

use crate::ui::config::UI;
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

    /// The mean draw rate over the last ten seconds, in frames per second.
    pub fn average(&self) -> f64 {
        let elapsed = self
            .0
            .front()
            .zip(self.0.back())
            .map_or(0.0, |(first, last)| {
                last.duration_since(*first).as_secs_f64()
            });
        match elapsed > 0.0 {
            true => self.0.len().saturating_sub(1) as f64 / elapsed,
            false => 0.0,
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
        Self::format(current, min, self.average())
    }

    fn format(current: f64, min: f64, average: f64) -> String {
        let [current, min, average] = [current, min, average].map(|fps| fps.clamp(0.0, 9999.9));
        let separator = UI.glyphs.separator.current();
        format!(
            "FPS {current:6.1} current {separator} {min:6.1} min {separator} {average:6.1} avg / 10s"
        )
    }
}

/// The memory a process holds in RAM, in bytes, where the platform reports it:
/// this process when `process` is none. Read from `/proc`, so only on Linux.
pub fn resident_bytes(process: Option<u32>) -> Option<u64> {
    let path = match process {
        Some(id) => format!("/proc/{id}/status"),
        None => "/proc/self/status".to_string(),
    };
    let status = std::fs::read_to_string(path).ok()?;
    let kibibytes = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .trim()
        .strip_suffix("kB")?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(kibibytes * 1024)
}

/// Bytes as mebibytes to one decimal place.
pub fn mebibytes(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}

#[cfg(test)]
mod tests {
    use super::{mebibytes, resident_bytes, FrameStats};

    #[test]
    fn mebibytes_show_one_decimal() {
        assert_eq!(mebibytes(3 * 1024 * 1024 + 512 * 1024), "3.5 MiB");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_process_holds_some_memory_and_a_missing_one_none() {
        assert!(resident_bytes(None).is_some_and(|bytes| bytes > 0));
        assert_eq!(resident_bytes(Some(u32::MAX)), None);
    }

    #[test]
    fn the_average_rate_counts_frames_between_the_first_and_last() {
        let mut stats = FrameStats::default();
        assert_eq!(stats.average(), 0.0);
        stats.tick();
        assert_eq!(stats.average(), 0.0);
    }

    #[test]
    fn label_width_does_not_follow_fps_digits() {
        assert_eq!(
            FrameStats::format(0.0, 9.9, 99.9).len(),
            FrameStats::format(999.9, 9999.9, 99_999.0).len()
        );
    }
}
