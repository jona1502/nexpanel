//! URL policy for update traffic: HTTPS only, fixed host allowlist, bounded redirects.
//! Pure string logic; the WinHTTP transport applies it to the request URL and to every
//! redirect target before following it.

/// Exact host names (no wildcard, no subdomains). `api.github.com` serves the release list;
/// GitHub answers release downloads with a redirect from `github.com` to the asset hosts
/// (`objects.githubusercontent.com`, currently `release-assets.githubusercontent.com`).
pub const ALLOWED_HOSTS: [&str; 4] = [
    "github.com",
    "api.github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

pub const MAX_REDIRECTS: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpsUrl {
    pub host: String,
    /// Path and query, always starting with `/`; the fragment is dropped.
    pub target: String,
}

impl HttpsUrl {
    pub fn to_url_string(&self) -> String {
        format!("https://{}{}", self.host, self.target)
    }
}

fn valid_target(target: &str) -> bool {
    // Must already be percent-encoded: printable ASCII only, no backslash.
    target.bytes().all(|b| b.is_ascii_graphic() && b != b'\\')
}

/// Parses and validates an absolute URL.
pub fn parse_url(url: &str) -> Result<HttpsUrl, String> {
    let url = url.trim();
    let rest = match url.get(..8) {
        Some(scheme) if scheme.eq_ignore_ascii_case("https://") => &url[8..],
        _ => return Err("nur https:// ist erlaubt".into()),
    };
    let rest = rest.split('#').next().unwrap_or_default();
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, target) = rest.split_at(end);
    if authority.contains('@') {
        return Err("Zugangsdaten in der URL sind nicht erlaubt".into());
    }
    let host = match authority.split_once(':') {
        Some((host, "443")) => host,
        Some(_) => return Err("nur Port 443 ist erlaubt".into()),
        None => authority,
    };
    let host = host.to_ascii_lowercase();
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
    {
        return Err("ungültiger Host".into());
    }
    if !ALLOWED_HOSTS.contains(&host.as_str()) {
        return Err(format!("Host nicht erlaubt: {host}"));
    }
    let target = if target.is_empty() {
        "/".to_owned()
    } else if target.starts_with('?') {
        format!("/{target}")
    } else {
        target.to_owned()
    };
    if !valid_target(&target) {
        return Err("ungültige Zeichen im Pfad".into());
    }
    Ok(HttpsUrl { host, target })
}

/// Resolves the `Location` header of a redirect against the current URL and validates the
/// result with the same rules as the first request.
pub fn resolve_redirect(base: &HttpsUrl, location: &str) -> Result<HttpsUrl, String> {
    let location = location.trim();
    if location.is_empty() {
        return Err("leere Weiterleitung".into());
    }
    if location.contains("://") {
        parse_url(location)
    } else if location.starts_with("//") {
        parse_url(&format!("https:{location}"))
    } else if location.starts_with('/') {
        parse_url(&format!("https://{}{location}", base.host))
    } else {
        Err("relative Weiterleitung ohne führenden Schrägstrich nicht erlaubt".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_allowed_hosts() {
        let u = parse_url("https://github.com/jona1502/nexpanel/releases/download/v0.2.0/a.zip")
            .unwrap();
        assert_eq!(u.host, "github.com");
        assert_eq!(
            u.target,
            "/jona1502/nexpanel/releases/download/v0.2.0/a.zip"
        );
        assert!(parse_url("https://api.github.com/repos/jona1502/nexpanel/releases").is_ok());
    }

    #[test]
    fn host_match_is_case_insensitive_and_exact() {
        assert!(parse_url("https://OBJECTS.githubusercontent.com/a?b=c").is_ok());
        assert!(parse_url("https://release-assets.githubusercontent.com:443/a").is_ok());
        for bad in [
            "https://evil.com/",
            "https://github.com.evil.com/",
            "https://evilgithub.com/",
            "https://sub.github.com/",
            "https://raw.githubusercontent.com/x",
            "https://githubusercontent.com/x",
            "https://github.com@evil.com/",
            "https://evil.com@github.com/",
            "https://github.com:8443/x",
            "https://github.com:80/x",
            "https://github.com:443:1/x",
            "https://[::1]/x",
            "https:///x",
            "https://github.com\\@evil.com/",
            "https://github.com./x",
            "https://gïthub.com/x",
        ] {
            assert!(parse_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn rejects_non_https_schemes() {
        for bad in [
            "http://github.com/x",
            "ftp://github.com/x",
            "file:///c:/x",
            "//github.com/x",
            "github.com/x",
            "",
            "https:/github.com/x",
            "javascript:alert(1)",
        ] {
            assert!(parse_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn normalizes_target() {
        assert_eq!(parse_url("https://github.com").unwrap().target, "/");
        assert_eq!(parse_url("https://github.com?x=1").unwrap().target, "/?x=1");
        assert_eq!(parse_url("https://github.com/a#frag").unwrap().target, "/a");
        assert!(parse_url("https://github.com/a b").is_err());
        assert!(parse_url("https://github.com/a\\b").is_err());
        assert!(parse_url("https://github.com/a\u{e4}").is_err());
        assert!(parse_url("https://github.com/a\r\nHost: evil").is_err());
    }

    #[test]
    fn redirects_are_revalidated() {
        let base = parse_url("https://github.com/a/b").unwrap();
        assert_eq!(
            resolve_redirect(
                &base,
                "https://release-assets.githubusercontent.com/x?sig=1"
            )
            .unwrap()
            .host,
            "release-assets.githubusercontent.com"
        );
        assert_eq!(
            resolve_redirect(&base, "/other").unwrap().to_url_string(),
            "https://github.com/other"
        );
        assert_eq!(
            resolve_redirect(&base, "//objects.githubusercontent.com/q")
                .unwrap()
                .host,
            "objects.githubusercontent.com"
        );
        for bad in [
            "http://objects.githubusercontent.com/x",
            "https://evil.com/x",
            "//evil.com/x",
            "relative/path",
            "",
            "ftp://github.com/x",
        ] {
            assert!(resolve_redirect(&base, bad).is_err(), "{bad}");
        }
    }
}
