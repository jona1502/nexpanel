//! Background thread that runs the update state machine. The UI thread only sends commands
//! and reads snapshots; every network and file operation happens here.

use super::{
    engine::{self, Package, Stage, Transport},
    install,
    machine::{Effect, ErrorKind, Event, Machine, State, UpdateError},
    release::Release,
    version::Version,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Manual or scheduled check; ignored while busy.
    Check,
    /// The user clicked "Jetzt aktualisieren".
    Install,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub state: State,
    /// This exe is a managed install that can be replaced.
    pub can_install: bool,
    /// The new version is installed and started; the UI quits the app now.
    pub quit: bool,
}

/// Everything that touches the installation. The real one works on the install directory,
/// tests use a mock.
pub trait Installer {
    /// Whether this installation may be replaced (marker present and matching).
    fn detect(&self) -> Result<(), UpdateError>;
    /// Removes `.old`/`.new` leftovers of an earlier update.
    fn cleanup(&self);
    fn install(&self, package: &Package) -> Result<(), UpdateError>;
    /// Starts the new exe; it waits for this process to exit.
    fn restart(&self) -> Result<(), UpdateError>;
}

/// Installer for the running exe.
pub struct FsInstaller {
    exe: Option<PathBuf>,
}

impl FsInstaller {
    pub fn new(exe: Option<PathBuf>) -> Self {
        Self { exe }
    }

    fn target(&self) -> Result<install::Target, UpdateError> {
        let exe = self.exe.as_deref().ok_or_else(|| {
            UpdateError::new(ErrorKind::NotInstalled, "Pfad der Anwendung unbekannt")
        })?;
        Ok(install::detect_target(exe)?)
    }
}

impl Installer for FsInstaller {
    fn detect(&self) -> Result<(), UpdateError> {
        self.target().map(|_| ())
    }

    fn cleanup(&self) {
        if let Ok(target) = self.target() {
            let removed = install::cleanup_old(&target.dir);
            if removed > 0 {
                log::info!("Update: {removed} Reste einer früheren Aktualisierung entfernt");
            }
        }
    }

    fn install(&self, package: &Package) -> Result<(), UpdateError> {
        Ok(install::apply(&self.target()?, package)?)
    }

    fn restart(&self) -> Result<(), UpdateError> {
        install::spawn_new_instance(&self.target()?).map_err(|e| {
            UpdateError::new(
                ErrorKind::RestartFailed,
                format!("neue Version konnte nicht gestartet werden: {e}"),
            )
        })
    }
}

pub struct Handle {
    pub tx: Sender<Command>,
    pub rx: Receiver<Snapshot>,
    /// Set on application exit so a running download stops early.
    pub cancel: Arc<AtomicBool>,
}

type TransportResult = Result<Box<dyn Transport>, UpdateError>;

/// Starts the worker. `make_transport` runs on the worker thread (WinHTTP handles stay there).
pub fn spawn(
    current: Version,
    include_prerelease: bool,
    make_transport: impl FnOnce() -> TransportResult + Send + 'static,
    installer: impl Installer + Send + 'static,
    crashed: &crate::logging::CrashFlag,
) -> Handle {
    let (tx, commands) = mpsc::channel();
    let (out, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    crate::logging::spawn_worker("nexpanel-update", crashed, move || {
        run(
            &commands,
            &out,
            &worker_cancel,
            &current,
            include_prerelease,
            make_transport,
            &installer,
        );
    });
    Handle { tx, rx, cancel }
}

/// Download progress is sent at most this often (plus on every whole-percent change).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

fn run(
    commands: &Receiver<Command>,
    out: &Sender<Snapshot>,
    cancel: &AtomicBool,
    current: &Version,
    include_prerelease: bool,
    make_transport: impl FnOnce() -> TransportResult,
    installer: &dyn Installer,
) {
    installer.cleanup();
    let can_install = match installer.detect() {
        Ok(()) => true,
        Err(e) => {
            log::info!("Update: Installation nicht ersetzbar ({e}); nur Prüfen möglich");
            false
        }
    };
    let transport = make_transport();
    let mut machine = Machine::default();
    let send = |machine: &Machine, quit: bool| {
        let _ = out.send(Snapshot {
            state: machine.state().clone(),
            can_install,
            quit,
        });
    };
    send(&machine, false);
    while let Ok(command) = commands.recv() {
        match command {
            Command::Shutdown => break,
            Command::Check => {
                if machine.handle(Event::CheckRequested) != Effect::StartCheck {
                    continue;
                }
                send(&machine, false);
                let result = match &transport {
                    Ok(t) => engine::check(t.as_ref(), current, include_prerelease),
                    Err(e) => Err(e.clone()),
                };
                match &result {
                    Ok(Some(r)) => log::info!("Update: Version {} verfügbar", r.version),
                    Ok(None) => log::info!("Update: aktuell ({current})"),
                    Err(e) => log::warn!("Update: Prüfung fehlgeschlagen: {e}"),
                }
                machine.handle(Event::CheckFinished(result));
                send(&machine, false);
            }
            Command::Install => {
                if let Effect::StartDownload(release) = machine.handle(Event::InstallRequested) {
                    send(&machine, false);
                    let quit = install_flow(
                        &mut machine,
                        &release,
                        &transport,
                        installer,
                        current,
                        cancel,
                        &send,
                    );
                    send(&machine, quit);
                }
            }
        }
    }
}

fn fail(machine: &mut Machine, error: UpdateError) {
    log::error!("Update fehlgeschlagen ({:?}): {error}", error.kind);
    machine.handle(Event::Failed(error));
}

/// Download, verify, install, restart. Returns `true` once the new version was started.
fn install_flow(
    machine: &mut Machine,
    release: &Release,
    transport: &TransportResult,
    installer: &dyn Installer,
    current: &Version,
    cancel: &AtomicBool,
    send: &dyn Fn(&Machine, bool),
) -> bool {
    log::info!("Update: Installation von {} gestartet", release.version);
    // Before downloading anything: an installation that cannot be replaced fails early.
    if let Err(e) = installer.detect() {
        fail(machine, e);
        return false;
    }
    let transport = match transport {
        Ok(t) => t.as_ref(),
        Err(e) => {
            fail(machine, e.clone());
            return false;
        }
    };
    let mut last_sent = (Instant::now(), 255u8);
    let package = engine::download_package(transport, current, release, &mut |stage| {
        match stage {
            Stage::Downloading { done, total } => {
                machine.handle(Event::Progress { done, total });
                let percent = machine.state().percent().unwrap_or(0);
                if percent != last_sent.1 || last_sent.0.elapsed() >= PROGRESS_INTERVAL {
                    last_sent = (Instant::now(), percent);
                    send(machine, false);
                }
            }
            Stage::Verifying => {
                machine.handle(Event::Downloaded);
                send(machine, false);
            }
        }
        !cancel.load(Ordering::Relaxed)
    });
    let package = match package {
        Ok(p) => p,
        Err(e) => {
            fail(machine, e);
            return false;
        }
    };
    machine.handle(Event::Verified);
    send(machine, false);
    machine.handle(Event::InstallStarted);
    send(machine, false);
    if let Err(e) = installer.install(&package) {
        fail(machine, e);
        return false;
    }
    log::info!(
        "Update: Version {} installiert, starte neu",
        package.version
    );
    if machine.handle(Event::Installed) != Effect::Restart {
        return false;
    }
    if let Err(e) = installer.restart() {
        fail(machine, e);
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::{
        engine::tests::MockTransport,
        release::{RELEASES_API_URL, package_name, test_release},
        sha256,
        zip::testutil::{Method, build},
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockInstaller {
        managed: bool,
        fail_install: bool,
        fail_restart: bool,
        calls: Mutex<Vec<&'static str>>,
    }

    impl Installer for &MockInstaller {
        fn detect(&self) -> Result<(), UpdateError> {
            self.calls.lock().unwrap().push("detect");
            if self.managed {
                Ok(())
            } else {
                Err(UpdateError::new(ErrorKind::NotInstalled, "dev build"))
            }
        }
        fn cleanup(&self) {
            self.calls.lock().unwrap().push("cleanup");
        }
        fn install(&self, _: &Package) -> Result<(), UpdateError> {
            self.calls.lock().unwrap().push("install");
            if self.fail_install {
                Err(UpdateError::new(ErrorKind::NoWritePermission, "denied"))
            } else {
                Ok(())
            }
        }
        fn restart(&self) -> Result<(), UpdateError> {
            self.calls.lock().unwrap().push("restart");
            if self.fail_restart {
                Err(UpdateError::new(ErrorKind::RestartFailed, "spawn"))
            } else {
                Ok(())
            }
        }
    }

    fn installs(m: &MockInstaller) -> usize {
        m.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| **c == "install")
            .count()
    }

    fn package_zip(version: &str) -> Vec<u8> {
        let mut exe = b"MZ".to_vec();
        exe.resize(4096, 1);
        build(&[
            ("nexpanel.exe", &exe, Method::Deflate),
            ("VERSION.txt", version.as_bytes(), Method::Stored),
        ])
    }

    fn api_body(tag: &str) -> Vec<u8> {
        let name = package_name(&tag[1..]);
        format!(
            r#"[{{"tag_name":"{tag}","draft":false,"prerelease":true,"assets":[{{"name":"{name}","size":10}},{{"name":"{name}.sha256","size":10}}]}}]"#
        )
        .into_bytes()
    }

    /// Transport that serves a release `tag` whose package says `zip_version`.
    fn transport(tag: &str, zip_version: &str, tamper: bool) -> MockTransport {
        let release = test_release(tag);
        let zip = package_zip(zip_version);
        let hash = format!("{}  {}", sha256::hex_digest(&zip), release.zip.name);
        let served = if tamper {
            let mut z = zip.clone();
            let mid = z.len() / 2;
            z[mid] ^= 1;
            z
        } else {
            zip
        };
        MockTransport::default()
            .with(RELEASES_API_URL, Ok(api_body(tag)))
            .with(&release.sha256.url, Ok(hash.into_bytes()))
            .with(&release.zip.url, Ok(served))
    }

    /// Runs the worker over `commands` and returns the snapshots it produced.
    fn drive(commands: &[Command], t: MockTransport, installer: &MockInstaller) -> Vec<Snapshot> {
        let (tx, rx) = mpsc::channel();
        let (out, snapshots) = mpsc::channel();
        for c in commands {
            tx.send(*c).unwrap();
        }
        tx.send(Command::Shutdown).unwrap();
        let cancel = AtomicBool::new(false);
        run(
            &rx,
            &out,
            &cancel,
            &Version::parse("0.1.0").unwrap(),
            true,
            move || Ok(Box::new(t) as Box<dyn Transport>),
            &installer,
        );
        drop(out);
        snapshots.iter().collect()
    }

    fn codes(snapshots: &[Snapshot]) -> Vec<i32> {
        snapshots.iter().map(|s| s.state.code()).collect()
    }

    #[test]
    fn check_alone_never_downloads_or_installs() {
        let installer = MockInstaller {
            managed: true,
            ..Default::default()
        };
        let t = transport("v0.2.0", "0.2.0", false);
        let snaps = drive(&[Command::Check], t, &installer);
        // Idle, Checking, Available
        assert_eq!(codes(&snaps), [0, 1, 3]);
        assert!(snaps.iter().all(|s| !s.quit));
        assert_eq!(installs(&installer), 0);
        assert!(!installer.calls.lock().unwrap().contains(&"restart"));
    }

    #[test]
    fn up_to_date_and_failed_checks() {
        let installer = MockInstaller {
            managed: true,
            ..Default::default()
        };
        let t = transport("v0.1.0", "0.1.0", false);
        assert_eq!(codes(&drive(&[Command::Check], t, &installer)), [0, 1, 2]);
        let offline = MockTransport::default();
        let snaps = drive(&[Command::Check], offline, &installer);
        assert_eq!(codes(&snaps), [0, 1, 8]);
        assert_eq!(snaps[2].state.error().unwrap().kind, ErrorKind::Network);
    }

    #[test]
    fn click_runs_download_verify_install_restart_once() {
        let installer = MockInstaller {
            managed: true,
            ..Default::default()
        };
        let t = transport("v0.2.0", "0.2.0", false);
        let snaps = drive(&[Command::Check, Command::Install], t, &installer);
        let last = snaps.last().unwrap();
        assert!(last.quit, "app must quit after the new version started");
        assert_eq!(last.state.code(), 7);
        let states = codes(&snaps);
        for expected in [4, 5, 6, 7] {
            assert!(states.contains(&expected), "{states:?}");
        }
        let calls = installer.calls.lock().unwrap();
        assert_eq!(calls.iter().filter(|c| **c == "install").count(), 1);
        assert_eq!(calls.iter().filter(|c| **c == "restart").count(), 1);
        assert!(
            calls.iter().position(|c| *c == "install") < calls.iter().position(|c| *c == "restart")
        );
    }

    #[test]
    fn install_without_a_known_update_does_nothing() {
        let installer = MockInstaller {
            managed: true,
            ..Default::default()
        };
        let t = transport("v0.2.0", "0.2.0", false);
        let snaps = drive(&[Command::Install], t, &installer);
        assert_eq!(codes(&snaps), [0]);
        assert_eq!(installs(&installer), 0);
    }

    #[test]
    fn tampered_download_is_never_installed() {
        let installer = MockInstaller {
            managed: true,
            ..Default::default()
        };
        let t = transport("v0.2.0", "0.2.0", true);
        let snaps = drive(&[Command::Check, Command::Install], t, &installer);
        let last = snaps.last().unwrap();
        assert_eq!(last.state.error().unwrap().kind, ErrorKind::HashMismatch);
        assert!(!last.quit);
        assert_eq!(installs(&installer), 0);
    }

    #[test]
    fn wrong_version_inside_package_is_never_installed() {
        let installer = MockInstaller {
            managed: true,
            ..Default::default()
        };
        let t = transport("v0.2.0", "0.1.0", false);
        let snaps = drive(&[Command::Check, Command::Install], t, &installer);
        assert_eq!(
            snaps.last().unwrap().state.error().unwrap().kind,
            ErrorKind::VersionMismatch
        );
        assert_eq!(installs(&installer), 0);
    }

    #[test]
    fn unmanaged_install_fails_before_downloading() {
        let installer = MockInstaller {
            managed: false,
            ..Default::default()
        };
        let t = transport("v0.2.0", "0.2.0", false);
        let snaps = drive(&[Command::Check, Command::Install], t, &installer);
        assert!(!snaps[0].can_install);
        assert_eq!(
            snaps.last().unwrap().state.error().unwrap().kind,
            ErrorKind::NotInstalled
        );
        assert_eq!(installs(&installer), 0);
    }

    #[test]
    fn install_and_restart_failures_are_reported() {
        let installer = MockInstaller {
            managed: true,
            fail_install: true,
            ..Default::default()
        };
        let snaps = drive(
            &[Command::Check, Command::Install],
            transport("v0.2.0", "0.2.0", false),
            &installer,
        );
        assert_eq!(
            snaps.last().unwrap().state.error().unwrap().kind,
            ErrorKind::NoWritePermission
        );
        assert!(!snaps.last().unwrap().quit);
        assert!(!installer.calls.lock().unwrap().contains(&"restart"));

        let installer = MockInstaller {
            managed: true,
            fail_restart: true,
            ..Default::default()
        };
        let snaps = drive(
            &[Command::Check, Command::Install],
            transport("v0.2.0", "0.2.0", false),
            &installer,
        );
        assert_eq!(
            snaps.last().unwrap().state.error().unwrap().kind,
            ErrorKind::RestartFailed
        );
        assert!(
            !snaps.last().unwrap().quit,
            "no quit when the new version did not start"
        );
    }

    #[test]
    fn cleanup_runs_first() {
        let installer = MockInstaller {
            managed: true,
            ..Default::default()
        };
        drive(&[], transport("v0.1.0", "0.1.0", false), &installer);
        assert_eq!(installer.calls.lock().unwrap()[0], "cleanup");
    }

    #[test]
    fn cancel_flag_aborts_the_download() {
        let installer = MockInstaller {
            managed: true,
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();
        let (out, snapshots) = mpsc::channel();
        tx.send(Command::Check).unwrap();
        tx.send(Command::Install).unwrap();
        tx.send(Command::Shutdown).unwrap();
        let cancel = AtomicBool::new(true);
        run(
            &rx,
            &out,
            &cancel,
            &Version::parse("0.1.0").unwrap(),
            true,
            || Ok(Box::new(transport("v0.2.0", "0.2.0", false)) as Box<dyn Transport>),
            &&installer,
        );
        drop(out);
        let snaps: Vec<_> = snapshots.iter().collect();
        assert_eq!(snaps.last().unwrap().state.code(), 8);
        assert_eq!(installs(&installer), 0);
    }
}
