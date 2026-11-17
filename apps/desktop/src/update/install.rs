//! Replaces the installed files with a verified package.
//!
//! Layout and marker come from `scripts/install.ps1`: the install directory holds
//! `nexpanel.exe` and the other package files plus `.nexpanel-install.json`. Only a directory
//! with a matching marker is updated, so a development build is never overwritten.
//!
//! A running exe can be renamed but not overwritten or deleted on Windows. Every file is
//! therefore written as `<name>.new`, the old one is renamed to `<name>.old` and `.new` is
//! renamed into place. Any failure rolls all completed renames back. Leftover `.old` files
//! (the running exe cannot be deleted) are removed by [`cleanup_old`] at the next start.

use super::{
    engine::{EXE_NAME, PACKAGE_FILES, Package},
    machine::{ErrorKind, UpdateError},
};
use std::{
    fs, io,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

pub const MARKER_FILE: &str = ".nexpanel-install.json";
/// Argument the new instance gets so it waits for the old process to release the
/// single-instance mutex.
pub const WAIT_PID_ARG: &str = "--wait-pid";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    /// Not an install made by `install.ps1`.
    NotManaged(String),
    NoWritePermission(String),
    Failed(String),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotManaged(d) | Self::NoWritePermission(d) | Self::Failed(d) => f.write_str(d),
        }
    }
}

impl From<InstallError> for UpdateError {
    fn from(error: InstallError) -> Self {
        match error {
            InstallError::NotManaged(d) => Self::new(ErrorKind::NotInstalled, d),
            InstallError::NoWritePermission(d) => Self::new(ErrorKind::NoWritePermission, d),
            InstallError::Failed(d) => Self::new(ErrorKind::InstallFailed, d),
        }
    }
}

fn classify(context: &str, error: &io::Error) -> InstallError {
    let text = format!("{context}: {error}");
    if error.kind() == io::ErrorKind::PermissionDenied {
        InstallError::NoWritePermission(text)
    } else {
        InstallError::Failed(text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub dir: PathBuf,
}

impl Target {
    pub fn exe_path(&self) -> PathBuf {
        self.dir.join(EXE_NAME)
    }
}

/// Lowercase, forward slashes to backslashes, no `\\?\` prefix, no trailing separator.
fn normalized(path: &str) -> String {
    let path = path.trim().replace('/', "\\");
    let path = path.strip_prefix("\\\\?\\").unwrap_or(&path);
    path.trim_end_matches('\\').to_lowercase()
}

pub fn same_path(a: &str, b: &str) -> bool {
    !a.trim().is_empty() && normalized(a) == normalized(b)
}

/// The directory of the running exe if it is a managed install (marker present and matching).
pub fn detect_target(current_exe: &Path) -> Result<Target, InstallError> {
    let not_managed = |why: &str| InstallError::NotManaged(why.to_owned());
    let dir = current_exe
        .parent()
        .ok_or_else(|| not_managed("kein Installationsordner"))?;
    let name_ok = current_exe
        .file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(EXE_NAME));
    if !name_ok {
        return Err(not_managed("die laufende Datei heißt nicht nexpanel.exe"));
    }
    let bytes = fs::read(dir.join(MARKER_FILE)).map_err(|_| {
        not_managed("keine Installationsmarkierung (nicht per install.ps1 installiert)")
    })?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let marker: serde_json::Value =
        serde_json::from_str(text).map_err(|_| not_managed("Installationsmarkierung unlesbar"))?;
    let product_ok = marker.get("product").and_then(|v| v.as_str()) == Some("NexPanel");
    let dir_ok = marker
        .get("directory")
        .and_then(|v| v.as_str())
        .is_some_and(|d| same_path(d, &dir.to_string_lossy()));
    if !product_ok || !dir_ok {
        return Err(not_managed(
            "Installationsmarkierung passt nicht zu diesem Ordner",
        ));
    }
    Ok(Target {
        dir: dir.to_path_buf(),
    })
}

fn sibling(dir: &Path, name: &str, suffix: &str) -> PathBuf {
    dir.join(format!("{name}{suffix}"))
}

fn retry_rename(from: &Path, to: &Path) -> io::Result<()> {
    let mut last = None;
    for attempt in 0..4 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
        if attempt < 3 {
            // Virus scanners briefly hold freshly written files.
            std::thread::sleep(Duration::from_millis(150));
        }
    }
    Err(last.expect("at least one attempt"))
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn write_new(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.write_all(data)?;
    file.sync_all()
}

struct Swapped {
    name: &'static str,
    had_old: bool,
}

fn rollback(dir: &Path, done: &[Swapped]) -> Vec<String> {
    let mut problems = Vec::new();
    for swap in done.iter().rev() {
        let target = dir.join(swap.name);
        let result = if swap.had_old {
            retry_rename(&sibling(dir, swap.name, ".old"), &target)
        } else {
            remove_if_exists(&target)
        };
        if let Err(e) = result {
            problems.push(format!("{}: {e}", swap.name));
        }
    }
    problems
}

/// Installs `package` into `target`. All-or-nothing: on error the previous files are back.
pub fn apply(target: &Target, package: &Package) -> Result<(), InstallError> {
    let dir = &target.dir;
    // Fixed order from PACKAGE_FILES (exe last), never the archive's order or names.
    let mut files: Vec<(&'static str, &[u8])> = Vec::new();
    for &name in &PACKAGE_FILES {
        if let Some((_, data)) = package.files.iter().find(|(n, _)| n == name) {
            files.push((name, data));
        }
    }
    if files.len() != package.files.len() || !files.iter().any(|(n, _)| *n == EXE_NAME) {
        return Err(InstallError::Failed(
            "Paket enthält unerwartete oder keine ausführbare Datei".into(),
        ));
    }

    // Phase 1: write everything next to the final location. Nothing is replaced yet, so a
    // missing write permission shows up here without any change to the installation.
    let mut written = Vec::new();
    for (name, data) in &files {
        let path = sibling(dir, name, ".new");
        written.push(path.clone());
        if let Err(e) = write_new(&path, data) {
            for path in &written {
                let _ = remove_if_exists(path);
            }
            let mut error = classify(&format!("{name} konnte nicht geschrieben werden"), &e);
            if let InstallError::NoWritePermission(text) = &mut error {
                text.push_str(&format!(" (Ordner: {})", dir.display()));
            }
            return Err(error);
        }
    }

    // Phase 2: swap file by file.
    let mut done: Vec<Swapped> = Vec::new();
    for &(name, _) in &files {
        let (target_path, old, new) = (
            dir.join(name),
            sibling(dir, name, ".old"),
            sibling(dir, name, ".new"),
        );
        let step = (|| -> Result<bool, InstallError> {
            remove_if_exists(&old).map_err(|e| {
                classify(
                    &format!(
                        "{name}.old konnte nicht entfernt werden (läuft noch eine alte Kopie?)"
                    ),
                    &e,
                )
            })?;
            let had_old = target_path.exists();
            if had_old {
                retry_rename(&target_path, &old)
                    .map_err(|e| classify(&format!("{name} konnte nicht gesichert werden"), &e))?;
            }
            if let Err(e) = retry_rename(&new, &target_path) {
                if had_old {
                    let _ = retry_rename(&old, &target_path);
                }
                return Err(classify(&format!("{name} konnte nicht ersetzt werden"), &e));
            }
            Ok(had_old)
        })();
        match step {
            Ok(had_old) => done.push(Swapped { name, had_old }),
            Err(error) => {
                let problems = rollback(dir, &done);
                for path in &written {
                    let _ = remove_if_exists(path);
                }
                return Err(if problems.is_empty() {
                    error
                } else {
                    InstallError::Failed(format!(
                        "{error}; Rücknahme unvollständig ({}). Bitte NexPanel neu installieren.",
                        problems.join("; ")
                    ))
                });
            }
        }
    }

    // Phase 3: best effort. The marker records the version; stale `.old` files go away (the
    // running exe's stays until the next start).
    if let Err(e) = write_marker_version(dir, &package.version.to_string()) {
        log::warn!("Installationsmarkierung nicht aktualisiert: {e}");
    }
    for swap in &done {
        if swap.name != EXE_NAME {
            let _ = remove_if_exists(&sibling(dir, swap.name, ".old"));
        }
    }
    Ok(())
}

fn write_marker_version(dir: &Path, version: &str) -> io::Result<()> {
    let path = dir.join(MARKER_FILE);
    let bytes = fs::read(&path)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut marker: serde_json::Value =
        serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(&text))
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let object = marker
        .as_object_mut()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Markierung ist kein Objekt"))?;
    object.insert("version".into(), serde_json::Value::String(version.into()));
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer_pretty(file.as_file_mut(), &marker)?;
    file.as_file_mut().sync_all()?;
    file.persist(&path).map_err(|e| e.error)?;
    Ok(())
}

/// Removes `.old` and `.new` leftovers of a previous update. Returns how many were removed.
/// Files that are still locked are left for the next start.
pub fn cleanup_old(dir: &Path) -> usize {
    let mut removed = 0;
    for name in PACKAGE_FILES {
        for suffix in [".old", ".new"] {
            let path = sibling(dir, name, suffix);
            if path.is_file() && fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

/// Starts the freshly installed exe. It waits (`--wait-pid`) until this process has exited
/// and released the single-instance mutex.
pub fn spawn_new_instance(target: &Target) -> io::Result<()> {
    std::process::Command::new(target.exe_path())
        .arg(WAIT_PID_ARG)
        .arg(std::process::id().to_string())
        .current_dir(&target.dir)
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::version::Version;

    fn write(dir: &Path, name: &str, data: &str) {
        fs::write(dir.join(name), data).unwrap();
    }
    fn read(dir: &Path, name: &str) -> String {
        fs::read_to_string(dir.join(name)).unwrap()
    }

    fn install_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        write(p, "nexpanel.exe", "old exe");
        write(p, "VERSION.txt", "0.1.0");
        write(p, "README.md", "old readme");
        write(p, "install.ps1", "old install");
        let marker = format!(
            r#"{{"product":"NexPanel","directory":{},"version":"0.1.0"}}"#,
            serde_json::to_string(&p.to_string_lossy()).unwrap()
        );
        write(p, MARKER_FILE, &marker);
        dir
    }

    fn package(version: &str) -> Package {
        Package {
            version: Version::parse(version).unwrap(),
            files: vec![
                ("README.md".into(), b"new readme".to_vec()),
                ("THIRD-PARTY-NOTICES.md".into(), b"new notices".to_vec()),
                ("VERSION.txt".into(), version.as_bytes().to_vec()),
                ("nexpanel.exe".into(), b"new exe".to_vec()),
            ],
        }
    }

    fn target(dir: &tempfile::TempDir) -> Target {
        Target {
            dir: dir.path().to_path_buf(),
        }
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".old") || n.ends_with(".new"))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn detects_managed_install_by_marker() {
        let dir = install_dir();
        let exe = dir.path().join("nexpanel.exe");
        assert_eq!(detect_target(&exe).unwrap().dir, dir.path());
        // Marker with a UTF-8 BOM (Windows PowerShell 5.1 writes one).
        let marker = fs::read(dir.path().join(MARKER_FILE)).unwrap();
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend(marker);
        fs::write(dir.path().join(MARKER_FILE), bom).unwrap();
        assert!(detect_target(&exe).is_ok());
    }

    #[test]
    fn unmanaged_installs_are_refused() {
        let dir = install_dir();
        let exe = dir.path().join("nexpanel.exe");
        let other = tempfile::tempdir().unwrap();
        // No marker.
        assert!(matches!(
            detect_target(&other.path().join("nexpanel.exe")),
            Err(InstallError::NotManaged(_))
        ));
        // Marker of another directory (copied folder).
        fs::copy(dir.path().join(MARKER_FILE), other.path().join(MARKER_FILE)).unwrap();
        assert!(matches!(
            detect_target(&other.path().join("nexpanel.exe")),
            Err(InstallError::NotManaged(_))
        ));
        // Wrong product, broken JSON, wrong exe name.
        write(
            dir.path(),
            MARKER_FILE,
            r#"{"product":"Other","directory":"x"}"#,
        );
        assert!(detect_target(&exe).is_err());
        write(dir.path(), MARKER_FILE, "{ nope");
        assert!(detect_target(&exe).is_err());
        let dev = dir.path().join("something-else.exe");
        assert!(detect_target(&dev).is_err());
    }

    #[test]
    fn path_comparison_ignores_case_slashes_and_verbatim_prefix() {
        assert!(same_path(
            "C:\\Users\\A\\NexPanel\\bin",
            "c:/users/a/nexpanel/bin/"
        ));
        assert!(same_path("\\\\?\\C:\\x", "C:\\X"));
        assert!(!same_path("C:\\x", "C:\\y"));
        assert!(!same_path("", ""));
    }

    #[test]
    fn replaces_files_keeps_exe_backup_and_updates_marker() {
        let dir = install_dir();
        let p = dir.path();
        apply(&target(&dir), &package("0.2.0")).unwrap();
        assert_eq!(read(p, "nexpanel.exe"), "new exe");
        assert_eq!(read(p, "VERSION.txt"), "0.2.0");
        assert_eq!(read(p, "README.md"), "new readme");
        assert_eq!(read(p, "THIRD-PARTY-NOTICES.md"), "new notices");
        // Untouched: not part of the package.
        assert_eq!(read(p, "install.ps1"), "old install");
        // Only the exe backup stays (the running exe cannot be deleted).
        assert_eq!(leftovers(p), ["nexpanel.exe.old"]);
        assert_eq!(read(p, "nexpanel.exe.old"), "old exe");
        let marker: serde_json::Value = serde_json::from_str(&read(p, MARKER_FILE)).unwrap();
        assert_eq!(marker["version"], "0.2.0");
        assert_eq!(marker["product"], "NexPanel");
        // The updated directory is still a managed install.
        assert!(detect_target(&p.join("nexpanel.exe")).is_ok());
        // Next start removes the backup.
        assert_eq!(cleanup_old(p), 1);
        assert!(leftovers(p).is_empty());
    }

    #[test]
    fn second_update_replaces_stale_old_exe() {
        let dir = install_dir();
        let p = dir.path();
        write(p, "nexpanel.exe.old", "ancient");
        apply(&target(&dir), &package("0.2.0")).unwrap();
        assert_eq!(read(p, "nexpanel.exe.old"), "old exe");
    }

    #[test]
    fn failure_at_the_exe_rolls_back_every_file() {
        let dir = install_dir();
        let p = dir.path();
        // A non-empty directory squats on the backup name: the exe step (last) fails after
        // the other files were already swapped.
        fs::create_dir(p.join("nexpanel.exe.old")).unwrap();
        fs::write(p.join("nexpanel.exe.old").join("blocker"), "x").unwrap();
        let err = apply(&target(&dir), &package("0.2.0")).unwrap_err();
        assert!(
            matches!(
                err,
                InstallError::Failed(_) | InstallError::NoWritePermission(_)
            ),
            "{err:?}"
        );
        assert_eq!(read(p, "nexpanel.exe"), "old exe");
        assert_eq!(read(p, "VERSION.txt"), "0.1.0");
        assert_eq!(read(p, "README.md"), "old readme");
        assert!(
            !p.join("THIRD-PARTY-NOTICES.md").exists(),
            "file that did not exist is removed again"
        );
        assert_eq!(read(p, "install.ps1"), "old install");
        let marker: serde_json::Value = serde_json::from_str(&read(p, MARKER_FILE)).unwrap();
        assert_eq!(marker["version"], "0.1.0");
        assert_eq!(
            leftovers(p),
            ["nexpanel.exe.old"],
            "only the squatter remains"
        );
        assert!(p.join("nexpanel.exe.old").is_dir());
    }

    #[test]
    fn failure_while_writing_changes_nothing() {
        let dir = install_dir();
        let p = dir.path();
        // `VERSION.txt.new` cannot be created because a directory has that name.
        fs::create_dir(p.join("VERSION.txt.new")).unwrap();
        assert!(apply(&target(&dir), &package("0.2.0")).is_err());
        assert_eq!(read(p, "nexpanel.exe"), "old exe");
        assert_eq!(read(p, "VERSION.txt"), "0.1.0");
        assert_eq!(read(p, "README.md"), "old readme");
        assert_eq!(
            leftovers(p),
            ["VERSION.txt.new"],
            "written .new files were removed"
        );
    }

    #[test]
    fn unknown_or_incomplete_packages_are_refused_untouched() {
        let dir = install_dir();
        let p = dir.path();
        let mut hostile = package("0.2.0");
        hostile.files.push(("..\\evil.dll".into(), b"x".to_vec()));
        assert!(apply(&target(&dir), &hostile).is_err());
        let mut no_exe = package("0.2.0");
        no_exe.files.retain(|(n, _)| n != "nexpanel.exe");
        assert!(apply(&target(&dir), &no_exe).is_err());
        assert_eq!(read(p, "nexpanel.exe"), "old exe");
        assert!(leftovers(p).is_empty());
        assert!(!p.parent().unwrap().join("evil.dll").exists());
    }

    #[test]
    fn missing_write_permission_maps_to_friendly_error() {
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert!(matches!(
            classify("x", &denied),
            InstallError::NoWritePermission(_)
        ));
        let other = io::Error::other("disk");
        assert!(matches!(classify("x", &other), InstallError::Failed(_)));
        let update: UpdateError = InstallError::NoWritePermission("x".into()).into();
        assert_eq!(update.kind, ErrorKind::NoWritePermission);
        let update: UpdateError = InstallError::NotManaged("x".into()).into();
        assert_eq!(update.kind, ErrorKind::NotInstalled);
    }

    #[test]
    fn cleanup_ignores_missing_and_unrelated_files() {
        let dir = install_dir();
        let p = dir.path();
        write(p, "settings.json.old", "mine");
        write(p, "README.md.new", "x");
        assert_eq!(cleanup_old(p), 1);
        assert!(p.join("settings.json.old").exists());
        assert_eq!(cleanup_old(p), 0);
    }
}
