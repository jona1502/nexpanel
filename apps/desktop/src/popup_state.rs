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
    /// The window is hidden now. Also sent when it was hidden from outside (state not Closed
    /// but window invisible).
    HideDone,
    /// Periodic reconciliation. `focused`: the popup is the foreground window (or focus
    /// closing is off). `tray_pressed`: primary button held over the tray icon, only
    /// meaningful if [`Popup::needs_press_state`] asked for it.
    Tick {
        focused: bool,
        tray_pressed: bool,
    },
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
    /// First tick that saw the button released (only for press evidence).
    released_at: Option<u64>,
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

    /// Should the caller sample the tray press state for the next `Tick`? Only when a lost
    /// focus could close the popup or press evidence is waiting for its toggle.
    pub fn needs_press_state(&self, focused: bool) -> bool {
        match self.state {
            State::Open | State::Opening => !focused,
            _ => self.blurred.is_some_and(|b| b.tray_pressed),
        }
    }

    fn blur(&mut self, tray_pressed: bool, now: u64) -> Action {
        self.blurred = Some(Blurred {
            at: now,
            tray_pressed,
            released_at: None,
        });
        self.close()
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
            (Event::Blur { tray_pressed }, Open) => self.blur(tray_pressed, now_ms),
            // Focus was lost without a blur event we could act on (the grace period swallowed
            // it, or the shell took activation away): same as a late blur.
            (
                Event::Tick {
                    focused: false,
                    tray_pressed,
                },
                Open,
            ) => self.blur(tray_pressed, now_ms),
            (Event::Tick { tray_pressed, .. }, Closed) => {
                // The press that closed the popup ended elsewhere: no toggle will follow.
                // Grace of `BLUR_FALLBACK_MS` after the first sighting of the release, so a
                // toggle that is still on its way is swallowed.
                if let Some(b) = self.blurred.as_mut().filter(|b| b.tray_pressed) {
                    if tray_pressed {
                        b.released_at = None;
                    } else {
                        let since = *b.released_at.get_or_insert(now_ms);
                        if now_ms.saturating_sub(since) >= BLUR_FALLBACK_MS {
                            self.blurred = None;
                        }
                    }
                }
                Action::Nothing
            }
            (Event::Tick { .. }, _) => Action::Nothing,
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

    const TICK_LOST: Event = Tick {
        focused: false,
        tray_pressed: false,
    };
    const TICK_HELD: Event = Tick {
        focused: true,
        tray_pressed: true,
    };
    const TICK_FREE: Event = Tick {
        focused: true,
        tray_pressed: false,
    };

    #[test]
    fn tick_transitions() {
        let cases: &[(&str, &[Step])] = &[
            (
                "lost focus after grace closes",
                &[OPEN, (TICK_LOST, 1200, Hide)],
            ),
            (
                "lost focus within grace ignored (still opening)",
                &[OPEN, (TICK_LOST, 1100, Nothing), (TrayToggle, 1120, Hide)],
            ),
            (
                "grace expired without focus: closes on first tick",
                &[OPEN, (BLUR, 1050, Nothing), (TICK_LOST, 1150, Hide)],
            ),
            (
                "focused tick keeps open",
                &[OPEN, (TICK_FREE, 5000, Nothing)],
            ),
            ("tick while closed", &[(TICK_LOST, 5, Nothing)]),
            (
                "lost focus then quick toggle swallowed (fallback)",
                &[OPEN, (TICK_LOST, 2000, Hide), (TrayToggle, 2100, Nothing)],
            ),
            (
                "external hide reconciled by HideDone, one click opens",
                &[OPEN, (HideDone, 2000, Nothing), (TrayToggle, 2100, Show)],
            ),
        ];
        for (name, steps) in cases {
            run(name, steps);
        }
    }

    #[test]
    fn press_evidence_discarded_after_release_elsewhere() {
        let mut p = Popup::new();
        p.handle(TrayToggle, 1000);
        assert_eq!(
            p.handle(
                Tick {
                    focused: false,
                    tray_pressed: true
                },
                1200
            ),
            Hide
        );
        p.handle(HideDone, 1200);
        assert!(p.needs_press_state(true));
        // Still held: evidence stays, however long.
        p.handle(TICK_HELD, 2000);
        p.handle(TICK_HELD, 4000);
        // Released: kept for the grace so a toggle in flight is swallowed ...
        p.handle(TICK_FREE, 4100);
        p.handle(TICK_FREE, 4100 + BLUR_FALLBACK_MS - 1);
        assert!(p.needs_press_state(true));
        // ... then dropped.
        p.handle(TICK_FREE, 4100 + BLUR_FALLBACK_MS);
        assert!(!p.needs_press_state(true));
        assert_eq!(
            p.handle(TrayToggle, 4500),
            Show,
            "next real click not swallowed"
        );
    }

    #[test]
    fn toggle_in_flight_after_release_still_swallowed() {
        let mut p = Popup::new();
        p.handle(TrayToggle, 1000);
        p.handle(
            Tick {
                focused: false,
                tray_pressed: true,
            },
            1200,
        );
        p.handle(HideDone, 1200);
        p.handle(TICK_FREE, 3000);
        assert_eq!(p.handle(TrayToggle, 3050), Nothing);
    }

    #[test]
    fn press_evidence_expires_by_age() {
        let mut p = Popup::new();
        p.handle(TrayToggle, 1000);
        p.handle(
            Tick {
                focused: false,
                tray_pressed: true,
            },
            1200,
        );
        p.handle(HideDone, 1200);
        p.handle(TICK_HELD, 1200 + PRESS_MAX_MS + 1);
        assert_eq!(p.handle(TrayToggle, 1200 + PRESS_MAX_MS + 2), Show);
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
