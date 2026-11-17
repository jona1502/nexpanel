//! Semantic versions (SemVer 2.0.0 precedence, including pre-release identifiers).
//! Build metadata (`+...`) is accepted and ignored for ordering, as the spec demands.

use std::{cmp::Ordering, fmt};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ident {
    Numeric(u64),
    Text(String),
}

impl Ord for Ident {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Ident::Numeric(a), Ident::Numeric(b)) => a.cmp(b),
            // Numeric identifiers always have lower precedence than alphanumeric ones.
            (Ident::Numeric(_), Ident::Text(_)) => Ordering::Less,
            (Ident::Text(_), Ident::Numeric(_)) => Ordering::Greater,
            (Ident::Text(a), Ident::Text(b)) => a.cmp(b),
        }
    }
}
impl PartialOrd for Ident {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pre: Vec<Ident>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseVersionError(pub String);

impl fmt::Display for ParseVersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ungültige Version: {}", self.0)
    }
}
impl std::error::Error for ParseVersionError {}

fn number(part: &str) -> Option<u64> {
    let valid = !part.is_empty()
        && part.bytes().all(|b| b.is_ascii_digit())
        && (part == "0" || !part.starts_with('0'));
    if valid { part.parse().ok() } else { None }
}

impl Version {
    /// Parses `1.2.3`, `1.2.3-beta.1` and `1.2.3+build`. A leading `v` (git tag) is accepted.
    pub fn parse(text: &str) -> Result<Self, ParseVersionError> {
        let fail = || ParseVersionError(text.chars().take(40).collect());
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let (core_and_pre, build) = match text.split_once('+') {
            Some((head, build)) => (head, Some(build)),
            None => (text, None),
        };
        if let Some(build) = build
            && !build.split('.').all(valid_ident_chars)
        {
            return Err(fail());
        }
        let (core, pre) = match core_and_pre.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (core_and_pre, None),
        };
        let mut parts = core.split('.');
        let (Some(a), Some(b), Some(c), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(fail());
        };
        let (major, minor, patch) = (
            number(a).ok_or_else(fail)?,
            number(b).ok_or_else(fail)?,
            number(c).ok_or_else(fail)?,
        );
        let mut idents = Vec::new();
        if let Some(pre) = pre {
            for part in pre.split('.') {
                if !valid_ident_chars(part) {
                    return Err(fail());
                }
                if part.bytes().all(|b| b.is_ascii_digit()) {
                    idents.push(Ident::Numeric(number(part).ok_or_else(fail)?));
                } else {
                    idents.push(Ident::Text(part.to_owned()));
                }
            }
        }
        Ok(Self {
            major,
            minor,
            patch,
            pre: idents,
        })
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }
}

fn valid_ident_chars(part: &str) -> bool {
    !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                // A release is newer than any of its pre-releases.
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                // Lexicographic over identifiers; with equal prefix the longer set wins.
                (false, false) => self.pre.cmp(&other.pre),
            })
    }
}
impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        for (i, id) in self.pre.iter().enumerate() {
            f.write_str(if i == 0 { "-" } else { "." })?;
            match id {
                Ident::Numeric(n) => write!(f, "{n}")?,
                Ident::Text(t) => f.write_str(t)?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn parses_core_pre_build_and_tag_prefix() {
        assert_eq!(v("1.2.3").to_string(), "1.2.3");
        assert_eq!(v("v0.1.0").to_string(), "0.1.0");
        assert_eq!(v("1.0.0-rc.1").to_string(), "1.0.0-rc.1");
        assert_eq!(v("1.0.0+build.5").to_string(), "1.0.0");
        assert_eq!(
            v(" 2.0.0-alpha.beta-1+x ").to_string(),
            "2.0.0-alpha.beta-1"
        );
        assert!(v("0.1.0-beta").is_prerelease());
        assert!(!v("0.1.0").is_prerelease());
    }

    #[test]
    fn rejects_malformed_versions() {
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.02.3",
            "1.2.-3",
            "a.b.c",
            "1.2.3-",
            "1.2.3-a..b",
            "1.2.3-01",
            "1.2.3+",
            "1.2.3-a_b",
            "1.2.3 beta",
            "vv1.2.3",
            "99999999999999999999.0.0",
            "1.2.3-é",
        ] {
            assert!(Version::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn orders_per_semver_spec() {
        // The canonical chain from semver.org section 11.
        let chain = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.1.0",
            "2.0.0",
        ];
        for pair in chain.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
            assert!(v(pair[1]) > v(pair[0]));
        }
    }

    #[test]
    fn numeric_comparison_not_lexical() {
        assert!(v("0.9.0") < v("0.10.0"));
        assert!(v("0.1.9") < v("0.1.10"));
        assert!(v("1.0.0-beta.2") < v("1.0.0-beta.11"));
    }

    #[test]
    fn build_metadata_does_not_change_precedence() {
        assert_eq!(v("1.0.0+a").cmp(&v("1.0.0+b")), Ordering::Equal);
    }

    #[test]
    fn pre_one_zero_is_newer_than_zero_one() {
        assert!(v("0.1.0") < v("0.1.1-beta"));
        assert!(v("0.1.1-beta") < v("0.1.1"));
    }
}
