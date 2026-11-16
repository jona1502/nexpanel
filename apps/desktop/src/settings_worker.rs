use crate::{
    config::{self, Config},
    platform,
};
use std::sync::mpsc::{self, Receiver, Sender};
pub enum Command {
    Save(Config),
    Reset,
    Shutdown,
}
pub struct Snapshot {
    pub config: Config,
    pub error: String,
    pub blocked: bool,
}
pub fn spawn() -> (
    Sender<Command>,
    Receiver<Snapshot>,
    std::thread::JoinHandle<Result<(), String>>,
) {
    spawn_with_path(config::default_path)
}
fn spawn_with_path(
    path: impl FnOnce() -> std::io::Result<std::path::PathBuf> + Send + 'static,
) -> (
    Sender<Command>,
    Receiver<Snapshot>,
    std::thread::JoinHandle<Result<(), String>>,
) {
    let (tx, rx) = mpsc::channel();
    let (stx, srx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let path = path();
        let loaded = path
            .as_ref()
            .map_err(|e| e.to_string())
            .and_then(|p| config::load(p).map_err(|e| e.to_string()));
        let mut blocked = loaded.is_err();
        let mut error=loaded.as_ref().err().map(|e|format!("Einstellungen nicht geladen: {e}. Datei bleibt erhalten. Zum Ersetzen zurücksetzen.")).unwrap_or_default();
        let mut current = loaded.unwrap_or_default();
        match platform::read_autostart() {
            Ok(v) => current.autostart = v,
            Err(e) => {
                error.push_str(&format!(" Autostart nicht lesbar: {e}"));
            }
        }
        match (&path, blocked) {
            (Ok(p), false) => log::info!("Einstellungen geladen: {}", p.display()),
            (_, true) => log::warn!("Einstellungen nicht geladen: {error}"),
            (Err(e), _) => log::warn!("Einstellungspfad unbekannt: {e}"),
        }
        let _ = stx.send(Snapshot {
            config: current.clone(),
            error,
            blocked,
        });
        let mut last_failure = None;
        while let Ok(cmd) = rx.recv() {
            let (reset, mut next) = match cmd {
                Command::Shutdown => break,
                Command::Save(c) => (false, c),
                Command::Reset => (true, Config::default()),
            };
            if !may_save(blocked, reset) {
                last_failure = Some(
                    "Speichern gesperrt: beschädigte Datei erst ausdrücklich zurücksetzen."
                        .to_string(),
                );
                let _ = stx.send(Snapshot {
                    config: current.clone(),
                    error: "Speichern gesperrt: beschädigte Datei erst ausdrücklich zurücksetzen."
                        .into(),
                    blocked,
                });
                continue;
            }
            if reset {
                next.autostart = current.autostart;
            }
            let changed = next.autostart != current.autostart;
            let result = (|| {
                let previous = if changed {
                    Some(platform::autostart_value()?)
                } else {
                    None
                };
                if changed {
                    platform::write_autostart(next.autostart)?;
                }
                let p = path
                    .as_ref()
                    .map_err(|e| std::io::Error::other(e.to_string()))?;
                if let Err(e) = save_file(p, &next, blocked, reset) {
                    if let Some(previous) = previous
                        && let Err(rollback) = platform::restore_autostart(previous)
                    {
                        if let Ok(actual) = platform::read_autostart() {
                            current.autostart = actual;
                        }
                        return Err(std::io::Error::other(format!(
                            "{e}; Autostart-Rücknahme fehlgeschlagen: {rollback}"
                        )));
                    }
                    return Err(e);
                }
                Ok::<_, std::io::Error>(())
            })();
            let error = match result {
                Ok(()) => {
                    current = next;
                    blocked = false;
                    String::new()
                }
                Err(e) => {
                    log::error!("Einstellungen speichern fehlgeschlagen: {e}");
                    format!("Einstellungen konnten nicht gespeichert werden: {e}")
                }
            };
            last_failure = if error.is_empty() {
                None
            } else {
                Some(error.clone())
            };
            let _ = stx.send(Snapshot {
                config: current.clone(),
                error,
                blocked,
            });
        }
        last_failure.map_or(Ok(()), Err)
    });
    (tx, srx, worker)
}

fn may_save(blocked: bool, reset: bool) -> bool {
    !blocked || reset
}
fn save_file(
    path: &std::path::Path,
    value: &Config,
    blocked: bool,
    reset: bool,
) -> std::io::Result<()> {
    if !may_save(blocked, reset) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Beschädigte Einstellungen erfordern ausdrückliches Zurücksetzen",
        ));
    }
    if reset && blocked {
        match config::backup_corrupt(path) {
            Ok(Some(backup)) => {
                log::info!("Beschädigte Einstellungen gesichert: {}", backup.display())
            }
            Ok(None) => {}
            Err(e) => {
                return Err(std::io::Error::other(format!(
                    "Sicherung der beschädigten Datei fehlgeschlagen: {e}"
                )));
            }
        }
    }
    config::save(path, value)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shutdown_ack_reports_queued_persistence_failure() {
        let dir = tempfile::tempdir().unwrap();
        let blocked_parent = dir.path().join("blocked");
        let path = blocked_parent.join("settings.json");
        let (tx, rx, worker) = spawn_with_path(move || Ok(path));
        let initial = rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
        std::fs::write(&blocked_parent, b"parent is a file").unwrap();
        tx.send(Command::Save(initial.config)).unwrap();
        tx.send(Command::Shutdown).unwrap();
        assert!(
            worker.join().unwrap().is_err(),
            "shutdown must acknowledge queued save failure even without UI polling"
        );
    }
    #[test]
    fn corrupt_bytes_survive_autosave_until_explicit_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, b"damaged configuration").unwrap();
        assert!(save_file(&path, &Config::default(), true, false).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"damaged configuration");
        save_file(&path, &Config::default(), true, true).unwrap();
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().contains("corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            std::fs::read(&backups[0]).unwrap(),
            b"damaged configuration"
        );
        assert_eq!(config::load(&path).unwrap(), Config::default());
    }
    #[test]
    fn damaged_config_requires_explicit_reset() {
        assert!(!may_save(true, false));
        assert!(may_save(true, true));
        assert!(may_save(false, false));
    }
}
