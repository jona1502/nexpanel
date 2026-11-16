//! Native resources live exclusively on the worker. Dxva2 has no cancellation deadline.
use super::{DisplayCommand, DisplayDevice, DisplaySnapshot};
use std::{
    collections::BTreeMap,
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};
fn native_value(min: u32, max: u32, value: u8) -> u32 {
    if max <= min {
        return min;
    }
    min + ((u64::from(max - min) * u64::from(value.min(100)) + 50) / 100) as u32
}
fn percentage(min: u32, max: u32, value: u32) -> u8 {
    if max <= min {
        return 0;
    }
    ((u64::from(value.clamp(min, max) - min) * 100 + u64::from(max - min) / 2)
        / u64::from(max - min)) as u8
}
fn identity(value: &str) -> String {
    let s = value
        .trim_start_matches(r"\\?\")
        .replace('#', r"\")
        .to_ascii_lowercase();
    let s = s.split(r"\{").next().unwrap_or(&s);
    s.strip_suffix("_0").unwrap_or(s).to_owned()
}
fn queue(
    pending: &mut BTreeMap<String, u8>,
    devices: &[DisplayDevice],
    command: DisplayCommand,
) -> Result<(), String> {
    match command {
        DisplayCommand::SetBrightness { id, value } => {
            if !devices.iter().any(|d| d.id == id && d.brightness.is_some()) {
                return Err(format!("Monitor nicht verfügbar oder entfernt: {id}"));
            }
            pending.insert(id, value.min(100));
        }
        DisplayCommand::SetLinkedBrightness(value) => {
            for d in devices.iter().filter(|d| d.brightness.is_some()) {
                pending.insert(d.id.clone(), value.min(100));
            }
        }
        _ => (),
    }
    Ok(())
}
/// Safety net only: hotplug arrives as `TopologyChanged` from `WM_DISPLAYCHANGE`.
fn poll_interval(active: bool) -> Duration {
    Duration::from_secs(if active { 30 } else { 60 })
}
/// Bursts of change messages (one per monitor/mode) collapse into a single refresh.
const DEBOUNCE: Duration = Duration::from_millis(300);
/// Retry delay after a failed write, so a stale status does not wait for the safety poll.
const RETRY_DELAY: Duration = Duration::from_secs(2);
/// How long the worker may sleep: until the safety poll or a debounce deadline, whichever is first.
fn wait_timeout(since_poll: Duration, active: bool, debounce_left: Option<Duration>) -> Duration {
    let poll_left = poll_interval(active).saturating_sub(since_poll);
    debounce_left.map_or(poll_left, |d| d.min(poll_left))
}
/// Whether a refresh should read brightness values from the monitors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Read {
    /// Panel hidden: nobody looks at the values.
    Skip,
    /// Panel visible: re-read when the last read is older than 30 s.
    IfStale,
    /// Panel just became visible: values may have changed meanwhile.
    Now,
}
fn read_mode(active: bool, activated: bool) -> Read {
    match (active, activated) {
        (true, true) => Read::Now,
        (true, false) => Read::IfStale,
        _ => Read::Skip,
    }
}
/// Returns immediately; send Shutdown without joining from the UI thread.
pub fn spawn() -> (
    Sender<DisplayCommand>,
    Receiver<DisplaySnapshot>,
    crate::logging::CrashFlag,
) {
    let (tx, rx) = mpsc::channel();
    let (out, snap) = mpsc::channel();
    let crashed = crate::logging::CrashFlag::default();
    crate::logging::spawn_worker("display", &crashed, move || run(rx, out));
    (tx, snap, crashed)
}
/// Read-only discovery; call on a background thread, never the UI thread.
pub fn diagnose() -> DisplaySnapshot {
    let mut backend = native::Backend::new();
    backend.discover();
    backend.snapshot.clone()
}
fn run(rx: Receiver<DisplayCommand>, out: Sender<DisplaySnapshot>) {
    let mut b = native::Backend::new();
    b.discover();
    let mut sent = b.snapshot.clone();
    log::info!(
        "Display-Erkennung: {} Monitor(e), regelbar: {}, Status: '{}'",
        b.snapshot.devices.len(),
        b.snapshot
            .devices
            .iter()
            .filter(|d| d.brightness.is_some())
            .count(),
        b.snapshot.status
    );
    if out.send(sent.clone()).is_err() {
        return;
    }
    let mut active = true;
    let mut last = Instant::now();
    let mut retry = false;
    let mut debounce: Option<Instant> = None;
    loop {
        let wait = wait_timeout(
            last.elapsed(),
            active,
            debounce.map(|d| d.saturating_duration_since(Instant::now())),
        );
        let first = rx.recv_timeout(wait);
        let mut commands = Vec::new();
        match first {
            Ok(c) => commands.push(c),
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(_) => (),
        };
        // Bound each drain so a continuous slider cannot starve shutdown/topology work.
        for _ in 0..255 {
            match rx.try_recv() {
                Ok(c) => commands.push(c),
                Err(_) => break,
            }
        }
        let mut pending = BTreeMap::new();
        let mut refresh = false;
        let mut activated = false;
        let mut error = None;
        for c in commands {
            match c {
                DisplayCommand::Shutdown => return,
                DisplayCommand::SetActive(v) => {
                    activated |= v && !active;
                    active = v;
                }
                DisplayCommand::TopologyChanged => debounce = Some(Instant::now() + DEBOUNCE),
                DisplayCommand::Refresh => refresh = true,
                c => {
                    if let Err(e) = queue(&mut pending, &b.snapshot.devices, c) {
                        error = Some(e);
                        refresh = true;
                    }
                }
            }
        }
        let due = last.elapsed() >= poll_interval(active);
        let settled = debounce.is_some_and(|d| Instant::now() >= d);
        if settled {
            debounce = None;
        }
        let due = due || settled || activated;
        if refresh || due {
            b.refresh(refresh || retry, read_mode(active, activated));
            retry = false;
            last = Instant::now();
        }
        let changed = !pending.is_empty();
        for (id, value) in pending {
            if let Err(e) = b.write(&id, value) {
                log::warn!("Helligkeit schreiben fehlgeschlagen ({id}): {e}");
                error = Some(e);
            }
        }
        if let Some(e) = error {
            retry = true;
            debounce = Some(Instant::now() + RETRY_DELAY);
            b.snapshot.status = e;
        }
        // Send only real changes; identical snapshots would just churn the UI.
        if (changed || refresh || due) && b.snapshot != sent {
            if out.send(b.snapshot.clone()).is_err() {
                return;
            }
            sent = b.snapshot.clone();
        }
    }
}
#[cfg(windows)]
mod native {
    use super::*;
    use serde::{Deserialize, Serialize};
    use windows::Win32::{
        Devices::Display::*,
        Foundation::{GetLastError, HANDLE, LPARAM, RECT},
        Graphics::Gdi::*,
    };
    use windows::core::BOOL;
    use wmi::WMIConnection;
    fn wide(v: &[u16]) -> String {
        String::from_utf16_lossy(&v[..v.iter().position(|c| *c == 0).unwrap_or(v.len())])
    }
    struct Physical(HANDLE);
    impl Drop for Physical {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyPhysicalMonitor(self.0);
            }
        }
    }
    enum Control {
        Ddc {
            handle: Physical,
            min: u32,
            max: u32,
        },
        Wmi {
            path: String,
            levels: Vec<u8>,
        },
    }
    struct Target {
        id: String,
        name: String,
        source: String,
        unavailable: Option<String>,
    }
    fn append_target(
        rows: &mut Vec<Target>,
        fallback: String,
        metadata: Result<(String, String, String), String>,
    ) {
        let row = match metadata {
            Ok((id, name, source)) => Target {
                id,
                name,
                source,
                unavailable: None,
            },
            Err(error) => Target {
                id: fallback,
                name: "Monitor".into(),
                source: String::new(),
                unavailable: Some(error),
            },
        };
        rows.push(row);
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Brightness {
        instance_name: String,
        current_brightness: u8,
        level: Vec<u8>,
        active: bool,
    }
    #[derive(Deserialize)]
    #[serde(rename = "WmiMonitorBrightnessMethods", rename_all = "PascalCase")]
    struct Method {
        instance_name: String,
        #[serde(rename = "__PATH")]
        path: String,
    }
    #[derive(Serialize)]
    #[serde(rename_all = "PascalCase")]
    struct Input {
        timeout: u32,
        brightness: u8,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Output {
        return_value: u32,
    }
    pub(super) struct Backend {
        pub snapshot: DisplaySnapshot,
        controls: BTreeMap<String, Control>,
        wmi: Option<WMIConnection>,
        topology: Vec<(String, String)>,
        last_read: Instant,
    }
    fn targets() -> Result<Vec<Target>, String> {
        unsafe {
            for _ in 0..3 {
                let (mut pc, mut mc) = (0, 0);
                GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut pc, &mut mc)
                    .ok()
                    .map_err(|e| e.to_string())?;
                let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); pc as usize];
                let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mc as usize];
                let r = QueryDisplayConfig(
                    QDC_ONLY_ACTIVE_PATHS,
                    &mut pc,
                    paths.as_mut_ptr(),
                    &mut mc,
                    modes.as_mut_ptr(),
                    None,
                );
                if r.0 == 122 {
                    continue;
                }
                r.ok().map_err(|e| e.to_string())?;
                let mut result = Vec::new();
                for p in paths.into_iter().take(pc as usize) {
                    let mut t = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
                    t.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
                        r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                        size: std::mem::size_of_val(&t) as u32,
                        adapterId: p.targetInfo.adapterId,
                        id: p.targetInfo.id,
                    };
                    let mut s = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
                    s.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
                        r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                        size: std::mem::size_of_val(&s) as u32,
                        adapterId: p.sourceInfo.adapterId,
                        id: p.sourceInfo.id,
                    };
                    let target_result = DisplayConfigGetDeviceInfo(&mut t.header);
                    let source_result = DisplayConfigGetDeviceInfo(&mut s.header);
                    let fallback = format!(
                        "target:{}:{}:{}",
                        p.targetInfo.adapterId.HighPart,
                        p.targetInfo.adapterId.LowPart,
                        p.targetInfo.id
                    );
                    let metadata = if target_result != 0 || source_result != 0 {
                        Err(format!(
                            "Monitoridentität nicht verfügbar (Ziel: {target_result}, Quelle: {source_result}); bitte aktualisieren"
                        ))
                    } else {
                        let path = wide(&t.monitorDevicePath);
                        let id = if path.is_empty() {
                            fallback.clone()
                        } else {
                            identity(&path)
                        };
                        let name = wide(&t.monitorFriendlyDeviceName);
                        Ok((
                            id,
                            if name.is_empty() {
                                "Monitor".into()
                            } else {
                                name
                            },
                            wide(&s.viewGdiDeviceName),
                        ))
                    };
                    append_target(&mut result, fallback, metadata);
                }
                result.sort_by(|a, b| a.id.cmp(&b.id));
                return Ok(result);
            }
            Err("Monitoranordnung hat sich wiederholt geändert; bitte aktualisieren".into())
        }
    }
    unsafe extern "system" fn enumerate(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        unsafe {
            let list = &mut *(data.0 as *mut Vec<(String, HMONITOR)>);
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            if GetMonitorInfoW(h, &mut info.monitorInfo).as_bool() {
                list.push((wide(&info.szDevice), h));
            }
            BOOL(1)
        }
    }
    impl Backend {
        pub fn new() -> Self {
            Self {
                snapshot: DisplaySnapshot::default(),
                controls: BTreeMap::new(),
                wmi: WMIConnection::with_namespace_path(r"ROOT\WMI").ok(),
                topology: Vec::new(),
                last_read: Instant::now(),
            }
        }
        pub fn discover(&mut self) {
            let ts = match targets() {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("Monitor-Erkennung fehlgeschlagen: {e}");
                    self.snapshot.status = e;
                    return;
                }
            };
            self.controls.clear();
            self.snapshot = DisplaySnapshot::default();
            self.topology = ts
                .iter()
                .map(|t| (t.id.clone(), t.source.clone()))
                .collect();
            let mut logical = Vec::new();
            unsafe {
                let _ = EnumDisplayMonitors(
                    None,
                    None,
                    Some(enumerate),
                    LPARAM(&mut logical as *mut Vec<(String, HMONITOR)> as isize),
                );
            }
            for t in &ts {
                let mut row = DisplayDevice {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    brightness: None,
                    status: "DDC/CI-Helligkeit nicht verfügbar".into(),
                };
                if let Some(error) = &t.unavailable {
                    row.status = error.clone();
                    self.snapshot.devices.push(row);
                    continue;
                }
                // Clone sources have no guaranteed target-to-physical index relationship.
                if ts.iter().filter(|other| other.source == t.source).count() != 1 {
                    row.status = "Duplizierte Anzeige: Hardwarezuordnung nicht eindeutig".into();
                } else if let Some((_, h)) = logical.iter().find(|(s, _)| s == &t.source) {
                    unsafe {
                        let mut n = 0;
                        if GetNumberOfPhysicalMonitorsFromHMONITOR(*h, &mut n).is_ok()
                            && n <= 64
                            && n > 0
                        {
                            let mut ps = vec![PHYSICAL_MONITOR::default(); n as usize];
                            if GetPhysicalMonitorsFromHMONITOR(*h, &mut ps).is_ok() {
                                let handles: Vec<Physical> = ps
                                    .into_iter()
                                    .map(|p| Physical(p.hPhysicalMonitor))
                                    .collect();
                                if handles.len() == 1 {
                                    let handle = handles.into_iter().next().unwrap();
                                    let (mut caps, mut temp, mut min, mut current, mut max) =
                                        (0, 0, 0, 0, 0);
                                    if GetMonitorCapabilities(handle.0, &mut caps, &mut temp) != 0
                                        && caps & 2 != 0
                                        && GetMonitorBrightness(
                                            handle.0,
                                            &mut min,
                                            &mut current,
                                            &mut max,
                                        ) != 0
                                        && max > min
                                    {
                                        row.brightness = Some(percentage(min, max, current));
                                        row.status = "DDC/CI".into();
                                        self.controls.insert(
                                            t.id.clone(),
                                            Control::Ddc { handle, min, max },
                                        );
                                    }
                                } else {
                                    row.status =
                                        "Mehrere physische Monitore: Zuordnung nicht eindeutig"
                                            .into();
                                }
                            }
                        }
                    }
                }
                self.snapshot.devices.push(row);
            }
            if let Some(w) = &self.wmi {
                let bs=w.raw_query::<Brightness>("SELECT InstanceName, CurrentBrightness, Level, Active FROM WmiMonitorBrightness");
                let ms = w.raw_query::<Method>(
                    "SELECT InstanceName, __PATH FROM WmiMonitorBrightnessMethods",
                );
                if let (Ok(bs), Ok(ms)) = (bs, ms) {
                    for b in bs.into_iter().filter(|b| b.active) {
                        let Some(m) = ms.iter().find(|m| m.instance_name == b.instance_name) else {
                            continue;
                        };
                        let id = identity(&b.instance_name);
                        let matches: Vec<_> = self
                            .snapshot
                            .devices
                            .iter()
                            .enumerate()
                            .filter(|(_, d)| d.id == id)
                            .map(|(i, _)| i)
                            .collect();
                        if matches.len() == 1 {
                            let row = &mut self.snapshot.devices[matches[0]];
                            if row.brightness.is_none() {
                                row.brightness = Some(b.current_brightness.min(100));
                                row.status = "Interner Bildschirm (WMI)".into();
                                self.controls.insert(
                                    row.id.clone(),
                                    Control::Wmi {
                                        path: m.path.clone(),
                                        levels: b.level,
                                    },
                                );
                            }
                        } else if matches.is_empty() {
                            self.snapshot.devices.push(DisplayDevice {
                                id: format!("wmi:{id}"),
                                name: format!("Interner Bildschirm ({})", b.instance_name),
                                brightness: Some(b.current_brightness.min(100)),
                                status: "WMI; keine passende aktive Anzeige".into(),
                            });
                            self.controls.insert(
                                format!("wmi:{id}"),
                                Control::Wmi {
                                    path: m.path.clone(),
                                    levels: b.level,
                                },
                            );
                        }
                    }
                }
            }
            self.last_read = Instant::now();
        }
        pub fn refresh(&mut self, force: bool, read: Read) {
            match targets() {
                Ok(ts) => {
                    let key: Vec<_> = ts.into_iter().map(|t| (t.id, t.source)).collect();
                    if force
                        || key != self.topology
                        || self
                            .snapshot
                            .devices
                            .iter()
                            .any(|d| d.brightness.is_none() && self.controls.contains_key(&d.id))
                    {
                        self.discover();
                        return;
                    }
                }
                Err(e) => {
                    self.snapshot.status = e;
                    return;
                }
            }
            if read == Read::Skip
                || (read == Read::IfStale && self.last_read.elapsed() < Duration::from_secs(30))
            {
                return;
            }
            self.last_read = Instant::now();
            for row in &mut self.snapshot.devices {
                if let Some(Control::Ddc { handle, min, max }) = self.controls.get_mut(&row.id) {
                    let mut current = 0;
                    unsafe {
                        if GetMonitorBrightness(handle.0, min, &mut current, max) != 0
                            && *max > *min
                        {
                            row.brightness = Some(percentage(*min, *max, current));
                        } else {
                            row.brightness = None;
                            row.status =
                                "Helligkeit konnte nicht gelesen werden; bitte aktualisieren"
                                    .into();
                        }
                    }
                }
            }
            let records = self.wmi.as_ref().ok_or_else(|| "WMI nicht verfügbar".to_owned()).and_then(|w| w.raw_query::<Brightness>("SELECT InstanceName, CurrentBrightness, Level, Active FROM WmiMonitorBrightness").map_err(|e| e.to_string()));
            self.apply_wmi_read(records);
        }
        fn apply_wmi_read(&mut self, records: Result<Vec<Brightness>, String>) {
            // Invalidate cached WMI values before reconciling the current active provider rows.
            // Retain the control entry so the existing refresh condition schedules rediscovery.
            let reason = match &records {
                Err(error) => format!("WMI-Helligkeit konnte nicht gelesen werden: {error}"),
                Ok(_) => "Interner Bildschirm nicht mehr aktiv oder verfügbar; bitte aktualisieren"
                    .into(),
            };
            for row in &mut self.snapshot.devices {
                if matches!(self.controls.get(&row.id), Some(Control::Wmi { .. })) {
                    row.brightness = None;
                    row.status = reason.clone();
                }
            }
            if let Ok(records) = records {
                for record in records.into_iter().filter(|record| record.active) {
                    let id = identity(&record.instance_name);
                    for row in &mut self.snapshot.devices {
                        if (row.id == id || row.id == format!("wmi:{id}"))
                            && matches!(self.controls.get(&row.id), Some(Control::Wmi { .. }))
                        {
                            row.brightness = Some(record.current_brightness.min(100));
                            row.status = "Interner Bildschirm (WMI)".into();
                        }
                    }
                }
            }
        }
        pub fn write(&mut self, id: &str, value: u8) -> Result<(), String> {
            let result = match self.controls.get(id) {
                Some(Control::Ddc { handle, min, max }) => unsafe {
                    if SetMonitorBrightness(handle.0, native_value(*min, *max, value)) != 0 {
                        Ok(value)
                    } else {
                        Err(format!(
                            "Helligkeit konnte nicht gesetzt werden: {:?}",
                            GetLastError()
                        ))
                    }
                },
                Some(Control::Wmi { path, levels }) => (|| {
                    let v = levels
                        .iter()
                        .copied()
                        .min_by_key(|v| v.abs_diff(value))
                        .unwrap_or(value);
                    let w = self.wmi.as_ref().ok_or("WMI nicht verfügbar")?;
                    let out: Output = w
                        .exec_instance_method::<Method, Output>(
                            path,
                            "WmiSetBrightness",
                            Input {
                                timeout: 0,
                                brightness: v,
                            },
                        )
                        .map_err(|e| e.to_string())?;
                    if out.return_value == 0 {
                        Ok(v)
                    } else {
                        Err(format!(
                            "WMI-Helligkeit meldet Fehlercode {}",
                            out.return_value
                        ))
                    }
                })(),
                None => Err("Monitor entfernt; zum Verbinden aktualisieren".into()),
            };
            match result {
                Ok(v) => {
                    if let Some(row) = self.snapshot.devices.iter_mut().find(|r| r.id == id) {
                        row.brightness = Some(v);
                    }
                    Ok(())
                }
                Err(e) => {
                    if let Some(row) = self.snapshot.devices.iter_mut().find(|r| r.id == id) {
                        row.status = e.clone();
                        row.brightness = None;
                    }
                    Err(e)
                }
            }
        }
    }
    #[cfg(test)]
    mod regression_tests {
        use super::*;
        #[test]
        fn metadata_failure_preserves_failed_and_healthy_targets() {
            let mut rows = Vec::new();
            append_target(
                &mut rows,
                "target:1:2:1".into(),
                Ok(("healthy-a".into(), "A".into(), "source-a".into())),
            );
            append_target(&mut rows, "target:1:2:2".into(), Err("query failed".into()));
            append_target(
                &mut rows,
                "target:1:2:3".into(),
                Ok(("healthy-b".into(), "B".into(), "source-b".into())),
            );
            assert_eq!(
                rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
                vec!["healthy-a", "target:1:2:2", "healthy-b"]
            );
            assert!(rows[1].unavailable.is_some());
            assert!(rows[0].unavailable.is_none());
            assert!(rows[2].unavailable.is_none());
        }
        fn backend() -> Backend {
            Backend {
                snapshot: DisplaySnapshot {
                    devices: vec![DisplayDevice {
                        id: "display\\panel\\instance".into(),
                        name: "Internal".into(),
                        brightness: Some(70),
                        status: "WMI".into(),
                    }],
                    status: String::new(),
                },
                controls: BTreeMap::from([(
                    "display\\panel\\instance".into(),
                    Control::Wmi {
                        path: "provider path".into(),
                        levels: vec![0, 50, 100],
                    },
                )]),
                wmi: None,
                topology: vec![],
                last_read: Instant::now(),
            }
        }
        fn assert_unavailable(b: &Backend) {
            assert_eq!(b.snapshot.devices[0].brightness, None);
            assert!(!b.snapshot.devices[0].status.is_empty());
            assert!(
                queue(
                    &mut BTreeMap::new(),
                    &b.snapshot.devices,
                    DisplayCommand::SetBrightness {
                        id: "display\\panel\\instance".into(),
                        value: 30
                    }
                )
                .is_err()
            );
            // Existing refresh condition must schedule rediscovery for this cached control.
            assert!(
                b.snapshot
                    .devices
                    .iter()
                    .any(|d| d.brightness.is_none() && b.controls.contains_key(&d.id))
            );
        }
        #[test]
        fn wmi_query_failure_disables_cached_control() {
            let mut b = backend();
            b.apply_wmi_read(Err("provider failed".into()));
            assert_unavailable(&b);
        }
        #[test]
        fn wmi_missing_record_disables_cached_control() {
            let mut b = backend();
            b.apply_wmi_read(Ok(vec![]));
            assert_unavailable(&b);
        }
        #[test]
        fn wmi_inactive_record_disables_cached_control() {
            let mut b = backend();
            b.apply_wmi_read(Ok(vec![Brightness {
                instance_name: "DISPLAY\\PANEL\\INSTANCE_0".into(),
                current_brightness: 90,
                level: vec![0, 50, 100],
                active: false,
            }]));
            assert_unavailable(&b);
        }
    }
}
#[cfg(not(windows))]
mod native {
    use super::*;
    pub struct Backend {
        pub snapshot: DisplaySnapshot,
    }
    impl Backend {
        pub fn new() -> Self {
            Self {
                snapshot: DisplaySnapshot {
                    devices: vec![],
                    status: "Monitorsteuerung benötigt Windows".into(),
                },
            }
        }
        pub fn discover(&mut self) {}
        pub fn refresh(&mut self, _: bool, _: Read) {}
        pub fn write(&mut self, _: &str, _: u8) -> Result<(), String> {
            Err("Windows erforderlich".into())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn devices() -> Vec<DisplayDevice> {
        vec![
            DisplayDevice {
                id: "a".into(),
                name: "A".into(),
                brightness: Some(20),
                status: String::new(),
            },
            DisplayDevice {
                id: "b".into(),
                name: "B".into(),
                brightness: None,
                status: String::new(),
            },
        ]
    }
    #[test]
    fn safety_poll_is_slow_and_slower_when_hidden() {
        assert_eq!(poll_interval(true), Duration::from_secs(30));
        assert_eq!(poll_interval(false), Duration::from_secs(60));
    }
    #[test]
    fn worker_sleeps_until_next_deadline() {
        let s = Duration::from_secs;
        assert_eq!(wait_timeout(s(0), true, None), s(30));
        assert_eq!(wait_timeout(s(25), false, None), s(35));
        assert_eq!(wait_timeout(s(99), false, None), Duration::ZERO);
        assert_eq!(wait_timeout(s(0), true, Some(DEBOUNCE)), DEBOUNCE);
        assert_eq!(wait_timeout(s(29), true, Some(s(5))), s(1));
    }
    #[test]
    fn brightness_is_read_only_while_visible_and_once_on_show() {
        assert_eq!(read_mode(false, false), Read::Skip);
        assert_eq!(read_mode(true, false), Read::IfStale);
        assert_eq!(read_mode(true, true), Read::Now);
    }
    #[test]
    fn native_range_is_respected() {
        assert_eq!(native_value(30, 20, 25), 30);
        assert_eq!(native_value(20, 220, 25), 70);
        assert_eq!(native_value(20, 220, 255), 220);
        assert_eq!(percentage(20, 220, 70), 25);
        assert_eq!(percentage(20, 220, 999), 100);
    }
    #[test]
    fn pending_writes_keep_last_value_and_link_only_supported() {
        let mut p = BTreeMap::new();
        queue(&mut p, &devices(), DisplayCommand::SetLinkedBrightness(60)).unwrap();
        queue(
            &mut p,
            &devices(),
            DisplayCommand::SetBrightness {
                id: "a".into(),
                value: 90,
            },
        )
        .unwrap();
        assert_eq!(
            p.into_iter().collect::<Vec<_>>(),
            vec![("a".to_owned(), 90)]
        );
    }
    #[test]
    fn stale_id_is_recoverable() {
        assert!(
            queue(
                &mut BTreeMap::new(),
                &devices(),
                DisplayCommand::SetBrightness {
                    id: "removed".into(),
                    value: 1
                }
            )
            .is_err()
        );
    }
    #[test]
    fn wmi_identity_matches_target_path() {
        assert_eq!(
            identity(r"\\?\DISPLAY#ABC123#4&1&0&UID1#{guid}"),
            identity(r"DISPLAY\ABC123\4&1&0&UID1_0")
        );
    }
}
