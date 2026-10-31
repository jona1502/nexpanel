#[derive(Clone, Copy, Debug)]
pub struct Area {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
pub fn popup_position(a: Area, c: (i32, i32), s: (i32, i32)) -> (i32, i32) {
    (
        (c.0 - s.0 / 2).clamp(a.left, (a.right - s.0).max(a.left)),
        (c.1 - s.1 - 8).clamp(a.top, (a.bottom - s.1).max(a.top)),
    )
}
pub fn autostart_command(path: &str) -> String {
    format!("\"{path}\" --background")
}
#[derive(Debug)]
pub struct PopupBounds {
    pub position: (i32, i32),
    pub physical_size: (u32, u32),
    pub logical_size: (f64, f64),
}
pub fn popup_bounds(area: Area, cursor: (i32, i32), scale: f64) -> PopupBounds {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let width = area.right.saturating_sub(area.left).max(1);
    let height = area.bottom.saturating_sub(area.top).max(1);
    let margin_x = if width > 16 { 8 } else { 0 };
    let margin_y = if height > 16 { 8 } else { 0 };
    let inset = Area {
        left: area.left + margin_x,
        top: area.top + margin_y,
        right: area.right - margin_x,
        bottom: area.bottom - margin_y,
    };
    let size = (
        ((400.0 * scale).round() as i32).clamp(1, width - margin_x * 2) as u32,
        ((620.0 * scale).round() as i32).clamp(1, height - margin_y * 2) as u32,
    );
    PopupBounds {
        position: popup_position(inset, cursor, (size.0 as i32, size.1 as i32)),
        physical_size: size,
        logical_size: (size.0 as f64 / scale, size.1 as f64 / scale),
    }
}
#[cfg(windows)]
pub fn position(window: &slint::Window) {
    use slint::winit_030::WinitWindowAccessor;
    use windows::Win32::{
        Foundation::POINT,
        Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
        UI::WindowsAndMessaging::GetCursorPos,
    };
    unsafe {
        let mut point = POINT::default();
        if GetCursorPos(&mut point).is_err() {
            return;
        }
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
        let Some(bounds) = window.with_winit_window(|native| {
            let scale = native
                .available_monitors()
                .find(|m| {
                    let p = m.position();
                    let s = m.size();
                    point.x >= p.x
                        && point.y >= p.y
                        && point.x < p.x + s.width as i32
                        && point.y < p.y + s.height as i32
                })
                .map_or(native.scale_factor(), |m| m.scale_factor());
            let a = info.rcWork;
            popup_bounds(
                Area {
                    left: a.left,
                    top: a.top,
                    right: a.right,
                    bottom: a.bottom,
                },
                (point.x, point.y),
                scale,
            )
        }) else {
            return;
        };
        let size = slint::PhysicalSize::new(bounds.physical_size.0, bounds.physical_size.1);
        if window.size() != size {
            window.set_size(size);
        }
        window.with_winit_window(|native| {
            let desired = slint::winit_030::winit::dpi::PhysicalPosition::new(
                bounds.position.0,
                bounds.position.1,
            );
            if native.outer_position().ok() != Some(desired) {
                native.set_outer_position(desired);
            }
        });
    }
}
#[cfg(not(windows))]
pub fn position(_: &slint::Window) {}
#[cfg(windows)]
pub fn alarm() {
    unsafe {
        let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
            windows::Win32::UI::WindowsAndMessaging::MB_ICONINFORMATION,
        );
    }
}
#[cfg(not(windows))]
pub fn alarm() {}

#[cfg(windows)]
pub fn read_autostart() -> std::io::Result<bool> {
    use winreg::{
        RegKey,
        enums::{HKEY_CURRENT_USER, KEY_READ},
    };
    let key = match RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\Run", KEY_READ)
    {
        Ok(k) => k,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    match key.get_value::<String, _>("NexPanel") {
        Ok(v) => Ok(!v.trim().is_empty()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}
#[cfg(windows)]
pub fn write_autostart(enabled: bool) -> std::io::Result<()> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")?;
    if enabled {
        key.set_value(
            "NexPanel",
            &autostart_command(&std::env::current_exe()?.to_string_lossy()),
        )
    } else {
        match key.delete_value("NexPanel") {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }
}
#[cfg(windows)]
pub type AutostartValue = Option<winreg::RegValue>;
#[cfg(windows)]
pub fn autostart_value() -> std::io::Result<AutostartValue> {
    use winreg::{
        RegKey,
        enums::{HKEY_CURRENT_USER, KEY_READ},
    };
    let key = match RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\Run", KEY_READ)
    {
        Ok(key) => key,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    match key.get_raw_value("NexPanel") {
        Ok(value) => Ok(Some(value)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
#[cfg(windows)]
pub fn restore_autostart(value: AutostartValue) -> std::io::Result<()> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")?;
    match value {
        Some(value) => key.set_raw_value("NexPanel", &value),
        None => match key.delete_value("NexPanel") {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            result => result,
        },
    }
}
#[cfg(not(windows))]
pub fn autostart_value() -> std::io::Result<Option<()>> {
    Ok(None)
}
#[cfg(not(windows))]
pub fn restore_autostart(_: Option<()>) -> std::io::Result<()> {
    Ok(())
}

#[cfg(windows)]
pub fn report_shutdown_error(error: &str) {
    use windows::{
        Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
        core::{PCWSTR, w},
    };
    let message: Vec<u16> = error.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            w!("NexPanel – Einstellungen nicht gespeichert"),
            MB_OK | MB_ICONERROR,
        );
    }
}
#[cfg(not(windows))]
pub fn report_shutdown_error(error: &str) {
    eprintln!("{error}");
}
#[cfg(not(windows))]
pub fn read_autostart() -> std::io::Result<bool> {
    Ok(false)
}
#[cfg(not(windows))]
pub fn write_autostart(_: bool) -> std::io::Result<()> {
    Err(std::io::Error::other("Autostart benötigt Windows"))
}

/// The named objects are scoped to the current Windows logon session.
#[cfg(windows)]
pub struct Instance {
    mutex: windows::Win32::Foundation::HANDLE,
    open: windows::Win32::Foundation::HANDLE,
}
#[cfg(windows)]
impl Instance {
    pub fn acquire() -> Result<Option<Self>, Box<dyn std::error::Error>> {
        use windows::{
            Win32::{
                Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError},
                System::Threading::{CreateEventW, CreateMutexW, SetEvent},
            },
            core::w,
        };
        unsafe {
            let mutex = CreateMutexW(None, false, w!("Local\\NexPanel.Panel.0.1"))?;
            let exists = GetLastError() == ERROR_ALREADY_EXISTS;
            let open = match CreateEventW(None, false, false, w!("Local\\NexPanel.Open.0.1")) {
                Ok(h) => h,
                Err(e) => {
                    let _ = CloseHandle(mutex);
                    return Err(e.into());
                }
            };
            if exists {
                let _ = SetEvent(open);
                let _ = CloseHandle(open);
                let _ = CloseHandle(mutex);
                return Ok(None);
            }
            Ok(Some(Self { mutex, open }))
        }
    }
    pub fn open_requested(&self) -> bool {
        unsafe {
            windows::Win32::System::Threading::WaitForSingleObject(self.open, 0)
                == windows::Win32::Foundation::WAIT_OBJECT_0
        }
    }
}
#[cfg(windows)]
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.open);
            let _ = windows::Win32::Foundation::CloseHandle(self.mutex);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn high_dpi_popup_fits_work_area_with_margin() {
        let area = Area {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        let bounds = popup_bounds(area, (1900, 1030), 2.0);
        assert!(bounds.logical_size.1 <= 512.0);
        assert_eq!(bounds.logical_size.0, 400.0);
        assert!(bounds.position.0 >= 8 && bounds.position.1 >= 8);
        assert!(bounds.position.0 + bounds.physical_size.0 as i32 <= area.right - 8);
        assert!(bounds.position.1 + bounds.physical_size.1 as i32 <= area.bottom - 8);
    }
    #[test]
    fn popup_bounds_fit_negative_monitor_with_top_taskbar() {
        let area = Area {
            left: -1920,
            top: 64,
            right: 0,
            bottom: 1080,
        };
        let bounds = popup_bounds(area, (-1900, 0), 2.0);
        assert_eq!(bounds.position.0, -1912);
        assert_eq!(bounds.position.1, 72);
        assert!(bounds.position.1 + bounds.physical_size.1 as i32 <= 1072);
        let normal = popup_bounds(area, (-50, 1000), 1.0);
        assert_eq!(normal.logical_size, (400.0, 620.0));
        assert_eq!(normal.physical_size, (400, 620));
    }
    #[test]
    fn clamps_negative_monitor_and_left_taskbar() {
        assert_eq!(
            popup_position(
                Area {
                    left: -1840,
                    top: 0,
                    right: 0,
                    bottom: 1080
                },
                (-1900, 1080),
                (400, 620)
            ),
            (-1840, 452)
        );
    }
    #[test]
    fn respects_top_taskbar_and_small_work_area() {
        assert_eq!(
            popup_position(
                Area {
                    left: 0,
                    top: 48,
                    right: 300,
                    bottom: 500
                },
                (20, 0),
                (400, 620)
            ),
            (0, 48)
        );
    }
    #[test]
    fn quotes_autostart_path() {
        assert_eq!(
            autostart_command(r"C:\Program Files\NexPanel\nexpanel.exe"),
            r#""C:\Program Files\NexPanel\nexpanel.exe" --background"#
        );
    }
}
