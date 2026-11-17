//! Glue between the NexSearch window (`ui/search.slint`), the search worker thread and the app.
//!
//! Queries run on a worker thread that owns the (non-`Send`) `SearchEngine`; the UI thread only
//! sends the newest query and applies the newest answer. All decisions (selection, keys,
//! geometry, action dispatch) live in `nexpanel::search_window` and are unit tested there.

use crate::{SearchItem, SearchWindow};
use nexpanel::{
    config::SearchPosition,
    nexsearch::{NexCommand, SearchResult},
    platform,
    search_window::{self as sw, ActionHost, SearchKey, Selection},
    tray,
};
use slint::{
    ComponentHandle, ModelRc, VecModel,
    winit_030::{EventResult, WinitWindowAccessor, winit},
};
use std::{
    cell::{Cell, RefCell},
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

struct Request {
    seq: u64,
    query: String,
}

struct Response {
    seq: u64,
    results: Vec<SearchResult>,
}

/// Newest answer of the worker; older ones are overwritten.
type Mailbox = Arc<Mutex<Option<Response>>>;

/// Delay between `show` and the fade-in: the first frame must be drawn with `open == false`.
const OPEN_DELAY: Duration = Duration::from_millis(10);

pub fn to_item(result: &SearchResult) -> SearchItem {
    SearchItem {
        title: result.title.as_str().into(),
        subtitle: result.subtitle.as_str().into(),
        glyph: sw::glyph(&result.action).into(),
        hint: sw::action_hint(&result.action).into(),
    }
}

/// Runs the results' side effects; app commands go to the closure provided by `main`.
struct UiHost<'a> {
    run_command: &'a dyn Fn(NexCommand),
}

impl ActionHost for UiHost<'_> {
    fn launch(&mut self, path: &Path) -> Result<(), String> {
        sw::launch(path)
    }
    fn copy_text(&mut self, text: &str) -> Result<(), String> {
        sw::copy_text(text)
    }
    fn command(&mut self, command: NexCommand) {
        (self.run_command)(command);
    }
}

struct State {
    selection: RefCell<Selection>,
    results: RefCell<Vec<SearchResult>>,
    model: Rc<VecModel<SearchItem>>,
    /// Id of the newest query; answers with another id are stale.
    seq: Arc<AtomicU64>,
    tx: mpsc::Sender<Request>,
    mailbox: Mailbox,
    shown_at: Cell<Option<Instant>>,
    position: Cell<SearchPosition>,
    run_command: Rc<dyn Fn(NexCommand)>,
    open_timer: slint::Timer,
    focus_timer: slint::Timer,
}

impl State {
    fn set_height(&self, win: &SearchWindow, height: f32) {
        win.set_win_height(height);
        if win.window().is_visible() {
            win.window()
                .set_size(slint::LogicalSize::new(sw::WINDOW_WIDTH, height));
        }
    }

    fn apply_results(&self, win: &SearchWindow, results: Vec<SearchResult>, query_blank: bool) {
        let status = if results.is_empty() && !query_blank {
            sw::NO_RESULTS
        } else {
            ""
        };
        self.selection.borrow_mut().set_len(results.len());
        self.model
            .set_vec(results.iter().map(to_item).collect::<Vec<_>>());
        let height = sw::window_height(results.len(), !status.is_empty());
        *self.results.borrow_mut() = results;
        win.set_selected(0);
        win.set_status(status.into());
        self.set_height(win, height);
    }

    fn on_edited(&self, win: &SearchWindow, text: &str) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        if text.trim().is_empty() {
            self.apply_results(win, Vec::new(), true);
        } else if self
            .tx
            .send(Request {
                seq,
                query: text.to_owned(),
            })
            .is_err()
        {
            log::error!("NexSearch-Worker ist nicht mehr erreichbar");
        }
    }

    fn on_results_ready(&self, win: &SearchWindow) {
        let response = self
            .mailbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(response) = response
            && response.seq == self.seq.load(Ordering::SeqCst)
        {
            let blank = win.get_query().trim().is_empty();
            self.apply_results(win, response.results, blank);
        }
    }

    fn navigate(&self, win: &SearchWindow, delta: i32) {
        let mut selection = self.selection.borrow_mut();
        selection.move_by(delta);
        if let Some(index) = selection.index() {
            win.set_selected(index as i32);
        }
    }

    fn hover(&self, win: &SearchWindow, index: i32) {
        if let Ok(index) = usize::try_from(index)
            && self.selection.borrow_mut().select(index)
        {
            win.set_selected(index as i32);
        }
    }

    fn activate_selected(&self, win: &SearchWindow) {
        let index = self.selection.borrow().index();
        if let Some(index) = index {
            self.activate(win, index);
        }
    }

    fn activate(&self, win: &SearchWindow, index: usize) {
        let action = self.results.borrow().get(index).map(|r| r.action.clone());
        let Some(action) = action else {
            return;
        };
        // Hide first: the launched program or the panel must get the focus back.
        self.hide(win);
        let mut host = UiHost {
            run_command: &*self.run_command,
        };
        sw::execute(&mut host, &action);
    }

    fn hide(&self, win: &SearchWindow) {
        self.open_timer.stop();
        self.focus_timer.stop();
        self.shown_at.set(None);
        // Invalidates answers that are still in flight.
        self.seq.fetch_add(1, Ordering::SeqCst);
        win.set_open(false);
        let _ = win.hide();
    }

    fn show(
        self: &Rc<Self>,
        win: &SearchWindow,
        accent: slint::Color,
        animations: bool,
        position: SearchPosition,
    ) {
        self.position.set(position);
        self.seq.fetch_add(1, Ordering::SeqCst);
        self.mailbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        win.set_query("".into());
        win.set_accent(accent);
        win.set_animations(animations);
        win.set_open(false);
        self.apply_results(win, Vec::new(), true);
        let height = sw::window_height(0, false);
        // Same dance as the tray popup: Slint creates the native window lazily on the first
        // show, so position before (later shows) and after (first show).
        sw::place_on_cursor_monitor(win.window(), height, position);
        let _ = win.show();
        platform::hide_from_alt_tab(win.window());
        sw::place_on_cursor_monitor(win.window(), height, position);
        win.window().with_winit_window(|w| w.focus_window());
        win.invoke_focus_input();
        self.shown_at.set(Some(Instant::now()));
        let weak = win.as_weak();
        self.open_timer
            .start(slint::TimerMode::SingleShot, OPEN_DELAY, move || {
                if let Some(win) = weak.upgrade() {
                    win.set_open(true);
                }
            });
        // A blur that never arrives (focus denied, shell stole activation) must not leave the
        // window stuck on screen.
        let (weak, state) = (win.as_weak(), Rc::downgrade(self));
        self.focus_timer.start(
            slint::TimerMode::SingleShot,
            sw::FOCUS_CHECK_DELAY,
            move || {
                if let (Some(win), Some(state)) = (weak.upgrade(), state.upgrade())
                    && win.window().is_visible()
                    && !tray::is_foreground(win.window())
                {
                    state.hide(&win);
                }
            },
        );
    }
}

fn spawn_worker(
    rx: mpsc::Receiver<Request>,
    seq: Arc<AtomicU64>,
    mailbox: Mailbox,
    weak: slint::Weak<SearchWindow>,
) {
    let spawned = std::thread::Builder::new()
        .name("nexpanel-search".into())
        .spawn(move || {
            // Providers are not `Send`: the engine is created and used on this thread only.
            let engine = sw::build_engine();
            while let Ok(mut request) = rx.recv() {
                // Typing is faster than searching: only the newest query matters.
                while let Ok(newer) = rx.try_recv() {
                    request = newer;
                }
                if request.seq != seq.load(Ordering::SeqCst) {
                    continue;
                }
                let results = catch_unwind(AssertUnwindSafe(|| engine.search(&request.query)))
                    .unwrap_or_else(|_| {
                        log::error!("Panik in der NexSearch-Suche abgefangen");
                        Vec::new()
                    });
                if request.seq != seq.load(Ordering::SeqCst) {
                    continue;
                }
                *mailbox.lock().unwrap_or_else(|e| e.into_inner()) = Some(Response {
                    seq: request.seq,
                    results,
                });
                if weak
                    .upgrade_in_event_loop(|win| win.invoke_results_ready())
                    .is_err()
                {
                    break;
                }
            }
        });
    if let Err(error) = spawned {
        log::error!("NexSearch-Worker konnte nicht starten: {error}");
    }
}

/// Owns the search window. Keep it alive for the whole run of the event loop.
pub struct SearchController {
    win: SearchWindow,
    state: Rc<State>,
}

impl SearchController {
    /// `run_command` performs NexPanel commands (open settings, quit, toggle autostart).
    pub fn new(run_command: Rc<dyn Fn(NexCommand)>) -> Result<Self, slint::PlatformError> {
        let win = SearchWindow::new()?;
        let (tx, rx) = mpsc::channel();
        let seq = Arc::new(AtomicU64::new(0));
        let mailbox: Mailbox = Arc::default();
        let model = Rc::new(VecModel::<SearchItem>::default());
        win.set_results(ModelRc::from(model.clone()));
        let state = Rc::new(State {
            selection: RefCell::new(Selection::new()),
            results: RefCell::new(Vec::new()),
            model,
            seq: seq.clone(),
            tx,
            mailbox: mailbox.clone(),
            shown_at: Cell::new(None),
            position: Cell::new(SearchPosition::default()),
            run_command,
            open_timer: slint::Timer::default(),
            focus_timer: slint::Timer::default(),
        });
        spawn_worker(rx, seq, mailbox, win.as_weak());

        win.on_edited({
            let (weak, state) = (win.as_weak(), state.clone());
            move |text| {
                if let Some(win) = weak.upgrade() {
                    state.on_edited(&win, text.as_str());
                }
            }
        });
        win.on_results_ready({
            let (weak, state) = (win.as_weak(), state.clone());
            move || {
                if let Some(win) = weak.upgrade() {
                    state.on_results_ready(&win);
                }
            }
        });
        win.on_activate({
            let (weak, state) = (win.as_weak(), state.clone());
            move |index| {
                if let (Some(win), Ok(index)) = (weak.upgrade(), usize::try_from(index)) {
                    state.activate(&win, index);
                }
            }
        });
        win.on_hover_row({
            let (weak, state) = (win.as_weak(), state.clone());
            move |index| {
                if let Some(win) = weak.upgrade() {
                    state.hover(&win, index);
                }
            }
        });
        win.window().on_winit_window_event({
            let (weak, state) = (win.as_weak(), state.clone());
            move |_, event| {
                use winit::event::{ElementState, WindowEvent};
                match event {
                    WindowEvent::Focused(false) => {
                        if let Some(win) = weak.upgrade()
                            && win.window().is_visible()
                            && state
                                .shown_at
                                .get()
                                .is_some_and(|t| sw::should_close_on_blur(t.elapsed()))
                        {
                            state.hide(&win);
                        }
                    }
                    WindowEvent::KeyboardInput { event: key, .. }
                        if key.state == ElementState::Pressed =>
                    {
                        if let Some(action) = sw::key_action(&key.logical_key) {
                            if let Some(win) = weak.upgrade() {
                                match action {
                                    SearchKey::Up => state.navigate(&win, -1),
                                    SearchKey::Down => state.navigate(&win, 1),
                                    SearchKey::Activate if !key.repeat => {
                                        state.activate_selected(&win)
                                    }
                                    SearchKey::Close if !key.repeat => state.hide(&win),
                                    SearchKey::Activate | SearchKey::Close => {}
                                }
                            }
                            // Keep Slint's text input from also moving its cursor or accepting.
                            return EventResult::PreventDefault;
                        }
                    }
                    WindowEvent::ScaleFactorChanged { .. } => {
                        // Apply after Slint has processed the new scale, outside this callback.
                        let weak = weak.clone();
                        let position = state.position.get();
                        slint::Timer::single_shot(Duration::ZERO, move || {
                            if let Some(win) = weak.upgrade()
                                && win.window().is_visible()
                            {
                                sw::place_on_cursor_monitor(
                                    win.window(),
                                    win.get_win_height(),
                                    position,
                                );
                            }
                        });
                    }
                    _ => {}
                }
                EventResult::Propagate
            }
        });
        Ok(Self { win, state })
    }

    /// Hotkey handler: opens the window on the cursor's monitor, or closes it again.
    pub fn toggle(&self, accent: slint::Color, animations: bool, position: SearchPosition) {
        if self.win.window().is_visible() {
            self.state.hide(&self.win);
        } else {
            self.state.show(&self.win, accent, animations, position);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexpanel::nexsearch::SearchAction;
    use std::path::PathBuf;

    #[test]
    fn items_carry_text_glyph_and_hint() {
        let result = SearchResult {
            id: "calc".into(),
            title: "42".into(),
            subtitle: "6*7".into(),
            score: 1.0,
            action: SearchAction::CopyText("42".into()),
        };
        let item = to_item(&result);
        assert_eq!(item.title, "42");
        assert_eq!(item.subtitle, "6*7");
        assert_eq!(item.glyph, sw::glyph(&result.action));
        assert_eq!(item.hint, "Kopieren");
        let launch = SearchResult {
            action: SearchAction::Launch(PathBuf::from("a.lnk")),
            ..result
        };
        assert_eq!(to_item(&launch).hint, "Öffnen");
    }

    #[test]
    fn settings_dropdown_labels_match_the_config_enums() {
        let ui: String = include_str!("../ui/main.slint")
            .split_whitespace()
            .collect();
        let hotkeys: Vec<String> = nexpanel::config::SearchHotkey::ALL
            .iter()
            .map(|h| format!("\"{}\"", h.hotkey().label()))
            .collect();
        assert!(
            ui.contains(&format!("hotkey-names:[{}];", hotkeys.join(","))),
            "{hotkeys:?}"
        );
        assert_eq!(nexpanel::config::SearchPosition::ALL.len(), 2);
        assert!(ui.contains(r#"position-names:["Mitte","Oben"];"#));
    }
}
