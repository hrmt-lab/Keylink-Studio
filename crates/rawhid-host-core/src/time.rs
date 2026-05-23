use chrono::Offset;
use thiserror::Error;

use crate::{
    config::{ClockMode, TimeConfig, TimeFormatHint},
    packet::{PacketError, TimeSyncPacket},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSnapshot {
    pub unix_time_sec: u64,
    pub tz_offset_min: i16,
}

pub trait Clock {
    fn now(&self) -> Result<TimeSnapshot, TimeError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Result<TimeSnapshot, TimeError> {
        let now = chrono::Local::now();
        let unix_time_sec = now.timestamp();
        if unix_time_sec < 0 {
            return Err(TimeError::UnixTimeOutOfRange(unix_time_sec));
        }
        let tz_offset_min = now.offset().fix().local_minus_utc() / 60;
        let tz_offset_min = i16::try_from(tz_offset_min)
            .map_err(|_| TimeError::TimezoneOffsetOutOfRange(tz_offset_min))?;

        Ok(TimeSnapshot {
            unix_time_sec: unix_time_sec as u64,
            tz_offset_min,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TimeSyncState {
    last_display_key: Option<DisplayKey>,
    last_periodic_sync_sec: Option<u64>,
    last_device_generation: Option<u64>,
}

impl TimeSyncState {
    pub fn build_due_packet(
        &mut self,
        config: &TimeConfig,
        snapshot: TimeSnapshot,
        device_generation: u64,
    ) -> Result<Option<TimeSyncPacket>, TimeError> {
        if !config.enabled {
            return Ok(None);
        }

        let tz_offset_min = config.tz_offset_min.unwrap_or(snapshot.tz_offset_min);
        validate_tz_offset_min(tz_offset_min)?;

        let display_key = DisplayKey::new(config.format_hint, snapshot, tz_offset_min);
        let device_changed = self.last_device_generation != Some(device_generation);
        let display_changed = self.last_display_key.as_ref() != Some(&display_key);
        let periodic_due = is_periodic_due(
            config.periodic_sync_sec,
            self.last_periodic_sync_sec,
            snapshot.unix_time_sec,
        );

        if !(device_changed || display_changed || periodic_due) {
            return Ok(None);
        }

        let packet = build_time_sync_packet(config, snapshot, tz_offset_min)?;
        self.last_display_key = Some(display_key);
        self.last_periodic_sync_sec = Some(snapshot.unix_time_sec);
        self.last_device_generation = Some(device_generation);
        Ok(Some(packet))
    }
}

fn build_time_sync_packet(
    config: &TimeConfig,
    snapshot: TimeSnapshot,
    tz_offset_min: i16,
) -> Result<TimeSyncPacket, TimeError> {
    let unix_time_sec = u32::try_from(snapshot.unix_time_sec)
        .map_err(|_| TimeError::UnixTimeTooLarge(snapshot.unix_time_sec))?;
    let weekday = iso_weekday(snapshot.unix_time_sec, tz_offset_min);
    TimeSyncPacket::new(
        unix_time_sec,
        tz_offset_min,
        weekday,
        config.format_hint.as_packet_value(),
        config.clock_mode.as_packet_value(),
    )
    .map_err(TimeError::Packet)
}

fn is_periodic_due(periodic_sync_sec: u64, last_sync: Option<u64>, now: u64) -> bool {
    periodic_sync_sec != 0
        && last_sync
            .and_then(|last_sync| now.checked_sub(last_sync))
            .is_some_and(|elapsed| elapsed >= periodic_sync_sec)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DisplayKey {
    kind: DisplayKeyKind,
    value: i64,
}

impl DisplayKey {
    fn new(format_hint: TimeFormatHint, snapshot: TimeSnapshot, tz_offset_min: i16) -> Self {
        let local_sec = local_unix_sec(snapshot.unix_time_sec, tz_offset_min);
        let kind = match format_hint {
            TimeFormatHint::TimeHm
            | TimeFormatHint::TimeHms
            | TimeFormatHint::DatetimeHm
            | TimeFormatHint::WeekdayHm => DisplayKeyKind::Minute,
            TimeFormatHint::DateYmd | TimeFormatHint::DateMd => DisplayKeyKind::Day,
        };
        let value = match kind {
            DisplayKeyKind::Minute => floor_div(local_sec, 60),
            DisplayKeyKind::Day => floor_div(local_sec, 86_400),
        };
        Self { kind, value }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayKeyKind {
    Minute,
    Day,
}

pub fn iso_weekday(unix_time_sec: u64, tz_offset_min: i16) -> u8 {
    let local_days = floor_div(local_unix_sec(unix_time_sec, tz_offset_min), 86_400);
    (local_days + 3).rem_euclid(7) as u8 + 1
}

fn local_unix_sec(unix_time_sec: u64, tz_offset_min: i16) -> i64 {
    unix_time_sec as i64 + i64::from(tz_offset_min) * 60
}

fn floor_div(value: i64, divisor: i64) -> i64 {
    value.div_euclid(divisor)
}

fn validate_tz_offset_min(offset: i16) -> Result<(), TimeError> {
    if !(-1440..=1440).contains(&offset) {
        return Err(TimeError::TimezoneOffsetOutOfRange(i32::from(offset)));
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum TimeError {
    #[error("system time before Unix epoch: {0}")]
    UnixTimeOutOfRange(i64),
    #[error("unix_time_sec does not fit uint32: {0}")]
    UnixTimeTooLarge(u64),
    #[error("timezone offset out of range: {0}")]
    TimezoneOffsetOutOfRange(i32),
    #[error("packet error: {0}")]
    Packet(#[from] PacketError),
}

#[allow(dead_code)]
fn _keep_config_types(_: ClockMode) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_config(format_hint: TimeFormatHint) -> TimeConfig {
        TimeConfig {
            enabled: true,
            format_hint,
            ..TimeConfig::default()
        }
    }

    #[test]
    fn weekday_uses_local_date() {
        // 2024-01-01T15:30:00Z is Tuesday in UTC+09:00.
        assert_eq!(iso_weekday(1_704_122_200, 540), 2);
    }

    #[test]
    fn initial_sync_is_due() {
        let mut state = TimeSyncState::default();
        let packet = state
            .build_due_packet(
                &enabled_config(TimeFormatHint::TimeHm),
                TimeSnapshot {
                    unix_time_sec: 1_704_122_200,
                    tz_offset_min: 540,
                },
                1,
            )
            .unwrap();

        assert!(packet.is_some());
    }

    #[test]
    fn time_hms_does_not_sync_every_second() {
        let mut state = TimeSyncState::default();
        let config = TimeConfig {
            periodic_sync_sec: 60,
            ..enabled_config(TimeFormatHint::TimeHms)
        };
        let first = TimeSnapshot {
            unix_time_sec: 1_704_122_200,
            tz_offset_min: 540,
        };
        assert!(state.build_due_packet(&config, first, 1).unwrap().is_some());

        let next_second = TimeSnapshot {
            unix_time_sec: first.unix_time_sec + 1,
            ..first
        };
        assert!(state
            .build_due_packet(&config, next_second, 1)
            .unwrap()
            .is_none());
    }

    #[test]
    fn periodic_sync_can_be_disabled() {
        let mut state = TimeSyncState::default();
        let config = TimeConfig {
            periodic_sync_sec: 0,
            ..enabled_config(TimeFormatHint::TimeHm)
        };
        let first = TimeSnapshot {
            unix_time_sec: 1_704_122_200,
            tz_offset_min: 540,
        };
        assert!(state.build_due_packet(&config, first, 1).unwrap().is_some());

        let later_same_minute = TimeSnapshot {
            unix_time_sec: first.unix_time_sec + 1,
            ..first
        };
        assert!(state
            .build_due_packet(&config, later_same_minute, 1)
            .unwrap()
            .is_none());
    }

    #[test]
    fn device_generation_change_resyncs() {
        let mut state = TimeSyncState::default();
        let config = enabled_config(TimeFormatHint::TimeHm);
        let snapshot = TimeSnapshot {
            unix_time_sec: 1_704_122_200,
            tz_offset_min: 540,
        };
        assert!(state
            .build_due_packet(&config, snapshot, 1)
            .unwrap()
            .is_some());
        assert!(state
            .build_due_packet(&config, snapshot, 2)
            .unwrap()
            .is_some());
    }
}
