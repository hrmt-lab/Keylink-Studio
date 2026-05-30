use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
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
    let sessions_dir = config
        .sessions_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(default_codex_sessions_dir);
    if let Some(snapshot) = codex_rate_limits_snapshot(&sessions_dir) {
        return snapshot;
    }
    if config.history_fallback_enabled {
        return codex_history_snapshot(&sessions_dir, config);
    }
    AiUsageSnapshot::error(
        AiUsageProvider::Codex,
        unix_now(),
        AiUsageErrorCode::NoUsageData,
    )
}

fn codex_rate_limits_snapshot(sessions_dir: &Path) -> Option<AiUsageSnapshot> {
    for path in sorted_jsonl_files(sessions_dir) {
        let Ok(text) = fs::read_to_string(&path) else {
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
            let five_hour = rate_limit_window(rate_limits.get("primary"))
                .or_else(|| rate_limit_window(rate_limits.get("secondary")))
                .filter(|window| window.0 == FIVE_HOUR_MINUTES)
                .map(|(_, window)| window);
            let seven_day = rate_limit_window(rate_limits.get("primary"))
                .or_else(|| rate_limit_window(rate_limits.get("secondary")))
                .filter(|window| window.0 == SEVEN_DAY_MINUTES)
                .map(|(_, window)| window);
            let primary = rate_limit_window(rate_limits.get("primary"));
            let secondary = rate_limit_window(rate_limits.get("secondary"));
            let five_hour = five_hour.or_else(|| {
                [primary.clone(), secondary.clone()]
                    .into_iter()
                    .flatten()
                    .find(|(minutes, _)| *minutes == FIVE_HOUR_MINUTES)
                    .map(|(_, window)| window)
            });
            let seven_day = seven_day.or_else(|| {
                [primary, secondary]
                    .into_iter()
                    .flatten()
                    .find(|(minutes, _)| *minutes == SEVEN_DAY_MINUTES)
                    .map(|(_, window)| window)
            });
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

fn codex_history_snapshot(sessions_dir: &Path, config: &CodexAiUsageConfig) -> AiUsageSnapshot {
    let now = unix_now();
    let mut five_tokens = 0u64;
    let mut seven_tokens = 0u64;
    let mut saw_usage = false;
    for event in codex_token_events(sessions_dir) {
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

fn codex_token_events(sessions_dir: &Path) -> Vec<TokenEvent> {
    let mut events = Vec::new();
    for path in sorted_jsonl_files(sessions_dir).into_iter().rev() {
        let Ok(text) = fs::read_to_string(&path) else {
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
    let credentials_path = config
        .credentials_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(default_claude_credentials_path);
    let text =
        fs::read_to_string(credentials_path).map_err(|_| AiUsageErrorCode::MissingCredentials)?;
    let value: Value = serde_json::from_str(&text).map_err(|_| AiUsageErrorCode::ParseFailed)?;
    let oauth = value
        .get("claudeAiOauth")
        .ok_or(AiUsageErrorCode::MissingCredentials)?;
    if let Some(expires_at) = oauth.get("expiresAt").and_then(Value::as_u64) {
        let now_ms = unix_now().saturating_mul(1000);
        if expires_at <= now_ms {
            return Err(AiUsageErrorCode::ExpiredCredentials);
        }
    }
    let access_token = oauth
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or(AiUsageErrorCode::MissingCredentials)?;

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(config.api_timeout_sec.max(1)))
        .build()
        .map_err(|_| AiUsageErrorCode::FetchFailed)?;
    let response = client
        .get(CLAUDE_USAGE_URL)
        .bearer_auth(access_token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .map_err(|_| AiUsageErrorCode::FetchFailed)?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(AiUsageErrorCode::AuthFailed);
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(AiUsageErrorCode::RateLimited);
    }
    if !status.is_success() {
        return Err(AiUsageErrorCode::FetchFailed);
    }
    let value: Value = response.json().map_err(|_| AiUsageErrorCode::ParseFailed)?;
    let five_hour = claude_window(value.get("five_hour"));
    let seven_day = claude_window(value.get("seven_day"));
    if five_hour.is_none() && seven_day.is_none() {
        return Err(AiUsageErrorCode::NoUsageData);
    }
    Ok(AiUsageSnapshot {
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
    })
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

fn sorted_jsonl_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_jsonl_files(root, &mut files);
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

        let snapshot = codex_rate_limits_snapshot(dir.path()).unwrap();

        assert!(snapshot.quota_source);
        assert!(!snapshot.estimated);
        assert_eq!(snapshot.five_hour.unwrap().used_bp, 1250);
        assert_eq!(snapshot.seven_day.unwrap().reset_unix, 1_800_100_000);
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

        let events = codex_token_events(dir.path());

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

        let events = codex_token_events(dir.path());

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

        let snapshot = codex_history_snapshot(dir.path(), &config);

        assert!(snapshot.estimated);
        assert!(snapshot.local_history_source);
        assert!(!snapshot.quota_source);
        assert_eq!(snapshot.five_hour.unwrap().reset_unix, 0);
    }
}
