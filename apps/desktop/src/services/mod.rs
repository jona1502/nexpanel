//! Plain Send data crosses worker boundaries; native resources stay on workers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayDevice {
    pub id: String,
    pub name: String,
    pub brightness: Option<u8>,
    pub status: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DisplaySnapshot {
    pub devices: Vec<DisplayDevice>,
    pub status: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayCommand {
    Refresh,
    // Windows reported a display/device change; the worker debounces and re-reads the topology.
    TopologyChanged,
    SetBrightness { id: String, value: u8 },
    SetLinkedBrightness(u8),
    SetActive(bool),
    Shutdown,
}
/// App icon as RGBA8 pixels. Equality is the cache key (hash of the exe path), so change-only
/// snapshot sending never compares pixel data.
#[derive(Clone)]
pub struct AppIcon {
    pub key: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: std::sync::Arc<[u8]>,
}
impl PartialEq for AppIcon {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl Eq for AppIcon {}
impl std::fmt::Debug for AppIcon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AppIcon({:x}, {}x{})", self.key, self.width, self.height)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioRow {
    pub id: String,
    pub name: String,
    pub volume: u8,
    pub muted: bool,
    /// None for system sounds and when extraction failed (UI shows a letter tile).
    pub icon: Option<AppIcon>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioSnapshot {
    pub master: Option<AudioRow>,
    pub sessions: Vec<AudioRow>,
    /// Friendly name of the default output device; empty when unknown.
    pub device: String,
    pub status: String,
}
/// Coarse change signal from Core Audio callbacks (sent from foreign COM threads).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioChange {
    /// Default device, or the set/state of devices, changed.
    Device,
    /// Session created, state changed or disconnected.
    Sessions,
    /// Volume or mute of an existing session changed.
    SessionValues,
    /// Master volume or mute changed.
    Endpoint,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioCommand {
    Refresh,
    SetVolume {
        id: String,
        value: u8,
    },
    SetMute {
        id: String,
        muted: bool,
    },
    SetActive(bool),
    /// Sent by Core Audio callbacks, not by the UI.
    Changed(AudioChange),
    Shutdown,
}

pub mod audio;
pub mod display;
