//! NexSearch window: pure logic (selection, key mapping, geometry, action dispatch) plus the
//! Win32 helpers for launching and the clipboard. The Slint component lives in
//! `ui/search.slint`; the glue that owns the window is `search_controller.rs` of the binary.

use crate::{
    nexsearch::{
        DEFAULT_LIMIT, NexCommand, SearchAction, SearchEngine, calc::CalcProvider,
        commands::CommandProvider, launcher::LauncherProvider,
    },
    platform::Area,
};
use slint::winit_030::winit::keyboard::{Key, NamedKey};
use std::{path::Path, time::Duration};

/// Fixed logical width; must equal the fixed width in `search.slint`.
pub const WINDOW_WIDTH: f32 = 640.0;
/// Heights of the parts of `search.slint` (logical px); keep in sync (see test).
pub const INPUT_HEIGHT: f32 = 56.0;
pub const SEPARATOR_HEIGHT: f32 = 1.0;
pub const ROW_HEIGHT: f32 = 48.0;
pub const LIST_PADDING: f32 = 6.0;
pub const STATUS_HEIGHT: f32 = 44.0;
/// Text of the empty-result row.
pub const NO_RESULTS: &str = "Keine Treffer";
/// A blur this soon after showing is the focus change of the show itself, not the user leaving.
pub const BLUR_GRACE: Duration = Duration::from_millis(250);
/// If the window has not become the foreground window this long after showing, give up.
pub const FOCUS_CHECK_DELAY: Duration = Duration::from_millis(400);

/// Logical window height for `result_count` rows (or the status row when there are none).
pub fn window_height(result_count: usize, has_status: bool) -> f32 {
    if result_count > 0 {
        INPUT_HEIGHT + SEPARATOR_HEIGHT + 2.0 * LIST_PADDING + result_count as f32 * ROW_HEIGHT
    } else if has_status {
        INPUT_HEIGHT + SEPARATOR_HEIGHT + STATUS_HEIGHT
    } else {
        INPUT_HEIGHT
    }
}

/// Tallest the window can get; used to keep it inside the work area.
pub fn max_window_height() -> f32 {
    window_height(DEFAULT_LIMIT, false)
}

pub fn should_close_on_blur(since_show: Duration) -> bool {
    since_show >= BLUR_GRACE
}

/// Selected row of the result list. Navigation wraps around like in most launchers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    len: usize,
    index: usize,
}

impl Selection {
    pub fn new() -> Self {
        Self::default()
    }

    /// New result list: the first row is selected again.
    pub fn set_len(&mut self, len: usize) {
        self.len = len;
        self.index = 0;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// `None` while there is nothing to select.
    pub fn index(&self) -> Option<usize> {
        (self.len > 0).then_some(self.index)
    }

    /// Moves by `delta` rows (usually -1/+1), wrapping at both ends.
    pub fn move_by(&mut self, delta: i32) {
        if self.len == 0 {
            return;
        }
        let len = self.len as i64;
        self.index = (self.index as i64 + i64::from(delta)).rem_euclid(len) as usize;
    }

    /// Selects `index` (mouse hover). Returns true if the selection changed.
    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.len || index == self.index {
            return false;
        }
        self.index = index;
        true
    }
}

/// Keys the search window handles itself before Slint's text input sees them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchKey {
    Up,
    Down,
    Activate,
    Close,
}

pub fn key_action(key: &Key) -> Option<SearchKey> {
    match key {
        Key::Named(NamedKey::ArrowUp) => Some(SearchKey::Up),
        Key::Named(NamedKey::ArrowDown) => Some(SearchKey::Down),
        Key::Named(NamedKey::Enter) => Some(SearchKey::Activate),
        Key::Named(NamedKey::Escape) => Some(SearchKey::Close),
        _ => None,
    }
}

/// Icon-tile glyph per action kind.
pub fn glyph(action: &SearchAction) -> &'static str {
    match action {
        SearchAction::Launch(_) => "↗",
        SearchAction::CopyText(_) => "=",
        SearchAction::Command(_) => "›",
    }
}

/// What Enter does with the selected row (shown right of it).
pub fn action_hint(action: &SearchAction) -> &'static str {
    match action {
        SearchAction::Launch(_) => "Öffnen",
        SearchAction::CopyText(_) => "Kopieren",
        SearchAction::Command(_) => "Ausführen",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowBounds {
    /// Top-left in physical screen pixels.
    pub position: (i32, i32),
    pub physical_width: u32,
}

/// Horizontal centre of `work`, upper fifth vertically (like Spotlight/PowerToys Run), so the
/// window grows downwards while typing. `scale` is the DPI scale of the target monitor.
pub fn window_bounds(work: Area, scale: f64) -> WindowBounds {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let work_w = work.right.saturating_sub(work.left).max(1);
    let work_h = work.bottom.saturating_sub(work.top).max(1);
    let margin = if work_w > 16 { 8 } else { 0 };
    let width =
        ((f64::from(WINDOW_WIDTH) * scale).round() as i32).clamp(1, (work_w - 2 * margin).max(1));
    let max_height = (f64::from(max_window_height()) * scale).round() as i32;
    let x = work.left + (work_w - width) / 2;
    let preferred_y = work.top + (f64::from(work_h) * 0.2) as i32;
    let lowest_y = (work.bottom - max_height).max(work.top);
    WindowBounds {
        position: (x, preferred_y.min(lowest_y)),
        physical_width: width as u32,
    }
}

/// Calculator and NexPanel commands: providers without I/O.
pub fn core_engine() -> SearchEngine {
    let mut engine = SearchEngine::new();
    engine.add_provider(Box::new(CalcProvider));
    engine.add_provider(Box::new(CommandProvider));
    engine
}

/// The engine of the running app. Providers are not `Send`, so call this on the thread that
/// queries it. Starts the Start menu index thread.
pub fn build_engine() -> SearchEngine {
    let mut engine = core_engine();
    engine.add_provider(Box::new(LauncherProvider::start_menu()));
    engine
}

/// Side effects of an activated result. The Win32 implementation lives in the binary glue; tests
/// use a mock.
pub trait ActionHost {
    fn launch(&mut self, path: &Path) -> Result<(), String>;
    fn copy_text(&mut self, text: &str) -> Result<(), String>;
    fn command(&mut self, command: NexCommand);
}

/// Runs `action` on `host`. Failures are logged and reported as `false`; they never panic.
pub fn execute(host: &mut dyn ActionHost, action: &SearchAction) -> bool {
    let result = match action {
        SearchAction::Launch(path) => host.launch(path),
        SearchAction::CopyText(text) => host.copy_text(text),
        SearchAction::Command(command) => {
            host.command(*command);
            Ok(())
        }
    };
    match result {
        Ok(()) => true,
        Err(error) => {
            log::warn!("NexSearch-Aktion fehlgeschlagen ({action:?}): {error}");
            false
        }
    }
}

#[cfg(windows)]
pub use win::{copy_text, launch, place_on_cursor_monitor};

#[cfg(windows)]
mod win {
    use super::*;
    use windows::{
        Win32::{
            Foundation::{GlobalFree, HANDLE, HGLOBAL, POINT},
            Graphics::Gdi::{
                GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
            },
            System::{
                Com::{
                    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx,
                    CoUninitialize,
                },
                DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
                Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
            },
            UI::{
                HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
                Shell::ShellExecuteW,
                WindowsAndMessaging::{GetCursorPos, SW_SHOWNORMAL},
            },
        },
        core::{PCWSTR, w},
    };

    /// `CF_UNICODETEXT`.
    const CF_UNICODETEXT: u32 = 13;
    const CLIPBOARD_ATTEMPTS: u32 = 5;

    /// Centres `window` (physical pixels, the DPI of the monitor under the cursor) like
    /// [`window_bounds`] says. Cheap and idempotent: call it before and after the first show.
    pub fn place_on_cursor_monitor(window: &slint::Window, logical_height: f32) {
        use slint::winit_030::WinitWindowAccessor;
        // SAFETY: plain queries with valid out pointers.
        let (work, scale) = unsafe {
            let mut cursor = POINT::default();
            if GetCursorPos(&mut cursor).is_err() {
                return;
            }
            let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return;
            }
            let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
            let scale = match GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) {
                Ok(()) if dpi_x > 0 => f64::from(dpi_x) / 96.0,
                _ => 1.0,
            };
            let a = info.rcWork;
            (
                Area {
                    left: a.left,
                    top: a.top,
                    right: a.right,
                    bottom: a.bottom,
                },
                scale,
            )
        };
        let bounds = window_bounds(work, scale);
        let height = (f64::from(logical_height) * scale).round().max(1.0) as u32;
        let size = slint::PhysicalSize::new(bounds.physical_width, height);
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

    /// Opens `path` with the shell (`ShellExecuteW "open"`) on a short-lived thread so a slow
    /// handler never blocks the UI thread. The outcome is only logged.
    pub fn launch(path: &Path) -> Result<(), String> {
        if path.as_os_str().is_empty() {
            return Err("leerer Pfad".into());
        }
        let path = path.to_path_buf();
        std::thread::Builder::new()
            .name("nexpanel-launch".into())
            .spawn(move || {
                use std::os::windows::ffi::OsStrExt;
                let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
                // SAFETY: COM is initialised and released on this thread; `wide` is NUL terminated
                // and outlives the call.
                let code = unsafe {
                    let com =
                        CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
                    let result = ShellExecuteW(
                        None,
                        w!("open"),
                        PCWSTR(wide.as_ptr()),
                        None,
                        None,
                        SW_SHOWNORMAL,
                    );
                    if com.is_ok() {
                        CoUninitialize();
                    }
                    result.0 as isize
                };
                // Values above 32 mean success (documented ShellExecute contract).
                if code <= 32 {
                    log::warn!(
                        "ShellExecute für {} fehlgeschlagen (Code {code})",
                        path.display()
                    );
                }
            })
            .map(|_| ())
            .map_err(|e| format!("Thread konnte nicht starten: {e}"))
    }

    /// Puts `text` on the clipboard as `CF_UNICODETEXT` (short-lived thread, a few retries while
    /// another process holds the clipboard).
    pub fn copy_text(text: &str) -> Result<(), String> {
        let text = text.to_owned();
        std::thread::Builder::new()
            .name("nexpanel-clipboard".into())
            .spawn(move || {
                if let Err(error) = set_clipboard(&text) {
                    log::warn!("Zwischenablage nicht beschreibbar: {error}");
                }
            })
            .map(|_| ())
            .map_err(|e| format!("Thread konnte nicht starten: {e}"))
    }

    fn set_clipboard(text: &str) -> Result<(), String> {
        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let bytes = wide.len() * std::mem::size_of::<u16>();
        // SAFETY: standard clipboard protocol. The global block is written while locked; on
        // success the clipboard owns it, on every failure path it is freed here.
        unsafe {
            let mut opened = false;
            for _ in 0..CLIPBOARD_ATTEMPTS {
                if OpenClipboard(None).is_ok() {
                    opened = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if !opened {
                return Err("Zwischenablage gesperrt".into());
            }
            let result = (|| -> Result<(), String> {
                EmptyClipboard().map_err(|e| e.to_string())?;
                let block: HGLOBAL =
                    GlobalAlloc(GMEM_MOVEABLE, bytes).map_err(|e| e.to_string())?;
                let target = GlobalLock(block).cast::<u16>();
                if target.is_null() {
                    let _ = GlobalFree(Some(block));
                    return Err("GlobalLock fehlgeschlagen".into());
                }
                std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
                let _ = GlobalUnlock(block);
                if let Err(e) = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(block.0))) {
                    let _ = GlobalFree(Some(block));
                    return Err(e.to_string());
                }
                Ok(())
            })();
            let _ = CloseClipboard();
            result
        }
    }
}

#[cfg(not(windows))]
pub fn place_on_cursor_monitor(_: &slint::Window, _: f32) {}
#[cfg(not(windows))]
pub fn launch(_: &Path) -> Result<(), String> {
    Err("Starten benötigt Windows".into())
}
#[cfg(not(windows))]
pub fn copy_text(_: &str) -> Result<(), String> {
    Err("Zwischenablage benötigt Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn area(left: i32, top: i32, right: i32, bottom: i32) -> Area {
        Area {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn selection_is_empty_until_results_arrive() {
        let mut s = Selection::new();
        assert_eq!(s.index(), None);
        s.move_by(1);
        assert_eq!(s.index(), None);
        assert!(!s.select(0));
        s.set_len(3);
        assert_eq!(s.index(), Some(0));
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn selection_wraps_in_both_directions() {
        let mut s = Selection::new();
        s.set_len(3);
        s.move_by(1);
        s.move_by(1);
        assert_eq!(s.index(), Some(2));
        s.move_by(1);
        assert_eq!(
            s.index(),
            Some(0),
            "down from the last row wraps to the first"
        );
        s.move_by(-1);
        assert_eq!(
            s.index(),
            Some(2),
            "up from the first row wraps to the last"
        );
        s.move_by(-4);
        assert_eq!(s.index(), Some(1));
    }

    #[test]
    fn selection_single_row_stays_put() {
        let mut s = Selection::new();
        s.set_len(1);
        s.move_by(1);
        s.move_by(-1);
        assert_eq!(s.index(), Some(0));
    }

    #[test]
    fn new_results_reset_selection_to_first_row() {
        let mut s = Selection::new();
        s.set_len(5);
        s.move_by(3);
        s.set_len(5);
        assert_eq!(s.index(), Some(0));
        s.set_len(0);
        assert_eq!(s.index(), None);
        assert!(s.is_empty());
    }

    #[test]
    fn hover_select_reports_changes_only_in_range() {
        let mut s = Selection::new();
        s.set_len(3);
        assert!(!s.select(0), "already selected");
        assert!(s.select(2));
        assert_eq!(s.index(), Some(2));
        assert!(!s.select(3), "out of range");
        assert_eq!(s.index(), Some(2));
    }

    #[test]
    fn extreme_deltas_do_not_overflow() {
        let mut s = Selection::new();
        s.set_len(7);
        s.move_by(i32::MIN);
        s.move_by(i32::MAX);
        assert!(s.index().is_some_and(|i| i < 7));
    }

    #[test]
    fn keys_map_to_search_keys() {
        assert_eq!(
            key_action(&Key::Named(NamedKey::ArrowUp)),
            Some(SearchKey::Up)
        );
        assert_eq!(
            key_action(&Key::Named(NamedKey::ArrowDown)),
            Some(SearchKey::Down)
        );
        assert_eq!(
            key_action(&Key::Named(NamedKey::Enter)),
            Some(SearchKey::Activate)
        );
        assert_eq!(
            key_action(&Key::Named(NamedKey::Escape)),
            Some(SearchKey::Close)
        );
        assert_eq!(key_action(&Key::Named(NamedKey::ArrowLeft)), None);
        assert_eq!(key_action(&Key::Named(NamedKey::Backspace)), None);
        assert_eq!(key_action(&Key::Character("a".into())), None);
    }

    #[test]
    fn height_follows_row_count() {
        assert_eq!(window_height(0, false), INPUT_HEIGHT);
        assert_eq!(
            window_height(0, true),
            INPUT_HEIGHT + SEPARATOR_HEIGHT + STATUS_HEIGHT
        );
        assert_eq!(window_height(1, false), 56.0 + 1.0 + 12.0 + 48.0);
        assert!(window_height(3, false) > window_height(2, false));
        // Rows win over the status row (stale rows stay visible while typing).
        assert_eq!(window_height(2, true), window_height(2, false));
        assert_eq!(max_window_height(), window_height(DEFAULT_LIMIT, false));
    }

    #[test]
    fn slint_layout_matches_constants() {
        let ui: String = include_str!("../ui/search.slint")
            .split_whitespace()
            .collect();
        for needle in [
            format!("width:{}px;", WINDOW_WIDTH as u32),
            format!("height:{}px;", INPUT_HEIGHT as u32),
            format!("height:{}px;", ROW_HEIGHT as u32),
            format!("height:{}px;", SEPARATOR_HEIGHT as u32),
            format!("padding:{}px;", LIST_PADDING as u32),
            format!("height:{}px;", STATUS_HEIGHT as u32),
        ] {
            assert!(ui.contains(&needle), "search.slint lacks {needle}");
        }
    }

    #[test]
    fn blur_grace_ignores_the_show_itself() {
        assert!(!should_close_on_blur(Duration::ZERO));
        assert!(!should_close_on_blur(BLUR_GRACE - Duration::from_millis(1)));
        assert!(should_close_on_blur(BLUR_GRACE));
        assert!(should_close_on_blur(Duration::from_secs(5)));
    }

    #[test]
    fn bounds_center_horizontally_in_upper_fifth() {
        let b = window_bounds(area(0, 0, 1920, 1040), 1.0);
        assert_eq!(b.physical_width, 640);
        assert_eq!(b.position, ((1920 - 640) / 2, 208));
    }

    #[test]
    fn bounds_scale_with_monitor_dpi() {
        let b = window_bounds(area(0, 0, 3840, 2120), 2.0);
        assert_eq!(b.physical_width, 1280);
        assert_eq!(b.position.0, (3840 - 1280) / 2);
        assert_eq!(b.position.1, 424);
        let b = window_bounds(area(0, 0, 2560, 1400), 1.5);
        assert_eq!(b.physical_width, 960);
    }

    #[test]
    fn bounds_keep_negative_monitor_origin() {
        let b = window_bounds(area(-1920, 30, 0, 1080), 1.0);
        assert_eq!(b.position.0, -1920 + (1920 - 640) / 2);
        assert!(b.position.1 >= 30);
    }

    #[test]
    fn bounds_stay_inside_small_work_areas() {
        let work = area(0, 0, 500, 400);
        let b = window_bounds(work, 1.0);
        assert!(b.physical_width as i32 <= 500 - 16);
        assert!(b.position.0 >= work.left);
        assert!(b.position.1 >= work.top);
        let work = area(0, 0, 1920, 600);
        let b = window_bounds(work, 1.0);
        let bottom = b.position.1 + max_window_height() as i32;
        assert!(bottom <= 600, "tallest window fits: {bottom}");
        // Work area shorter than the window: top edge wins.
        let b = window_bounds(area(0, 100, 1920, 300), 1.0);
        assert_eq!(b.position.1, 100);
    }

    #[test]
    fn bounds_survive_bad_scale() {
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                window_bounds(area(0, 0, 1920, 1080), scale).physical_width,
                640
            );
        }
    }

    #[derive(Default)]
    struct MockHost {
        launched: Vec<PathBuf>,
        copied: Vec<String>,
        commands: Vec<NexCommand>,
        fail: bool,
    }

    impl ActionHost for MockHost {
        fn launch(&mut self, path: &Path) -> Result<(), String> {
            self.launched.push(path.to_path_buf());
            if self.fail {
                Err("boom".into())
            } else {
                Ok(())
            }
        }
        fn copy_text(&mut self, text: &str) -> Result<(), String> {
            self.copied.push(text.into());
            if self.fail {
                Err("boom".into())
            } else {
                Ok(())
            }
        }
        fn command(&mut self, command: NexCommand) {
            self.commands.push(command);
        }
    }

    #[test]
    fn execute_routes_each_action_kind() {
        let mut host = MockHost::default();
        assert!(execute(
            &mut host,
            &SearchAction::Launch(PathBuf::from(r"C:\x.exe"))
        ));
        assert!(execute(&mut host, &SearchAction::CopyText("42".into())));
        assert!(execute(&mut host, &SearchAction::Command(NexCommand::Quit)));
        assert!(execute(
            &mut host,
            &SearchAction::Command(NexCommand::OpenSettings)
        ));
        assert_eq!(host.launched, vec![PathBuf::from(r"C:\x.exe")]);
        assert_eq!(host.copied, vec!["42".to_string()]);
        assert_eq!(
            host.commands,
            vec![NexCommand::Quit, NexCommand::OpenSettings]
        );
    }

    #[test]
    fn execute_reports_failures_without_panicking() {
        let mut host = MockHost {
            fail: true,
            ..Default::default()
        };
        assert!(!execute(
            &mut host,
            &SearchAction::Launch(PathBuf::from("a"))
        ));
        assert!(!execute(&mut host, &SearchAction::CopyText("b".into())));
        // Commands cannot fail.
        assert!(execute(
            &mut host,
            &SearchAction::Command(NexCommand::ToggleAutostart)
        ));
    }

    #[test]
    fn core_engine_answers_math_and_commands() {
        let engine = core_engine();
        let math = engine.search("2+2");
        assert!(matches!(
            math.first().map(|r| &r.action),
            Some(SearchAction::CopyText(t)) if t == "4"
        ));
        let settings = engine.search("einstellungen");
        assert_eq!(
            settings.first().map(|r| r.action.clone()),
            Some(SearchAction::Command(NexCommand::OpenSettings))
        );
        assert!(engine.search("   ").is_empty());
    }

    #[test]
    fn glyphs_and_hints_cover_every_action_kind() {
        let actions = [
            SearchAction::Launch(PathBuf::from("a")),
            SearchAction::CopyText("b".into()),
            SearchAction::Command(NexCommand::Quit),
        ];
        let glyphs: Vec<_> = actions.iter().map(glyph).collect();
        let hints: Vec<_> = actions.iter().map(action_hint).collect();
        for (i, a) in glyphs.iter().enumerate() {
            assert!(!a.is_empty());
            assert!(glyphs[i + 1..].iter().all(|b| a != b));
        }
        for (i, a) in hints.iter().enumerate() {
            assert!(!a.is_empty());
            assert!(hints[i + 1..].iter().all(|b| a != b));
        }
    }
}
