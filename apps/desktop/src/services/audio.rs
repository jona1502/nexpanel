//! Core Audio interfaces never leave their owning worker thread.
use super::{AudioChange, AudioCommand, AudioRow, AudioSnapshot};
use std::{
    collections::BTreeMap,
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
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

/// Recompute the aggregated row of a group after member volumes changed in place.
fn recompute<T>(group: &mut Group<T>) {
    group.row.volume = group
        .members
        .iter()
        .map(|m| normalized(m.volume))
        .max()
        .unwrap_or(0);
    group.row.muted = group.members.iter().all(|m| m.muted);
}

/// What the worker has to re-read. Set by COM callbacks (via `AudioChange`) and by writes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Dirty(u8);
impl Dirty {
    /// Default device or device set changed: rebuild everything and re-register.
    const REDISCOVER: Dirty = Dirty(1);
    /// Session list changed: re-enumerate sessions.
    const SESSIONS: Dirty = Dirty(2);
    /// Volume or mute of existing sessions changed.
    const VALUES: Dirty = Dirty(4);
    /// Master volume or mute changed.
    const ENDPOINT: Dirty = Dirty(8);
    const ALL: Dirty = Dirty(15);
    fn has(self, other: Dirty) -> bool {
        self.0 & other.0 != 0
    }
    fn insert(&mut self, other: Dirty) {
        self.0 |= other.0;
    }
    fn is_empty(self) -> bool {
        self.0 == 0
    }
    fn take(&mut self) -> Dirty {
        std::mem::take(self)
    }
}
fn dirty_for(change: AudioChange) -> Dirty {
    match change {
        AudioChange::Device => Dirty::REDISCOVER,
        AudioChange::Sessions => Dirty::SESSIONS,
        AudioChange::SessionValues => Dirty::VALUES,
        AudioChange::Endpoint => Dirty::ENDPOINT,
    }
}
/// Only the default multimedia render endpoint is shown.
#[cfg_attr(not(windows), allow(dead_code))]
fn default_change_relevant(render: bool, multimedia: bool) -> bool {
    render && multimedia
}

const DEBOUNCE: Duration = Duration::from_millis(40);
const MAX_DEBOUNCE: Duration = Duration::from_millis(250);
/// Safety net only; normal updates are event driven.
const FALLBACK: Duration = Duration::from_secs(30);

/// Everything absorbed during one wake-up of the worker.
#[derive(Default)]
struct Pending {
    dirty: Dirty,
    writes: Vec<AudioCommand>,
    active: Option<bool>,
    shutdown: bool,
    /// Explicit user action (write or refresh): never delayed by debouncing.
    urgent: bool,
}
impl Pending {
    fn absorb(&mut self, command: AudioCommand) {
        match command {
            AudioCommand::Shutdown => self.shutdown = true,
            AudioCommand::SetActive(value) => self.active = Some(value),
            AudioCommand::SetVolume { id, value } => {
                // Last value for a target wins, while mute commands retain order.
                self.writes
                    .retain(|c| !matches!(c, AudioCommand::SetVolume{id:old,..} if old == &id));
                self.writes.push(AudioCommand::SetVolume {
                    id,
                    value: normalized(value),
                });
                self.urgent = true;
            }
            AudioCommand::SetMute { .. } => {
                self.writes.push(command);
                self.urgent = true;
            }
            AudioCommand::Refresh => {
                self.dirty.insert(Dirty::ALL);
                self.urgent = true;
            }
            AudioCommand::Changed(change) => self.dirty.insert(dirty_for(change)),
        }
    }
}
/// How much longer to wait for further events before processing, `None` = process now.
/// Bursts of events are merged, but never delayed beyond `MAX_DEBOUNCE`.
fn debounce_wait(pending: &Pending, since_first: Duration) -> Option<Duration> {
    if pending.urgent || pending.shutdown || pending.dirty.is_empty() || since_first >= MAX_DEBOUNCE
    {
        return None;
    }
    Some(DEBOUNCE.min(MAX_DEBOUNCE - since_first))
}

pub fn spawn() -> (
    Sender<AudioCommand>,
    Receiver<AudioSnapshot>,
    crate::logging::CrashFlag,
) {
    let (commands, input) = mpsc::channel();
    let (output, snapshots) = mpsc::channel();
    let crashed = crate::logging::CrashFlag::default();
    let events = commands.clone();
    crate::logging::spawn_worker("nexpanel-audio", &crashed, move || {
        worker(input, output, events)
    });
    (commands, snapshots, crashed)
}
pub fn diagnose() -> AudioSnapshot {
    native::diagnose()
}

fn worker(
    input: Receiver<AudioCommand>,
    output: Sender<AudioSnapshot>,
    events: Sender<AudioCommand>,
) {
    let mut active = true;
    let mut backend = match native::Backend::new(Some(events)) {
        Ok(backend) => backend,
        Err(error) => {
            log::error!("Audio-Backend nicht verfügbar: {error}");
            let _ = output.send(AudioSnapshot {
                status: error,
                ..Default::default()
            });
            return;
        }
    };
    backend.mark(Dirty::ALL);
    let mut last = backend.refresh();
    log::info!(
        "Audio-Erkennung: Master: {}, Sitzungen: {}, Status: '{}'",
        last.master.is_some(),
        last.sessions.len(),
        last.status
    );
    if output.send(last.clone()).is_err() {
        return;
    }
    // Changes seen while the panel is hidden; applied once it is shown again.
    let mut deferred = Dirty::default();
    loop {
        let mut pending = Pending::default();
        let first = if active {
            match input.recv_timeout(FALLBACK) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    pending.dirty.insert(Dirty::ALL);
                    None
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match input.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            }
        };
        let started = Instant::now();
        if let Some(command) = first {
            pending.absorb(command);
        }
        loop {
            for command in input.try_iter().take(256) {
                pending.absorb(command);
            }
            let Some(wait) = debounce_wait(&pending, started.elapsed()) else {
                break;
            };
            match input.recv_timeout(wait) {
                Ok(command) => pending.absorb(command),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        if pending.shutdown {
            return;
        }
        if let Some(value) = pending.active {
            if value && !active {
                pending.dirty.insert(Dirty::ALL);
            }
            active = value;
        }
        let mut dirty = pending.dirty;
        dirty.insert(deferred.take());
        if !active && pending.writes.is_empty() {
            deferred = dirty;
            continue;
        }
        if dirty.is_empty() && pending.writes.is_empty() {
            continue;
        }
        backend.mark(dirty);
        let snapshot = run_batch(
            &mut backend,
            pending.writes,
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
            log::warn!("Audio schreiben fehlgeschlagen: {error}");
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
    use std::collections::{HashMap, HashSet};
    use windows::{
        Win32::{
            Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
            Foundation::{CloseHandle, HANDLE, PROPERTYKEY, S_OK},
            Media::Audio::{
                Endpoints::{
                    IAudioEndpointVolume, IAudioEndpointVolumeCallback,
                    IAudioEndpointVolumeCallback_Impl,
                },
                *,
            },
            System::{
                Com::{
                    StructuredStorage::{PropVariantClear, PropVariantToStringAlloc},
                    *,
                },
                Threading::*,
            },
        },
        core::{Interface, PCWSTR, PWSTR, Ref, implement},
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
    /// Friendly endpoint name ("Lautsprecher (Realtek Audio)"); best effort.
    fn device_name(device: &IMMDevice) -> String {
        unsafe {
            let Ok(store) = device.OpenPropertyStore(STGM_READ) else {
                return String::new();
            };
            let Ok(mut value) = store.GetValue(&PKEY_Device_FriendlyName) else {
                return String::new();
            };
            let text = PropVariantToStringAlloc(&value)
                .map(|p| Allocation(p).string())
                .unwrap_or_default();
            let _ = PropVariantClear(&mut value);
            text
        }
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

    // Callbacks run on foreign COM threads. They only send a tiny message to the
    // worker; no COM calls, no locks, no allocation beyond the channel node.
    fn notify(tx: &Sender<AudioCommand>, change: AudioChange) {
        let _ = tx.send(AudioCommand::Changed(change));
    }
    #[implement(IMMNotificationClient)]
    struct DeviceEvents {
        tx: Sender<AudioCommand>,
    }
    impl IMMNotificationClient_Impl for DeviceEvents_Impl {
        fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
            notify(&self.tx, AudioChange::Device);
            Ok(())
        }
        fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
            notify(&self.tx, AudioChange::Device);
            Ok(())
        }
        fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
            notify(&self.tx, AudioChange::Device);
            Ok(())
        }
        fn OnDefaultDeviceChanged(
            &self,
            flow: EDataFlow,
            role: ERole,
            _: &PCWSTR,
        ) -> windows::core::Result<()> {
            if default_change_relevant(flow == eRender, role == eMultimedia) {
                notify(&self.tx, AudioChange::Device);
            }
            Ok(())
        }
        fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> windows::core::Result<()> {
            Ok(())
        }
    }
    #[implement(IAudioEndpointVolumeCallback)]
    struct EndpointEvents {
        tx: Sender<AudioCommand>,
    }
    impl IAudioEndpointVolumeCallback_Impl for EndpointEvents_Impl {
        fn OnNotify(&self, _: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
            notify(&self.tx, AudioChange::Endpoint);
            Ok(())
        }
    }
    #[implement(IAudioSessionNotification)]
    struct NewSessionEvents {
        tx: Sender<AudioCommand>,
    }
    impl IAudioSessionNotification_Impl for NewSessionEvents_Impl {
        fn OnSessionCreated(&self, _: Ref<IAudioSessionControl>) -> windows::core::Result<()> {
            notify(&self.tx, AudioChange::Sessions);
            Ok(())
        }
    }
    #[implement(IAudioSessionEvents)]
    struct SessionEvents {
        tx: Sender<AudioCommand>,
    }
    impl IAudioSessionEvents_Impl for SessionEvents_Impl {
        fn OnDisplayNameChanged(
            &self,
            _: &PCWSTR,
            _: *const windows::core::GUID,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn OnIconPathChanged(
            &self,
            _: &PCWSTR,
            _: *const windows::core::GUID,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn OnSimpleVolumeChanged(
            &self,
            _: f32,
            _: windows::core::BOOL,
            _: *const windows::core::GUID,
        ) -> windows::core::Result<()> {
            notify(&self.tx, AudioChange::SessionValues);
            Ok(())
        }
        fn OnChannelVolumeChanged(
            &self,
            _: u32,
            _: *const f32,
            _: u32,
            _: *const windows::core::GUID,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn OnGroupingParamChanged(
            &self,
            _: *const windows::core::GUID,
            _: *const windows::core::GUID,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn OnStateChanged(&self, _: AudioSessionState) -> windows::core::Result<()> {
            notify(&self.tx, AudioChange::Sessions);
            Ok(())
        }
        fn OnSessionDisconnected(
            &self,
            _: AudioSessionDisconnectReason,
        ) -> windows::core::Result<()> {
            notify(&self.tx, AudioChange::Sessions);
            Ok(())
        }
    }
    /// A registered per-session callback; unregisters itself before the COM objects go away.
    struct SessionReg {
        control: IAudioSessionControl,
        events: IAudioSessionEvents,
    }
    impl Drop for SessionReg {
        fn drop(&mut self) {
            // Fails harmlessly for sessions that were already disconnected.
            let _ = unsafe {
                self.control
                    .UnregisterAudioSessionNotification(&self.events)
            };
        }
    }

    struct State {
        id: String,
        endpoint: IAudioEndpointVolume,
        manager: IAudioSessionManager2,
        groups: Vec<Group<ISimpleAudioVolume>>,
        master: AudioRow,
        device_name: String,
        skipped: usize,
        tx: Option<Sender<AudioCommand>>,
        endpoint_cb: Option<IAudioEndpointVolumeCallback>,
        session_cb: Option<IAudioSessionNotification>,
        sessions: HashMap<String, SessionReg>,
    }
    impl Drop for State {
        fn drop(&mut self) {
            // Unregister before the interfaces are released, so no callback can
            // outlive the objects it was registered on.
            self.sessions.clear();
            if let Some(cb) = self.session_cb.take() {
                let _ = unsafe { self.manager.UnregisterSessionNotification(&cb) };
            }
            if let Some(cb) = self.endpoint_cb.take() {
                let _ = unsafe { self.endpoint.UnregisterControlChangeNotify(&cb) };
            }
        }
    }
    impl State {
        fn build(
            enumerator: &IMMDeviceEnumerator,
            tx: Option<&Sender<AudioCommand>>,
        ) -> windows::core::Result<State> {
            unsafe {
                let device = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
                let id = Allocation(device.GetId()?).string();
                let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
                let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
                // Quirk: session notifications may not fire unless the enumerator was requested once.
                let _ = manager.GetSessionEnumerator();
                let mut state = State {
                    device_name: device_name(&device),
                    master: AudioRow {
                        id: format!("{id}|master"),
                        name: "Standardausgabegerät".into(),
                        volume: 0,
                        muted: false,
                    },
                    id,
                    endpoint,
                    manager,
                    groups: Vec::new(),
                    skipped: 0,
                    tx: tx.cloned(),
                    endpoint_cb: None,
                    session_cb: None,
                    sessions: HashMap::new(),
                };
                if let Some(tx) = tx {
                    let cb: IAudioEndpointVolumeCallback = EndpointEvents { tx: tx.clone() }.into();
                    match state.endpoint.RegisterControlChangeNotify(&cb) {
                        Ok(()) => state.endpoint_cb = Some(cb),
                        Err(e) => log::warn!("Master-Lautstärke-Events nicht verfügbar: {e}"),
                    }
                    let cb: IAudioSessionNotification = NewSessionEvents { tx: tx.clone() }.into();
                    match state.manager.RegisterSessionNotification(&cb) {
                        Ok(()) => state.session_cb = Some(cb),
                        Err(e) => log::warn!("Sitzungs-Events nicht verfügbar: {e}"),
                    }
                }
                state.reenumerate()?;
                state.reread_master()?;
                Ok(state)
            }
        }
        fn reread_master(&mut self) -> windows::core::Result<()> {
            unsafe {
                self.master.volume = percent(self.endpoint.GetMasterVolumeLevelScalar()?);
                self.master.muted = self.endpoint.GetMute()?.as_bool();
            }
            Ok(())
        }
        /// Cheap path: existing sessions only changed volume or mute.
        fn reread_values(&mut self) -> windows::core::Result<()> {
            for group in &mut self.groups {
                for member in &mut group.members {
                    unsafe {
                        member.volume = percent(member.control.GetMasterVolume()?);
                        member.muted = member.control.GetMute()?.as_bool();
                    }
                }
                recompute(group);
            }
            Ok(())
        }
        fn reenumerate(&mut self) -> windows::core::Result<()> {
            unsafe {
                let sessions = self.manager.GetSessionEnumerator()?;
                let mut members = Vec::new();
                let mut controls = Vec::new();
                let mut skipped = 0;
                for index in 0..sessions.GetCount()? {
                    let result = (|| -> windows::core::Result<
                        Option<(Member<ISimpleAudioVolume>, IAudioSessionControl)>,
                    > {
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
                        Ok(Some((
                            Member {
                                instance,
                                pid,
                                system,
                                name,
                                volume: percent(volume.GetMasterVolume()?),
                                muted: volume.GetMute()?.as_bool(),
                                control: volume,
                            },
                            control,
                        )))
                    })();
                    match result {
                        Ok(Some((member, control))) => {
                            members.push(member);
                            controls.push(control);
                        }
                        Ok(None) => {}
                        Err(_) => skipped += 1,
                    }
                }
                // Keep per-session event registrations in sync with the live sessions.
                if let Some(tx) = &self.tx {
                    let live: HashSet<&str> = members.iter().map(|m| m.instance.as_str()).collect();
                    self.sessions
                        .retain(|instance, _| live.contains(instance.as_str()));
                    for (member, control) in members.iter().zip(controls) {
                        if self.sessions.contains_key(&member.instance) {
                            continue;
                        }
                        let events: IAudioSessionEvents = SessionEvents { tx: tx.clone() }.into();
                        match control.RegisterAudioSessionNotification(&events) {
                            Ok(()) => {
                                self.sessions.insert(
                                    member.instance.clone(),
                                    SessionReg { control, events },
                                );
                            }
                            Err(e) => log::warn!("Sitzungs-Events nicht registrierbar: {e}"),
                        }
                    }
                }
                self.groups = groups(&self.id, members);
                self.skipped = skipped;
            }
            Ok(())
        }
    }

    pub(super) struct Backend {
        // Field order is drop order: state (unregisters its callbacks) first, COM last.
        state: Option<State>,
        failure: String,
        dirty: Dirty,
        tx: Option<Sender<AudioCommand>>,
        client: Option<IMMNotificationClient>,
        enumerator: IMMDeviceEnumerator,
        _com: Com,
    }
    impl Drop for Backend {
        fn drop(&mut self) {
            if let Some(client) = self.client.take() {
                let _ = unsafe {
                    self.enumerator
                        .UnregisterEndpointNotificationCallback(&client)
                };
            }
            self.state = None;
        }
    }
    impl Backend {
        /// `events`: channel the COM callbacks signal on. `None` = read-only diagnostics.
        pub(super) fn new(events: Option<Sender<AudioCommand>>) -> Result<Self, String> {
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }.map_err(error)?;
            let com = Com;
            let enumerator: IMMDeviceEnumerator =
                unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                    .map_err(error)?;
            let mut client = None;
            if let Some(tx) = &events {
                let cb: IMMNotificationClient = DeviceEvents { tx: tx.clone() }.into();
                match unsafe { enumerator.RegisterEndpointNotificationCallback(&cb) } {
                    Ok(()) => client = Some(cb),
                    Err(e) => log::warn!("Geräte-Events nicht verfügbar: {e}"),
                }
            }
            Ok(Self {
                state: None,
                failure: String::new(),
                dirty: Dirty::ALL,
                tx: events,
                client,
                enumerator,
                _com: com,
            })
        }
        pub(super) fn mark(&mut self, dirty: Dirty) {
            self.dirty.insert(dirty);
        }
        fn rediscover(&mut self) {
            // Drop the old state first: unregisters its callbacks.
            self.state = None;
            match State::build(&self.enumerator, self.tx.as_ref()) {
                Ok(state) => {
                    self.failure.clear();
                    self.state = Some(state);
                }
                Err(e) => self.failure = error(e),
            }
        }
        /// Apply pending dirty flags, reading only what changed.
        fn sync(&mut self) {
            let dirty = self.dirty.take();
            if dirty.is_empty() {
                return;
            }
            let Some(state) = self
                .state
                .as_mut()
                .filter(|_| !dirty.has(Dirty::REDISCOVER))
            else {
                self.rediscover();
                return;
            };
            let result = (|| {
                if dirty.has(Dirty::SESSIONS) {
                    state.reenumerate()?;
                } else if dirty.has(Dirty::VALUES) {
                    state.reread_values()?;
                }
                if dirty.has(Dirty::ENDPOINT) {
                    state.reread_master()?;
                }
                Ok::<(), windows::core::Error>(())
            })();
            if let Err(e) = result {
                log::warn!("Audio-Teilaktualisierung fehlgeschlagen, vollständige Erkennung: {e}");
                self.rediscover();
            }
        }
        fn snapshot(&self) -> AudioSnapshot {
            match &self.state {
                Some(state) => AudioSnapshot {
                    master: Some(state.master.clone()),
                    sessions: state.groups.iter().map(|g| g.row.clone()).collect(),
                    device: state.device_name.clone(),
                    status: if state.skipped == 0 {
                        String::new()
                    } else {
                        format!(
                            "{} Audio-Sitzung(en) konnten nicht gelesen werden.",
                            state.skipped
                        )
                    },
                },
                None => AudioSnapshot {
                    status: self.failure.clone(),
                    ..Default::default()
                },
            }
        }
        pub(super) fn refresh(&mut self) -> AudioSnapshot {
            self.sync();
            self.snapshot()
        }
        pub(super) fn write(&mut self, command: AudioCommand) -> Result<(), String> {
            // Apply pending changes first: a queued ID never targets a stale endpoint or session.
            self.sync();
            let state = self
                .state
                .as_ref()
                .ok_or("Kein Standardausgabegerät verfügbar.")?;
            let (id, volume, mute) = match command {
                AudioCommand::SetVolume { id, value } => (id, Some(normalized(value)), None),
                AudioCommand::SetMute { id, muted } => (id, None, Some(muted)),
                _ => return Ok(()),
            };
            if id == state.master.id {
                self.dirty.insert(Dirty::ENDPOINT);
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
                self.dirty.insert(Dirty::VALUES);
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
        match Backend::new(None) {
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
        pub(super) fn new(_: Option<Sender<AudioCommand>>) -> Result<Self, String> {
            Err("Audio-Steuerung benötigt Windows.".into())
        }
        pub(super) fn mark(&mut self, _: Dirty) {}
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

    #[test]
    fn events_map_to_minimal_dirty_flags() {
        assert_eq!(dirty_for(AudioChange::Device), Dirty::REDISCOVER);
        assert_eq!(dirty_for(AudioChange::Sessions), Dirty::SESSIONS);
        assert_eq!(dirty_for(AudioChange::SessionValues), Dirty::VALUES);
        assert_eq!(dirty_for(AudioChange::Endpoint), Dirty::ENDPOINT);
        let mut d = Dirty::default();
        assert!(d.is_empty());
        d.insert(Dirty::VALUES);
        d.insert(Dirty::ENDPOINT);
        assert!(d.has(Dirty::VALUES) && d.has(Dirty::ENDPOINT));
        assert!(!d.has(Dirty::SESSIONS) && !d.has(Dirty::REDISCOVER));
        assert_eq!(d.take(), Dirty(12));
        assert!(d.is_empty());
        assert!(Dirty::ALL.has(Dirty::REDISCOVER));
    }
    #[test]
    fn only_default_multimedia_render_change_is_relevant() {
        assert!(default_change_relevant(true, true));
        assert!(!default_change_relevant(false, true));
        assert!(!default_change_relevant(true, false));
    }
    #[test]
    fn debounce_waits_only_for_pure_event_bursts() {
        let mut p = Pending::default();
        assert_eq!(debounce_wait(&p, Duration::ZERO), None);
        p.absorb(AudioCommand::Changed(AudioChange::Endpoint));
        assert_eq!(debounce_wait(&p, Duration::ZERO), Some(DEBOUNCE));
        // Never beyond the cap.
        let near = MAX_DEBOUNCE - Duration::from_millis(10);
        assert_eq!(debounce_wait(&p, near), Some(Duration::from_millis(10)));
        assert_eq!(debounce_wait(&p, MAX_DEBOUNCE), None);
        // User actions are not delayed.
        p.absorb(AudioCommand::SetMute {
            id: "a".into(),
            muted: true,
        });
        assert_eq!(debounce_wait(&p, Duration::ZERO), None);
        let mut r = Pending::default();
        r.absorb(AudioCommand::Changed(AudioChange::Sessions));
        r.absorb(AudioCommand::Refresh);
        assert_eq!(debounce_wait(&r, Duration::ZERO), None);
        let mut s = Pending::default();
        s.absorb(AudioCommand::Changed(AudioChange::Sessions));
        s.absorb(AudioCommand::Shutdown);
        assert_eq!(debounce_wait(&s, Duration::ZERO), None);
    }
    #[test]
    fn pending_coalesces_volume_latest_wins_and_merges_events() {
        let mut p = Pending::default();
        let vol = |id: &str, value| AudioCommand::SetVolume {
            id: id.into(),
            value,
        };
        p.absorb(vol("a", 10));
        p.absorb(AudioCommand::SetMute {
            id: "a".into(),
            muted: true,
        });
        p.absorb(vol("b", 20));
        p.absorb(vol("a", 200));
        p.absorb(AudioCommand::Changed(AudioChange::Sessions));
        p.absorb(AudioCommand::Changed(AudioChange::SessionValues));
        p.absorb(AudioCommand::SetActive(false));
        assert_eq!(
            p.writes,
            vec![
                AudioCommand::SetMute {
                    id: "a".into(),
                    muted: true
                },
                vol("b", 20),
                vol("a", 100),
            ]
        );
        assert!(p.dirty.has(Dirty::SESSIONS) && p.dirty.has(Dirty::VALUES));
        assert!(!p.dirty.has(Dirty::ENDPOINT));
        assert_eq!(p.active, Some(false));
        assert!(!p.shutdown);
    }
    #[test]
    fn recompute_aggregates_after_in_place_change() {
        let mut g = groups(
            "e",
            vec![member("a", 7, 20, true), member("b", 7, 30, true)],
        );
        assert!(g[0].row.muted);
        g[0].members[1].volume = 90;
        g[0].members[0].muted = false;
        recompute(&mut g[0]);
        assert_eq!(g[0].row.volume, 90);
        assert!(!g[0].row.muted);
    }
    #[test]
    fn snapshot_diff_suppresses_identical_snapshots() {
        let a = AudioSnapshot {
            master: Some(AudioRow {
                id: "e|master".into(),
                name: "M".into(),
                volume: 5,
                muted: false,
            }),
            ..Default::default()
        };
        let mut b = a.clone();
        assert_eq!(a, b);
        b.master.as_mut().unwrap().volume = 6;
        assert_ne!(a, b);
    }
}
