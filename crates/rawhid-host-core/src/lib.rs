pub mod active_app;
pub mod ai_usage;
pub mod app_match;
pub mod config;
pub mod hid;
pub mod packet;
pub mod runner;
pub mod time;

pub use active_app::{ActiveApp, ActiveAppProvider, SystemActiveAppProvider};
pub use ai_usage::{
    AiUsageProviderStatus, AiUsageRuntime, AiUsageSendState, AiUsageShared, AiUsageStatusKind,
};
pub use app_match::{LayerAction, RuleMatch};
pub use config::{
    AiUsageConfig, AppConfig, ClaudeCodeAiUsageConfig, ClockMode, CodexAiUsageConfig, ConfigPaths,
    HidConfig, LayerSwitchConfig, PollingConfig, RuleConfig, TimeConfig, TimeFormatHint,
};
pub use hid::{DeviceInfo, HidDeviceManager, HidTransport, ProbeResult};
pub use packet::{
    AiUsageErrorCode, AiUsageFlags, AiUsagePacket, AiUsageProvider, AppLayerAction, Packet,
    PacketType, TimeSyncPacket, PACKET_SIZE, REPORT_SIZE,
};
pub use runner::{RunEvent, Runner};
pub use time::{Clock, SystemClock, TimeSnapshot};
