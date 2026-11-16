//! Microwave-style HH:MM:SS entry for the countdown (pure, unit-tested).
//! The state is the six displayed digits as one decimal number (`HHMMSS`); digits shift in from
//! the right. Segments may temporarily exceed 59 (e.g. `00:00:90`) until `normalize` carries them.

/// Longest selectable countdown (24 h), matching `Countdown::new`.
pub const MAX_SECONDS: u32 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TimeInput(u32);

/// Segment index used by `step`: 0 = hours, 1 = minutes, 2 = seconds.
pub const UNITS: [i64; 3] = [3600, 60, 1];

impl TimeInput {
    pub fn from_seconds(seconds: u32) -> Self {
        let s = seconds.min(MAX_SECONDS);
        Self(s / 3600 * 10_000 + s / 60 % 60 * 100 + s % 60)
    }
    /// Shifts a digit in from the right; the oldest digit falls off after six.
    pub fn push_digit(self, digit: u32) -> Self {
        if digit > 9 {
            return self;
        }
        Self((self.0 * 10 + digit) % 1_000_000)
    }
    pub fn backspace(self) -> Self {
        Self(self.0 / 10)
    }
    pub fn clear(self) -> Self {
        Self(0)
    }
    /// Total seconds with carry (`00:00:90` is 90 s), clamped to 24 h.
    pub fn to_seconds(self) -> u32 {
        let (h, m, s) = self.segments();
        (h * 3600 + m * 60 + s).min(MAX_SECONDS)
    }
    pub fn normalize(self) -> Self {
        Self::from_seconds(self.to_seconds())
    }
    /// Adds `delta` units of one segment (arrows, mouse wheel); carries and clamps to 0..=24 h.
    pub fn step(self, segment: usize, delta: i32) -> Self {
        let unit = UNITS.get(segment).copied().unwrap_or(1);
        let total = (i64::from(self.to_seconds()) + unit * i64::from(delta))
            .clamp(0, i64::from(MAX_SECONDS));
        Self::from_seconds(total as u32)
    }
    pub fn segments(self) -> (u32, u32, u32) {
        (self.0 / 10_000, self.0 / 100 % 100, self.0 % 100)
    }
    pub fn text(self) -> String {
        let (h, m, s) = self.segments();
        format!("{h:02}:{m:02}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(digits: &str) -> TimeInput {
        digits.chars().fold(TimeInput::default(), |t, c| {
            t.push_digit(c.to_digit(10).expect("digit"))
        })
    }

    #[test]
    fn digits_fill_from_the_right() {
        for (keys, text) in [
            ("", "00:00:00"),
            ("1", "00:00:01"),
            ("15", "00:00:15"),
            ("153", "00:01:53"),
            ("1530", "00:15:30"),
            ("123456", "12:34:56"),
            ("1234567", "23:45:67"),
        ] {
            assert_eq!(typed(keys).text(), text, "keys {keys}");
        }
    }

    #[test]
    fn backspace_and_clear() {
        assert_eq!(typed("153").backspace().text(), "00:00:15");
        assert_eq!(TimeInput::default().backspace().text(), "00:00:00");
        assert_eq!(typed("153").clear().text(), "00:00:00");
    }

    #[test]
    fn seconds_roundtrip_and_clamp() {
        for (seconds, text) in [
            (0, "00:00:00"),
            (59, "00:00:59"),
            (60, "00:01:00"),
            (300, "00:05:00"),
            (3661, "01:01:01"),
            (86_400, "24:00:00"),
            (999_999, "24:00:00"),
        ] {
            assert_eq!(TimeInput::from_seconds(seconds).text(), text);
        }
        assert_eq!(TimeInput::from_seconds(3661).to_seconds(), 3661);
    }

    #[test]
    fn normalize_carries_overflowing_segments() {
        for (keys, seconds, text) in [
            ("90", 90, "00:01:30"),
            ("99", 99, "00:01:39"),
            ("9999", 99 * 60 + 99, "01:40:39"),
            ("995959", 86_400, "24:00:00"),
        ] {
            let t = typed(keys);
            assert_eq!(t.to_seconds(), seconds, "keys {keys}");
            assert_eq!(t.normalize().text(), text, "keys {keys}");
        }
    }

    #[test]
    fn step_carries_and_clamps() {
        let t = TimeInput::from_seconds(59);
        assert_eq!(t.step(2, 1).text(), "00:01:00");
        assert_eq!(t.step(1, 1).text(), "00:01:59");
        assert_eq!(t.step(0, 1).text(), "01:00:59");
        assert_eq!(TimeInput::default().step(2, -1).text(), "00:00:00");
        assert_eq!(
            TimeInput::from_seconds(86_399).step(2, 5).text(),
            "24:00:00"
        );
        assert_eq!(t.step(9, 1).text(), "00:01:00");
    }

    #[test]
    fn non_digits_are_ignored() {
        assert_eq!(TimeInput::default().push_digit(10), TimeInput::default());
    }
}
