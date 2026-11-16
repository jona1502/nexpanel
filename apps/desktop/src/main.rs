#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use nexpanel::{
    config::{Accent, Config},
    platform,
    services::{self, AudioCommand, DisplayCommand},
    settings_worker::{self, Command},
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
type TimerCheck = Rc<RefCell<Option<Box<dyn Fn()>>>>;
/// Monotonic ms including standby (see `nexpanel::clock`); wall-clock jumps do not affect timers.
fn now() -> u64 {
    nexpanel::clock::mono_ms()
}
/// Polls expiry and refreshes the timer texts. Notifies (balloon, never a focus-stealing
/// panel) exactly once per expiry: `Countdown::poll_expired` latches.
fn update_timers(
    a: &MainWindow,
    countdown: &RefCell<Countdown>,
    stopwatch: &RefCell<Stopwatch>,
    drift: &nexpanel::clock::DriftDetector,
) {
    drift.check();
    let t = now();
    let mut c = countdown.borrow_mut();
    if c.poll_expired(t) {
        a.set_expired(true);
        if !tray::notify("NexPanel", "Timer abgelaufen") {
            platform::alarm();
        }
    }
    a.set_countdown_text(clock(c.remaining_ms(t).saturating_add(999)));
    a.set_countdown_running(c.is_running());
    let s = stopwatch.borrow();
    a.set_stopwatch_text(clock(s.elapsed_ms(t)));
    a.set_stopwatch_running(s.is_running());
}
fn clock(ms: u64) -> slint::SharedString {
    let s = ms / 1000;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60).into()
}
fn show(app: &MainWindow) {
    let _ = app.show();
    platform::hide_from_alt_tab(app.window());
    platform::position(app.window());
    app.window().with_winit_window(|w| w.focus_window());
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
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mode = parse_arguments(&args)?;
    let capture_mode = matches!(mode, Mode::Capture(_));
    if mode != Mode::Help {
        nexpanel::logging::init();
        log::info!(
            "NexPanel {} gestartet, Argumente: {args:?}",
            env!("CARGO_PKG_VERSION")
        );
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
        match platform::Instance::acquire()? {
            Some(instance) => Some(instance),
            None => return Ok(()),
        }
    };
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
        .select()?;
    let app = MainWindow::new()?;
    app.set_app_version(env!("CARGO_PKG_VERSION").into());
    let (dtx, drx, display_crashed) = services::display::spawn();
    let (atx, arx, audio_crashed) = services::audio::spawn();
    let (stx, srx, settings_thread) = settings_worker::spawn();
    // Persistent models: rows are updated in place so a dragged slider survives.
    let display_model = Rc::new(VecModel::<DisplayRow>::default());
    let audio_model = Rc::new(VecModel::<AudioItem>::default());
    app.set_displays(ModelRc::from(display_model.clone()));
    app.set_audio(ModelRc::from(audio_model.clone()));
    // Rows the user is currently changing ignore worker values for a short time.
    let display_hold = Rc::new(RefCell::new(LocalHold::default()));
    let audio_hold = Rc::new(RefCell::new(LocalHold::default()));
    let last_blur = Rc::new(Cell::new(None::<Instant>));
    let weak = app.as_weak();
    let blur = last_blur.clone();
    app.window().on_winit_window_event(move |window, event| {
        match event {
            winit::event::WindowEvent::Focused(false) => {
                if let Some(app) = weak.upgrade()
                    && !capture_mode
                    && app.get_focus_close()
                    && app.window().is_visible()
                {
                    blur.set(Some(Instant::now()));
                    let _ = window.hide();
                }
            }
            winit::event::WindowEvent::KeyboardInput { event, .. }
                if event.state == winit::event::ElementState::Pressed
                    && event.logical_key
                        == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape) =>
            {
                let _ = window.hide();
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
        move || {
            if let Some(app) = weak.upgrade() {
                let _ = app.hide();
            }
            slint::CloseRequestResponse::KeepWindowShown
        }
    });
    app.on_hide_panel({
        let weak = app.as_weak();
        move || {
            if let Some(app) = weak.upgrade() {
                let _ = app.hide();
            }
        }
    });
    // Filled once the timers exist; lets the tray trigger an immediate check after resume.
    let timer_check: TimerCheck = Rc::default();
    let tray = if capture_mode {
        None
    } else {
        Some(tray::TrayHost::new(Box::new({
            let weak = app.as_weak();
            let timer_check = timer_check.clone();
            move |event| {
                let Some(app) = weak.upgrade() else {
                    return;
                };
                match event {
                    TrayEvent::Toggle if app.window().is_visible() => {
                        let _ = app.hide();
                    }
                    TrayEvent::Toggle => {
                        if last_blur
                            .get()
                            .is_none_or(|t| t.elapsed() > Duration::from_millis(250))
                        {
                            show(&app);
                        }
                    }
                    TrayEvent::Open => show(&app),
                    TrayEvent::NotificationClicked => {
                        app.set_tab(2);
                        show(&app);
                    }
                    TrayEvent::Resumed => {
                        if let Some(check) = timer_check.borrow().as_ref() {
                            check();
                        }
                    }
                    TrayEvent::Quit => {
                        let _ = slint::quit_event_loop();
                    }
                }
            }
        }))?)
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
    let stopwatch = Rc::new(RefCell::new(Stopwatch::default()));
    app.on_countdown_action({
        let c = countdown.clone();
        let weak = app.as_weak();
        move |action| {
            if let Some(a) = weak.upgrade() {
                let mut c = c.borrow_mut();
                let mut selected_duration = duration.get();
                let mut expired = a.get_expired();
                apply_countdown_action(
                    &mut c,
                    &mut selected_duration,
                    &mut expired,
                    a.get_seconds(),
                    action == 1,
                    now(),
                );
                duration.set(selected_duration);
                a.set_expired(expired);
                a.set_countdown_running(c.is_running());
                a.set_countdown_text(clock(c.remaining_ms(now()).saturating_add(999)));
            }
        }
    });
    app.on_stopwatch_action({
        let s = stopwatch.clone();
        move |action| {
            let mut s = s.borrow_mut();
            if action == 1 {
                s.reset();
            } else if s.is_running() {
                s.pause(now());
            } else {
                s.start(now());
            }
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
    let tick = slint::Timer::default();
    let ready = Rc::new(Cell::new((false, false)));
    let snapshot_ready = ready.clone();
    let weak = app.as_weak();
    let mut visible = None;
    let mut crash_shown = (false, false);
    tick.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(200),
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
                let mut rows: Vec<_> = s
                    .master
                    .into_iter()
                    .chain(s.sessions)
                    .map(|r| AudioItem {
                        id: r.id.into(),
                        name: r.name.into(),
                        value: r.volume as i32,
                        muted: r.muted,
                    })
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
                show(&a);
            }
            update_timers(&a, &countdown, &stopwatch, &drift);
            let active = a.window().is_visible();
            if visible != Some(active) {
                visible = Some(active);
                let _ = dtx.send(DisplayCommand::SetActive(active));
                let _ = atx.send(AudioCommand::SetActive(active));
            }
        },
    );
    if mode != Mode::Normal(true) {
        show(&app);
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
