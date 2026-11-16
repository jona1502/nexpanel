//! Time sources for the timer tools.
//!
//! Decision: countdown and stopwatch run on a *monotonic* clock that keeps counting during
//! standby/hibernate (`QueryInterruptTime`, 100 ns units since boot, includes sleep; the
//! `Unbiased` variant would exclude it, and `std::time::Instant` is not guaranteed to count
//! sleep on Windows, so it is only the non-Windows fallback). Wall-clock jumps (manual change,
//! NTP, DST/timezone fiddling) therefore never change a running timer. The wall clock is only
//! sampled to *detect and log* such jumps (`DriftDetector`). Timer state is not persisted over
//! restarts, so no wall-clock deadline is stored.

use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

/// Wall/monotonic divergence between two samples above which a jump is reported.
pub const DRIFT_THRESHOLD_MS: i64 = 2_000;

/// Monotonic milliseconds, standby time included. Origin is arbitrary.
#[cfg(windows)]
pub fn mono_ms() -> u64 {
    // SAFETY: plain call without arguments; returns 100 ns units.
    unsafe { windows::Win32::System::WindowsProgramming::QueryInterruptTime() / 10_000 }
}
#[cfg(not(windows))]
pub fn mono_ms() -> u64 {
    use std::{sync::OnceLock, time::Instant};
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Wall clock as epoch milliseconds (can jump).
pub fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

/// Compares wall and monotonic progress between consecutive samples.
#[derive(Debug, Default)]
pub struct DriftDetector {
    last: Cell<Option<(u64, u64)>>,
}
impl DriftDetector {
    /// Returns the wall-clock jump in ms (positive = forward) if wall time advanced more
    /// or less than monotonic time by over `DRIFT_THRESHOLD_MS` since the previous sample.
    pub fn sample(&self, wall: u64, mono: u64) -> Option<i64> {
        let prev = self.last.replace(Some((wall, mono)))?;
        let wall_delta = wall as i64 - prev.0 as i64;
        let mono_delta = mono as i64 - prev.1 as i64;
        let jump = wall_delta - mono_delta;
        (jump.abs() > DRIFT_THRESHOLD_MS).then_some(jump)
    }

    /// Samples the real clocks and logs a detected jump.
    pub fn check(&self) {
        if let Some(jump) = self.sample(wall_ms(), mono_ms()) {
            log::warn!(
                "Systemuhr sprang um {jump} ms; Timer folgt der monotonen Uhr und bleibt unverändert"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time_tools::{Countdown, Stopwatch};

    #[test]
    fn steady_clocks_and_sleep_are_no_drift() {
        let d = DriftDetector::default();
        assert_eq!(d.sample(1_000_000, 500), None, "first sample is baseline");
        assert_eq!(d.sample(1_000_200, 700), None);
        // Standby: both clocks advance by an hour.
        assert_eq!(d.sample(1_000_200 + 3_600_000, 700 + 3_600_000), None);
        // Small NTP slew below threshold.
        assert_eq!(d.sample(4_600_200 + 200 + 1_500, 3_600_700 + 200), None);
    }

    #[test]
    fn forward_and_backward_jumps_are_reported() {
        let d = DriftDetector::default();
        d.sample(1_000_000, 0);
        assert_eq!(d.sample(1_000_200 + 3_600_000, 200), Some(3_600_000));
        assert_eq!(d.sample(1_000_400 + 3_600_000 - 60_000, 400), Some(-60_000));
        assert_eq!(d.sample(1_000_600 + 3_600_000 - 60_000, 600), None);
    }

    #[test]
    fn timers_ignore_wall_clock_jumps() {
        // Timers only ever see the monotonic value; wall jumps cannot reach them.
        let mut c = Countdown::new(10_000).unwrap();
        let mut s = Stopwatch::default();
        let (mono0, wall0) = (5_000u64, 1_700_000_000_000u64);
        c.start(mono0);
        s.start(mono0);
        let d = DriftDetector::default();
        d.sample(wall0, mono0);
        // 4 s later monotonic, wall clock set back one day.
        let (mono1, wall1) = (mono0 + 4_000, wall0 - 86_400_000);
        assert!(d.sample(wall1, mono1).is_some());
        assert_eq!(c.remaining_ms(mono1), 6_000);
        assert_eq!(s.elapsed_ms(mono1), 4_000);
        assert!(!c.poll_expired(mono1));
        // Standby counts: monotonic includes it, expiry fires exactly once on resume.
        let mono2 = mono0 + 3_600_000;
        assert!(c.poll_expired(mono2));
        assert!(!c.poll_expired(mono2 + 1));
    }

    #[test]
    fn real_monotonic_clock_does_not_go_backwards() {
        let a = mono_ms();
        assert!(mono_ms() >= a);
    }
}
