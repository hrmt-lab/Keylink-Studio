use thiserror::Error;

pub const PACKET_SIZE: usize = 32;
pub const REPORT_SIZE: usize = PACKET_SIZE + 1;
pub const MAGIC: [u8; 2] = *b"HL";
pub const VERSION: u8 = 0x01;
pub const MAX_LAYER: u8 = 31;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PacketType {
    SetLayer = 0x01,
    Clear = 0x02,
    Hello = 0x10,
    HelloResponse = 0x11,
    TimeSync = 0x20,
}

impl TryFrom<u8> for PacketType {
    type Error = PacketError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::SetLayer),
            0x02 => Ok(Self::Clear),
            0x10 => Ok(Self::Hello),
            0x11 => Ok(Self::HelloResponse),
            0x20 => Ok(Self::TimeSync),
            other => Err(PacketError::UnknownType(other)),
        }
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
    pub layer: u8,
    pub flags: u8,
    pub seq: u8,
}

impl Packet {
    pub fn set_layer(layer: u8, seq: u8) -> Result<Self, PacketError> {
        validate_layer(layer)?;
        Ok(Self {
            packet_type: PacketType::SetLayer,
            layer,
            flags: 0,
            seq,
        })
    }

    pub fn clear(seq: u8) -> Self {
        Self {
            packet_type: PacketType::Clear,
            layer: 0,
            flags: 0,
            seq,
        }
    }

    pub fn hello(seq: u8) -> Self {
        Self {
            packet_type: PacketType::Hello,
            layer: 0,
            flags: 0,
            seq,
        }
    }

    pub fn encode_payload(self) -> [u8; PACKET_SIZE] {
        let mut bytes = [0u8; PACKET_SIZE];
        bytes[0..2].copy_from_slice(&MAGIC);
        bytes[2] = VERSION;
        bytes[3] = self.packet_type as u8;
        bytes[4] = self.layer;
        bytes[5] = self.flags;
        bytes[6] = self.seq;
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
        if bytes[7..].iter().any(|b| *b != 0) {
            return Err(PacketError::ReservedNotZero);
        }

        let packet_type = PacketType::try_from(bytes[3])?;
        let layer = bytes[4];
        if packet_type == PacketType::SetLayer {
            validate_layer(layer)?;
        }

        Ok(Self {
            packet_type,
            layer,
            flags: bytes[5],
            seq: bytes[6],
        })
    }

    pub fn is_matching_hello_response(self, seq: u8) -> bool {
        self.packet_type == PacketType::HelloResponse && self.seq == seq
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
    #[error("reserved bytes must be zero")]
    ReservedNotZero,
    #[error("invalid weekday {0}; expected 1-7")]
    InvalidWeekday(u8),
    #[error("invalid timezone offset {0}; expected -1440..=1440")]
    InvalidTimezoneOffset(i16),
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
        assert_eq!(report[4], PacketType::SetLayer as u8);
        assert_eq!(report[5], 3);
        assert_eq!(report[7], 7);
        assert!(report[8..].iter().all(|b| *b == 0));
    }

    #[test]
    fn decodes_payload() {
        let payload = Packet::hello(42).encode_payload();
        let packet = Packet::decode_payload(&payload).unwrap();

        assert_eq!(packet.packet_type, PacketType::Hello);
        assert_eq!(packet.seq, 42);
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
}
