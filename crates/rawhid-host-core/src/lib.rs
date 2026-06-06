pub mod active_app;
pub mod ai_usage;
pub mod app_match;
pub mod config;
pub mod hid;
pub mod packet;
pub mod runner;
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
pub use hid::{DeviceInfo, HidDeviceManager, HidTransport, ProbeResult};
pub use packet::{
    AiUsageErrorCode, AiUsageFlags, AiUsagePacket, AiUsageProvider, AppLayerAction, DeviceHello,
    Packet, PacketType, TimeSyncPacket, CAPABILITY_AI_USAGE, CAPABILITY_APP_LAYER,
    CAPABILITY_THEME, CAPABILITY_TIME_SYNC, PACKET_SIZE, REPORT_SIZE,
};
pub use runner::{RunEvent, Runner};
pub use studio::{
    KeymapViewerStatus, StudioBinding, StudioDeviceStatus, StudioError, StudioErrorCode,
    StudioKeymapSnapshot, StudioLayer, StudioLayoutSource, StudioLockState, StudioPhysicalKey,
    StudioPhysicalLayout, StudioRpcStatus,
};
pub use time::{Clock, SystemClock, TimeSnapshot};
