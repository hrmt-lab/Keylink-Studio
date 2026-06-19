pub mod active_app;
pub mod ai_usage;
pub mod app_match;
pub mod config;
pub mod hid;
pub mod packet;
pub mod runner;
pub mod stats;
pub mod studio;
pub mod time;

pub use active_app::{ActiveApp, ActiveAppProvider, SystemActiveAppProvider};
pub use ai_usage::{
    AiUsageProviderStatus, AiUsageRuntime, AiUsageSendState, AiUsageShared, AiUsageStatusKind,
};
pub use app_match::{LayerAction, RuleMatch};
pub use config::{
    AiUsageConfig, AppConfig, ClaudeCodeAiUsageConfig, ClockMode, CodexAiUsageConfig, ConfigPaths,
    DeviceLayerSwitchConfig, HidConfig, LayerSwitchConfig, PollingConfig, RuleConfig, StudioConfig,
    TimeConfig, TimeFormatHint, UnmatchedAction,
};
pub use hid::{DeviceConnectionType, DeviceInfo, HidDeviceManager, HidTransport, ProbeResult};
pub use packet::{
    AiUsageErrorCode, AiUsageFlags, AiUsagePacket, AiUsageProvider, AppLayerAction, BatteryEntry,
    BatteryStatusPacket, DeviceHello, HostActionPacket, KeyStatsEntry, KeyStatsPacket,
    LayerStatePacket, Packet, PacketType, TimeSyncPacket, UplinkPacket, CAPABILITY_AI_USAGE,
    CAPABILITY_APP_LAYER, CAPABILITY_BATTERY, CAPABILITY_HOST_ACTION, CAPABILITY_KEY_STATS,
    CAPABILITY_LAYER_STATE, CAPABILITY_THEME, CAPABILITY_TIME_SYNC, PACKET_SIZE, REPORT_SIZE,
};
pub use runner::{
    uplink_device_key, DeviceBatterySource, DeviceBatteryStatus, DeviceLayerState, RunEvent,
    Runner, UplinkEvent,
};
pub use stats::{KeyStatsStore, KeyStatsSummary, SharedKeyStatsStore, StatsPeriod};
pub use studio::{
    KeymapViewerStatus, StudioBinding, StudioDeviceStatus, StudioError, StudioErrorCode,
    StudioKeymapSnapshot, StudioLayer, StudioLayoutSource, StudioLockState, StudioPhysicalKey,
    StudioPhysicalLayout, StudioRpcStatus,
};
pub use time::{Clock, SystemClock, TimeSnapshot};
