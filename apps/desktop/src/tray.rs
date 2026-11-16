//! Own Win32 tray host (ADR-001).
//!
//! Replaces Slint's `SystemTrayIcon`, whose Windows backend owns an `HWND_MESSAGE` window and
//! therefore never receives the `TaskbarCreated` broadcast. The host owns a hidden *top-level*
//! window on the UI thread. winit's event loop runs a regular `GetMessage`/`DispatchMessage`
//! pump for the whole thread, so the host's window procedure is called from `slint::run_event_loop`
//! without an extra thread.

/// What the tray asks the application to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// Left click / keyboard activation: open or close the popup.
    Toggle,
    /// Context menu "Öffnen".
    Open,
    /// Context menu "Einstellungen": open the popup on the settings tab.
    Settings,
    /// Context menu "Mit Windows starten": flip the autostart setting.
    ToggleAutostart,
    /// Context menu "Beenden".
    Quit,
    /// User clicked the balloon/toast (`NIN_BALLOONUSERCLICK`).
    NotificationClicked,
    /// System resumed from standby/hibernate: timers must be re-checked immediately.
    Resumed,
    /// Monitor hotplug, resolution or layout change (`WM_DISPLAYCHANGE` / `DBT_DEVNODES_CHANGED`).
    DisplayChange,
}

const NIN_BALLOONUSERCLICK: u32 = 0x405;
const WM_POWERBROADCAST: u32 = 0x0218;
const PBT_APMRESUMESUSPEND: usize = 0x0007;
const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;

/// `WM_POWERBROADCAST` `wParam` values that mean "just woke up".
fn power_event(wparam: usize) -> Option<TrayEvent> {
    matches!(wparam, PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND).then_some(TrayEvent::Resumed)
}

/// Copies `s` into a NUL terminated UTF-16 buffer, truncating to `N - 1` units.
fn wide<const N: usize>(s: &str) -> [u16; N] {
    let mut out = [0u16; N];
    for (slot, unit) in out.iter_mut().zip(s.encode_utf16().take(N - 1)) {
        *slot = unit;
    }
    out
}

const WM_DISPLAYCHANGE: u32 = 0x007E;
const WM_DEVICECHANGE: u32 = 0x0219;
const DBT_DEVNODES_CHANGED: usize = 0x0007;

/// Messages that may mean the monitor topology changed.
fn is_display_change(msg: u32, wparam: usize) -> bool {
    msg == WM_DISPLAYCHANGE || (msg == WM_DEVICECHANGE && wparam == DBT_DEVNODES_CHANGED)
}

/// Shell notification codes (`NIN_SELECT`, `NIN_KEYSELECT`) and legacy mouse messages.
const NIN_SELECT: u32 = 0x400;
const NIN_KEYSELECT: u32 = 0x401;
const WM_CONTEXTMENU: u32 = 0x007B;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONUP: u32 = 0x0205;
const ID_OPEN: u32 = 1;
const ID_QUIT: u32 = 2;
const ID_SETTINGS: u32 = 3;
const ID_AUTOSTART: u32 = 4;
/// Two activations closer than this are one user action (Space can send `NIN_SELECT` and
/// `NIN_KEYSELECT`).
const ACTIVATION_GAP_MS: u64 = 120;
const MAX_RETRIES: u8 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gesture {
    Activate,
    ContextMenu,
    Balloon,
}

/// `LOWORD(lParam)` is the event for `NOTIFYICON_VERSION_4`; the legacy protocol sends the
/// mouse message itself.
fn decode(v4: bool, lparam: isize) -> Option<Gesture> {
    let code = if v4 {
        lparam as u32 & 0xFFFF
    } else {
        lparam as u32
    };
    match code {
        NIN_SELECT | NIN_KEYSELECT => Some(Gesture::Activate),
        WM_CONTEXTMENU => Some(Gesture::ContextMenu),
        NIN_BALLOONUSERCLICK => Some(Gesture::Balloon),
        WM_LBUTTONUP if !v4 => Some(Gesture::Activate),
        WM_RBUTTONUP if !v4 => Some(Gesture::ContextMenu),
        _ => None,
    }
}

fn menu_event(id: u32) -> Option<TrayEvent> {
    match id {
        ID_OPEN => Some(TrayEvent::Open),
        ID_SETTINGS => Some(TrayEvent::Settings),
        ID_AUTOSTART => Some(TrayEvent::ToggleAutostart),
        ID_QUIT => Some(TrayEvent::Quit),
        _ => None,
    }
}

/// Drops activations that follow the previous one within `ACTIVATION_GAP_MS`.
#[derive(Clone, Copy, Debug, Default)]
struct ActivationFilter {
    last: Option<u64>,
}
impl ActivationFilter {
    fn accept(&mut self, now_ms: u64) -> bool {
        if self
            .last
            .is_some_and(|t| now_ms.saturating_sub(t) < ACTIVATION_GAP_MS)
        {
            return false;
        }
        self.last = Some(now_ms);
        true
    }
}

/// Bounded number of re-registration retries after `TaskbarCreated`.
#[derive(Clone, Copy, Debug)]
struct Retry {
    left: u8,
}
impl Retry {
    fn new() -> Self {
        Self { left: MAX_RETRIES }
    }
    fn next(&mut self) -> bool {
        if self.left == 0 {
            return false;
        }
        self.left -= 1;
        true
    }
}

/// `x`/`y` packed into a v4 `wParam`: two signed 16 bit words (`GET_X_LPARAM`/`GET_Y_LPARAM`).
fn anchor_from_wparam(wparam: usize) -> (i32, i32) {
    (
        (wparam & 0xFFFF) as u16 as i16 as i32,
        ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32,
    )
}

#[cfg(windows)]
pub use win::{TrayHost, icon_rect, notify, tray_pressed};

/// `p` inside `a` (right/bottom exclusive).
fn contains(a: &crate::platform::Area, p: (i32, i32)) -> bool {
    p.0 >= a.left && p.0 < a.right && p.1 >= a.top && p.1 < a.bottom
}

#[cfg(windows)]
mod win {
    use super::*;
    use crate::platform::Area;
    use std::{cell::Cell, sync::Mutex, time::Instant};
    use windows::{
        Win32::{
            Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM},
            System::LibraryLoader::GetModuleHandleW,
            UI::{
                Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON},
                Shell::{
                    NIF_GUID, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO,
                    NIIF_RESPECT_QUIET_TIME, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
                    NOTIFY_ICON_DATA_FLAGS, NOTIFY_ICON_MESSAGE, NOTIFYICON_VERSION_4,
                    NOTIFYICONDATAW, NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect,
                    Shell_NotifyIconW,
                },
                WindowsAndMessaging::{
                    AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
                    DestroyIcon, DestroyMenu, DestroyWindow, GWLP_USERDATA, GetCursorPos,
                    GetSystemMetrics, GetWindowLongPtrW, HICON, HMENU, KillTimer, MF_CHECKED,
                    MF_SEPARATOR, MF_STRING, MF_UNCHECKED, PostMessageW, RegisterClassW,
                    RegisterWindowMessageW, SM_SWAPBUTTON, SetForegroundWindow, SetTimer,
                    SetWindowLongPtrW, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD,
                    TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_APP, WM_NULL, WM_TIMER, WNDCLASSW,
                    WS_EX_TOOLWINDOW, WS_POPUP,
                },
            },
        },
        core::{GUID, PCWSTR, w},
    };

    const CLASS_NAME: PCWSTR = w!("NexPanelTrayHost");
    const WM_TRAY: u32 = WM_APP + 1;
    const TRAY_UID: u32 = 1;
    const RETRY_TIMER: usize = 1;
    const RETRY_INTERVAL_MS: u32 = 1000;
    /// Unsigned executables bind a GUID to their path, so every build flavor gets its own.
    #[cfg(debug_assertions)]
    const TRAY_GUID: GUID = GUID::from_u128(0x7f0b5a4e_9c3d_4e21_8a6b_1d2f3c4b5a60);
    #[cfg(not(debug_assertions))]
    const TRAY_GUID: GUID = GUID::from_u128(0x2c91d7a3_41be_4f08_b5e6_9a07d3c8e152);
    const TIP: &str = "NexPanel – Systemsteuerung";
    const ICON_PNG: &[u8] = include_bytes!("../assets/tray.png");

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Identity {
        Guid,
        Uid,
    }

    #[derive(Clone, Copy)]
    struct IconId {
        hwnd: isize,
        identity: Identity,
    }
    static ICON: Mutex<Option<IconId>> = Mutex::new(None);

    /// Screen rectangle (physical pixels) of the tray icon, if the shell can tell.
    pub fn icon_rect() -> Option<Area> {
        let id = (*ICON.lock().ok()?)?;
        let mut identifier = NOTIFYICONIDENTIFIER {
            cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: HWND(id.hwnd as *mut _),
            uID: TRAY_UID,
            ..Default::default()
        };
        if id.identity == Identity::Guid {
            identifier.guidItem = TRAY_GUID;
        }
        // SAFETY: `identifier` is fully initialised and outlives the call.
        let rect = unsafe { Shell_NotifyIconGetRect(&identifier) }.ok()?;
        (rect.right > rect.left && rect.bottom > rect.top).then_some(Area {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        })
    }

    /// Shows a balloon/toast on the registered icon (UI thread). The shell plays its own
    /// notification sound and suppresses it in quiet time. Returns false if the icon is not
    /// registered or the shell refused.
    pub fn notify(title: &str, text: &str) -> bool {
        let Some(id) = ICON.lock().ok().and_then(|g| *g) else {
            return false;
        };
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: HWND(id.hwnd as *mut _),
            uID: TRAY_UID,
            uFlags: NIF_INFO,
            szInfo: wide(text),
            szInfoTitle: wide(title),
            dwInfoFlags: NIIF_INFO | NIIF_RESPECT_QUIET_TIME,
            ..Default::default()
        };
        if id.identity == Identity::Guid {
            data.uFlags |= NIF_GUID;
            data.guidItem = TRAY_GUID;
        }
        // SAFETY: `data` is fully initialised and outlives the call.
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool()
    }

    /// True while the primary mouse button is held with the cursor over the tray icon. Called
    /// when the popup loses focus: a click on the icon blurs the popup on mouse-down, and the
    /// toggle only arrives on mouse-up.
    pub fn tray_pressed() -> bool {
        let Some(rect) = icon_rect() else {
            return false;
        };
        // SAFETY: plain queries without pointers.
        let down = unsafe {
            let swapped = GetSystemMetrics(SM_SWAPBUTTON) != 0;
            let key = if swapped { VK_RBUTTON } else { VK_LBUTTON };
            GetAsyncKeyState(key.0 as i32) < 0
        };
        down && contains(&rect, cursor())
    }

    /// State shared between `TrayHost` and the window procedure. Boxed, so the address stored
    /// in `GWLP_USERDATA` stays valid for the lifetime of the host.
    struct Shared {
        hwnd: Cell<HWND>,
        hicon: HICON,
        taskbar_created: u32,
        identity: Cell<Identity>,
        v4: Cell<bool>,
        filter: Cell<ActivationFilter>,
        retry: Cell<Retry>,
        menu_open: Cell<bool>,
        started: Instant,
        handler: Box<dyn Fn(TrayEvent)>,
        autostart: Box<dyn Fn() -> bool>,
    }

    pub struct TrayHost {
        shared: Box<Shared>,
    }

    impl TrayHost {
        /// Must be called on the UI thread; `handler` and `autostart` (current autostart state
        /// for the menu check mark) run there, from inside the message pump.
        pub fn new(
            handler: Box<dyn Fn(TrayEvent)>,
            autostart: Box<dyn Fn() -> bool>,
        ) -> windows::core::Result<Self> {
            let hicon = create_icon()?;
            // SAFETY: plain call, the name is a NUL terminated literal.
            let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
            let shared = Box::new(Shared {
                hwnd: Cell::new(HWND::default()),
                hicon,
                taskbar_created,
                identity: Cell::new(Identity::Guid),
                v4: Cell::new(false),
                filter: Cell::new(ActivationFilter::default()),
                retry: Cell::new(Retry::new()),
                menu_open: Cell::new(false),
                started: Instant::now(),
                handler,
                autostart,
            });
            let hwnd = match create_window() {
                Ok(hwnd) => hwnd,
                Err(e) => {
                    // SAFETY: icon was created above and is not used elsewhere.
                    unsafe {
                        let _ = DestroyIcon(hicon);
                    }
                    return Err(e);
                }
            };
            shared.hwnd.set(hwnd);
            let host = Self { shared };
            // SAFETY: the pointer targets the boxed `Shared`, which `Drop` detaches again
            // before the box is freed.
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*host.shared as *const Shared as isize);
            }
            if !host.shared.register() {
                // E.g. autostart before Explorer is ready: retry, `TaskbarCreated` follows anyway.
                host.shared.schedule_retry();
            }
            Ok(host)
        }
    }

    impl Drop for TrayHost {
        fn drop(&mut self) {
            let s = &self.shared;
            s.delete();
            if let Ok(mut icon) = ICON.lock() {
                *icon = None;
            }
            // SAFETY: detach the window procedure from `Shared` before anything is freed, then
            // destroy the window and icon this host created.
            unsafe {
                SetWindowLongPtrW(s.hwnd.get(), GWLP_USERDATA, 0);
                let _ = DestroyWindow(s.hwnd.get());
                let _ = DestroyIcon(s.hicon);
            }
        }
    }

    impl TrayHost {
        /// Balloon/toast via the registered icon; see [`notify`].
        pub fn notify(&self, title: &str, text: &str) -> bool {
            notify(title, text)
        }
    }

    impl Shared {
        fn data(&self, identity: Identity, flags: NOTIFY_ICON_DATA_FLAGS) -> NOTIFYICONDATAW {
            let tip = wide(TIP);
            let mut data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.hwnd.get(),
                uID: TRAY_UID,
                uFlags: flags,
                uCallbackMessage: WM_TRAY,
                hIcon: self.hicon,
                szTip: tip,
                ..Default::default()
            };
            if identity == Identity::Guid {
                data.uFlags |= NIF_GUID;
                data.guidItem = TRAY_GUID;
            }
            data
        }

        fn notify(&self, message: NOTIFY_ICON_MESSAGE, data: &NOTIFYICONDATAW) -> bool {
            // SAFETY: `data` is a fully initialised NOTIFYICONDATAW of the current size.
            unsafe { Shell_NotifyIconW(message, data) }.as_bool()
        }

        fn add_flags() -> NOTIFY_ICON_DATA_FLAGS {
            NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP
        }

        fn delete(&self) {
            self.notify(
                NIM_DELETE,
                &self.data(self.identity.get(), NOTIFY_ICON_DATA_FLAGS(0)),
            );
        }

        /// NIM_ADD with GUID, one NIM_DELETE + NIM_ADD retry, then plain `uID` mode.
        fn register(&self) -> bool {
            let guid = self.data(Identity::Guid, Self::add_flags());
            let mut identity = Identity::Guid;
            if !self.notify(NIM_ADD, &guid) {
                log::warn!("NIM_ADD (GUID) fehlgeschlagen, versuche NIM_DELETE + NIM_ADD");
                self.notify(NIM_DELETE, &guid);
                if !self.notify(NIM_ADD, &guid) {
                    log::warn!("NIM_ADD (GUID) erneut fehlgeschlagen, wechsle auf uID-Modus");
                    identity = Identity::Uid;
                    if !self.notify(NIM_ADD, &self.data(identity, Self::add_flags())) {
                        log::error!("NIM_ADD (uID) fehlgeschlagen");
                        return false;
                    }
                }
            }
            self.identity.set(identity);
            let mut version = self.data(identity, NOTIFY_ICON_DATA_FLAGS(0));
            version.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let v4 = self.notify(NIM_SETVERSION, &version);
            if !v4 {
                log::warn!("NIM_SETVERSION fehlgeschlagen, nutze Legacy-Nachrichten");
            }
            self.v4.set(v4);
            if let Ok(mut icon) = ICON.lock() {
                *icon = Some(IconId {
                    hwnd: self.hwnd.get().0 as isize,
                    identity,
                });
            }
            log::info!("Icon registriert ({identity:?}, v4={v4})");
            true
        }

        fn taskbar_created(&self) {
            log::info!("TaskbarCreated empfangen");
            // Stale registration (spurious broadcast) must not make NIM_ADD fail.
            self.delete();
            self.retry.set(Retry::new());
            self.cancel_retry();
            if !self.register() {
                self.schedule_retry();
            }
        }

        fn schedule_retry(&self) {
            // SAFETY: valid window of this thread; no timer callback.
            unsafe {
                SetTimer(Some(self.hwnd.get()), RETRY_TIMER, RETRY_INTERVAL_MS, None);
            }
        }

        fn cancel_retry(&self) {
            // SAFETY: valid window of this thread. Fails harmlessly if no timer exists.
            unsafe {
                let _ = KillTimer(Some(self.hwnd.get()), RETRY_TIMER);
            }
        }

        fn retry_tick(&self) {
            let mut retry = self.retry.get();
            let more = retry.next();
            self.retry.set(retry);
            if self.register() {
                self.cancel_retry();
            } else if !more {
                self.cancel_retry();
                log::error!("Icon konnte nicht erneut registriert werden, gebe auf");
            }
        }

        fn context_menu(&self, anchor: (i32, i32)) {
            if self.menu_open.replace(true) {
                return;
            }
            let command = track_menu(self.hwnd.get(), anchor, (self.autostart)());
            self.menu_open.set(false);
            if let Some(event) = command.and_then(menu_event) {
                (self.handler)(event);
            }
        }

        fn on_tray(&self, wparam: usize, lparam: isize) {
            match decode(self.v4.get(), lparam) {
                Some(Gesture::Activate) => {
                    let mut filter = self.filter.get();
                    let accepted = filter.accept(self.started.elapsed().as_millis() as u64);
                    self.filter.set(filter);
                    if accepted {
                        (self.handler)(TrayEvent::Toggle);
                    }
                }
                Some(Gesture::ContextMenu) => {
                    let anchor = if self.v4.get() {
                        anchor_from_wparam(wparam)
                    } else {
                        cursor()
                    };
                    self.context_menu(anchor);
                }
                Some(Gesture::Balloon) => (self.handler)(TrayEvent::NotificationClicked),
                None => {}
            }
        }
    }

    fn cursor() -> (i32, i32) {
        let mut p = POINT::default();
        // SAFETY: `p` is a valid out pointer.
        if unsafe { GetCursorPos(&mut p) }.is_ok() {
            (p.x, p.y)
        } else {
            (0, 0)
        }
    }

    /// Shows the native context menu and returns the chosen command id.
    fn track_menu(host: HWND, anchor: (i32, i32), autostart: bool) -> Option<u32> {
        // SAFETY: menu is created, used and destroyed in this function; `host` is our window.
        unsafe {
            let menu: HMENU = CreatePopupMenu().ok()?;
            let check = if autostart { MF_CHECKED } else { MF_UNCHECKED };
            let _ = AppendMenuW(menu, MF_STRING, ID_OPEN as usize, w!("Öffnen"));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS as usize, w!("Einstellungen"));
            let _ = AppendMenuW(
                menu,
                MF_STRING | check,
                ID_AUTOSTART as usize,
                w!("Mit Windows starten"),
            );
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(menu, MF_STRING, ID_QUIT as usize, w!("Beenden"));
            // Required so the menu closes when the user clicks elsewhere.
            let _ = SetForegroundWindow(host);
            let chosen = TrackPopupMenuEx(
                menu,
                (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_LEFTALIGN).0,
                anchor.0,
                anchor.1,
                host,
                None,
            );
            // Documented companion of TrackPopupMenu for notification icons.
            let _ = PostMessageW(Some(host), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            let id = chosen.0;
            (id > 0).then_some(id as u32)
        }
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // SAFETY: the pointer is either null or the boxed `Shared` of a live `TrayHost`
        // (detached in `Drop` before it is freed). Everything runs on the creating thread.
        let shared = unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Shared).as_ref() };
        if let Some(shared) = shared {
            if msg == WM_TRAY {
                shared.on_tray(wparam.0, lparam.0);
                return LRESULT(0);
            }
            if msg == shared.taskbar_created {
                shared.taskbar_created();
                return LRESULT(0);
            }
            if msg == WM_POWERBROADCAST {
                if let Some(event) = power_event(wparam.0) {
                    (shared.handler)(event);
                }
                // TRUE: request granted / ignored.
                return LRESULT(1);
            }
            if msg == WM_TIMER && wparam.0 == RETRY_TIMER {
                shared.retry_tick();
                return LRESULT(0);
            }
            if is_display_change(msg, wparam.0) {
                // Only a signal; the display worker debounces. Default handling still runs.
                (shared.handler)(TrayEvent::DisplayChange);
            }
        }
        // SAFETY: forwards the unchanged arguments of this window procedure.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    /// Hidden top-level window: unlike `HWND_MESSAGE` windows it receives broadcasts.
    fn create_window() -> windows::core::Result<HWND> {
        // SAFETY: standard class registration and window creation; the class name and
        // procedure are 'static. Registering twice is tolerated (class already exists).
        unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None)?.into();
            let class = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance,
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            RegisterClassW(&class);
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                CLASS_NAME,
                w!("NexPanel Tray"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            )
        }
    }

    /// `HICON` from the embedded PNG (straight-alpha BGRA, like winit's window icons).
    fn create_icon() -> windows::core::Result<HICON> {
        let image = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
            .map_err(|e| {
                windows::core::Error::new(
                    windows::Win32::Foundation::E_FAIL,
                    format!("Tray-Icon nicht lesbar: {e}"),
                )
            })?
            .into_rgba8();
        let (width, height) = image.dimensions();
        let mut xor = image.into_raw();
        for pixel in xor.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let and = vec![0u8; (width.div_ceil(8) * height) as usize];
        // SAFETY: both buffers are alive for the call and sized for a width x height 32 bpp
        // icon (xor) and 1 bpp mask (and, rows padded to bytes); CreateIcon copies them.
        unsafe {
            CreateIcon(
                None,
                width as i32,
                height as i32,
                1,
                32,
                and.as_ptr(),
                xor.as_ptr(),
            )
        }
    }
}

#[cfg(not(windows))]
pub struct TrayHost;
#[cfg(not(windows))]
impl TrayHost {
    pub fn new(
        _: Box<dyn Fn(TrayEvent)>,
        _: Box<dyn Fn() -> bool>,
    ) -> Result<Self, std::io::Error> {
        Ok(Self)
    }
}
#[cfg(not(windows))]
impl TrayHost {
    pub fn notify(&self, _: &str, _: &str) -> bool {
        false
    }
}
#[cfg(not(windows))]
pub fn notify(_: &str, _: &str) -> bool {
    false
}
#[cfg(not(windows))]
pub fn tray_pressed() -> bool {
    false
}
#[cfg(not(windows))]
pub fn icon_rect() -> Option<crate::platform::Area> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_topology_messages_signal_display_change() {
        assert!(is_display_change(WM_DISPLAYCHANGE, 0));
        assert!(is_display_change(WM_DEVICECHANGE, DBT_DEVNODES_CHANGED));
        assert!(!is_display_change(WM_DEVICECHANGE, 0x8000));
        assert!(!is_display_change(WM_APP_TEST, DBT_DEVNODES_CHANGED));
    }
    const WM_APP_TEST: u32 = 0x8001;
    #[test]
    fn v4_uses_loword_of_lparam() {
        // High word carries the icon id in v4.
        let with_id = |code: u32| ((7u32 << 16) | code) as isize;
        assert_eq!(decode(true, with_id(NIN_SELECT)), Some(Gesture::Activate));
        assert_eq!(
            decode(true, with_id(NIN_KEYSELECT)),
            Some(Gesture::Activate)
        );
        assert_eq!(
            decode(true, with_id(WM_CONTEXTMENU)),
            Some(Gesture::ContextMenu)
        );
        assert_eq!(decode(true, with_id(0x0200)), None, "mouse move ignored");
        assert_eq!(
            decode(true, with_id(WM_LBUTTONUP)),
            None,
            "legacy ignored in v4"
        );
    }
    #[test]
    fn legacy_mode_uses_mouse_messages() {
        assert_eq!(
            decode(false, WM_LBUTTONUP as isize),
            Some(Gesture::Activate)
        );
        assert_eq!(
            decode(false, WM_RBUTTONUP as isize),
            Some(Gesture::ContextMenu)
        );
        assert_eq!(decode(false, 0x0201), None);
    }
    #[test]
    fn balloon_click_decodes_in_both_protocols() {
        assert_eq!(
            decode(true, ((7u32 << 16) | NIN_BALLOONUSERCLICK) as isize),
            Some(Gesture::Balloon)
        );
        assert_eq!(
            decode(false, NIN_BALLOONUSERCLICK as isize),
            Some(Gesture::Balloon)
        );
        assert_eq!(decode(true, 0x402), None, "balloon show is not a click");
    }
    #[test]
    fn resume_power_events_map_to_resumed() {
        assert_eq!(
            power_event(PBT_APMRESUMEAUTOMATIC),
            Some(TrayEvent::Resumed)
        );
        assert_eq!(power_event(PBT_APMRESUMESUSPEND), Some(TrayEvent::Resumed));
        assert_eq!(power_event(0x0004), None, "suspend is not resume");
    }
    #[test]
    fn wide_truncates_and_terminates() {
        let w = wide::<4>("abcdef");
        assert_eq!(w, [97, 98, 99, 0]);
    }
    #[test]
    fn menu_ids_map_to_events() {
        assert_eq!(menu_event(ID_OPEN), Some(TrayEvent::Open));
        assert_eq!(menu_event(ID_QUIT), Some(TrayEvent::Quit));
        assert_eq!(menu_event(ID_SETTINGS), Some(TrayEvent::Settings));
        assert_eq!(menu_event(ID_AUTOSTART), Some(TrayEvent::ToggleAutostart));
        assert_eq!(menu_event(0), None);
        assert_eq!(menu_event(99), None);
    }
    #[test]
    fn menu_ids_are_distinct_and_nonzero() {
        let ids = [ID_OPEN, ID_QUIT, ID_SETTINGS, ID_AUTOSTART];
        for (i, a) in ids.iter().enumerate() {
            assert_ne!(*a, 0, "0 means no selection");
            assert!(ids[i + 1..].iter().all(|b| a != b));
        }
    }
    #[test]
    fn contains_is_half_open() {
        let a = crate::platform::Area {
            left: 10,
            top: 20,
            right: 30,
            bottom: 40,
        };
        assert!(contains(&a, (10, 20)));
        assert!(contains(&a, (29, 39)));
        assert!(!contains(&a, (30, 20)));
        assert!(!contains(&a, (10, 40)));
        assert!(!contains(&a, (9, 25)));
    }
    #[test]
    fn activation_filter_collapses_double_notifications() {
        let mut f = ActivationFilter::default();
        assert!(f.accept(1000));
        assert!(!f.accept(1010), "NIN_KEYSELECT right after NIN_SELECT");
        assert!(f.accept(1000 + ACTIVATION_GAP_MS));
        assert!(!f.accept(0), "clock going backwards is not a new click");
    }
    #[test]
    fn retry_is_bounded() {
        let mut r = Retry::new();
        assert_eq!((0..10).filter(|_| r.next()).count(), MAX_RETRIES as usize);
    }
    #[test]
    fn wparam_anchor_is_signed_per_word() {
        assert_eq!(anchor_from_wparam(0x0010_0020), (0x20, 0x10));
        assert_eq!(anchor_from_wparam(0x0000_FFFF), (-1, 0));
        let neg = ((-1920i16 as u16 as usize) << 16) | (-5i16 as u16 as usize);
        assert_eq!(anchor_from_wparam(neg), (-5, -1920));
    }
}
