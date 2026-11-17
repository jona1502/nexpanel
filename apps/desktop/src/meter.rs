//! Level meter math for the audio tab (pure, unit-tested): peak -> level, smoothing, colors.
use std::{collections::HashMap, time::Duration};

/// Quietest peak that still shows (dBFS); everything below is an empty bar.
pub const FLOOR_DB: f32 = -60.0;
/// Time constant of the rise: the bar follows loud peaks almost immediately.
pub const ATTACK: Duration = Duration::from_millis(40);
/// Time constant of the fall: the bar sinks slowly so short peaks stay readable.
pub const RELEASE: Duration = Duration::from_millis(350);
/// Sample rate of the meter while the audio tab is visible (~25 Hz).
pub const INTERVAL: Duration = Duration::from_millis(40);

/// Maps a linear peak (`IAudioMeterInformation::GetPeakValue`, 0..=1) to a display level in 0..=1.
/// dB scale: linear amplitude would hide everything but very loud audio.
pub fn peak_to_level(peak: f32) -> f32 {
    if !peak.is_finite() || peak <= 0. {
        return 0.;
    }
    let db = 20. * peak.min(1.).log10();
    ((db - FLOOR_DB) / -FLOOR_DB).clamp(0., 1.)
}

/// One smoothing step towards `target` over `dt`: fast attack, slow release (exponential).
pub fn smooth(previous: f32, target: f32, dt: Duration) -> f32 {
    let tau = if target > previous { ATTACK } else { RELEASE };
    let alpha = 1. - (-dt.as_secs_f32() / tau.as_secs_f32()).exp();
    let next = previous + (target - previous) * alpha;
    // Do not creep towards silence forever.
    if next < 0.005 && target <= 0. {
        0.
    } else {
        next.clamp(0., 1.)
    }
}

/// Display level 0..=1 as whole percent for the UI (change detection is cheap on integers).
pub fn level_percent(level: f32) -> u8 {
    (level.clamp(0., 1.) * 100.).round() as u8
}

const GREEN: [u8; 3] = [0x3e, 0xcf, 0x6e];
const YELLOW: [u8; 3] = [0xe8, 0xc9, 0x3a];
const RED: [u8; 3] = [0xe5, 0x48, 0x4d];
/// Level (percent) at which the bar is fully yellow.
const YELLOW_AT: u8 = 70;

fn mix(from: [u8; 3], to: [u8; 3], t: f32) -> [u8; 3] {
    let channel =
        |i: usize| (f32::from(from[i]) + (f32::from(to[i]) - f32::from(from[i])) * t).round() as u8;
    [channel(0), channel(1), channel(2)]
}

/// Bar color for a level in percent: green, then yellow, then red towards full scale.
pub fn level_color(percent: u8) -> [u8; 3] {
    let percent = percent.min(100);
    if percent <= YELLOW_AT {
        mix(GREEN, YELLOW, f32::from(percent) / f32::from(YELLOW_AT))
    } else {
        mix(
            YELLOW,
            RED,
            f32::from(percent - YELLOW_AT) / f32::from(100 - YELLOW_AT),
        )
    }
}

/// Smoothed level per row id. Rows that vanish from the input are forgotten.
#[derive(Default)]
pub struct Smoother {
    levels: HashMap<String, f32>,
}

impl Smoother {
    /// Feeds raw linear peaks measured `dt` after the previous call; returns display
    /// percents in input order.
    pub fn update(&mut self, peaks: &[(String, f32)], dt: Duration) -> Vec<(String, u8)> {
        let mut next = HashMap::with_capacity(peaks.len());
        let out = peaks
            .iter()
            .map(|(id, peak)| {
                let previous = self.levels.get(id).copied().unwrap_or(0.);
                let level = smooth(previous, peak_to_level(*peak), dt);
                next.insert(id.clone(), level);
                (id.clone(), level_percent(level))
            })
            .collect();
        self.levels = next;
        out
    }
    pub fn clear(&mut self) {
        self.levels.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const DT: Duration = INTERVAL;

    #[test]
    fn peak_maps_on_db_scale_and_clamps() {
        assert_eq!(peak_to_level(0.), 0.);
        assert_eq!(peak_to_level(-1.), 0.);
        assert_eq!(peak_to_level(f32::NAN), 0.);
        assert_eq!(peak_to_level(f32::INFINITY), 0.);
        assert!((peak_to_level(1.) - 1.).abs() < 1e-6);
        assert_eq!(peak_to_level(2.), 1.);
        // -6 dB (half amplitude) is 90 % of a 60 dB range.
        assert!((peak_to_level(0.501_187) - 0.9).abs() < 1e-3);
        assert_eq!(peak_to_level(0.000_1), 0.); // -80 dB, below floor
        assert!(peak_to_level(0.01) < peak_to_level(0.1));
    }

    #[test]
    fn attack_is_faster_than_release() {
        let up = smooth(0., 1., DT);
        let down = 1. - smooth(1., 0., DT);
        assert!(up > 0.5, "rise {up}");
        assert!(down < 0.2, "fall {down}");
        assert!(up > down * 3.);
    }

    #[test]
    fn smoothing_converges_and_never_overshoots() {
        let mut level = 0.;
        for _ in 0..50 {
            level = smooth(level, 0.8, DT);
            assert!(level <= 0.8 + 1e-6);
        }
        assert!((level - 0.8).abs() < 1e-3);
        for _ in 0..100 {
            level = smooth(level, 0., DT);
        }
        assert_eq!(level, 0.);
    }

    #[test]
    fn zero_dt_changes_nothing() {
        assert_eq!(smooth(0.4, 1., Duration::ZERO), 0.4);
        assert_eq!(smooth(0.4, 0., Duration::ZERO), 0.4);
    }

    #[test]
    fn percent_rounds_and_clamps() {
        assert_eq!(level_percent(0.), 0);
        assert_eq!(level_percent(0.504), 50);
        assert_eq!(level_percent(1.), 100);
        assert_eq!(level_percent(7.), 100);
        assert_eq!(level_percent(-1.), 0);
    }

    #[test]
    fn color_runs_green_yellow_red() {
        assert_eq!(level_color(0), GREEN);
        assert_eq!(level_color(YELLOW_AT), YELLOW);
        assert_eq!(level_color(100), RED);
        assert_eq!(level_color(250), RED);
        // Green channel dominates low, red channel dominates high levels.
        let low = level_color(10);
        let high = level_color(95);
        assert!(low[1] > low[0]);
        assert!(high[0] > high[1]);
        // Monotonic hue: red channel never falls up to yellow.
        assert!((1..=YELLOW_AT).all(|p| level_color(p)[0] >= level_color(p - 1)[0]));
    }

    #[test]
    fn smoother_tracks_rows_independently_and_forgets_removed_ones() {
        let mut smoother = Smoother::default();
        let rows = |a: f32, b: f32| vec![("a".to_owned(), a), ("b".to_owned(), b)];
        let first = smoother.update(&rows(1., 0.), DT);
        assert_eq!(first[0].0, "a");
        assert!(first[0].1 > 50);
        assert_eq!(first[1], ("b".to_owned(), 0));
        // Falls slowly after the peak is gone.
        let second = smoother.update(&rows(0., 0.), DT);
        assert!(second[0].1 > 0 && second[0].1 < first[0].1);
        // "a" vanishes and reappears: starts from zero again.
        smoother.update(&[("b".to_owned(), 0.)], DT);
        let again = smoother.update(&rows(0., 0.), DT);
        assert_eq!(again[0], ("a".to_owned(), 0));
        smoother.update(&rows(1., 1.), DT);
        smoother.clear();
        assert_eq!(smoother.update(&rows(0., 0.), DT)[0].1, 0);
    }
}
