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
//! * [`alert`], a native error dialog used for eframe startup failures;
//! * coarse [`Stage`] tracking with a startup watchdog: eframe keeps the
//!   window hidden until the first frame has been painted, so a hang
//!   anywhere between process start and the first present would otherwise
//!   leave a zombie process with no window at all. The watchdog turns that
//!   into a logged report plus a native dialog.
//!
//! Log level can be raised with the `MRPC_LOG_LEVEL` environment variable
//! (`error`..`trace`, default `info`), and the log destination can be pinned
//! with `MRPC_LOG_FILE`.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use log::LevelFilter;

static LOG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
static SINK: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();

/// Initialises the file logger and the panic hook. Call before anything else
/// in `main`.
pub fn init() {
    let _ = T0.set(Instant::now());
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

// ------------------------------------------------------------------ stages

/// Coarse startup stages, reported by the watchdog when startup stalls. The
/// names appear in the log (`startup stage: …`) and in the watchdog dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Stage {
    /// Logger installed; `main` is about to try to launch the app.
    LoggerReady = 1,
    /// `eframe::run_native` entered for a renderer plan.
    RunNative = 2,
    /// eframe built the window + graphics device and calls the app creator.
    AppCreator = 3,
    /// The creator returned; the event loop should now start.
    AppCreated = 4,
    /// The first `App::ui` pass began (building the UI).
    FirstUi = 5,
    /// The egui_wgpu paint pass began (first viewport `prepare`).
    FirstPaint = 6,
    /// A second UI pass ran, which proves the first frame was painted and
    /// presented — at that point eframe has made the window visible.
    FirstFrame = 7,
    /// Steady state.
    Running = 8,
}

impl Stage {
    fn from_u8(value: u8) -> Option<Stage> {
        match value {
            1 => Some(Stage::LoggerReady),
            2 => Some(Stage::RunNative),
            3 => Some(Stage::AppCreator),
            4 => Some(Stage::AppCreated),
            5 => Some(Stage::FirstUi),
            6 => Some(Stage::FirstPaint),
            7 => Some(Stage::FirstFrame),
            8 => Some(Stage::Running),
            _ => None,
        }
    }

    /// Human-readable explanation of where the app is.
    pub fn describe(self) -> &'static str {
        match self {
            Stage::LoggerReady => "process start (logger ready)",
            Stage::RunNative => "launching the window/event loop (eframe::run_native)",
            Stage::AppCreator => "creating the application (fonts, demo pack, audio thread)",
            Stage::AppCreated => "application created; event loop starting",
            Stage::FirstUi => "first UI pass (building the interface)",
            Stage::FirstPaint => "first GPU paint pass (pipelines, buffers, present)",
            Stage::FirstFrame => "first frame presented (window shown)",
            Stage::Running => "running",
        }
    }
}

static STAGE: AtomicU8 = AtomicU8::new(0);
static STARTUP_DONE: AtomicBool = AtomicBool::new(false);
static T0: OnceLock<Instant> = OnceLock::new();

/// Records that startup reached `stage`. Stages only move forward; each new
/// stage is logged with the time since logger init.
pub fn set_stage(stage: Stage) {
    let prev = STAGE.fetch_max(stage as u8, Ordering::SeqCst);
    if prev < stage as u8 {
        let elapsed_ms = elapsed_ms();
        log::info!("startup stage: {stage:?} (+{elapsed_ms} ms)");
    }
}

/// The highest stage reached so far, if any.
pub fn current_stage() -> Option<Stage> {
    Stage::from_u8(STAGE.load(Ordering::SeqCst))
}

/// Milliseconds since logger init (0 when the clock is somehow missing).
fn elapsed_ms() -> u128 {
    T0.get().map(|t| t.elapsed().as_millis()).unwrap_or(0)
}

/// Marks that the window is up and painting; stops the watchdog.
pub fn mark_startup_complete() {
    if !STARTUP_DONE.swap(true, Ordering::SeqCst) {
        log::info!("startup complete: window shown after {} ms", elapsed_ms());
    }
}

/// Whether the startup completed (the window is up and painting).
pub fn startup_complete() -> bool {
    STARTUP_DONE.load(Ordering::SeqCst)
}

/// Spawns the startup watchdog.
///
/// eframe deliberately keeps the window hidden until the first frame has
/// been painted (no white flash). Any indefinite hang between process start
/// and that first present therefore shows up as "a process exists, but no
/// window ever appears". Instead of leaving such a zombie behind, the
/// watchdog escalates: it first relaunches the app with the software renderer
/// (WARP / llvmpipe, via `MRPC_RENDERER=software`) — a hang in the hardware
/// driver then costs the user nothing but a late window — and only if a
/// relaunch is not allowed (already relaunched, or the renderer was pinned via
/// `MRPC_RENDERER`) or the respawn itself fails does it report the last
/// reached stage, show the failure dialog and exit with code 2.
pub fn spawn_startup_watchdog(timeout_secs: u64) {
    let spawned = std::thread::Builder::new()
        .name("startup-watchdog".to_string())
        .spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(timeout_secs);
            while Instant::now() < deadline {
                if startup_complete() {
                    return; // healthy startup
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            if startup_complete() {
                return;
            }
            if let Some(child_pid) = relaunch_with_software_renderer() {
                log::error!(
                    "startup watchdog: no first frame after {timeout_secs}s; \
                     last stage reached: {} — restarted with the software renderer \
                     (WARP / llvmpipe, child pid {child_pid})",
                    current_stage().map(Stage::describe).unwrap_or("unknown")
                );
                // The respawned child is on its way; end the hung process so
                // it does not linger as a windowless zombie.
                std::process::exit(2);
            }
            let stage = current_stage().map(Stage::describe).unwrap_or("unknown");
            log::error!(
                "startup watchdog: no first frame after {timeout_secs}s; \
                 last stage reached: {stage}"
            );
            let log_hint = match LOG_PATH.get().and_then(|p| p.as_deref()) {
                Some(p) => format!("A full log was written to:\n{}", p.display()),
                None => "No log file could be written.".to_string(),
            };
            alert(
                "MTR Resource Package Creator failed to start / \u{542f}\u{52a8}\u{5931}\u{8d25}",
                &format!(
                    "The window never appeared: startup stalled for {timeout_secs} seconds.\
                     \n\u{542f}\u{52a8}\u{505c}\u{6ede}\u{ff0c}\u{7a97}\u{53e3}\u{672a}\u{51fa}\u{73b0}\u{3002}\n\n\
                     Startup stalled at:\n{stage}\n\n\
                     {log_hint}\n\n\
                     Tips:\n\
                     \u{2022} Update your graphics driver and try again.\n\
                     \u{2022} Try setting the environment variable WGPU_BACKEND=gl.\n\
                     \u{2022} Try setting MRPC_VSYNC=1 to change the present mode.\n\
                     \u{2022} Try setting MRPC_NO_AUDIO=1 to rule out audio-driver issues.\n\n\
                     Please report this (with the log file) at:\n\
                     https://github.com/BugQuantumCore/Resource-Package-Creator/issues\n\n\
                     Click OK to close the app / \u{70b9}\u{51fb}\u{786e}\u{5b9a}\u{5173}\u{95ed}\u{7a0b}\u{5e8f}."
                ),
            );
            if startup_complete() {
                // The app recovered while the dialog was open; keep it alive.
                log::info!("startup watchdog: recovered while the dialog was open");
                return;
            }
            log::error!("startup watchdog: exiting (no first frame)");
            std::process::exit(2);
        });
    if let Err(err) = spawned {
        log::warn!("could not spawn the startup watchdog: {err}");
    }
}

/// Respawns this executable with the software renderer forced, for use when
/// the startup watchdog detects a hung first frame. Returns the child's PID
/// when the respawn actually happened; `None` when escalation is not allowed
/// (this is already the relaunched child, or the renderer was pinned manually
/// with `MRPC_RENDERER`) or the spawn itself failed.
///
/// The child gets `MRPC_WD_CHILD=1`, so ITS watchdog never relaunches again —
/// if the software renderer also hangs, the child shows the failure dialog
/// and exits instead of looping forever.
fn relaunch_with_software_renderer() -> Option<u32> {
    let already_child =
        std::env::var("MRPC_WD_CHILD").map(|v| !v.is_empty()).unwrap_or(false);
    let pinned_renderer =
        std::env::var("MRPC_RENDERER").map(|v| !v.is_empty()).unwrap_or(false);
    if already_child || pinned_renderer {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let child = std::process::Command::new(exe)
        .env("MRPC_RENDERER", "software")
        .env("MRPC_WD_CHILD", "1")
        .spawn()
        .map_err(|err| {
            log::error!("startup watchdog: could not relaunch the app: {err}");
            err
        })
        .ok()?;
    Some(child.id())
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
