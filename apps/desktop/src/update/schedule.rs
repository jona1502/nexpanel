//! When the automatic update check is due. Pure: the caller supplies the clock.

use std::time::{SystemTime, UNIX_EPOCH};

/// At most one automatic check per 24 hours (the timestamp is stored in the settings file).
pub const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `last_check` is the stored Unix timestamp of the last attempt (0 = never). A timestamp in
/// the future (clock set back) counts as due so a wrong clock cannot suppress checks forever;
/// the attempt then stores the corrected time.
pub fn auto_check_due(enabled: bool, last_check: u64, now: u64) -> bool {
    enabled && (last_check == 0 || last_check > now || now - last_check >= CHECK_INTERVAL_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = CHECK_INTERVAL_SECS;

    #[test]
    fn never_checked_is_due_when_enabled() {
        assert!(auto_check_due(true, 0, 1_800_000_000));
        assert!(!auto_check_due(false, 0, 1_800_000_000));
    }

    #[test]
    fn at_most_once_per_24_hours() {
        let last = 1_800_000_000;
        assert!(!auto_check_due(true, last, last));
        assert!(!auto_check_due(true, last, last + DAY - 1));
        assert!(auto_check_due(true, last, last + DAY));
        assert!(auto_check_due(true, last, last + 10 * DAY));
        assert!(!auto_check_due(false, last, last + 10 * DAY));
    }

    #[test]
    fn timestamp_in_the_future_is_due() {
        assert!(auto_check_due(true, 2_000_000_000, 1_800_000_000));
    }

    #[test]
    fn clock_is_sane() {
        assert!(unix_now() > 1_700_000_000);
    }
}
