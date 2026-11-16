/// Epoch milliseconds are supplied by the caller, so suspended time counts.
#[derive(Debug, Default, Clone)]
pub struct Stopwatch {
    elapsed: u64,
    started: Option<u64>,
}
impl Stopwatch {
    pub fn start(&mut self, now: u64) {
        if self.started.is_none() {
            self.started = Some(now);
        }
    }
    pub fn pause(&mut self, now: u64) {
        self.elapsed = self.elapsed_ms(now);
        self.started = None;
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn elapsed_ms(&self, now: u64) -> u64 {
        self.elapsed
            .saturating_add(self.started.map_or(0, |start| now.saturating_sub(start)))
    }
    pub fn is_running(&self) -> bool {
        self.started.is_some()
    }
}
#[derive(Debug, Clone)]
pub struct Countdown {
    duration: u64,
    clock: Stopwatch,
    expired: bool,
}
impl Countdown {
    pub fn new(duration_ms: u64) -> Result<Self, &'static str> {
        if !(1_000..=86_400_000).contains(&duration_ms) {
            return Err("Dauer muss zwischen 1 Sekunde und 24 Stunden liegen");
        }
        Ok(Self {
            duration: duration_ms,
            clock: Stopwatch::default(),
            expired: false,
        })
    }
    pub fn start(&mut self, now: u64) {
        if !self.expired {
            self.clock.start(now);
        }
    }
    pub fn pause(&mut self, now: u64) {
        self.clock.pause(now);
    }
    pub fn reset(&mut self) {
        self.clock.reset();
        self.expired = false;
    }
    pub fn remaining_ms(&self, now: u64) -> u64 {
        self.duration.saturating_sub(self.clock.elapsed_ms(now))
    }
    pub fn is_running(&self) -> bool {
        self.clock.is_running()
    }
    /// True once started (running, paused part-way or expired) until the next reset.
    pub fn is_started(&self, now: u64) -> bool {
        self.clock.elapsed_ms(now) > 0 || self.clock.is_running()
    }
    pub fn poll_expired(&mut self, now: u64) -> bool {
        if self.expired || self.remaining_ms(now) != 0 {
            return false;
        }
        self.expired = true;
        self.clock.pause(now);
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn countdown_rejects_outside_one_second_to_one_day() {
        assert!(Countdown::new(0).is_err());
        assert!(Countdown::new(999).is_err());
        assert!(Countdown::new(1_000).is_ok());
        assert!(Countdown::new(86_400_000).is_ok());
        assert!(Countdown::new(86_400_001).is_err());
    }
    #[test]
    fn countdown_pause_resume_excludes_paused_time() {
        let mut timer = Countdown::new(10_000).unwrap();
        timer.start(100);
        timer.pause(2_100);
        assert_eq!(timer.remaining_ms(50_000), 8_000);
        timer.start(50_000);
        assert_eq!(timer.remaining_ms(51_000), 7_000);
        timer.reset();
        assert_eq!(timer.remaining_ms(90_000), 10_000);
        assert!(!timer.is_running());
    }
    #[test]
    fn countdown_sleep_counts_and_expiry_is_once_per_run() {
        let mut timer = Countdown::new(1_000).unwrap();
        timer.start(1_000);
        assert!(timer.poll_expired(3_601_000));
        assert!(!timer.poll_expired(3_602_000));
        timer.reset();
        timer.start(4_000_000);
        assert!(timer.poll_expired(4_001_000));
    }
    #[test]
    fn stopwatch_pause_resume_reset_and_sleep() {
        let mut timer = Stopwatch::default();
        timer.start(100);
        timer.start(200);
        assert_eq!(timer.elapsed_ms(3_600_100), 3_600_000);
        timer.pause(3_600_100);
        assert_eq!(timer.elapsed_ms(9_000_000), 3_600_000);
        timer.start(9_000_000);
        assert_eq!(timer.elapsed_ms(9_001_000), 3_601_000);
        timer.reset();
        assert_eq!(timer.elapsed_ms(10_000_000), 0);
        assert!(!timer.is_running());
    }
}
