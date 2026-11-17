//! Global hotkey for NexSearch (default Alt+Space).
//!
//! The registration logic is pure Rust behind [`HotkeyBackend`] so conflict handling can be
//! tested with a mock. The Win32 backend ([`SystemHotkey`]) owns a hidden message-only window on
//! the UI thread; `WM_HOTKEY` is delivered by the message pump that `slint::run_event_loop`
//! already runs, so no extra thread is needed.

/// `MOD_*` flag values of `RegisterHotKey`.
pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_WIN: u32 = 0x0008;
/// `VK_SPACE`.
pub const VK_SPACE: u32 = 0x20;
/// `ERROR_HOTKEY_ALREADY_REGISTERED`: Alt+Space is also the system menu shortcut of many apps.
pub const ERROR_HOTKEY_ALREADY_REGISTERED: u32 = 1409;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: u32,
    pub vk: u32,
}

impl Hotkey {
    pub const ALT_SPACE: Hotkey = Hotkey {
        modifiers: MOD_ALT,
        vk: VK_SPACE,
    };

    /// Human readable name, e.g. `Alt+Leertaste`.
    pub fn label(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for (flag, name) in [
            (MOD_WIN, "Win"),
            (MOD_CONTROL, "Strg"),
            (MOD_ALT, "Alt"),
            (MOD_SHIFT, "Umschalt"),
        ] {
            if self.modifiers & flag != 0 {
                parts.push(name.into());
            }
        }
        parts.push(match self.vk {
            VK_SPACE => "Leertaste".into(),
            vk @ (0x30..=0x39 | 0x41..=0x5A) => char::from(vk as u8).to_string(),
            vk => format!("0x{vk:02X}"),
        });
        parts.join("+")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyError {
    /// Another program (or the system) already owns the combination.
    Conflict,
    /// Any other Win32 error code.
    Other(u32),
}

impl HotkeyError {
    /// Accepts a plain Win32 error code or an `HRESULT_FROM_WIN32` value (`0x8007xxxx`).
    pub fn from_code(code: u32) -> Self {
        let win32 = if code >> 16 == 0x8007 {
            code & 0xFFFF
        } else {
            code
        };
        if win32 == ERROR_HOTKEY_ALREADY_REGISTERED {
            Self::Conflict
        } else {
            Self::Other(win32)
        }
    }
}

impl std::fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict => write!(f, "Tastenkombination ist bereits belegt (Fehler 1409)"),
            Self::Other(code) => write!(f, "Registrierung fehlgeschlagen (Fehler {code})"),
        }
    }
}

/// Registration service; the Win32 implementation and the test mock both implement it.
pub trait HotkeyBackend {
    fn register(&mut self, hotkey: Hotkey) -> Result<(), HotkeyError>;
    fn unregister(&mut self) -> Result<(), HotkeyError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyStatus {
    /// Not registered (yet, or released again).
    Inactive,
    Active,
    /// Combination already taken by someone else.
    Conflict,
    Failed(u32),
}

impl HotkeyStatus {
    /// True if the user should be told that the hotkey does not work.
    pub fn is_problem(&self) -> bool {
        matches!(self, Self::Conflict | Self::Failed(_))
    }

    /// Hint for the settings tab; empty while nothing is worth saying.
    pub fn message(&self, hotkey: &Hotkey) -> String {
        self.message_with(hotkey, None)
    }

    /// Like [`message`](Self::message); `fallback` is a previous hotkey that stayed active
    /// because `hotkey` could not be registered.
    pub fn message_with(&self, hotkey: &Hotkey, fallback: Option<&Hotkey>) -> String {
        let name = hotkey.label();
        let tail = match fallback {
            Some(old) => format!("Weiterhin aktiv: {}.", old.label()),
            None => "Die globale Suche ist per Tastenkombination nicht erreichbar.".into(),
        };
        match self {
            Self::Inactive => String::new(),
            Self::Active => format!("Globale Suche: {name}"),
            Self::Conflict => format!(
                "{name} ist bereits belegt (anderes Programm oder Systemfunktion, z. B. PowerToys Run). {tail}"
            ),
            Self::Failed(code) => {
                format!("{name} konnte nicht registriert werden (Fehler {code}). {tail}")
            }
        }
    }
}

impl From<HotkeyError> for HotkeyStatus {
    fn from(error: HotkeyError) -> Self {
        match error {
            HotkeyError::Conflict => Self::Conflict,
            HotkeyError::Other(code) => Self::Failed(code),
        }
    }
}

/// Owns one registration: registers on demand, releases exactly once, never panics.
/// `hotkey`/`status` describe the requested combination; `active` is what is really registered
/// (differs while a previous hotkey stays alive after a failed change).
pub struct HotkeyManager<B: HotkeyBackend> {
    backend: B,
    hotkey: Hotkey,
    status: HotkeyStatus,
    active: Option<Hotkey>,
}

impl<B: HotkeyBackend> HotkeyManager<B> {
    pub fn new(backend: B, hotkey: Hotkey) -> Self {
        Self {
            backend,
            hotkey,
            status: HotkeyStatus::Inactive,
            active: None,
        }
    }

    pub fn hotkey(&self) -> Hotkey {
        self.hotkey
    }

    pub fn status(&self) -> HotkeyStatus {
        self.status
    }

    /// The previous hotkey that is still registered because the requested one failed.
    pub fn fallback(&self) -> Option<Hotkey> {
        self.active.filter(|active| *active != self.hotkey)
    }

    /// Settings hint for the current state.
    pub fn message(&self) -> String {
        self.status
            .message_with(&self.hotkey, self.fallback().as_ref())
    }

    /// Registers the hotkey. A no-op while already active; after a conflict it can be called
    /// again to retry.
    pub fn register(&mut self) -> HotkeyStatus {
        if self.status == HotkeyStatus::Active {
            return self.status;
        }
        self.release();
        self.try_register()
    }

    /// Switches to `hotkey` at runtime. If it cannot be registered the previous hotkey is
    /// registered again, so the search stays reachable ([`fallback`](Self::fallback)).
    /// Returns the status of the requested hotkey.
    pub fn set_hotkey(&mut self, hotkey: Hotkey) -> HotkeyStatus {
        if hotkey == self.hotkey && self.status == HotkeyStatus::Active {
            return self.status;
        }
        let previous = self.active;
        self.release();
        self.hotkey = hotkey;
        let status = self.try_register();
        if status.is_problem()
            && let Some(old) = previous.filter(|old| *old != hotkey)
        {
            match self.backend.register(old) {
                Ok(()) => {
                    self.active = Some(old);
                    log::info!("Hotkey {} bleibt aktiv", old.label());
                }
                Err(error) => log::warn!(
                    "Hotkey {} konnte nicht wiederhergestellt werden: {error}",
                    old.label()
                ),
            }
        }
        status
    }

    /// Gives up whatever is registered with the backend.
    fn release(&mut self) {
        if self.active.take().is_none() {
            return;
        }
        if let Err(error) = self.backend.unregister() {
            log::warn!(
                "Hotkey {} konnte nicht freigegeben werden: {error}",
                self.hotkey.label()
            );
        }
    }

    fn try_register(&mut self) -> HotkeyStatus {
        self.status = match self.backend.register(self.hotkey) {
            Ok(()) => {
                self.active = Some(self.hotkey);
                log::info!("Hotkey {} registriert", self.hotkey.label());
                HotkeyStatus::Active
            }
            Err(HotkeyError::Conflict) => {
                log::warn!(
                    "Hotkey {} ist bereits belegt (Fehler 1409), globale Suche nur ohne Hotkey",
                    self.hotkey.label()
                );
                HotkeyStatus::Conflict
            }
            Err(HotkeyError::Other(code)) => {
                log::error!(
                    "Hotkey {} nicht registrierbar (Fehler {code})",
                    self.hotkey.label()
                );
                HotkeyStatus::Failed(code)
            }
        };
        self.status
    }

    /// Releases the hotkey if (and only if) something is registered.
    pub fn unregister(&mut self) {
        self.release();
        self.status = HotkeyStatus::Inactive;
    }
}

impl<B: HotkeyBackend> Drop for HotkeyManager<B> {
    fn drop(&mut self) {
        self.unregister();
    }
}

#[cfg(windows)]
pub use win::SystemHotkey;

#[cfg(windows)]
mod win {
    use super::*;
    use windows::{
        Win32::{
            Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
            System::LibraryLoader::GetModuleHandleW,
            UI::{
                Input::KeyboardAndMouse::{
                    HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
                },
                WindowsAndMessaging::{
                    CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA,
                    GetWindowLongPtrW, HWND_MESSAGE, RegisterClassW, SetWindowLongPtrW,
                    WINDOW_EX_STYLE, WINDOW_STYLE, WM_HOTKEY, WNDCLASSW,
                },
            },
        },
        core::{PCWSTR, w},
    };

    const CLASS_NAME: PCWSTR = w!("NexPanelHotkeyHost");
    const HOTKEY_ID: i32 = 1;

    struct Shared {
        handler: Box<dyn Fn()>,
    }

    /// Win32 backend: a hidden message-only window that receives `WM_HOTKEY` on the UI thread.
    pub struct SystemHotkey {
        hwnd: HWND,
        /// Kept alive for the window procedure (reached through `GWLP_USERDATA`).
        _shared: Box<Shared>,
        registered: bool,
    }

    impl SystemHotkey {
        /// Must be called on the UI thread; `handler` runs there, inside the message pump.
        pub fn new(handler: Box<dyn Fn()>) -> Result<Self, HotkeyError> {
            let hwnd = create_window().map_err(|e| HotkeyError::from_code(e.code().0 as u32))?;
            let shared = Box::new(Shared { handler });
            // SAFETY: the pointer targets the boxed `Shared`, which `Drop` detaches again
            // before the box is freed.
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*shared as *const Shared as isize);
            }
            Ok(Self {
                hwnd,
                _shared: shared,
                registered: false,
            })
        }
    }

    impl HotkeyBackend for SystemHotkey {
        fn register(&mut self, hotkey: Hotkey) -> Result<(), HotkeyError> {
            // SAFETY: valid window of this thread, plain values.
            let result = unsafe {
                RegisterHotKey(
                    Some(self.hwnd),
                    HOTKEY_ID,
                    HOT_KEY_MODIFIERS(hotkey.modifiers) | MOD_NOREPEAT,
                    hotkey.vk,
                )
            };
            match result {
                Ok(()) => {
                    self.registered = true;
                    Ok(())
                }
                Err(e) => Err(HotkeyError::from_code(e.code().0 as u32)),
            }
        }

        fn unregister(&mut self) -> Result<(), HotkeyError> {
            if !self.registered {
                return Ok(());
            }
            self.registered = false;
            // SAFETY: valid window of this thread, id registered by `register`.
            unsafe { UnregisterHotKey(Some(self.hwnd), HOTKEY_ID) }
                .map_err(|e| HotkeyError::from_code(e.code().0 as u32))
        }
    }

    impl Drop for SystemHotkey {
        fn drop(&mut self) {
            let _ = self.unregister();
            // SAFETY: detach the window procedure from `Shared` before anything is freed, then
            // destroy the window this host created.
            unsafe {
                SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_HOTKEY && wparam.0 == HOTKEY_ID as usize {
            // SAFETY: the pointer is either null or the boxed `Shared` of a live `SystemHotkey`
            // (detached in `Drop` before it is freed). Everything runs on the creating thread.
            let shared =
                unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Shared).as_ref() };
            if let Some(shared) = shared {
                // A panic must not unwind through the `extern "system"` window procedure.
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (shared.handler)()))
                    .is_err()
                {
                    log::error!("Panik im Hotkey-Handler abgefangen");
                }
            }
            return LRESULT(0);
        }
        // SAFETY: forwards the unchanged arguments of this window procedure.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

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
                WINDOW_EX_STYLE(0),
                CLASS_NAME,
                w!("NexPanel Hotkey"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance),
                None,
            )
        }
    }
}

#[cfg(not(windows))]
pub struct SystemHotkey;
#[cfg(not(windows))]
impl SystemHotkey {
    pub fn new(_: Box<dyn Fn()>) -> Result<Self, HotkeyError> {
        Ok(Self)
    }
}
#[cfg(not(windows))]
impl HotkeyBackend for SystemHotkey {
    fn register(&mut self, _: Hotkey) -> Result<(), HotkeyError> {
        Err(HotkeyError::Other(0))
    }
    fn unregister(&mut self) -> Result<(), HotkeyError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};

    #[derive(Default)]
    struct Calls {
        registered: Vec<Hotkey>,
        unregistered: usize,
    }

    /// Replays scripted `register` results; `unregister` results are scripted separately.
    struct Mock {
        calls: Rc<RefCell<Calls>>,
        register_results: VecDeque<Result<(), HotkeyError>>,
        unregister_result: Result<(), HotkeyError>,
    }

    impl Mock {
        fn new(results: Vec<Result<(), HotkeyError>>) -> (Self, Rc<RefCell<Calls>>) {
            let calls = Rc::new(RefCell::new(Calls::default()));
            (
                Self {
                    calls: calls.clone(),
                    register_results: results.into(),
                    unregister_result: Ok(()),
                },
                calls,
            )
        }
    }

    impl HotkeyBackend for Mock {
        fn register(&mut self, hotkey: Hotkey) -> Result<(), HotkeyError> {
            self.calls.borrow_mut().registered.push(hotkey);
            self.register_results.pop_front().unwrap_or(Ok(()))
        }
        fn unregister(&mut self) -> Result<(), HotkeyError> {
            self.calls.borrow_mut().unregistered += 1;
            self.unregister_result
        }
    }

    #[test]
    fn classifies_win32_codes_and_hresults() {
        assert_eq!(HotkeyError::from_code(1409), HotkeyError::Conflict);
        assert_eq!(HotkeyError::from_code(0x8007_0581), HotkeyError::Conflict);
        assert_eq!(HotkeyError::from_code(1400), HotkeyError::Other(1400));
        assert_eq!(
            HotkeyError::from_code(0x8007_0578),
            HotkeyError::Other(0x578)
        );
        // Non-Win32 facility: 0x0581 low bits must not be mistaken for 1409.
        assert_eq!(
            HotkeyError::from_code(0x8004_0581),
            HotkeyError::Other(0x8004_0581)
        );
    }

    #[test]
    fn successful_registration_is_active_and_released_on_drop() {
        let (mock, calls) = Mock::new(vec![Ok(())]);
        let mut manager = HotkeyManager::new(mock, Hotkey::ALT_SPACE);
        assert_eq!(manager.status(), HotkeyStatus::Inactive);
        assert_eq!(manager.register(), HotkeyStatus::Active);
        assert_eq!(calls.borrow().registered, vec![Hotkey::ALT_SPACE]);
        drop(manager);
        assert_eq!(calls.borrow().unregistered, 1);
    }

    #[test]
    fn conflict_is_reported_without_unregister() {
        let (mock, calls) = Mock::new(vec![Err(HotkeyError::Conflict)]);
        let mut manager = HotkeyManager::new(mock, Hotkey::ALT_SPACE);
        assert_eq!(manager.register(), HotkeyStatus::Conflict);
        assert!(manager.status().is_problem());
        manager.unregister();
        drop(manager);
        assert_eq!(
            calls.borrow().unregistered,
            0,
            "nothing registered, nothing to release"
        );
    }

    #[test]
    fn other_error_keeps_code() {
        let (mock, _) = Mock::new(vec![Err(HotkeyError::Other(1400))]);
        let mut manager = HotkeyManager::new(mock, Hotkey::ALT_SPACE);
        assert_eq!(manager.register(), HotkeyStatus::Failed(1400));
        assert!(manager.status().is_problem());
    }

    #[test]
    fn register_is_idempotent_while_active() {
        let (mock, calls) = Mock::new(vec![Ok(()), Ok(())]);
        let mut manager = HotkeyManager::new(mock, Hotkey::ALT_SPACE);
        manager.register();
        manager.register();
        assert_eq!(calls.borrow().registered.len(), 1);
    }

    #[test]
    fn retry_after_conflict_can_succeed() {
        let (mock, calls) = Mock::new(vec![Err(HotkeyError::Conflict), Ok(())]);
        let mut manager = HotkeyManager::new(mock, Hotkey::ALT_SPACE);
        assert_eq!(manager.register(), HotkeyStatus::Conflict);
        assert_eq!(manager.register(), HotkeyStatus::Active);
        assert_eq!(calls.borrow().registered.len(), 2);
        assert!(!manager.status().is_problem());
    }

    #[test]
    fn unregister_releases_once() {
        let (mock, calls) = Mock::new(vec![Ok(())]);
        let mut manager = HotkeyManager::new(mock, Hotkey::ALT_SPACE);
        manager.register();
        manager.unregister();
        manager.unregister();
        assert_eq!(manager.status(), HotkeyStatus::Inactive);
        drop(manager);
        assert_eq!(calls.borrow().unregistered, 1);
    }

    #[test]
    fn failing_unregister_does_not_panic_and_resets_status() {
        let (mut mock, calls) = Mock::new(vec![Ok(())]);
        mock.unregister_result = Err(HotkeyError::Other(1419));
        let mut manager = HotkeyManager::new(mock, Hotkey::ALT_SPACE);
        manager.register();
        manager.unregister();
        assert_eq!(manager.status(), HotkeyStatus::Inactive);
        assert_eq!(calls.borrow().unregistered, 1);
    }

    #[test]
    fn labels_and_messages() {
        assert_eq!(Hotkey::ALT_SPACE.label(), "Alt+Leertaste");
        assert_eq!(
            Hotkey {
                modifiers: MOD_CONTROL | MOD_SHIFT,
                vk: u32::from(b'K')
            }
            .label(),
            "Strg+Umschalt+K"
        );
        assert_eq!(
            Hotkey {
                modifiers: MOD_WIN,
                vk: 0x70
            }
            .label(),
            "Win+0x70"
        );
        let hk = Hotkey::ALT_SPACE;
        assert_eq!(HotkeyStatus::Inactive.message(&hk), "");
        assert_eq!(
            HotkeyStatus::Active.message(&hk),
            "Globale Suche: Alt+Leertaste"
        );
        assert!(HotkeyStatus::Conflict.message(&hk).contains("belegt"));
        assert!(HotkeyStatus::Failed(5).message(&hk).contains("Fehler 5"));
        assert!(!HotkeyStatus::Active.is_problem());
        assert!(!HotkeyStatus::Inactive.is_problem());
    }

    const CTRL_ALT_SPACE: Hotkey = Hotkey {
        modifiers: MOD_CONTROL | MOD_ALT,
        vk: VK_SPACE,
    };

    #[test]
    fn changing_hotkey_releases_old_and_registers_new() {
        let (mock, calls) = Mock::new(vec![Ok(()), Ok(())]);
        let mut manager = HotkeyManager::new(mock, Hotkey::ALT_SPACE);
        manager.register();
        assert_eq!(manager.set_hotkey(CTRL_ALT_SPACE), HotkeyStatus::Active);
        assert_eq!(manager.hotkey(), CTRL_ALT_SPACE);
        assert_eq!(manager.fallback(), None);
        assert_eq!(
            calls.borrow().registered,
            vec![Hotkey::ALT_SPACE, CTRL_ALT_SPACE]
        );
        assert_eq!(calls.borrow().unregistered, 1);
        assert_eq!(manager.message(), "Globale Suche: Strg+Alt+Leertaste");
        drop(manager);
        assert_eq!(calls.borrow().unregistered, 2);
    }

    #[test]
    fn conflicting_change_keeps_previous_hotkey_alive() {
        let (mock, calls) = Mock::new(vec![Ok(()), Err(HotkeyError::Conflict), Ok(())]);
        let mut manager = HotkeyManager::new(mock, Hotkey::ALT_SPACE);
        manager.register();
        assert_eq!(manager.set_hotkey(CTRL_ALT_SPACE), HotkeyStatus::Conflict);
        assert!(manager.status().is_problem());
        assert_eq!(manager.fallback(), Some(Hotkey::ALT_SPACE));
        assert_eq!(
            calls.borrow().registered,
            vec![Hotkey::ALT_SPACE, CTRL_ALT_SPACE, Hotkey::ALT_SPACE]
        );
        let message = manager.message();
        assert!(
            message.contains("Strg+Alt+Leertaste ist bereits belegt"),
            "{message}"
        );
        assert!(
            message.contains("Weiterhin aktiv: Alt+Leertaste"),
            "{message}"
        );
        drop(manager);
        assert_eq!(calls.borrow().unregistered, 2, "old one released on drop");
    }

    #[test]
    fn failed_change_without_restorable_previous_has_no_fallback() {
        let (mock, _) = Mock::new(vec![
            Ok(()),
            Err(HotkeyError::Conflict),
            Err(HotkeyError::Other(5)),
        ]);
        let mut manager = HotkeyManager::new(mock, Hotkey::ALT_SPACE);
        manager.register();
        assert_eq!(manager.set_hotkey(CTRL_ALT_SPACE), HotkeyStatus::Conflict);
        assert_eq!(manager.fallback(), None);
        assert!(manager.message().contains("nicht erreichbar"));
    }

    #[test]
    fn changing_after_initial_conflict_registers_the_new_one() {
        let (mock, calls) = Mock::new(vec![Err(HotkeyError::Conflict), Ok(())]);
        let mut manager = HotkeyManager::new(mock, Hotkey::ALT_SPACE);
        assert_eq!(manager.register(), HotkeyStatus::Conflict);
        assert_eq!(manager.set_hotkey(CTRL_ALT_SPACE), HotkeyStatus::Active);
        assert_eq!(manager.fallback(), None);
        assert_eq!(
            calls.borrow().unregistered,
            0,
            "nothing was registered before"
        );
    }

    #[test]
    fn setting_the_same_hotkey_is_a_noop_when_active_and_a_retry_after_conflict() {
        let (mock, calls) = Mock::new(vec![Ok(())]);
        let mut manager = HotkeyManager::new(mock, Hotkey::ALT_SPACE);
        manager.register();
        manager.set_hotkey(Hotkey::ALT_SPACE);
        assert_eq!(calls.borrow().registered.len(), 1);

        let (mock, calls) = Mock::new(vec![Err(HotkeyError::Conflict), Ok(())]);
        let mut manager = HotkeyManager::new(mock, Hotkey::ALT_SPACE);
        manager.register();
        assert_eq!(manager.set_hotkey(Hotkey::ALT_SPACE), HotkeyStatus::Active);
        assert_eq!(calls.borrow().registered.len(), 2);
    }

    #[test]
    fn register_after_fallback_drops_it_and_retries_the_requested_hotkey() {
        let (mock, calls) = Mock::new(vec![Ok(()), Err(HotkeyError::Conflict), Ok(()), Ok(())]);
        let mut manager = HotkeyManager::new(mock, Hotkey::ALT_SPACE);
        manager.register();
        manager.set_hotkey(CTRL_ALT_SPACE);
        assert_eq!(manager.register(), HotkeyStatus::Active);
        assert_eq!(manager.fallback(), None);
        assert_eq!(calls.borrow().unregistered, 2);
        assert_eq!(calls.borrow().registered.last(), Some(&CTRL_ALT_SPACE));
    }

    #[test]
    fn error_display_mentions_code() {
        assert!(HotkeyError::Conflict.to_string().contains("1409"));
        assert!(HotkeyError::Other(7).to_string().contains('7'));
    }
}
