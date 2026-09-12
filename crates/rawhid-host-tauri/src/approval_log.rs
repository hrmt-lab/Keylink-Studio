//! Best-effort, privacy-preserving audit sink for physical approval actions.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::Mutex,
};

use chrono::Local;
use directories::ProjectDirs;

const PREFIX: &str = "keylink-studio-approval.";
const KEEP_FILES: usize = 7;

/// Metadata recorded for one physical HUD operation or its result. Callers
/// must pass only a stable session label and normalized protocol
/// metadata; request bodies, options, and answer values have no fields here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalLogEvent {
    pub session_label: String,
    pub client: String,
    pub request_kind: String,
    pub action: String,
    pub allowed: bool,
    pub outcome: String,
    pub reason: Option<String>,
}

impl ApprovalLogEvent {
    pub fn new(
        session_label: impl Into<String>,
        client: impl Into<String>,
        request_kind: impl Into<String>,
        action: impl Into<String>,
        allowed: bool,
        outcome: impl Into<String>,
        reason: Option<impl Into<String>>,
    ) -> Self {
        Self {
            session_label: session_label.into(),
            client: client.into(),
            request_kind: request_kind.into(),
            action: action.into(),
            allowed,
            outcome: outcome.into(),
            reason: reason.map(Into::into),
        }
    }
}

#[derive(Debug)]
pub struct ApprovalLog {
    directory: Option<PathBuf>,
    lock: Mutex<()>,
}

impl Default for ApprovalLog {
    fn default() -> Self {
        Self::new()
    }
}

impl ApprovalLog {
    pub fn new() -> Self {
        let directory =
            ProjectDirs::from("", "", "Keylink Studio").map(|dirs| dirs.config_dir().join("logs"));
        Self {
            directory,
            lock: Mutex::new(()),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_directory(directory: PathBuf) -> Self {
        Self {
            directory: Some(directory),
            lock: Mutex::new(()),
        }
    }

    /// Writes one metadata-only event. Errors are intentionally ignored so an
    /// unavailable AppData directory can never block a HUD response.
    pub fn record_event(&self, event: ApprovalLogEvent) {
        let Some(directory) = self.directory.as_ref() else {
            return;
        };
        let Ok(_guard) = self.lock.lock() else {
            return;
        };
        if fs::create_dir_all(directory).is_err() {
            return;
        }
        let date = Local::now().format("%Y-%m-%d").to_string();
        let path = directory.join(format!("{PREFIX}{date}.log"));
        let mut line = format!(
            "{} session={} client={} kind={} action={} allowed={} outcome={}",
            Local::now().format("%Y-%m-%dT%H:%M:%S%:z"),
            sanitize(&event.session_label),
            sanitize(&event.client),
            sanitize(&event.request_kind),
            sanitize(&event.action),
            event.allowed,
            sanitize(&event.outcome),
        );
        if let Some(reason) = event.reason.as_deref() {
            line.push_str(" reason=");
            line.push_str(&sanitize(reason));
        }
        line.push('\n');
        let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
            return;
        };
        if file.write_all(line.as_bytes()).is_err() {
            return;
        }
        prune_old_files(directory);
    }
}

fn prune_old_files(directory: &PathBuf) {
    let mut files: Vec<PathBuf> = fs::read_dir(directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(PREFIX) && name.ends_with(".log"))
        })
        .collect();
    files.sort();
    while files.len() > KEEP_FILES {
        if let Some(oldest) = files.first().cloned() {
            let _ = fs::remove_file(oldest);
            files.remove(0);
        }
    }
}

/// Keep each field on one line and bound opaque identifiers. The logger's
/// schema contains metadata only; this sanitizer also prevents accidental
/// line/key injection if a future caller supplies an unexpected token.
fn sanitize(value: &str) -> String {
    value
        .chars()
        .take(96)
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | ':' | '/' => ch,
            _ => '_',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn test_directory(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("keylink-approval-log-{label}-{nonce}"))
    }

    fn today_log(directory: &PathBuf) -> PathBuf {
        directory.join(format!("{PREFIX}{}.log", Local::now().format("%Y-%m-%d")))
    }

    #[test]
    fn log_is_metadata_only_and_sanitizes_newlines() {
        let directory = test_directory("privacy");
        let log = ApprovalLog::for_directory(directory.clone());
        log.record_event(ApprovalLogEvent::new(
            "codex:opaque\nrequest",
            "codex",
            "input",
            "Confirm",
            true,
            "sent",
            Some("turn_resolved\n"),
        ));
        let text = fs::read_to_string(today_log(&directory)).unwrap();
        assert!(text.contains("session=codex:opaque_request"));
        assert!(text.contains("reason=turn_resolved_"));
        assert_eq!(text.lines().count(), 1);
        assert!(!text.contains("primary_text"));
        assert!(!text.contains("options"));
        assert!(!text.contains("answer_value"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn log_keeps_seven_daily_generations() {
        let directory = test_directory("retention");
        fs::create_dir_all(&directory).unwrap();
        for day in 1..=8 {
            fs::write(
                directory.join(format!("{PREFIX}2020-01-{day:02}.log")),
                "old\n",
            )
            .unwrap();
        }
        let log = ApprovalLog::for_directory(directory.clone());
        log.record_event(ApprovalLogEvent::new(
            "opaque",
            "codex",
            "approval",
            "Select",
            true,
            "allowed",
            None::<String>,
        ));
        let count = fs::read_dir(&directory)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(PREFIX))
            .count();
        assert_eq!(count, KEEP_FILES);
        assert!(!directory.join(format!("{PREFIX}2020-01-01.log")).exists());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn log_records_each_physical_outcome() {
        let directory = test_directory("outcomes");
        let log = ApprovalLog::for_directory(directory.clone());
        for (allowed, outcome, reason) in [
            (true, "allowed", None),
            (true, "sent", None),
            (false, "rejected", Some("user_rejected")),
            (false, "failed", Some("transport_failed")),
            (false, "withdrawn", Some("hid_device_lost")),
        ] {
            log.record_event(ApprovalLogEvent::new(
                "opaque", "codex", "approval", "Confirm", allowed, outcome, reason,
            ));
        }
        let text = fs::read_to_string(today_log(&directory)).unwrap();
        for outcome in ["allowed", "sent", "rejected", "failed", "withdrawn"] {
            assert!(text.contains(&format!("outcome={outcome}")));
        }
        assert!(text.contains("reason=user_rejected"));
        assert!(text.contains("reason=transport_failed"));
        assert!(text.contains("reason=hid_device_lost"));
        let _ = fs::remove_dir_all(directory);
    }
}
