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
#[serde(default, deny_unknown_fields)]
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
