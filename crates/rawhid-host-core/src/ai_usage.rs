use std::{
    collections::HashSet,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::{
    config::{AiUsageConfig, ClaudeCodeAiUsageConfig, CodexAiUsageConfig},
    packet::{AiUsageErrorCode, AiUsageFlags, AiUsagePacket, AiUsageProvider, PacketError},
};

const FIVE_HOUR_MINUTES: u64 = 300;
const SEVEN_DAY_MINUTES: u64 = 10_080;
const FIVE_HOUR_SECONDS: u64 = FIVE_HOUR_MINUTES * 60;
const SEVEN_DAY_SECONDS: u64 = SEVEN_DAY_MINUTES * 60;
const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Cap how much of each session JSONL file is read per poll. Codex session logs
/// can grow large; the entries we care about (latest rate_limits, recent token
/// usage) live near the end, so we read at most the trailing window of bytes.
const MAX_SESSION_READ_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AiUsageStatusKind {
    Disabled,
    Ok,
    Stale,
    NoData,
    MissingCredentials,
    ExpiredCredentials,
    AuthFailed,
    RateLimited,
    FetchFailed,
    ParseFailed,
    MissingLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AiUsageSourceKind {
    None,
    Quota,
    LocalHistory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AiUsageCredentialSourceKind {
    ExplicitPath,
    WindowsDefault,
    Wsl,
    ExtraPath,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AiUsageProviderStatus {
    pub provider: String,
    pub status: AiUsageStatusKind,
    pub source: AiUsageSourceKind,
    pub updated_unix: Option<u64>,
    pub stale: bool,
    pub last_error_code: Option<u8>,
    pub five_hour_used_bp: Option<u16>,
    pub seven_day_used_bp: Option<u16>,
    pub five_hour_reset_unix: Option<u32>,
    pub seven_day_reset_unix: Option<u32>,
    pub five_hour_valid: bool,
    pub seven_day_valid: bool,
    pub estimated: bool,
    pub quota_source: bool,
    pub local_history_source: bool,
    pub fallback_limit: bool,
    pub error_present: bool,
    pub credential_source: Option<AiUsageCredentialSourceKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiUsageWindow {
    pub used_bp: u16,
    pub reset_unix: u32,
}

#[derive(Debug, Clone)]
pub struct AiUsageSnapshot {
    pub provider: AiUsageProvider,
    pub five_hour: Option<AiUsageWindow>,
    pub seven_day: Option<AiUsageWindow>,
    pub updated_unix: u64,
    pub estimated: bool,
    pub local_history_source: bool,
    pub quota_source: bool,
    pub fallback_limit: bool,
    pub force_stale: bool,
    pub error_code: AiUsageErrorCode,
    pub credential_source: Option<AiUsageCredentialSourceKind>,
}

impl AiUsageSnapshot {
    fn error(provider: AiUsageProvider, now: u64, error_code: AiUsageErrorCode) -> Self {
        Self {
            provider,
            five_hour: None,
            seven_day: None,
            updated_unix: now,
            estimated: false,
            local_history_source: false,
            quota_source: false,
            fallback_limit: false,
            force_stale: false,
            error_code,
            credential_source: None,
        }
    }

    fn is_stale(&self, now: u64, stale_after_sec: u64) -> bool {
        self.force_stale
            || self
                .updated_unix
                .checked_add(stale_after_sec)
                .is_some_and(|deadline| now >= deadline)
    }

    pub fn to_packet(&self, now: u64, stale_after_sec: u64) -> Result<AiUsagePacket, PacketError> {
        let stale = self.is_stale(now, stale_after_sec);
        let error_present = self.error_code != AiUsageErrorCode::None;
        let flags = AiUsageFlags::default()
            .with(AiUsageFlags::FIVE_HOUR_VALID, self.five_hour.is_some())
            .with(AiUsageFlags::SEVEN_DAY_VALID, self.seven_day.is_some())
            .with(AiUsageFlags::ESTIMATED, self.estimated)
            .with(
                AiUsageFlags::LOCAL_HISTORY_SOURCE,
                self.local_history_source,
            )
            .with(AiUsageFlags::QUOTA_SOURCE, self.quota_source)
            .with(AiUsageFlags::STALE, stale)
            .with(AiUsageFlags::FALLBACK_LIMIT, self.fallback_limit)
            .with(AiUsageFlags::ERROR_PRESENT, error_present);

        AiUsagePacket::new(
            self.provider,
            flags,
            self.five_hour.as_ref().map_or(0, |w| w.used_bp),
            self.seven_day.as_ref().map_or(0, |w| w.used_bp),
            self.five_hour.as_ref().map_or(0, |w| w.reset_unix),
            self.seven_day.as_ref().map_or(0, |w| w.reset_unix),
            u32::try_from(self.updated_unix).unwrap_or(u32::MAX),
            self.error_code,
        )
    }

    fn status(&self, now: u64, stale_after_sec: u64) -> AiUsageProviderStatus {
        let stale = self.is_stale(now, stale_after_sec);
        let status = if stale {
            AiUsageStatusKind::Stale
        } else {
            status_from_error(self.error_code)
        };
        let source = if self.quota_source {
            AiUsageSourceKind::Quota
        } else if self.local_history_source {
            AiUsageSourceKind::LocalHistory
        } else {
            AiUsageSourceKind::None
        };
        AiUsageProviderStatus {
            provider: provider_name(self.provider).to_string(),
            status,
            source,
            updated_unix: Some(self.updated_unix),
            stale,
            last_error_code: (self.error_code != AiUsageErrorCode::None)
                .then_some(self.error_code as u8),
            five_hour_used_bp: self.five_hour.as_ref().map(|w| w.used_bp),
            seven_day_used_bp: self.seven_day.as_ref().map(|w| w.used_bp),
            five_hour_reset_unix: self.five_hour.as_ref().map(|w| w.reset_unix),
            seven_day_reset_unix: self.seven_day.as_ref().map(|w| w.reset_unix),
            five_hour_valid: self.five_hour.is_some(),
            seven_day_valid: self.seven_day.is_some(),
            estimated: self.estimated,
            quota_source: self.quota_source,
            local_history_source: self.local_history_source,
            fallback_limit: self.fallback_limit,
            error_present: self.error_code != AiUsageErrorCode::None,
            credential_source: self.credential_source,
        }
    }
}

#[derive(Debug, Default)]
pub struct AiUsageSharedState {
    generation: u64,
    snapshots: Vec<AiUsageSnapshot>,
}

#[derive(Debug, Clone)]
pub struct AiUsageShared {
    inner: Arc<Mutex<AiUsageSharedState>>,
}

impl AiUsageShared {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(AiUsageSharedState::default())),
        }
    }

    fn update(&self, snapshots: Vec<AiUsageSnapshot>) {
        let mut state = self.inner.lock().unwrap();
        state.generation = state.generation.wrapping_add(1);
        state.snapshots = snapshots;
    }

    pub fn generation(&self) -> u64 {
        self.inner.lock().unwrap().generation
    }

    pub fn snapshots(&self) -> Vec<AiUsageSnapshot> {
        self.inner.lock().unwrap().snapshots.clone()
    }

    pub fn statuses(&self, stale_after_sec: u64) -> Vec<AiUsageProviderStatus> {
        let now = unix_now();
        self.snapshots()
            .iter()
            .map(|snapshot| snapshot.status(now, stale_after_sec))
            .collect()
    }
}

#[derive(Debug)]
pub struct AiUsageRuntime {
    shared: AiUsageShared,
    command_tx: Option<mpsc::Sender<AiUsageWorkerCommand>>,
    refresh_pending: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

#[derive(Debug)]
enum AiUsageWorkerCommand {
    Stop,
    Refresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiUsageRefreshError {
    InProgress,
    Stopped,
}

impl AiUsageRuntime {
    pub fn start(config: AiUsageConfig) -> Option<Self> {
        if !config.enabled {
            return None;
        }
        let shared = AiUsageShared::new();
        let thread_shared = shared.clone();
        let (command_tx, command_rx) = mpsc::channel();
        let refresh_pending = Arc::new(AtomicBool::new(false));
        let thread_refresh_pending = Arc::clone(&refresh_pending);
        let join = thread::spawn(move || {
            run_worker(config, thread_shared, command_rx, thread_refresh_pending);
        });
        Some(Self {
            shared,
            command_tx: Some(command_tx),
            refresh_pending,
            join: Some(join),
        })
    }

    pub fn shared(&self) -> AiUsageShared {
        self.shared.clone()
    }

    pub fn statuses(&self, stale_after_sec: u64) -> Vec<AiUsageProviderStatus> {
        self.shared.statuses(stale_after_sec)
    }

    pub fn refresh(&self) -> Result<(), AiUsageRefreshError> {
        let Some(tx) = &self.command_tx else {
            return Err(AiUsageRefreshError::Stopped);
        };
        if self
            .refresh_pending
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(AiUsageRefreshError::InProgress);
        }
        if tx.send(AiUsageWorkerCommand::Refresh).is_err() {
            self.refresh_pending.store(false, Ordering::SeqCst);
            return Err(AiUsageRefreshError::Stopped);
        }
        Ok(())
    }
}

impl Drop for AiUsageRuntime {
    fn drop(&mut self) {
        if let Some(tx) = self.command_tx.take() {
            let _ = tx.send(AiUsageWorkerCommand::Stop);
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct AiUsageSendState {
    last_generation: Option<u64>,
    last_device_generation: Option<u64>,
    last_stale: Vec<(AiUsageProvider, bool)>,
}

impl AiUsageSendState {
    pub fn due_packets(
        &mut self,
        shared: &AiUsageShared,
        stale_after_sec: u64,
        device_generation: u64,
    ) -> Vec<AiUsagePacket> {
        let generation = shared.generation();
        let snapshots = shared.snapshots();
        let now = unix_now();
        let stale_state: Vec<_> = snapshots
            .iter()
            .map(|snapshot| (snapshot.provider, snapshot.is_stale(now, stale_after_sec)))
            .collect();
        let should_send = self.last_generation != Some(generation)
            || self.last_device_generation != Some(device_generation)
            || self.last_stale != stale_state;
        if !should_send {
            return Vec::new();
        }
        self.last_generation = Some(generation);
        self.last_device_generation = Some(device_generation);
        self.last_stale = stale_state;

        snapshots
            .iter()
            .filter_map(|snapshot| snapshot.to_packet(now, stale_after_sec).ok())
            .collect()
    }
}

fn run_worker(
    config: AiUsageConfig,
    shared: AiUsageShared,
    command_rx: mpsc::Receiver<AiUsageWorkerCommand>,
    refresh_pending: Arc<AtomicBool>,
) {
    let interval = Duration::from_secs(config.poll_interval_sec.max(1));
    let mut previous: Vec<AiUsageSnapshot> = Vec::new();
    loop {
        collect_and_update(&config, &shared, &mut previous);
        match command_rx.recv_timeout(interval) {
            Ok(AiUsageWorkerCommand::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(AiUsageWorkerCommand::Refresh) => {
                collect_and_update(&config, &shared, &mut previous);
                if drain_refresh_commands(&command_rx) {
                    refresh_pending.store(false, Ordering::SeqCst);
                    break;
                }
                refresh_pending.store(false, Ordering::SeqCst);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn collect_and_update(
    config: &AiUsageConfig,
    shared: &AiUsageShared,
    previous: &mut Vec<AiUsageSnapshot>,
) {
    let snapshots = collect_snapshots(config, previous);
    *previous = snapshots.clone();
    shared.update(snapshots);
}

fn drain_refresh_commands(command_rx: &mpsc::Receiver<AiUsageWorkerCommand>) -> bool {
    loop {
        match command_rx.try_recv() {
            Ok(AiUsageWorkerCommand::Stop) | Err(mpsc::TryRecvError::Disconnected) => return true,
            Ok(AiUsageWorkerCommand::Refresh) => {}
            Err(mpsc::TryRecvError::Empty) => return false,
        }
    }
}

fn collect_snapshots(config: &AiUsageConfig, previous: &[AiUsageSnapshot]) -> Vec<AiUsageSnapshot> {
    let mut snapshots = Vec::new();
    if config.codex.enabled {
        snapshots.push(collect_codex(&config.codex));
    } else {
        snapshots.push(AiUsageSnapshot::error(
            AiUsageProvider::Codex,
            unix_now(),
            AiUsageErrorCode::SourceDisabled,
        ));
    }
    if config.claude_code.enabled {
        snapshots.push(match collect_claude(&config.claude_code) {
            Ok(snapshot) => snapshot,
            Err(error) => stale_or_error(previous, AiUsageProvider::ClaudeCode, error),
        });
    } else {
        snapshots.push(AiUsageSnapshot::error(
            AiUsageProvider::ClaudeCode,
            unix_now(),
            AiUsageErrorCode::SourceDisabled,
        ));
    }
    snapshots
}

fn stale_or_error(
    previous: &[AiUsageSnapshot],
    provider: AiUsageProvider,
    error_code: AiUsageErrorCode,
) -> AiUsageSnapshot {
    if let Some(snapshot) = previous.iter().find(|snapshot| {
        snapshot.provider == provider
            && (snapshot.five_hour.is_some() || snapshot.seven_day.is_some())
    }) {
        let mut snapshot = snapshot.clone();
        snapshot.force_stale = true;
        snapshot.error_code = error_code;
        return snapshot;
    }
    AiUsageSnapshot::error(provider, unix_now(), error_code)
}

fn collect_codex(config: &CodexAiUsageConfig) -> AiUsageSnapshot {
    let sessions_dirs = codex_sessions_dirs(config);
    if let Some(snapshot) = codex_rate_limits_snapshot(&sessions_dirs) {
        return snapshot;
    }
    if config.history_fallback_enabled {
        return codex_history_snapshot(&sessions_dirs, config);
    }
    AiUsageSnapshot::error(
        AiUsageProvider::Codex,
        unix_now(),
        AiUsageErrorCode::NoUsageData,
    )
}

/// Resolve the set of Codex session directories to read. An explicit
/// `sessions_dir` is always included; when auto-detection is on we also add the
/// Windows default, every WSL distro's `~/.codex/sessions`, and any extra paths.
fn codex_sessions_dirs(config: &CodexAiUsageConfig) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();
    if let Some(dir) = &config.sessions_dir {
        push_unique_path(&mut dirs, &mut seen, PathBuf::from(dir));
    }
    if config.sessions_auto_detect {
        push_unique_path(&mut dirs, &mut seen, default_codex_sessions_dir());
        if config.include_wsl_sessions {
            for path in wsl_codex_sessions_paths() {
                push_unique_path(&mut dirs, &mut seen, path);
            }
        }
        for path in &config.extra_sessions_paths {
            push_unique_path(&mut dirs, &mut seen, PathBuf::from(path));
        }
    }
    dirs
}

fn push_unique_path(dirs: &mut Vec<PathBuf>, seen: &mut HashSet<String>, path: PathBuf) {
    let key = path.to_string_lossy().to_ascii_lowercase();
    if seen.insert(key) {
        dirs.push(path);
    }
}

fn codex_rate_limits_snapshot(sessions_dirs: &[PathBuf]) -> Option<AiUsageSnapshot> {
    for path in sorted_jsonl_files(sessions_dirs) {
        let Some(text) = read_session_text(&path) else {
            continue;
        };
        for line in text.lines().rev() {
            if !line.contains("rate_limits") {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let Some(rate_limits) = value.pointer("/payload/rate_limits") else {
                continue;
            };
            let primary = rate_limit_window(rate_limits.get("primary"));
            let secondary = rate_limit_window(rate_limits.get("secondary"));
            let window_for = |minutes: u64| -> Option<AiUsageWindow> {
                [primary.as_ref(), secondary.as_ref()]
                    .into_iter()
                    .flatten()
                    .find(|(window_minutes, _)| *window_minutes == minutes)
                    .map(|(_, window)| window.clone())
            };
            let five_hour = window_for(FIVE_HOUR_MINUTES);
            let seven_day = window_for(SEVEN_DAY_MINUTES);
            if five_hour.is_some() || seven_day.is_some() {
                return Some(AiUsageSnapshot {
                    provider: AiUsageProvider::Codex,
                    five_hour,
                    seven_day,
                    updated_unix: unix_now(),
                    estimated: false,
                    local_history_source: false,
                    quota_source: true,
                    fallback_limit: false,
                    force_stale: false,
                    error_code: AiUsageErrorCode::None,
                    credential_source: None,
                });
            }
        }
    }
    None
}

fn rate_limit_window(value: Option<&Value>) -> Option<(u64, AiUsageWindow)> {
    let value = value?;
    let minutes = value.get("window_minutes")?.as_u64()?;
    let percent = value.get("used_percent")?.as_f64()?;
    let reset_unix = value.get("resets_at")?.as_u64()?;
    Some((
        minutes,
        AiUsageWindow {
            used_bp: percent_to_basis_points(percent)?,
            reset_unix: u32::try_from(reset_unix).ok()?,
        },
    ))
}

fn codex_history_snapshot(sessions_dirs: &[PathBuf], config: &CodexAiUsageConfig) -> AiUsageSnapshot {
    let now = unix_now();
    let mut five_tokens = 0u64;
    let mut seven_tokens = 0u64;
    let mut saw_usage = false;
    for event in codex_token_events(sessions_dirs) {
        saw_usage = true;
        if now.saturating_sub(event.timestamp_unix) <= FIVE_HOUR_SECONDS {
            five_tokens = five_tokens.saturating_add(event.tokens);
        }
        if now.saturating_sub(event.timestamp_unix) <= SEVEN_DAY_SECONDS {
            seven_tokens = seven_tokens.saturating_add(event.tokens);
        }
    }
    if !saw_usage {
        return AiUsageSnapshot::error(AiUsageProvider::Codex, now, AiUsageErrorCode::NoUsageData);
    }
    if !config.allow_activity_baseline {
        return AiUsageSnapshot::error(AiUsageProvider::Codex, now, AiUsageErrorCode::MissingLimit);
    }

    let five_hour = (config.activity_five_hour_token_baseline > 0)
        .then(|| usage_window_from_tokens(five_tokens, config.activity_five_hour_token_baseline));
    let seven_day = (config.activity_seven_day_token_baseline > 0)
        .then(|| usage_window_from_tokens(seven_tokens, config.activity_seven_day_token_baseline));
    if five_hour.is_none() && seven_day.is_none() {
        return AiUsageSnapshot::error(AiUsageProvider::Codex, now, AiUsageErrorCode::MissingLimit);
    }

    AiUsageSnapshot {
        provider: AiUsageProvider::Codex,
        five_hour,
        seven_day,
        updated_unix: now,
        estimated: true,
        local_history_source: true,
        quota_source: false,
        fallback_limit: true,
        force_stale: false,
        error_code: AiUsageErrorCode::None,
        credential_source: None,
    }
}

fn usage_window_from_tokens(tokens: u64, baseline: u64) -> AiUsageWindow {
    let percent = if baseline == 0 {
        0.0
    } else {
        tokens as f64 / baseline as f64 * 100.0
    };
    AiUsageWindow {
        used_bp: percent_to_basis_points(percent).unwrap_or(10_000),
        reset_unix: 0,
    }
}

#[derive(Debug)]
struct TokenEvent {
    timestamp_unix: u64,
    tokens: u64,
}

fn codex_token_events(sessions_dirs: &[PathBuf]) -> Vec<TokenEvent> {
    let mut events = Vec::new();
    for path in sorted_jsonl_files(sessions_dirs).into_iter().rev() {
        let Some(text) = read_session_text(&path) else {
            continue;
        };
        let mut previous_total: Option<u64> = None;
        let mut seen = HashSet::new();
        for line in text.lines() {
            if !line.contains("token_count") {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if value.pointer("/payload/type").and_then(Value::as_str) != Some("token_count") {
                continue;
            }
            let timestamp_unix = value
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_rfc3339_unix)
                .unwrap_or_else(unix_now);
            let last = value
                .pointer("/payload/info/last_token_usage")
                .and_then(token_usage_total);
            let total = value
                .pointer("/payload/info/total_token_usage")
                .and_then(token_usage_total);
            let key = format!("{timestamp_unix}:{last:?}:{total:?}");
            if !seen.insert(key) {
                continue;
            }
            let tokens = if let Some(last) = last {
                if let Some(total) = total {
                    previous_total = Some(total);
                }
                last
            } else if let Some(total) = total {
                let delta = previous_total
                    .map(|previous| total.saturating_sub(previous))
                    .unwrap_or(total);
                previous_total = Some(total);
                delta
            } else {
                0
            };
            if tokens > 0 {
                events.push(TokenEvent {
                    timestamp_unix,
                    tokens,
                });
            }
        }
    }
    events
}

fn token_usage_total(value: &Value) -> Option<u64> {
    value
        .get("total_tokens")
        .and_then(Value::as_u64)
        .or_else(|| {
            let fields = [
                "input_tokens",
                "cached_input_tokens",
                "output_tokens",
                "reasoning_output_tokens",
            ];
            let mut total = 0u64;
            let mut found = false;
            for field in fields {
                if let Some(value) = value.get(field).and_then(Value::as_u64) {
                    found = true;
                    total = total.saturating_add(value);
                }
            }
            found.then_some(total)
        })
}

fn collect_claude(config: &ClaudeCodeAiUsageConfig) -> Result<AiUsageSnapshot, AiUsageErrorCode> {
    collect_claude_with_url(config, CLAUDE_USAGE_URL)
}

fn collect_claude_with_url(
    config: &ClaudeCodeAiUsageConfig,
    usage_url: &str,
) -> Result<AiUsageSnapshot, AiUsageErrorCode> {
    let candidates = claude_credentials_candidates(config);
    let credentials = select_claude_credentials(candidates, unix_now().saturating_mul(1000))?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(config.api_timeout_sec.max(1)))
        .build()
        .map_err(|_| AiUsageErrorCode::FetchFailed)?;

    let mut auth_failed = false;
    for credential in credentials {
        match fetch_claude_usage(&client, usage_url, &credential.access_token) {
            Ok(value) => {
                let five_hour = claude_window(value.get("five_hour"));
                let seven_day = claude_window(value.get("seven_day"));
                if five_hour.is_none() && seven_day.is_none() {
                    return Err(AiUsageErrorCode::NoUsageData);
                }
                return Ok(AiUsageSnapshot {
                    provider: AiUsageProvider::ClaudeCode,
                    five_hour,
                    seven_day,
                    updated_unix: unix_now(),
                    estimated: false,
                    local_history_source: false,
                    quota_source: true,
                    fallback_limit: false,
                    force_stale: false,
                    error_code: AiUsageErrorCode::None,
                    credential_source: Some(credential.source),
                });
            }
            Err(ClaudeUsageFetchError::AuthFailed) => {
                auth_failed = true;
            }
            Err(ClaudeUsageFetchError::RateLimited) => return Err(AiUsageErrorCode::RateLimited),
            Err(ClaudeUsageFetchError::Other(error)) => return Err(error),
        }
    }

    if auth_failed {
        Err(AiUsageErrorCode::AuthFailed)
    } else {
        Err(AiUsageErrorCode::MissingCredentials)
    }
}

#[derive(Debug, Clone)]
struct ClaudeCredentialsCandidate {
    path: PathBuf,
    source: AiUsageCredentialSourceKind,
    explicit: bool,
    order: usize,
}

#[derive(Debug, Clone)]
struct ClaudeCredentials {
    access_token: String,
    expires_at: Option<u64>,
    mtime: SystemTime,
    source: AiUsageCredentialSourceKind,
    explicit: bool,
    order: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaudeCandidateFailure {
    Missing,
    ParseFailed,
    MissingToken,
    Expired,
}

#[derive(Debug)]
enum ClaudeUsageFetchError {
    AuthFailed,
    RateLimited,
    Other(AiUsageErrorCode),
}

fn claude_credentials_candidates(
    config: &ClaudeCodeAiUsageConfig,
) -> Vec<ClaudeCredentialsCandidate> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    if let Some(path) = &config.credentials_path {
        push_claude_candidate(
            &mut candidates,
            &mut seen,
            PathBuf::from(path),
            AiUsageCredentialSourceKind::ExplicitPath,
            true,
        );
    }
    if config.credentials_auto_detect {
        push_claude_candidate(
            &mut candidates,
            &mut seen,
            default_claude_credentials_path(),
            AiUsageCredentialSourceKind::WindowsDefault,
            false,
        );
        if config.include_wsl_credentials {
            for path in wsl_claude_credentials_paths() {
                push_claude_candidate(
                    &mut candidates,
                    &mut seen,
                    path,
                    AiUsageCredentialSourceKind::Wsl,
                    false,
                );
            }
        }
        for path in &config.extra_credentials_paths {
            push_claude_candidate(
                &mut candidates,
                &mut seen,
                PathBuf::from(path),
                AiUsageCredentialSourceKind::ExtraPath,
                false,
            );
        }
    }
    candidates
}

fn push_claude_candidate(
    candidates: &mut Vec<ClaudeCredentialsCandidate>,
    seen: &mut HashSet<String>,
    path: PathBuf,
    source: AiUsageCredentialSourceKind,
    explicit: bool,
) {
    let key = path.to_string_lossy().to_ascii_lowercase();
    if seen.insert(key) {
        let order = candidates.len();
        candidates.push(ClaudeCredentialsCandidate {
            path,
            source,
            explicit,
            order,
        });
    }
}

fn wsl_claude_credentials_paths() -> Vec<PathBuf> {
    wsl_home_dirs()
        .into_iter()
        .map(|home| home.join(".claude").join(".credentials.json"))
        .collect()
}

fn wsl_codex_sessions_paths() -> Vec<PathBuf> {
    wsl_home_dirs()
        .into_iter()
        .map(|home| home.join(".codex").join("sessions"))
        .collect()
}

/// Enumerate per-user home directories (`\\wsl$\<distro>\home\<user>`) across
/// every detectable WSL distro, deduplicating distro roots.
fn wsl_home_dirs() -> Vec<PathBuf> {
    let mut homes = Vec::new();
    let mut distro_paths = Vec::new();
    for root in [PathBuf::from(r"\\wsl$"), PathBuf::from(r"\\wsl.localhost")] {
        let Ok(distros) = fs::read_dir(&root) else {
            continue;
        };
        for distro in distros.flatten() {
            distro_paths.push(distro.path());
        }
    }
    for distro in wsl_distro_names_from_command() {
        distro_paths.push(PathBuf::from(format!(r"\\wsl$\{distro}")));
        distro_paths.push(PathBuf::from(format!(r"\\wsl.localhost\{distro}")));
    }

    let mut seen = HashSet::new();
    for distro_path in distro_paths {
        let key = distro_path.to_string_lossy().to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        let home = distro_path.join("home");
        let Ok(users) = fs::read_dir(home) else {
            continue;
        };
        for user in users.flatten() {
            let Ok(meta) = user.metadata() else {
                continue;
            };
            if meta.is_dir() {
                homes.push(user.path());
            }
        }
    }
    homes
}

fn wsl_distro_names_from_command() -> Vec<String> {
    let Ok(output) = Command::new("wsl.exe").args(["-l", "-q"]).output() else {
        return Vec::new();
    };
    parse_wsl_distro_names(&output.stdout)
}

fn parse_wsl_distro_names(bytes: &[u8]) -> Vec<String> {
    let looks_utf16_le = bytes.starts_with(&[0xff, 0xfe])
        || bytes
            .iter()
            .skip(1)
            .step_by(2)
            .filter(|byte| **byte == 0)
            .count()
            > bytes.len() / 4;
    let text = if looks_utf16_le {
        let chunks = bytes
            .strip_prefix(&[0xff, 0xfe])
            .unwrap_or(bytes)
            .chunks_exact(2);
        let utf16: Vec<u16> = chunks
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16_lossy(&utf16)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.lines()
        .map(|line| line.trim_matches(|ch: char| ch == '\0' || ch.is_whitespace()))
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn select_claude_credentials(
    candidates: Vec<ClaudeCredentialsCandidate>,
    now_ms: u64,
) -> Result<Vec<ClaudeCredentials>, AiUsageErrorCode> {
    if candidates.is_empty() {
        return Err(AiUsageErrorCode::MissingCredentials);
    }

    let mut credentials = Vec::new();
    let mut failures = Vec::new();
    for candidate in candidates {
        match read_claude_credentials(candidate, now_ms) {
            Ok(credential) => credentials.push(credential),
            Err(failure) => failures.push(failure),
        }
    }

    if !credentials.is_empty() {
        credentials.sort_by(|a, b| {
            b.explicit
                .cmp(&a.explicit)
                .then_with(|| b.expires_at.unwrap_or(0).cmp(&a.expires_at.unwrap_or(0)))
                .then_with(|| b.mtime.cmp(&a.mtime))
                .then_with(|| a.order.cmp(&b.order))
        });
        return Ok(credentials);
    }

    if failures.contains(&ClaudeCandidateFailure::Expired) {
        return Err(AiUsageErrorCode::ExpiredCredentials);
    }
    if failures.contains(&ClaudeCandidateFailure::ParseFailed) {
        return Err(AiUsageErrorCode::ParseFailed);
    }
    Err(AiUsageErrorCode::MissingCredentials)
}

fn read_claude_credentials(
    candidate: ClaudeCredentialsCandidate,
    now_ms: u64,
) -> Result<ClaudeCredentials, ClaudeCandidateFailure> {
    let text = fs::read_to_string(&candidate.path).map_err(|_| ClaudeCandidateFailure::Missing)?;
    let value: Value =
        serde_json::from_str(&text).map_err(|_| ClaudeCandidateFailure::ParseFailed)?;
    let oauth = value
        .get("claudeAiOauth")
        .ok_or(ClaudeCandidateFailure::MissingToken)?;
    let access_token = oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or(ClaudeCandidateFailure::MissingToken)?
        .to_string();
    let expires_at = oauth.get("expiresAt").and_then(Value::as_u64);
    if expires_at.is_some_and(|expires_at| expires_at <= now_ms) {
        return Err(ClaudeCandidateFailure::Expired);
    }
    let mtime = file_mtime(&candidate.path);
    Ok(ClaudeCredentials {
        access_token,
        expires_at,
        mtime,
        source: candidate.source,
        explicit: candidate.explicit,
        order: candidate.order,
    })
}

fn fetch_claude_usage(
    client: &reqwest::blocking::Client,
    usage_url: &str,
    access_token: &str,
) -> Result<Value, ClaudeUsageFetchError> {
    let response = client
        .get(usage_url)
        .bearer_auth(access_token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .map_err(|_| ClaudeUsageFetchError::Other(AiUsageErrorCode::FetchFailed))?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(ClaudeUsageFetchError::AuthFailed);
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(ClaudeUsageFetchError::RateLimited);
    }
    if !status.is_success() {
        return Err(ClaudeUsageFetchError::Other(AiUsageErrorCode::FetchFailed));
    }
    response
        .json()
        .map_err(|_| ClaudeUsageFetchError::Other(AiUsageErrorCode::ParseFailed))
}
fn claude_window(value: Option<&Value>) -> Option<AiUsageWindow> {
    let value = value?;
    let percent = value
        .get("utilization")
        .or_else(|| value.get("used_percentage"))?
        .as_f64()?;
    let reset_unix = parse_reset_unix(value.get("resets_at")?)?;
    Some(AiUsageWindow {
        used_bp: percent_to_basis_points(percent)?,
        reset_unix,
    })
}

fn parse_reset_unix(value: &Value) -> Option<u32> {
    if let Some(seconds) = value.as_u64() {
        return u32::try_from(seconds).ok();
    }
    let text = value.as_str()?;
    if let Ok(seconds) = text.parse::<u64>() {
        return u32::try_from(seconds).ok();
    }
    u32::try_from(parse_rfc3339_unix(text)?).ok()
}

/// Read a session JSONL file, capping the read at `MAX_SESSION_READ_BYTES`.
/// For oversized files only the trailing window is read and the first
/// (likely partial) line is dropped so JSON parsing stays valid.
fn read_session_text(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len <= MAX_SESSION_READ_BYTES {
        return fs::read_to_string(path).ok();
    }
    file.seek(SeekFrom::Start(len - MAX_SESSION_READ_BYTES)).ok()?;
    let mut buf = Vec::with_capacity(MAX_SESSION_READ_BYTES as usize);
    file.take(MAX_SESSION_READ_BYTES).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    Some(match text.find('\n') {
        Some(idx) => text[idx + 1..].to_string(),
        None => text,
    })
}

fn sorted_jsonl_files(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for root in roots {
        collect_jsonl_files(root, &mut files);
    }
    files.sort_by(|a, b| {
        let a_mtime = file_mtime(a);
        let b_mtime = file_mtime(b);
        b_mtime
            .cmp(&a_mtime)
            .then_with(|| a.to_string_lossy().cmp(&b.to_string_lossy()))
    });
    files
}

fn collect_jsonl_files(path: &Path, files: &mut Vec<PathBuf>) {
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    if meta.is_file() {
        if path.extension().is_some_and(|ext| ext == "jsonl") {
            files.push(path.to_path_buf());
        }
        return;
    }
    if !meta.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        collect_jsonl_files(&entry.path(), files);
    }
}

fn file_mtime(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .unwrap_or(UNIX_EPOCH)
}

fn parse_rfc3339_unix(text: &str) -> Option<u64> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|dt| dt.with_timezone(&Utc).timestamp())
        .and_then(|timestamp| u64::try_from(timestamp).ok())
}

fn percent_to_basis_points(percent: f64) -> Option<u16> {
    if !(0.0..=100.0).contains(&percent) {
        return None;
    }
    Some((percent * 100.0).round() as u16)
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn default_codex_sessions_dir() -> PathBuf {
    home_dir().join(".codex").join("sessions")
}

fn default_claude_credentials_path() -> PathBuf {
    home_dir().join(".claude").join(".credentials.json")
}

fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn status_from_error(error_code: AiUsageErrorCode) -> AiUsageStatusKind {
    match error_code {
        AiUsageErrorCode::None => AiUsageStatusKind::Ok,
        AiUsageErrorCode::SourceDisabled => AiUsageStatusKind::Disabled,
        AiUsageErrorCode::MissingCredentials => AiUsageStatusKind::MissingCredentials,
        AiUsageErrorCode::ExpiredCredentials => AiUsageStatusKind::ExpiredCredentials,
        AiUsageErrorCode::AuthFailed => AiUsageStatusKind::AuthFailed,
        AiUsageErrorCode::RateLimited => AiUsageStatusKind::RateLimited,
        AiUsageErrorCode::FetchFailed => AiUsageStatusKind::FetchFailed,
        AiUsageErrorCode::ParseFailed => AiUsageStatusKind::ParseFailed,
        AiUsageErrorCode::NoUsageData => AiUsageStatusKind::NoData,
        AiUsageErrorCode::MissingLimit => AiUsageStatusKind::MissingLimit,
    }
}

fn provider_name(provider: AiUsageProvider) -> &'static str {
    match provider {
        AiUsageProvider::Codex => "codex",
        AiUsageProvider::ClaudeCode => "claude_code",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_jsonl(dir: &Path, name: &str, lines: &[&str]) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        let mut file = fs::File::create(&path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
        path
    }

    #[test]
    fn codex_rate_limits_use_quota_source() {
        let dir = tempfile::tempdir().unwrap();
        write_jsonl(
            dir.path(),
            "a.jsonl",
            &[
                r#"{"type":"event_msg","timestamp":"2026-01-01T00:00:00Z","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":12.5,"window_minutes":300,"resets_at":1800000000},"secondary":{"used_percent":50.0,"window_minutes":10080,"resets_at":1800100000}}}}"#,
            ],
        );

        let snapshot = codex_rate_limits_snapshot(&[dir.path().to_path_buf()]).unwrap();

        assert!(snapshot.quota_source);
        assert!(!snapshot.estimated);
        assert_eq!(snapshot.five_hour.unwrap().used_bp, 1250);
        assert_eq!(snapshot.seven_day.unwrap().reset_unix, 1_800_100_000);
    }

    #[test]
    fn codex_rate_limits_merge_picks_most_recent_across_dirs() {
        let older = tempfile::tempdir().unwrap();
        let newer = tempfile::tempdir().unwrap();
        let older_file = write_jsonl(
            older.path(),
            "a.jsonl",
            &[
                r#"{"type":"event_msg","timestamp":"2026-01-01T00:00:00Z","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":10.0,"window_minutes":300,"resets_at":1800000000}}}}"#,
            ],
        );
        let newer_file = write_jsonl(
            newer.path(),
            "b.jsonl",
            &[
                r#"{"type":"event_msg","timestamp":"2026-01-02T00:00:00Z","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":42.0,"window_minutes":300,"resets_at":1800500000}}}}"#,
            ],
        );
        // Pin mtimes so the second directory's file sorts as most recent regardless
        // of filesystem timestamp resolution.
        let base = SystemTime::now();
        fs::File::options()
            .write(true)
            .open(&older_file)
            .unwrap()
            .set_modified(base)
            .unwrap();
        fs::File::options()
            .write(true)
            .open(&newer_file)
            .unwrap()
            .set_modified(base + Duration::from_secs(10))
            .unwrap();

        let snapshot = codex_rate_limits_snapshot(&[
            older.path().to_path_buf(),
            newer.path().to_path_buf(),
        ])
        .unwrap();

        assert_eq!(snapshot.five_hour.unwrap().used_bp, 4200);
    }

    #[test]
    fn codex_token_events_merge_aggregates_across_dirs() {
        let win = tempfile::tempdir().unwrap();
        let wsl = tempfile::tempdir().unwrap();
        write_jsonl(
            win.path(),
            "a.jsonl",
            &[
                r#"{"timestamp":"2026-01-01T00:00:00Z","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":30}}}}"#,
            ],
        );
        write_jsonl(
            wsl.path(),
            "b.jsonl",
            &[
                r#"{"timestamp":"2026-01-01T00:00:01Z","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":12}}}}"#,
            ],
        );

        let events = codex_token_events(&[win.path().to_path_buf(), wsl.path().to_path_buf()]);

        assert_eq!(events.iter().map(|e| e.tokens).sum::<u64>(), 42);
    }

    #[test]
    fn codex_history_uses_positive_total_deltas_once() {
        let dir = tempfile::tempdir().unwrap();
        write_jsonl(
            dir.path(),
            "a.jsonl",
            &[
                r#"{"timestamp":"2026-01-01T00:00:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":100}}}}"#,
                r#"{"timestamp":"2026-01-01T00:00:01Z","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":100}}}}"#,
                r#"{"timestamp":"2026-01-01T00:00:02Z","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":160}}}}"#,
            ],
        );

        let events = codex_token_events(&[dir.path().to_path_buf()]);

        assert_eq!(
            events.iter().map(|e| e.tokens).collect::<Vec<_>>(),
            vec![100, 60]
        );
    }

    #[test]
    fn codex_history_skips_duplicate_token_count_lines() {
        let dir = tempfile::tempdir().unwrap();
        let line = r#"{"timestamp":"2026-01-01T00:00:00Z","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":42}}}}"#;
        write_jsonl(dir.path(), "a.jsonl", &[line, line]);

        let events = codex_token_events(&[dir.path().to_path_buf()]);

        assert_eq!(
            events.iter().map(|e| e.tokens).collect::<Vec<_>>(),
            vec![42]
        );
    }

    #[test]
    fn history_fallback_resets_are_zero_and_not_quota_source() {
        let dir = tempfile::tempdir().unwrap();
        let now = unix_now();
        write_jsonl(
            dir.path(),
            "a.jsonl",
            &[&format!(
                r#"{{"timestamp":"{}","payload":{{"type":"token_count","info":{{"last_token_usage":{{"total_tokens":50}}}}}}}}"#,
                DateTime::<Utc>::from(UNIX_EPOCH + Duration::from_secs(now)).to_rfc3339()
            )],
        );
        let config = CodexAiUsageConfig {
            allow_activity_baseline: true,
            activity_five_hour_token_baseline: 100,
            activity_seven_day_token_baseline: 100,
            ..CodexAiUsageConfig::default()
        };

        let snapshot = codex_history_snapshot(&[dir.path().to_path_buf()], &config);

        assert!(snapshot.estimated);
        assert!(snapshot.local_history_source);
        assert!(!snapshot.quota_source);
        assert_eq!(snapshot.five_hour.unwrap().reset_unix, 0);
    }
    fn write_claude_credentials(path: &Path, token: &str, expires_at: u64) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(
            path,
            format!(r#"{{"claudeAiOauth":{{"accessToken":"{token}","expiresAt":{expires_at}}}}}"#),
        )
        .unwrap();
    }

    fn claude_candidate(
        path: PathBuf,
        source: AiUsageCredentialSourceKind,
        explicit: bool,
        order: usize,
    ) -> ClaudeCredentialsCandidate {
        ClaudeCredentialsCandidate {
            path,
            source,
            explicit,
            order,
        }
    }

    #[test]
    fn claude_credentials_explicit_path_only_when_auto_detect_disabled() {
        let config = ClaudeCodeAiUsageConfig {
            credentials_path: Some("C:\\Users\\me\\.claude\\.credentials.json".to_string()),
            credentials_auto_detect: false,
            ..ClaudeCodeAiUsageConfig::default()
        };

        let candidates = claude_credentials_candidates(&config);

        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].explicit);
        assert_eq!(
            candidates[0].source,
            AiUsageCredentialSourceKind::ExplicitPath
        );
    }

    #[test]
    fn claude_credentials_missing_when_no_path_and_auto_detect_disabled() {
        let config = ClaudeCodeAiUsageConfig {
            credentials_auto_detect: false,
            ..ClaudeCodeAiUsageConfig::default()
        };

        let error = select_claude_credentials(claude_credentials_candidates(&config), unix_now())
            .unwrap_err();

        assert_eq!(error, AiUsageErrorCode::MissingCredentials);
    }

    #[test]
    fn claude_credentials_valid_candidate_wins_over_parse_failed_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.json");
        let good = dir.path().join("good.json");
        fs::write(&bad, "not json").unwrap();
        write_claude_credentials(&good, "good-token", 4_000_000_000_000);

        let credentials = select_claude_credentials(
            vec![
                claude_candidate(bad, AiUsageCredentialSourceKind::WindowsDefault, false, 0),
                claude_candidate(good, AiUsageCredentialSourceKind::Wsl, false, 1),
            ],
            3_000_000_000_000,
        )
        .unwrap();

        assert_eq!(credentials.len(), 1);
        assert_eq!(credentials[0].access_token, "good-token");
        assert_eq!(credentials[0].source, AiUsageCredentialSourceKind::Wsl);
    }

    #[test]
    fn claude_credentials_prefers_explicit_valid_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let explicit = dir.path().join("explicit.json");
        let wsl = dir.path().join("wsl.json");
        write_claude_credentials(&explicit, "explicit-token", 3_100_000_000_000);
        write_claude_credentials(&wsl, "wsl-token", 4_000_000_000_000);

        let credentials = select_claude_credentials(
            vec![
                claude_candidate(explicit, AiUsageCredentialSourceKind::ExplicitPath, true, 0),
                claude_candidate(wsl, AiUsageCredentialSourceKind::Wsl, false, 1),
            ],
            3_000_000_000_000,
        )
        .unwrap();

        assert_eq!(credentials[0].access_token, "explicit-token");
        assert_eq!(
            credentials[0].source,
            AiUsageCredentialSourceKind::ExplicitPath
        );
    }

    #[test]
    fn claude_credentials_prefers_furthest_expiry_for_auto_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let windows = dir.path().join("windows.json");
        let wsl = dir.path().join("wsl.json");
        write_claude_credentials(&windows, "windows-token", 3_500_000_000_000);
        write_claude_credentials(&wsl, "wsl-token", 4_000_000_000_000);

        let credentials = select_claude_credentials(
            vec![
                claude_candidate(
                    windows,
                    AiUsageCredentialSourceKind::WindowsDefault,
                    false,
                    0,
                ),
                claude_candidate(wsl, AiUsageCredentialSourceKind::Wsl, false, 1),
            ],
            3_000_000_000_000,
        )
        .unwrap();

        assert_eq!(credentials[0].access_token, "wsl-token");
    }

    #[test]
    fn claude_credentials_all_expired_returns_expired_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let expired = dir.path().join("expired.json");
        write_claude_credentials(&expired, "expired-token", 2_000_000_000_000);

        let error = select_claude_credentials(
            vec![claude_candidate(
                expired,
                AiUsageCredentialSourceKind::WindowsDefault,
                false,
                0,
            )],
            3_000_000_000_000,
        )
        .unwrap_err();

        assert_eq!(error, AiUsageErrorCode::ExpiredCredentials);
    }

    #[test]
    fn claude_credentials_parse_failed_only_returns_parse_failed() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.json");
        fs::write(&bad, "not json").unwrap();

        let error = select_claude_credentials(
            vec![claude_candidate(
                bad,
                AiUsageCredentialSourceKind::WindowsDefault,
                false,
                0,
            )],
            3_000_000_000_000,
        )
        .unwrap_err();

        assert_eq!(error, AiUsageErrorCode::ParseFailed);
    }

    #[test]
    fn parse_wsl_distro_names_handles_utf16le_output() {
        let bytes: Vec<u8> = "Ubuntu
"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();

        assert_eq!(parse_wsl_distro_names(&bytes), vec!["Ubuntu"]);
    }

    #[test]
    fn parse_wsl_distro_names_handles_utf8_output() {
        assert_eq!(
            parse_wsl_distro_names(
                b"Ubuntu
Debian
"
            ),
            vec!["Ubuntu", "Debian"]
        );
    }
}
