use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use directories::ProjectDirs;
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AppConfig {
    pub app: AppBehaviorConfig,
    pub polling: PollingConfig,
    pub hid: HidConfig,
    pub layer_switch: LayerSwitchConfig,
    pub time: TimeConfig,
    pub ai_usage: AiUsageConfig,
    pub studio: StudioConfig,
    pub stats: StatsConfig,
    pub actions: ActionsConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            app: AppBehaviorConfig::default(),
            polling: PollingConfig::default(),
            hid: HidConfig::default(),
            layer_switch: LayerSwitchConfig::default(),
            time: TimeConfig::default(),
            ai_usage: AiUsageConfig::default(),
            studio: StudioConfig::default(),
            stats: StatsConfig::default(),
            actions: ActionsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct StatsConfig {
    /// Record per-position key statistics reported by KEY_STATS-capable devices.
    pub enabled: bool,
    pub flush_interval_sec: u64,
}

impl Default for StatsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            flush_interval_sec: 60,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct ActionsConfig {
    /// Execute HOST_ACTION packets. Disabled by default; unbound action ids
    /// are only logged even when enabled.
    pub enabled: bool,
    /// Bindings are configured per device, keyed by the stable uid returned
    /// in DEVICE_HELLO (same keying as `layer_switch.devices`). Devices
    /// without an entry only have their actions logged.
    pub devices: BTreeMap<String, DeviceActionsConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DeviceActionsConfig {
    pub display_name: Option<String>,
    pub enabled: bool,
    pub bindings: Vec<ActionBinding>,
}

impl Default for DeviceActionsConfig {
    fn default() -> Self {
        Self {
            display_name: None,
            enabled: true,
            bindings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ActionBinding {
    pub action_id: u8,
    pub action: HostActionKind,
    /// Executable path for `launch`. The HID value byte is never used as a path.
    pub path: Option<String>,
}

impl Default for ActionBinding {
    fn default() -> Self {
        Self {
            action_id: 0,
            action: HostActionKind::ShowWindow,
            path: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostActionKind {
    ShowWindow,
    StartMonitoring,
    StopMonitoring,
    RefreshAiUsage,
    Launch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AppBehaviorConfig {
    /// Start monitoring automatically when the GUI launches.
    pub start_monitoring_on_launch: bool,
}

impl Default for AppBehaviorConfig {
    fn default() -> Self {
        Self {
            start_monitoring_on_launch: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PollingConfig {
    pub interval_ms: u64,
}

impl Default for PollingConfig {
    fn default() -> Self {
        Self { interval_ms: 500 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct HidConfig {
    pub usage_page: u16,
    pub usage: u16,
    pub hello_timeout_ms: i32,
    pub rescan_interval_sec: u64,
}

impl Default for HidConfig {
    fn default() -> Self {
        Self {
            usage_page: 0xFF60,
            usage: 0x61,
            hello_timeout_ms: 200,
            rescan_interval_sec: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct StudioConfig {
    pub probe_timeout_ms: u64,
    pub keymap_read_timeout_ms: u64,
}

impl Default for StudioConfig {
    fn default() -> Self {
        Self {
            probe_timeout_ms: 1000,
            keymap_read_timeout_ms: 8000,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LayerSwitchConfig {
    pub enabled: bool,
    pub unmatched_action: UnmatchedAction,
    pub devices: BTreeMap<String, DeviceLayerSwitchConfig>,
}

impl Default for LayerSwitchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            unmatched_action: UnmatchedAction::ClearManaged,
            devices: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DeviceLayerSwitchConfig {
    pub display_name: Option<String>,
    pub enabled: bool,
    pub rules: Vec<RuleConfig>,
    pub unmatched_action: Option<UnmatchedAction>,
}

impl Default for DeviceLayerSwitchConfig {
    fn default() -> Self {
        Self {
            display_name: None,
            enabled: true,
            rules: Vec::new(),
            unmatched_action: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnmatchedAction {
    ClearManaged,
    Keep,
}

impl Default for UnmatchedAction {
    fn default() -> Self {
        Self::ClearManaged
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuleConfig {
    pub name: String,
    pub layer: u8,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub exe: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TimeConfig {
    pub enabled: bool,
    pub format_hint: TimeFormatHint,
    pub clock_mode: ClockMode,
    pub periodic_sync_sec: u64,
    #[serde(default, deserialize_with = "deserialize_tz_offset_min")]
    pub tz_offset_min: Option<i16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AiUsageConfig {
    pub enabled: bool,
    pub poll_interval_sec: u64,
    pub stale_after_sec: u64,
    pub codex: CodexAiUsageConfig,
    pub claude_code: ClaudeCodeAiUsageConfig,
}

impl Default for AiUsageConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_interval_sec: 300,
            stale_after_sec: 900,
            codex: CodexAiUsageConfig::default(),
            claude_code: ClaudeCodeAiUsageConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CodexAiUsageConfig {
    pub enabled: bool,
    pub sessions_dir: Option<String>,
    pub sessions_auto_detect: bool,
    pub include_wsl_sessions: bool,
    pub extra_sessions_paths: Vec<String>,
    pub history_fallback_enabled: bool,
    pub allow_activity_baseline: bool,
    pub activity_five_hour_token_baseline: u64,
    pub activity_seven_day_token_baseline: u64,
}

impl Default for CodexAiUsageConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sessions_dir: None,
            sessions_auto_detect: true,
            include_wsl_sessions: true,
            extra_sessions_paths: Vec::new(),
            history_fallback_enabled: true,
            allow_activity_baseline: false,
            activity_five_hour_token_baseline: 0,
            activity_seven_day_token_baseline: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ClaudeCodeAiUsageConfig {
    pub enabled: bool,
    pub credentials_path: Option<String>,
    pub credentials_auto_detect: bool,
    pub include_wsl_credentials: bool,
    pub extra_credentials_paths: Vec<String>,
    pub api_timeout_sec: u64,
}

impl Default for ClaudeCodeAiUsageConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            credentials_path: None,
            credentials_auto_detect: true,
            include_wsl_credentials: true,
            extra_credentials_paths: Vec::new(),
            api_timeout_sec: 10,
        }
    }
}

impl Default for TimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            format_hint: TimeFormatHint::TimeHm,
            clock_mode: ClockMode::TwentyFourHour,
            periodic_sync_sec: 60,
            tz_offset_min: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimeFormatHint {
    TimeHm,
    TimeHms,
    DateYmd,
    DateMd,
    DatetimeHm,
    WeekdayHm,
}

impl TimeFormatHint {
    pub fn as_packet_value(self) -> u8 {
        match self {
            Self::TimeHm => 0,
            Self::TimeHms => 1,
            Self::DateYmd => 2,
            Self::DateMd => 3,
            Self::DatetimeHm => 4,
            Self::WeekdayHm => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ClockMode {
    #[serde(rename = "24h")]
    TwentyFourHour,
    #[serde(rename = "12h")]
    TwelveHour,
}

impl ClockMode {
    pub fn as_packet_value(self) -> u8 {
        match self {
            Self::TwentyFourHour => 0,
            Self::TwelveHour => 1,
        }
    }
}

fn deserialize_tz_offset_min<'de, D>(deserializer: D) -> Result<Option<i16>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<i16>::deserialize(deserializer)?;
    if let Some(offset) = value {
        if !(-1440..=1440).contains(&offset) {
            return Err(serde::de::Error::custom(
                "tz_offset_min must be between -1440 and 1440",
            ));
        }
    }
    Ok(value)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    pub explicit: Option<PathBuf>,
    pub cwd: PathBuf,
    pub user: Option<PathBuf>,
}

impl ConfigPaths {
    pub fn discover(explicit: Option<PathBuf>) -> Self {
        Self {
            explicit,
            cwd: PathBuf::from("rawhid-host.toml"),
            user: user_config_path(),
        }
    }

    pub fn selected_path(&self) -> Option<PathBuf> {
        if let Some(explicit) = &self.explicit {
            return Some(explicit.clone());
        }
        if self.cwd.exists() {
            return Some(self.cwd.clone());
        }
        if let Some(user) = &self.user {
            if user.exists() {
                return Some(user.clone());
            }
        }
        self.user.clone()
    }

    pub fn existing_path(&self) -> Option<PathBuf> {
        if let Some(explicit) = &self.explicit {
            return explicit.exists().then(|| explicit.clone());
        }
        if self.cwd.exists() {
            return Some(self.cwd.clone());
        }
        self.user
            .as_ref()
            .and_then(|user| user.exists().then(|| user.clone()))
    }
}

pub fn load_config(explicit: Option<PathBuf>) -> Result<(AppConfig, Option<PathBuf>), ConfigError> {
    let paths = ConfigPaths::discover(explicit);
    let Some(path) = paths.existing_path() else {
        return Ok((AppConfig::default(), None));
    };

    let text = fs::read_to_string(&path).map_err(|source| ConfigError::Read {
        path: path.clone(),
        source,
    })?;
    let config = toml::from_str(&text).map_err(|source| ConfigError::Parse {
        path: path.clone(),
        source,
    })?;
    Ok((config, Some(path)))
}

pub fn write_default_config(path: &Path, overwrite: bool) -> Result<(), ConfigError> {
    if path.exists() && !overwrite {
        return Err(ConfigError::AlreadyExists(path.to_path_buf()));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ConfigError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(path, example_config()).map_err(|source| ConfigError::Write {
        path: path.to_path_buf(),
        source,
    })
}

pub fn example_config() -> &'static str {
    r#"# RawHID Host configuration

[app]
# Start monitoring automatically when the GUI launches.
start_monitoring_on_launch = false

[polling]
interval_ms = 500

[hid]
usage_page = 65376 # 0xFF60
usage = 97         # 0x61
hello_timeout_ms = 200
rescan_interval_sec = 5

[studio]
probe_timeout_ms = 1000
keymap_read_timeout_ms = 8000

[layer_switch]
enabled = true
unmatched_action = "clear_managed"

# Layer rules are configured per device, keyed by the stable uid returned
# in DEVICE_HELLO. Devices without an entry are not layer-managed.
#[layer_switch.devices."uid:7a91c3e4d102ab55"]
#display_name = "Example Keyboard"
#enabled = true
#
# exe matches the full executable file name, including ".exe".
# Prefer path rules when the same exe name can appear in multiple locations.
#[[layer_switch.devices."uid:7a91c3e4d102ab55".rules]]
#name = "VS Code"
#exe = "Code.exe"
#layer = 3

[time]
enabled = false
format_hint = "time_hm"
clock_mode = "24h"
periodic_sync_sec = 60
# tz_offset_min = 540

[stats]
# Record per-position key statistics from KEY_STATS-capable keyboards.
enabled = true
flush_interval_sec = 60

[actions]
# Execute HOST_ACTION packets sent from the keyboard. Disabled by default.
enabled = false

# Bindings are configured per device, keyed by the stable uid returned in
# DEVICE_HELLO (same keying as layer_switch.devices). Unbound ids are only
# logged.
#[actions.devices."uid:7a91c3e4d102ab55"]
#display_name = "Example Keyboard"
#enabled = true
#
#[[actions.devices."uid:7a91c3e4d102ab55".bindings]]
#action_id = 1
#action = "show_window"  # show_window | start_monitoring | stop_monitoring | refresh_ai_usage | launch
#
#[[actions.devices."uid:7a91c3e4d102ab55".bindings]]
#action_id = 2
#action = "launch"
#path = "C:\\tools\\example.exe"

[ai_usage]
enabled = false
poll_interval_sec = 300
stale_after_sec = 900

[ai_usage.codex]
enabled = true
# sessions_dir = "C:\\Users\\<user>\\.codex\\sessions"
sessions_auto_detect = true
include_wsl_sessions = true
extra_sessions_paths = []
history_fallback_enabled = true
allow_activity_baseline = false
activity_five_hour_token_baseline = 0
activity_seven_day_token_baseline = 0

[ai_usage.claude_code]
enabled = true
# credentials_path = "C:\\Users\\<user>\\.claude\\.credentials.json"
credentials_auto_detect = true
include_wsl_credentials = true
extra_credentials_paths = []
api_timeout_sec = 10
"#
}

fn user_config_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", "RawHID Host").map(|dirs| dirs.config_dir().join("config.toml"))
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("failed to create config directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write config {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("config already exists: {0}")]
    AlreadyExists(PathBuf),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_example_config() {
        let config: AppConfig = toml::from_str(example_config()).unwrap();

        assert_eq!(config.polling.interval_ms, 500);
        assert_eq!(config.hid.usage_page, 0xFF60);
        assert_eq!(config.hid.usage, 0x61);
        assert_eq!(config.hid.rescan_interval_sec, 5);
        assert_eq!(config.studio.probe_timeout_ms, 1000);
        assert_eq!(config.studio.keymap_read_timeout_ms, 8000);
        assert!(config.layer_switch.enabled);
        assert_eq!(
            config.layer_switch.unmatched_action,
            UnmatchedAction::ClearManaged
        );
        assert!(config.layer_switch.devices.is_empty());
        assert!(config.stats.enabled);
        assert_eq!(config.stats.flush_interval_sec, 60);
        assert!(!config.actions.enabled);
        assert!(config.actions.devices.is_empty());
        assert!(!config.time.enabled);
        assert_eq!(config.time.format_hint, TimeFormatHint::TimeHm);
        assert_eq!(config.time.clock_mode, ClockMode::TwentyFourHour);
        assert_eq!(config.time.periodic_sync_sec, 60);
        assert!(!config.ai_usage.enabled);
        assert!(config.ai_usage.codex.enabled);
    }

    #[test]
    fn parses_device_layer_switch_config() {
        let config: AppConfig = toml::from_str(
            r#"
[layer_switch]
enabled = true
unmatched_action = "clear_managed"

[layer_switch.devices."uid:7a91c3e4d102ab55"]
display_name = "LotusUni Dongle"
enabled = true

[[layer_switch.devices."uid:7a91c3e4d102ab55".rules]]
name = "VS Code"
exe = "Code.exe"
layer = 3

[layer_switch.devices."uid:91ac51d6ef0201aa"]
display_name = "LeftPad"
enabled = true
"#,
        )
        .unwrap();

        assert_eq!(config.layer_switch.devices.len(), 2);
        let lotus = config
            .layer_switch
            .devices
            .get("uid:7a91c3e4d102ab55")
            .unwrap();
        assert_eq!(lotus.display_name.as_deref(), Some("LotusUni Dongle"));
        assert_eq!(lotus.rules.len(), 1);
        let left = config
            .layer_switch
            .devices
            .get("uid:91ac51d6ef0201aa")
            .unwrap();
        assert!(left.rules.is_empty());
    }
    #[test]
    fn parses_actions_config() {
        let config: AppConfig = toml::from_str(
            r#"
[actions]
enabled = true

[actions.devices."uid:7a91c3e4d102ab55"]
display_name = "LotusUni Dongle"
enabled = true

[[actions.devices."uid:7a91c3e4d102ab55".bindings]]
action_id = 1
action = "show_window"

[[actions.devices."uid:7a91c3e4d102ab55".bindings]]
action_id = 7
action = "launch"
path = "C:\\tools\\example.exe"

[actions.devices."uid:91ac51d6ef0201aa"]
enabled = false
"#,
        )
        .unwrap();

        assert!(config.actions.enabled);
        assert_eq!(config.actions.devices.len(), 2);
        let lotus = config.actions.devices.get("uid:7a91c3e4d102ab55").unwrap();
        assert_eq!(lotus.display_name.as_deref(), Some("LotusUni Dongle"));
        assert!(lotus.enabled);
        assert_eq!(lotus.bindings.len(), 2);
        assert_eq!(lotus.bindings[0].action_id, 1);
        assert_eq!(lotus.bindings[0].action, HostActionKind::ShowWindow);
        assert_eq!(lotus.bindings[1].action, HostActionKind::Launch);
        assert_eq!(
            lotus.bindings[1].path.as_deref(),
            Some("C:\\tools\\example.exe")
        );
        let left = config.actions.devices.get("uid:91ac51d6ef0201aa").unwrap();
        assert!(!left.enabled);
        assert!(left.bindings.is_empty());
    }

    #[test]
    fn parses_time_config() {
        let config: AppConfig = toml::from_str(
            r#"
[time]
enabled = true
format_hint = "weekday_hm"
clock_mode = "12h"
periodic_sync_sec = 0
tz_offset_min = 540
"#,
        )
        .unwrap();

        assert!(config.time.enabled);
        assert_eq!(config.time.format_hint, TimeFormatHint::WeekdayHm);
        assert_eq!(config.time.clock_mode, ClockMode::TwelveHour);
        assert_eq!(config.time.periodic_sync_sec, 0);
        assert_eq!(config.time.tz_offset_min, Some(540));
    }

    #[test]
    fn rejects_out_of_range_tz_offset() {
        let error = toml::from_str::<AppConfig>(
            r#"
[time]
tz_offset_min = 1441
"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("tz_offset_min"));
    }

    #[test]
    fn parses_ai_usage_config() {
        let config: AppConfig = toml::from_str(
            r#"
[ai_usage]
enabled = true
poll_interval_sec = 120
stale_after_sec = 600

[ai_usage.codex]
enabled = true
sessions_dir = "C:\\Users\\me\\.codex\\sessions"
sessions_auto_detect = true
include_wsl_sessions = true
extra_sessions_paths = ["\\\\wsl.localhost\\Ubuntu\\home\\me\\.codex\\sessions"]
history_fallback_enabled = true
allow_activity_baseline = true
activity_five_hour_token_baseline = 1000
activity_seven_day_token_baseline = 7000

[ai_usage.claude_code]
enabled = true
credentials_path = "C:\\Users\\me\\.claude\\.credentials.json"
credentials_auto_detect = true
include_wsl_credentials = true
extra_credentials_paths = ["C:\\Users\\me\\.claude-alt\\.credentials.json"]
api_timeout_sec = 5
"#,
        )
        .unwrap();

        assert!(config.ai_usage.enabled);
        assert_eq!(config.ai_usage.poll_interval_sec, 120);
        assert_eq!(
            config.ai_usage.codex.activity_five_hour_token_baseline,
            1000
        );
        assert_eq!(config.ai_usage.claude_code.api_timeout_sec, 5);
        assert!(config.ai_usage.codex.sessions_auto_detect);
        assert!(config.ai_usage.codex.include_wsl_sessions);
        assert_eq!(config.ai_usage.codex.extra_sessions_paths.len(), 1);
        assert!(config.ai_usage.claude_code.credentials_auto_detect);
        assert!(config.ai_usage.claude_code.include_wsl_credentials);
        assert_eq!(config.ai_usage.claude_code.extra_credentials_paths.len(), 1);
    }

    #[test]
    fn parses_studio_config() {
        let config: AppConfig = toml::from_str(
            r#"
[studio]
probe_timeout_ms = 250
keymap_read_timeout_ms = 5000
"#,
        )
        .unwrap();

        assert_eq!(config.studio.probe_timeout_ms, 250);
        assert_eq!(config.studio.keymap_read_timeout_ms, 5000);
    }
    #[test]
    fn explicit_path_is_selected_even_when_missing() {
        let paths = ConfigPaths {
            explicit: Some(PathBuf::from("custom.toml")),
            cwd: PathBuf::from("rawhid-host.toml"),
            user: Some(PathBuf::from("user.toml")),
        };

        assert_eq!(paths.selected_path(), Some(PathBuf::from("custom.toml")));
    }

    #[test]
    fn missing_config_loads_default() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.toml");
        let (config, path) = load_config(Some(missing)).unwrap();

        assert_eq!(config, AppConfig::default());
        assert_eq!(path, None);
    }
}
