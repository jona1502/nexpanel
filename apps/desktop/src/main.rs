#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use nexpanel::{
    config::{Accent, Config},
    platform,
    popup_state::{Action, Event, Popup, State},
    services::{self, AudioCommand, DisplayCommand},
    settings_worker::{self, Command},
    time_input::TimeInput,
    time_tools::{Countdown, Stopwatch},
    tray::{self, TrayEvent},
    ui_models::{LocalHold, keep_held, reconcile},
};
use slint::{
    ComponentHandle, Model, ModelRc, VecModel,
    winit_030::{EventResult, WinitWindowAccessor, winit},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
slint::include_modules!();
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
fn capture_name(tab: usize) -> &'static str {
    [
        "01-displays.png",
        "02-audio.png",
        "03-zeit.png",
        "04-einstellungen.png",
    ][tab]
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
/// Delay for the one-shot timer that wakes the loop exactly when a running countdown ends.
fn expiry_delay(running: bool, remaining_ms: u64) -> Option<Duration> {
    // +10 ms so the wake-up never lands before the deadline (poll_expired needs `now >= end`).
    running.then(|| Duration::from_millis(remaining_ms.saturating_add(10)))
}
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
        // first thing the user sees when the panel is opened next (tab 2 = timer).
        if !a.window().is_visible() {
            a.set_tab(2);
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
            let _ = app.hide();
            popup.borrow_mut().handle(Event::HideDone, ms);
        }
        Action::Nothing => {}
    }
}
fn config_from(app: &MainWindow) -> Config {
    Config {
        accent: match app.get_accent_index() {
            1 => Accent::Purple,
            2 => Accent::Green,
            3 => Accent::Orange,
            _ => Accent::Blue,
        },
        animations: app.get_animations(),
        close_on_focus_loss: app.get_focus_close(),
        linked_displays: app.get_linked(),
        autostart: app.get_autostart(),
    }
}
fn apply_config(app: &MainWindow, c: &Config) {
    app.set_accent_index(match c.accent {
        Accent::Blue => 0,
        Accent::Purple => 1,
        Accent::Green => 2,
        Accent::Orange => 3,
    });
    app.set_animations(c.animations);
    app.set_focus_close(c.close_on_focus_loss);
    app.set_linked(c.linked_displays);
    app.set_autostart(c.autostart);
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
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mode = parse_arguments(&args)?;
    let capture_mode = matches!(mode, Mode::Capture(_));
    // Normal starts log only after `acquire` succeeded: a second instance must not touch the log.
    if !matches!(mode, Mode::Help | Mode::Normal(_)) {
        start_logging(&args);
    }
    if mode == Mode::Help {
        println!(
            "NexPanel 0.1\n  --background   Im Tray starten\n  --diagnostics  Display/Audio nur lesen und beenden\n  --capture-preview <Verzeichnis>  Vier echte UI-Tabs als PNG speichern, dann beenden (nur lesend)\n  --help         Hilfe"
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
    let (atx, arx, audio_crashed) = services::audio::spawn();
    let (stx, srx, settings_thread) = settings_worker::spawn();
    let tick_body: TickBody = Rc::new(RefCell::new(Box::new(|| {})));
    // Persistent models: rows are updated in place so a dragged slider survives.
    let display_model = Rc::new(VecModel::<DisplayRow>::default());
    let audio_model = Rc::new(VecModel::<AudioItem>::default());
    app.set_displays(ModelRc::from(display_model.clone()));
    app.set_audio(ModelRc::from(audio_model.clone()));
    // Rows the user is currently changing ignore worker values for a short time.
    let display_hold = Rc::new(RefCell::new(LocalHold::default()));
    let audio_hold = Rc::new(RefCell::new(LocalHold::default()));
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
                        }
                    }
                    EscAction::ClosePanel => {
                        let mut popup = blur_popup.borrow_mut();
                        popup.handle(Event::Esc, ms());
                        if let Some(app) = weak.upgrade() {
                            app.set_dropdown_open(false);
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
                    move |event| {
                        let Some(app) = weak.upgrade() else {
                            return;
                        };
                        match event {
                            TrayEvent::Toggle => dispatch(&app, &popup, Event::TrayToggle, ms()),
                            TrayEvent::Open => dispatch(&app, &popup, Event::TrayOpen, ms()),
                            TrayEvent::Settings => {
                                app.set_tab(3);
                                dispatch(&app, &popup, Event::TrayOpen, ms());
                            }
                            // Before the first settings snapshot the save would be discarded
                            // while the property flips: ignore the click.
                            TrayEvent::ToggleAutostart if !app.get_settings_ready() => {}
                            TrayEvent::ToggleAutostart => {
                                // Same path as the checkbox in the settings tab.
                                app.set_autostart(!app.get_autostart());
                                app.invoke_save_settings();
                            }
                            TrayEvent::NotificationClicked => {
                                app.set_tab(2);
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
                            TrayEvent::Quit => {
                                let _ = slint::quit_event_loop();
                            }
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
        move || {
            if capture_mode {
                return;
            }
            if let Some(a) = weak.upgrade()
                && a.get_settings_ready()
            {
                let _ = tx.send(Command::Save(config_from(&a)));
            }
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
    let snapshot_ready = ready.clone();
    let weak = app.as_weak();
    let tick_popup = popup.clone();
    let mut visible = None;
    let mut crash_shown = (false, false);
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
                    }
                };
                let mut rows: Vec<_> = s
                    .master
                    .into_iter()
                    .map(|r| to_item(r, true))
                    .chain(s.sessions.into_iter().map(|r| to_item(r, false)))
                    .collect();
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
            if let Some(s) = srx.try_iter().last() {
                apply_config(&a, &s.config);
                a.set_settings_error(s.error.into());
                a.set_settings_blocked(s.blocked);
                a.set_settings_ready(true);
            }
            #[cfg(windows)]
            if instance.as_ref().is_some_and(|i| i.open_requested()) {
                dispatch(&a, &tick_popup, Event::ShowRequested, ms());
            }
            update_timers(&a, &countdown, &stopwatch, &drift);
            let mut active = a.window().is_visible();
            // Window hidden from outside: reconcile, or the next toggle would hide nothing.
            if !active && tick_popup.borrow().state() != State::Closed {
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
                    if tab == 4 {
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
    drop(tray);
    log::info!("Event-Loop beendet, Worker werden gestoppt");
    tick.stop();
    capture_timer.stop();
    drop(capture_timer);
    let _ = shutdown_display.send(DisplayCommand::Shutdown);
    let _ = shutdown_audio.send(AudioCommand::Shutdown);
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
    fn captures_have_four_distinct_ordered_png_names() {
        assert_eq!(
            (0..4).map(capture_name).collect::<Vec<_>>(),
            vec![
                "01-displays.png",
                "02-audio.png",
                "03-zeit.png",
                "04-einstellungen.png"
            ]
        );
    }
}
