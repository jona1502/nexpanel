use serde::{Deserialize, Serialize};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    #[default]
    Blue,
    Purple,
    Green,
    Orange,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// Unknown fields are ignored so a downgrade does not block on newer settings files.
#[serde(default)]
pub struct Config {
    pub accent: Accent,
    pub animations: bool,
    pub close_on_focus_loss: bool,
    pub linked_displays: bool,
    pub autostart: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            accent: Accent::Blue,
            animations: true,
            close_on_focus_loss: true,
            linked_displays: false,
            autostart: false,
        }
    }
}
impl Config {
    /// All settings are bounded enums or booleans; deserialization validates values.
    pub fn validate(&self) -> io::Result<()> {
        Ok(())
    }
}
/// Resolve on a background worker; no filesystem access occurs here.
pub fn default_path() -> io::Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA fehlt"))?;
    Ok(PathBuf::from(base).join("NexPanel").join("settings.json"))
}
/// A malformed existing file is an error, never silently replaced.
pub fn load(path: &Path) -> io::Result<Config> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(error) => return Err(error),
    };
    let config: Config = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    config.validate()?;
    Ok(config)
}
/// Copy a damaged settings file to `settings.corrupt-<unix-ts>.json` next to it.
/// Returns `None` if there is nothing to back up.
pub fn backup_corrupt(path: &Path) -> io::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let stem = path
        .file_stem()
        .map_or_else(|| "settings".into(), |s| s.to_string_lossy().into_owned());
    let mut target = path.with_file_name(format!("{stem}.corrupt-{ts}.json"));
    let mut n = 1;
    while target.exists() {
        target = path.with_file_name(format!("{stem}.corrupt-{ts}-{n}.json"));
        n += 1;
    }
    std::fs::copy(path, &target)?;
    Ok(Some(target))
}
/// Explicit save also serves as the user's deliberate reset of a damaged file.
pub fn save(path: &Path, config: &Config) -> io::Result<()> {
    config.validate()?;
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(file.as_file_mut(), config)?;
    file.write_all(b"\n")?;
    file.as_file_mut().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_config_uses_valid_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = load(&dir.path().join("settings.json")).unwrap();
        assert!(!config.autostart);
        assert!(config.validate().is_ok());
    }
    #[test]
    fn save_replaces_previous_complete_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/settings.json");
        save(&path, &Config::default()).unwrap();
        let config = Config {
            autostart: true,
            accent: Accent::Green,
            ..Config::default()
        };
        save(&path, &config).unwrap();
        assert_eq!(load(&path).unwrap(), config);
    }
    #[test]
    fn unknown_fields_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, br#"{"autostart":true,"future_option":1}"#).unwrap();
        let config = load(&path).unwrap();
        assert!(config.autostart);
    }
    #[test]
    fn backup_copies_corrupt_file_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(backup_corrupt(&path).unwrap(), None);
        std::fs::write(&path, b"broken").unwrap();
        let a = backup_corrupt(&path).unwrap().unwrap();
        let b = backup_corrupt(&path).unwrap().unwrap();
        assert_ne!(a, b);
        assert_eq!(std::fs::read(&a).unwrap(), b"broken");
        assert!(
            a.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("settings.corrupt-")
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"broken");
    }
    #[test]
    fn corrupt_or_unknown_settings_are_preserved_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        for bytes in [
            b"not json".as_slice(),
            br#"{"accent":"unknown"}"#.as_slice(),
            br#"{"autostart":"yes"}"#.as_slice(),
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(load(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
}
