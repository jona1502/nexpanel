//! Tiny file logger (`log` facade) plus panic hook and panic-aware worker spawning.
//! Logging never fails the app: without a writable log file it silently does nothing.
use log::{LevelFilter, Log, Metadata, Record};
use std::{
    fs::{File, OpenOptions},
    io::{LineWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_LOG_BYTES: u64 = 1024 * 1024;
const LOG_NAME: &str = "nexpanel.log";
const OLD_LOG_NAME: &str = "nexpanel.old.log";

/// Set when a worker thread panicked (or could not start); read by the UI tick.
pub type CrashFlag = Arc<AtomicBool>;

struct FileLogger {
    level: LevelFilter,
    file: Mutex<Option<LineWriter<File>>>,
    stderr: bool,
}

static LOGGER: OnceLock<FileLogger> = OnceLock::new();

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // Dependencies (wmi, ...) only log Info and above unless Trace is requested.
        let own = metadata.target().starts_with("nexpanel");
        metadata.level() <= self.level
            && (own || metadata.level() <= log::Level::Info || self.level == LevelFilter::Trace)
    }
    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let thread = std::thread::current();
        let line = format_line(
            now_ms(),
            thread.name().unwrap_or("?"),
            record.level(),
            record.target(),
            &record.args().to_string(),
        );
        if self.stderr {
            eprintln!("{line}");
        }
        let mut guard = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(file) = guard.as_mut() {
            let _ = writeln!(file, "{line}");
        }
    }
    fn flush(&self) {
        let mut guard = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(file) = guard.as_mut() {
            let _ = file.flush();
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

/// UTC timestamp `YYYY-MM-DDTHH:MM:SS.mmmZ` from Unix milliseconds.
fn timestamp(ms: u64) -> String {
    let secs = ms / 1000;
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60,
        ms % 1000
    )
}

fn format_line(ms: u64, thread: &str, level: log::Level, target: &str, message: &str) -> String {
    format!(
        "{} [{thread}] {level:<5} {target}: {message}",
        timestamp(ms)
    )
}

/// Accepts `off|error|warn|info|debug|trace` (case-insensitive).
pub fn parse_level(value: &str) -> Option<LevelFilter> {
    value.trim().parse().ok()
}

fn default_level() -> LevelFilter {
    if cfg!(debug_assertions) {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    }
}

fn effective_level(env: Option<&str>) -> LevelFilter {
    env.and_then(parse_level).unwrap_or_else(default_level)
}

fn needs_rotation(len: u64) -> bool {
    len > MAX_LOG_BYTES
}

/// `%LOCALAPPDATA%\NexPanel\logs`, next to `settings.json`.
pub fn log_dir() -> Option<PathBuf> {
    crate::config::default_path()
        .ok()?
        .parent()
        .map(|dir| dir.join("logs"))
}

fn open_log(dir: &Path) -> std::io::Result<File> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(LOG_NAME);
    if std::fs::metadata(&path).is_ok_and(|m| needs_rotation(m.len())) {
        let old = dir.join(OLD_LOG_NAME);
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(&path, &old);
    }
    OpenOptions::new().create(true).append(true).open(path)
}

/// Install logger and panic hook. Later calls are ignored.
pub fn init() {
    let file = log_dir().and_then(|dir| open_log(&dir).ok());
    let env = std::env::var("NEXPANEL_LOG").ok();
    let logger = LOGGER.get_or_init(|| FileLogger {
        level: effective_level(env.as_deref()),
        file: Mutex::new(file.map(LineWriter::new)),
        stderr: cfg!(debug_assertions),
    });
    if log::set_logger(logger).is_err() {
        return;
    }
    log::set_max_level(logger.level);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(kein Text)".into());
        let location = info
            .location()
            .map_or_else(|| "?".into(), |l| format!("{}:{}", l.file(), l.line()));
        log::error!(
            "PANIC in Thread '{}' bei {location}: {message}",
            thread.name().unwrap_or("?")
        );
        let backtrace = std::backtrace::Backtrace::capture();
        if backtrace.status() == std::backtrace::BacktraceStatus::Captured {
            log::error!("Backtrace:\n{backtrace}");
        }
        log::logger().flush();
        previous(info);
    }));
}

static WORKERS: std::sync::Mutex<Vec<(String, std::thread::JoinHandle<()>)>> =
    std::sync::Mutex::new(Vec::new());

/// Join all workers started by `spawn_worker`, waiting at most `timeout` in
/// total. Workers still running afterwards are logged and left to the process exit.
pub fn join_workers(timeout: std::time::Duration) {
    let workers = std::mem::take(&mut *WORKERS.lock().unwrap_or_else(|e| e.into_inner()));
    let deadline = std::time::Instant::now() + timeout;
    for (name, handle) in workers {
        while !handle.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if handle.is_finished() {
            let _ = handle.join();
            log::info!("Worker '{name}' gejoint");
        } else {
            log::warn!("Worker '{name}' beendet sich nicht rechtzeitig, wird abgebrochen");
        }
    }
}

/// Spawn a named worker. A panic is caught (the hook has already logged it) and
/// sets `crashed`, so the UI can show an error. A spawn failure is logged and
/// also sets `crashed`.
pub fn spawn_worker(name: &str, crashed: &CrashFlag, run: impl FnOnce() + Send + 'static) {
    let thread_name = name.to_owned();
    let flag = crashed.clone();
    let result = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            log::info!("Worker '{thread_name}' gestartet");
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).is_err() {
                flag.store(true, Ordering::SeqCst);
                log::error!("Worker '{thread_name}' durch Panic beendet");
            } else {
                log::info!("Worker '{thread_name}' beendet");
            }
        });
    match result {
        Ok(handle) => WORKERS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((name.to_owned(), handle)),
        Err(error) => {
            crashed.store(true, Ordering::SeqCst);
            log::error!("Worker '{name}' konnte nicht gestartet werden: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_parsing() {
        assert_eq!(parse_level("debug"), Some(LevelFilter::Debug));
        assert_eq!(parse_level(" WARN "), Some(LevelFilter::Warn));
        assert_eq!(parse_level("off"), Some(LevelFilter::Off));
        assert_eq!(parse_level("verbose"), None);
        assert_eq!(effective_level(Some("trace")), LevelFilter::Trace);
        assert_eq!(effective_level(Some("quatsch")), default_level());
        assert_eq!(effective_level(None), default_level());
    }

    #[test]
    fn rotation_threshold() {
        assert!(!needs_rotation(0));
        assert!(!needs_rotation(MAX_LOG_BYTES));
        assert!(needs_rotation(MAX_LOG_BYTES + 1));
    }

    #[test]
    fn timestamp_formats_utc() {
        assert_eq!(timestamp(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(timestamp(1_709_164_800_123), "2024-02-29T00:00:00.123Z");
    }

    #[test]
    fn line_has_thread_level_target() {
        let line = format_line(0, "display", log::Level::Warn, "nexpanel::x", "hallo");
        assert_eq!(
            line,
            "1970-01-01T00:00:00.000Z [display] WARN  nexpanel::x: hallo"
        );
    }

    #[test]
    fn rotation_renames_large_log() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(LOG_NAME), vec![b'x'; 1024 * 1024 + 1]).unwrap();
        drop(open_log(dir.path()).unwrap());
        assert!(dir.path().join(OLD_LOG_NAME).exists());
        assert_eq!(
            std::fs::metadata(dir.path().join(LOG_NAME)).unwrap().len(),
            0
        );
    }

    #[test]
    fn join_workers_waits_for_finished_workers() {
        let flag = CrashFlag::default();
        spawn_worker("test-join", &flag, || {});
        join_workers(std::time::Duration::from_secs(2));
        assert!(!flag.load(Ordering::SeqCst));
    }
    #[test]
    fn panicking_worker_sets_flag() {
        let flag = CrashFlag::default();
        spawn_worker("test-panic", &flag, || panic!("boom"));
        for _ in 0..200 {
            if flag.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("flag not set");
    }
}
