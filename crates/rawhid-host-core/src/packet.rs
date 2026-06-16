use thiserror::Error;

pub const PACKET_SIZE: usize = 32;
pub const REPORT_SIZE: usize = PACKET_SIZE + 1;
pub const MAGIC: [u8; 2] = *b"HL";
pub const VERSION: u8 = 0x01;
pub const MAX_LAYER: u8 = 31;
pub const AI_USAGE_MAX_BASIS_POINTS: u16 = 10_000;
pub const CAPABILITY_APP_LAYER: u32 = 1 << 0;
pub const CAPABILITY_TIME_SYNC: u32 = 1 << 1;
pub const CAPABILITY_AI_USAGE: u32 = 1 << 2;
pub const CAPABILITY_THEME: u32 = 1 << 3;
pub const CAPABILITY_BATTERY: u32 = 1 << 4;
pub const CAPABILITY_HOST_ACTION: u32 = 1 << 5;
pub const CAPABILITY_KEY_STATS: u32 = 1 << 6;
pub const CAPABILITY_LAYER_STATE: u32 = 1 << 7;
pub const CAPABILITY_KEY_PRESS: u32 = 1 << 8;

pub const BATTERY_LEVEL_UNKNOWN: u8 = 0xFF;
pub const KEY_STATS_MAX_ENTRIES: usize = 8;
pub const KEY_STATS_FLAG_MORE_FOLLOWS: u8 = 1 << 0;
pub const KEY_PRESS_FLAG_PRESSED: u8 = 1 << 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PacketType {
    HostHello = 0x01,
    DeviceHello = 0x02,
    Error = 0x03,
    Ping = 0x04,
    Pong = 0x05,
    AiUsage = 0x10,
    TimeSync = 0x20,
    AppLayer = 0x30,
    BatteryStatus = 0x40,
    HostAction = 0x50,
    KeyStats = 0x60,
    LayerState = 0x70,
    KeyPress = 0x80,
}

impl TryFrom<u8> for PacketType {
    type Error = PacketError;

    fn try_from(value: u8) -> Result<Self, <Self as TryFrom<u8>>::Error> {
        match value {
            0x01 => Ok(Self::HostHello),
            0x02 => Ok(Self::DeviceHello),
            0x03 => Ok(Self::Error),
            0x04 => Ok(Self::Ping),
            0x05 => Ok(Self::Pong),
            0x10 => Ok(Self::AiUsage),
            0x20 => Ok(Self::TimeSync),
            0x30 => Ok(Self::AppLayer),
            0x40 => Ok(Self::BatteryStatus),
            0x50 => Ok(Self::HostAction),
            0x60 => Ok(Self::KeyStats),
            0x70 => Ok(Self::LayerState),
            0x80 => Ok(Self::KeyPress),
            other => Err(PacketError::UnknownType(other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AppLayerAction {
    Set = 1,
    Clear = 2,
}

impl TryFrom<u8> for AppLayerAction {
    type Error = PacketError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Set),
            2 => Ok(Self::Clear),
            other => Err(PacketError::InvalidAppLayerAction(other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AiUsageProvider {
    Codex = 1,
    ClaudeCode = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AiUsageErrorCode {
    None = 0,
    SourceDisabled = 1,
    MissingCredentials = 2,
    ExpiredCredentials = 3,
    AuthFailed = 4,
    RateLimited = 5,
    FetchFailed = 6,
    ParseFailed = 7,
    NoUsageData = 8,
    MissingLimit = 9,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AiUsageFlags(u8);

impl AiUsageFlags {
    pub const FIVE_HOUR_VALID: u8 = 1 << 0;
    pub const SEVEN_DAY_VALID: u8 = 1 << 1;
    pub const ESTIMATED: u8 = 1 << 2;
    pub const LOCAL_HISTORY_SOURCE: u8 = 1 << 3;
    pub const QUOTA_SOURCE: u8 = 1 << 4;
    pub const STALE: u8 = 1 << 5;
    pub const FALLBACK_LIMIT: u8 = 1 << 6;
    pub const ERROR_PRESENT: u8 = 1 << 7;

    pub fn new(bits: u8) -> Self {
        Self(bits)
    }

    pub fn bits(self) -> u8 {
        self.0
    }

    pub fn with(mut self, bit: u8, enabled: bool) -> Self {
        if enabled {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiUsagePacket {
    pub provider: AiUsageProvider,
    pub flags: AiUsageFlags,
    pub five_hour_used_bp: u16,
    pub seven_day_used_bp: u16,
    pub five_hour_reset_unix: u32,
    pub seven_day_reset_unix: u32,
    pub updated_unix: u32,
    pub error_code: AiUsageErrorCode,
}

impl AiUsagePacket {
    pub fn new(
        provider: AiUsageProvider,
        flags: AiUsageFlags,
        five_hour_used_bp: u16,
        seven_day_used_bp: u16,
        five_hour_reset_unix: u32,
        seven_day_reset_unix: u32,
        updated_unix: u32,
        error_code: AiUsageErrorCode,
    ) -> Result<Self, PacketError> {
        if five_hour_used_bp > AI_USAGE_MAX_BASIS_POINTS {
            return Err(PacketError::InvalidAiUsageBasisPoints(five_hour_used_bp));
        }
        if seven_day_used_bp > AI_USAGE_MAX_BASIS_POINTS {
            return Err(PacketError::InvalidAiUsageBasisPoints(seven_day_used_bp));
        }
        Ok(Self {
            provider,
            flags,
            five_hour_used_bp,
            seven_day_used_bp,
            five_hour_reset_unix,
            seven_day_reset_unix,
            updated_unix,
            error_code,
        })
    }

    pub fn encode_payload(self) -> [u8; PACKET_SIZE] {
        let mut bytes = [0u8; PACKET_SIZE];
        bytes[0..2].copy_from_slice(&MAGIC);
        bytes[2] = VERSION;
        bytes[3] = PacketType::AiUsage as u8;
        bytes[4] = self.provider as u8;
        bytes[5] = self.flags.bits();
        bytes[6..8].copy_from_slice(&self.five_hour_used_bp.to_le_bytes());
        bytes[8..10].copy_from_slice(&self.seven_day_used_bp.to_le_bytes());
        bytes[10..14].copy_from_slice(&self.five_hour_reset_unix.to_le_bytes());
        bytes[14..18].copy_from_slice(&self.seven_day_reset_unix.to_le_bytes());
        bytes[18..22].copy_from_slice(&self.updated_unix.to_le_bytes());
        bytes[22] = self.error_code as u8;
        bytes
    }

    pub fn encode_report(self) -> [u8; REPORT_SIZE] {
        let mut report = [0u8; REPORT_SIZE];
        report[1..].copy_from_slice(&self.encode_payload());
        report
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSyncPacket {
    pub unix_time_sec: u32,
    pub tz_offset_min: i16,
    pub weekday: u8,
    pub format_hint: u8,
    pub clock_mode: u8,
}

impl TimeSyncPacket {
    pub fn new(
        unix_time_sec: u32,
        tz_offset_min: i16,
        weekday: u8,
        format_hint: u8,
        clock_mode: u8,
    ) -> Result<Self, PacketError> {
        if !(1..=7).contains(&weekday) {
            return Err(PacketError::InvalidWeekday(weekday));
        }
        if !(-1440..=1440).contains(&tz_offset_min) {
            return Err(PacketError::InvalidTimezoneOffset(tz_offset_min));
        }
        Ok(Self {
            unix_time_sec,
            tz_offset_min,
            weekday,
            format_hint,
            clock_mode,
        })
    }

    pub fn encode_payload(self) -> [u8; PACKET_SIZE] {
        let mut bytes = [0u8; PACKET_SIZE];
        bytes[0..2].copy_from_slice(&MAGIC);
        bytes[2] = VERSION;
        bytes[3] = PacketType::TimeSync as u8;
        bytes[4..8].copy_from_slice(&self.unix_time_sec.to_le_bytes());
        bytes[8..10].copy_from_slice(&self.tz_offset_min.to_le_bytes());
        bytes[10] = self.weekday;
        bytes[11] = self.format_hint;
        bytes[12] = self.clock_mode;
        bytes
    }

    pub fn encode_report(self) -> [u8; REPORT_SIZE] {
        let mut report = [0u8; REPORT_SIZE];
        report[1..].copy_from_slice(&self.encode_payload());
        report
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packet {
    pub packet_type: PacketType,
    pub action: u8,
    pub layer: u8,
    pub seq: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceHello {
    pub protocol_min: u8,
    pub protocol_max: u8,
    pub seq: u8,
    pub capabilities: u32,
    pub device_uid_hash: Option<u64>,
}

impl DeviceHello {
    pub fn decode_payload(bytes: &[u8]) -> Result<Self, PacketError> {
        if bytes.len() != PACKET_SIZE {
            return Err(PacketError::InvalidLength {
                expected: PACKET_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0..2] != MAGIC {
            return Err(PacketError::InvalidMagic);
        }
        if bytes[2] != VERSION {
            return Err(PacketError::UnsupportedVersion(bytes[2]));
        }
        let packet_type = PacketType::try_from(bytes[3])?;
        if packet_type != PacketType::DeviceHello {
            return Err(PacketError::DecodeUnsupportedType(bytes[3]));
        }
        if bytes[6] != 0 || bytes[20..].iter().any(|b| *b != 0) {
            return Err(PacketError::ReservedNotZero);
        }

        let capabilities = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let raw_uid = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
        Ok(Self {
            protocol_min: bytes[4],
            protocol_max: bytes[5],
            seq: bytes[7],
            capabilities,
            device_uid_hash: (raw_uid != 0).then_some(raw_uid),
        })
    }

    pub fn supports_app_layer(self) -> bool {
        self.capabilities & CAPABILITY_APP_LAYER != 0
    }
}

// ─── Device-initiated (uplink) packets ───────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryEntry {
    /// 0 = self/dongle (central), 1 = left peripheral, 2 = right peripheral, 3 = aux.
    pub source: u8,
    /// 0..=100; None when the device reported 0xFF (unknown / disconnected).
    pub level: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatteryStatusPacket {
    pub entries: Vec<BatteryEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostActionPacket {
    pub action_id: u8,
    pub value: u8,
    pub seq: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyStatsEntry {
    pub position: u8,
    pub delta: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyStatsPacket {
    pub entries: Vec<KeyStatsEntry>,
    pub more_follows: bool,
    pub seq: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerStatePacket {
    pub active_layer: u8,
    /// Bit i set = layer i active. 0 means the firmware reports the top layer only.
    pub layer_mask: u32,
    pub seq: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPressPacket {
    pub position: u8,
    pub pressed: bool,
    pub seq: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UplinkPacket {
    Battery(BatteryStatusPacket),
    HostAction(HostActionPacket),
    KeyStats(KeyStatsPacket),
    LayerState(LayerStatePacket),
    KeyPress(KeyPressPacket),
}

impl UplinkPacket {
    pub fn packet_type(&self) -> PacketType {
        match self {
            Self::Battery(_) => PacketType::BatteryStatus,
            Self::HostAction(_) => PacketType::HostAction,
            Self::KeyStats(_) => PacketType::KeyStats,
            Self::LayerState(_) => PacketType::LayerState,
            Self::KeyPress(_) => PacketType::KeyPress,
        }
    }

    pub fn required_capability(&self) -> u32 {
        match self {
            Self::Battery(_) => CAPABILITY_BATTERY,
            Self::HostAction(_) => CAPABILITY_HOST_ACTION,
            Self::KeyStats(_) => CAPABILITY_KEY_STATS,
            Self::LayerState(_) => CAPABILITY_LAYER_STATE,
            Self::KeyPress(_) => CAPABILITY_KEY_PRESS,
        }
    }

    pub fn decode_payload(bytes: &[u8]) -> Result<Self, PacketError> {
        if bytes.len() != PACKET_SIZE {
            return Err(PacketError::InvalidLength {
                expected: PACKET_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0..2] != MAGIC {
            return Err(PacketError::InvalidMagic);
        }
        if bytes[2] != VERSION {
            return Err(PacketError::UnsupportedVersion(bytes[2]));
        }
        match PacketType::try_from(bytes[3])? {
            PacketType::BatteryStatus => Self::decode_battery(bytes),
            PacketType::HostAction => Self::decode_host_action(bytes),
            PacketType::KeyStats => Self::decode_key_stats(bytes),
            PacketType::LayerState => Self::decode_layer_state(bytes),
            PacketType::KeyPress => Self::decode_key_press(bytes),
            other => Err(PacketError::DecodeUnsupportedType(other as u8)),
        }
    }

    fn decode_battery(bytes: &[u8]) -> Result<Self, PacketError> {
        let count = bytes[4] as usize;
        if !(1..=4).contains(&count) {
            return Err(PacketError::InvalidBatteryCount(bytes[4]));
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let source = bytes[5 + 2 * i];
            let raw_level = bytes[6 + 2 * i];
            if source > 3 {
                return Err(PacketError::InvalidBatterySource(source));
            }
            if entries.iter().any(|e: &BatteryEntry| e.source == source) {
                return Err(PacketError::DuplicateBatterySource(source));
            }
            let level = match raw_level {
                0..=100 => Some(raw_level),
                BATTERY_LEVEL_UNKNOWN => None,
                other => return Err(PacketError::InvalidBatteryLevel(other)),
            };
            entries.push(BatteryEntry { source, level });
        }
        if bytes[5 + 2 * count..].iter().any(|b| *b != 0) {
            return Err(PacketError::ReservedNotZero);
        }
        Ok(Self::Battery(BatteryStatusPacket { entries }))
    }

    fn decode_host_action(bytes: &[u8]) -> Result<Self, PacketError> {
        if bytes[6] != 0 || bytes[8..].iter().any(|b| *b != 0) {
            return Err(PacketError::ReservedNotZero);
        }
        Ok(Self::HostAction(HostActionPacket {
            action_id: bytes[4],
            value: bytes[5],
            seq: bytes[7],
        }))
    }

    fn decode_key_stats(bytes: &[u8]) -> Result<Self, PacketError> {
        let count = bytes[4] as usize;
        if !(1..=KEY_STATS_MAX_ENTRIES).contains(&count) {
            return Err(PacketError::InvalidKeyStatsEntryCount(bytes[4]));
        }
        let flags = bytes[5];
        if flags & !KEY_STATS_FLAG_MORE_FOLLOWS != 0 {
            return Err(PacketError::InvalidKeyStatsFlags(flags));
        }
        if bytes[6] != 0 {
            return Err(PacketError::ReservedNotZero);
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let base = 8 + 3 * i;
            let position = bytes[base];
            let delta = u16::from_le_bytes(bytes[base + 1..base + 3].try_into().unwrap());
            if delta == 0 {
                return Err(PacketError::InvalidKeyStatsDelta(position));
            }
            entries.push(KeyStatsEntry { position, delta });
        }
        if bytes[8 + 3 * count..].iter().any(|b| *b != 0) {
            return Err(PacketError::ReservedNotZero);
        }
        Ok(Self::KeyStats(KeyStatsPacket {
            entries,
            more_follows: flags & KEY_STATS_FLAG_MORE_FOLLOWS != 0,
            seq: bytes[7],
        }))
    }

    fn decode_layer_state(bytes: &[u8]) -> Result<Self, PacketError> {
        if bytes[5] != 0 || bytes[6] != 0 || bytes[12..].iter().any(|b| *b != 0) {
            return Err(PacketError::ReservedNotZero);
        }
        let active_layer = bytes[4];
        validate_layer(active_layer)?;
        let layer_mask = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        if layer_mask != 0 && layer_mask & (1 << active_layer) == 0 {
            return Err(PacketError::InvalidLayerMask {
                active_layer,
                layer_mask,
            });
        }
        Ok(Self::LayerState(LayerStatePacket {
            active_layer,
            layer_mask,
            seq: bytes[7],
        }))
    }

    fn decode_key_press(bytes: &[u8]) -> Result<Self, PacketError> {
        let flags = bytes[5];
        if flags & !KEY_PRESS_FLAG_PRESSED != 0
            || bytes[6] != 0
            || bytes[8..].iter().any(|b| *b != 0)
        {
            return Err(PacketError::ReservedNotZero);
        }
        Ok(Self::KeyPress(KeyPressPacket {
            position: bytes[4],
            pressed: flags & KEY_PRESS_FLAG_PRESSED != 0,
            seq: bytes[7],
        }))
    }

    /// Encode for round-trip tests and debug injection. Not used in the
    /// production send path (these packets are device-initiated).
    pub fn encode_payload(&self) -> [u8; PACKET_SIZE] {
        let mut bytes = [0u8; PACKET_SIZE];
        bytes[0..2].copy_from_slice(&MAGIC);
        bytes[2] = VERSION;
        bytes[3] = self.packet_type() as u8;
        match self {
            Self::Battery(p) => {
                bytes[4] = p.entries.len() as u8;
                for (i, entry) in p.entries.iter().enumerate() {
                    bytes[5 + 2 * i] = entry.source;
                    bytes[6 + 2 * i] = entry.level.unwrap_or(BATTERY_LEVEL_UNKNOWN);
                }
            }
            Self::HostAction(p) => {
                bytes[4] = p.action_id;
                bytes[5] = p.value;
                bytes[7] = p.seq;
            }
            Self::KeyStats(p) => {
                bytes[4] = p.entries.len() as u8;
                bytes[5] = if p.more_follows {
                    KEY_STATS_FLAG_MORE_FOLLOWS
                } else {
                    0
                };
                bytes[7] = p.seq;
                for (i, entry) in p.entries.iter().enumerate() {
                    let base = 8 + 3 * i;
                    bytes[base] = entry.position;
                    bytes[base + 1..base + 3].copy_from_slice(&entry.delta.to_le_bytes());
                }
            }
            Self::LayerState(p) => {
                bytes[4] = p.active_layer;
                bytes[7] = p.seq;
                bytes[8..12].copy_from_slice(&p.layer_mask.to_le_bytes());
            }
            Self::KeyPress(p) => {
                bytes[4] = p.position;
                bytes[5] = if p.pressed { KEY_PRESS_FLAG_PRESSED } else { 0 };
                bytes[7] = p.seq;
            }
        }
        bytes
    }
}

impl Packet {
    pub fn set_layer(layer: u8, seq: u8) -> Result<Self, PacketError> {
        validate_layer(layer)?;
        Ok(Self {
            packet_type: PacketType::AppLayer,
            action: AppLayerAction::Set as u8,
            layer,
            seq,
        })
    }

    pub fn clear(seq: u8) -> Self {
        Self {
            packet_type: PacketType::AppLayer,
            action: AppLayerAction::Clear as u8,
            layer: 0,
            seq,
        }
    }

    pub fn host_hello(seq: u8) -> Self {
        Self {
            packet_type: PacketType::HostHello,
            action: 0,
            layer: 0,
            seq,
        }
    }

    pub fn encode_payload(self) -> [u8; PACKET_SIZE] {
        let mut bytes = [0u8; PACKET_SIZE];
        bytes[0..2].copy_from_slice(&MAGIC);
        bytes[2] = VERSION;
        bytes[3] = self.packet_type as u8;
        match self.packet_type {
            PacketType::HostHello
            | PacketType::DeviceHello
            | PacketType::Ping
            | PacketType::Pong => {
                bytes[7] = self.seq;
            }
            PacketType::AppLayer => {
                bytes[4] = self.action;
                bytes[5] = self.layer;
                bytes[7] = self.seq;
            }
            PacketType::Error
            | PacketType::AiUsage
            | PacketType::TimeSync
            | PacketType::BatteryStatus
            | PacketType::HostAction
            | PacketType::KeyStats
            | PacketType::LayerState
            | PacketType::KeyPress => {}
        }
        bytes
    }

    pub fn encode_report(self) -> [u8; REPORT_SIZE] {
        let mut report = [0u8; REPORT_SIZE];
        report[1..].copy_from_slice(&self.encode_payload());
        report
    }

    pub fn decode_payload(bytes: &[u8]) -> Result<Self, PacketError> {
        if bytes.len() != PACKET_SIZE {
            return Err(PacketError::InvalidLength {
                expected: PACKET_SIZE,
                actual: bytes.len(),
            });
        }
        if bytes[0..2] != MAGIC {
            return Err(PacketError::InvalidMagic);
        }
        if bytes[2] != VERSION {
            return Err(PacketError::UnsupportedVersion(bytes[2]));
        }
        let packet_type = PacketType::try_from(bytes[3])?;
        match packet_type {
            PacketType::HostHello | PacketType::Ping | PacketType::Pong => {
                if bytes[4..7].iter().any(|b| *b != 0) || bytes[8..].iter().any(|b| *b != 0) {
                    return Err(PacketError::ReservedNotZero);
                }
                Ok(Self {
                    packet_type,
                    action: 0,
                    layer: 0,
                    seq: bytes[7],
                })
            }
            PacketType::DeviceHello => {
                let hello = DeviceHello::decode_payload(bytes)?;
                Ok(Self {
                    packet_type,
                    action: 0,
                    layer: 0,
                    seq: hello.seq,
                })
            }
            PacketType::AppLayer => {
                if bytes[6] != 0 || bytes[8..].iter().any(|b| *b != 0) {
                    return Err(PacketError::ReservedNotZero);
                }
                let action = AppLayerAction::try_from(bytes[4])?;
                let layer = bytes[5];
                match action {
                    AppLayerAction::Set => validate_layer(layer)?,
                    AppLayerAction::Clear if layer != 0 => {
                        return Err(PacketError::InvalidClearLayer(layer));
                    }
                    AppLayerAction::Clear => {}
                }
                Ok(Self {
                    packet_type,
                    action: bytes[4],
                    layer,
                    seq: bytes[7],
                })
            }
            PacketType::Error
            | PacketType::AiUsage
            | PacketType::TimeSync
            | PacketType::BatteryStatus
            | PacketType::HostAction
            | PacketType::KeyStats
            | PacketType::LayerState
            | PacketType::KeyPress => Err(PacketError::DecodeUnsupportedType(packet_type as u8)),
        }
    }

    pub fn is_matching_device_hello(self, seq: u8) -> bool {
        self.packet_type == PacketType::DeviceHello && self.seq == seq
    }
}

fn validate_layer(layer: u8) -> Result<(), PacketError> {
    if layer > MAX_LAYER {
        return Err(PacketError::InvalidLayer(layer));
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PacketError {
    #[error("invalid packet length: expected {expected}, got {actual}")]
    InvalidLength { expected: usize, actual: usize },
    #[error("invalid packet magic")]
    InvalidMagic,
    #[error("unsupported packet version {0}")]
    UnsupportedVersion(u8),
    #[error("unknown packet type {0:#04x}")]
    UnknownType(u8),
    #[error("invalid layer {0}; expected 0-31")]
    InvalidLayer(u8),
    #[error("invalid app layer action {0}; expected 1=set or 2=clear")]
    InvalidAppLayerAction(u8),
    #[error("invalid clear layer {0}; expected 0")]
    InvalidClearLayer(u8),
    #[error("reserved bytes must be zero")]
    ReservedNotZero,
    #[error("invalid weekday {0}; expected 1-7")]
    InvalidWeekday(u8),
    #[error("invalid timezone offset {0}; expected -1440..=1440")]
    InvalidTimezoneOffset(i16),
    #[error("invalid AI usage basis points {0}; expected 0..=10000")]
    InvalidAiUsageBasisPoints(u16),
    #[error("packet type {0:#04x} is not decoded as a generic packet")]
    DecodeUnsupportedType(u8),
    #[error("invalid battery entry count {0}; expected 1-4")]
    InvalidBatteryCount(u8),
    #[error("invalid battery source {0}; expected 0-3")]
    InvalidBatterySource(u8),
    #[error("invalid battery level {0}; expected 0-100 or 255")]
    InvalidBatteryLevel(u8),
    #[error("duplicate battery source {0}")]
    DuplicateBatterySource(u8),
    #[error("invalid key stats entry count {0}; expected 1-8")]
    InvalidKeyStatsEntryCount(u8),
    #[error("invalid key stats flags {0:#04x}")]
    InvalidKeyStatsFlags(u8),
    #[error("key stats delta for position {0} must be non-zero")]
    InvalidKeyStatsDelta(u8),
    #[error("layer mask {layer_mask:#010x} does not include active layer {active_layer}")]
    InvalidLayerMask { active_layer: u8, layer_mask: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_report_with_zero_report_id() {
        let report = Packet::set_layer(3, 7).unwrap().encode_report();

        assert_eq!(report.len(), REPORT_SIZE);
        assert_eq!(report[0], 0);
        assert_eq!(&report[1..3], b"HL");
        assert_eq!(report[3], VERSION);
        assert_eq!(report[4], PacketType::AppLayer as u8);
        assert_eq!(report[5], AppLayerAction::Set as u8);
        assert_eq!(report[6], 3);
        assert_eq!(report[8], 7);
        assert!(report[9..].iter().all(|b| *b == 0));
    }

    #[test]
    fn decodes_device_hello_capabilities_and_uid() {
        let mut payload = Packet::host_hello(9).encode_payload();
        payload[3] = PacketType::DeviceHello as u8;
        payload[4] = 1;
        payload[5] = 1;
        payload[8..12]
            .copy_from_slice(&(CAPABILITY_APP_LAYER | CAPABILITY_TIME_SYNC).to_le_bytes());
        payload[12..20].copy_from_slice(&0x7a91c3e4d102ab55u64.to_le_bytes());

        let hello = DeviceHello::decode_payload(&payload).unwrap();

        assert_eq!(hello.protocol_min, 1);
        assert_eq!(hello.protocol_max, 1);
        assert_eq!(hello.seq, 9);
        assert_eq!(
            hello.capabilities,
            CAPABILITY_APP_LAYER | CAPABILITY_TIME_SYNC
        );
        assert_eq!(hello.device_uid_hash, Some(0x7a91c3e4d102ab55));
        assert!(hello.supports_app_layer());
    }

    #[test]
    fn device_hello_zero_uid_is_normalized_to_none() {
        let mut payload = Packet::host_hello(3).encode_payload();
        payload[3] = PacketType::DeviceHello as u8;
        payload[8..12].copy_from_slice(&CAPABILITY_APP_LAYER.to_le_bytes());

        let hello = DeviceHello::decode_payload(&payload).unwrap();

        assert_eq!(hello.device_uid_hash, None);
    }

    #[test]
    fn decodes_host_hello_payload() {
        let payload = Packet::host_hello(42).encode_payload();
        let packet = Packet::decode_payload(&payload).unwrap();

        assert_eq!(packet.packet_type, PacketType::HostHello);
        assert_eq!(packet.seq, 42);
    }

    #[test]
    fn decodes_app_layer_payloads() {
        let set = Packet::set_layer(4, 9).unwrap();
        let decoded = Packet::decode_payload(&set.encode_payload()).unwrap();
        assert_eq!(decoded.packet_type, PacketType::AppLayer);
        assert_eq!(decoded.action, AppLayerAction::Set as u8);
        assert_eq!(decoded.layer, 4);
        assert_eq!(decoded.seq, 9);

        let clear = Packet::clear(10);
        let decoded = Packet::decode_payload(&clear.encode_payload()).unwrap();
        assert_eq!(decoded.action, AppLayerAction::Clear as u8);
        assert_eq!(decoded.layer, 0);
        assert_eq!(decoded.seq, 10);
    }

    #[test]
    fn rejects_invalid_layer() {
        assert_eq!(
            Packet::set_layer(32, 0).unwrap_err(),
            PacketError::InvalidLayer(32)
        );
    }

    #[test]
    fn rejects_non_zero_reserved_bytes() {
        let mut payload = Packet::clear(1).encode_payload();
        payload[31] = 1;

        assert_eq!(
            Packet::decode_payload(&payload).unwrap_err(),
            PacketError::ReservedNotZero
        );
    }

    #[test]
    fn rejects_invalid_app_layer_fields() {
        let mut payload = Packet::clear(1).encode_payload();
        payload[4] = 9;
        assert_eq!(
            Packet::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidAppLayerAction(9)
        );

        let mut payload = Packet::clear(1).encode_payload();
        payload[5] = 1;
        assert_eq!(
            Packet::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidClearLayer(1)
        );
    }

    #[test]
    fn encodes_time_sync_report() {
        let report = TimeSyncPacket::new(0x12345678, 540, 6, 4, 0)
            .unwrap()
            .encode_report();

        assert_eq!(report.len(), REPORT_SIZE);
        assert_eq!(report[0], 0);
        assert_eq!(&report[1..3], b"HL");
        assert_eq!(report[3], VERSION);
        assert_eq!(report[4], PacketType::TimeSync as u8);
        assert_eq!(&report[5..9], &0x12345678u32.to_le_bytes());
        assert_eq!(&report[9..11], &540i16.to_le_bytes());
        assert_eq!(report[11], 6);
        assert_eq!(report[12], 4);
        assert_eq!(report[13], 0);
        assert!(report[14..].iter().all(|b| *b == 0));
    }

    #[test]
    fn encodes_ai_usage_report() {
        let flags = AiUsageFlags::default()
            .with(AiUsageFlags::FIVE_HOUR_VALID, true)
            .with(AiUsageFlags::QUOTA_SOURCE, true);
        let report = AiUsagePacket::new(
            AiUsageProvider::Codex,
            flags,
            1234,
            0,
            1_700_000_000,
            0,
            1_699_999_900,
            AiUsageErrorCode::None,
        )
        .unwrap()
        .encode_report();

        assert_eq!(report[0], 0);
        assert_eq!(&report[1..3], b"HL");
        assert_eq!(report[3], VERSION);
        assert_eq!(report[4], PacketType::AiUsage as u8);
        assert_eq!(report[5], AiUsageProvider::Codex as u8);
        assert_eq!(report[6], flags.bits());
        assert_eq!(&report[7..9], &1234u16.to_le_bytes());
        assert!(report[24..].iter().all(|b| *b == 0));
    }

    #[test]
    fn rejects_invalid_time_sync_fields() {
        assert_eq!(
            TimeSyncPacket::new(0, 0, 0, 0, 0).unwrap_err(),
            PacketError::InvalidWeekday(0)
        );
        assert_eq!(
            TimeSyncPacket::new(0, 1441, 1, 0, 0).unwrap_err(),
            PacketError::InvalidTimezoneOffset(1441)
        );
    }

    #[test]
    fn battery_status_round_trips() {
        let packet = UplinkPacket::Battery(BatteryStatusPacket {
            entries: vec![
                BatteryEntry {
                    source: 0,
                    level: Some(87),
                },
                BatteryEntry {
                    source: 1,
                    level: Some(42),
                },
                BatteryEntry {
                    source: 2,
                    level: None,
                },
            ],
        });

        let decoded = UplinkPacket::decode_payload(&packet.encode_payload()).unwrap();
        assert_eq!(decoded, packet);
        assert_eq!(decoded.required_capability(), CAPABILITY_BATTERY);
    }

    #[test]
    fn battery_status_rejects_invalid_fields() {
        let base = UplinkPacket::Battery(BatteryStatusPacket {
            entries: vec![BatteryEntry {
                source: 0,
                level: Some(50),
            }],
        })
        .encode_payload();

        let mut payload = base;
        payload[4] = 0;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidBatteryCount(0)
        );

        let mut payload = base;
        payload[4] = 5;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidBatteryCount(5)
        );

        let mut payload = base;
        payload[6] = 101;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidBatteryLevel(101)
        );

        let mut payload = base;
        payload[5] = 4;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidBatterySource(4)
        );

        // Two entries with the same source.
        let mut payload = base;
        payload[4] = 2;
        payload[7] = 0; // source duplicate of entry 0
        payload[8] = 60;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::DuplicateBatterySource(0)
        );

        let mut payload = base;
        payload[31] = 1;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::ReservedNotZero
        );
    }

    #[test]
    fn host_action_round_trips() {
        let packet = UplinkPacket::HostAction(HostActionPacket {
            action_id: 7,
            value: 3,
            seq: 200,
        });

        let decoded = UplinkPacket::decode_payload(&packet.encode_payload()).unwrap();
        assert_eq!(decoded, packet);
        assert_eq!(decoded.required_capability(), CAPABILITY_HOST_ACTION);
    }

    #[test]
    fn host_action_rejects_reserved_bytes() {
        let mut payload = UplinkPacket::HostAction(HostActionPacket {
            action_id: 1,
            value: 0,
            seq: 0,
        })
        .encode_payload();
        payload[8] = 1;

        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::ReservedNotZero
        );
    }

    #[test]
    fn key_stats_round_trips_with_full_packet() {
        let entries: Vec<KeyStatsEntry> = (0..KEY_STATS_MAX_ENTRIES as u8)
            .map(|i| KeyStatsEntry {
                position: 10 + i,
                delta: 100 + u16::from(i),
            })
            .collect();
        let packet = UplinkPacket::KeyStats(KeyStatsPacket {
            entries,
            more_follows: true,
            seq: 9,
        });

        let payload = packet.encode_payload();
        // 8 entries * 3 bytes fill bytes 8..32 exactly.
        assert!(payload[8..].iter().any(|b| *b != 0));
        let decoded = UplinkPacket::decode_payload(&payload).unwrap();
        assert_eq!(decoded, packet);
    }

    #[test]
    fn key_stats_rejects_invalid_fields() {
        let base = UplinkPacket::KeyStats(KeyStatsPacket {
            entries: vec![KeyStatsEntry {
                position: 4,
                delta: 2,
            }],
            more_follows: false,
            seq: 1,
        })
        .encode_payload();

        let mut payload = base;
        payload[4] = 0;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidKeyStatsEntryCount(0)
        );

        let mut payload = base;
        payload[4] = 9;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidKeyStatsEntryCount(9)
        );

        let mut payload = base;
        payload[5] = 0x02;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidKeyStatsFlags(0x02)
        );

        let mut payload = base;
        payload[9] = 0;
        payload[10] = 0;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidKeyStatsDelta(4)
        );

        let mut payload = base;
        payload[31] = 1;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::ReservedNotZero
        );
    }

    #[test]
    fn layer_state_round_trips() {
        let packet = UplinkPacket::LayerState(LayerStatePacket {
            active_layer: 3,
            layer_mask: 0b1001,
            seq: 17,
        });

        let decoded = UplinkPacket::decode_payload(&packet.encode_payload()).unwrap();
        assert_eq!(decoded, packet);
        assert_eq!(decoded.required_capability(), CAPABILITY_LAYER_STATE);
    }

    #[test]
    fn layer_state_allows_zero_mask() {
        let packet = UplinkPacket::LayerState(LayerStatePacket {
            active_layer: 5,
            layer_mask: 0,
            seq: 0,
        });

        assert_eq!(
            UplinkPacket::decode_payload(&packet.encode_payload()).unwrap(),
            packet
        );
    }

    #[test]
    fn layer_state_rejects_invalid_fields() {
        let base = UplinkPacket::LayerState(LayerStatePacket {
            active_layer: 2,
            layer_mask: 0b0100,
            seq: 0,
        })
        .encode_payload();

        let mut payload = base;
        payload[4] = 32;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidLayer(32)
        );

        // Mask set but missing the active layer bit.
        let mut payload = base;
        payload[8..12].copy_from_slice(&0b0010u32.to_le_bytes());
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidLayerMask {
                active_layer: 2,
                layer_mask: 0b0010
            }
        );

        let mut payload = base;
        payload[12] = 1;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::ReservedNotZero
        );
    }

    #[test]
    fn uplink_decode_rejects_downlink_types() {
        let payload = Packet::host_hello(1).encode_payload();
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::DecodeUnsupportedType(PacketType::HostHello as u8)
        );
    }
}
