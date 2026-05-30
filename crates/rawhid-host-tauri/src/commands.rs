use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
};

use rawhid_host_core::{
    active_app::SystemActiveAppProvider,
    config::{load_config, AppConfig, ConfigPaths},
    hid::{HidDeviceManager, ProbeResult},
    runner::{RunEvent, Runner},
};
use tauri::{AppHandle, Emitter, State};

use crate::state::{add_log, AppState, LogEntry, MonitorCommand, MonitorStatus};

#[derive(Debug, Clone, serde::Serialize)]
pub struct RunningApp {
    pub exe: String,
    pub path: Option<String>,
    pub display_name: String,
    pub titles: Vec<String>,
}

#[tauri::command]
pub fn get_running_apps() -> Vec<RunningApp> {
    #[cfg(windows)]
    {
        windows_running_apps()
    }
    #[cfg(not(windows))]
    {
        vec![]
    }
}

#[cfg(windows)]
fn windows_running_apps() -> Vec<RunningApp> {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::{
        Foundation::{BOOL, HWND, LPARAM},
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW,
            GetWindowThreadProcessId, IsWindowVisible, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
        },
    };

    struct CollectData {
        entries: HashMap<String, RunningApp>,
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let data = &mut *(lparam.0 as *mut CollectData);

        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }

        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
            return BOOL(1);
        }

        let title_len = GetWindowTextLengthW(hwnd);
        if title_len == 0 {
            return BOOL(1);
        }
        let mut title_buf = vec![0u16; title_len as usize + 1];
        let copied = GetWindowTextW(hwnd, &mut title_buf);
        if copied == 0 {
            return BOOL(1);
        }
        title_buf.truncate(copied as usize);
        let title = OsString::from_wide(&title_buf)
            .to_string_lossy()
            .to_string();

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return BOOL(1);
        }

        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return BOOL(1);
        };
        let mut path_buf = vec![0u16; 32768];
        let mut path_len = path_buf.len() as u32;
        let path_str = if QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(path_buf.as_mut_ptr()),
            &mut path_len,
        )
        .is_ok()
            && path_len > 0
        {
            path_buf.truncate(path_len as usize);
            Some(OsString::from_wide(&path_buf).to_string_lossy().to_string())
        } else {
            None
        };
        let _ = windows::Win32::Foundation::CloseHandle(handle);

        let exe = path_str
            .as_ref()
            .and_then(|p| std::path::Path::new(p).file_name())
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        if exe.is_empty() {
            return BOOL(1);
        }

        let exe_lower = exe.to_lowercase();
        let system_procs = [
            "explorer.exe",
            "searchhost.exe",
            "searchapp.exe",
            "shellexperiencehost.exe",
            "startmenuexperiencehost.exe",
            "applicationframehost.exe",
            "systemsettings.exe",
            "textinputhost.exe",
            "lockapp.exe",
            "dwm.exe",
            "taskhostw.exe",
            "sihost.exe",
            "ctfmon.exe",
        ];
        if system_procs.contains(&exe_lower.as_str()) {
            return BOOL(1);
        }

        let entry = data.entries.entry(exe.clone()).or_insert_with(|| {
            let stem = std::path::Path::new(&exe)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| exe.clone());
            RunningApp {
                exe: exe.clone(),
                path: path_str,
                display_name: stem,
                titles: Vec::new(),
            }
        });

        if !entry.titles.contains(&title) && entry.titles.len() < 3 {
            entry.titles.push(title);
        }

        BOOL(1)
    }

    let mut data = CollectData {
        entries: HashMap::new(),
    };
    unsafe {
        let _ = EnumWindows(
            Some(enum_proc),
            LPARAM(&mut data as *mut CollectData as isize),
        );
    }

    let mut list: Vec<RunningApp> = data.entries.into_values().collect();
    list.sort_by(|a, b| {
        a.display_name
            .to_lowercase()
            .cmp(&b.display_name.to_lowercase())
    });
    list
}

pub fn load_initial_config() -> (AppConfig, Option<PathBuf>) {
    load_config(preferred_existing_config_path()).unwrap_or_default()
}

fn preferred_config_path() -> Option<PathBuf> {
    workspace_config_path().or_else(|| ConfigPaths::discover(None).selected_path())
}

fn preferred_existing_config_path() -> Option<PathBuf> {
    workspace_config_path().filter(|path| path.exists())
}

#[cfg(debug_assertions)]
fn workspace_config_path() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest_dir.parent()?.parent()?;
    Some(workspace.join("rawhid-host.toml"))
}

#[cfg(not(debug_assertions))]
fn workspace_config_path() -> Option<PathBuf> {
    None
}

#[tauri::command]
pub fn get_config(state: State<AppState>) -> AppConfig {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
pub fn get_config_path(state: State<AppState>) -> Option<String> {
    state
        .config_path
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
pub fn save_config(config: AppConfig, state: State<AppState>) -> Result<(), String> {
    let toml_str = toml::to_string_pretty(&config).map_err(|e| e.to_string())?;

    let path = {
        let config_path = state.config_path.lock().unwrap();
        match config_path.clone() {
            Some(p) => p,
            None => preferred_config_path().unwrap_or_else(|| PathBuf::from("rawhid-host.toml")),
        }
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, &toml_str).map_err(|e| e.to_string())?;

    *state.config.lock().unwrap() = config.clone();
    *state.config_path.lock().unwrap() = Some(path);

    if let Some(tx) = state.stop_tx.lock().unwrap().as_ref() {
        let _ = tx.send(MonitorCommand::UpdateConfig(config));
    }

    Ok(())
}

#[tauri::command]
pub fn reload_config(state: State<AppState>) -> Result<AppConfig, String> {
    let (config, path) =
        load_config(preferred_existing_config_path()).map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = config.clone();
    *state.config_path.lock().unwrap() = path;
    Ok(config)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConfigLocationResult {
    pub path: String,
    pub revealed: bool,
}

#[tauri::command]
pub fn show_config_file_location(state: State<AppState>) -> Result<ConfigLocationResult, String> {
    let path = {
        let config_path = state.config_path.lock().unwrap();
        match config_path.clone() {
            Some(p) => p,
            None => preferred_config_path().unwrap_or_else(|| PathBuf::from("rawhid-host.toml")),
        }
    };
    let revealed = reveal_config_path(&path);
    Ok(ConfigLocationResult {
        path: path.to_string_lossy().to_string(),
        revealed,
    })
}

#[cfg(windows)]
fn reveal_config_path(path: &std::path::Path) -> bool {
    std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", path.to_string_lossy()))
        .spawn()
        .is_ok()
}

#[cfg(not(windows))]
fn reveal_config_path(_path: &std::path::Path) -> bool {
    false
}

#[tauri::command]
pub fn get_status(state: State<AppState>) -> MonitorStatus {
    state.status.lock().unwrap().clone()
}

#[tauri::command]
pub fn get_log_entries(state: State<AppState>) -> Vec<LogEntry> {
    state.log_entries.lock().unwrap().iter().cloned().collect()
}

#[tauri::command]
pub fn probe_devices(state: State<AppState>) -> Result<Vec<ProbeResult>, String> {
    let hid_config = state.config.lock().unwrap().hid.clone();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let result = (|| -> Result<Vec<ProbeResult>, String> {
            let mut manager = HidDeviceManager::real(hid_config).map_err(|e| e.to_string())?;
            manager.probe().map_err(|e| e.to_string())
        })();
        let _ = tx.send(result);
    });

    rx.recv().map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn start_monitoring(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let already_running = state.stop_tx.lock().unwrap().is_some();
    if already_running {
        return Err("Already monitoring".to_string());
    }

    let config = state.config.lock().unwrap().clone();
    let status = Arc::clone(&state.status);
    let log_entries = Arc::clone(&state.log_entries);
    let log_counter = Arc::clone(&state.log_counter);
    let stop_tx_arc = Arc::clone(&state.stop_tx);

    let (tx, rx) = mpsc::channel();
    *stop_tx_arc.lock().unwrap() = Some(tx);

    thread::spawn(move || {
        run_monitor_loop(app, config, status, log_entries, log_counter, rx);
        *stop_tx_arc.lock().unwrap() = None;
    });

    Ok(())
}

#[tauri::command]
pub fn stop_monitoring(state: State<AppState>) -> Result<(), String> {
    let tx = state.stop_tx.lock().unwrap().take();
    match tx {
        Some(tx) => {
            let _ = tx.send(MonitorCommand::Stop);
            Ok(())
        }
        None => Err("Not monitoring".to_string()),
    }
}

#[tauri::command]
pub fn refresh_ai_usage(state: State<AppState>) -> Result<(), String> {
    if state
        .ai_usage_refreshing
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("refresh_in_progress".to_string());
    }
    let _guard = RefreshGuard(Arc::clone(&state.ai_usage_refreshing));
    let tx = state.stop_tx.lock().unwrap().clone();
    let Some(tx) = tx else {
        return Err("not_running".to_string());
    };
    let (reply_tx, reply_rx) = mpsc::channel();
    tx.send(MonitorCommand::RefreshAiUsage(reply_tx))
        .map_err(|_| "not_running".to_string())?;
    reply_rx.recv().map_err(|_| "refresh_failed".to_string())?
}

struct RefreshGuard(Arc<AtomicBool>);

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn rebuild_runner(
    config: &AppConfig,
) -> Result<Runner<SystemActiveAppProvider, rawhid_host_core::hid::RealHidTransport>, String> {
    let hid = HidDeviceManager::real(config.hid.clone()).map_err(|e| e.to_string())?;
    Ok(Runner::new(config.clone(), SystemActiveAppProvider, hid))
}

fn apply_monitor_config(
    app: &AppHandle,
    next_config: AppConfig,
    runner: &mut Runner<SystemActiveAppProvider, rawhid_host_core::hid::RealHidTransport>,
    interval: &mut Duration,
    status: &Arc<std::sync::Mutex<MonitorStatus>>,
) -> Result<(), String> {
    *runner = rebuild_runner(&next_config)?;
    *interval = Duration::from_millis(next_config.polling.interval_ms);

    let mut s = status.lock().unwrap();
    s.current_layer = None;
    s.current_rule = None;
    s.connected_devices = 0;
    s.connected_device_names = Vec::new();
    s.ai_usage = runner.ai_usage_statuses();
    s.last_error = None;
    let _ = app.emit("status-update", &*s);

    Ok(())
}

fn run_monitor_loop(
    app: AppHandle,
    config: AppConfig,
    status: Arc<std::sync::Mutex<MonitorStatus>>,
    log_entries: Arc<std::sync::Mutex<std::collections::VecDeque<LogEntry>>>,
    log_counter: Arc<std::sync::Mutex<u64>>,
    rx: mpsc::Receiver<MonitorCommand>,
) {
    let mut runner = match rebuild_runner(&config) {
        Ok(runner) => runner,
        Err(e) => {
            let msg = format!("HID init error: {}", e);
            let entry = add_log(&log_entries, &log_counter, "error", &msg);
            {
                let mut s = status.lock().unwrap();
                s.last_error = Some(msg);
                let _ = app.emit("status-update", &*s);
            }
            let _ = app.emit("log-added", entry);
            return;
        }
    };

    {
        let mut s = status.lock().unwrap();
        s.running = true;
        s.last_error = None;
        let _ = app.emit("status-update", &*s);
    }

    let entry = add_log(&log_entries, &log_counter, "info", "Monitoring started");
    let _ = app.emit("log-added", entry);

    let mut interval = Duration::from_millis(config.polling.interval_ms);

    loop {
        let mut should_stop = false;
        loop {
            match rx.try_recv() {
                Ok(MonitorCommand::Stop) | Err(mpsc::TryRecvError::Disconnected) => {
                    should_stop = true;
                    break;
                }
                Ok(MonitorCommand::UpdateConfig(next_config)) => {
                    match apply_monitor_config(
                        &app,
                        next_config,
                        &mut runner,
                        &mut interval,
                        &status,
                    ) {
                        Ok(()) => {}
                        Err(e) => {
                            let msg = format!("HID init error: {}", e);
                            {
                                let mut s = status.lock().unwrap();
                                s.last_error = Some(msg.clone());
                                let _ = app.emit("status-update", &*s);
                            }
                            let entry = add_log(&log_entries, &log_counter, "error", &msg);
                            let _ = app.emit("log-added", entry);
                        }
                    }
                }
                Ok(MonitorCommand::RefreshAiUsage(reply_tx)) => {
                    let result = runner
                        .refresh_ai_usage()
                        .then_some(())
                        .ok_or_else(|| "not_running".to_string());
                    let _ = reply_tx.send(result);
                    let mut s = status.lock().unwrap();
                    s.ai_usage = runner.ai_usage_statuses();
                    let _ = app.emit("status-update", &*s);
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        if should_stop {
            break;
        }

        match runner.tick() {
            Ok(RunEvent::SetLayer { layer, rule_name }) => {
                let devices = runner.verified_device_count();
                let device_names = runner.verified_device_names();
                let ai_usage = runner.ai_usage_statuses();
                {
                    let mut s = status.lock().unwrap();
                    s.current_layer = Some(layer);
                    s.current_rule = Some(rule_name.clone());
                    s.connected_devices = devices;
                    s.connected_device_names = device_names;
                    s.ai_usage = ai_usage;
                    s.last_error = None;
                    let _ = app.emit("status-update", &*s);
                }
                let msg = format!("Switched to layer {} (rule: {})", layer, rule_name);
                let entry = add_log(&log_entries, &log_counter, "info", &msg);
                let _ = app.emit("log-added", entry);
            }
            Ok(RunEvent::Clear) => {
                let devices = runner.verified_device_count();
                let device_names = runner.verified_device_names();
                let ai_usage = runner.ai_usage_statuses();
                {
                    let mut s = status.lock().unwrap();
                    s.current_layer = None;
                    s.current_rule = None;
                    s.connected_devices = devices;
                    s.connected_device_names = device_names;
                    s.ai_usage = ai_usage;
                    let _ = app.emit("status-update", &*s);
                }
            }
            Ok(RunEvent::Unchanged) => {
                let devices = runner.verified_device_count();
                let ai_usage = runner.ai_usage_statuses();
                let mut s = status.lock().unwrap();
                let ai_usage_changed = s.ai_usage != ai_usage;
                if s.connected_devices != devices || ai_usage_changed {
                    let device_names = runner.verified_device_names();
                    s.connected_devices = devices;
                    s.connected_device_names = device_names;
                    s.ai_usage = ai_usage;
                    let _ = app.emit("status-update", &*s);
                }
            }
            Err(e) => {
                let msg = format!("Error: {}", e);
                {
                    let mut s = status.lock().unwrap();
                    s.last_error = Some(msg.clone());
                    let _ = app.emit("status-update", &*s);
                }
                let entry = add_log(&log_entries, &log_counter, "error", &msg);
                let _ = app.emit("log-added", entry);
            }
        }

        match rx.recv_timeout(interval) {
            Ok(MonitorCommand::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(MonitorCommand::UpdateConfig(next_config)) => {
                match apply_monitor_config(&app, next_config, &mut runner, &mut interval, &status) {
                    Ok(()) => {}
                    Err(e) => {
                        let msg = format!("HID init error: {}", e);
                        {
                            let mut s = status.lock().unwrap();
                            s.last_error = Some(msg.clone());
                            let _ = app.emit("status-update", &*s);
                        }
                        let entry = add_log(&log_entries, &log_counter, "error", &msg);
                        let _ = app.emit("log-added", entry);
                    }
                }
            }
            Ok(MonitorCommand::RefreshAiUsage(reply_tx)) => {
                let result = runner
                    .refresh_ai_usage()
                    .then_some(())
                    .ok_or_else(|| "not_running".to_string());
                let _ = reply_tx.send(result);
                let mut s = status.lock().unwrap();
                s.ai_usage = runner.ai_usage_statuses();
                let _ = app.emit("status-update", &*s);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }

    {
        let mut s = status.lock().unwrap();
        s.running = false;
        s.connected_devices = 0;
        s.connected_device_names = Vec::new();
        s.current_layer = None;
        s.current_rule = None;
        s.ai_usage = Vec::new();
        let _ = app.emit("status-update", &*s);
    }

    let entry = add_log(&log_entries, &log_counter, "info", "Monitoring stopped");
    let _ = app.emit("log-added", entry);
}
