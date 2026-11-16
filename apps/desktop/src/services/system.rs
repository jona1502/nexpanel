//! System monitor: CPU, RAM, GPU load and VRAM. Sampled once per second, only while the panel is
//! visible and the System tab is active. While inactive the worker blocks on its channel
//! (no timeout, no wakeups) and holds no PDH handles.
use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

/// Delay between activation and the first value (CPU/PDH need two collections for a delta).
pub const PRIME_DELAY: Duration = Duration::from_millis(250);
/// Sampling period while active.
pub const PERIOD: Duration = Duration::from_secs(1);
/// The PDH wildcard set is rebuilt every n-th sample: process engines come and go.
#[cfg(windows)]
const REEXPAND_EVERY: u32 = 10;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SystemSnapshot {
    /// Whole percent.
    pub cpu_pct: f32,
    pub ram_used_mb: u32,
    pub ram_total_mb: u32,
    /// Whole percent; `None` when no GPU counters exist.
    pub gpu_pct: Option<f32>,
    pub vram_used_mb: Option<u32>,
    pub vram_total_mb: Option<u32>,
    pub gpu_name: Option<String>,
    pub status: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemCommand {
    /// True only while the panel is visible and the System tab is shown.
    SetActive(bool),
    Shutdown,
}

pub fn spawn() -> (
    Sender<SystemCommand>,
    Receiver<SystemSnapshot>,
    crate::logging::CrashFlag,
) {
    let (tx, rx) = mpsc::channel();
    let (out, snap) = mpsc::channel();
    let crashed = crate::logging::CrashFlag::default();
    crate::logging::spawn_worker("nexpanel-system", &crashed, move || {
        run_loop(rx, out, native::Sampler::default(), PRIME_DELAY, PERIOD)
    });
    (tx, snap, crashed)
}

/// Source of measurements; the OS implementation lives in `native`.
pub trait Sampler {
    /// Panel became active: prime counters.
    fn start(&mut self);
    fn sample(&mut self) -> SystemSnapshot;
    /// Panel became inactive: release handles.
    fn stop(&mut self);
}

/// Blocks on `rx` while inactive. While active it wakes once per `period` (first after `prime`)
/// and sends a snapshot only when it differs from the last one sent.
pub fn run_loop(
    rx: Receiver<SystemCommand>,
    out: Sender<SystemSnapshot>,
    mut sampler: impl Sampler,
    prime: Duration,
    period: Duration,
) {
    let mut sent: Option<SystemSnapshot> = None;
    let mut active = false;
    let mut first = true;
    loop {
        let command = if active {
            match rx.recv_timeout(if first { prime } else { period }) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => break,
            }
        };
        match command {
            Some(SystemCommand::Shutdown) => break,
            Some(SystemCommand::SetActive(true)) if !active => {
                active = true;
                first = true;
                sampler.start();
            }
            Some(SystemCommand::SetActive(false)) if active => {
                active = false;
                sampler.stop();
            }
            Some(_) => {}
            None => {
                first = false;
                let snapshot = sampler.sample();
                if sent.as_ref() != Some(&snapshot) {
                    if out.send(snapshot.clone()).is_err() {
                        break;
                    }
                    sent = Some(snapshot);
                }
            }
        }
    }
    sampler.stop();
}

// ---------------------------------------------------------------- pure logic

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTimes {
    pub idle: u64,
    /// Includes idle time (GetSystemTimes semantics).
    pub kernel: u64,
    pub user: u64,
}
/// Busy share between two readings in whole percent. Counter wraparound is handled modulo 2^64,
/// an empty interval or inconsistent idle value yields 0.
pub fn cpu_percent(prev: CpuTimes, now: CpuTimes) -> f32 {
    let total = now
        .kernel
        .wrapping_sub(prev.kernel)
        .wrapping_add(now.user.wrapping_sub(prev.user));
    if total == 0 {
        return 0.0;
    }
    let idle = now.idle.wrapping_sub(prev.idle).min(total);
    ((total - idle) as f64 * 100.0 / total as f64)
        .round()
        .clamp(0.0, 100.0) as f32
}

/// RAM rounded to 0.1 GB (in MB) so tiny fluctuations do not produce new snapshots.
pub fn round_ram_mb(mb: u64) -> u32 {
    let tenth_gb = (mb as f64 / 1024.0 * 10.0).round();
    (tenth_gb * 1024.0 / 10.0).round().min(u32::MAX as f64) as u32
}
/// German number format with one decimal; a trailing ",0" is dropped (12 not 12,0).
pub fn format_gb(mb: u32) -> String {
    let tenths = (mb as f64 / 1024.0 * 10.0).round() as u64;
    if tenths.is_multiple_of(10) {
        format!("{}", tenths / 10)
    } else {
        format!("{},{}", tenths / 10, tenths % 10)
    }
}
pub fn percent_of(used: u32, total: u32) -> i32 {
    if total == 0 {
        0
    } else {
        (used as f64 * 100.0 / total as f64)
            .round()
            .clamp(0.0, 100.0) as i32
    }
}
/// 0 normal (accent), 1 amber from 80 %, 2 red from 95 %.
pub fn bar_level(pct: i32) -> i32 {
    match pct {
        95.. => 2,
        80.. => 1,
        _ => 0,
    }
}

/// Windows adapter identity (LUID), as in PDH instance names `luid_0xHIGH_0xLOW_...`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AdapterId {
    pub high: u32,
    pub low: u32,
}
pub fn parse_adapter(instance: &str) -> Option<AdapterId> {
    let rest = &instance[instance.find("luid_")? + 5..];
    let mut parts = rest.split('_');
    let hex = |p: Option<&str>| u32::from_str_radix(p?.strip_prefix("0x")?, 16).ok();
    Some(AdapterId {
        high: hex(parts.next())?,
        low: hex(parts.next())?,
    })
}
/// `pid_123_luid_0x..._0x..._phys_0_eng_0_engtype_3D` -> (adapter, "3D").
pub fn parse_engine(instance: &str) -> Option<(AdapterId, String)> {
    let kind = &instance[instance.find("engtype_")? + 8..];
    if kind.is_empty() {
        return None;
    }
    Some((parse_adapter(instance)?, kind.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuAdapter {
    pub id: AdapterId,
    pub name: String,
    pub vram_total_mb: Option<u32>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct GpuReading {
    pub pct: f32,
    pub vram_used_mb: Option<u32>,
    pub vram_total_mb: Option<u32>,
    pub name: Option<String>,
}
/// Per adapter: sum per engine type, maximum over the types. Headline adapter: highest whole-percent
/// load, ties by most dedicated VRAM, then lowest LUID (stable). `engines` are
/// (instance name, percent), `memory` are (instance name, dedicated bytes).
pub fn aggregate_gpu(
    engines: &[(String, f64)],
    memory: &[(String, f64)],
    adapters: &[GpuAdapter],
) -> Option<GpuReading> {
    let mut per_type: BTreeMap<AdapterId, BTreeMap<String, f64>> = BTreeMap::new();
    for (name, value) in engines {
        if let Some((adapter, kind)) = parse_engine(name)
            && value.is_finite()
        {
            *per_type
                .entry(adapter)
                .or_default()
                .entry(kind)
                .or_default() += value.max(0.0);
        }
    }
    let total_of = |id: &AdapterId| {
        adapters
            .iter()
            .find(|a| a.id == *id)
            .and_then(|a| a.vram_total_mb)
    };
    let load = |id: &AdapterId| {
        per_type[id]
            .values()
            .fold(0.0_f64, |m, v| m.max(*v))
            .clamp(0.0, 100.0)
            .round()
    };
    let best = per_type.keys().copied().max_by(|a, b| {
        load(a)
            .total_cmp(&load(b))
            .then(total_of(a).unwrap_or(0).cmp(&total_of(b).unwrap_or(0)))
            .then(b.cmp(a))
    })?;
    let mut used_bytes: Option<f64> = None;
    for (name, value) in memory {
        if parse_adapter(name) == Some(best) && value.is_finite() {
            *used_bytes.get_or_insert(0.0) += value.max(0.0);
        }
    }
    Some(GpuReading {
        pct: load(&best) as f32,
        vram_used_mb: used_bytes.map(|b| (b / 1_048_576.0).round().min(u32::MAX as f64) as u32),
        vram_total_mb: total_of(&best),
        name: adapters
            .iter()
            .find(|a| a.id == best)
            .map(|a| a.name.clone()),
    })
}

pub fn build_snapshot(
    cpu_pct: f32,
    ram_total_bytes: u64,
    ram_avail_bytes: u64,
    gpu: Option<GpuReading>,
) -> SystemSnapshot {
    let total_mb = ram_total_bytes / 1_048_576;
    let used_mb = total_mb.saturating_sub(ram_avail_bytes / 1_048_576);
    SystemSnapshot {
        cpu_pct: cpu_pct.round(),
        ram_used_mb: round_ram_mb(used_mb),
        ram_total_mb: round_ram_mb(total_mb),
        gpu_pct: gpu.as_ref().map(|g| g.pct.round()),
        vram_used_mb: gpu.as_ref().and_then(|g| g.vram_used_mb),
        vram_total_mb: gpu.as_ref().and_then(|g| g.vram_total_mb),
        gpu_name: gpu.and_then(|g| g.name),
        status: String::new(),
    }
}

/// Everything the UI shows, preformatted (the UI thread only copies it into properties).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemView {
    pub cpu_pct: i32,
    pub cpu_text: String,
    pub cpu_level: i32,
    pub ram_pct: i32,
    pub ram_text: String,
    pub ram_level: i32,
    pub gpu_available: bool,
    pub gpu_pct: i32,
    pub gpu_text: String,
    pub gpu_level: i32,
    pub vram_pct: i32,
    pub vram_text: String,
    pub vram_level: i32,
    pub gpu_name: String,
}
pub fn view(s: &SystemSnapshot) -> SystemView {
    let cpu = s.cpu_pct.round() as i32;
    let ram = percent_of(s.ram_used_mb, s.ram_total_mb);
    let gpu = s.gpu_pct.map(|g| g.round() as i32);
    let vram = match (s.vram_used_mb, s.vram_total_mb) {
        (Some(u), Some(t)) => percent_of(u, t),
        _ => 0,
    };
    SystemView {
        cpu_pct: cpu,
        cpu_text: format!("{cpu} %"),
        cpu_level: bar_level(cpu),
        ram_pct: ram,
        ram_text: format!(
            "{} / {} GB  {ram} %",
            format_gb(s.ram_used_mb),
            format_gb(s.ram_total_mb)
        ),
        ram_level: bar_level(ram),
        gpu_available: gpu.is_some(),
        gpu_pct: gpu.unwrap_or(0),
        gpu_text: gpu.map_or("nicht verfügbar".into(), |g| format!("{g} %")),
        gpu_level: gpu.map_or(0, bar_level),
        vram_pct: vram,
        vram_text: match (gpu, s.vram_used_mb, s.vram_total_mb) {
            (None, ..) | (_, None, _) => "—".into(),
            (_, Some(u), Some(t)) => format!("{} / {} GB", format_gb(u), format_gb(t)),
            (_, Some(u), None) => format!("{} GB", format_gb(u)),
        },
        vram_level: bar_level(vram),
        gpu_name: s.gpu_name.clone().unwrap_or_default(),
    }
}

// ---------------------------------------------------------------- OS access

#[cfg(windows)]
mod native {
    use super::*;
    use windows::{
        Win32::{
            Foundation::FILETIME,
            Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1},
            System::{
                Performance::*,
                SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
                Threading::GetSystemTimes,
            },
        },
        core::{PCWSTR, w},
    };

    fn filetime(f: FILETIME) -> u64 {
        (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)
    }
    fn cpu_times() -> Option<CpuTimes> {
        let (mut idle, mut kernel, mut user) = Default::default();
        // SAFETY: valid out pointers to local FILETIMEs.
        unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
        Some(CpuTimes {
            idle: filetime(idle),
            kernel: filetime(kernel),
            user: filetime(user),
        })
    }
    fn memory() -> Option<(u64, u64)> {
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        // SAFETY: dwLength is set, pointer valid.
        unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
        Some((status.ullTotalPhys, status.ullAvailPhys))
    }
    /// Hardware adapters with name and dedicated VRAM; empty when DXGI fails.
    fn adapters() -> Vec<GpuAdapter> {
        let mut list = Vec::new();
        // SAFETY: plain DXGI enumeration, COM objects are released on drop.
        unsafe {
            let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
                return list;
            };
            let mut index = 0;
            while let Ok(adapter) = factory.EnumAdapters1(index) {
                index += 1;
                let Ok(desc) = adapter.GetDesc1() else {
                    continue;
                };
                if desc.Flags & 2 != 0 {
                    continue; // DXGI_ADAPTER_FLAG_SOFTWARE (Basic Render Driver)
                }
                let end = desc
                    .Description
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(desc.Description.len());
                list.push(GpuAdapter {
                    id: AdapterId {
                        high: desc.AdapterLuid.HighPart as u32,
                        low: desc.AdapterLuid.LowPart,
                    },
                    name: String::from_utf16_lossy(&desc.Description[..end]),
                    vram_total_mb: u32::try_from(desc.DedicatedVideoMemory / 1_048_576)
                        .ok()
                        .filter(|&m| m > 0),
                });
            }
        }
        list
    }

    /// PDH query with the two wildcard GPU counters. Closed on drop.
    struct PdhSet {
        query: PDH_HQUERY,
        engine: PDH_HCOUNTER,
        memory: Option<PDH_HCOUNTER>,
    }
    impl PdhSet {
        /// Opens the query and takes the baseline collection.
        fn open() -> Result<Self, u32> {
            let mut query = PDH_HQUERY(std::ptr::null_mut());
            // SAFETY: out pointer is valid; the handle is closed in Drop.
            let status = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) };
            if status != 0 {
                return Err(status);
            }
            let mut set = PdhSet {
                query,
                engine: PDH_HCOUNTER(std::ptr::null_mut()),
                memory: None,
            };
            // SAFETY: valid query and static wide strings.
            let status = unsafe {
                PdhAddEnglishCounterW(
                    query,
                    w!("\\GPU Engine(*)\\Utilization Percentage"),
                    0,
                    &mut set.engine,
                )
            };
            if status != 0 {
                return Err(status); // Drop closes the query
            }
            let mut memory = PDH_HCOUNTER(std::ptr::null_mut());
            // SAFETY: as above. VRAM is optional; load works without it.
            let status = unsafe {
                PdhAddEnglishCounterW(
                    query,
                    w!("\\GPU Adapter Memory(*)\\Dedicated Usage"),
                    0,
                    &mut memory,
                )
            };
            if status == 0 {
                set.memory = Some(memory);
            }
            set.collect();
            Ok(set)
        }
        fn collect(&self) -> bool {
            // SAFETY: valid query handle.
            unsafe { PdhCollectQueryData(self.query) == 0 }
        }
        /// (instance, value) pairs; empty while no valid delta exists yet
        /// (first collection: PDH_CALC_NEGATIVE_DENOMINATOR / invalid data) or on error.
        fn read(&self, counter: PDH_HCOUNTER) -> Vec<(String, f64)> {
            let (mut size, mut count) = (0u32, 0u32);
            // SAFETY: size query with a null buffer is the documented protocol.
            let status = unsafe {
                PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None)
            };
            if status != PDH_MORE_DATA {
                return Vec::new();
            }
            for _ in 0..4 {
                // u64 elements: 8-byte alignment as required by the item struct.
                let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
                let items = buffer.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
                // SAFETY: buffer holds at least `size` bytes; PDH writes `count` items plus
                // their name strings into it; they are read before the buffer drops.
                let status = unsafe {
                    PdhGetFormattedCounterArrayW(
                        counter,
                        PDH_FMT_DOUBLE,
                        &mut size,
                        &mut count,
                        Some(items),
                    )
                };
                if status == PDH_MORE_DATA {
                    continue; // instance count grew between the calls
                }
                if status != 0 {
                    return Vec::new();
                }
                // SAFETY: see above; `count` items were written.
                let items = unsafe { std::slice::from_raw_parts(items, count as usize) };
                return items
                    .iter()
                    .filter(|i| {
                        i.FmtValue.CStatus == PDH_CSTATUS_VALID_DATA
                            || i.FmtValue.CStatus == PDH_CSTATUS_NEW_DATA
                    })
                    .filter_map(|i| {
                        // SAFETY: name is a NUL-terminated string inside `buffer`; the value
                        // was requested as PDH_FMT_DOUBLE.
                        let name = unsafe { i.szName.to_string() }.ok()?;
                        Some((name, unsafe { i.FmtValue.Anonymous.doubleValue }))
                    })
                    .collect();
            }
            Vec::new()
        }
    }
    impl Drop for PdhSet {
        fn drop(&mut self) {
            // SAFETY: closes the query and its counters.
            unsafe { PdhCloseQuery(self.query) };
        }
    }

    #[derive(Default)]
    pub struct Sampler {
        cpu: Option<CpuTimes>,
        pdh: Option<PdhSet>,
        adapters: Option<Vec<GpuAdapter>>,
        samples: u32,
        last_gpu: Option<GpuReading>,
        warned: bool,
    }
    impl super::Sampler for Sampler {
        fn start(&mut self) {
            self.cpu = cpu_times();
            self.samples = 0;
            self.adapters.get_or_insert_with(adapters);
            self.pdh = match PdhSet::open() {
                Ok(set) => Some(set),
                Err(status) => {
                    log::warn!("GPU-Zähler nicht verfügbar (PDH 0x{status:08X})");
                    None
                }
            };
        }
        fn sample(&mut self) -> SystemSnapshot {
            let cpu = match (self.cpu, cpu_times()) {
                (Some(prev), Some(now)) => {
                    self.cpu = Some(now);
                    cpu_percent(prev, now)
                }
                (_, now) => {
                    self.cpu = now;
                    0.0
                }
            };
            let (total, avail) = memory().unwrap_or((0, 0));
            if let Some(set) = &self.pdh {
                self.samples += 1;
                // Rebuild (and prime) a fresh set now, swap after reading from the old one:
                // no gap in values, new process engines get picked up.
                let fresh = self
                    .samples
                    .is_multiple_of(REEXPAND_EVERY)
                    .then(|| PdhSet::open().ok())
                    .flatten();
                if set.collect() {
                    let engines = set.read(set.engine);
                    let memory = set.memory.map(|m| set.read(m)).unwrap_or_default();
                    let adapters = self.adapters.as_deref().unwrap_or_default();
                    match aggregate_gpu(&engines, &memory, adapters) {
                        Some(reading) => self.last_gpu = Some(reading),
                        None if self.samples >= 2 && self.last_gpu.is_none() && !self.warned => {
                            self.warned = true;
                            log::warn!("GPU-Zähler liefern keine Engine-Werte");
                        }
                        None => {}
                    }
                }
                if let Some(fresh) = fresh {
                    self.pdh = Some(fresh);
                }
            }
            let gpu = self.pdh.as_ref().and_then(|_| self.last_gpu.clone());
            build_snapshot(cpu, total, avail, gpu)
        }
        fn stop(&mut self) {
            self.pdh = None;
            self.cpu = None;
            self.last_gpu = None;
        }
    }
}
#[cfg(not(windows))]
mod native {
    #[derive(Default)]
    pub struct Sampler;
    impl super::Sampler for Sampler {
        fn start(&mut self) {}
        fn sample(&mut self) -> super::SystemSnapshot {
            super::SystemSnapshot::default()
        }
        fn stop(&mut self) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    fn t(idle: u64, kernel: u64, user: u64) -> CpuTimes {
        CpuTimes { idle, kernel, user }
    }
    #[test]
    fn cpu_delta_table() {
        let cases = [
            (t(0, 0, 0), t(75, 100, 100), 63.0), // busy 125 of 200
            (t(0, 0, 0), t(100, 100, 0), 0.0),   // fully idle
            (t(0, 0, 0), t(0, 50, 50), 100.0),
            (t(5, 10, 10), t(5, 10, 10), 0.0), // empty interval: no division by zero
            (t(0, 0, 0), t(500, 100, 100), 0.0), // idle > total: clamped
            (t(10, u64::MAX - 49, 0), t(20, 50, 50), 93.0), // kernel wrapped: delta 100 + 50, idle 10
        ];
        for (prev, now, want) in cases {
            assert_eq!(cpu_percent(prev, now), want, "{prev:?} -> {now:?}");
        }
    }
    #[test]
    fn ram_rounding_and_format() {
        assert_eq!(format_gb(9626), "9,4");
        assert_eq!(format_gb(32665), "31,9");
        assert_eq!(format_gb(12288), "12");
        assert_eq!(format_gb(0), "0");
        assert_eq!(format_gb(1843), "1,8");
        assert_eq!(round_ram_mb(9600), round_ram_mb(9620));
        assert_ne!(round_ram_mb(9600), round_ram_mb(9800));
        assert_eq!(round_ram_mb(0), 0);
        assert_eq!(percent_of(9626, 32665), 29);
        assert_eq!(percent_of(1, 0), 0);
        assert_eq!(percent_of(5, 3), 100);
    }
    #[test]
    fn bar_levels() {
        for (pct, level) in [(0, 0), (79, 0), (80, 1), (94, 1), (95, 2), (100, 2)] {
            assert_eq!(bar_level(pct), level, "{pct}");
        }
    }
    #[test]
    fn instance_name_parsing() {
        let id = AdapterId {
            high: 0,
            low: 0xD8C5,
        };
        assert_eq!(
            parse_engine("pid_4242_luid_0x00000000_0x0000D8C5_phys_0_eng_3_engtype_3D"),
            Some((id, "3D".to_string()))
        );
        assert_eq!(
            parse_engine("pid_1_luid_0x00000000_0x0000d8c5_phys_0_eng_0_engtype_VideoDecode"),
            Some((id, "VideoDecode".to_string()))
        );
        assert_eq!(
            parse_adapter("luid_0x00000001_0x0000FFFF_phys_0"),
            Some(AdapterId {
                high: 1,
                low: 0xFFFF
            })
        );
        for bad in [
            "",
            "pid_1_phys_0",
            "luid_0xZZ_0x1",
            "luid_0x1",
            "pid_1_luid_0x0_0x1_phys_0_eng_0",
        ] {
            assert_eq!(parse_engine(bad), None, "{bad}");
        }
    }
    fn engine(luid: u32, kind: &str, pid: u32) -> String {
        format!("pid_{pid}_luid_0x00000000_0x{luid:08X}_phys_0_eng_0_engtype_{kind}")
    }
    fn adapter(luid: u32, name: &str, mb: Option<u32>) -> GpuAdapter {
        GpuAdapter {
            id: AdapterId { high: 0, low: luid },
            name: name.into(),
            vram_total_mb: mb,
        }
    }
    fn mem(luid: u32, bytes: f64) -> (String, f64) {
        (format!("luid_0x00000000_0x{luid:08X}_phys_0"), bytes)
    }
    #[test]
    fn gpu_aggregation() {
        let adapters = [
            adapter(1, "iGPU", Some(128)),
            adapter(2, "dGPU", Some(12288)),
        ];
        // sum per type over processes, max over types: 3D = 30+25, Copy = 70 -> 70
        let engines = vec![
            (engine(2, "3D", 10), 30.0),
            (engine(2, "3D", 11), 25.0),
            (engine(2, "Copy", 10), 70.0),
            (engine(1, "3D", 10), 5.0),
        ];
        let memory = vec![mem(2, 1.8 * 1_073_741_824.0), mem(1, 50.0 * 1_048_576.0)];
        let r = aggregate_gpu(&engines, &memory, &adapters).unwrap();
        assert_eq!(r.pct, 70.0);
        assert_eq!(r.name.as_deref(), Some("dGPU"));
        assert_eq!(r.vram_total_mb, Some(12288));
        assert_eq!(r.vram_used_mb, Some(1843));
        // sums above 100 are clamped
        let r = aggregate_gpu(
            &[(engine(2, "3D", 1), 80.0), (engine(2, "3D", 2), 80.0)],
            &[],
            &adapters,
        )
        .unwrap();
        assert_eq!((r.pct, r.vram_used_mb), (100.0, None));
        // busiest adapter wins
        let engines = vec![(engine(2, "3D", 1), 3.0), (engine(1, "3D", 1), 40.0)];
        let r = aggregate_gpu(&engines, &memory, &adapters).unwrap();
        assert_eq!(
            (r.pct, r.name.as_deref(), r.vram_used_mb),
            (40.0, Some("iGPU"), Some(50))
        );
        // tie (even 0 %): adapter with most VRAM, independent of input order
        for engines in [
            vec![(engine(1, "3D", 1), 0.0), (engine(2, "3D", 1), 0.0)],
            vec![(engine(2, "3D", 1), 0.0), (engine(1, "3D", 1), 0.0)],
        ] {
            let r = aggregate_gpu(&engines, &memory, &adapters).unwrap();
            assert_eq!(r.name.as_deref(), Some("dGPU"));
        }
        // unknown adapter: only used; no engines / garbage: None
        let r = aggregate_gpu(
            &[(engine(9, "3D", 1), 4.4)],
            &[mem(9, 2_097_152.0)],
            &adapters,
        )
        .unwrap();
        assert_eq!(
            (r.vram_total_mb, r.vram_used_mb, r.name),
            (None, Some(2), None)
        );
        assert_eq!(aggregate_gpu(&[], &memory, &adapters), None);
        assert_eq!(
            aggregate_gpu(
                &[("garbage".into(), 5.0), (engine(1, "3D", 1), f64::NAN)],
                &[],
                &adapters
            ),
            None
        );
    }
    #[test]
    fn view_texts() {
        let snap = SystemSnapshot {
            cpu_pct: 12.0,
            ram_used_mb: 9626,
            ram_total_mb: 32665,
            gpu_pct: Some(96.0),
            vram_used_mb: Some(1843),
            vram_total_mb: Some(12288),
            gpu_name: Some("GPU".into()),
            status: String::new(),
        };
        let v = view(&snap);
        assert_eq!(v.cpu_text, "12 %");
        assert_eq!(v.ram_text, "9,4 / 31,9 GB  29 %");
        assert_eq!((v.gpu_text.as_str(), v.gpu_level), ("96 %", 2));
        assert_eq!(v.vram_text, "1,8 / 12 GB");
        assert_eq!(v.vram_pct, 15);
        let none = view(&SystemSnapshot::default());
        assert!(!none.gpu_available);
        assert_eq!(
            (none.gpu_text.as_str(), none.vram_text.as_str()),
            ("nicht verfügbar", "—")
        );
        let only_used = view(&SystemSnapshot {
            gpu_pct: Some(1.0),
            vram_used_mb: Some(1843),
            ..Default::default()
        });
        assert_eq!(only_used.vram_text, "1,8 GB");
    }
    #[test]
    fn snapshot_build_rounds() {
        let gb = 1_073_741_824u64;
        let s = build_snapshot(12.4, 32 * gb, 22 * gb, None);
        assert_eq!((s.cpu_pct, s.ram_total_mb, s.gpu_pct), (12.0, 32768, None));
        assert_eq!(s.ram_used_mb, round_ram_mb(10240));
        // avail > total (race): no underflow
        assert_eq!(build_snapshot(0.0, gb, 2 * gb, None).ram_used_mb, 0);
    }

    struct Fake {
        calls: Arc<AtomicU32>,
        starts: Arc<AtomicU32>,
        values: Vec<f32>,
    }
    impl Sampler for Fake {
        fn start(&mut self) {
            self.starts.fetch_add(1, Ordering::SeqCst);
        }
        fn sample(&mut self) -> SystemSnapshot {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) as usize;
            SystemSnapshot {
                cpu_pct: self.values[n.min(self.values.len() - 1)],
                ..Default::default()
            }
        }
        fn stop(&mut self) {}
    }
    #[test]
    fn worker_samples_only_while_active_and_sends_only_changes() {
        let (tx, rx) = mpsc::channel();
        let (out, snaps) = mpsc::channel();
        let calls = Arc::new(AtomicU32::new(0));
        let starts = Arc::new(AtomicU32::new(0));
        let fake = Fake {
            calls: calls.clone(),
            starts: starts.clone(),
            values: vec![5.0, 5.0, 5.0, 9.0],
        };
        let worker = std::thread::spawn(move || {
            run_loop(
                rx,
                out,
                fake,
                Duration::from_millis(10),
                Duration::from_millis(20),
            )
        });
        // Inactive: nothing happens.
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        tx.send(SystemCommand::SetActive(true)).unwrap();
        tx.send(SystemCommand::SetActive(true)).unwrap(); // duplicate ignored
        let first = snaps.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(first.cpu_pct, 5.0);
        // repeated 5.0 are not sent, 9.0 arrives next.
        let second = snaps.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(second.cpu_pct, 9.0);
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        tx.send(SystemCommand::SetActive(false)).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        let after_stop = calls.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            after_stop,
            "no sampling while inactive"
        );
        tx.send(SystemCommand::Shutdown).unwrap();
        worker.join().unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn live_sample_is_plausible() {
        let mut s = native::Sampler::default();
        s.start();
        std::thread::sleep(PRIME_DELAY);
        let snap = s.sample();
        println!("{snap:?}\n{:?}", view(&snap));
        assert!(snap.ram_total_mb > 0 && snap.ram_used_mb <= snap.ram_total_mb);
        assert!((0.0..=100.0).contains(&snap.cpu_pct));
        s.stop();
    }
}
