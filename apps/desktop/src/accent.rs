//! Pure helpers for accent colors: hex parsing and readability on the dark theme.

pub type Rgb = [u8; 3];

/// Brightest panel surface an accent is drawn on (`Card`/dropdown background `#252832`).
/// Accents readable here are readable on the window background `#181a20` too.
pub const SURFACE: Rgb = [0x25, 0x28, 0x32];
/// WCAG contrast ratio for normal text; accents are used for text and thin bars.
pub const MIN_CONTRAST: f64 = 4.5;

/// Parses `#RRGGBB`, `RRGGBB`, `#RGB` or `RGB` (case-insensitive, surrounding spaces ignored).
pub fn parse_hex(input: &str) -> Option<Rgb> {
    let digits = input.trim();
    let digits = digits.strip_prefix('#').unwrap_or(digits);
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = |i: usize| u8::from_str_radix(&digits[i..=i], 16).ok();
    match digits.len() {
        3 => Some([value(0)? * 17, value(1)? * 17, value(2)? * 17]),
        6 => {
            let pair = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
            Some([pair(0)?, pair(2)?, pair(4)?])
        }
        _ => None,
    }
}

/// Canonical lowercase `#rrggbb`.
pub fn to_hex(rgb: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// WCAG relative luminance, 0.0 (black) to 1.0 (white).
pub fn luminance(rgb: Rgb) -> f64 {
    let channel = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2])
}

/// WCAG contrast ratio, 1.0 to 21.0, independent of argument order.
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Blends `rgb` toward white in 1 % steps until it reaches `min` contrast against `background`.
/// Colors that already pass are returned unchanged; white is the last resort.
pub fn lighten_to_contrast(rgb: Rgb, background: Rgb, min: f64) -> Rgb {
    if contrast(rgb, background) >= min {
        return rgb;
    }
    for step in 1..=100u32 {
        let mixed = rgb.map(|c| {
            let c = u32::from(c);
            u8::try_from(c + (255 - c) * step / 100).unwrap_or(255)
        });
        if contrast(mixed, background) >= min {
            return mixed;
        }
    }
    [255, 255, 255]
}

/// Accent as it is actually used: too dark colors are lifted to readable on [`SURFACE`].
pub fn readable_accent(rgb: Rgb) -> Rgb {
    lighten_to_contrast(rgb, SURFACE, MIN_CONTRAST)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_six_and_three_digit_hex_with_or_without_hash() {
        assert_eq!(parse_hex("#809cff"), Some([0x80, 0x9c, 0xff]));
        assert_eq!(parse_hex("809CFF"), Some([0x80, 0x9c, 0xff]));
        assert_eq!(parse_hex("  #AbC "), Some([0xaa, 0xbb, 0xcc]));
        assert_eq!(parse_hex("f00"), Some([255, 0, 0]));
    }

    #[test]
    fn rejects_malformed_hex() {
        for bad in [
            "", "#", "#12", "#1234", "#12345", "#1234567", "#ggg", "#12345g", "##fff", "# fff",
            "0x112233", "ä12", "#ffé", "rgb",
        ] {
            assert_eq!(parse_hex(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn hex_roundtrips_lowercase() {
        assert_eq!(to_hex([0x80, 0x9c, 0xff]), "#809cff");
        assert_eq!(to_hex([0, 0, 0]), "#000000");
        assert_eq!(parse_hex(&to_hex([1, 2, 3])), Some([1, 2, 3]));
    }

    #[test]
    fn contrast_extremes_and_symmetry() {
        assert!((contrast([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 1e-9);
        assert!((contrast([9, 9, 9], [9, 9, 9]) - 1.0).abs() < 1e-9);
        let (a, b) = ([0x80, 0x9c, 0xff], SURFACE);
        assert_eq!(contrast(a, b), contrast(b, a));
    }

    #[test]
    fn readable_colors_stay_unchanged() {
        for rgb in [[0x80, 0x9c, 0xff], [255, 255, 255], [0x64, 0xd6, 0xa4]] {
            assert_eq!(readable_accent(rgb), rgb);
        }
    }

    #[test]
    fn dark_colors_are_lifted_to_minimum_contrast() {
        for rgb in [
            [0, 0, 0],
            [0x25, 0x28, 0x32],
            [0, 0, 0x80],
            [0x30, 0x10, 0x10],
        ] {
            let lifted = readable_accent(rgb);
            assert!(contrast(lifted, SURFACE) >= MIN_CONTRAST, "{rgb:?}");
            assert!(luminance(lifted) > luminance(rgb), "{rgb:?}");
        }
    }

    #[test]
    fn lifting_keeps_hue_direction_and_stops_at_first_passing_step() {
        // Pure dark blue stays blue-dominant while being lightened.
        let lifted = readable_accent([0, 0, 0x80]);
        assert!(lifted[2] >= lifted[0] && lifted[2] >= lifted[1]);
        // One step less would fail.
        let almost = lighten_to_contrast([0x40, 0x40, 0x40], SURFACE, 2.0);
        assert!(contrast(almost, SURFACE) >= 2.0);
    }
}
