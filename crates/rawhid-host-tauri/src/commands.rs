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
    ai_usage::{AiUsageRefreshError, AiUsageRuntime, AiUsageShared},
    config::{load_config, AppConfig, ConfigPaths},
    hid::{HidDeviceManager, ProbeResult},
    runner::{RunEvent, Runner},
    studio::{
        probe_usb_serial_devices, read_keymap_for_device, StudioDeviceStatus, StudioError,
        StudioKeymapSnapshot,
    },
};
use tauri::{AppHandle, Emitter, State};

use crate::foreground::ForegroundWatcher;
use crate::state::{add_log, AppState, LogEntry, MonitorCommand, MonitorStatus};
use crate::{icon, startup};

type MonitorRunner = Runner<SystemActiveAppProvider, rawhid_host_core::hid::RealHidTransport>;

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

    let ai_usage_shared = restart_ai_usage_runtime(&state, &config);

    if let Some(tx) = state.stop_tx.lock().unwrap().as_ref() {
        let _ = tx.send(MonitorCommand::UpdateConfig(config, ai_usage_shared));
    } else {
        update_ai_usage_status(&state);
    }

    Ok(())
}

#[tauri::command]
pub fn reload_config(state: State<AppState>) -> Result<AppConfig, String> {
    let (config, path) =
        load_config(preferred_existing_config_path()).map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = config.clone();
    *state.config_path.lock().unwrap() = path;
    restart_ai_usage_runtime(&state, &config);
    update_ai_usage_status(&state);
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
    update_ai_usage_status(&state);
    state.status.lock().unwrap().clone()
}

#[tauri::command]
pub fn get_log_entries(state: State<AppState>) -> Vec<LogEntry> {
    state.log_entries.lock().unwrap().iter().cloned().collect()
}

#[tauri::command]
pub async fn probe_devices(state: State<'_, AppState>) -> Result<Vec<ProbeResult>, String> {
    let hid_config = state.config.lock().unwrap().hid.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut manager = HidDeviceManager::real(hid_config).map_err(|e| e.to_string())?;
        manager.probe().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn probe_studio_devices(
    state: State<'_, AppState>,
) -> Result<Vec<StudioDeviceStatus>, String> {
    let studio_config = state.config.lock().unwrap().studio.clone();
    tauri::async_runtime::spawn_blocking(move || probe_usb_serial_devices(&studio_config))
        .await
        .map_err(|_| "studio_probe_failed".to_string())
}

#[tauri::command]
pub async fn read_studio_keymap(
    device_id: String,
    state: State<'_, AppState>,
) -> Result<StudioKeymapSnapshot, String> {
    let studio_config = state.config.lock().unwrap().studio.clone();
    tauri::async_runtime::spawn_blocking(move || {
        read_keymap_for_device(&device_id, &studio_config).map_err(studio_error_code)
    })
    .await
    .map_err(|_| "studio_read_failed".to_string())?
}

fn studio_error_code(error: StudioError) -> String {
    match error {
        StudioError::DeviceNotFound => "device_not_found",
        StudioError::Locked => "locked",
        StudioError::Timeout => "timeout",
        StudioError::RpcFailed => "rpc_failed",
    }
    .to_string()
}
#[tauri::command]
pub fn start_monitoring(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    begin_monitoring(app, state.inner())
}

/// Shared monitoring-start logic, callable from the command, the tray menu, and
/// the auto-start-on-launch path.
pub fn begin_monitoring(app: AppHandle, state: &AppState) -> Result<(), String> {
    let already_running = state.stop_tx.lock().unwrap().is_some();
    if already_running {
        return Err("Already monitoring".to_string());
    }

    let config = state.config.lock().unwrap().clone();
    let status = Arc::clone(&state.status);
    let log_entries = Arc::clone(&state.log_entries);
    let log_counter = Arc::clone(&state.log_counter);
    let stop_tx_arc = Arc::clone(&state.stop_tx);
    let ai_usage_shared = state
        .ai_usage_runtime
        .lock()
        .unwrap()
        .as_ref()
        .map(AiUsageRuntime::shared);

    let (tx, rx) = mpsc::channel();
    let watcher_tx = tx.clone();
    *stop_tx_arc.lock().unwrap() = Some(tx);

    thread::spawn(move || {
        // The foreground watcher delivers instant wake-ups; polling stays as a
        // fallback. It is unhooked when this scope ends (monitoring stops).
        let _foreground_watcher = ForegroundWatcher::new(watcher_tx);
        run_monitor_loop(
            app,
            config,
            status,
            log_entries,
            log_counter,
            rx,
            ai_usage_shared,
        );
        *stop_tx_arc.lock().unwrap() = None;
    });

    Ok(())
}

#[tauri::command]
pub fn stop_monitoring(state: State<AppState>) -> Result<(), String> {
    stop_monitoring_internal(state.inner())
}

/// Shared monitoring-stop logic, callable from the command and the tray menu.
pub fn stop_monitoring_internal(state: &AppState) -> Result<(), String> {
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
pub fn refresh_ai_usage(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    if state
        .ai_usage_refreshing
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("refresh_in_progress".to_string());
    }

    // Capture the snapshot generation before requesting a refresh so the watcher
    // thread can detect when the worker has produced fresh data.
    let baseline = {
        let runtime = state.ai_usage_runtime.lock().unwrap();
        let Some(runtime) = runtime.as_ref() else {
            state.ai_usage_refreshing.store(false, Ordering::SeqCst);
            return Err("source_disabled".to_string());
        };
        let generation = runtime.shared().generation();
        if let Err(error) = runtime.refresh() {
            state.ai_usage_refreshing.store(false, Ordering::SeqCst);
            return Err(refresh_error_code(error));
        }
        generation
    };

    // Wait for completion off the command thread, then push a status-update so
    // the UI reflects new data even while monitoring is stopped (no tick loop).
    spawn_ai_refresh_watcher(
        app,
        Arc::clone(&state.config),
        Arc::clone(&state.ai_usage_runtime),
        Arc::clone(&state.status),
        Arc::clone(&state.ai_usage_refreshing),
        baseline,
    );
    Ok(())
}

fn spawn_ai_refresh_watcher(
    app: AppHandle,
    config: Arc<std::sync::Mutex<AppConfig>>,
    runtime: Arc<std::sync::Mutex<Option<AiUsageRuntime>>>,
    status: Arc<std::sync::Mutex<MonitorStatus>>,
    refreshing: Arc<AtomicBool>,
    baseline: u64,
) {
    thread::spawn(move || {
        let _guard = RefreshGuard(refreshing);
        // Poll for the worker to publish a new snapshot generation (best effort).
        for _ in 0..100 {
            let changed = runtime
                .lock()
                .unwrap()
                .as_ref()
                .map(|runtime| runtime.shared().generation() != baseline)
                .unwrap_or(true);
            if changed {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }

        let stale_after_sec = config.lock().unwrap().ai_usage.stale_after_sec;
        let statuses = runtime
            .lock()
            .unwrap()
            .as_ref()
            .map(|runtime| runtime.statuses(stale_after_sec))
            .unwrap_or_default();
        let snapshot = {
            let mut s = status.lock().unwrap();
            s.ai_usage = statuses;
            s.clone()
        };
        let _ = app.emit("status-update", &snapshot);
    });
}

fn restart_ai_usage_runtime(state: &AppState, config: &AppConfig) -> Option<AiUsageShared> {
    let runtime = AiUsageRuntime::start(config.ai_usage.clone());
    let shared = runtime.as_ref().map(AiUsageRuntime::shared);
    *state.ai_usage_runtime.lock().unwrap() = runtime;
    shared
}

fn update_ai_usage_status(state: &AppState) {
    let config = state.config.lock().unwrap().clone();
    let ai_usage = state
        .ai_usage_runtime
        .lock()
        .unwrap()
        .as_ref()
        .map(|runtime| runtime.statuses(config.ai_usage.stale_after_sec))
        .unwrap_or_default();
    state.status.lock().unwrap().ai_usage = ai_usage;
}

struct RefreshGuard(Arc<AtomicBool>);

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn rebuild_runner(
    config: &AppConfig,
    ai_usage_shared: Option<AiUsageShared>,
) -> Result<MonitorRunner, String> {
    let hid = HidDeviceManager::real(config.hid.clone()).map_err(|e| e.to_string())?;
    Ok(Runner::new_with_ai_usage_shared(
        config.clone(),
        SystemActiveAppProvider,
        hid,
        rawhid_host_core::time::SystemClock,
        ai_usage_shared,
    ))
}

/// Copy the runner's device/AI-usage view into the shared status.
fn apply_runner_view(s: &mut MonitorStatus, runner: &MonitorRunner) {
    s.connected_devices = runner.verified_device_count();
    s.connected_device_names = runner.verified_device_names();
    s.host_link_devices = runner.verified_devices();
    s.ai_usage = runner.ai_usage_statuses();
}

fn apply_monitor_config(
    app: &AppHandle,
    next_config: AppConfig,
    runner: &mut MonitorRunner,
    interval: &mut Duration,
    status: &Arc<std::sync::Mutex<MonitorStatus>>,
    ai_usage_shared: Option<AiUsageShared>,
) -> Result<(), String> {
    *runner = rebuild_runner(&next_config, ai_usage_shared)?;
    *interval = Duration::from_millis(next_config.polling.interval_ms);

    let mut s = status.lock().unwrap();
    s.current_layer = None;
    s.current_rule = None;
    s.connected_devices = 0;
    s.connected_device_names = Vec::new();
    s.host_link_devices = Vec::new();
    s.ai_usage = runner.ai_usage_statuses();
    s.last_error = None;
    let _ = app.emit("status-update", &*s);

    Ok(())
}

/// Apply a single monitor command. Returns `true` when the loop should stop.
fn process_command(
    command: MonitorCommand,
    app: &AppHandle,
    runner: &mut MonitorRunner,
    interval: &mut Duration,
    status: &Arc<std::sync::Mutex<MonitorStatus>>,
    log_entries: &Arc<std::sync::Mutex<std::collections::VecDeque<LogEntry>>>,
    log_counter: &Arc<std::sync::Mutex<u64>>,
) -> bool {
    match command {
        MonitorCommand::Stop => true,
        MonitorCommand::ForegroundChanged => false,
        MonitorCommand::UpdateConfig(next_config, ai_usage_shared) => {
            if let Err(e) = apply_monitor_config(
                app,
                next_config,
                runner,
                interval,
                status,
                ai_usage_shared,
            ) {
                let msg = format!("HID init error: {}", e);
                {
                    let mut s = status.lock().unwrap();
                    s.last_error = Some(msg.clone());
                    let _ = app.emit("status-update", &*s);
                }
                let entry = add_log(log_entries, log_counter, "error", &msg);
                let _ = app.emit("log-added", entry);
            }
            false
        }
    }
}

fn run_monitor_loop(
    app: AppHandle,
    config: AppConfig,
    status: Arc<std::sync::Mutex<MonitorStatus>>,
    log_entries: Arc<std::sync::Mutex<std::collections::VecDeque<LogEntry>>>,
    log_counter: Arc<std::sync::Mutex<u64>>,
    rx: mpsc::Receiver<MonitorCommand>,
    ai_usage_shared: Option<AiUsageShared>,
) {
    let mut runner = match rebuild_runner(&config, ai_usage_shared) {
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
                Ok(command) => {
                    if process_command(
                        command,
                        &app,
                        &mut runner,
                        &mut interval,
                        &status,
                        &log_entries,
                        &log_counter,
                    ) {
                        should_stop = true;
                        break;
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    should_stop = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        if should_stop {
            break;
        }

        match runner.tick() {
            Ok(RunEvent::SetLayer { layer, rule_name }) => {
                {
                    let mut s = status.lock().unwrap();
                    s.current_layer = Some(layer);
                    s.current_rule = Some(rule_name.clone());
                    apply_runner_view(&mut s, &runner);
                    s.last_error = None;
                    let _ = app.emit("status-update", &*s);
                }
                let msg = format!("Switched to layer {} (rule: {})", layer, rule_name);
                let entry = add_log(&log_entries, &log_counter, "info", &msg);
                let _ = app.emit("log-added", entry);
            }
            Ok(RunEvent::Clear) => {
                let mut s = status.lock().unwrap();
                s.current_layer = None;
                s.current_rule = None;
                apply_runner_view(&mut s, &runner);
                let _ = app.emit("status-update", &*s);
            }
            Ok(RunEvent::Unchanged) => {
                let devices = runner.verified_device_count();
                let host_link_devices = runner.verified_devices();
                let ai_usage = runner.ai_usage_statuses();
                let mut s = status.lock().unwrap();
                if s.connected_devices != devices
                    || s.ai_usage != ai_usage
                    || s.host_link_devices != host_link_devices
                {
                    apply_runner_view(&mut s, &runner);
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
            Ok(command) => {
                if process_command(
                    command,
                    &app,
                    &mut runner,
                    &mut interval,
                    &status,
                    &log_entries,
                    &log_counter,
                ) {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }

    {
        let mut s = status.lock().unwrap();
        s.running = false;
        s.connected_devices = 0;
        s.connected_device_names = Vec::new();
        s.host_link_devices = Vec::new();
        s.current_layer = None;
        s.current_rule = None;
        let _ = app.emit("status-update", &*s);
    }

    let entry = add_log(&log_entries, &log_counter, "info", "Monitoring stopped");
    let _ = app.emit("log-added", entry);
}

#[tauri::command]
pub fn get_app_icons(paths: Vec<String>) -> std::collections::HashMap<String, String> {
    let mut icons = std::collections::HashMap::new();
    for path in paths {
        if let Some(data_url) = icon::app_icon_data_url(&path) {
            icons.insert(path, data_url);
        }
    }
    icons
}

#[tauri::command]
pub fn get_launch_at_login() -> bool {
    startup::is_launch_at_login()
}

#[tauri::command]
pub fn set_launch_at_login(enabled: bool) -> Result<(), String> {
    startup::set_launch_at_login(enabled)
}

fn refresh_error_code(error: AiUsageRefreshError) -> String {
    match error {
        AiUsageRefreshError::InProgress => "refresh_in_progress",
        AiUsageRefreshError::Stopped => "not_running",
    }
    .to_string()
}
