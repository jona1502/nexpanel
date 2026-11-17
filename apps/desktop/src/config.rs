use crate::hotkey::{Hotkey, MOD_ALT, MOD_CONTROL, MOD_WIN, VK_SPACE};
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
/// Global hotkey that opens NexSearch. PowerToys Run and others often own Alt+Space, so there
/// are alternatives; changing it re-registers at runtime.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchHotkey {
    #[default]
    AltSpace,
    CtrlAltSpace,
    WinAltSpace,
}
impl SearchHotkey {
    /// Order of the entries in the settings dropdown.
    pub const ALL: [SearchHotkey; 3] = [Self::AltSpace, Self::CtrlAltSpace, Self::WinAltSpace];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|h| *h == self).unwrap_or(0)
    }
    /// Out-of-range indices fall back to the default.
    pub fn from_index(index: i32) -> Self {
        usize::try_from(index)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
            .unwrap_or_default()
    }
    pub fn hotkey(self) -> Hotkey {
        let modifiers = match self {
            Self::AltSpace => MOD_ALT,
            Self::CtrlAltSpace => MOD_CONTROL | MOD_ALT,
            Self::WinAltSpace => MOD_WIN | MOD_ALT,
        };
        Hotkey {
            modifiers,
            vk: VK_SPACE,
        }
    }
}
/// Where the search window opens on the monitor under the cursor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchPosition {
    /// Search bar in the vertical middle of the work area.
    #[default]
    Center,
    /// Close to the top edge of the work area.
    Top,
}
impl SearchPosition {
    pub const ALL: [SearchPosition; 2] = [Self::Center, Self::Top];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }
    pub fn from_index(index: i32) -> Self {
        usize::try_from(index)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
            .unwrap_or_default()
    }
}
/// Unknown values (typo, newer version) fall back to the default instead of making the whole
/// file unreadable.
fn lenient_enum<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}
/// Number of tabs in the panel; `last_tab` is always below this.
pub const TAB_COUNT: u8 = 5;
/// Any value that is not an integer in `0..TAB_COUNT` (negative, huge, text) becomes 0
/// (Audio) instead of making the whole file unreadable.
fn lenient_tab<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u8, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_u64()
        .and_then(|n| u8::try_from(n).ok())
        .filter(|n| *n < TAB_COUNT)
        .unwrap_or(0))
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
    /// Tab that was open when the panel was last closed (0 = Audio).
    #[serde(deserialize_with = "lenient_tab")]
    pub last_tab: u8,
    /// Global hotkey for NexSearch (default Alt+Space).
    #[serde(deserialize_with = "lenient_enum")]
    pub search_hotkey: SearchHotkey,
    /// Where the NexSearch window opens (default: vertically centered).
    #[serde(deserialize_with = "lenient_enum")]
    pub search_position: SearchPosition,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            accent: Accent::Blue,
            animations: true,
            close_on_focus_loss: true,
            linked_displays: false,
            autostart: false,
            last_tab: 0,
            search_hotkey: SearchHotkey::default(),
            search_position: SearchPosition::default(),
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
    fn last_tab_roundtrips_and_defaults_to_audio() {
        assert_eq!(Config::default().last_tab, 0);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let config = Config {
            last_tab: 3,
            ..Config::default()
        };
        save(&path, &config).unwrap();
        assert_eq!(load(&path).unwrap().last_tab, 3);
    }
    #[test]
    fn old_files_without_last_tab_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, br#"{"autostart":true,"accent":"green"}"#).unwrap();
        let config = load(&path).unwrap();
        assert!(config.autostart);
        assert_eq!(config.last_tab, 0);
    }
    #[test]
    fn invalid_last_tab_falls_back_to_audio() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        for raw in ["5", "255", "300", "-1", "2.5", "\"x\"", "null", "[1]"] {
            std::fs::write(&path, format!(r#"{{"autostart":true,"last_tab":{raw}}}"#)).unwrap();
            let config = load(&path).unwrap();
            assert_eq!(config.last_tab, 0, "{raw}");
            assert!(config.autostart);
        }
        std::fs::write(&path, br#"{"last_tab":4}"#).unwrap();
        assert_eq!(load(&path).unwrap().last_tab, 4);
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
#[cfg(test)]
mod search_tests {
    use super::*;
    #[test]
    fn search_options_default_for_old_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, br#"{"autostart":true}"#).unwrap();
        let config = load(&path).unwrap();
        assert_eq!(config.search_hotkey, SearchHotkey::AltSpace);
        assert_eq!(config.search_position, SearchPosition::Center);
    }
    #[test]
    fn search_options_roundtrip_and_use_snake_case() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let config = Config {
            search_hotkey: SearchHotkey::WinAltSpace,
            search_position: SearchPosition::Top,
            ..Config::default()
        };
        save(&path, &config).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains(r#""search_hotkey": "win_alt_space""#),
            "{text}"
        );
        assert!(text.contains(r#""search_position": "top""#), "{text}");
        assert_eq!(load(&path).unwrap(), config);
    }
    #[test]
    fn unknown_search_values_fall_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        for raw in ["\"f13\"", "3", "null", "[1]"] {
            std::fs::write(
                &path,
                format!(r#"{{"autostart":true,"search_hotkey":{raw},"search_position":{raw}}}"#),
            )
            .unwrap();
            let config = load(&path).unwrap();
            assert_eq!(config.search_hotkey, SearchHotkey::AltSpace, "{raw}");
            assert_eq!(config.search_position, SearchPosition::Center, "{raw}");
            assert!(config.autostart);
        }
    }
    #[test]
    fn dropdown_indices_roundtrip_and_clamp() {
        for h in SearchHotkey::ALL {
            assert_eq!(SearchHotkey::from_index(h.index() as i32), h);
        }
        for p in SearchPosition::ALL {
            assert_eq!(SearchPosition::from_index(p.index() as i32), p);
        }
        assert_eq!(SearchHotkey::from_index(-1), SearchHotkey::AltSpace);
        assert_eq!(SearchHotkey::from_index(99), SearchHotkey::AltSpace);
        assert_eq!(SearchPosition::from_index(7), SearchPosition::Center);
    }
    #[test]
    fn hotkeys_are_distinct_and_labelled() {
        let labels: Vec<String> = SearchHotkey::ALL
            .iter()
            .map(|h| h.hotkey().label())
            .collect();
        assert_eq!(labels[0], "Alt+Leertaste");
        assert_eq!(labels[1], "Strg+Alt+Leertaste");
        assert_eq!(labels[2], "Win+Alt+Leertaste");
        let mut sorted = labels.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 3);
    }
}
