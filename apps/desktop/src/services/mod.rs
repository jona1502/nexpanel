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
    SetBrightness { id: String, value: u8 },
    SetLinkedBrightness(u8),
    SetActive(bool),
    Shutdown,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioRow {
    pub id: String,
    pub name: String,
    pub volume: u8,
    pub muted: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioSnapshot {
    pub master: Option<AudioRow>,
    pub sessions: Vec<AudioRow>,
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
