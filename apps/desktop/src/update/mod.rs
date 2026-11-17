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
