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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioCommand {
    Refresh,
    SetVolume { id: String, value: u8 },
    SetMute { id: String, muted: bool },
    SetActive(bool),
    Shutdown,
}

pub mod display;
