//! Startup diagnostics for the desktop app.
//!
//! Release builds use `#![windows_subsystem = "windows"]` on Windows, so
//! there is no console: panics and startup errors would otherwise vanish
//! without a trace ("a process exists, but no window ever appears"). This
//! module installs:
//!
//! * a file logger (`mtr-resource-package-creator.log` next to the
//!   executable, falling back to the user's state directory and then the
//!   temp directory) that also mirrors to stderr when one is attached;
//! * a panic hook that logs the message, location and backtrace and shows a
//!   native error dialog on Windows;
//! * [`alert`], a native error dialog used for eframe startup failures.
//!
//! Log level can be raised with the `MRPC_LOG_LEVEL` environment variable
//! (`error`..`trace`, default `info`), and the log destination can be pinned
//! with `MRPC_LOG_FILE`.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use log::LevelFilter;

static LOG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
static SINK: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();

/// Initialises the file logger and the panic hook. Call before anything else
/// in `main`.
pub fn init() {
    let level = level_from_env();
    let (path, file) = open_sink();
    let _ = LOG_PATH.set(path);
    let _ = SINK.set(Mutex::new(file));

    if let Err(err) = log::set_boxed_logger(Box::new(FileLogger { level })) {
        // Someone already installed a logger; degrade to stderr only.
        let _ = writeln!(std::io::stderr(), "[logging] set_boxed_logger failed: {err}");
    }
    log::set_max_level(level);

    log::info!("---- MTR Resource Package Creator v{} ----", env!("CARGO_PKG_VERSION"));
    log::info!(
        "platform: {} / {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    match LOG_PATH.get() {
        Some(Some(p)) => log::info!("log file: {}", p.display()),
        _ => log::warn!("no log file could be opened; logging to stderr only"),
    }
}

/// Installs a panic hook that logs (with backtrace) and shows a native dialog
/// so GUI-only builds never fail silently.
pub fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic payload".to_string()
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown location>".to_string());
        let backtrace = std::backtrace::Backtrace::force_capture();

        log::error!("PANIC: {message}\n  at {location}\n{backtrace}");
        let log_hint = match LOG_PATH.get() {
            Some(Some(p)) => format!("Details were written to:\n{}", p.display()),
            _ => "No log file is available; run from a terminal to see details.".to_string(),
        };
        alert(
            "MTR Resource Package Creator crashed",
            &format!(
                "{message}\n\nat {location}\n\n{log_hint}\n\n\
                 Please report this at:\n\
                 https://github.com/BugQuantumCore/Resource-Package-Creator/issues"
            ),
        );

        default_hook(info);
    }));
}

/// Path of the log file that is being written, if any.
pub fn log_path() -> Option<&'static std::path::Path> {
    LOG_PATH.get().and_then(|p| p.as_deref())
}

/// Logs the message and shows a native error dialog (Windows) or writes to
/// stderr (other platforms). Never panics.
pub fn alert(title: &str, text: &str) {
    log::error!("{title}\n{text}");
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONERROR, MB_OK,
        };
        let title_w: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let text_w: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        MessageBoxW(
            std::ptr::null_mut(),
            text_w.as_ptr(),
            title_w.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
    #[cfg(not(windows))]
    {
        eprintln!("{title}\n{text}");
    }
}

fn level_from_env() -> LevelFilter {
    match std::env::var("MRPC_LOG_LEVEL")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => LevelFilter::Info,
    }
}

/// Candidate log destinations, in priority order.
fn candidate_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(custom) = std::env::var("MRPC_LOG_FILE") {
        if !custom.is_empty() {
            paths.push(PathBuf::from(custom));
        }
    }

    // Portable-friendly: right next to the executable (e.g. an unzipped
    // download). Fails with EACCES when installed under Program Files, in
    // which case the user state directory is used instead.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join("mtr-resource-package-creator.log"));
        }
    }

    #[cfg(windows)]
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        paths.push(
            PathBuf::from(local)
                .join("mtr-resource-package-creator")
                .join("app.log"),
        );
    }

    #[cfg(not(windows))]
    {
        let state_dir = std::env::var("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                PathBuf::from(home).join(".local").join("state")
            });
        paths.push(
            state_dir
                .join("mtr-resource-package-creator")
                .join("app.log"),
        );
    }

    paths.push(std::env::temp_dir().join("mtr-resource-package-creator.log"));
    paths
}

/// Opens the first writable candidate. `None` if every candidate failed.
fn open_sink() -> (Option<PathBuf>, Option<std::fs::File>) {
    for path in candidate_paths() {
        if let Some(dir) = path.parent() {
            if std::fs::create_dir_all(dir).is_err() {
                continue;
            }
        }
        // Truncate a run-away log.
        let truncate = std::fs::metadata(&path)
            .map(|m| m.len() > 4 * 1024 * 1024)
            .unwrap_or(false);
        let file = OpenOptions::new()
            .create(true)
            .append(!truncate)
            .truncate(truncate)
            .open(&path);
        if let Ok(file) = file {
            return (Some(path), Some(file));
        }
    }
    (None, None)
}

struct FileLogger {
    level: LevelFilter,
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} {:>5} [{:<11}] {}\n",
            timestamp(),
            record.level().as_str(),
            record.target(),
            record.args()
        );
        write_line(&line);
    }

    fn flush(&self) {
        if let Some(sink) = SINK.get() {
            if let Ok(mut guard) = sink.lock() {
                if let Some(file) = guard.as_mut() {
                    let _ = file.flush();
                }
            }
        }
    }
}

fn write_line(line: &str) {
    if let Some(sink) = SINK.get() {
        if let Ok(mut guard) = sink.lock() {
            if let Some(file) = guard.as_mut() {
                let _ = file.write_all(line.as_bytes());
            }
        }
    }
    // Mirror to stderr; `let _` keeps this safe when no console exists
    // (Windows GUI subsystem) — `eprintln!` would panic on such a handle.
    let _ = writeln!(std::io::stderr(), "{line}");
}

/// `YYYY-MM-DD HH:MM:SS.mmm` in UTC from the epoch clock, without pulling in a
/// date-time crate.
fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let millis = now.subsec_millis();
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}.{millis:03}",
        secs_of_day / 3_600,
        (secs_of_day / 60) % 60,
        secs_of_day % 60
    )
}

/// Howard Hinnant's `civil_from_days` algorithm (days since 1970-01-01 → date).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}
