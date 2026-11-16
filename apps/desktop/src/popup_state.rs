//! Popup open/close state machine (ADR-002).
//!
//! Replaces the old "ignore a toggle 250 ms after a blur" heuristic. A click on the tray icon
//! while the popup is open first takes focus away (blur, on mouse-down) and only later, on
//! mouse-up, sends the toggle. Without help the toggle would reopen the popup that this very
//! click just closed.
//!
//! The machine is pure: time is passed in (`now_ms`, any monotonic millisecond clock) and the
//! caller performs the returned [`Action`]. At blur time the caller reports whether the left
//! mouse button was held over the tray icon (`tray_pressed`). Then the matching toggle is
//! swallowed however long the press lasts (press-and-hold), up to [`PRESS_MAX_MS`]. Without
//! such evidence a toggle shortly after the blur ([`BLUR_FALLBACK_MS`]) is swallowed as before.

/// Longest mouse-down to mouse-up time still counted as one click that closed the popup.
pub const PRESS_MAX_MS: u64 = 5000;
/// Fallback window when the blur carried no press evidence.
pub const BLUR_FALLBACK_MS: u64 = 250;
/// Blurs this soon after opening are focus noise of the show sequence and are ignored.
pub const OPENING_GRACE_MS: u64 = 150;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Closed,
    Opening,
    Open,
    Closing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Tray left click / keyboard activation.
    TrayToggle,
    /// Tray menu "Öffnen" / "Einstellungen".
    TrayOpen,
    /// Popup lost focus. `tray_pressed`: left button down over the tray icon right now.
    Blur {
        tray_pressed: bool,
    },
    Esc,
    /// Open from elsewhere (startup, second instance, timer).
    ShowRequested,
    /// The window is hidden now.
    HideDone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Nothing,
    Show,
    Hide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Blurred {
    at: u64,
    tray_pressed: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Popup {
    state: State,
    opened_at: u64,
    blurred: Option<Blurred>,
}

impl Default for Popup {
    fn default() -> Self {
        Self::new()
    }
}

impl Popup {
    pub fn new() -> Self {
        Self {
            state: State::Closed,
            opened_at: 0,
            blurred: None,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    fn open(&mut self, now: u64) -> Action {
        self.state = State::Opening;
        self.opened_at = now;
        self.blurred = None;
        Action::Show
    }

    fn close(&mut self) -> Action {
        self.state = State::Closing;
        Action::Hide
    }

    /// Does a toggle at `now` belong to the click that closed the popup by blur?
    fn swallows_toggle(&self, now: u64) -> bool {
        self.blurred.is_some_and(|b| {
            let age = now.saturating_sub(b.at);
            if b.tray_pressed {
                age <= PRESS_MAX_MS
            } else {
                age <= BLUR_FALLBACK_MS
            }
        })
    }

    pub fn handle(&mut self, event: Event, now_ms: u64) -> Action {
        if self.state == State::Opening && now_ms.saturating_sub(self.opened_at) >= OPENING_GRACE_MS
        {
            self.state = State::Open;
        }
        use State::*;
        match (event, self.state) {
            (Event::TrayToggle, Open | Opening) => self.close(),
            (Event::TrayToggle, Closing) => Action::Nothing,
            (Event::TrayToggle, Closed) => {
                if self.swallows_toggle(now_ms) {
                    self.blurred = None;
                    Action::Nothing
                } else {
                    self.open(now_ms)
                }
            }
            (Event::TrayOpen | Event::ShowRequested, Closed | Closing) => self.open(now_ms),
            // Already visible: only bring it to front.
            (Event::TrayOpen | Event::ShowRequested, Open | Opening) => Action::Show,
            (Event::Blur { .. }, Opening) => Action::Nothing,
            (Event::Blur { tray_pressed }, Open) => {
                self.blurred = Some(Blurred {
                    at: now_ms,
                    tray_pressed,
                });
                self.close()
            }
            (Event::Blur { .. }, Closed | Closing) => Action::Nothing,
            (Event::Esc, Open | Opening) => {
                self.blurred = None;
                self.close()
            }
            (Event::Esc, Closed | Closing) => Action::Nothing,
            (Event::HideDone, _) => {
                self.state = Closed;
                Action::Nothing
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Action::{Hide, Nothing, Show};
    use super::*;
    use Event::*;

    type Step = (Event, u64, Action);
    const BLUR: Event = Blur {
        tray_pressed: false,
    };
    const BLUR_PRESS: Event = Blur { tray_pressed: true };
    const OPEN: Step = (TrayToggle, 1000, Show);

    /// Feeds `steps` and asserts each action; `HideDone` follows every `Hide` at once, like
    /// the synchronous `hide()` in the app.
    fn run(name: &str, steps: &[Step]) {
        let mut p = Popup::new();
        for (i, &(event, t, expected)) in steps.iter().enumerate() {
            let got = p.handle(event, t);
            assert_eq!(got, expected, "{name}, step {i}: {event:?} at {t}");
            if got == Hide {
                p.handle(HideDone, t);
            }
        }
    }

    #[test]
    fn transitions() {
        let cases: &[(&str, &[Step])] = &[
            ("toggle opens", &[OPEN]),
            ("toggle closes", &[OPEN, (TrayToggle, 2000, Hide)]),
            (
                "toggle reopens",
                &[OPEN, (TrayToggle, 2000, Hide), (TrayToggle, 3000, Show)],
            ),
            (
                "rapid double click closes, then reopens",
                &[OPEN, (TrayToggle, 2000, Hide), (TrayToggle, 2050, Show)],
            ),
            ("esc closes", &[OPEN, (Esc, 1500, Hide)]),
            ("esc while closed", &[(Esc, 10, Nothing)]),
            (
                "esc then toggle reopens",
                &[OPEN, (Esc, 1500, Hide), (TrayToggle, 1600, Show)],
            ),
            (
                "blur then quick toggle swallowed (fallback)",
                &[OPEN, (BLUR, 2000, Hide), (TrayToggle, 2100, Nothing)],
            ),
            (
                "blur then late toggle opens",
                &[OPEN, (BLUR, 2000, Hide), (TrayToggle, 2400, Show)],
            ),
            (
                "blur swallows only once",
                &[
                    OPEN,
                    (BLUR, 2000, Hide),
                    (TrayToggle, 2100, Nothing),
                    (TrayToggle, 2200, Show),
                ],
            ),
            (
                "press-and-hold 3 s swallowed",
                &[OPEN, (BLUR_PRESS, 2000, Hide), (TrayToggle, 5000, Nothing)],
            ),
            (
                "press-and-hold at limit swallowed",
                &[OPEN, (BLUR_PRESS, 2000, Hide), (TrayToggle, 7000, Nothing)],
            ),
            (
                "press-and-hold beyond limit opens",
                &[OPEN, (BLUR_PRESS, 2000, Hide), (TrayToggle, 7001, Show)],
            ),
            (
                "press-and-hold swallowed once, next click opens",
                &[
                    OPEN,
                    (BLUR_PRESS, 2000, Hide),
                    (TrayToggle, 3000, Nothing),
                    (TrayToggle, 3500, Show),
                ],
            ),
            (
                "menu open after blur shows and clears evidence",
                &[
                    OPEN,
                    (BLUR_PRESS, 2000, Hide),
                    (TrayOpen, 2050, Show),
                    (Esc, 2400, Hide),
                    (TrayToggle, 2450, Show),
                ],
            ),
            (
                "blur during opening grace ignored",
                &[OPEN, (BLUR, 1050, Nothing), (TrayToggle, 1100, Hide)],
            ),
            ("blur after grace closes", &[OPEN, (BLUR, 1200, Hide)]),
            ("blur while closed ignored", &[(BLUR, 5, Nothing)]),
            ("show requested opens", &[(ShowRequested, 5, Show)]),
            (
                "show requested while open only raises",
                &[OPEN, (ShowRequested, 3000, Show), (TrayToggle, 3100, Hide)],
            ),
        ];
        for (name, steps) in cases {
            run(name, steps);
        }
    }

    #[test]
    fn closing_ignores_toggle_until_hide_done() {
        let mut p = Popup::new();
        assert_eq!(p.handle(TrayToggle, 0), Show);
        assert_eq!(p.handle(TrayToggle, 500), Hide);
        assert_eq!(p.state(), State::Closing);
        assert_eq!(p.handle(TrayToggle, 510), Nothing);
        p.handle(HideDone, 520);
        assert_eq!(p.state(), State::Closed);
    }

    #[test]
    fn opening_settles_to_open() {
        let mut p = Popup::new();
        p.handle(TrayToggle, 0);
        assert_eq!(p.state(), State::Opening);
        p.handle(ShowRequested, OPENING_GRACE_MS);
        assert_eq!(p.state(), State::Open);
    }

    #[test]
    fn clock_going_backwards_does_not_panic() {
        let mut p = Popup::new();
        p.handle(TrayToggle, 5000);
        p.handle(BLUR, 6000);
        p.handle(HideDone, 6000);
        // Saturating age 0: still counts as the same click.
        assert_eq!(p.handle(TrayToggle, 10), Nothing);
    }
}
