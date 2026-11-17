//! Self-update from GitHub releases. Everything here is plain logic behind small traits so it
//! can be tested without network or Windows APIs; only `winhttp` and `install` touch the system.

pub mod engine;
pub mod install;
pub mod machine;
pub mod net;
pub mod release;
pub mod schedule;
pub mod sha256;
pub mod version;
#[cfg(windows)]
pub mod winhttp;
pub mod worker;
pub mod zip;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The WinHTTP transport; created on the worker thread.
pub fn system_transport() -> Result<Box<dyn engine::Transport>, machine::UpdateError> {
    #[cfg(windows)]
    {
        winhttp::WinHttpTransport::new()
            .map(|t| Box::new(t) as Box<dyn engine::Transport>)
            .map_err(Into::into)
    }
    #[cfg(not(windows))]
    {
        Err(machine::UpdateError::new(
            machine::ErrorKind::Other,
            "Updates benötigen Windows",
        ))
    }
}
