use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use rawhid_host_core::config::AppConfig;

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
    pub current_layer: Option<u8>,
    pub current_rule: Option<String>,
    pub last_error: Option<String>,
}

impl Default for MonitorStatus {
    fn default() -> Self {
        Self {
            running: false,
            connected_devices: 0,
            connected_device_names: Vec::new(),
            current_layer: None,
            current_rule: None,
            last_error: None,
        }
    }
}

pub struct AppState {
    pub config: Arc<Mutex<AppConfig>>,
    pub config_path: Arc<Mutex<Option<PathBuf>>>,
    pub status: Arc<Mutex<MonitorStatus>>,
    pub log_entries: Arc<Mutex<VecDeque<LogEntry>>>,
    pub log_counter: Arc<Mutex<u64>>,
    pub stop_tx: Arc<Mutex<Option<std::sync::mpsc::Sender<MonitorCommand>>>>,
}

#[derive(Debug, Clone)]
pub enum MonitorCommand {
    Stop,
    UpdateConfig(AppConfig),
}

impl AppState {
    pub fn new(config: AppConfig, config_path: Option<PathBuf>) -> Self {
        Self {
            config: Arc::new(Mutex::new(config)),
            config_path: Arc::new(Mutex::new(config_path)),
            status: Arc::new(Mutex::new(MonitorStatus::default())),
            log_entries: Arc::new(Mutex::new(VecDeque::new())),
            log_counter: Arc::new(Mutex::new(0)),
            stop_tx: Arc::new(Mutex::new(None)),
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
