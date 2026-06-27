//! Persistent per-device key-press statistics fed by KEY_STATS uplink packets.
//!
//! Counts are stored per local day so the UI can aggregate "today", "last 7
//! days" and "all time" without the firmware keeping any history. Files live
//! under a caller-provided directory (production: `<data_dir>/stats/`), one
//! JSON file per device uid.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

use crate::packet::KeyStatsEntry;

pub type SharedKeyStatsStore = Arc<Mutex<KeyStatsStore>>;

/// Production stats directory: `<user data dir>/Keylink Studio/data/stats`.
pub fn default_stats_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "Keylink Studio")
        .map(|dirs| dirs.data_dir().join("stats"))
}

const FILE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsPeriod {
    Today,
    Last7Days,
    All,
}

impl StatsPeriod {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "today" => Some(Self::Today),
            "last7days" => Some(Self::Last7Days),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
struct DeviceKeyStatsFile {
    version: u32,
    /// "YYYY-MM-DD" (local date) → position → count.
    days: BTreeMap<String, BTreeMap<u16, u64>>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PositionCount {
    pub position: u16,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct KeyStatsSummary {
    pub device_key: String,
    pub total: u64,
    pub per_position: Vec<PositionCount>,
    pub days_covered: u32,
}

#[derive(Debug)]
pub struct KeyStatsStore {
    dir: PathBuf,
    devices: HashMap<String, DeviceKeyStatsFile>,
    dirty: HashSet<String>,
    last_flush: Option<Instant>,
    flush_interval: Duration,
}

impl KeyStatsStore {
    pub fn new(dir: PathBuf, flush_interval: Duration) -> Self {
        Self {
            dir,
            devices: HashMap::new(),
            dirty: HashSet::new(),
            last_flush: None,
            flush_interval,
        }
    }

    pub fn set_flush_interval(&mut self, interval: Duration) {
        self.flush_interval = interval;
    }

    /// Add key-press deltas for `device_key` into the bucket for `local_date`
    /// ("YYYY-MM-DD").
    pub fn apply_diff(&mut self, device_key: &str, entries: &[KeyStatsEntry], local_date: &str) {
        if entries.is_empty() {
            return;
        }
        self.load_device(device_key);
        let day = self
            .devices
            .get_mut(device_key)
            .expect("device loaded above")
            .days
            .entry(local_date.to_string())
            .or_default();
        for entry in entries {
            let count = day.entry(u16::from(entry.position)).or_insert(0);
            *count = count.saturating_add(u64::from(entry.delta));
        }
        self.dirty.insert(device_key.to_string());
    }

    /// Flush dirty devices when the flush interval elapsed.
    pub fn flush_if_due(&mut self, now: Instant) {
        let due = match self.last_flush {
            Some(last) => now.duration_since(last) >= self.flush_interval,
            None => true,
        };
        if due {
            self.flush_all();
            self.last_flush = Some(now);
        }
    }

    pub fn flush_all(&mut self) {
        let dirty: Vec<String> = self.dirty.drain().collect();
        for device_key in dirty {
            if let Some(data) = self.devices.get(&device_key) {
                if let Err(error) = self.write_device_file(&device_key, data) {
                    warn!("failed to persist key stats for {device_key}: {error}");
                    // Keep the data in memory; retry on the next flush.
                    self.dirty.insert(device_key);
                }
            }
        }
    }

    pub fn summary(
        &mut self,
        device_key: &str,
        period: StatsPeriod,
        today: &str,
    ) -> KeyStatsSummary {
        self.load_device(device_key);
        let data = self.devices.get(device_key).expect("device loaded above");

        let since = match period {
            StatsPeriod::Today => Some(today.to_string()),
            StatsPeriod::Last7Days => date_days_before(today, 6),
            StatsPeriod::All => None,
        };

        let mut per_position: BTreeMap<u16, u64> = BTreeMap::new();
        let mut days_covered = 0u32;
        for (date, counts) in &data.days {
            if let Some(since) = &since {
                if date < since {
                    continue;
                }
            }
            // Future-dated buckets (clock changes) are still included under All.
            days_covered += 1;
            for (position, count) in counts {
                let total = per_position.entry(*position).or_insert(0);
                *total = total.saturating_add(*count);
            }
        }

        let total = per_position.values().copied().sum();
        KeyStatsSummary {
            device_key: device_key.to_string(),
            total,
            per_position: per_position
                .into_iter()
                .map(|(position, count)| PositionCount { position, count })
                .collect(),
            days_covered,
        }
    }

    /// Device keys with any recorded stats (loaded or on disk).
    pub fn device_keys(&self) -> Vec<String> {
        let mut keys: HashSet<String> = self.devices.keys().cloned().collect();
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if let Some(stem) = name.strip_suffix(".json") {
                    if let Some(hex) = stem.strip_prefix("uid_") {
                        keys.insert(format!("uid:{hex}"));
                    }
                }
            }
        }
        let mut keys: Vec<String> = keys.into_iter().collect();
        keys.sort();
        keys
    }

    fn load_device(&mut self, device_key: &str) {
        if self.devices.contains_key(device_key) {
            return;
        }
        let path = self.device_file_path(device_key);
        let data = match fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<DeviceKeyStatsFile>(&text) {
                Ok(data) if data.version == FILE_VERSION => data,
                Ok(data) => {
                    warn!(
                        "unsupported key stats file version {} for {device_key}; starting fresh",
                        data.version
                    );
                    quarantine_file(&path);
                    DeviceKeyStatsFile::default()
                }
                Err(error) => {
                    warn!("corrupt key stats file for {device_key}: {error}; starting fresh");
                    quarantine_file(&path);
                    DeviceKeyStatsFile::default()
                }
            },
            Err(_) => DeviceKeyStatsFile::default(),
        };
        let mut data = data;
        data.version = FILE_VERSION;
        self.devices.insert(device_key.to_string(), data);
    }

    fn write_device_file(
        &self,
        device_key: &str,
        data: &DeviceKeyStatsFile,
    ) -> Result<(), StatsError> {
        fs::create_dir_all(&self.dir).map_err(StatsError::Io)?;
        let path = self.device_file_path(device_key);
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string(data).map_err(StatsError::Serialize)?;
        fs::write(&tmp, text).map_err(StatsError::Io)?;
        fs::rename(&tmp, &path).map_err(StatsError::Io)?;
        Ok(())
    }

    fn device_file_path(&self, device_key: &str) -> PathBuf {
        // "uid:0123..." → "uid_0123....json"; anything else is sanitized the
        // same way so a file name is always valid.
        let safe: String = device_key
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        self.dir.join(format!("{safe}.json"))
    }
}

impl Drop for KeyStatsStore {
    fn drop(&mut self) {
        self.flush_all();
    }
}

fn quarantine_file(path: &PathBuf) {
    let corrupt = path.with_extension("json.corrupt");
    let _ = fs::rename(path, corrupt);
}

/// "YYYY-MM-DD" minus `days`, using chrono via the time module's date math.
fn date_days_before(date: &str, days: u64) -> Option<String> {
    let parsed = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    let earlier = parsed.checked_sub_days(chrono::Days::new(days))?;
    Some(earlier.format("%Y-%m-%d").to_string())
}

#[derive(Debug, Error)]
pub enum StatsError {
    #[error("io error: {0}")]
    Io(std::io::Error),
    #[error("serialize error: {0}")]
    Serialize(serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(position: u8, delta: u16) -> KeyStatsEntry {
        KeyStatsEntry { position, delta }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "keylink-studio-stats-test-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn apply_and_summarize_today() {
        let dir = temp_dir("today");
        let mut store = KeyStatsStore::new(dir.clone(), Duration::from_secs(60));

        store.apply_diff("uid:1", &[entry(3, 10), entry(5, 2)], "2026-06-11");
        store.apply_diff("uid:1", &[entry(3, 1)], "2026-06-11");

        let summary = store.summary("uid:1", StatsPeriod::Today, "2026-06-11");
        assert_eq!(summary.total, 13);
        assert_eq!(
            summary.per_position,
            vec![
                PositionCount {
                    position: 3,
                    count: 11
                },
                PositionCount {
                    position: 5,
                    count: 2
                },
            ]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn period_filters_exclude_old_days() {
        let dir = temp_dir("period");
        let mut store = KeyStatsStore::new(dir.clone(), Duration::from_secs(60));

        store.apply_diff("uid:1", &[entry(1, 1)], "2026-06-01"); // outside 7d
        store.apply_diff("uid:1", &[entry(1, 2)], "2026-06-05"); // inside 7d
        store.apply_diff("uid:1", &[entry(1, 4)], "2026-06-11"); // today

        let today = store.summary("uid:1", StatsPeriod::Today, "2026-06-11");
        assert_eq!(today.total, 4);
        assert_eq!(today.days_covered, 1);

        let week = store.summary("uid:1", StatsPeriod::Last7Days, "2026-06-11");
        assert_eq!(week.total, 6);
        assert_eq!(week.days_covered, 2);

        let all = store.summary("uid:1", StatsPeriod::All, "2026-06-11");
        assert_eq!(all.total, 7);
        assert_eq!(all.days_covered, 3);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn flush_and_reload_round_trips() {
        let dir = temp_dir("reload");

        {
            let mut store = KeyStatsStore::new(dir.clone(), Duration::from_secs(60));
            store.apply_diff("uid:abcd", &[entry(2, 7)], "2026-06-11");
            store.flush_all();
        }

        let mut store = KeyStatsStore::new(dir.clone(), Duration::from_secs(60));
        let summary = store.summary("uid:abcd", StatsPeriod::All, "2026-06-11");
        assert_eq!(summary.total, 7);
        assert_eq!(store.device_keys(), vec!["uid:abcd".to_string()]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn drop_flushes_dirty_data() {
        let dir = temp_dir("drop");

        {
            let mut store = KeyStatsStore::new(dir.clone(), Duration::from_secs(60));
            store.apply_diff("uid:1", &[entry(1, 3)], "2026-06-11");
            // No explicit flush; Drop must persist.
        }

        let mut store = KeyStatsStore::new(dir.clone(), Duration::from_secs(60));
        assert_eq!(
            store.summary("uid:1", StatsPeriod::All, "2026-06-11").total,
            3
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_file_is_quarantined_and_reset() {
        let dir = temp_dir("corrupt");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("uid_1.json"), "not json").unwrap();

        let mut store = KeyStatsStore::new(dir.clone(), Duration::from_secs(60));
        let summary = store.summary("uid:1", StatsPeriod::All, "2026-06-11");

        assert_eq!(summary.total, 0);
        assert!(dir.join("uid_1.json.corrupt").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn flush_if_due_respects_interval() {
        let dir = temp_dir("interval");
        let mut store = KeyStatsStore::new(dir.clone(), Duration::from_secs(60));
        let start = Instant::now();

        store.apply_diff("uid:1", &[entry(1, 1)], "2026-06-11");
        store.flush_if_due(start); // first call always flushes
        assert!(store.dirty.is_empty());

        store.apply_diff("uid:1", &[entry(1, 1)], "2026-06-11");
        store.flush_if_due(start + Duration::from_secs(30));
        assert!(!store.dirty.is_empty()); // not due yet

        store.flush_if_due(start + Duration::from_secs(61));
        assert!(store.dirty.is_empty());
        let _ = fs::remove_dir_all(dir);
    }
}
