//! Update state machine. Pure: events in, new state and one requested side effect out.
//! The worker thread performs the effects and feeds the results back as events.
//!
//! `Available -> Downloading` happens only through `InstallRequested`, i.e. the user's click on
//! "Jetzt aktualisieren". Nothing installs on its own; once clicked, the chain runs through
//! download, verification and installation without further prompts.

use super::release::Release;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// No connection, timeout, HTTP error status.
    Network,
    /// URL, redirect or host rejected by the policy.
    Rejected,
    TooLarge,
    HashMismatch,
    BadPackage,
    /// VERSION.txt does not match the release, or the release is not newer.
    VersionMismatch,
    /// The running exe is not a managed install (no install marker), e.g. a dev build.
    NotInstalled,
    NoWritePermission,
    InstallFailed,
    RestartFailed,
    Other,
}

impl ErrorKind {
    /// Stable numbers shared with the Slint UI (`update-error-kind`).
    pub fn code(self) -> i32 {
        match self {
            Self::Network => 1,
            Self::Rejected => 2,
            Self::TooLarge => 3,
            Self::HashMismatch => 4,
            Self::BadPackage => 5,
            Self::VersionMismatch => 6,
            Self::NotInstalled => 7,
            Self::NoWritePermission => 8,
            Self::InstallFailed => 9,
            Self::RestartFailed => 10,
            Self::Other => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateError {
    pub kind: ErrorKind,
    /// Technical detail for the log and the status line; not translated.
    pub detail: String,
}

impl UpdateError {
    pub fn new(kind: ErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Nothing checked yet.
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading {
        release: Release,
        done: u64,
        total: Option<u64>,
    },
    Verifying(Release),
    ReadyToInstall(Release),
    Installing(Release),
    Error {
        error: UpdateError,
        /// Set when the failure happened after an update was known: the user can retry it.
        retry: Option<Release>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    CheckRequested,
    CheckFinished(Result<Option<Release>, UpdateError>),
    /// The user clicked "Jetzt aktualisieren".
    InstallRequested,
    Progress {
        done: u64,
        total: Option<u64>,
    },
    Downloaded,
    Verified,
    InstallStarted,
    Installed,
    /// Download, verification or installation failed.
    Failed(UpdateError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    None,
    StartCheck,
    StartDownload(Release),
    StartInstall(Release),
    Restart,
}

impl State {
    /// Stable numbers shared with the Slint UI (`update-state`).
    pub fn code(&self) -> i32 {
        match self {
            Self::Idle => 0,
            Self::Checking => 1,
            Self::UpToDate => 2,
            Self::Available(_) => 3,
            Self::Downloading { .. } => 4,
            Self::Verifying(_) => 5,
            Self::ReadyToInstall(_) => 6,
            Self::Installing(_) => 7,
            Self::Error { .. } => 8,
        }
    }

    /// A check or an installation is running; new requests are ignored.
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Self::Checking
                | Self::Downloading { .. }
                | Self::Verifying(_)
                | Self::ReadyToInstall(_)
                | Self::Installing(_)
        )
    }

    /// The release the user could install now.
    pub fn installable(&self) -> Option<&Release> {
        match self {
            Self::Available(r) => Some(r),
            Self::Error { retry, .. } => retry.as_ref(),
            _ => None,
        }
    }

    pub fn release(&self) -> Option<&Release> {
        match self {
            Self::Available(r)
            | Self::Verifying(r)
            | Self::ReadyToInstall(r)
            | Self::Installing(r)
            | Self::Downloading { release: r, .. } => Some(r),
            Self::Error { retry, .. } => retry.as_ref(),
            _ => None,
        }
    }

    /// Download progress 0..=100, `None` while unknown or outside the download.
    pub fn percent(&self) -> Option<u8> {
        match self {
            Self::Downloading {
                done,
                total: Some(total),
                ..
            } if *total > 0 => Some((u128::from(*done) * 100 / u128::from(*total)).min(100) as u8),
            _ => None,
        }
    }

    pub fn error(&self) -> Option<&UpdateError> {
        match self {
            Self::Error { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// Applies `event`; events that make no sense in the current state are ignored.
pub fn transition(state: State, event: Event) -> (State, Effect) {
    use Event as E;
    use State as S;
    match (state, event) {
        (S::Idle | S::UpToDate | S::Available(_) | S::Error { .. }, E::CheckRequested) => {
            (S::Checking, Effect::StartCheck)
        }
        (S::Checking, E::CheckFinished(Ok(None))) => (S::UpToDate, Effect::None),
        (S::Checking, E::CheckFinished(Ok(Some(release)))) => (S::Available(release), Effect::None),
        (S::Checking, E::CheckFinished(Err(error))) => {
            (S::Error { error, retry: None }, Effect::None)
        }

        (S::Available(release), E::InstallRequested)
        | (
            S::Error {
                retry: Some(release),
                ..
            },
            E::InstallRequested,
        ) => (
            S::Downloading {
                release: release.clone(),
                done: 0,
                total: None,
            },
            Effect::StartDownload(release),
        ),

        (S::Downloading { release, .. }, E::Progress { done, total }) => (
            S::Downloading {
                release,
                done,
                total,
            },
            Effect::None,
        ),
        (S::Downloading { release, .. }, E::Downloaded) => (S::Verifying(release), Effect::None),
        (S::Verifying(release), E::Verified) => (
            S::ReadyToInstall(release.clone()),
            Effect::StartInstall(release),
        ),
        (S::ReadyToInstall(release), E::InstallStarted) => (S::Installing(release), Effect::None),
        (S::Installing(release), E::Installed) => (S::Installing(release), Effect::Restart),

        (
            S::Downloading { release, .. }
            | S::Verifying(release)
            | S::ReadyToInstall(release)
            | S::Installing(release),
            E::Failed(error),
        ) => (
            S::Error {
                error,
                retry: Some(release),
            },
            Effect::None,
        ),
        (state, _) => (state, Effect::None),
    }
}

/// Holds the state and applies events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    state: State,
}

impl Default for Machine {
    fn default() -> Self {
        Self { state: State::Idle }
    }
}

impl Machine {
    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn handle(&mut self, event: Event) -> Effect {
        let (state, effect) = transition(std::mem::replace(&mut self.state, State::Idle), event);
        self.state = state;
        effect
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::release::test_release as release;

    fn err(kind: ErrorKind) -> UpdateError {
        UpdateError::new(kind, "x")
    }

    #[test]
    fn full_happy_path_runs_only_after_the_click() {
        let mut m = Machine::default();
        assert_eq!(m.handle(Event::CheckRequested), Effect::StartCheck);
        assert_eq!(m.state(), &State::Checking);
        assert_eq!(
            m.handle(Event::CheckFinished(Ok(Some(release("v0.2.0"))))),
            Effect::None
        );
        // Available stays put until the user asks; no event but InstallRequested leaves it.
        for ignored in [
            Event::Downloaded,
            Event::Verified,
            Event::Installed,
            Event::InstallStarted,
        ] {
            assert_eq!(m.handle(ignored), Effect::None);
            assert_eq!(m.state().code(), 3);
        }
        assert_eq!(
            m.handle(Event::InstallRequested),
            Effect::StartDownload(release("v0.2.0"))
        );
        assert_eq!(m.state().code(), 4);
        m.handle(Event::Progress {
            done: 50,
            total: Some(200),
        });
        assert_eq!(m.state().percent(), Some(25));
        m.handle(Event::Downloaded);
        assert_eq!(m.state().code(), 5);
        assert_eq!(
            m.handle(Event::Verified),
            Effect::StartInstall(release("v0.2.0"))
        );
        assert_eq!(m.state().code(), 6);
        assert_eq!(m.handle(Event::InstallStarted), Effect::None);
        assert_eq!(m.state().code(), 7);
        assert_eq!(m.handle(Event::Installed), Effect::Restart);
    }

    #[test]
    fn check_results() {
        let mut m = Machine::default();
        m.handle(Event::CheckRequested);
        m.handle(Event::CheckFinished(Ok(None)));
        assert_eq!(m.state(), &State::UpToDate);
        assert_eq!(m.handle(Event::CheckRequested), Effect::StartCheck);
        m.handle(Event::CheckFinished(Err(err(ErrorKind::Network))));
        assert_eq!(m.state().error().unwrap().kind, ErrorKind::Network);
        assert!(m.state().installable().is_none());
        // A failed check can be repeated.
        assert_eq!(m.handle(Event::CheckRequested), Effect::StartCheck);
    }

    #[test]
    fn busy_states_ignore_new_requests() {
        let mut m = Machine::default();
        m.handle(Event::CheckRequested);
        assert!(m.state().is_busy());
        assert_eq!(m.handle(Event::CheckRequested), Effect::None);
        assert_eq!(m.handle(Event::InstallRequested), Effect::None);
        assert_eq!(m.state(), &State::Checking);
        m.handle(Event::CheckFinished(Ok(Some(release("v0.2.0")))));
        m.handle(Event::InstallRequested);
        assert!(m.state().is_busy());
        assert_eq!(m.handle(Event::CheckRequested), Effect::None);
        assert_eq!(m.handle(Event::InstallRequested), Effect::None);
        assert_eq!(m.state().code(), 4);
    }

    #[test]
    fn failures_keep_the_release_for_retry() {
        for fail_at in 0..3 {
            let mut m = Machine::default();
            m.handle(Event::CheckRequested);
            m.handle(Event::CheckFinished(Ok(Some(release("v0.2.0")))));
            m.handle(Event::InstallRequested);
            if fail_at >= 1 {
                m.handle(Event::Downloaded);
            }
            if fail_at >= 2 {
                m.handle(Event::Verified);
            }
            m.handle(Event::Failed(err(ErrorKind::HashMismatch)));
            assert_eq!(m.state().code(), 8, "fail_at {fail_at}");
            assert_eq!(m.state().installable(), Some(&release("v0.2.0")));
            assert_eq!(
                m.handle(Event::InstallRequested),
                Effect::StartDownload(release("v0.2.0"))
            );
        }
    }

    #[test]
    fn restart_failure_reports_error() {
        let mut m = Machine::default();
        m.handle(Event::CheckRequested);
        m.handle(Event::CheckFinished(Ok(Some(release("v0.2.0")))));
        m.handle(Event::InstallRequested);
        m.handle(Event::Downloaded);
        m.handle(Event::Verified);
        m.handle(Event::InstallStarted);
        m.handle(Event::Installed);
        m.handle(Event::Failed(err(ErrorKind::RestartFailed)));
        assert_eq!(m.state().error().unwrap().kind, ErrorKind::RestartFailed);
    }

    #[test]
    fn progress_is_bounded_and_only_while_downloading() {
        let mut m = Machine::default();
        m.handle(Event::Progress {
            done: 5,
            total: Some(10),
        });
        assert_eq!(m.state(), &State::Idle);
        m.handle(Event::CheckRequested);
        m.handle(Event::CheckFinished(Ok(Some(release("v0.2.0")))));
        m.handle(Event::InstallRequested);
        m.handle(Event::Progress {
            done: 500,
            total: Some(10),
        });
        assert_eq!(m.state().percent(), Some(100));
        m.handle(Event::Progress {
            done: 5,
            total: None,
        });
        assert_eq!(m.state().percent(), None);
        m.handle(Event::Progress {
            done: 5,
            total: Some(0),
        });
        assert_eq!(m.state().percent(), None);
    }

    #[test]
    fn error_codes_are_distinct() {
        use ErrorKind::*;
        let kinds = [
            Network,
            Rejected,
            TooLarge,
            HashMismatch,
            BadPackage,
            VersionMismatch,
            NotInstalled,
            NoWritePermission,
            InstallFailed,
            RestartFailed,
            Other,
        ];
        let mut codes: Vec<_> = kinds.iter().map(|k| k.code()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), kinds.len());
    }
}
