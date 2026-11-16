//! DDC/CI feasibility spike: enumerate monitors, read/set brightness (VCP 0x10),
//! read capabilities, benchmark set latency, query WMI internal brightness.

use std::time::{Duration, Instant};

use windows::Win32::Devices::Display::{
    CapabilitiesRequestAndCapabilitiesReply, DestroyPhysicalMonitor, GetCapabilitiesStringLength,
    GetMonitorBrightness, GetMonitorCapabilities, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, GetVCPFeatureAndVCPFeatureReply, PHYSICAL_MONITOR,
    SetVCPFeature,
};
use windows::Win32::Foundation::{HANDLE, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    CoSetProxyBlanket, CoUninitialize, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::Variant::{VARIANT, VT_BSTR, VT_UI1};
use windows::Win32::System::Wmi::{
    IWbemLocator, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_INFINITE, WbemLocator,
};
use windows::core::{BSTR, w};

const VCP_BRIGHTNESS: u8 = 0x10;

type WinResult<T> = windows::core::Result<T>;

// ---------------------------------------------------------------- RAII guard

/// Owns one physical monitor handle; destroys it on drop.
struct PhysMonitor {
    handle: HANDLE,
    description: String,
}

impl Drop for PhysMonitor {
    fn drop(&mut self) {
        // SAFETY: handle came from GetPhysicalMonitorsFromHMONITOR, destroyed exactly once.
        if let Err(e) = unsafe { DestroyPhysicalMonitor(self.handle) } {
            eprintln!("  warn: DestroyPhysicalMonitor({}): {e}", self.description);
        }
    }
}

/// One entry in the flat monitor list (stable index = position).
struct Entry {
    gdi_name: String,
    rect: RECT,
    primary: bool,
    mon: PhysMonitor,
}

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

// ------------------------------------------------------------- enumeration

fn enum_hmonitors() -> Vec<HMONITOR> {
    unsafe extern "system" fn cb(
        h: HMONITOR,
        _: HDC,
        _: *mut RECT,
        lp: LPARAM,
    ) -> windows::core::BOOL {
        // SAFETY: lp is &mut Vec<HMONITOR> passed below.
        let v = unsafe { &mut *(lp.0 as *mut Vec<HMONITOR>) };
        v.push(h);
        true.into()
    }
    let mut v: Vec<HMONITOR> = Vec::new();
    // SAFETY: callback only uses the pointer during this call.
    let ok =
        unsafe { EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut v as *mut _ as isize)) };
    if !ok.as_bool() {
        eprintln!(
            "EnumDisplayMonitors failed: {}",
            windows::core::Error::from_thread()
        );
    }
    v
}

fn monitor_info(h: HMONITOR) -> WinResult<(String, RECT, bool)> {
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: cbSize announces the EX struct; pointer is layout-prefix compatible.
    unsafe { GetMonitorInfoW(h, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO).ok()? };
    Ok((
        wide_to_string(&mi.szDevice),
        mi.monitorInfo.rcMonitor,
        mi.monitorInfo.dwFlags & 1 != 0,
    ))
}

fn physical_count(h: HMONITOR) -> WinResult<u32> {
    let mut n = 0u32;
    // SAFETY: valid out pointer.
    unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(h, &mut n)? };
    Ok(n)
}

fn physical_monitors(h: HMONITOR, n: u32) -> WinResult<Vec<PhysMonitor>> {
    if n == 0 {
        return Ok(Vec::new());
    }
    let mut arr = vec![PHYSICAL_MONITOR::default(); n as usize];
    // SAFETY: arr has n elements.
    unsafe { GetPhysicalMonitorsFromHMONITOR(h, &mut arr)? };
    Ok(arr
        .iter()
        .map(|p| {
            // copy out of (possibly packed) struct
            let desc = p.szPhysicalMonitorDescription;
            let handle = p.hPhysicalMonitor;
            PhysMonitor {
                handle,
                description: wide_to_string(&desc),
            }
        })
        .collect())
}

/// Build flat list of physical monitors; errors are printed, enumeration continues.
fn collect(verbose: bool) -> Vec<Entry> {
    let mut out = Vec::new();
    for (i, h) in enum_hmonitors().into_iter().enumerate() {
        let (name, rect, primary) = match monitor_info(h) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("HMONITOR #{i}: GetMonitorInfoW failed: {e}");
                continue;
            }
        };
        if verbose {
            println!(
                "HMONITOR #{i}: {name} rect=({},{})-({},{}) primary={primary}",
                rect.left, rect.top, rect.right, rect.bottom
            );
        }
        let n = match physical_count(h) {
            Ok(n) => {
                if verbose {
                    println!("  physical monitors: {n}");
                }
                n
            }
            Err(e) => {
                eprintln!("  GetNumberOfPhysicalMonitorsFromHMONITOR({name}): {e}");
                continue;
            }
        };
        match physical_monitors(h, n) {
            Ok(v) => {
                for mon in v {
                    out.push(Entry {
                        gdi_name: name.clone(),
                        rect,
                        primary,
                        mon,
                    });
                }
            }
            Err(e) => eprintln!("  GetPhysicalMonitorsFromHMONITOR({name}): {e}"),
        }
    }
    out
}

// ------------------------------------------------------------- DDC wrappers

fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let r = f();
    (r, t.elapsed())
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

/// Maps a legacy BOOL-style return (0 = failure) to GetLastError.
fn check(r: i32) -> WinResult<()> {
    if r == 0 {
        Err(windows::core::Error::from_thread())
    } else {
        Ok(())
    }
}

fn hl_brightness(h: HANDLE) -> WinResult<(u32, u32, u32)> {
    let (mut min, mut cur, mut max) = (0u32, 0u32, 0u32);
    // SAFETY: valid out pointers.
    check(unsafe { GetMonitorBrightness(h, &mut min, &mut cur, &mut max) })?;
    Ok((min, cur, max))
}

fn vcp_get(h: HANDLE, code: u8) -> WinResult<(u32, u32)> {
    let (mut cur, mut max) = (0u32, 0u32);
    // SAFETY: valid out pointers; type pointer is optional.
    check(unsafe { GetVCPFeatureAndVCPFeatureReply(h, code, None, &mut cur, Some(&mut max)) })?;
    Ok((cur, max))
}

fn vcp_set(h: HANDLE, code: u8, v: u32) -> WinResult<()> {
    // SAFETY: plain handle + values.
    check(unsafe { SetVCPFeature(h, code, v) })
}

/// Returns (caps string, duration of the request call only).
fn capabilities(h: HANDLE) -> WinResult<(String, Duration)> {
    let mut len = 0u32;
    // SAFETY: valid out pointer.
    check(unsafe { GetCapabilitiesStringLength(h, &mut len) })?;
    let mut buf = vec![0u8; len as usize];
    let (r, d) = timed(|| {
        // SAFETY: buf has len bytes.
        check(unsafe { CapabilitiesRequestAndCapabilitiesReply(h, &mut buf) })
    });
    r?;
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    Ok((String::from_utf8_lossy(&buf[..end]).into_owned(), d))
}

// ------------------------------------------------------------- commands

fn cmd_list() {
    let entries = collect(true);
    println!("\n{} physical monitor(s)", entries.len());
    for (i, e) in entries.iter().enumerate() {
        println!(
            "\n[{i}] \"{}\" on {} primary={} ({}x{})",
            e.mon.description,
            e.gdi_name,
            e.primary,
            e.rect.right - e.rect.left,
            e.rect.bottom - e.rect.top
        );
        let h = e.mon.handle;
        let (r, d) = timed(|| hl_brightness(h));
        match r {
            Ok((mn, c, mx)) => {
                println!(
                    "  GetMonitorBrightness: min={mn} cur={c} max={mx} [{}]",
                    ms(d)
                )
            }
            Err(err) => println!("  GetMonitorBrightness: ERROR {err} [{}]", ms(d)),
        }
        let (r, d) = timed(|| vcp_get(h, VCP_BRIGHTNESS));
        match r {
            Ok((c, mx)) => println!("  VCP 0x10 get: cur={c} max={mx} [{}]", ms(d)),
            Err(err) => println!("  VCP 0x10 get: ERROR {err} [{}]", ms(d)),
        }
        let (mut caps, mut temps) = (0u32, 0u32);
        // SAFETY: valid out pointers. Read-only.
        let ok = unsafe { GetMonitorCapabilities(h, &mut caps, &mut temps) } != 0;
        println!(
            "  GetMonitorCapabilities: ok={ok} caps={caps:#x} (MC_CAPS_BRIGHTNESS 0x2: {}) color temps={temps:#x}",
            caps & 2 != 0
        );
    }
    // entries dropped here -> DestroyPhysicalMonitor
}

fn pick(entries: &[Entry], idx: Option<&String>) -> Option<usize> {
    let Some(i) = idx.and_then(|s| s.parse::<usize>().ok()) else {
        eprintln!("missing/invalid <idx>");
        return None;
    };
    if i >= entries.len() {
        eprintln!("no monitor with idx {i} ({} found)", entries.len());
        return None;
    }
    Some(i)
}

fn cmd_caps(args: &[String]) {
    let entries = collect(false);
    let Some(i) = pick(&entries, args.first()) else {
        return;
    };
    println!(
        "Reading capabilities of [{i}] \"{}\" ...",
        entries[i].mon.description
    );
    let (r, total) = timed(|| capabilities(entries[i].mon.handle));
    match r {
        Ok((s, d)) => println!(
            "caps ({} chars): {s}\nreply call {} (length query does the slow DDC read), total {}",
            s.len(),
            ms(d),
            ms(total)
        ),
        Err(e) => println!("capabilities ERROR: {e} [{}]", ms(total)),
    }
}

fn cmd_set(args: &[String]) {
    let entries = collect(false);
    let Some(i) = pick(&entries, args.first()) else {
        return;
    };
    let Some(v) = args.get(1).and_then(|s| s.parse::<u32>().ok()) else {
        eprintln!("missing/invalid <value>");
        return;
    };
    let h = entries[i].mon.handle;
    match vcp_get(h, VCP_BRIGHTNESS) {
        Ok((c, m)) => println!("before: cur={c} max={m}"),
        Err(e) => println!("before read ERROR: {e}"),
    }
    let (r, d) = timed(|| vcp_set(h, VCP_BRIGHTNESS, v));
    match r {
        Ok(()) => println!("SetVCPFeature(0x10,{v}) ok [{}]", ms(d)),
        Err(e) => println!("SetVCPFeature(0x10,{v}) ERROR: {e} [{}]", ms(d)),
    }
    let (r, d) = timed(|| vcp_get(h, VCP_BRIGHTNESS));
    match r {
        Ok((c, m)) => println!("readback: cur={c} max={m} [{}]", ms(d)),
        Err(e) => println!("readback ERROR: {e}"),
    }
}

fn cmd_bench(args: &[String]) {
    let entries = collect(false);
    let Some(i) = pick(&entries, args.first()) else {
        return;
    };
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let h = entries[i].mon.handle;
    let (orig, max) = match vcp_get(h, VCP_BRIGHTNESS) {
        Ok(x) => x,
        Err(e) => {
            println!("cannot read original brightness, abort: {e}");
            return;
        }
    };
    let alt = if orig < max {
        orig + 1
    } else {
        orig.saturating_sub(1)
    };
    println!("bench [{i}]: n={n} orig={orig} alt={alt}");
    let mut lat: Vec<Duration> = Vec::new();
    let mut errs = 0;
    for k in 0..n {
        // last op always restores original
        let target = if k + 1 == n || k % 2 == 1 { orig } else { alt };
        let (r, d) = timed(|| vcp_set(h, VCP_BRIGHTNESS, target));
        match r {
            Ok(()) => lat.push(d),
            Err(e) => {
                errs += 1;
                println!("  set #{k} -> {target} ERROR: {e}");
            }
        }
    }
    if let (Some(min), Some(mx)) = (lat.iter().min(), lat.iter().max()) {
        let avg = lat.iter().sum::<Duration>() / lat.len() as u32;
        println!(
            "set latency over {} ok ({errs} errors): min={} avg={} max={}",
            lat.len(),
            ms(*min),
            ms(avg),
            ms(*mx)
        );
    }
    if errs > 0 {
        let _ = vcp_set(h, VCP_BRIGHTNESS, orig);
    }
    match vcp_get(h, VCP_BRIGHTNESS) {
        Ok((c, _)) => println!(
            "final value={c} (original {orig}) {}",
            if c == orig { "OK" } else { "MISMATCH" }
        ),
        Err(e) => println!("final read ERROR: {e}"),
    }
}

// ------------------------------------------------------------------- WMI

fn variant_str(v: &VARIANT) -> String {
    // SAFETY: vt checked before union access.
    unsafe {
        let a = &v.Anonymous.Anonymous;
        if a.vt == VT_BSTR {
            a.Anonymous.bstrVal.to_string()
        } else {
            format!("<vt {}>", a.vt.0)
        }
    }
}

fn variant_u8(v: &VARIANT) -> String {
    // SAFETY: vt checked before union access.
    unsafe {
        let a = &v.Anonymous.Anonymous;
        if a.vt == VT_UI1 {
            a.Anonymous.bVal.to_string()
        } else {
            format!("<vt {}>", a.vt.0)
        }
    }
}

/// Ok(true) if at least one instance was printed.
fn wmi_query() -> WinResult<bool> {
    // SAFETY: standard COM/WMI call sequence; all interfaces are RAII wrappers.
    unsafe {
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
        let svc = locator.ConnectServer(
            &BSTR::from("ROOT\\WMI"),
            &BSTR::new(),
            &BSTR::new(),
            &BSTR::new(),
            0,
            &BSTR::new(),
            None,
        )?;
        CoSetProxyBlanket(
            &svc,
            RPC_C_AUTHN_WINNT,
            RPC_C_AUTHZ_NONE,
            None,
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )?;
        let en = svc.ExecQuery(
            &BSTR::from("WQL"),
            &BSTR::from("SELECT * FROM WmiMonitorBrightness"),
            WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
            None,
        )?;
        let mut found = false;
        loop {
            let mut objs = [None];
            let mut ret = 0u32;
            en.Next(WBEM_INFINITE, &mut objs, &mut ret).ok()?;
            let Some(obj) = objs[0].take() else { break };
            if ret == 0 {
                break;
            }
            let mut name = VARIANT::default();
            obj.Get(w!("InstanceName"), 0, &mut name, None, None)?;
            let mut cur = VARIANT::default();
            obj.Get(w!("CurrentBrightness"), 0, &mut cur, None, None)?;
            println!(
                "  InstanceName={} CurrentBrightness={}",
                variant_str(&name),
                variant_u8(&cur)
            );
            found = true;
        }
        Ok(found)
    }
}

fn cmd_wmi() {
    // SAFETY: balanced with CoUninitialize below.
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if hr.is_err() {
        println!("CoInitializeEx failed: {hr:?}");
        return;
    }
    let (r, d) = timed(wmi_query);
    match r {
        Ok(true) => println!("WMI ok [{}]", ms(d)),
        Ok(false) => println!(
            "WMI brightness: not supported (no WmiMonitorBrightness instances) [{}]",
            ms(d)
        ),
        Err(e) => println!("WMI brightness: not supported / error: {e} [{}]", ms(d)),
    }
    // SAFETY: matches successful CoInitializeEx.
    unsafe { CoUninitialize() };
    println!("(WMI set via WmiMonitorBrightnessMethods.WmiSetBrightness not implemented in spike)");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = if args.len() > 1 { &args[1..] } else { &[][..] };
    match args.first().map(String::as_str).unwrap_or("list") {
        "list" => cmd_list(),
        "caps" => cmd_caps(rest),
        "set" => cmd_set(rest),
        "bench" => cmd_bench(rest),
        "wmi" => cmd_wmi(),
        other => eprintln!(
            "unknown command '{other}'. usage: list | caps <idx> | set <idx> <value> | bench <idx> <n> | wmi"
        ),
    }
}
