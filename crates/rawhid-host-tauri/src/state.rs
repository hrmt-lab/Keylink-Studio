use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rawhid_host_core::{
    ai_usage::{AiUsageProviderStatus, AiUsageRuntime, AiUsageShared},
    codex_activity::{AiClientStateSnapshot, CodexActivityRuntime},
    codex_broker::CodexBrokerManager,
    config::AppConfig,
    hid::{DeviceInfo, ProbeResult},
    packet::{
        ComboInfo, ComboItem, EncoderBinding, EncoderGetBindings, EncoderGetInfo, UplinkPacket,
    },
    runner::{DeviceBatteryStatus, DeviceLayerState},
    stats::{default_stats_dir, KeyStatsStore, SharedKeyStatsStore},
    studio::StudioEditSession,
};

use rawhid_host_core::{
    ClaudeObserverCounters, ClaudeObserverEvents, ClaudeObserverReceiver, ClaudeSessionRegistry,
};

pub const MAX_LOG_ENTRIES: usize = 200;

#[derive(Debug, Clone, serde::Serialize)]
pub struct LogEntry {
    pub id: u64,
    pub timestamp_ms: u64,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MonitorStatus {
    pub running: bool,
    pub connected_devices: usize,
    pub connected_device_names: Vec<String>,
    pub host_link_devices: Vec<DeviceInfo>,
    pub current_layer: Option<u8>,
    pub current_rule: Option<String>,
    pub last_error: Option<String>,
    pub ai_usage: Vec<AiUsageProviderStatus>,
    pub device_battery: Vec<DeviceBatteryStatus>,
    pub device_layers: Vec<DeviceLayerState>,
}

impl Default for MonitorStatus {
    fn default() -> Self {
        Self {
            running: false,
            connected_devices: 0,
            connected_device_names: Vec::new(),
            host_link_devices: Vec::new(),
            current_layer: None,
            current_rule: None,
            last_error: None,
            ai_usage: Vec::new(),
            device_battery: Vec::new(),
            device_layers: Vec::new(),
        }
    }
}

pub struct AppState {
    pub config: Arc<Mutex<AppConfig>>,
    pub config_path: Arc<Mutex<Option<PathBuf>>>,
    pub status: Arc<Mutex<MonitorStatus>>,
    pub log_entries: Arc<Mutex<VecDeque<LogEntry>>>,
    pub log_counter: Arc<Mutex<u64>>,
    pub monitor_tx: Arc<Mutex<Option<std::sync::mpsc::Sender<MonitorCommand>>>>,
    pub ai_usage_refreshing: Arc<AtomicBool>,
    pub ai_usage_runtime: Arc<Mutex<Option<AiUsageRuntime>>>,
    pub codex_activity: Arc<CodexActivityRuntime>,
    pub claude_integration: Arc<Mutex<Option<ClaudeIntegration>>>,
    pub ai_display_selection: Arc<Mutex<AiDisplaySelection>>,
    pub codex_broker: CodexBrokerManager,
    pub key_stats: SharedKeyStatsStore,
    pub studio_edit: Arc<Mutex<Option<StudioEditSession>>>,
    pub encoder_restore_rollbacks:
        Arc<Mutex<HashMap<(String, u64), BTreeMap<(u32, u8), EncoderGetBindings>>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiDisplayTarget {
    Codex {
        thread_id: String,
    },
    Claude {
        launch_id: String,
        session_id: String,
    },
}

impl AiDisplayTarget {
    pub fn label(&self) -> String {
        match self {
            Self::Codex { thread_id } => {
                format!("Codex {}", thread_id.chars().take(8).collect::<String>())
            }
            Self::Claude { session_id, .. } => format!("Claude Code {session_id}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AiDisplayCandidate {
    pub target: AiDisplayTarget,
    pub snapshot: AiClientStateSnapshot,
    /// First successful registration order across all AI client types.
    pub registration_order: u64,
}

#[derive(Debug, Default)]
pub struct AiDisplaySelection {
    candidates: Vec<AiDisplayCandidate>,
    selected: Option<AiDisplayTarget>,
    epoch: u64,
}

impl AiDisplaySelection {
    pub fn update_candidates(&mut self, candidates: Vec<AiDisplayCandidate>) {
        let old_index = self.selected.as_ref().and_then(|selected| {
            self.candidates
                .iter()
                .position(|candidate| &candidate.target == selected)
        });
        let mut candidates = candidates;
        candidates.sort_by_key(|candidate| candidate.registration_order);
        let next = self
            .selected
            .as_ref()
            .filter(|selected| {
                candidates
                    .iter()
                    .any(|candidate| &candidate.target == *selected)
            })
            .cloned()
            .or_else(|| {
                (!candidates.is_empty()).then(|| {
                    candidates[old_index.unwrap_or(0) % candidates.len()]
                        .target
                        .clone()
                })
            });
        self.candidates = candidates;
        self.set_selected(next);
    }

    pub fn cycle(&mut self) -> Option<AiDisplayTarget> {
        if self.candidates.is_empty() {
            self.set_selected(None);
            return None;
        }
        let next_index = self
            .selected
            .as_ref()
            .and_then(|selected| {
                self.candidates
                    .iter()
                    .position(|candidate| &candidate.target == selected)
            })
            .map(|index| (index + 1) % self.candidates.len())
            .unwrap_or(0);
        let next = self.candidates[next_index].target.clone();
        self.set_selected(Some(next.clone()));
        Some(next)
    }

    pub fn selected_target(&self) -> Option<&AiDisplayTarget> {
        self.selected.as_ref()
    }

    pub fn selected_snapshot(&self) -> Option<AiClientStateSnapshot> {
        let selected = self.selected.as_ref()?;
        self.candidates
            .iter()
            .find(|candidate| &candidate.target == selected)
            .map(|candidate| candidate.snapshot)
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    fn set_selected(&mut self, selected: Option<AiDisplayTarget>) {
        if self.selected != selected {
            self.selected = selected;
            self.epoch = self.epoch.wrapping_add(1);
        }
    }
}

pub struct ClaudeIntegration {
    pub launches: BTreeMap<String, ClaudeLaunchIntegration>,
    pub registry: ClaudeSessionRegistry,
}

pub struct ClaudeLaunchIntegration {
    pub receiver: ClaudeObserverReceiver,
    pub events: ClaudeObserverEvents,
    pub last_counters: ClaudeObserverCounters,
    pub plugin_root: PathBuf,
}

#[derive(Debug)]
pub enum MonitorCommand {
    SetAutomationEnabled(bool, std::sync::mpsc::Sender<Result<(), String>>),
    Probe(std::sync::mpsc::Sender<Result<Vec<ProbeResult>, String>>),
    Config(HostLinkCall),
    Shutdown,
    UpdateConfig(AppConfig, Option<AiUsageShared>),
    /// The OS foreground window changed; wake the loop to re-evaluate immediately.
    ForegroundChanged,
    /// Debug-only: feed a synthetic uplink packet through the normal path.
    InjectUplink(DeviceInfo, UplinkPacket),
}

#[derive(Debug)]
pub struct HostLinkCall {
    pub uid: u64,
    pub request: HostLinkRequest,
    pub deadline: Instant,
    pub reply: std::sync::mpsc::Sender<Result<HostLinkResponse, String>>,
}

#[derive(Debug, Clone, Copy)]
pub enum HostLinkRequest {
    EncoderGetInfo,
    EncoderGetBindings {
        layer_id: u32,
        encoder_id: u8,
    },
    EncoderSetBindings {
        layer_id: u32,
        encoder_id: u8,
        cw: EncoderBinding,
        ccw: EncoderBinding,
    },
    EncoderGetDirty,
    EncoderSave,
    EncoderDiscard,
    EncoderClearOverride {
        layer_id: u32,
        encoder_id: u8,
    },
    ComboGetInfo,
    ComboGet {
        slot: u8,
    },
    ComboSet {
        item: ComboItem,
    },
    ComboGetDirty,
    ComboSave,
    ComboDiscard,
    ComboDelete {
        slot: u8,
    },
    ComboResetToKeymap,
}

#[derive(Debug)]
pub enum HostLinkResponse {
    EncoderInfo(EncoderGetInfo),
    EncoderBindings(EncoderGetBindings),
    ComboInfo(ComboInfo),
    ComboItem(ComboItem),
    Dirty(bool),
    Done,
}

impl AppState {
    pub fn new(config: AppConfig, config_path: Option<PathBuf>) -> Self {
        let codex_broker = CodexBrokerManager::new();
        let codex_activity = Arc::new(CodexActivityRuntime::start(codex_broker.clone()));
        let ai_usage_runtime = AiUsageRuntime::start(config.ai_usage.clone());
        let ai_usage_statuses = ai_usage_runtime
            .as_ref()
            .map(|runtime| runtime.statuses(config.ai_usage.stale_after_sec))
            .unwrap_or_default();
        let mut status = MonitorStatus::default();
        status.ai_usage = ai_usage_statuses;
        let stats_dir = default_stats_dir()
            .unwrap_or_else(|| std::env::temp_dir().join("keylink-studio").join("stats"));
        let key_stats = Arc::new(Mutex::new(KeyStatsStore::new(
            stats_dir,
            Duration::from_secs(config.stats.flush_interval_sec.max(1)),
        )));
        Self {
            config: Arc::new(Mutex::new(config)),
            config_path: Arc::new(Mutex::new(config_path)),
            status: Arc::new(Mutex::new(status)),
            log_entries: Arc::new(Mutex::new(VecDeque::new())),
            log_counter: Arc::new(Mutex::new(0)),
            monitor_tx: Arc::new(Mutex::new(None)),
            ai_usage_refreshing: Arc::new(AtomicBool::new(false)),
            ai_usage_runtime: Arc::new(Mutex::new(ai_usage_runtime)),
            codex_activity,
            claude_integration: Arc::new(Mutex::new(None)),
            ai_display_selection: Arc::new(Mutex::new(AiDisplaySelection::default())),
            codex_broker,
            key_stats,
            studio_edit: Arc::new(Mutex::new(None)),
            encoder_restore_rollbacks: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn add_log(
    log_entries: &Arc<Mutex<VecDeque<LogEntry>>>,
    log_counter: &Arc<Mutex<u64>>,
    level: &str,
    message: &str,
) -> LogEntry {
    let id = {
        let mut counter = log_counter.lock().unwrap();
        *counter += 1;
        *counter
    };
    let entry = LogEntry {
        id,
        timestamp_ms: now_ms(),
        level: level.to_string(),
        message: message.to_string(),
    };
    let mut entries = log_entries.lock().unwrap();
    entries.push_back(entry.clone());
    while entries.len() > MAX_LOG_ENTRIES {
        entries.pop_front();
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use rawhid_host_core::packet::{AiActivityState, AiClientType, AiClientVariant, AiWorkPhase};

    fn candidate(target: AiDisplayTarget, revision: u16) -> AiDisplayCandidate {
        AiDisplayCandidate {
            target,
            snapshot: AiClientStateSnapshot {
                client_type: AiClientType::Codex,
                client_variant: AiClientVariant::Cli,
                session_active: true,
                activity_state: AiActivityState::Available,
                work_phase: AiWorkPhase::Unspecified,
                revision,
            },
            registration_order: u64::from(revision),
        }
    }

    fn claude(session_id: &str) -> AiDisplayTarget {
        AiDisplayTarget::Claude {
            launch_id: "launch-1".to_string(),
            session_id: session_id.to_string(),
        }
    }

    fn codex(thread_id: &str) -> AiDisplayTarget {
        AiDisplayTarget::Codex {
            thread_id: thread_id.to_string(),
        }
    }

    #[test]
    fn adding_a_candidate_does_not_steal_the_current_selection() {
        let mut selection = AiDisplaySelection::default();
        selection.update_candidates(vec![candidate(claude("one"), 1)]);
        selection.update_candidates(vec![
            candidate(codex("codex-one"), 2),
            candidate(claude("one"), 1),
        ]);

        assert_eq!(selection.selected_target(), Some(&claude("one")));
    }

    #[test]
    fn removing_the_selected_candidate_advances_from_its_old_position() {
        let mut selection = AiDisplaySelection::default();
        selection.update_candidates(vec![
            candidate(codex("codex-one"), 1),
            candidate(claude("one"), 2),
            candidate(claude("two"), 3),
        ]);
        selection.cycle();
        assert_eq!(selection.selected_target(), Some(&claude("one")));

        selection.update_candidates(vec![
            candidate(codex("codex-one"), 1),
            candidate(claude("two"), 3),
        ]);
        assert_eq!(selection.selected_target(), Some(&claude("two")));
    }

    #[test]
    fn candidates_cycle_in_cross_client_registration_order() {
        let mut selection = AiDisplaySelection::default();
        selection.update_candidates(vec![candidate(claude("one"), 1)]);
        selection.update_candidates(vec![
            candidate(codex("codex-one"), 2),
            candidate(claude("one"), 1),
        ]);
        selection.update_candidates(vec![
            candidate(codex("codex-one"), 2),
            candidate(codex("codex-two"), 3),
            candidate(claude("one"), 1),
            candidate(claude("two"), 4),
        ]);

        assert_eq!(selection.selected_target(), Some(&claude("one")));
        assert_eq!(selection.cycle(), Some(codex("codex-one")));
        assert_eq!(selection.cycle(), Some(codex("codex-two")));
        assert_eq!(selection.cycle(), Some(claude("two")));
        assert_eq!(selection.cycle(), Some(claude("one")));
    }

    #[test]
    fn first_batch_uses_shared_registration_order_not_client_type_order() {
        let mut selection = AiDisplaySelection::default();
        selection.update_candidates(vec![
            candidate(codex("codex-first-in_input"), 2),
            candidate(claude("claude-registered-first"), 1),
        ]);

        assert_eq!(
            selection.selected_target(),
            Some(&claude("claude-registered-first"))
        );
        assert_eq!(selection.cycle(), Some(codex("codex-first-in_input")));
    }

    #[test]
    fn reactivated_candidate_returns_to_its_original_registration_position() {
        let mut selection = AiDisplaySelection::default();
        selection.update_candidates(vec![candidate(claude("a"), 1), candidate(claude("b"), 2)]);
        // B temporarily leaves the active candidates, while later sessions
        // become active. A resumed B must retain its original position.
        selection.update_candidates(vec![candidate(claude("a"), 1)]);
        selection.update_candidates(vec![
            candidate(claude("a"), 1),
            candidate(codex("c"), 3),
            candidate(codex("d"), 4),
        ]);
        selection.update_candidates(vec![
            candidate(claude("a"), 1),
            candidate(claude("b"), 2),
            candidate(codex("c"), 3),
            candidate(codex("d"), 4),
        ]);

        assert_eq!(selection.selected_target(), Some(&claude("a")));
        assert_eq!(selection.cycle(), Some(claude("b")));
        assert_eq!(selection.cycle(), Some(codex("c")));
        assert_eq!(selection.cycle(), Some(codex("d")));
        assert_eq!(selection.cycle(), Some(claude("a")));
    }

    #[test]
    fn updating_a_non_selected_candidate_does_not_change_selection() {
        let mut selection = AiDisplaySelection::default();
        selection.update_candidates(vec![
            candidate(codex("codex-one"), 1),
            candidate(codex("codex-two"), 2),
        ]);
        selection.update_candidates(vec![
            candidate(codex("codex-one"), 1),
            candidate(codex("codex-two"), 99),
        ]);

        assert_eq!(selection.selected_target(), Some(&codex("codex-one")));
        assert_eq!(selection.selected_snapshot().unwrap().revision, 1);
    }

    #[test]
    fn zero_and_one_candidate_are_safe_to_cycle() {
        let mut selection = AiDisplaySelection::default();
        assert_eq!(selection.cycle(), None);
        assert_eq!(selection.selected_target(), None);

        let only = codex("codex-one");
        selection.update_candidates(vec![candidate(only.clone(), 1)]);
        assert_eq!(selection.cycle(), Some(only.clone()));
        assert_eq!(selection.cycle(), Some(only));

        selection.update_candidates(Vec::new());
        assert_eq!(selection.selected_target(), None);
    }
}
