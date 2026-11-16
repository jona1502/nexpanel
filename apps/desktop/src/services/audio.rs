//! Core Audio interfaces never leave their owning worker thread.
use super::{AudioCommand, AudioRow, AudioSnapshot};
use std::{
    collections::BTreeMap,
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

struct Member<T> {
    instance: String,
    pid: u32,
    system: bool,
    name: String,
    volume: u8,
    muted: bool,
    control: T,
}
struct Group<T> {
    row: AudioRow,
    members: Vec<Member<T>>,
}
fn normalized(value: u8) -> u8 {
    value.min(100)
}
fn groups<T>(endpoint: &str, members: Vec<Member<T>>) -> Vec<Group<T>> {
    let mut grouped: BTreeMap<String, Group<T>> = BTreeMap::new();
    for member in members {
        let key = if member.system {
            "system".to_owned()
        } else {
            format!("pid:{}", member.pid)
        };
        let group = grouped.entry(key.clone()).or_insert_with(|| Group {
            row: AudioRow {
                id: format!("{endpoint}|{key}"),
                name: if member.system {
                    "Systemsounds".into()
                } else {
                    member.name.clone()
                },
                volume: 0,
                muted: true,
            },
            members: vec![],
        });
        // Instance identifiers validate the fresh enumeration and prevent duplicates.
        if group.members.iter().any(|m| m.instance == member.instance) {
            continue;
        }
        group.row.volume = group.row.volume.max(normalized(member.volume));
        group.row.muted &= member.muted;
        group.members.push(member);
    }
    grouped
        .into_values()
        .map(|mut group| {
            // Full, length-prefixed identifiers avoid hash collisions and remain
            // stable under enumeration reordering, but change with membership.
            let mut instances: Vec<_> = group.members.iter().map(|m| m.instance.as_str()).collect();
            instances.sort_unstable();
            group.row.id.push('|');
            for instance in instances {
                group
                    .row
                    .id
                    .push_str(&format!("{}:{instance}", instance.len()));
            }
            group
        })
        .collect()
}
fn target_indices<T>(groups: &[Group<T>], id: &str) -> Result<Vec<usize>, &'static str> {
    groups
        .iter()
        .find(|g| g.row.id == id)
        .map(|g| (0..g.members.len()).collect())
        .ok_or("Audio-Sitzung ist nicht mehr verfügbar.")
}
fn apply_group<T>(
    groups: &[Group<T>],
    id: &str,
    mut write: impl FnMut(&T) -> Result<(), String>,
) -> Result<(), String> {
    let indices = target_indices(groups, id).map_err(str::to_owned)?;
    let group = groups
        .iter()
        .find(|g| g.row.id == id)
        .expect("validated group");
    // Continue after individual errors so one disappearing session cannot skip others.
    let mut failure = None;
    for index in indices {
        if let Err(error) = write(&group.members[index].control) {
            failure = Some(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

pub fn spawn() -> (Sender<AudioCommand>, Receiver<AudioSnapshot>) {
    let (commands, input) = mpsc::channel();
    let (output, snapshots) = mpsc::channel();
    std::thread::Builder::new()
        .name("nexpanel-audio".into())
        .spawn(move || worker(input, output))
        .expect("Audio-Worker konnte nicht gestartet werden");
    (commands, snapshots)
}
pub fn diagnose() -> AudioSnapshot {
    native::diagnose()
}

fn worker(input: Receiver<AudioCommand>, output: Sender<AudioSnapshot>) {
    let mut active = true;
    let mut backend = match native::Backend::new() {
        Ok(backend) => backend,
        Err(error) => {
            let _ = output.send(AudioSnapshot {
                status: error,
                ..Default::default()
            });
            return;
        }
    };
    let mut last = backend.refresh();
    if output.send(last.clone()).is_err() {
        return;
    }
    loop {
        let interval = Duration::from_secs(if active { 1 } else { 5 });
        let first = match input.recv_timeout(interval) {
            Ok(command) => Some(command),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        let mut pending = Vec::new();
        if let Some(command) = first {
            pending.push(command);
        }
        for command in input.try_iter().take(63) {
            pending.push(command);
        }
        let mut writes = Vec::new();
        for command in pending {
            match command {
                AudioCommand::Shutdown => return,
                AudioCommand::SetActive(value) => active = value,
                AudioCommand::SetVolume { id, value } => {
                    // Last value for a target wins, while mute commands retain order.
                    writes
                        .retain(|c| !matches!(c, AudioCommand::SetVolume{id:old,..} if old == &id));
                    writes.push(AudioCommand::SetVolume {
                        id,
                        value: normalized(value),
                    });
                }
                AudioCommand::SetMute { .. } => writes.push(command),
                AudioCommand::Refresh => {}
            }
        }
        let snapshot = run_batch(
            &mut backend,
            writes,
            native::Backend::refresh,
            native::Backend::write,
        );
        // Unchanged snapshots would only make the UI redo identical work.
        if snapshot != last {
            if output.send(snapshot.clone()).is_err() {
                return;
            }
            last = snapshot;
        }
    }
}

fn run_batch<B>(
    backend: &mut B,
    writes: Vec<AudioCommand>,
    mut refresh: impl FnMut(&mut B) -> AudioSnapshot,
    mut write: impl FnMut(&mut B, AudioCommand) -> Result<(), String>,
) -> AudioSnapshot {
    let mut errors = Vec::new();
    for command in writes {
        if let Err(error) = write(backend, command) {
            errors.push(error);
        }
    }
    // Read after all attempts, including partial failures and endpoint removal.
    let mut snapshot = refresh(backend);
    if !errors.is_empty() {
        if !snapshot.status.is_empty() {
            snapshot.status.push_str("; ");
        }
        snapshot.status.push_str(&errors.join("; "));
    }
    snapshot
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows::{
        Win32::{
            Foundation::{CloseHandle, HANDLE, S_OK},
            Media::Audio::{Endpoints::IAudioEndpointVolume, *},
            System::{Com::*, Threading::*},
        },
        core::{Interface, PWSTR},
    };
    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    struct Allocation(PWSTR);
    impl Allocation {
        fn string(self) -> String {
            unsafe { self.0.to_string().unwrap_or_default() }
        }
    }
    impl Drop for Allocation {
        fn drop(&mut self) {
            unsafe {
                CoTaskMemFree(Some(self.0.0.cast()));
            }
        }
    }
    struct Process(HANDLE);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
    fn error(error: windows::core::Error) -> String {
        format!("Audio ist momentan nicht verfügbar: {error}")
    }
    fn process_name(pid: u32) -> Option<String> {
        let process =
            Process(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?);
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                process.0,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut length,
            )
        }
        .ok()?;
        let path = String::from_utf16_lossy(&buffer[..length as usize]);
        std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
    }
    struct State {
        id: String,
        endpoint: IAudioEndpointVolume,
        groups: Vec<Group<ISimpleAudioVolume>>,
    }
    pub(super) struct Backend {
        state: Option<State>,
        enumerator: IMMDeviceEnumerator,
        _com: Com,
    }
    impl Backend {
        pub(super) fn new() -> Result<Self, String> {
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }.map_err(error)?;
            let com = Com;
            let enumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                .map_err(error)?;
            Ok(Self {
                state: None,
                enumerator,
                _com: com,
            })
        }
        fn read(&self) -> windows::core::Result<(State, AudioSnapshot)> {
            unsafe {
                let device = self
                    .enumerator
                    .GetDefaultAudioEndpoint(eRender, eMultimedia)?;
                let id = Allocation(device.GetId()?).string();
                let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
                let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
                let sessions = manager.GetSessionEnumerator()?;
                let mut members = Vec::new();
                let mut skipped = 0;
                for index in 0..sessions.GetCount()? {
                    let result =
                        (|| -> windows::core::Result<Option<Member<ISimpleAudioVolume>>> {
                            let control = sessions.GetSession(index)?;
                            if control.GetState()? == AudioSessionStateExpired {
                                return Ok(None);
                            }
                            let extended: IAudioSessionControl2 = control.cast()?;
                            let volume: ISimpleAudioVolume = control.cast()?;
                            let instance =
                                Allocation(extended.GetSessionInstanceIdentifier()?).string();
                            if instance.is_empty() {
                                return Ok(None);
                            }
                            let system = extended.IsSystemSoundsSession() == S_OK;
                            let pid = extended.GetProcessId()?;
                            let display = control
                                .GetDisplayName()
                                .map(|s| Allocation(s).string())
                                .unwrap_or_default();
                            let name = process_name(pid)
                                .filter(|s| !s.is_empty())
                                .or_else(|| (!display.trim().is_empty()).then_some(display))
                                .unwrap_or_else(|| format!("Anwendung {pid}"));
                            Ok(Some(Member {
                                instance,
                                pid,
                                system,
                                name,
                                volume: percent(volume.GetMasterVolume()?),
                                muted: volume.GetMute()?.as_bool(),
                                control: volume,
                            }))
                        })();
                    match result {
                        Ok(Some(member)) => members.push(member),
                        Ok(None) => {}
                        Err(_) => skipped += 1,
                    }
                }
                let groups = groups(&id, members);
                let snapshot = AudioSnapshot {
                    master: Some(AudioRow {
                        id: format!("{id}|master"),
                        name: "Standardausgabegerät".into(),
                        volume: percent(endpoint.GetMasterVolumeLevelScalar()?),
                        muted: endpoint.GetMute()?.as_bool(),
                    }),
                    sessions: groups.iter().map(|g| g.row.clone()).collect(),
                    status: if skipped == 0 {
                        String::new()
                    } else {
                        format!("{skipped} Audio-Sitzung(en) konnten nicht gelesen werden.")
                    },
                };
                Ok((
                    State {
                        id,
                        endpoint,
                        groups,
                    },
                    snapshot,
                ))
            }
        }
        pub(super) fn refresh(&mut self) -> AudioSnapshot {
            match self.read() {
                Ok((state, snapshot)) => {
                    self.state = Some(state);
                    snapshot
                }
                Err(e) => {
                    self.state = None;
                    AudioSnapshot {
                        status: error(e),
                        ..Default::default()
                    }
                }
            }
        }
        pub(super) fn write(&mut self, command: AudioCommand) -> Result<(), String> {
            // Reacquire immediately before each mutation: a queued ID never targets a new endpoint.
            let _ = self.refresh();
            let state = self
                .state
                .as_ref()
                .ok_or("Kein Standardausgabegerät verfügbar.")?;
            let (id, volume, mute) = match command {
                AudioCommand::SetVolume { id, value } => (id, Some(normalized(value)), None),
                AudioCommand::SetMute { id, muted } => (id, None, Some(muted)),
                _ => return Ok(()),
            };
            if id == format!("{}|master", state.id) {
                unsafe {
                    if let Some(value) = volume {
                        state
                            .endpoint
                            .SetMasterVolumeLevelScalar(f32::from(value) / 100., std::ptr::null())
                    } else {
                        state
                            .endpoint
                            .SetMute(mute.unwrap_or(false), std::ptr::null())
                    }
                }
                .map_err(error)
            } else {
                apply_group(&state.groups, &id, |control| {
                    unsafe {
                        if let Some(value) = volume {
                            control.SetMasterVolume(f32::from(value) / 100., std::ptr::null())
                        } else {
                            control.SetMute(mute.unwrap_or(false), std::ptr::null())
                        }
                    }
                    .map_err(error)
                })
            }
        }
    }
    fn percent(value: f32) -> u8 {
        (value.clamp(0., 1.) * 100.).round() as u8
    }
    pub(super) fn diagnose() -> AudioSnapshot {
        match Backend::new() {
            Ok(mut backend) => backend.refresh(),
            Err(status) => AudioSnapshot {
                status,
                ..Default::default()
            },
        }
    }
}
#[cfg(not(windows))]
mod native {
    use super::*;
    pub(super) struct Backend;
    impl Backend {
        pub(super) fn new() -> Result<Self, String> {
            Err("Audio-Steuerung benötigt Windows.".into())
        }
        pub(super) fn refresh(&mut self) -> AudioSnapshot {
            diagnose()
        }
        pub(super) fn write(&mut self, _: AudioCommand) -> Result<(), String> {
            Err("Audio-Steuerung benötigt Windows.".into())
        }
    }
    pub(super) fn diagnose() -> AudioSnapshot {
        AudioSnapshot {
            status: "Audio-Steuerung benötigt Windows.".into(),
            ..Default::default()
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn member(instance: &str, pid: u32, volume: u8, muted: bool) -> Member<()> {
        Member {
            instance: instance.into(),
            pid,
            system: false,
            name: "App".into(),
            volume,
            muted,
            control: (),
        }
    }
    #[test]
    fn groups_same_pid_and_aggregates_max_and_all_muted() {
        let g = groups(
            "endpoint",
            vec![
                member("a", 7, 20, true),
                member("b", 7, 80, false),
                member("c", 8, 30, true),
            ],
        );
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].row.volume, 80);
        assert!(!g[0].row.muted);
        assert_eq!(g[0].members.len(), 2);
        assert!(g[1].row.muted);
    }
    #[test]
    fn grouped_commands_target_each_current_member_and_reject_stale_endpoint() {
        let g = groups(
            "endpoint",
            vec![member("a", 7, 20, true), member("b", 7, 80, false)],
        );
        assert_eq!(target_indices(&g, &g[0].row.id), Ok(vec![0, 1]));
        assert!(target_indices(&g, "old|pid:7").is_err());
        assert!(target_indices(&g, "endpoint|pid:8").is_err());
        assert!(target_indices::<()>(&[], "endpoint|pid:7").is_err());
    }
    #[test]
    fn volume_percent_is_clamped() {
        assert_eq!(normalized(255), 100);
        assert_eq!(normalized(42), 42);
        assert_eq!(normalized(0), 0);
    }
    #[test]
    fn system_sounds_have_distinct_group() {
        let mut s = member("sys", 7, 50, true);
        s.system = true;
        let g = groups("e", vec![s, member("app", 7, 10, true)]);
        assert_eq!(g.len(), 2);
        assert!(g.iter().any(|g| g.row.name == "Systemsounds"));
    }
    #[test]
    fn grouped_write_uses_every_current_control_for_volume_and_mute() {
        use std::cell::Cell;
        let make = |id: &str| Member {
            instance: id.into(),
            pid: 9,
            system: false,
            name: "App".into(),
            volume: 1,
            muted: false,
            control: Cell::new((1u8, false)),
        };
        let g = groups("e", vec![make("a"), make("b")]);
        apply_group(&g, &g[0].row.id, |c| {
            c.set((normalized(255), true));
            Ok(())
        })
        .unwrap();
        assert_eq!(
            g[0].members
                .iter()
                .map(|m| m.control.get())
                .collect::<Vec<_>>(),
            vec![(100, true), (100, true)]
        );
        assert!(
            apply_group(&g, "stale|pid:9", |c| {
                c.set((0, false));
                Ok(())
            })
            .is_err()
        );
        assert!(g[0].members.iter().all(|m| m.control.get() == (100, true)));
    }
    #[test]
    fn grouped_write_continues_after_one_member_disappears() {
        use std::cell::Cell;
        let g = groups(
            "e",
            vec![member("a", 7, 20, false), member("b", 7, 20, false)],
        );
        let calls = Cell::new(0);
        assert!(
            apply_group(&g, &g[0].row.id, |_| {
                calls.set(calls.get() + 1);
                if calls.get() == 1 {
                    Err("gone".into())
                } else {
                    Ok(())
                }
            })
            .is_err()
        );
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn replaced_member_same_pid_rejects_previous_group_id() {
        let old = groups("e", vec![member("old-instance", 7, 20, false)]);
        let current = groups("e", vec![member("replacement", 7, 20, false)]);
        assert!(target_indices(&current, &old[0].row.id).is_err());
        let reversed = groups(
            "e",
            vec![member("b", 7, 20, false), member("a", 7, 20, false)],
        );
        let ordered = groups(
            "e",
            vec![member("a", 7, 20, false), member("b", 7, 20, false)],
        );
        assert_eq!(reversed[0].row.id, ordered[0].row.id);
    }
    #[test]
    fn batch_reports_refreshed_partial_group_write_and_keeps_read_status() {
        use std::cell::Cell;
        let make = |id: &str| Member {
            instance: id.into(),
            pid: 7,
            system: false,
            name: "App".into(),
            volume: 20,
            muted: false,
            control: Cell::new(20u8),
        };
        let mut current = groups("e", vec![make("a"), make("b")]);
        let id = current[0].row.id.clone();
        let snapshot = run_batch(
            &mut current,
            vec![AudioCommand::SetVolume { id, value: 80 }],
            |g| AudioSnapshot {
                sessions: g
                    .iter()
                    .map(|group| AudioRow {
                        volume: group
                            .members
                            .iter()
                            .map(|m| m.control.get())
                            .max()
                            .unwrap_or(0),
                        ..group.row.clone()
                    })
                    .collect(),
                status: "Lesestatus".into(),
                ..Default::default()
            },
            |g, command| {
                let AudioCommand::SetVolume { id, value } = command else {
                    unreachable!()
                };
                apply_group(g, &id, |control| {
                    if std::ptr::eq(control, &g[0].members[0].control) {
                        Err("Sitzung verschwunden".into())
                    } else {
                        control.set(value);
                        Ok(())
                    }
                })
            },
        );
        assert_eq!(snapshot.sessions[0].volume, 80);
        assert!(snapshot.status.contains("Lesestatus"));
        assert!(snapshot.status.contains("Sitzung verschwunden"));
    }
    #[test]
    fn batch_refresh_clears_removed_endpoint_after_write_error() {
        let mut current = AudioSnapshot {
            master: Some(AudioRow {
                id: "e|master".into(),
                name: "Master".into(),
                volume: 20,
                muted: false,
            }),
            ..Default::default()
        };
        let snapshot = run_batch(
            &mut current,
            vec![AudioCommand::SetMute {
                id: "e|master".into(),
                muted: true,
            }],
            |state| state.clone(),
            |state, _| {
                *state = AudioSnapshot {
                    status: "Kein Ausgabegerät".into(),
                    ..Default::default()
                };
                Err("Schreiben fehlgeschlagen".into())
            },
        );
        assert!(snapshot.master.is_none());
        assert!(snapshot.status.contains("Kein Ausgabegerät"));
        assert!(snapshot.status.contains("Schreiben fehlgeschlagen"));
    }
}
