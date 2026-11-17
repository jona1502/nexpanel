//! Check, download and verify. All network access goes through [`Transport`]; the tests use a
//! mock. The result of a download is a verified [`Package`] in memory, nothing touches the
//! install directory here.

use super::{
    machine::{ErrorKind, UpdateError},
    release::{
        MAX_HASH_BYTES, MAX_LIST_BYTES, MAX_PACKAGE_BYTES, RELEASES_API_URL, Release,
        parse_releases, select_update,
    },
    sha256,
    version::Version,
    zip::{self, Limits},
};

pub const EXE_NAME: &str = "nexpanel.exe";
pub const VERSION_FILE: &str = "VERSION.txt";
/// Files of a release package, as written by `scripts/package.ps1`. Only these are ever read
/// from the archive and only these are written into the install directory.
pub const PACKAGE_FILES: [&str; 6] = [
    "install.ps1",
    "uninstall.ps1",
    "README.md",
    "THIRD-PARTY-NOTICES.md",
    VERSION_FILE,
    EXE_NAME,
];
/// Package files that must be present; the rest is optional.
const REQUIRED: [&str; 2] = [EXE_NAME, VERSION_FILE];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// Connection problem, timeout, HTTP error status.
    Network(String),
    /// URL or redirect not allowed by the policy.
    Rejected(String),
    /// Response larger than the given limit.
    TooLarge,
    /// The progress callback asked to stop.
    Cancelled,
}

/// HTTPS GET with a size limit. `progress(received, total)` is called while data arrives and
/// returns `false` to abort the transfer.
pub trait Transport {
    fn fetch(
        &self,
        url: &str,
        max_bytes: u64,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<Vec<u8>, TransportError>;
}

impl From<TransportError> for UpdateError {
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::Network(d) => Self::new(ErrorKind::Network, d),
            TransportError::Rejected(d) => Self::new(ErrorKind::Rejected, d),
            TransportError::TooLarge => Self::new(ErrorKind::TooLarge, "Antwort zu groß"),
            TransportError::Cancelled => Self::new(ErrorKind::Other, "abgebrochen"),
        }
    }
}

/// Newest release that is newer than `current`, or `None`.
pub fn check(
    transport: &dyn Transport,
    current: &Version,
    include_prerelease: bool,
) -> Result<Option<Release>, UpdateError> {
    let body = transport.fetch(RELEASES_API_URL, MAX_LIST_BYTES, &mut |_, _| true)?;
    let text = String::from_utf8(body)
        .map_err(|_| UpdateError::new(ErrorKind::Other, "Release-Liste ist kein UTF-8"))?;
    let releases = parse_releases(&text).map_err(|e| UpdateError::new(ErrorKind::Other, e))?;
    Ok(select_update(current, &releases, include_prerelease))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Downloading { done: u64, total: Option<u64> },
    Verifying,
}

/// A downloaded, hash-checked and unpacked package. `files` holds only [`PACKAGE_FILES`] names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub version: Version,
    pub files: Vec<(String, Vec<u8>)>,
}

fn bad_package(detail: impl Into<String>) -> UpdateError {
    UpdateError::new(ErrorKind::BadPackage, detail)
}

/// Downloads `release`, checks the SHA-256 from the release's `.sha256` asset, unpacks the
/// allowed files and checks that `VERSION.txt` names the release version. `progress` returns
/// `false` to abort.
///
/// The hash comes from the same release as the archive, so it proves completeness, not
/// authenticity (releases are unsigned).
pub fn download_package(
    transport: &dyn Transport,
    current: &Version,
    release: &Release,
    progress: &mut dyn FnMut(Stage) -> bool,
) -> Result<Package, UpdateError> {
    if release.version <= *current {
        return Err(UpdateError::new(
            ErrorKind::VersionMismatch,
            format!("{} ist nicht neuer als {current}", release.version),
        ));
    }
    if release.zip.size > MAX_PACKAGE_BYTES {
        return Err(UpdateError::new(
            ErrorKind::TooLarge,
            "Paket größer als erlaubt",
        ));
    }
    let hash_text = transport.fetch(&release.sha256.url, MAX_HASH_BYTES, &mut |_, _| true)?;
    let hash_text = String::from_utf8(hash_text)
        .map_err(|_| UpdateError::new(ErrorKind::HashMismatch, "Prüfsummendatei ist kein UTF-8"))?;
    let expected = sha256::parse_sha256_file(&hash_text, &release.zip.name)
        .map_err(|e| UpdateError::new(ErrorKind::HashMismatch, e))?;

    let archive = transport.fetch(&release.zip.url, MAX_PACKAGE_BYTES, &mut |done, total| {
        progress(Stage::Downloading { done, total })
    })?;
    if !progress(Stage::Verifying) {
        return Err(TransportError::Cancelled.into());
    }
    if sha256::digest(&archive) != expected {
        return Err(UpdateError::new(
            ErrorKind::HashMismatch,
            "SHA256 des Downloads stimmt nicht mit der Prüfsumme überein",
        ));
    }

    let optional: Vec<&str> = PACKAGE_FILES
        .iter()
        .copied()
        .filter(|n| !REQUIRED.contains(n))
        .collect();
    let files = zip::read_entries(&archive, &PACKAGE_FILES, &optional, &Limits::default())
        .map_err(|e| bad_package(e.to_string()))?;
    let version_text = files
        .iter()
        .find(|(name, _)| name == VERSION_FILE)
        .map(|(_, data)| String::from_utf8_lossy(data).trim().to_owned())
        .ok_or_else(|| bad_package("VERSION.txt fehlt"))?;
    let packaged = Version::parse(&version_text)
        .map_err(|e| UpdateError::new(ErrorKind::VersionMismatch, e.to_string()))?;
    if packaged != release.version {
        return Err(UpdateError::new(
            ErrorKind::VersionMismatch,
            format!(
                "Paket enthält Version {packaged}, Release ist {}",
                release.version
            ),
        ));
    }
    let exe = files
        .iter()
        .find(|(name, _)| name == EXE_NAME)
        .map(|(_, data)| data.as_slice())
        .ok_or_else(|| bad_package("nexpanel.exe fehlt"))?;
    if exe.len() < 1024 || &exe[..2] != b"MZ" {
        return Err(bad_package(
            "nexpanel.exe ist keine ausführbare Windows-Datei",
        ));
    }
    Ok(Package {
        version: packaged,
        files,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::update::{
        release::test_release,
        zip::testutil::{Method, build},
    };
    use std::{cell::RefCell, collections::HashMap};

    /// Scripted transport: url -> response; records the requested URLs.
    #[derive(Default)]
    pub(crate) struct MockTransport {
        pub responses: RefCell<HashMap<String, Result<Vec<u8>, TransportError>>>,
        pub requests: RefCell<Vec<String>>,
    }

    impl MockTransport {
        pub fn with(self, url: &str, response: Result<Vec<u8>, TransportError>) -> Self {
            self.responses.borrow_mut().insert(url.to_owned(), response);
            self
        }
    }

    impl Transport for MockTransport {
        fn fetch(
            &self,
            url: &str,
            max_bytes: u64,
            progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
        ) -> Result<Vec<u8>, TransportError> {
            self.requests.borrow_mut().push(url.to_owned());
            let response = self
                .responses
                .borrow()
                .get(url)
                .cloned()
                .unwrap_or(Err(TransportError::Network("404".into())))?;
            if response.len() as u64 > max_bytes {
                return Err(TransportError::TooLarge);
            }
            let total = Some(response.len() as u64);
            let half = response.len() as u64 / 2;
            if !progress(half, total) || !progress(response.len() as u64, total) {
                return Err(TransportError::Cancelled);
            }
            Ok(response)
        }
    }

    fn exe() -> Vec<u8> {
        let mut data = b"MZ".to_vec();
        data.resize(4096, 7);
        data
    }

    fn package_zip(version: &str) -> Vec<u8> {
        build(&[
            ("nexpanel.exe", &exe(), Method::Deflate),
            ("VERSION.txt", version.as_bytes(), Method::Stored),
            ("README.md", b"readme", Method::Deflate),
        ])
    }

    fn transport_for(release: &Release, zip: &[u8], hash_line: Option<String>) -> MockTransport {
        let hash = hash_line
            .unwrap_or_else(|| format!("{}  {}\r\n", sha256::hex_digest(zip), release.zip.name));
        MockTransport::default()
            .with(&release.zip.url, Ok(zip.to_vec()))
            .with(&release.sha256.url, Ok(hash.into_bytes()))
    }

    fn current() -> Version {
        Version::parse("0.1.0").unwrap()
    }

    fn download(t: &MockTransport, release: &Release) -> Result<Package, UpdateError> {
        download_package(t, &current(), release, &mut |_| true)
    }

    #[test]
    fn downloads_verifies_and_unpacks() {
        let release = test_release("v0.2.0");
        let zip = package_zip("0.2.0\r\n");
        let t = transport_for(&release, &zip, None);
        let mut stages = Vec::new();
        let package = download_package(&t, &current(), &release, &mut |s| {
            stages.push(s);
            true
        })
        .unwrap();
        assert_eq!(package.version, release.version);
        let names: Vec<_> = package.files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["README.md", "VERSION.txt", "nexpanel.exe"]);
        assert!(matches!(stages.first(), Some(Stage::Downloading { .. })));
        assert_eq!(stages.last(), Some(&Stage::Verifying));
        // Hash is fetched before the package.
        assert_eq!(
            *t.requests.borrow(),
            [release.sha256.url.clone(), release.zip.url.clone()]
        );
    }

    #[test]
    fn hash_mismatch_is_rejected() {
        let release = test_release("v0.2.0");
        let zip = package_zip("0.2.0");
        let wrong = format!("{}  {}", sha256::hex_digest(b"other"), release.zip.name);
        let err = download(&transport_for(&release, &zip, Some(wrong)), &release).unwrap_err();
        assert_eq!(err.kind, ErrorKind::HashMismatch);
        // One flipped byte in the download.
        let mut tampered = zip.clone();
        let last = tampered.len() / 2;
        tampered[last] ^= 1;
        let t = transport_for(&release, &zip, None).with(&release.zip.url, Ok(tampered));
        assert_eq!(
            download(&t, &release).unwrap_err().kind,
            ErrorKind::HashMismatch
        );
    }

    #[test]
    fn hash_file_of_another_package_or_garbage_is_rejected() {
        let release = test_release("v0.2.0");
        let zip = package_zip("0.2.0");
        for line in [
            format!(
                "{}  nexpanel-0.1.0-windows-x64.zip",
                sha256::hex_digest(&zip)
            ),
            "not a hash".to_string(),
            String::new(),
        ] {
            let err = download(&transport_for(&release, &zip, Some(line)), &release).unwrap_err();
            assert_eq!(err.kind, ErrorKind::HashMismatch);
        }
    }

    #[test]
    fn version_file_must_match_release() {
        let release = test_release("v0.2.0");
        for version in ["0.3.0", "0.1.0", "garbage", ""] {
            let zip = package_zip(version);
            let err = download(&transport_for(&release, &zip, None), &release).unwrap_err();
            assert_eq!(err.kind, ErrorKind::VersionMismatch, "{version:?}");
        }
    }

    #[test]
    fn refuses_downgrade_and_same_version_without_network() {
        let t = MockTransport::default();
        for tag in ["v0.1.0", "v0.0.9"] {
            let err = download(&t, &test_release(tag)).unwrap_err();
            assert_eq!(err.kind, ErrorKind::VersionMismatch);
        }
        assert!(t.requests.borrow().is_empty());
    }

    #[test]
    fn oversized_announcement_is_rejected_before_download() {
        let mut release = test_release("v0.2.0");
        release.zip.size = MAX_PACKAGE_BYTES + 1;
        let t = MockTransport::default();
        assert_eq!(
            download(&t, &release).unwrap_err().kind,
            ErrorKind::TooLarge
        );
        assert!(t.requests.borrow().is_empty());
    }

    #[test]
    fn broken_packages_are_rejected() {
        let release = test_release("v0.2.0");
        let no_exe = build(&[("VERSION.txt", b"0.2.0", Method::Stored)]);
        let no_version = build(&[("nexpanel.exe", &exe(), Method::Stored)]);
        let not_exe = build(&[
            ("nexpanel.exe", b"#!/bin/sh", Method::Stored),
            ("VERSION.txt", b"0.2.0", Method::Stored),
        ]);
        let slip = build(&[
            ("nexpanel.exe", &exe(), Method::Stored),
            ("VERSION.txt", b"0.2.0", Method::Stored),
            ("..\\evil.dll", b"x", Method::Stored),
        ]);
        for zip in [no_exe, no_version, not_exe, slip, b"not a zip".to_vec()] {
            let err = download(&transport_for(&release, &zip, None), &release).unwrap_err();
            assert_eq!(err.kind, ErrorKind::BadPackage, "{err:?}");
        }
    }

    #[test]
    fn network_errors_and_cancellation_propagate() {
        let release = test_release("v0.2.0");
        let t = MockTransport::default();
        assert_eq!(download(&t, &release).unwrap_err().kind, ErrorKind::Network);
        let zip = package_zip("0.2.0");
        let t = transport_for(&release, &zip, None).with(
            &release.zip.url,
            Err(TransportError::Rejected("host".into())),
        );
        assert_eq!(
            download(&t, &release).unwrap_err().kind,
            ErrorKind::Rejected
        );
        let t = transport_for(&release, &zip, None);
        let err = download_package(&t, &current(), &release, &mut |_| false).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Other);
    }

    fn api_body(tags: &[&str]) -> Vec<u8> {
        let entries: Vec<String> = tags
            .iter()
            .map(|tag| {
                let name = crate::update::release::package_name(&tag[1..]);
                format!(
                    r#"{{"tag_name":"{tag}","draft":false,"prerelease":true,"assets":[{{"name":"{name}","size":10}},{{"name":"{name}.sha256","size":10}}]}}"#
                )
            })
            .collect();
        format!("[{}]", entries.join(",")).into_bytes()
    }

    #[test]
    fn check_finds_newer_release_only() {
        let t = MockTransport::default()
            .with(RELEASES_API_URL, Ok(api_body(&["v0.1.0", "v0.2.0-beta.1"])));
        let found = check(&t, &current(), true).unwrap().unwrap();
        assert_eq!(found.tag, "v0.2.0-beta.1");
        assert_eq!(
            check(&t, &Version::parse("0.2.0").unwrap(), true).unwrap(),
            None
        );
        let none = MockTransport::default().with(RELEASES_API_URL, Ok(api_body(&["v0.1.0"])));
        assert_eq!(check(&none, &current(), true).unwrap(), None);
    }

    #[test]
    fn check_reports_bad_responses() {
        let t = MockTransport::default()
            .with(RELEASES_API_URL, Ok(b"<html>rate limit</html>".to_vec()));
        assert_eq!(
            check(&t, &current(), true).unwrap_err().kind,
            ErrorKind::Other
        );
        let t = MockTransport::default().with(RELEASES_API_URL, Ok(vec![0xFF, 0xFE]));
        assert!(check(&t, &current(), true).is_err());
        let t = MockTransport::default().with(
            RELEASES_API_URL,
            Err(TransportError::Network("offline".into())),
        );
        assert_eq!(
            check(&t, &current(), true).unwrap_err().kind,
            ErrorKind::Network
        );
    }
}
