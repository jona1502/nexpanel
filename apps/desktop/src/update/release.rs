//! Reads the GitHub release list and picks the newest release that ships the expected assets.
//! Pure: takes JSON text, performs no network access.

use super::version::Version;
use serde::Deserialize;

pub const REPO: &str = "jona1502/nexpanel";
/// Release list including pre-releases (every 0.x version is a pre-release).
pub const RELEASES_API_URL: &str =
    "https://api.github.com/repos/jona1502/nexpanel/releases?per_page=30";

/// Largest accepted release-list response.
pub const MAX_LIST_BYTES: u64 = 4 * 1024 * 1024;
/// Largest accepted package (zip), checked against the announced and the received size.
pub const MAX_PACKAGE_BYTES: u64 = 100 * 1024 * 1024;
/// Largest accepted `.sha256` asset.
pub const MAX_HASH_BYTES: u64 = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
    pub prerelease: bool,
    pub zip: Asset,
    pub sha256: Asset,
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    #[serde(default)]
    size: u64,
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

/// File name of the package of release `version_text` (the tag without its `v`).
pub fn package_name(version_text: &str) -> String {
    format!("nexpanel-{version_text}-windows-x64.zip")
}

fn tag_is_plain(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 64
        && tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
}

/// The download URLs are built here from the tag, never taken from the API response, so a
/// manipulated `browser_download_url` cannot redirect the download.
fn asset_url(tag: &str, name: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/{tag}/{name}")
}

/// All usable releases (not draft, parsable tag, both assets present with their exact names),
/// newest first. Everything else is skipped silently: it may simply be an old release.
pub fn parse_releases(json: &str) -> Result<Vec<Release>, String> {
    let raw: Vec<RawRelease> =
        serde_json::from_str(json).map_err(|e| format!("Release-Liste unlesbar: {e}"))?;
    let mut out = Vec::new();
    for release in raw {
        if release.draft || !tag_is_plain(&release.tag_name) {
            continue;
        }
        let Ok(version) = Version::parse(&release.tag_name) else {
            continue;
        };
        if !release.tag_name.starts_with('v') {
            continue;
        }
        let bare = &release.tag_name[1..];
        let zip_name = package_name(bare);
        let hash_name = format!("{zip_name}.sha256");
        let find = |name: &str| {
            release
                .assets
                .iter()
                .find(|a| a.name == name)
                .map(|a| Asset {
                    name: name.to_owned(),
                    url: asset_url(&release.tag_name, name),
                    size: a.size,
                })
        };
        if let (Some(zip), Some(sha256)) = (find(&zip_name), find(&hash_name)) {
            out.push(Release {
                version,
                tag: release.tag_name,
                prerelease: release.prerelease,
                zip,
                sha256,
            });
        }
    }
    out.sort_by(|a, b| b.version.cmp(&a.version));
    Ok(out)
}

/// Newest release that is strictly newer than `current`.
pub fn select_update(
    current: &Version,
    releases: &[Release],
    include_prerelease: bool,
) -> Option<Release> {
    releases
        .iter()
        .filter(|r| include_prerelease || !(r.prerelease || r.version.is_prerelease()))
        .filter(|r| r.version > *current)
        .max_by(|a, b| a.version.cmp(&b.version))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(tag: &str, draft: bool, pre: bool, assets: &[&str]) -> String {
        let assets: Vec<String> = assets
            .iter()
            .map(|a| format!(r#"{{"name":"{a}","size":1234,"browser_download_url":"https://evil.example/{a}"}}"#))
            .collect();
        format!(
            r#"{{"tag_name":"{tag}","draft":{draft},"prerelease":{pre},"assets":[{}]}}"#,
            assets.join(",")
        )
    }

    fn full(tag: &str, pre: bool) -> String {
        let zip = package_name(&tag[1..]);
        entry(tag, false, pre, &[&zip, &format!("{zip}.sha256")])
    }

    fn list(entries: &[String]) -> String {
        format!("[{}]", entries.join(","))
    }

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn picks_newest_including_prereleases() {
        let json = list(&[
            full("v0.1.0", true),
            full("v0.3.0", true),
            full("v0.2.0", true),
        ]);
        let releases = parse_releases(&json).unwrap();
        assert_eq!(releases[0].tag, "v0.3.0");
        let pick = select_update(&v("0.1.0"), &releases, true).unwrap();
        assert_eq!(pick.version, v("0.3.0"));
        assert!(pick.prerelease);
    }

    #[test]
    fn urls_are_built_from_tag_not_from_api() {
        let json = list(&[full("v0.2.0", true)]);
        let r = &parse_releases(&json).unwrap()[0];
        assert_eq!(
            r.zip.url,
            "https://github.com/jona1502/nexpanel/releases/download/v0.2.0/nexpanel-0.2.0-windows-x64.zip"
        );
        assert_eq!(r.sha256.url, format!("{}.sha256", r.zip.url));
        assert_eq!(r.zip.size, 1234);
    }

    #[test]
    fn same_or_older_version_is_no_update() {
        let releases =
            parse_releases(&list(&[full("v0.1.0", true), full("v0.0.9", true)])).unwrap();
        assert_eq!(select_update(&v("0.1.0"), &releases, true), None);
        assert_eq!(select_update(&v("0.2.0"), &releases, true), None);
    }

    #[test]
    fn prerelease_is_older_than_its_release() {
        let releases = parse_releases(&list(&[full("v0.2.0-beta.1", true)])).unwrap();
        assert!(select_update(&v("0.2.0"), &releases, true).is_none());
        assert!(select_update(&v("0.1.0"), &releases, true).is_some());
    }

    #[test]
    fn prerelease_filter() {
        let releases =
            parse_releases(&list(&[full("v0.2.0", true), full("v1.0.0-rc.1", false)])).unwrap();
        assert!(select_update(&v("0.1.0"), &releases, false).is_none());
        assert_eq!(
            select_update(&v("0.1.0"), &releases, true).unwrap().tag,
            "v1.0.0-rc.1"
        );
    }

    #[test]
    fn skips_drafts_bad_tags_and_missing_assets() {
        let zip = package_name("0.5.0");
        let json = list(&[
            entry("v0.5.0", true, true, &[&zip, &format!("{zip}.sha256")]),
            full("nightly", true),
            entry("v0.4.0", false, true, &[&package_name("0.4.0")]),
            entry(
                "v0.3.0",
                false,
                true,
                &["nexpanel-0.3.0-linux.zip", "x.sha256"],
            ),
            entry(
                "v0.2.0",
                false,
                true,
                &[
                    &package_name("0.9.9"),
                    &format!("{}.sha256", package_name("0.9.9")),
                ],
            ),
            entry("v../0.1.0", false, true, &[]),
            full("v0.1.5", true),
        ]);
        let releases = parse_releases(&json).unwrap();
        assert_eq!(
            releases.iter().map(|r| r.tag.as_str()).collect::<Vec<_>>(),
            ["v0.1.5"]
        );
    }

    #[test]
    fn invalid_json_is_an_error_and_empty_list_is_fine() {
        assert!(parse_releases("<html>").is_err());
        assert!(parse_releases("{}").is_err());
        assert!(parse_releases("[]").unwrap().is_empty());
    }
}
