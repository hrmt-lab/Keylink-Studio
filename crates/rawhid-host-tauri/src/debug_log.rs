//! Debug-only file log sink (`[debug_log]` in `keylink-studio.toml`).
//!
//! This is a separate sink from the in-memory UI log (`state::add_log` /
//! `log_entries`); it does not change that mechanism's behavior. It exists to
//! give diagnostic `tracing::` call sites somewhere to go when the user opts
//! in, since without a registered `tracing` subscriber they are silently
//! dropped:
//! - the AI display slot send/clear WARNs in `commands.rs`
//! - the AI wire-send / slot-assignment diagnostics in `commands.rs`, which
//!   are logged at DEBUG under [`AI_DISPLAY_LOG_TARGET`] specifically so they
//!   no longer clutter the in-memory UI log (see that module for the call
//!   sites; their message content and the `AiWireSend` dedup logic feeding
//!   them are unchanged by this move).
//!
//! # Why this is structured around a shared, toggleable writer
//!
//! A `tracing` subscriber can only be installed once per process
//! (`tracing_subscriber::fmt().init()` panics on a second call). The Settings
//! toggle, however, can flip on/off any number of times while the app is
//! running. So instead of re-initializing the subscriber on every toggle,
//! [`init`] installs it exactly once at startup — regardless of whether
//! logging starts enabled — and hands back a [`DebugLogHandle`] whose
//! `enabled` flag and open [`RollingFileAppender`] are shared (via `Arc`)
//! with the writer the subscriber holds. Toggling later
//! ([`DebugLogHandle::set_enabled`]) just flips that shared state; the
//! subscriber itself is never touched again.
//!
//! # Filtering
//!
//! There is no user-facing "log level" concept — the Settings page exposes a
//! single on/off toggle. Underneath, the subscriber's base filter is WARN,
//! with one exception: [`AI_DISPLAY_LOG_TARGET`] is allowed through at DEBUG.
//! That target is used only by the AI wire-send / slot-assignment
//! diagnostics above, so turning the toggle on makes those visible too, on
//! top of the WARNs every other target already produces.
//!
//! # Rotation
//!
//! One file per calendar day, keeping the most recent 7
//! (`tracing_appender::rolling` with `Rotation::DAILY` and
//! `max_log_files(7)`). Because rotation is file-count based, not
//! calendar-age based, "7" means the 7 newest files that exist on disk, not
//! "no file older than 7 days" — e.g. if the app isn't run for two weeks, the
//! next run's rotation check still only trims down to the newest 7 of
//! whatever is there (it does not proactively delete files just for being
//! old; deletion happens at the moment a new file is created). Output always
//! goes to a `logs` subfolder next to the running `.exe` — no destination
//! picker. The exact file name includes the date
//! (`keylink-studio-debug.<date>.log`).

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{filter::EnvFilter, fmt::MakeWriter};

const LOG_FILENAME_PREFIX: &str = "keylink-studio-debug";
const LOG_FILENAME_SUFFIX: &str = "log";
/// File-count kept by rotation, not a day count (see module doc).
const MAX_LOG_FILES: usize = 7;

/// Dedicated `tracing` target for the AI wire-send / slot-assignment
/// diagnostics in `commands.rs`. The subscriber installed by [`init`] filters
/// everything else at WARN but allows DEBUG through for this target only.
pub const AI_DISPLAY_LOG_TARGET: &str = "keylink_studio::ai_display";

/// Shared handle to the debug log file sink. Cheap to clone (all state is
/// behind `Arc`); every clone controls the same underlying writer that the
/// process-wide `tracing` subscriber was installed with.
#[derive(Clone)]
pub struct DebugLogHandle {
    enabled: Arc<AtomicBool>,
    appender: Arc<Mutex<Option<RollingFileAppender>>>,
}

impl DebugLogHandle {
    fn new() -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            appender: Arc::new(Mutex::new(None)),
        }
    }

    fn writer(&self) -> DebugLogWriter {
        DebugLogWriter {
            enabled: self.enabled.clone(),
            appender: self.appender.clone(),
        }
    }

    /// Enable or disable the file sink.
    ///
    /// Enabling builds a daily-rotating appender rooted in the `logs`
    /// subfolder next to the running `.exe`. If the directory can't be used
    /// (e.g. not writable), this returns `Err` and leaves the sink disabled (no
    /// partially-enabled state) — the caller is expected to surface the error
    /// to the UI and keep `enabled` at `false`.
    ///
    /// Disabling drops the appender, closing its file handle so no lock is
    /// left behind.
    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        self.set_enabled_with(enabled, log_dir_path)
    }

    /// Returns whether the sink is currently enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Same as [`Self::set_enabled`], but with the log directory resolved by
    /// `resolve_dir` instead of always [`log_dir_path`]. Split out so tests
    /// can point at a temp directory instead of the real executable's
    /// directory.
    fn set_enabled_with(
        &self,
        enabled: bool,
        resolve_dir: impl FnOnce() -> io::Result<PathBuf>,
    ) -> Result<(), String> {
        if enabled {
            let dir = resolve_dir().map_err(|e| e.to_string())?;
            let appender = build_appender(&dir)?;
            *self.appender.lock().unwrap() = Some(appender);
            self.enabled.store(true, Ordering::SeqCst);
        } else {
            // Flip the flag first so any write already past the atomic check
            // is the last one to reach the mutex, then drop the appender.
            self.enabled.store(false, Ordering::SeqCst);
            *self.appender.lock().unwrap() = None;
        }
        Ok(())
    }
}

fn build_appender(dir: &Path) -> Result<RollingFileAppender, String> {
    fs::create_dir_all(dir).map_err(|e| {
        format!(
            "failed to create debug log directory {}: {e}",
            dir.display()
        )
    })?;
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILENAME_PREFIX)
        .filename_suffix(LOG_FILENAME_SUFFIX)
        .max_log_files(MAX_LOG_FILES)
        .build(dir)
        .map_err(|e| format!("failed to open debug log directory {}: {e}", dir.display()))
}

/// The `tracing_subscriber` writer backing [`DebugLogHandle`]. Discards all
/// writes while disabled (rather than building an appender lazily per line),
/// and writes to the shared appender while enabled.
#[derive(Clone)]
struct DebugLogWriter {
    enabled: Arc<AtomicBool>,
    appender: Arc<Mutex<Option<RollingFileAppender>>>,
}

impl Write for DebugLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !self.enabled.load(Ordering::Relaxed) {
            return Ok(buf.len());
        }
        let mut guard = self.appender.lock().unwrap();
        if let Some(appender) = guard.as_mut() {
            appender.write_all(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut guard = self.appender.lock().unwrap();
        if let Some(appender) = guard.as_mut() {
            return appender.flush();
        }
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for DebugLogWriter {
    type Writer = DebugLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Directory the rotating log files are written into: a `logs` subfolder
/// next to the running executable. `build_appender` creates it (via
/// `fs::create_dir_all`) if it doesn't exist yet.
fn log_dir_path() -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    exe.parent().map(|dir| dir.join("logs")).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "executable has no parent directory",
        )
    })
}

/// Installs the process-wide `tracing` subscriber exactly once (see the
/// module doc for why this must not be called again on every Settings
/// toggle), then applies `initial_enabled` — from `debug_log.enabled` in the
/// loaded config — to the returned handle.
///
/// docs/spec.md: "output destination cannot be opened -> show the error in
/// the UI and disable file logging" applies at startup too, not just when
/// the Settings toggle is flipped later. This function itself has no UI to
/// report to, so it returns the failure (if any) as `Some(message)` instead
/// of swallowing it; the caller (`lib.rs::run`) is responsible for surfacing
/// that message to the UI log and for keeping the in-memory config's
/// `debug_log.enabled` in sync with the handle's actual (disabled) state.
pub fn init(initial_enabled: bool) -> (DebugLogHandle, Option<String>) {
    let handle = DebugLogHandle::new();

    // Base level is WARN; `AI_DISPLAY_LOG_TARGET` alone is allowed through at
    // DEBUG. This is a fixed policy, not read from `RUST_LOG` -- there is no
    // user-facing level to override.
    let filter = EnvFilter::new(format!("warn,{AI_DISPLAY_LOG_TARGET}=debug"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(handle.writer())
        .with_ansi(false)
        .init();

    let error = if initial_enabled {
        handle.set_enabled(true).err()
    } else {
        None
    };

    (handle, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::File, io::Read};

    /// Finds the rotated log file `build_appender` created under `dir`. The
    /// exact name includes the rotation date
    /// (`keylink-studio-debug.<date>.log`), so tests match on the prefix
    /// rather than hardcoding tracing-appender's date format.
    fn find_log_file(dir: &Path) -> Option<PathBuf> {
        fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok())
            .find_map(|e| {
                let path = e.path();
                let starts_with_prefix = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(LOG_FILENAME_PREFIX));
                starts_with_prefix.then_some(path)
            })
    }

    fn read_log_file(dir: &Path) -> String {
        let path = find_log_file(dir).expect("rotating log file should exist");
        let mut contents = String::new();
        File::open(path)
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        contents
    }

    #[test]
    fn disabled_writer_discards_writes() {
        let handle = DebugLogHandle::new();
        let dir = tempfile::tempdir().unwrap();

        let mut writer = handle.writer();
        writer.write_all(b"dropped line\n").unwrap();

        assert!(
            find_log_file(dir.path()).is_none(),
            "disabled sink must not create a file"
        );
    }

    #[test]
    fn enabling_opens_the_file_and_writes_flow_through() {
        let handle = DebugLogHandle::new();
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_path_buf();

        handle
            .set_enabled_with(true, || Ok(dir_path.clone()))
            .unwrap();
        assert!(handle.is_enabled());

        let mut writer = handle.writer();
        writer.write_all(b"warn: something happened\n").unwrap();
        writer.flush().unwrap();

        assert_eq!(read_log_file(&dir_path), "warn: something happened\n");
    }

    #[test]
    fn disabling_stops_writes_and_releases_the_handle() {
        let handle = DebugLogHandle::new();
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_path_buf();

        handle
            .set_enabled_with(true, || Ok(dir_path.clone()))
            .unwrap();
        let mut writer = handle.writer();
        writer.write_all(b"first\n").unwrap();

        handle
            .set_enabled_with(false, || Ok(dir_path.clone()))
            .unwrap();
        assert!(!handle.is_enabled());
        writer.write_all(b"second (should be dropped)\n").unwrap();

        // The appender (and its file handle) must have been dropped, not
        // just stopped being written to, otherwise it would still hold a
        // lock/lingering handle on Windows. Deleting it here is the
        // observable proof.
        let path = find_log_file(&dir_path).expect("file should exist after the first write");
        std::fs::remove_file(&path).expect("file handle must be released after disabling");

        // Re-enabling reopens (append mode) without error.
        handle
            .set_enabled_with(true, || Ok(dir_path.clone()))
            .unwrap();
        let mut writer = handle.writer();
        writer.write_all(b"third\n").unwrap();

        assert_eq!(read_log_file(&dir_path), "third\n");
    }

    #[test]
    fn failing_to_open_the_directory_leaves_the_sink_disabled() {
        let handle = DebugLogHandle::new();
        // Treat a plain file as though it were a directory component:
        // creating "<file>/nested" as a directory fails deterministically on
        // every platform, unlike guessing at a missing drive letter or
        // permission error.
        let dir = tempfile::tempdir().unwrap();
        let not_a_dir = dir.path().join("not-a-directory");
        File::create(&not_a_dir).unwrap();
        let bogus = not_a_dir.join("nested");

        let result = handle.set_enabled_with(true, || Ok(bogus));
        assert!(result.is_err());
        assert!(!handle.is_enabled());
    }
}
