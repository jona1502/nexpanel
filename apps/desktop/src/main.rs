#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use nexpanel::{
    accent,
    config::{Accent, Config, SearchHotkey, SearchPosition},
    hotkey::{Hotkey, HotkeyManager, HotkeyStatus, SystemHotkey},
    meter,
    nexsearch::NexCommand,
    platform,
    popup_state::{Action, Event, Popup, State},
    services::{self, AudioCommand, DisplayCommand, system::SystemCommand},
    settings_worker::{self, Command},
    time_input::TimeInput,
    time_tools::{Countdown, Stopwatch},
    tray::{self, TrayEvent},
    ui_models::{LocalHold, keep_held, reconcile},
    update::{
        self, schedule,
        version::Version,
        worker::{self as update_worker, Command as UpdateCommand},
    },
};
use slint::{
    ComponentHandle, Model, ModelRc, VecModel,
    winit_030::{EventResult, WinitWindowAccessor, winit},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{atomic::Ordering, mpsc::Sender},
    time::{Duration, Instant},
};
slint::include_modules!();
/// Unix time of the last update check; part of the settings file but not shown in the UI.
static LAST_UPDATE_CHECK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
mod search_controller;
#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Normal(bool),
    Help,
    Diagnostics,
    Capture(std::path::PathBuf),
}
fn parse_arguments(args: &[String]) -> Result<Mode, &'static str> {
    match args {
        [] => Ok(Mode::Normal(false)),
        [arg] if arg == "--background" => Ok(Mode::Normal(true)),
        [arg] if arg == "--help" || arg == "-h" => Ok(Mode::Help),
        [arg] if arg == "--diagnostics" => Ok(Mode::Diagnostics),
        [arg, directory]
            if arg == "--capture-preview"
                && !directory.is_empty()
                && !directory.starts_with('-') =>
        {
            Ok(Mode::Capture(directory.into()))
        }
        _ => Err("Ungültige Argumente; --help zeigt die Optionen"),
    }
}
/// Removes `--wait-pid <pid>` (set by the updater when it starts the new version) from `args`.
fn take_wait_pid(args: &mut Vec<String>) -> Result<Option<u32>, &'static str> {
    let Some(position) = args
        .iter()
        .position(|a| a == nexpanel::update::install::WAIT_PID_ARG)
    else {
        return Ok(None);
    };
    let pid = args
        .get(position + 1)
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|pid| *pid != 0)
        .ok_or("Ungültige Argumente; --wait-pid erwartet eine Prozess-ID")?;
    args.drain(position..position + 2);
    Ok(Some(pid))
}
/// Tab indices in `main.slint` (`for label[i] in [...]`, `root.tab==N`).
/// Order: Audio, Display, System, Zeit, Einstellungen.
///
/// The panel shows the tab that was open when it was last closed (kept in memory while the
/// app runs, persisted as `last_tab` in settings.json across restarts). Exceptions that
/// override the remembered tab for one opening: timer expiry while hidden and a balloon
/// click select `TAB_TIMER`, the tray menu "Einstellungen" selects `TAB_SETTINGS`. The
/// overriding tab is remembered again when the panel closes.
const TAB_AUDIO: i32 = 0;
const TAB_SYSTEM: i32 = 2;
const TAB_TIMER: i32 = 3;
const TAB_SETTINGS: i32 = 4;
/// Fixed logical panel size; must equal the fixed size in `main.slint`.
#[cfg(test)]
const PANEL_SIZE: (u32, u32) = (392, 440);
const TAB_COUNT: usize = nexpanel::config::TAB_COUNT as usize;
/// Out-of-range values (corrupt or newer settings file) fall back to the Audio tab.
fn sanitize_tab(tab: u8) -> i32 {
    if usize::from(tab) < TAB_COUNT {
        i32::from(tab)
    } else {
        TAB_AUDIO
    }
}
/// Persist the current tab through the settings worker if it changed since the last save.
/// Never saves before the first snapshot or while the file is blocked (corrupt).
fn save_tab_if_changed(app: &MainWindow, tx: &Sender<Command>, saved: &Cell<i32>) {
    let tab = app.get_tab();
    if tab == saved.get() || !app.get_settings_ready() || app.get_settings_blocked() {
        return;
    }
    saved.set(tab);
    let _ = tx.send(Command::Save(config_from(app)));
}
fn capture_name(tab: usize) -> &'static str {
    [
        "01-audio.png",
        "02-displays.png",
        "03-system.png",
        "04-zeit.png",
        "05-einstellungen.png",
    ][tab]
}
fn apply_update(a: &MainWindow, snapshot: &update_worker::Snapshot) {
    let v = snapshot.view();
    a.set_update_state(v.state);
    a.set_update_version(v.version.into());
    a.set_update_progress(v.progress);
    a.set_update_error_kind(v.error_kind);
    a.set_update_error_detail(v.error_detail.into());
    a.set_update_managed(v.managed);
    a.set_update_busy(v.busy);
    a.set_update_can_install(v.can_install);
}
fn apply_system(a: &MainWindow, s: &services::system::SystemSnapshot) {
    let v = services::system::view(s);
    a.set_system_status(s.status.clone().into());
    a.set_cpu_text(v.cpu_text.into());
    a.set_cpu_pct(v.cpu_pct);
    a.set_cpu_level(v.cpu_level);
    a.set_ram_text(v.ram_text.into());
    a.set_ram_pct(v.ram_pct);
    a.set_ram_level(v.ram_level);
    a.set_gpu_available(v.gpu_available);
    a.set_gpu_text(v.gpu_text.into());
    a.set_gpu_pct(v.gpu_pct);
    a.set_gpu_level(v.gpu_level);
    a.set_vram_text(v.vram_text.into());
    a.set_vram_pct(v.vram_pct);
    a.set_vram_level(v.vram_level);
    a.set_gpu_name(v.gpu_name.into());
}
/// Tick while the panel is open: text, sliders and worker snapshots must feel live.
const FAST_TICK: Duration = Duration::from_millis(200);
/// Tick while hidden: only drains worker channels, crash flags and the "open" request of a
/// second instance. Countdown expiry has its own one-shot timer.
const IDLE_TICK: Duration = Duration::from_secs(1);
/// What decides how often the UI tick has to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TickState {
    visible: bool,
    /// Workers or a second instance may still need attention while hidden.
    watching: bool,
}
/// `None` stops the tick completely until something re-arms it.
fn next_tick_interval(state: &TickState) -> Option<Duration> {
    if state.visible {
        Some(FAST_TICK)
    } else if state.watching {
        Some(IDLE_TICK)
    } else {
        None
    }
}
fn peak_color(percent: u8) -> slint::Color {
    let [r, g, b] = meter::level_color(percent);
    slint::Color::from_rgb_u8(r, g, b)
}
/// Writes new levels into the audio rows; rows missing from `peaks` go back to 0. Only rows whose
/// level changed are touched (`set_row_data`), so an ongoing slider drag is not disturbed.
fn apply_peaks(model: &VecModel<AudioItem>, peaks: &[(String, u8)]) {
    for i in 0..model.row_count() {
        let Some(mut row) = model.row_data(i) else {
            continue;
        };
        let level = peaks
            .iter()
            .find(|(id, _)| row.id.as_str() == id)
            .map_or(0, |(_, level)| i32::from(*level));
        if row.peak != level {
            row.peak = level;
            row.peak_color = peak_color(level.clamp(0, 100) as u8);
            model.set_row_data(i, row);
        }
    }
}
/// Delay for the one-shot timer that wakes the loop exactly when a running countdown ends.
fn expiry_delay(running: bool, remaining_ms: u64) -> Option<Duration> {
    // +10 ms so the wake-up never lands before the deadline (poll_expired needs `now >= end`).
    running.then(|| Duration::from_millis(remaining_ms.saturating_add(10)))
}
/// Applies the hotkey chosen in the settings (registers or re-registers it).
type HotkeySync = Rc<dyn Fn(&MainWindow, SearchHotkey)>;
type TickBody = Rc<RefCell<Box<dyn FnMut()>>>;
/// Runs the tick now (no-op while it is already running).
fn wake(body: &TickBody) {
    if let Ok(mut f) = body.try_borrow_mut() {
        f();
    }
}
type TimerCheck = Rc<RefCell<Option<Box<dyn Fn()>>>>;
/// Monotonic ms including standby (see `nexpanel::clock`); wall-clock jumps do not affect timers.
fn now() -> u64 {
    nexpanel::clock::mono_ms()
}
/// Polls expiry and refreshes the timer texts. Notifies (balloon, never a focus-stealing
/// panel) exactly once per expiry: `Countdown::poll_expired` latches.
///
/// No `RefCell` borrow may be held across `tray::notify`: `Shell_NotifyIconW` is a
/// cross-process call that dispatches incoming sent messages (e.g. `WM_POWERBROADCAST` ->
/// `Resumed` -> this function again). A re-entrant call finds the countdown borrowed and skips.
fn update_timers(
    a: &MainWindow,
    countdown: &RefCell<Countdown>,
    stopwatch: &RefCell<Stopwatch>,
    drift: &nexpanel::clock::DriftDetector,
) {
    let (Ok(mut c), Ok(s)) = (countdown.try_borrow_mut(), stopwatch.try_borrow()) else {
        return;
    };
    drift.check();
    let t = now();
    let expired = c.poll_expired(t);
    if expired {
        a.set_expired(true);
    }
    a.set_countdown_text(clock(c.remaining_ms(t).saturating_add(999)));
    a.set_countdown_running(c.is_running());
    a.set_countdown_active(c.is_started(t));
    a.set_stopwatch_text(clock(s.elapsed_ms(t)));
    a.set_stopwatch_running(s.is_running());
    drop((c, s));
    if expired {
        // Focus Assist can swallow the balloon without any error: the banner must be the
        // first thing the user sees when the panel is opened next (TAB_TIMER).
        if !a.window().is_visible() {
            a.set_tab(TAB_TIMER);
        }
        if !tray::notify("NexPanel", "Timer abgelaufen") {
            platform::alarm();
        }
    }
}
thread_local! {
    /// Slint images per icon key; built once on the UI thread, never touched elsewhere.
    static ICON_IMAGES: RefCell<std::collections::HashMap<u64, slint::Image>> = Default::default();
}
fn slint_icon(icon: &services::AppIcon) -> slint::Image {
    ICON_IMAGES.with(|cache| {
        cache
            .borrow_mut()
            .entry(icon.key)
            .or_insert_with(|| {
                slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                    &icon.rgba,
                    icon.width,
                    icon.height,
                ))
            })
            .clone()
    })
}
fn show_input(a: &MainWindow, t: TimeInput) {
    let (h, m, s) = t.segments();
    a.set_input_h(format!("{h:02}").into());
    a.set_input_m(format!("{m:02}").into());
    a.set_input_s(format!("{s:02}").into());
}
fn clock(ms: u64) -> slint::SharedString {
    let s = ms / 1000;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60).into()
}
fn show(app: &MainWindow) {
    // Once the native window exists (every show after the first), place it
    // before it becomes visible so no frame appears at the old position. On the
    // first show Slint creates the window lazily, so position() is a no-op
    // there and the call after show() applies it.
    platform::position(app.window());
    let _ = app.show();
    platform::hide_from_alt_tab(app.window());
    platform::position(app.window());
    app.window().with_winit_window(|w| w.focus_window());
}
/// Feeds `event` to the popup state machine and performs the resulting action.
/// What Esc does: an open dropdown list absorbs it, otherwise the panel hides.
#[derive(Debug, PartialEq, Eq)]
enum EscAction {
    CloseDropdown,
    ClosePanel,
}
fn esc_action(dropdown_open: bool) -> EscAction {
    if dropdown_open {
        EscAction::CloseDropdown
    } else {
        EscAction::ClosePanel
    }
}
fn dispatch(app: &MainWindow, popup: &RefCell<Popup>, event: Event, ms: u64) {
    let action = popup.borrow_mut().handle(event, ms);
    match action {
        Action::Show => show(app),
        Action::Hide => {
            app.set_dropdown_open(false);
            app.set_tip_visible(false);
            let _ = app.hide();
            popup.borrow_mut().handle(Event::HideDone, ms);
        }
        Action::Nothing => {}
    }
}
fn config_from(app: &MainWindow) -> Config {
    Config {
        accent: Accent::from_index(app.get_accent_index()),
        accent_custom: None,
        animations: app.get_animations(),
        close_on_focus_loss: app.get_focus_close(),
        linked_displays: app.get_linked(),
        autostart: app.get_autostart(),
        search_hotkey: SearchHotkey::from_index(app.get_hotkey_index()),
        search_position: SearchPosition::from_index(app.get_position_index()),
        auto_update_check: app.get_auto_update_check(),
        last_update_check: LAST_UPDATE_CHECK.load(Ordering::Relaxed),
        last_tab: u8::try_from(sanitize_tab(u8::try_from(app.get_tab()).unwrap_or(u8::MAX)))
            .unwrap_or(0),
    }
}
/// Colors for the accent dropdown: presets plus the custom entry (default blue while unset).
fn accent_dots(custom: Option<&str>) -> Vec<slint::Color> {
    let custom = custom.and_then(accent::parse_hex);
    Accent::ALL
        .iter()
        .map(|a| {
            let rgb = a.rgb().or(custom).unwrap_or([0x80, 0x9c, 0xff]);
            let [r, g, b] = accent::readable_accent(rgb);
            slint::Color::from_rgb_u8(r, g, b)
        })
        .collect()
}
fn set_accent_dots(app: &MainWindow, custom: Option<&str>) {
    app.set_accent_dots(ModelRc::new(VecModel::from(accent_dots(custom))));
}
fn apply_config(app: &MainWindow, c: &Config) {
    set_accent_dots(app, c.accent_custom.as_deref());
    app.set_accent_index(c.accent.index() as i32);
    app.set_animations(c.animations);
    app.set_focus_close(c.close_on_focus_loss);
    app.set_linked(c.linked_displays);
    app.set_autostart(c.autostart);
    app.set_hotkey_index(c.search_hotkey.index() as i32);
    app.set_position_index(c.search_position.index() as i32);
    app.set_auto_update_check(c.auto_update_check);
    LAST_UPDATE_CHECK.store(c.last_update_check, Ordering::Relaxed);
}
fn apply_countdown_action(
    timer: &mut Countdown,
    duration: &mut i32,
    expired: &mut bool,
    selected: i32,
    reset: bool,
    now: u64,
) {
    let selected = selected.clamp(1, 86400);
    if reset {
        *timer = Countdown::new(selected as u64 * 1000).expect("bounded duration");
        *duration = selected;
        *expired = false;
    } else if timer.is_running() {
        timer.pause(now);
    } else {
        if *expired || *duration != selected {
            *timer = Countdown::new(selected as u64 * 1000).expect("bounded duration");
            *duration = selected;
            *expired = false;
        }
        timer.start(now);
    }
}
fn start_logging(args: &[String]) {
    nexpanel::logging::init();
    log::info!(
        "NexPanel {} gestartet, Argumente: {args:?}",
        env!("CARGO_PKG_VERSION")
    );
}
/// Startup failures have no console (`windows_subsystem`): log them and tell the user.
fn startup_error<E: std::fmt::Display>(
    what: &'static str,
    silent: bool,
) -> impl FnOnce(E) -> Box<dyn std::error::Error> {
    move |error| {
        let message = format!("{what}: {error}");
        log::error!("Start fehlgeschlagen: {message}");
        if !silent {
            report_startup_error(&message);
        }
        message.into()
    }
}
#[cfg(windows)]
fn report_startup_error(message: &str) {
    use windows::{
        Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
        core::{PCWSTR, w},
    };
    let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    // SAFETY: `message` is NUL terminated and outlives the call.
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            w!("NexPanel – Start fehlgeschlagen"),
            MB_OK | MB_ICONERROR,
        );
    }
}
#[cfg(not(windows))]
fn report_startup_error(message: &str) {
    eprintln!("{message}");
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<_> = std::env::args().skip(1).collect();
    let wait_pid = take_wait_pid(&mut args)?;
    let mode = parse_arguments(&args)?;
    let capture_mode = matches!(mode, Mode::Capture(_));
    // Normal starts log only after `acquire` succeeded: a second instance must not touch the log.
    if !matches!(mode, Mode::Help | Mode::Normal(_)) {
        start_logging(&args);
    }
    if mode == Mode::Help {
        println!(
            "NexPanel 0.1\n  --background   Im Tray starten\n  --diagnostics  Display/Audio nur lesen und beenden\n  --capture-preview <Verzeichnis>  Fünf echte UI-Tabs als PNG speichern, dann beenden (nur lesend)\n  --help         Hilfe"
        );
        return Ok(());
    }
    if mode == Mode::Diagnostics {
        let d = std::thread::spawn(services::display::diagnose);
        let a = std::thread::spawn(services::audio::diagnose);
        println!(
            "Displays: {:#?}\nAudio: {:#?}",
            d.join().map_err(|_| "Displaydiagnose fehlgeschlagen")?,
            a.join().map_err(|_| "Audiodiagnose fehlgeschlagen")?
        );
        match nexpanel::config::default_path().and_then(|p| nexpanel::config::load(&p)) {
            Ok(c) => println!("Konfiguration: {c:?}"),
            Err(e) => println!("Konfigurationsfehler (unverändert): {e}"),
        };
        println!("Autostart (gelesen): {:?}", platform::read_autostart());
        return Ok(());
    }
    // Started by the updater: the old instance still holds the single-instance mutex until it
    // has exited.
    if let Some(pid) = wait_pid
        && matches!(mode, Mode::Normal(_))
    {
        platform::wait_for_process_exit(pid, Duration::from_secs(15));
    }
    #[cfg(windows)]
    let instance = if capture_mode {
        None
    } else {
        match platform::Instance::acquire(mode != Mode::Normal(true))
            .map_err(startup_error("Instanzprüfung fehlgeschlagen", false))?
        {
            Some(instance) => Some(instance),
            None => return Ok(()),
        }
    };
    if matches!(mode, Mode::Normal(_)) {
        start_logging(&args);
    }
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .with_winit_window_attributes_hook(|attributes| {
            #[cfg(windows)]
            {
                use winit::platform::windows::WindowAttributesExtWindows;
                attributes.with_skip_taskbar(true)
            }
            #[cfg(not(windows))]
            {
                attributes
            }
        })
        .select()
        .map_err(startup_error(
            "Fenster-Backend nicht verfügbar",
            capture_mode,
        ))?;
    let app = MainWindow::new().map_err(startup_error(
        "Fenster konnte nicht erstellt werden",
        capture_mode,
    ))?;
    app.set_app_version(env!("CARGO_PKG_VERSION").into());
    let (dtx, drx, display_crashed) = services::display::spawn();
    let (atx, arx, audio_peaks, audio_crashed) = services::audio::spawn();
    let (ytx, yrx, system_crashed) = services::system::spawn();
    let (stx, srx, settings_thread) = settings_worker::spawn();
    // Update worker: network and installation never run on the UI thread. Not in capture mode.
    let updates = (!capture_mode).then(|| {
        update_worker::spawn(
            Version::parse(update::CURRENT_VERSION).expect("package version is valid semver"),
            true,
            update::system_transport,
            update_worker::FsInstaller::new(std::env::current_exe().ok()),
            &nexpanel::logging::CrashFlag::default(),
        )
    });
    let update_tx = updates.as_ref().map(|u| u.tx.clone());
    let update_cancel = updates.as_ref().map(|u| u.cancel.clone());
    let update_rx = updates.map(|u| u.rx);
    // Last tab written to settings.json (or loaded from it); see `save_tab_if_changed`.
    let saved_tab = Rc::new(Cell::new(TAB_AUDIO));
    let tab_restored = Rc::new(Cell::new(false));
    let tick_body: TickBody = Rc::new(RefCell::new(Box::new(|| {})));
    // Persistent models: rows are updated in place so a dragged slider survives.
    let display_model = Rc::new(VecModel::<DisplayRow>::default());
    let audio_model = Rc::new(VecModel::<AudioItem>::default());
    set_accent_dots(&app, None);
    app.set_displays(ModelRc::from(display_model.clone()));
    app.set_audio(ModelRc::from(audio_model.clone()));
    // Rows the user is currently changing ignore worker values for a short time.
    let display_hold = Rc::new(RefCell::new(LocalHold::default()));
    let audio_hold = Rc::new(RefCell::new(LocalHold::default()));
    // Level meter: polls the worker's peak channel at ~25 Hz, only while it is switched on.
    let meter_timer = Rc::new(slint::Timer::default());
    let meter_drain: Rc<dyn Fn(bool)> = {
        let model = audio_model.clone();
        Rc::new(move |clear: bool| {
            let latest = audio_peaks.try_iter().last();
            if clear {
                apply_peaks(&model, &[]);
            } else if let Some(peaks) = latest {
                apply_peaks(&model, &peaks);
            }
        })
    };
    let popup = Rc::new(RefCell::new(Popup::new()));
    let started = Instant::now();
    let ms = move || started.elapsed().as_millis() as u64;
    let weak = app.as_weak();
    let blur_popup = popup.clone();
    app.window().on_winit_window_event(move |window, event| {
        match event {
            winit::event::WindowEvent::Focused(false) => {
                if let Some(app) = weak.upgrade()
                    && !capture_mode
                    && app.get_focus_close()
                    && app.window().is_visible()
                {
                    let tray_pressed = tray::tray_pressed();
                    let action = blur_popup
                        .borrow_mut()
                        .handle(Event::Blur { tray_pressed }, ms());
                    if action == Action::Hide {
                        app.set_dropdown_open(false);
                        app.set_tip_visible(false);
                        let _ = window.hide();
                        blur_popup.borrow_mut().handle(Event::HideDone, ms());
                    }
                }
            }
            winit::event::WindowEvent::KeyboardInput { event, .. }
                if event.state == winit::event::ElementState::Pressed
                    && event.logical_key
                        == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) =>
            {
                let dropdown_open = weak.upgrade().is_some_and(|a| a.get_dropdown_open());
                match esc_action(dropdown_open) {
                    EscAction::CloseDropdown => {
                        if let Some(app) = weak.upgrade() {
                            app.set_dropdown_open(false);
                            app.set_tip_visible(false);
                        }
                    }
                    EscAction::ClosePanel => {
                        let mut popup = blur_popup.borrow_mut();
                        popup.handle(Event::Esc, ms());
                        if let Some(app) = weak.upgrade() {
                            app.set_dropdown_open(false);
                            app.set_tip_visible(false);
                        }
                        let _ = window.hide();
                        popup.handle(Event::HideDone, ms());
                    }
                }
                return EventResult::PreventDefault;
            }
            winit::event::WindowEvent::ScaleFactorChanged { .. } => {
                // Apply after Slint has processed the new scale, outside this native event callback.
                let weak = weak.clone();
                slint::Timer::single_shot(Duration::ZERO, move || {
                    if let Some(app) = weak.upgrade() {
                        platform::position(app.window());
                    }
                });
            }
            _ => {}
        }
        EventResult::Propagate
    });
    app.window().on_close_requested({
        let weak = app.as_weak();
        let popup = popup.clone();
        move || {
            if let Some(app) = weak.upgrade() {
                dispatch(&app, &popup, Event::Esc, ms());
            }
            slint::CloseRequestResponse::KeepWindowShown
        }
    });
    app.on_hide_panel({
        let weak = app.as_weak();
        let popup = popup.clone();
        move || {
            if let Some(app) = weak.upgrade() {
                dispatch(&app, &popup, Event::Esc, ms());
            }
        }
    });
    // Filled once the timers exist; lets the tray trigger an immediate check after resume.
    let timer_check: TimerCheck = Rc::default();
    // App actions shared by the tray menu and NexSearch commands. Callers wake the tick
    // afterwards (the panel may have been shown).
    let app_command: Rc<dyn Fn(NexCommand)> = Rc::new({
        let weak = app.as_weak();
        let popup = popup.clone();
        let quit_tx = stx.clone();
        let saved_tab = saved_tab.clone();
        move |command| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            match command {
                NexCommand::OpenSettings => {
                    app.set_tab(TAB_SETTINGS);
                    dispatch(&app, &popup, Event::TrayOpen, ms());
                }
                // Before the first settings snapshot the save would be discarded
                // while the property flips: ignore the request.
                NexCommand::ToggleAutostart if !app.get_settings_ready() => {}
                NexCommand::ToggleAutostart => {
                    // Same path as the checkbox in the settings tab.
                    app.set_autostart(!app.get_autostart());
                    app.invoke_save_settings();
                }
                NexCommand::Quit => {
                    if !capture_mode {
                        save_tab_if_changed(&app, &quit_tx, &saved_tab);
                    }
                    let _ = slint::quit_event_loop();
                }
            }
        }
    });
    let tray = if capture_mode {
        None
    } else {
        Some(
            tray::TrayHost::new(
                Box::new({
                    let weak = app.as_weak();
                    let popup = popup.clone();
                    let timer_check = timer_check.clone();
                    let display_changes = dtx.clone();
                    let tick_body = tick_body.clone();
                    let app_command = app_command.clone();
                    move |event| {
                        let Some(app) = weak.upgrade() else {
                            return;
                        };
                        match event {
                            TrayEvent::Toggle => dispatch(&app, &popup, Event::TrayToggle, ms()),
                            TrayEvent::Open => dispatch(&app, &popup, Event::TrayOpen, ms()),
                            TrayEvent::Settings => app_command(NexCommand::OpenSettings),
                            TrayEvent::ToggleAutostart => app_command(NexCommand::ToggleAutostart),
                            TrayEvent::NotificationClicked => {
                                app.set_tab(TAB_TIMER);
                                dispatch(&app, &popup, Event::ShowRequested, ms());
                            }
                            TrayEvent::Resumed => {
                                if let Some(check) = timer_check.borrow().as_ref() {
                                    check();
                                }
                            }
                            TrayEvent::DisplayChange => {
                                let _ = display_changes.send(DisplayCommand::TopologyChanged);
                            }
                            TrayEvent::Quit => app_command(NexCommand::Quit),
                        }
                        // Panel may have been shown: refresh visibility, tick rate and snapshots now.
                        wake(&tick_body);
                    }
                }),
                Box::new({
                    let weak = app.as_weak();
                    move || weak.upgrade().is_some_and(|app| app.get_autostart())
                }),
            )
            .map_err(startup_error(
                "Tray-Icon konnte nicht erstellt werden",
                false,
            ))?,
        )
    };
    // NexSearch: window, worker and the global hotkey. Neither exists in capture mode.
    let search = if capture_mode {
        None
    } else {
        let command: Rc<dyn Fn(NexCommand)> = Rc::new({
            let app_command = app_command.clone();
            let tick_body = tick_body.clone();
            move |command| {
                app_command(command);
                wake(&tick_body);
            }
        });
        match search_controller::SearchController::new(command) {
            Ok(controller) => Some(Rc::new(controller)),
            Err(error) => {
                log::error!("NexSearch-Fenster konnte nicht erstellt werden: {error}");
                None
            }
        }
    };
    // The hotkey is registered once the settings snapshot says which one the user chose
    // (`sync_hotkey`); changing it later re-registers at runtime without a restart.
    let hotkey: Rc<RefCell<Option<HotkeyManager<SystemHotkey>>>> = Rc::default();
    if let Some(search) = &search {
        let handler = Box::new({
            let (weak, search) = (app.as_weak(), search.clone());
            move || {
                if let Some(app) = weak.upgrade() {
                    search.toggle(
                        app.get_accent(),
                        app.get_animations(),
                        SearchPosition::from_index(app.get_position_index()),
                    );
                }
            }
        });
        match SystemHotkey::new(handler) {
            Ok(backend) => {
                *hotkey.borrow_mut() = Some(HotkeyManager::new(backend, Hotkey::ALT_SPACE));
            }
            Err(error) => {
                log::error!("Hotkey-Fenster konnte nicht erstellt werden: {error}");
                let status = HotkeyStatus::from(error);
                app.set_hotkey_status(status.message(&Hotkey::ALT_SPACE).as_str().into());
                app.set_hotkey_warning(true);
            }
        }
    }
    let sync_hotkey: HotkeySync = Rc::new({
        let hotkey = hotkey.clone();
        let applied = Cell::new(None::<SearchHotkey>);
        move |app, choice| {
            // Only a changed choice (or the first one) registers: saving unrelated settings must
            // neither retry a conflicting hotkey nor show its balloon again.
            if applied.replace(Some(choice)) == Some(choice) {
                return;
            }
            let mut guard = hotkey.borrow_mut();
            let Some(manager) = guard.as_mut() else {
                return;
            };
            let status = manager.set_hotkey(choice.hotkey());
            let message = manager.message();
            app.set_hotkey_status(message.as_str().into());
            app.set_hotkey_warning(status.is_problem());
            if status.is_problem() {
                // One balloon so the user learns that the hotkey is missing; the settings tab
                // keeps the explanation.
                tray::notify("NexPanel", &message);
            }
        }
    });
    app.on_refresh_display({
        let tx = dtx.clone();
        move || {
            let _ = tx.send(DisplayCommand::Refresh);
        }
    });
    app.on_refresh_audio({
        let tx = atx.clone();
        move || {
            let _ = tx.send(AudioCommand::Refresh);
        }
    });
    app.on_brightness({
        let tx = dtx.clone();
        let weak = app.as_weak();
        let (model, hold) = (display_model.clone(), display_hold.clone());
        move |id, value| {
            if capture_mode {
                return;
            }
            hold.borrow_mut().touch(&id, Instant::now());
            if let Some(i) = model.iter().position(|r| r.id == id) {
                let mut row = model.row_data(i).expect("row in range");
                row.value = value.clamp(0, 100);
                model.set_row_data(i, row);
            }
            let linked = weak.upgrade().is_some_and(|a| a.get_linked());
            let value = value.clamp(0, 100) as u8;
            let _ = tx.send(if linked {
                DisplayCommand::SetLinkedBrightness(value)
            } else {
                DisplayCommand::SetBrightness {
                    id: id.into(),
                    value,
                }
            });
        }
    });
    app.on_volume({
        let tx = atx.clone();
        let (model, hold) = (audio_model.clone(), audio_hold.clone());
        move |id, value| {
            if capture_mode {
                return;
            }
            hold.borrow_mut().touch(&id, Instant::now());
            if let Some(i) = model.iter().position(|r| r.id == id) {
                let mut row = model.row_data(i).expect("row in range");
                row.value = value.clamp(0, 100);
                model.set_row_data(i, row);
            }
            let _ = tx.send(AudioCommand::SetVolume {
                id: id.into(),
                value: value.clamp(0, 100) as u8,
            });
        }
    });
    app.on_mute({
        let tx = atx.clone();
        move |id, muted| {
            if capture_mode {
                return;
            }
            let _ = tx.send(AudioCommand::SetMute {
                id: id.into(),
                muted,
            });
        }
    });
    app.on_save_settings({
        let tx = stx.clone();
        let weak = app.as_weak();
        let sync_hotkey = sync_hotkey.clone();
        move || {
            if capture_mode {
                return;
            }
            if let Some(a) = weak.upgrade()
                && a.get_settings_ready()
            {
                let _ = tx.send(Command::Save(config_from(&a)));
                sync_hotkey(&a, SearchHotkey::from_index(a.get_hotkey_index()));
            }
        }
    });
    app.on_custom_hex_edited({
        let weak = app.as_weak();
        move |text| {
            let Some(a) = weak.upgrade() else {
                return;
            };
            // Empty field = nothing entered yet; anything else must parse or it is rejected
            // (marked invalid, previous accepted color kept).
            if text.trim().is_empty() {
                a.set_custom_hex_valid(true);
                return;
            }
            let Some(rgb) = accent::parse_hex(&text) else {
                a.set_custom_hex_valid(false);
                return;
            };
            let hex = accent::to_hex(rgb);
            a.set_custom_hex_valid(true);
            a.set_custom_adjusted(accent::readable_accent(rgb) != rgb);
            a.set_custom_hex_saved(hex.as_str().into());
            set_accent_dots(&a, Some(&hex));
            a.invoke_save_settings();
        }
    });
    app.on_reset_settings({
        let tx = stx.clone();
        move || {
            if capture_mode {
                return;
            }
            let _ = tx.send(Command::Reset);
        }
    });
    // Manual and scheduled checks share this path: remember the attempt (also persisted in
    // settings.json) so a failing or offline check is not repeated more than once per 24 h.
    let check_updates: Rc<dyn Fn(&MainWindow)> = Rc::new({
        let (tx, settings_tx) = (update_tx.clone(), stx.clone());
        move |app| {
            let Some(tx) = &tx else {
                return;
            };
            LAST_UPDATE_CHECK.store(schedule::unix_now(), Ordering::Relaxed);
            if app.get_settings_ready() && !app.get_settings_blocked() {
                let _ = settings_tx.send(Command::Save(config_from(app)));
            }
            let _ = tx.send(UpdateCommand::Check);
        }
    });
    app.on_check_updates({
        let (weak, check_updates) = (app.as_weak(), check_updates.clone());
        move || {
            if let Some(a) = weak.upgrade() {
                check_updates(&a);
            }
        }
    });
    // The only way to install: the user clicked "Jetzt aktualisieren".
    app.on_install_update({
        let tx = update_tx.clone();
        move || {
            if let Some(tx) = &tx {
                let _ = tx.send(UpdateCommand::Install);
            }
        }
    });
    let countdown = Rc::new(RefCell::new(
        Countdown::new(300_000).expect("valid default"),
    ));
    let duration = Rc::new(Cell::new(300));
    let input = Rc::new(Cell::new(TimeInput::from_seconds(300)));
    show_input(&app, input.get());
    app.on_time_digit({
        let (input, weak) = (input.clone(), app.as_weak());
        move |digit| {
            if let (Some(a), Ok(d)) = (weak.upgrade(), u32::try_from(digit)) {
                input.set(input.get().push_digit(d));
                show_input(&a, input.get());
            }
        }
    });
    app.on_time_backspace({
        let (input, weak) = (input.clone(), app.as_weak());
        move |clear| {
            if let Some(a) = weak.upgrade() {
                input.set(if clear {
                    input.get().clear()
                } else {
                    input.get().backspace()
                });
                show_input(&a, input.get());
            }
        }
    });
    app.on_time_step({
        let (input, weak) = (input.clone(), app.as_weak());
        move |segment, delta| {
            if let Some(a) = weak.upgrade() {
                input.set(
                    input
                        .get()
                        .normalize()
                        .step(usize::try_from(segment).unwrap_or(2), delta),
                );
                show_input(&a, input.get());
            }
        }
    });
    let stopwatch = Rc::new(RefCell::new(Stopwatch::default()));
    app.on_countdown_action({
        let c = countdown.clone();
        let input = input.clone();
        let weak = app.as_weak();
        let tick_body = tick_body.clone();
        move |action| {
            if let Some(a) = weak.upgrade() {
                let mut c = c.borrow_mut();
                if !c.is_started(now()) {
                    // Idle: the field is the source; carry overflowing segments (00:00:90 -> 00:01:30).
                    input.set(input.get().normalize());
                    show_input(&a, input.get());
                    if input.get().to_seconds() == 0 && action == 0 {
                        return;
                    }
                }
                let mut selected_duration = duration.get();
                let mut expired = a.get_expired();
                apply_countdown_action(
                    &mut c,
                    &mut selected_duration,
                    &mut expired,
                    input.get().to_seconds() as i32,
                    action == 1,
                    now(),
                );
                duration.set(selected_duration);
                a.set_expired(expired);
                a.set_countdown_running(c.is_running());
                a.set_countdown_active(c.is_started(now()));
                a.set_countdown_text(clock(c.remaining_ms(now()).saturating_add(999)));
            }
            // Re-arm the expiry timer for the new deadline.
            wake(&tick_body);
        }
    });
    app.on_stopwatch_action({
        let s = stopwatch.clone();
        let tick_body = tick_body.clone();
        move |action| {
            {
                let mut s = s.borrow_mut();
                if action == 1 {
                    s.reset();
                } else if s.is_running() {
                    s.pause(now());
                } else {
                    s.start(now());
                }
            }
            wake(&tick_body);
        }
    });
    let shutdown_display = dtx.clone();
    let shutdown_audio = atx.clone();
    let shutdown_system = ytx.clone();
    let drift = Rc::new(nexpanel::clock::DriftDetector::default());
    *timer_check.borrow_mut() = Some(Box::new({
        let (weak, c, s, d) = (
            app.as_weak(),
            countdown.clone(),
            stopwatch.clone(),
            drift.clone(),
        );
        move || {
            if let Some(a) = weak.upgrade() {
                update_timers(&a, &c, &s, &d);
            }
        }
    }));
    let tick = Rc::new(slint::Timer::default());
    let expiry = Rc::new(slint::Timer::default());
    let tick_interval = Cell::new(Some(FAST_TICK));
    let ready = Rc::new(Cell::new((false, false)));
    let system_ready = Rc::new(Cell::new(false));
    let system_seen = system_ready.clone();
    let snapshot_ready = ready.clone();
    let weak = app.as_weak();
    let tick_popup = popup.clone();
    let tick_sync_hotkey = sync_hotkey.clone();
    let tick_update_check = check_updates.clone();
    let tick_app_command = app_command.clone();
    let tab_tx = stx.clone();
    let mut visible = None;
    let mut meter_active = None;
    let mut crash_shown = (false, false, false);
    let mut system_active = None;
    let tick_timer = Rc::downgrade(&tick);
    let tick_wake = Rc::downgrade(&tick_body);
    *tick_body.borrow_mut() = Box::new({
        move || {
            let Some(a) = weak.upgrade() else {
                return;
            };
            if let Some(s) = drx.try_iter().last() {
                snapshot_ready.set((true, snapshot_ready.get().1));
                a.set_display_status(s.status.into());
                let mut rows: Vec<_> = s
                    .devices
                    .into_iter()
                    .map(|d| DisplayRow {
                        id: d.id.into(),
                        name: d.name.into(),
                        value: d.brightness.unwrap_or(0) as i32,
                        supported: d.brightness.is_some(),
                        status: d.status.into(),
                    })
                    .collect();
                let mut hold = display_hold.borrow_mut();
                keep_held(&*display_model, &mut rows, &mut hold, Instant::now(), |r| {
                    r.id.clone()
                });
                reconcile(&*display_model, rows, |r| r.id.clone());
            }
            if let Some(s) = arx.try_iter().last() {
                snapshot_ready.set((snapshot_ready.get().0, true));
                a.set_audio_status(s.status.into());
                a.set_audio_device(if s.device.is_empty() {
                    "Lautstärke".into()
                } else {
                    s.device.into()
                });
                let to_item = |r: services::AudioRow, master: bool| {
                    let icon = r.icon.as_ref().map(slint_icon);
                    AudioItem {
                        initial: r
                            .name
                            .chars()
                            .next()
                            .map(|c| c.to_uppercase().collect::<String>())
                            .unwrap_or_else(|| "?".into())
                            .into(),
                        id: r.id.into(),
                        name: r.name.into(),
                        value: r.volume as i32,
                        muted: r.muted,
                        master,
                        has_icon: icon.is_some(),
                        icon: icon.unwrap_or_default(),
                        peak: 0,
                        peak_color: peak_color(0),
                    }
                };
                let mut rows: Vec<_> = s
                    .master
                    .into_iter()
                    .map(|r| to_item(r, true))
                    .chain(s.sessions.into_iter().map(|r| to_item(r, false)))
                    .collect();
                // Snapshots carry no levels: keep the live ones so bars do not flicker.
                for row in &mut rows {
                    if let Some(old) = audio_model.iter().find(|o| o.id == row.id) {
                        row.peak = old.peak;
                        row.peak_color = old.peak_color;
                    }
                }
                let mut hold = audio_hold.borrow_mut();
                keep_held(&*audio_model, &mut rows, &mut hold, Instant::now(), |r| {
                    r.id.clone()
                });
                reconcile(&*audio_model, rows, |r| r.id.clone());
            }
            if !crash_shown.0 && display_crashed.load(Ordering::SeqCst) {
                crash_shown.0 = true;
                a.set_display_status("Display-Dienst abgestürzt – siehe Log".into());
                a.set_displays(ModelRc::default());
            }
            if !crash_shown.1 && audio_crashed.load(Ordering::SeqCst) {
                crash_shown.1 = true;
                a.set_audio_status("Audio-Dienst abgestürzt – siehe Log".into());
                a.set_audio(ModelRc::default());
            }
            if let Some(s) = yrx.try_iter().last() {
                system_seen.set(true);
                apply_system(&a, &s);
            }
            if !crash_shown.2 && system_crashed.load(Ordering::SeqCst) {
                crash_shown.2 = true;
                a.set_system_status("System-Dienst abgestürzt – siehe Log".into());
            }
            if let Some(s) = srx.try_iter().last() {
                apply_config(&a, &s.config);
                tick_sync_hotkey(&a, s.config.search_hotkey);
                // First snapshot only: restore the remembered tab unless an override
                // (balloon click, tray "Einstellungen", timer expiry) already chose one.
                if !tab_restored.replace(true) && !capture_mode {
                    saved_tab.set(sanitize_tab(s.config.last_tab));
                    if a.get_tab() == TAB_AUDIO {
                        a.set_tab(saved_tab.get());
                    }
                }
                a.set_settings_error(s.error.into());
                a.set_settings_blocked(s.blocked);
                a.set_settings_ready(true);
            }
            if let Some(rx) = &update_rx {
                if let Some(snapshot) = rx.try_iter().last() {
                    apply_update(&a, &snapshot);
                    if snapshot.quit {
                        // The new version is installed and running; leave cleanly so it can
                        // take over the single-instance mutex.
                        tick_app_command(NexCommand::Quit);
                    }
                }
                // At start and then at most once per 24 h; never before the settings are loaded
                // (the timestamp lives there) and never while a corrupt file blocks saving.
                if a.get_settings_ready()
                    && !a.get_settings_blocked()
                    && !a.get_update_busy()
                    && schedule::auto_check_due(
                        a.get_auto_update_check(),
                        LAST_UPDATE_CHECK.load(Ordering::Relaxed),
                        schedule::unix_now(),
                    )
                {
                    tick_update_check(&a);
                }
            }
            #[cfg(windows)]
            if instance.as_ref().is_some_and(|i| i.open_requested()) {
                dispatch(&a, &tick_popup, Event::ShowRequested, ms());
            }
            update_timers(&a, &countdown, &stopwatch, &drift);
            let mut active = a.window().is_visible();
            // Window hidden from outside: reconcile, or the next toggle would hide nothing.
            if !active && tick_popup.borrow().state() != State::Closed {
                a.set_dropdown_open(false);
                a.set_tip_visible(false);
                tick_popup.borrow_mut().handle(Event::HideDone, ms());
            }
            // Fallback for a missed blur (grace period, shell stole activation) and for
            // press evidence that no toggle will ever redeem.
            let focused = !active || !a.get_focus_close() || tray::is_foreground(a.window());
            let tray_pressed =
                tick_popup.borrow().needs_press_state(focused) && tray::tray_pressed();
            dispatch(
                &a,
                &tick_popup,
                Event::Tick {
                    focused,
                    tray_pressed,
                },
                ms(),
            );
            active = a.window().is_visible();
            if visible != Some(active) {
                visible = Some(active);
                let _ = dtx.send(DisplayCommand::SetActive(active));
                let _ = atx.send(AudioCommand::SetActive(active));
                // Panel just closed: remember the tab (only written if it changed).
                if !active && !capture_mode {
                    save_tab_if_changed(&a, &tab_tx, &saved_tab);
                }
            }
            // Level meter: measured and polled only while the panel is visible AND the Audio tab is shown.
            let want_meter = active && a.get_tab() == TAB_AUDIO && !capture_mode;
            if meter_active != Some(want_meter) {
                meter_active = Some(want_meter);
                let _ = atx.send(AudioCommand::SetMetering(want_meter));
                if want_meter {
                    let drain = meter_drain.clone();
                    meter_timer.start(slint::TimerMode::Repeated, meter::INTERVAL, move || {
                        drain(false)
                    });
                } else {
                    meter_timer.stop();
                    meter_drain(true);
                }
            }
            // Sample only while the panel is visible AND the System tab is shown.
            let want_system = active && a.get_tab() == TAB_SYSTEM;
            if system_active != Some(want_system) {
                system_active = Some(want_system);
                let _ = ytx.send(SystemCommand::SetActive(want_system));
            }
            // Countdown expiry: one-shot timer at the exact deadline instead of polling.
            let delay = {
                let c = countdown.borrow();
                expiry_delay(c.is_running(), c.remaining_ms(now()))
            };
            match delay {
                Some(delay) => {
                    let again = tick_wake.clone();
                    expiry.start(slint::TimerMode::SingleShot, delay, move || {
                        if let Some(body) = again.upgrade() {
                            wake(&body);
                        }
                    });
                }
                None => expiry.stop(),
            }
            let next = next_tick_interval(&TickState {
                visible: active,
                watching: !capture_mode,
            });
            if next != tick_interval.get()
                && let Some(timer) = tick_timer.upgrade()
            {
                tick_interval.set(next);
                match next {
                    Some(d) => {
                        timer.set_interval(d);
                        if !timer.running() {
                            timer.restart();
                        }
                    }
                    None => timer.stop(),
                }
            }
        }
    });
    tick.start(slint::TimerMode::Repeated, FAST_TICK, {
        let body = tick_body.clone();
        move || wake(&body)
    });
    if mode != Mode::Normal(true) {
        dispatch(&app, &popup, Event::ShowRequested, ms());
    }
    let capture_error = Rc::new(RefCell::new(None::<String>));
    let capture_timer = slint::Timer::default();
    let mut capture_worker = None;
    if let Mode::Capture(directory) = mode {
        let (tx, rx) =
            std::sync::mpsc::channel::<(usize, slint::SharedPixelBuffer<slint::Rgba8Pixel>)>();
        capture_worker = Some(std::thread::spawn(move || -> Result<(), String> {
            std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
            for (tab, image) in rx {
                let path = directory.join(capture_name(tab));
                image::save_buffer(
                    &path,
                    image.as_bytes(),
                    image.width(),
                    image.height(),
                    image::ColorType::Rgba8,
                )
                .map_err(|e| e.to_string())?;
                println!("Vorschau: {}", path.display());
            }
            Ok(())
        }));
        let started = Instant::now();
        let weak = app.as_weak();
        let error = capture_error.clone();
        let mut tab = 0;
        let mut next_capture = Duration::from_secs(3);
        capture_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(100),
            move || {
                if started.elapsed() < next_capture {
                    return;
                }
                if let Some(app) = weak.upgrade() {
                    if tab == 0
                        && (!ready.get().0 || !ready.get().1 || !app.get_settings_ready())
                        && started.elapsed() < Duration::from_secs(15)
                    {
                        return;
                    }
                    // System values need the worker to be activated and prime first.
                    if tab == TAB_SYSTEM as usize
                        && !system_ready.get()
                        && started.elapsed() < next_capture + Duration::from_secs(5)
                    {
                        return;
                    }
                    let result = app
                        .window()
                        .take_snapshot()
                        .map_err(|e| e.to_string())
                        .and_then(|image| tx.send((tab, image)).map_err(|e| e.to_string()));
                    if let Err(e) = result {
                        *error.borrow_mut() = Some(e);
                        let _ = slint::quit_event_loop();
                        return;
                    }
                    tab += 1;
                    if tab == TAB_COUNT {
                        let _ = slint::quit_event_loop();
                    } else {
                        app.set_tab(tab as i32);
                        next_capture = started.elapsed() + Duration::from_millis(500);
                    }
                } else {
                    let _ = slint::quit_event_loop();
                }
            },
        );
    }
    // The own tray host no longer keeps the loop alive; hiding the popup must not quit.
    slint::run_event_loop_until_quit()?;
    // Release the hotkey before the windows go away. The closures above still share the
    // manager, so take it out explicitly.
    drop(hotkey.borrow_mut().take());
    drop(search);
    drop(tray);
    log::info!("Event-Loop beendet, Worker werden gestoppt");
    tick.stop();
    capture_timer.stop();
    drop(capture_timer);
    let _ = shutdown_display.send(DisplayCommand::Shutdown);
    let _ = shutdown_audio.send(AudioCommand::Shutdown);
    let _ = shutdown_system.send(SystemCommand::Shutdown);
    if let Some(cancel) = &update_cancel {
        cancel.store(true, Ordering::Relaxed);
    }
    if let Some(tx) = &update_tx {
        let _ = tx.send(UpdateCommand::Shutdown);
    }
    // Display and audio workers: wait briefly so COM/DDC handles are released.
    nexpanel::logging::join_workers(Duration::from_secs(1));
    let _ = stx.send(Command::Shutdown);
    let settings_result = settings_thread
        .join()
        .map_err(|_| "Einstellungsworker unerwartet beendet")?;
    if let Err(error) = settings_result {
        if !capture_mode {
            platform::report_shutdown_error(&error);
        }
        return Err(error.into());
    }
    if let Some(worker) = capture_worker {
        worker
            .join()
            .map_err(|_| "Vorschauworker unerwartet beendet")??;
    }
    if let Some(error) = capture_error.borrow_mut().take() {
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn esc_closes_dropdown_before_panel() {
        assert_eq!(esc_action(true), EscAction::CloseDropdown);
        assert_eq!(esc_action(false), EscAction::ClosePanel);
    }
    use super::*;
    #[test]
    fn expired_countdown_controller_restarts_selected_duration_and_clears_alarm() {
        let mut timer = Countdown::new(1000).unwrap();
        let mut duration = 1;
        let mut expired = false;
        apply_countdown_action(&mut timer, &mut duration, &mut expired, 1, false, 1000);
        expired = timer.poll_expired(2000);
        assert!(expired);
        apply_countdown_action(&mut timer, &mut duration, &mut expired, 1, false, 2500);
        assert!(
            timer.is_running(),
            "same-duration restart must start a new countdown"
        );
        assert!(!expired, "restart clears the persistent alarm banner");
        assert_eq!(timer.remaining_ms(2500), 1000);
        assert!(timer.poll_expired(3500));
        assert!(!timer.poll_expired(3600));
        apply_countdown_action(&mut timer, &mut duration, &mut expired, 30, true, 4000);
        assert!(!timer.is_running());
        assert_eq!(timer.remaining_ms(4000), 30000);
    }
    #[test]
    fn tick_is_fast_only_while_visible() {
        let state = |visible, watching| TickState { visible, watching };
        assert_eq!(next_tick_interval(&state(true, true)), Some(FAST_TICK));
        assert_eq!(next_tick_interval(&state(true, false)), Some(FAST_TICK));
        assert_eq!(next_tick_interval(&state(false, true)), Some(IDLE_TICK));
        assert_eq!(next_tick_interval(&state(false, false)), None);
        assert!(IDLE_TICK >= Duration::from_secs(1) && FAST_TICK <= Duration::from_millis(200));
    }
    #[test]
    fn expiry_timer_fires_at_deadline_only_while_running() {
        assert_eq!(expiry_delay(false, 5000), None);
        assert_eq!(expiry_delay(true, 5000), Some(Duration::from_millis(5010)));
        assert_eq!(expiry_delay(true, 0), Some(Duration::from_millis(10)));
        assert!(expiry_delay(true, u64::MAX).is_some());
    }
    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn capture_cli_requires_exact_directory_shape() {
        assert_eq!(
            parse_arguments(&args(&["--capture-preview", "C:/temp/previews"])),
            Ok(Mode::Capture("C:/temp/previews".into()))
        );
        for invalid in [
            args(&["--capture-preview"]),
            args(&["--capture-preview", ""]),
            args(&["--capture-preview", "--background"]),
            args(&["--capture-preview", "out", "extra"]),
        ] {
            assert!(parse_arguments(&invalid).is_err());
        }
        assert_eq!(parse_arguments(&[]), Ok(Mode::Normal(false)));
        assert_eq!(
            parse_arguments(&args(&["--background"])),
            Ok(Mode::Normal(true))
        );
    }
    #[test]
    fn captures_have_five_distinct_ordered_png_names() {
        assert_eq!(
            (0..TAB_COUNT).map(capture_name).collect::<Vec<_>>(),
            vec![
                "01-audio.png",
                "02-displays.png",
                "03-system.png",
                "04-zeit.png",
                "05-einstellungen.png"
            ]
        );
    }
    #[test]
    fn hover_tip_overlay_is_last_and_clamps_to_window() {
        let ui: String = include_str!("../ui/main.slint")
            .split_whitespace()
            .collect();
        let tip = ui.find("HoverTip{").expect("overlay");
        let dropdown = ui
            .find("ifroot.dropdown-open:DropdownList")
            .expect("dropdown");
        assert!(tip > dropdown, "tooltip is the last overlay");
        assert!(ui[tip..].matches("}").count() >= 1 && !ui[tip..].contains("TouchArea"));
        let tooltip: String = include_str!("../ui/tooltip.slint")
            .split_whitespace()
            .collect();
        assert!(tooltip.contains("min(label.preferred-width+20px,300px)"));
        assert!(tooltip.contains("root.width-self.width-4px"));
        assert!(tooltip.contains("interval:450ms"));
    }
    #[test]
    fn panel_size_is_fixed_in_slint() {
        let ui = include_str!("../ui/main.slint");
        let compact: String = ui.split_whitespace().collect();
        let (w, h) = PANEL_SIZE;
        let area = platform::Area {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let bounds = platform::popup_bounds(area, (900, 500), 1.0);
        assert_eq!(bounds.physical_size, (w, h), "Rust popup size");
        for p in ["preferred", "min", "max"] {
            assert!(compact.contains(&format!("{p}-width:{w}px;")), "{p}-width");
            assert!(
                compact.contains(&format!("{p}-height:{h}px;")),
                "{p}-height"
            );
        }
        // The audio collapse feature was removed; the property must not come back.
        assert!(!compact.contains("apps-open"));
    }
    #[test]
    fn peaks_update_only_changed_rows_and_reset_missing_ones() {
        let row = |id: &str, value: i32, peak: i32| AudioItem {
            id: id.into(),
            value,
            peak,
            ..Default::default()
        };
        let model = VecModel::from(vec![row("a", 40, 10), row("b", 55, 0), row("c", 70, 30)]);
        let rows =
            |m: &VecModel<AudioItem>| m.iter().map(|r| (r.value, r.peak)).collect::<Vec<_>>();
        apply_peaks(&model, &[("a".into(), 80), ("b".into(), 0)]);
        // "a" rose, "b" unchanged, "c" is not reported anymore and drops to zero.
        assert_eq!(rows(&model), [(40, 80), (55, 0), (70, 0)]);
        assert_eq!(model.row_data(0).unwrap().peak_color, peak_color(80));
        // Volume values are never touched by level updates.
        apply_peaks(&model, &[]);
        assert_eq!(rows(&model), [(40, 0), (55, 0), (70, 0)]);
        assert_eq!(model.row_data(0).unwrap().peak_color, peak_color(0));
    }
    #[test]
    fn tab_constants_match_slint() {
        let ui = include_str!("../ui/main.slint");
        let compact: String = ui.split_whitespace().collect();
        assert!(compact.contains(r#"["Audio","Displays","System","Zeit","Einstellungen"]"#));
        for i in 0..TAB_COUNT {
            assert!(compact.contains(&format!("root.tab=={i}:")), "tab {i}");
        }
        assert!(compact.contains(&format!(
            "root.tab=={TAB_SYSTEM}:VerticalLayout{{spacing:6px"
        )));
        assert!(compact.contains(&format!(
            "root.tab=={TAB_SETTINGS}:VerticalLayout{{spacing:12px"
        )));
        assert!(compact.contains(&format!(
            "root.tab=={TAB_TIMER}:VerticalLayout{{spacing:8px"
        )));
        assert!(compact.contains(&format!(
            "root.tab=={TAB_AUDIO}:VerticalLayout{{vertical-stretch:1"
        )));
        assert_eq!(
            (TAB_AUDIO, TAB_SYSTEM, TAB_TIMER, TAB_SETTINGS),
            (0, 2, 3, 4)
        );
    }
    #[test]
    fn wait_pid_argument_is_extracted_and_validated() {
        let mut a = args(&["--background", "--wait-pid", "1234"]);
        assert_eq!(take_wait_pid(&mut a), Ok(Some(1234)));
        assert_eq!(a, args(&["--background"]));
        assert_eq!(parse_arguments(&a), Ok(Mode::Normal(true)));
        let mut none = args(&["--background"]);
        assert_eq!(take_wait_pid(&mut none), Ok(None));
        assert_eq!(none, args(&["--background"]));
        for bad in [
            args(&["--wait-pid"]),
            args(&["--wait-pid", "abc"]),
            args(&["--wait-pid", "0"]),
            args(&["--wait-pid", "-1"]),
            args(&["--wait-pid", "99999999999"]),
        ] {
            assert!(take_wait_pid(&mut bad.clone()).is_err(), "{bad:?}");
        }
        // Anything left after removing the flag is still validated by the normal parser.
        let mut extra = args(&["--wait-pid", "5", "--bogus"]);
        take_wait_pid(&mut extra).unwrap();
        assert!(parse_arguments(&extra).is_err());
    }
    #[test]
    fn sanitize_tab_clamps_out_of_range_to_audio() {
        for valid in 0..TAB_COUNT as u8 {
            assert_eq!(sanitize_tab(valid), i32::from(valid));
        }
        for invalid in [TAB_COUNT as u8, 6, 255] {
            assert_eq!(sanitize_tab(invalid), TAB_AUDIO);
        }
    }
}
