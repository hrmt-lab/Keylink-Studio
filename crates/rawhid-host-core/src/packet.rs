use thiserror::Error;

pub const PACKET_SIZE: usize = 64;
pub const REPORT_SIZE: usize = PACKET_SIZE + 1;
pub const MAGIC: [u8; 2] = *b"HL";
pub const VERSION: u8 = 0x02;
pub const HEADER_SIZE: usize = 12;
pub const PAYLOAD_OFFSET: usize = HEADER_SIZE;
pub const MAX_PAYLOAD_LEN: usize = PACKET_SIZE - HEADER_SIZE;
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
pub const CAPABILITY_CONFIG_RPC: u32 = 1 << 9;

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
    ConfigRequest = 0x90,
    ConfigResponse = 0x91,
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
            0x90 => Ok(Self::ConfigRequest),
            0x91 => Ok(Self::ConfigResponse),
            other => Err(PacketError::UnknownType(other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CommonHeader {
    packet_type: PacketType,
    seq: u8,
    feature: u8,
    op: u8,
    status_or_flags: u8,
    payload_len: u8,
}

fn encode_header(bytes: &mut [u8; PACKET_SIZE], header: CommonHeader) {
    bytes[0..2].copy_from_slice(&MAGIC);
    bytes[2] = VERSION;
    bytes[3] = header.packet_type as u8;
    bytes[4] = header.seq;
    bytes[5] = header.feature;
    bytes[6] = header.op;
    bytes[7] = header.status_or_flags;
    bytes[8] = header.payload_len;
    bytes[9..12].fill(0);
}

fn decode_header(bytes: &[u8]) -> Result<CommonHeader, PacketError> {
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
    let payload_len = bytes[8];
    if payload_len as usize > MAX_PAYLOAD_LEN {
        return Err(PacketError::InvalidPayloadLength {
            max: MAX_PAYLOAD_LEN,
            actual: payload_len,
        });
    }
    if bytes[9..12].iter().any(|b| *b != 0) {
        return Err(PacketError::ReservedNotZero);
    }
    validate_payload_padding(bytes, payload_len as usize)?;
    Ok(CommonHeader {
        packet_type,
        seq: bytes[4],
        feature: bytes[5],
        op: bytes[6],
        status_or_flags: bytes[7],
        payload_len,
    })
}

fn validate_payload_padding(bytes: &[u8], payload_len: usize) -> Result<(), PacketError> {
    if bytes[PAYLOAD_OFFSET + payload_len..]
        .iter()
        .any(|b| *b != 0)
    {
        return Err(PacketError::ReservedNotZero);
    }
    Ok(())
}

fn payload(bytes: &[u8], expected_len: usize) -> Result<&[u8], PacketError> {
    let header = decode_header(bytes)?;
    if header.payload_len as usize != expected_len {
        return Err(PacketError::InvalidPayloadLength {
            max: expected_len,
            actual: header.payload_len,
        });
    }
    Ok(&bytes[PAYLOAD_OFFSET..PAYLOAD_OFFSET + expected_len])
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ConfigFeature {
    Encoder = 0x01,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ConfigOp {
    GetInfo = 0x01,
    GetBindings = 0x02,
    SetBindings = 0x03,
    GetDirty = 0x04,
    Save = 0x05,
    Discard = 0x06,
    ClearOverride = 0x07,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ConfigStatus {
    Ok = 0x00,
    BadPacket = 0x01,
    UnsupportedFeature = 0x02,
    UnsupportedOp = 0x03,
    InvalidArgument = 0x04,
    Busy = 0x05,
    NotFound = 0x06,
    StorageError = 0x07,
    InternalError = 0x08,
}

impl TryFrom<u8> for ConfigStatus {
    type Error = PacketError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(Self::Ok),
            0x01 => Ok(Self::BadPacket),
            0x02 => Ok(Self::UnsupportedFeature),
            0x03 => Ok(Self::UnsupportedOp),
            0x04 => Ok(Self::InvalidArgument),
            0x05 => Ok(Self::Busy),
            0x06 => Ok(Self::NotFound),
            0x07 => Ok(Self::StorageError),
            0x08 => Ok(Self::InternalError),
            other => Err(PacketError::InvalidConfigStatus(other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderGetInfo {
    pub layer_count: u8,
    pub encoder_count: u8,
    pub capabilities: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderBinding {
    pub behavior_id: u16,
    pub param1: u32,
    pub param2: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EncoderBindingSource {
    Keymap = 0x00,
    Override = 0x01,
}

impl TryFrom<u8> for EncoderBindingSource {
    type Error = PacketError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(Self::Keymap),
            0x01 => Ok(Self::Override),
            other => Err(PacketError::InvalidEncoderBindingSource(other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EncoderBindingFlags(u8);

impl EncoderBindingFlags {
    pub const STALE_SAVED_EXISTS: u8 = 1 << 0;
    pub const SAVED_EXISTS: u8 = 1 << 1;
    pub const RUNTIME_DIRTY: u8 = 1 << 2;
    pub const INVALID_SAVED_EXISTS: u8 = 1 << 3;
    pub const VALID_MASK: u8 = Self::STALE_SAVED_EXISTS
        | Self::SAVED_EXISTS
        | Self::RUNTIME_DIRTY
        | Self::INVALID_SAVED_EXISTS;

    pub fn new(bits: u8) -> Result<Self, PacketError> {
        if bits & !Self::VALID_MASK != 0 {
            return Err(PacketError::InvalidEncoderBindingFlags(bits));
        }
        Ok(Self(bits))
    }

    pub fn bits(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderGetBindings {
    pub layer_id: u32,
    pub encoder_id: u8,
    pub source: EncoderBindingSource,
    pub flags: EncoderBindingFlags,
    pub cw_binding: EncoderBinding,
    pub ccw_binding: EncoderBinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigRequest {
    pub seq: u8,
    pub feature: u8,
    pub op: u8,
    payload_len: u8,
    payload: [u8; MAX_PAYLOAD_LEN],
}

impl ConfigRequest {
    pub fn encoder_get_info(seq: u8) -> Self {
        Self {
            seq,
            feature: ConfigFeature::Encoder as u8,
            op: ConfigOp::GetInfo as u8,
            payload_len: 0,
            payload: [0; MAX_PAYLOAD_LEN],
        }
    }

    pub fn encoder_get_bindings(seq: u8, layer_id: u32, encoder_id: u8) -> Self {
        let mut payload = [0u8; MAX_PAYLOAD_LEN];
        payload[0..4].copy_from_slice(&layer_id.to_le_bytes());
        payload[4] = encoder_id;
        Self {
            seq,
            feature: ConfigFeature::Encoder as u8,
            op: ConfigOp::GetBindings as u8,
            payload_len: 5,
            payload,
        }
    }

    /// Requested (layer_id, encoder_id) when this is an ENCODER GET_BINDINGS
    /// request; used by `ConfigResponse::is_response_to` to verify that an OK
    /// response echoes the same target.
    pub fn encoder_bindings_target(&self) -> Option<(u32, u8)> {
        if self.feature != ConfigFeature::Encoder as u8
            || self.op != ConfigOp::GetBindings as u8
            || self.payload_len < 5
        {
            return None;
        }
        let layer_id = u32::from_le_bytes(self.payload[0..4].try_into().expect("4-byte slice"));
        Some((layer_id, self.payload[4]))
    }

    pub fn encoder_set_bindings(
        seq: u8,
        layer_id: u32,
        encoder_id: u8,
        cw_binding: EncoderBinding,
        ccw_binding: EncoderBinding,
    ) -> Self {
        let mut payload = [0u8; MAX_PAYLOAD_LEN];
        payload[0..4].copy_from_slice(&layer_id.to_le_bytes());
        payload[4] = encoder_id;
        payload[5] = 0x03;
        payload[6] = 0;
        payload[7] = 0;
        encode_encoder_binding(&mut payload[8..18], cw_binding);
        encode_encoder_binding(&mut payload[18..28], ccw_binding);
        Self {
            seq,
            feature: ConfigFeature::Encoder as u8,
            op: ConfigOp::SetBindings as u8,
            payload_len: 28,
            payload,
        }
    }

    pub fn encoder_get_dirty(seq: u8) -> Self {
        Self {
            seq,
            feature: ConfigFeature::Encoder as u8,
            op: ConfigOp::GetDirty as u8,
            payload_len: 0,
            payload: [0; MAX_PAYLOAD_LEN],
        }
    }

    pub fn encoder_save(seq: u8) -> Self {
        Self {
            seq,
            feature: ConfigFeature::Encoder as u8,
            op: ConfigOp::Save as u8,
            payload_len: 0,
            payload: [0; MAX_PAYLOAD_LEN],
        }
    }

    pub fn encoder_discard(seq: u8) -> Self {
        Self {
            seq,
            feature: ConfigFeature::Encoder as u8,
            op: ConfigOp::Discard as u8,
            payload_len: 0,
            payload: [0; MAX_PAYLOAD_LEN],
        }
    }

    pub fn encoder_clear_override(seq: u8, layer_id: u32, encoder_id: u8) -> Self {
        let mut payload = [0u8; MAX_PAYLOAD_LEN];
        payload[0..4].copy_from_slice(&layer_id.to_le_bytes());
        payload[4] = encoder_id;
        Self {
            seq,
            feature: ConfigFeature::Encoder as u8,
            op: ConfigOp::ClearOverride as u8,
            payload_len: 5,
            payload,
        }
    }

    pub fn encode_payload(self) -> [u8; PACKET_SIZE] {
        let mut bytes = [0u8; PACKET_SIZE];
        encode_header(
            &mut bytes,
            CommonHeader {
                packet_type: PacketType::ConfigRequest,
                seq: self.seq,
                feature: self.feature,
                op: self.op,
                status_or_flags: 0,
                payload_len: self.payload_len,
            },
        );
        let payload_len = self.payload_len as usize;
        bytes[PAYLOAD_OFFSET..PAYLOAD_OFFSET + payload_len]
            .copy_from_slice(&self.payload[..payload_len]);
        bytes
    }

    pub fn encode_report(self) -> [u8; REPORT_SIZE] {
        let mut report = [0u8; REPORT_SIZE];
        report[1..].copy_from_slice(&self.encode_payload());
        report
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigResponse {
    pub seq: u8,
    pub feature: u8,
    pub op: u8,
    pub status: ConfigStatus,
    pub encoder_get_info: Option<EncoderGetInfo>,
    pub encoder_get_bindings: Option<EncoderGetBindings>,
    pub encoder_get_dirty: Option<bool>,
}

impl ConfigResponse {
    pub fn encoder_get_info_ok(seq: u8, info: EncoderGetInfo) -> Self {
        Self {
            seq,
            feature: ConfigFeature::Encoder as u8,
            op: ConfigOp::GetInfo as u8,
            status: ConfigStatus::Ok,
            encoder_get_info: Some(info),
            encoder_get_bindings: None,
            encoder_get_dirty: None,
        }
    }

    pub fn encoder_get_bindings_ok(seq: u8, bindings: EncoderGetBindings) -> Self {
        Self {
            seq,
            feature: ConfigFeature::Encoder as u8,
            op: ConfigOp::GetBindings as u8,
            status: ConfigStatus::Ok,
            encoder_get_info: None,
            encoder_get_bindings: Some(bindings),
            encoder_get_dirty: None,
        }
    }

    pub fn encoder_get_dirty_ok(seq: u8, dirty: bool) -> Self {
        Self {
            seq,
            feature: ConfigFeature::Encoder as u8,
            op: ConfigOp::GetDirty as u8,
            status: ConfigStatus::Ok,
            encoder_get_info: None,
            encoder_get_bindings: None,
            encoder_get_dirty: Some(dirty),
        }
    }

    pub fn status(seq: u8, feature: u8, op: u8, status: ConfigStatus) -> Self {
        Self {
            seq,
            feature,
            op,
            status,
            encoder_get_info: None,
            encoder_get_bindings: None,
            encoder_get_dirty: None,
        }
    }

    pub fn encode_payload(self) -> [u8; PACKET_SIZE] {
        let mut bytes = [0u8; PACKET_SIZE];
        let payload_len = if self.status == ConfigStatus::Ok
            && self.feature == ConfigFeature::Encoder as u8
            && self.op == ConfigOp::GetInfo as u8
        {
            4
        } else if self.status == ConfigStatus::Ok
            && self.feature == ConfigFeature::Encoder as u8
            && self.op == ConfigOp::GetBindings as u8
        {
            28
        } else if self.status == ConfigStatus::Ok
            && self.feature == ConfigFeature::Encoder as u8
            && self.op == ConfigOp::GetDirty as u8
        {
            1
        } else {
            0
        };
        encode_header(
            &mut bytes,
            CommonHeader {
                packet_type: PacketType::ConfigResponse,
                seq: self.seq,
                feature: self.feature,
                op: self.op,
                status_or_flags: self.status as u8,
                payload_len,
            },
        );
        if let Some(info) = self.encoder_get_info {
            let p = &mut bytes[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 4];
            p[0] = info.layer_count;
            p[1] = info.encoder_count;
            p[2] = info.capabilities;
            p[3] = 0;
        }
        if let Some(bindings) = self.encoder_get_bindings {
            let p = &mut bytes[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 28];
            p[0..4].copy_from_slice(&bindings.layer_id.to_le_bytes());
            p[4] = bindings.encoder_id;
            p[5] = bindings.source as u8;
            p[6] = bindings.flags.bits();
            p[7] = 0;
            encode_encoder_binding(&mut p[8..18], bindings.cw_binding);
            encode_encoder_binding(&mut p[18..28], bindings.ccw_binding);
        }
        if let Some(dirty) = self.encoder_get_dirty {
            bytes[PAYLOAD_OFFSET] = u8::from(dirty);
        }
        bytes
    }

    pub fn decode_payload(bytes: &[u8]) -> Result<Self, PacketError> {
        let header = decode_header(bytes)?;
        if header.packet_type != PacketType::ConfigResponse {
            return Err(PacketError::DecodeUnsupportedType(header.packet_type as u8));
        }
        let status = ConfigStatus::try_from(header.status_or_flags)?;
        let mut encoder_get_info = None;
        let mut encoder_get_bindings = None;
        let mut encoder_get_dirty = None;
        if status == ConfigStatus::Ok
            && header.feature == ConfigFeature::Encoder as u8
            && header.op == ConfigOp::GetInfo as u8
        {
            if header.payload_len != 4 {
                return Err(PacketError::InvalidPayloadLength {
                    max: 4,
                    actual: header.payload_len,
                });
            }
            let p = &bytes[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 4];
            if p[3] != 0 {
                return Err(PacketError::ReservedNotZero);
            }
            encoder_get_info = Some(EncoderGetInfo {
                layer_count: p[0],
                encoder_count: p[1],
                capabilities: p[2],
            });
        } else if status == ConfigStatus::Ok
            && header.feature == ConfigFeature::Encoder as u8
            && header.op == ConfigOp::GetBindings as u8
        {
            if header.payload_len != 28 {
                return Err(PacketError::InvalidPayloadLength {
                    max: 28,
                    actual: header.payload_len,
                });
            }
            let p = &bytes[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 28];
            if p[7] != 0 {
                return Err(PacketError::ReservedNotZero);
            }
            encoder_get_bindings = Some(EncoderGetBindings {
                layer_id: u32::from_le_bytes(p[0..4].try_into().unwrap()),
                encoder_id: p[4],
                source: EncoderBindingSource::try_from(p[5])?,
                flags: EncoderBindingFlags::new(p[6])?,
                cw_binding: decode_encoder_binding(&p[8..18]),
                ccw_binding: decode_encoder_binding(&p[18..28]),
            });
        } else if status == ConfigStatus::Ok
            && header.feature == ConfigFeature::Encoder as u8
            && header.op == ConfigOp::GetDirty as u8
        {
            if header.payload_len != 1 {
                return Err(PacketError::InvalidPayloadLength {
                    max: 1,
                    actual: header.payload_len,
                });
            }
            let dirty = bytes[PAYLOAD_OFFSET];
            encoder_get_dirty = Some(match dirty {
                0 => false,
                1 => true,
                other => return Err(PacketError::InvalidConfigDirty(other)),
            });
        } else {
            if header.payload_len != 0 {
                return Err(PacketError::InvalidPayloadLength {
                    max: 0,
                    actual: header.payload_len,
                });
            }
        }

        Ok(Self {
            seq: header.seq,
            feature: header.feature,
            op: header.op,
            status,
            encoder_get_info,
            encoder_get_bindings,
            encoder_get_dirty,
        })
    }

    pub fn is_response_to(self, request: ConfigRequest) -> bool {
        if self.seq != request.seq || self.feature != request.feature || self.op != request.op {
            return false;
        }
        // (seq, feature, op) alone cannot distinguish two GET_BINDINGS requests
        // issued by independent manager instances (each starts its seq at 0),
        // so an OK response must also echo the requested layer/encoder target.
        // Error-status responses carry no echo payload and are accepted as-is.
        if self.status == ConfigStatus::Ok {
            if let Some((layer_id, encoder_id)) = request.encoder_bindings_target() {
                return match self.encoder_get_bindings {
                    Some(bindings) => {
                        bindings.layer_id == layer_id && bindings.encoder_id == encoder_id
                    }
                    None => false,
                };
            }
        }
        true
    }
}

fn encode_encoder_binding(bytes: &mut [u8], binding: EncoderBinding) {
    bytes[0..2].copy_from_slice(&binding.behavior_id.to_le_bytes());
    bytes[2..6].copy_from_slice(&binding.param1.to_le_bytes());
    bytes[6..10].copy_from_slice(&binding.param2.to_le_bytes());
}

fn decode_encoder_binding(bytes: &[u8]) -> EncoderBinding {
    EncoderBinding {
        behavior_id: u16::from_le_bytes(bytes[0..2].try_into().unwrap()),
        param1: u32::from_le_bytes(bytes[2..6].try_into().unwrap()),
        param2: u32::from_le_bytes(bytes[6..10].try_into().unwrap()),
    }
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
        encode_header(
            &mut bytes,
            CommonHeader {
                packet_type: PacketType::AiUsage,
                seq: 0,
                feature: 0,
                op: 0,
                status_or_flags: 0,
                payload_len: 19,
            },
        );
        let p = &mut bytes[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 19];
        p[0] = self.provider as u8;
        p[1] = self.flags.bits();
        p[2..4].copy_from_slice(&self.five_hour_used_bp.to_le_bytes());
        p[4..6].copy_from_slice(&self.seven_day_used_bp.to_le_bytes());
        p[6..10].copy_from_slice(&self.five_hour_reset_unix.to_le_bytes());
        p[10..14].copy_from_slice(&self.seven_day_reset_unix.to_le_bytes());
        p[14..18].copy_from_slice(&self.updated_unix.to_le_bytes());
        p[18] = self.error_code as u8;
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
        encode_header(
            &mut bytes,
            CommonHeader {
                packet_type: PacketType::TimeSync,
                seq: 0,
                feature: 0,
                op: 0,
                status_or_flags: 0,
                payload_len: 9,
            },
        );
        let p = &mut bytes[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 9];
        p[0..4].copy_from_slice(&self.unix_time_sec.to_le_bytes());
        p[4..6].copy_from_slice(&self.tz_offset_min.to_le_bytes());
        p[6] = self.weekday;
        p[7] = self.format_hint;
        p[8] = self.clock_mode;
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
    pub seq: u8,
    pub capabilities: u32,
    pub device_uid_hash: Option<u64>,
}

impl DeviceHello {
    pub fn decode_payload(bytes: &[u8]) -> Result<Self, PacketError> {
        let header = decode_header(bytes)?;
        if header.packet_type != PacketType::DeviceHello {
            return Err(PacketError::DecodeUnsupportedType(header.packet_type as u8));
        }
        if header.payload_len != 12 {
            return Err(PacketError::InvalidPayloadLength {
                max: 12,
                actual: header.payload_len,
            });
        }
        let p = &bytes[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 12];
        let capabilities = u32::from_le_bytes(p[0..4].try_into().unwrap());
        let raw_uid = u64::from_le_bytes(p[4..12].try_into().unwrap());
        Ok(Self {
            seq: header.seq,
            capabilities,
            device_uid_hash: (raw_uid != 0).then_some(raw_uid),
        })
    }

    pub fn supports_app_layer(self) -> bool {
        self.capabilities & CAPABILITY_APP_LAYER != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryEntry {
    /// 0 = central/self, 1..=3 = peripheral 1..=3.
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
        let header = decode_header(bytes)?;
        match header.packet_type {
            PacketType::BatteryStatus => Self::decode_battery(bytes, header),
            PacketType::HostAction => Self::decode_host_action(bytes, header),
            PacketType::KeyStats => Self::decode_key_stats(bytes, header),
            PacketType::LayerState => Self::decode_layer_state(bytes, header),
            PacketType::KeyPress => Self::decode_key_press(bytes, header),
            other => Err(PacketError::DecodeUnsupportedType(other as u8)),
        }
    }

    fn decode_battery(bytes: &[u8], header: CommonHeader) -> Result<Self, PacketError> {
        let len = header.payload_len as usize;
        if len < 1 {
            return Err(PacketError::InvalidBatteryCount(0));
        }
        let p = &bytes[PAYLOAD_OFFSET..PAYLOAD_OFFSET + len];
        let count = p[0] as usize;
        if !(1..=4).contains(&count) {
            return Err(PacketError::InvalidBatteryCount(p[0]));
        }
        if len != 1 + 2 * count {
            return Err(PacketError::InvalidPayloadLength {
                max: 1 + 2 * count,
                actual: header.payload_len,
            });
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let source = p[1 + 2 * i];
            let raw_level = p[2 + 2 * i];
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
        Ok(Self::Battery(BatteryStatusPacket { entries }))
    }

    fn decode_host_action(bytes: &[u8], header: CommonHeader) -> Result<Self, PacketError> {
        if header.payload_len != 2 {
            return Err(PacketError::InvalidPayloadLength {
                max: 2,
                actual: header.payload_len,
            });
        }
        let p = &bytes[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 2];
        Ok(Self::HostAction(HostActionPacket {
            action_id: p[0],
            value: p[1],
            seq: header.seq,
        }))
    }

    fn decode_key_stats(bytes: &[u8], header: CommonHeader) -> Result<Self, PacketError> {
        let len = header.payload_len as usize;
        if len < 4 {
            return Err(PacketError::InvalidKeyStatsEntryCount(0));
        }
        let p = &bytes[PAYLOAD_OFFSET..PAYLOAD_OFFSET + len];
        let count = p[0] as usize;
        if !(1..=KEY_STATS_MAX_ENTRIES).contains(&count) {
            return Err(PacketError::InvalidKeyStatsEntryCount(p[0]));
        }
        let flags = p[1];
        if flags & !KEY_STATS_FLAG_MORE_FOLLOWS != 0 {
            return Err(PacketError::InvalidKeyStatsFlags(flags));
        }
        if p[2] != 0 || p[3] != 0 {
            return Err(PacketError::ReservedNotZero);
        }
        if len != 4 + 3 * count {
            return Err(PacketError::InvalidPayloadLength {
                max: 4 + 3 * count,
                actual: header.payload_len,
            });
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let base = 4 + 3 * i;
            let position = p[base];
            let delta = u16::from_le_bytes(p[base + 1..base + 3].try_into().unwrap());
            if delta == 0 {
                return Err(PacketError::InvalidKeyStatsDelta(position));
            }
            entries.push(KeyStatsEntry { position, delta });
        }
        Ok(Self::KeyStats(KeyStatsPacket {
            entries,
            more_follows: flags & KEY_STATS_FLAG_MORE_FOLLOWS != 0,
            seq: header.seq,
        }))
    }

    fn decode_layer_state(bytes: &[u8], header: CommonHeader) -> Result<Self, PacketError> {
        if header.payload_len != 8 {
            return Err(PacketError::InvalidPayloadLength {
                max: 8,
                actual: header.payload_len,
            });
        }
        let p = &bytes[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 8];
        if p[1] != 0 || p[2] != 0 || p[3] != 0 {
            return Err(PacketError::ReservedNotZero);
        }
        let active_layer = p[0];
        validate_layer(active_layer)?;
        let layer_mask = u32::from_le_bytes(p[4..8].try_into().unwrap());
        if layer_mask != 0 && layer_mask & (1 << active_layer) == 0 {
            return Err(PacketError::InvalidLayerMask {
                active_layer,
                layer_mask,
            });
        }
        Ok(Self::LayerState(LayerStatePacket {
            active_layer,
            layer_mask,
            seq: header.seq,
        }))
    }

    fn decode_key_press(bytes: &[u8], header: CommonHeader) -> Result<Self, PacketError> {
        if header.payload_len != 2 {
            return Err(PacketError::InvalidPayloadLength {
                max: 2,
                actual: header.payload_len,
            });
        }
        let p = &bytes[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 2];
        let flags = p[1];
        if flags & !KEY_PRESS_FLAG_PRESSED != 0 {
            return Err(PacketError::ReservedNotZero);
        }
        Ok(Self::KeyPress(KeyPressPacket {
            position: p[0],
            pressed: flags & KEY_PRESS_FLAG_PRESSED != 0,
            seq: header.seq,
        }))
    }

    /// Encode for round-trip tests and debug injection. Not used in the
    /// production send path (these packets are device-initiated).
    pub fn encode_payload(&self) -> [u8; PACKET_SIZE] {
        let mut bytes = [0u8; PACKET_SIZE];
        let (seq, payload_len) = match self {
            Self::Battery(p) => (0, 1 + 2 * p.entries.len()),
            Self::HostAction(p) => (p.seq, 2),
            Self::KeyStats(p) => (p.seq, 4 + 3 * p.entries.len()),
            Self::LayerState(p) => (p.seq, 8),
            Self::KeyPress(p) => (p.seq, 2),
        };
        encode_header(
            &mut bytes,
            CommonHeader {
                packet_type: self.packet_type(),
                seq,
                feature: 0,
                op: 0,
                status_or_flags: 0,
                payload_len: payload_len as u8,
            },
        );
        let p = &mut bytes[PAYLOAD_OFFSET..];
        match self {
            Self::Battery(packet) => {
                p[0] = packet.entries.len() as u8;
                for (i, entry) in packet.entries.iter().enumerate() {
                    p[1 + 2 * i] = entry.source;
                    p[2 + 2 * i] = entry.level.unwrap_or(BATTERY_LEVEL_UNKNOWN);
                }
            }
            Self::HostAction(packet) => {
                p[0] = packet.action_id;
                p[1] = packet.value;
            }
            Self::KeyStats(packet) => {
                p[0] = packet.entries.len() as u8;
                p[1] = if packet.more_follows {
                    KEY_STATS_FLAG_MORE_FOLLOWS
                } else {
                    0
                };
                for (i, entry) in packet.entries.iter().enumerate() {
                    let base = 4 + 3 * i;
                    p[base] = entry.position;
                    p[base + 1..base + 3].copy_from_slice(&entry.delta.to_le_bytes());
                }
            }
            Self::LayerState(packet) => {
                p[0] = packet.active_layer;
                p[4..8].copy_from_slice(&packet.layer_mask.to_le_bytes());
            }
            Self::KeyPress(packet) => {
                p[0] = packet.position;
                p[1] = if packet.pressed {
                    KEY_PRESS_FLAG_PRESSED
                } else {
                    0
                };
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
        let payload_len = match self.packet_type {
            PacketType::AppLayer => 2,
            _ => 0,
        };
        encode_header(
            &mut bytes,
            CommonHeader {
                packet_type: self.packet_type,
                seq: self.seq,
                feature: 0,
                op: 0,
                status_or_flags: 0,
                payload_len,
            },
        );
        if self.packet_type == PacketType::AppLayer {
            let p = &mut bytes[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 2];
            p[0] = self.action;
            p[1] = self.layer;
        }
        bytes
    }

    pub fn encode_report(self) -> [u8; REPORT_SIZE] {
        let mut report = [0u8; REPORT_SIZE];
        report[1..].copy_from_slice(&self.encode_payload());
        report
    }

    pub fn decode_payload(bytes: &[u8]) -> Result<Self, PacketError> {
        let header = decode_header(bytes)?;
        match header.packet_type {
            PacketType::HostHello | PacketType::Ping | PacketType::Pong => {
                if header.payload_len != 0 {
                    return Err(PacketError::InvalidPayloadLength {
                        max: 0,
                        actual: header.payload_len,
                    });
                }
                Ok(Self {
                    packet_type: header.packet_type,
                    action: 0,
                    layer: 0,
                    seq: header.seq,
                })
            }
            PacketType::DeviceHello => {
                let hello = DeviceHello::decode_payload(bytes)?;
                Ok(Self {
                    packet_type: header.packet_type,
                    action: 0,
                    layer: 0,
                    seq: hello.seq,
                })
            }
            PacketType::AppLayer => {
                let p = payload(bytes, 2)?;
                let action = AppLayerAction::try_from(p[0])?;
                let layer = p[1];
                match action {
                    AppLayerAction::Set => validate_layer(layer)?,
                    AppLayerAction::Clear if layer != 0 => {
                        return Err(PacketError::InvalidClearLayer(layer));
                    }
                    AppLayerAction::Clear => {}
                }
                Ok(Self {
                    packet_type: header.packet_type,
                    action: p[0],
                    layer,
                    seq: header.seq,
                })
            }
            PacketType::Error
            | PacketType::AiUsage
            | PacketType::TimeSync
            | PacketType::BatteryStatus
            | PacketType::HostAction
            | PacketType::KeyStats
            | PacketType::LayerState
            | PacketType::KeyPress
            | PacketType::ConfigRequest
            | PacketType::ConfigResponse => {
                Err(PacketError::DecodeUnsupportedType(header.packet_type as u8))
            }
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
    #[error("invalid payload length: max/expected {max}, got {actual}")]
    InvalidPayloadLength { max: usize, actual: u8 },
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
    #[error("invalid config status {0:#04x}")]
    InvalidConfigStatus(u8),
    #[error("invalid encoder binding source {0:#04x}")]
    InvalidEncoderBindingSource(u8),
    #[error("invalid encoder binding flags {0:#04x}")]
    InvalidEncoderBindingFlags(u8),
    #[error("invalid Config RPC dirty value {0:#04x}")]
    InvalidConfigDirty(u8),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device_hello(seq: u8, capabilities: u32, uid: u64) -> [u8; PACKET_SIZE] {
        let mut bytes = [0u8; PACKET_SIZE];
        encode_header(
            &mut bytes,
            CommonHeader {
                packet_type: PacketType::DeviceHello,
                seq,
                feature: 0,
                op: 0,
                status_or_flags: 0,
                payload_len: 12,
            },
        );
        bytes[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 4].copy_from_slice(&capabilities.to_le_bytes());
        bytes[PAYLOAD_OFFSET + 4..PAYLOAD_OFFSET + 12].copy_from_slice(&uid.to_le_bytes());
        bytes
    }

    #[test]
    fn encodes_report_with_zero_report_id() {
        let report = Packet::set_layer(3, 7).unwrap().encode_report();

        assert_eq!(report.len(), REPORT_SIZE);
        assert_eq!(REPORT_SIZE, 65);
        assert_eq!(report[0], 0);
        assert_eq!(&report[1..3], b"HL");
        assert_eq!(report[3], VERSION);
        assert_eq!(report[4], PacketType::AppLayer as u8);
        assert_eq!(report[5], 7);
        assert_eq!(report[9], 2);
        assert_eq!(report[13], AppLayerAction::Set as u8);
        assert_eq!(report[14], 3);
        assert!(report[15..].iter().all(|b| *b == 0));
    }

    #[test]
    fn common_header_rejects_invalid_fields() {
        let base = Packet::host_hello(1).encode_payload();

        let mut payload = base;
        payload[0] = b'X';
        assert_eq!(
            Packet::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidMagic
        );

        let mut payload = base;
        payload[2] = 0x01;
        assert_eq!(
            Packet::decode_payload(&payload).unwrap_err(),
            PacketError::UnsupportedVersion(0x01)
        );

        let mut payload = base;
        payload[3] = 0xFE;
        assert_eq!(
            Packet::decode_payload(&payload).unwrap_err(),
            PacketError::UnknownType(0xFE)
        );

        let mut payload = base;
        payload[8] = (MAX_PAYLOAD_LEN + 1) as u8;
        assert_eq!(
            Packet::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidPayloadLength {
                max: MAX_PAYLOAD_LEN,
                actual: (MAX_PAYLOAD_LEN + 1) as u8
            }
        );

        let mut payload = base;
        payload[9] = 1;
        assert_eq!(
            Packet::decode_payload(&payload).unwrap_err(),
            PacketError::ReservedNotZero
        );

        let mut payload = base;
        payload[PAYLOAD_OFFSET] = 1;
        assert_eq!(
            Packet::decode_payload(&payload).unwrap_err(),
            PacketError::ReservedNotZero
        );
    }

    #[test]
    fn decodes_device_hello_capabilities_and_uid() {
        let payload = device_hello(
            9,
            CAPABILITY_APP_LAYER | CAPABILITY_TIME_SYNC,
            0x7a91c3e4d102ab55,
        );

        let hello = DeviceHello::decode_payload(&payload).unwrap();

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
        let payload = device_hello(3, CAPABILITY_APP_LAYER, 0);

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
    fn rejects_invalid_app_layer_fields() {
        let mut payload = Packet::clear(1).encode_payload();
        payload[PAYLOAD_OFFSET] = 9;
        assert_eq!(
            Packet::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidAppLayerAction(9)
        );

        let mut payload = Packet::clear(1).encode_payload();
        payload[PAYLOAD_OFFSET + 1] = 1;
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

        assert_eq!(report[0], 0);
        assert_eq!(report[4], PacketType::TimeSync as u8);
        assert_eq!(report[9], 9);
        assert_eq!(&report[13..17], &0x12345678u32.to_le_bytes());
        assert_eq!(&report[17..19], &540i16.to_le_bytes());
        assert_eq!(report[19], 6);
        assert_eq!(report[20], 4);
        assert_eq!(report[21], 0);
        assert!(report[22..].iter().all(|b| *b == 0));
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

        assert_eq!(report[4], PacketType::AiUsage as u8);
        assert_eq!(report[9], 19);
        assert_eq!(report[13], AiUsageProvider::Codex as u8);
        assert_eq!(report[14], flags.bits());
        assert_eq!(&report[15..17], &1234u16.to_le_bytes());
        assert!(report[32..].iter().all(|b| *b == 0));
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

        let encoded = packet.encode_payload();
        assert_eq!(encoded[8], 7);
        let decoded = UplinkPacket::decode_payload(&encoded).unwrap();
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
        payload[PAYLOAD_OFFSET] = 0;
        payload[8] = 1;
        payload[PAYLOAD_OFFSET + 1..].fill(0);
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidBatteryCount(0)
        );

        let mut payload = base;
        payload[PAYLOAD_OFFSET + 2] = 101;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidBatteryLevel(101)
        );

        let mut payload = base;
        payload[PAYLOAD_OFFSET + 1] = 4;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidBatterySource(4)
        );
    }

    #[test]
    fn host_action_round_trips() {
        let packet = UplinkPacket::HostAction(HostActionPacket {
            action_id: 7,
            value: 3,
            seq: 200,
        });

        let encoded = packet.encode_payload();
        assert_eq!(encoded[4], 200);
        let decoded = UplinkPacket::decode_payload(&encoded).unwrap();
        assert_eq!(decoded, packet);
        assert_eq!(decoded.required_capability(), CAPABILITY_HOST_ACTION);
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
        assert_eq!(payload[8], 28);
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
        payload[PAYLOAD_OFFSET] = 0;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidKeyStatsEntryCount(0)
        );

        let mut payload = base;
        payload[PAYLOAD_OFFSET + 1] = 0x02;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidKeyStatsFlags(0x02)
        );

        let mut payload = base;
        payload[PAYLOAD_OFFSET + 5] = 0;
        payload[PAYLOAD_OFFSET + 6] = 0;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidKeyStatsDelta(4)
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
    fn layer_state_rejects_invalid_fields() {
        let base = UplinkPacket::LayerState(LayerStatePacket {
            active_layer: 2,
            layer_mask: 0b0100,
            seq: 0,
        })
        .encode_payload();

        let mut payload = base;
        payload[PAYLOAD_OFFSET] = 32;
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidLayer(32)
        );

        let mut payload = base;
        payload[PAYLOAD_OFFSET + 4..PAYLOAD_OFFSET + 8].copy_from_slice(&0b0010u32.to_le_bytes());
        assert_eq!(
            UplinkPacket::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidLayerMask {
                active_layer: 2,
                layer_mask: 0b0010
            }
        );
    }

    #[test]
    fn key_press_round_trips() {
        let packet = UplinkPacket::KeyPress(KeyPressPacket {
            position: 44,
            pressed: true,
            seq: 2,
        });
        assert_eq!(
            UplinkPacket::decode_payload(&packet.encode_payload()).unwrap(),
            packet
        );
    }

    #[test]
    fn config_packet_types_are_known_but_not_typed_decoded() {
        let mut request = [0u8; PACKET_SIZE];
        encode_header(
            &mut request,
            CommonHeader {
                packet_type: PacketType::ConfigRequest,
                seq: 1,
                feature: 1,
                op: 1,
                status_or_flags: 0,
                payload_len: 0,
            },
        );
        assert_eq!(
            Packet::decode_payload(&request).unwrap_err(),
            PacketError::DecodeUnsupportedType(PacketType::ConfigRequest as u8)
        );

        let mut response = [0u8; PACKET_SIZE];
        encode_header(
            &mut response,
            CommonHeader {
                packet_type: PacketType::ConfigResponse,
                seq: 1,
                feature: 1,
                op: 1,
                status_or_flags: 0,
                payload_len: 0,
            },
        );
        assert_eq!(
            UplinkPacket::decode_payload(&response).unwrap_err(),
            PacketError::DecodeUnsupportedType(PacketType::ConfigResponse as u8)
        );
    }

    #[test]
    fn config_get_info_request_encodes_common_header() {
        let request = ConfigRequest::encoder_get_info(11);
        let payload = request.encode_payload();

        assert_eq!(&payload[0..2], b"HL");
        assert_eq!(payload[2], VERSION);
        assert_eq!(payload[3], PacketType::ConfigRequest as u8);
        assert_eq!(payload[4], 11);
        assert_eq!(payload[5], ConfigFeature::Encoder as u8);
        assert_eq!(payload[6], ConfigOp::GetInfo as u8);
        assert_eq!(payload[7], 0);
        assert_eq!(payload[8], 0);
        assert!(payload[9..].iter().all(|b| *b == 0));
    }

    #[test]
    fn config_get_bindings_request_encodes_payload() {
        let request = ConfigRequest::encoder_get_bindings(11, 0x01020304, 2);
        let payload = request.encode_payload();

        assert_eq!(&payload[0..2], b"HL");
        assert_eq!(payload[2], VERSION);
        assert_eq!(payload[3], PacketType::ConfigRequest as u8);
        assert_eq!(payload[4], 11);
        assert_eq!(payload[5], ConfigFeature::Encoder as u8);
        assert_eq!(payload[6], ConfigOp::GetBindings as u8);
        assert_eq!(payload[7], 0);
        assert_eq!(payload[8], 5);
        assert_eq!(&payload[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 4], &[4, 3, 2, 1]);
        assert_eq!(payload[PAYLOAD_OFFSET + 4], 2);
        assert!(payload[PAYLOAD_OFFSET + 5..].iter().all(|b| *b == 0));
    }

    #[test]
    fn config_set_bindings_request_encodes_payload() {
        let cw_binding = EncoderBinding {
            behavior_id: 0x1234,
            param1: 0x01020304,
            param2: 0x05060708,
        };
        let ccw_binding = EncoderBinding {
            behavior_id: 0x5678,
            param1: 0x11121314,
            param2: 0x15161718,
        };
        let request =
            ConfigRequest::encoder_set_bindings(11, 0x0a0b0c0d, 2, cw_binding, ccw_binding);
        let payload = request.encode_payload();

        assert_eq!(&payload[0..2], b"HL");
        assert_eq!(payload[2], VERSION);
        assert_eq!(payload[3], PacketType::ConfigRequest as u8);
        assert_eq!(payload[4], 11);
        assert_eq!(payload[5], ConfigFeature::Encoder as u8);
        assert_eq!(payload[6], ConfigOp::SetBindings as u8);
        assert_eq!(payload[7], 0);
        assert_eq!(payload[8], 28);
        assert_eq!(
            &payload[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 4],
            &[0x0d, 0x0c, 0x0b, 0x0a]
        );
        assert_eq!(payload[PAYLOAD_OFFSET + 4], 2);
        assert_eq!(payload[PAYLOAD_OFFSET + 5], 0x03);
        assert_eq!(payload[PAYLOAD_OFFSET + 6], 0);
        assert_eq!(payload[PAYLOAD_OFFSET + 7], 0);
        assert_eq!(
            &payload[PAYLOAD_OFFSET + 8..PAYLOAD_OFFSET + 18],
            &[0x34, 0x12, 0x04, 0x03, 0x02, 0x01, 0x08, 0x07, 0x06, 0x05]
        );
        assert_eq!(
            &payload[PAYLOAD_OFFSET + 18..PAYLOAD_OFFSET + 28],
            &[0x78, 0x56, 0x14, 0x13, 0x12, 0x11, 0x18, 0x17, 0x16, 0x15]
        );
        assert!(payload[PAYLOAD_OFFSET + 28..].iter().all(|b| *b == 0));
    }

    #[test]
    fn config_lifecycle_requests_encode_payloads() {
        let get_dirty = ConfigRequest::encoder_get_dirty(21).encode_payload();
        assert_eq!(get_dirty[3], PacketType::ConfigRequest as u8);
        assert_eq!(get_dirty[4], 21);
        assert_eq!(get_dirty[5], ConfigFeature::Encoder as u8);
        assert_eq!(get_dirty[6], ConfigOp::GetDirty as u8);
        assert_eq!(get_dirty[8], 0);
        assert!(get_dirty[PAYLOAD_OFFSET..].iter().all(|b| *b == 0));

        let save = ConfigRequest::encoder_save(22).encode_payload();
        assert_eq!(save[4], 22);
        assert_eq!(save[6], ConfigOp::Save as u8);
        assert_eq!(save[8], 0);

        let discard = ConfigRequest::encoder_discard(23).encode_payload();
        assert_eq!(discard[4], 23);
        assert_eq!(discard[6], ConfigOp::Discard as u8);
        assert_eq!(discard[8], 0);

        let clear = ConfigRequest::encoder_clear_override(24, 0x01020304, 5).encode_payload();
        assert_eq!(clear[4], 24);
        assert_eq!(clear[6], ConfigOp::ClearOverride as u8);
        assert_eq!(clear[8], 5);
        assert_eq!(&clear[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 4], &[4, 3, 2, 1]);
        assert_eq!(clear[PAYLOAD_OFFSET + 4], 5);
        assert!(clear[PAYLOAD_OFFSET + 5..].iter().all(|b| *b == 0));
    }

    #[test]
    fn config_get_info_response_decodes_ok_payload() {
        let info = EncoderGetInfo {
            layer_count: 4,
            encoder_count: 2,
            capabilities: 0,
        };
        let payload = ConfigResponse::encoder_get_info_ok(12, info).encode_payload();

        let decoded = ConfigResponse::decode_payload(&payload).unwrap();

        assert_eq!(decoded.seq, 12);
        assert_eq!(decoded.feature, ConfigFeature::Encoder as u8);
        assert_eq!(decoded.op, ConfigOp::GetInfo as u8);
        assert_eq!(decoded.status, ConfigStatus::Ok);
        assert_eq!(decoded.encoder_get_info, Some(info));
    }

    #[test]
    fn config_get_bindings_response_decodes_ok_payload() {
        let bindings = EncoderGetBindings {
            layer_id: 7,
            encoder_id: 1,
            source: EncoderBindingSource::Keymap,
            flags: EncoderBindingFlags::new(
                EncoderBindingFlags::SAVED_EXISTS | EncoderBindingFlags::RUNTIME_DIRTY,
            )
            .unwrap(),
            cw_binding: EncoderBinding {
                behavior_id: 0x1234,
                param1: 56,
                param2: 78,
            },
            ccw_binding: EncoderBinding {
                behavior_id: 0x5678,
                param1: 90,
                param2: 12,
            },
        };
        let payload = ConfigResponse::encoder_get_bindings_ok(12, bindings).encode_payload();

        let decoded = ConfigResponse::decode_payload(&payload).unwrap();

        assert_eq!(decoded.seq, 12);
        assert_eq!(decoded.feature, ConfigFeature::Encoder as u8);
        assert_eq!(decoded.op, ConfigOp::GetBindings as u8);
        assert_eq!(decoded.status, ConfigStatus::Ok);
        assert_eq!(decoded.encoder_get_bindings, Some(bindings));
        assert_eq!(payload[8], 28);
        assert!(payload[PAYLOAD_OFFSET + 28..].iter().all(|b| *b == 0));
    }

    #[test]
    fn config_get_dirty_response_decodes_ok_payload() {
        let payload = ConfigResponse::encoder_get_dirty_ok(12, true).encode_payload();

        let decoded = ConfigResponse::decode_payload(&payload).unwrap();

        assert_eq!(decoded.seq, 12);
        assert_eq!(decoded.feature, ConfigFeature::Encoder as u8);
        assert_eq!(decoded.op, ConfigOp::GetDirty as u8);
        assert_eq!(decoded.status, ConfigStatus::Ok);
        assert_eq!(decoded.encoder_get_dirty, Some(true));
        assert_eq!(payload[8], 1);
        assert_eq!(payload[PAYLOAD_OFFSET], 1);
        assert!(payload[PAYLOAD_OFFSET + 1..].iter().all(|b| *b == 0));

        let payload = ConfigResponse::encoder_get_dirty_ok(13, false).encode_payload();
        let decoded = ConfigResponse::decode_payload(&payload).unwrap();
        assert_eq!(decoded.encoder_get_dirty, Some(false));
        assert_eq!(payload[PAYLOAD_OFFSET], 0);
    }

    #[test]
    fn config_response_decodes_non_ok_status_without_payload() {
        let payload = ConfigResponse::status(
            12,
            ConfigFeature::Encoder as u8,
            ConfigOp::GetInfo as u8,
            ConfigStatus::UnsupportedOp,
        )
        .encode_payload();

        let decoded = ConfigResponse::decode_payload(&payload).unwrap();

        assert_eq!(decoded.status, ConfigStatus::UnsupportedOp);
        assert_eq!(decoded.encoder_get_info, None);
        assert_eq!(decoded.encoder_get_bindings, None);
        assert_eq!(decoded.encoder_get_dirty, None);
    }

    #[test]
    fn config_set_bindings_response_decodes_ok_status_without_payload() {
        let payload = ConfigResponse::status(
            12,
            ConfigFeature::Encoder as u8,
            ConfigOp::SetBindings as u8,
            ConfigStatus::Ok,
        )
        .encode_payload();

        let decoded = ConfigResponse::decode_payload(&payload).unwrap();

        assert_eq!(decoded.seq, 12);
        assert_eq!(decoded.feature, ConfigFeature::Encoder as u8);
        assert_eq!(decoded.op, ConfigOp::SetBindings as u8);
        assert_eq!(decoded.status, ConfigStatus::Ok);
        assert_eq!(decoded.encoder_get_info, None);
        assert_eq!(decoded.encoder_get_bindings, None);
        assert_eq!(decoded.encoder_get_dirty, None);
        assert_eq!(payload[8], 0);
    }

    #[test]
    fn config_lifecycle_responses_decode_ok_status_without_payload() {
        for op in [ConfigOp::Save, ConfigOp::Discard, ConfigOp::ClearOverride] {
            let payload = ConfigResponse::status(
                12,
                ConfigFeature::Encoder as u8,
                op as u8,
                ConfigStatus::Ok,
            )
            .encode_payload();

            let decoded = ConfigResponse::decode_payload(&payload).unwrap();

            assert_eq!(decoded.seq, 12);
            assert_eq!(decoded.feature, ConfigFeature::Encoder as u8);
            assert_eq!(decoded.op, op as u8);
            assert_eq!(decoded.status, ConfigStatus::Ok);
            assert_eq!(decoded.encoder_get_info, None);
            assert_eq!(decoded.encoder_get_bindings, None);
            assert_eq!(decoded.encoder_get_dirty, None);
            assert_eq!(payload[8], 0);
        }
    }

    #[test]
    fn config_get_info_response_rejects_invalid_payload() {
        let info = EncoderGetInfo {
            layer_count: 4,
            encoder_count: 2,
            capabilities: 0,
        };
        let mut payload = ConfigResponse::encoder_get_info_ok(12, info).encode_payload();
        payload[PAYLOAD_OFFSET + 3] = 1;

        assert_eq!(
            ConfigResponse::decode_payload(&payload).unwrap_err(),
            PacketError::ReservedNotZero
        );

        let mut payload = ConfigResponse::encoder_get_info_ok(12, info).encode_payload();
        payload[7] = 0xFE;

        assert_eq!(
            ConfigResponse::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidConfigStatus(0xFE)
        );
    }

    #[test]
    fn config_get_bindings_response_rejects_invalid_payload() {
        let bindings = EncoderGetBindings {
            layer_id: 7,
            encoder_id: 1,
            source: EncoderBindingSource::Keymap,
            flags: EncoderBindingFlags::default(),
            cw_binding: EncoderBinding {
                behavior_id: 0,
                param1: 0,
                param2: 0,
            },
            ccw_binding: EncoderBinding {
                behavior_id: 0,
                param1: 0,
                param2: 0,
            },
        };

        let mut payload = ConfigResponse::encoder_get_bindings_ok(12, bindings).encode_payload();
        payload[PAYLOAD_OFFSET + 7] = 1;
        assert_eq!(
            ConfigResponse::decode_payload(&payload).unwrap_err(),
            PacketError::ReservedNotZero
        );

        let mut payload = ConfigResponse::encoder_get_bindings_ok(12, bindings).encode_payload();
        payload[PAYLOAD_OFFSET + 5] = 0xFE;
        assert_eq!(
            ConfigResponse::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidEncoderBindingSource(0xFE)
        );

        let mut payload = ConfigResponse::encoder_get_bindings_ok(12, bindings).encode_payload();
        payload[PAYLOAD_OFFSET + 6] = 0x10;
        assert_eq!(
            ConfigResponse::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidEncoderBindingFlags(0x10)
        );

        let mut payload = ConfigResponse::encoder_get_bindings_ok(12, bindings).encode_payload();
        payload[8] = 27;
        payload[PAYLOAD_OFFSET + 27..].fill(0);
        assert_eq!(
            ConfigResponse::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidPayloadLength {
                max: 28,
                actual: 27
            }
        );
    }

    #[test]
    fn config_get_dirty_response_rejects_invalid_payload() {
        let mut payload = ConfigResponse::encoder_get_dirty_ok(12, true).encode_payload();
        payload[PAYLOAD_OFFSET] = 2;
        assert_eq!(
            ConfigResponse::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidConfigDirty(2)
        );

        let mut payload = ConfigResponse::encoder_get_dirty_ok(12, true).encode_payload();
        payload[8] = 0;
        payload[PAYLOAD_OFFSET] = 0;
        assert_eq!(
            ConfigResponse::decode_payload(&payload).unwrap_err(),
            PacketError::InvalidPayloadLength { max: 1, actual: 0 }
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
