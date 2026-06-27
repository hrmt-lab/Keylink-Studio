use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(debug_assertions)]
use rawhid_host_core::hid::DeviceInfo;
use rawhid_host_core::{
    active_app::SystemActiveAppProvider,
    ai_usage::{AiUsageRefreshError, AiUsageRuntime, AiUsageShared},
    config::{load_config, ActionsConfig, AppConfig, ConfigPaths},
    hid::{HidDeviceManager, ProbeResult},
    packet::UplinkPacket,
    runner::{uplink_device_key, RunEvent, Runner},
    stats::{KeyStatsSummary, SharedKeyStatsStore, StatsPeriod},
    studio::{
        key_catalog, keymap_backup_from_snapshot, parse_keymap_backup,
        probe_studio_devices as probe_all_studio_devices, read_keymap_for_device,
        resolve_behavior_labels_for_device, serialize_keymap_backup, EditBehavior, KeyCatalogEntry,
        KeymapFileError, RestoreReport, StudioBindingLabelPatch, StudioDeviceStatus,
        StudioEditSession, StudioError, StudioKeymapSnapshot, StudioRawBinding,
        KEYMAP_BACKUP_MAX_BYTES,
    },
};
use tauri::{AppHandle, Emitter, State};

use crate::foreground::ForegroundWatcher;
use crate::state::{add_log, AppState, LogEntry, MonitorCommand, MonitorStatus};
use crate::{actions, icon, startup};

type MonitorRunner = Runner<SystemActiveAppProvider, rawhid_host_core::hid::RealHidTransport>;

/// Shared handles the monitor loop needs beyond its own status/log arcs,
/// mainly for executing HOST_ACTION packets and persisting key stats.
pub struct MonitorExtras {
    pub config: Arc<std::sync::Mutex<AppConfig>>,
    pub ai_usage_runtime: Arc<std::sync::Mutex<Option<AiUsageRuntime>>>,
    pub ai_usage_refreshing: Arc<AtomicBool>,
    pub key_stats: SharedKeyStatsStore,
}

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
    Some(workspace.join("keylink-studio.toml"))
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
            None => preferred_config_path().unwrap_or_else(|| PathBuf::from("keylink-studio.toml")),
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
            None => preferred_config_path().unwrap_or_else(|| PathBuf::from("keylink-studio.toml")),
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
    if state.studio_edit.lock().unwrap().is_some() {
        return Err("port_busy".to_string());
    }
    let studio_config = state.config.lock().unwrap().studio.clone();
    tauri::async_runtime::spawn_blocking(move || probe_all_studio_devices(&studio_config))
        .await
        .map_err(|_| "studio_probe_failed".to_string())
}

#[tauri::command]
pub async fn read_studio_keymap(
    device_id: String,
    state: State<'_, AppState>,
) -> Result<StudioKeymapSnapshot, String> {
    let edit = Arc::clone(&state.studio_edit);
    let studio_config = state.config.lock().unwrap().studio.clone();
    tauri::async_runtime::spawn_blocking(move || {
        {
            let mut guard = edit.lock().unwrap();
            if let Some(session) = guard.as_mut() {
                if session.device_id == device_id {
                    return session.snapshot().map_err(studio_error_code);
                }
                return Err("port_busy".to_string());
            }
        }
        read_keymap_for_device(&device_id, &studio_config).map_err(studio_error_code)
    })
    .await
    .map_err(|_| "studio_read_failed".to_string())?
}

#[tauri::command]
pub async fn studio_export_keymap(
    device_id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let edit = Arc::clone(&state.studio_edit);
    let studio_config = state.config.lock().unwrap().studio.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = {
            let mut guard = edit.lock().unwrap();
            if let Some(session) = guard.as_mut() {
                if session.device_id == device_id {
                    session.snapshot().map_err(studio_error_code)?
                } else {
                    return Err(studio_error_code(StudioError::PortBusy));
                }
            } else {
                read_keymap_for_device(&device_id, &studio_config).map_err(studio_error_code)?
            }
        };
        let backup =
            keymap_backup_from_snapshot(&snapshot, Default::default(), env!("CARGO_PKG_VERSION"));
        let text = serialize_keymap_backup(&backup).map_err(keymap_file_error_code)?;
        write_keymap_backup_file(&PathBuf::from(path), &text)
    })
    .await
    .map_err(|_| "keymap_export_failed".to_string())?
}

#[tauri::command]
pub async fn studio_preview_keymap_restore(
    device_id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<RestoreReport, String> {
    let edit = Arc::clone(&state.studio_edit);
    let studio_config = state.config.lock().unwrap().studio.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let backup = read_keymap_backup_file(&PathBuf::from(path))?;
        let mut guard = edit.lock().unwrap();
        let session = guard
            .as_mut()
            .ok_or_else(|| studio_error_code(StudioError::NoEditSession))?;
        if session.device_id != device_id {
            return Err(studio_error_code(StudioError::SessionDeviceMismatch));
        }
        let snapshot = session.snapshot().map_err(studio_error_code)?;
        let ids = backup_behavior_ids(&backup);
        let target_names = session
            .resolve_behavior_names(
                &ids,
                Duration::from_millis(studio_config.keymap_read_timeout_ms.max(1)),
            )
            .map_err(studio_error_code)?;
        Ok(
            rawhid_host_core::studio::plan_keymap_restore(
                &snapshot,
                target_names.as_ref(),
                &backup,
            )
            .report,
        )
    })
    .await
    .map_err(|_| "keymap_restore_preview_failed".to_string())?
}

#[tauri::command]
pub async fn studio_apply_keymap_restore(
    device_id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<(StudioKeymapSnapshot, RestoreReport), String> {
    let edit = Arc::clone(&state.studio_edit);
    let studio_config = state.config.lock().unwrap().studio.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let backup = read_keymap_backup_file(&PathBuf::from(path))?;
        let mut guard = edit.lock().unwrap();
        let session = guard
            .as_mut()
            .ok_or_else(|| studio_error_code(StudioError::NoEditSession))?;
        if session.device_id != device_id {
            return Err(studio_error_code(StudioError::SessionDeviceMismatch));
        }
        let snapshot = session.snapshot().map_err(studio_error_code)?;
        let ids = backup_behavior_ids(&backup);
        let target_names = session
            .resolve_behavior_names(
                &ids,
                Duration::from_millis(studio_config.keymap_read_timeout_ms.max(1)),
            )
            .map_err(studio_error_code)?;
        let plan = rawhid_host_core::studio::plan_keymap_restore(
            &snapshot,
            target_names.as_ref(),
            &backup,
        );
        if !plan.report.can_apply {
            return Err("restore_structure_mismatch".to_string());
        }
        let next = session
            .apply_raw_writes(&plan.writes)
            .map_err(studio_error_code)?;
        Ok((next, plan.report))
    })
    .await
    .map_err(|_| "keymap_restore_apply_failed".to_string())?
}

fn backup_behavior_ids(backup: &rawhid_host_core::studio::KeymapBackup) -> BTreeSet<i32> {
    backup
        .layers
        .iter()
        .flat_map(|layer| layer.bindings.iter().map(|binding| binding.behavior_id))
        .collect()
}

fn read_keymap_backup_file(
    path: &std::path::Path,
) -> Result<rawhid_host_core::studio::KeymapBackup, String> {
    let metadata = std::fs::metadata(path).map_err(|_| "keymap_invalid_path".to_string())?;
    if !metadata.is_file() {
        return Err("keymap_invalid_path".to_string());
    }
    if metadata.len() > KEYMAP_BACKUP_MAX_BYTES as u64 {
        return Err("keymap_file_too_large".to_string());
    }
    let text = std::fs::read_to_string(path).map_err(|_| "keymap_invalid_file".to_string())?;
    parse_keymap_backup(&text).map_err(keymap_file_error_code)
}

fn write_keymap_backup_file(path: &std::path::Path, text: &str) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Err("keymap_invalid_path".to_string());
    };
    if !parent.is_dir() {
        return Err("keymap_invalid_path".to_string());
    }
    std::fs::write(path, text).map_err(|_| "keymap_invalid_path".to_string())
}

fn keymap_file_error_code(error: KeymapFileError) -> String {
    match error {
        KeymapFileError::InvalidFile => "keymap_invalid_file",
        KeymapFileError::UnsupportedVersion => "keymap_unsupported_version",
        KeymapFileError::FileTooLarge => "keymap_file_too_large",
    }
    .to_string()
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EditBehaviorDto {
    KeyPress {
        hid_usage: u32,
    },
    Transparent,
    None,
    MomentaryLayer {
        target_layer_index: u32,
    },
    ToggleLayer {
        target_layer_index: u32,
    },
    ToLayer {
        target_layer_index: u32,
    },
    ModTap {
        hold_hid_usage: u32,
        tap_hid_usage: u32,
    },
    LayerTap {
        target_layer_index: u32,
        tap_hid_usage: u32,
    },
    StickyKey {
        hid_usage: u32,
    },
    StickyLayer {
        target_layer_index: u32,
    },
    Bluetooth {
        command: u32,
        value: u32,
    },
    OutputSelection {
        value: u32,
    },
    MouseKeyPress {
        value: u32,
    },
    MouseMove {
        value: u32,
    },
    MouseScroll {
        value: u32,
    },
    CapsWord,
    KeyRepeat,
    Reset,
    Bootloader,
    StudioUnlock,
    GraveEscape,
}

impl From<EditBehaviorDto> for EditBehavior {
    fn from(value: EditBehaviorDto) -> Self {
        match value {
            EditBehaviorDto::KeyPress { hid_usage } => EditBehavior::KeyPress(hid_usage),
            EditBehaviorDto::Transparent => EditBehavior::Transparent,
            EditBehaviorDto::None => EditBehavior::None,
            EditBehaviorDto::MomentaryLayer { target_layer_index } => {
                EditBehavior::MomentaryLayer(target_layer_index)
            }
            EditBehaviorDto::ToggleLayer { target_layer_index } => {
                EditBehavior::ToggleLayer(target_layer_index)
            }
            EditBehaviorDto::ToLayer { target_layer_index } => {
                EditBehavior::ToLayer(target_layer_index)
            }
            EditBehaviorDto::ModTap {
                hold_hid_usage,
                tap_hid_usage,
            } => EditBehavior::ModTap {
                hold: hold_hid_usage,
                tap: tap_hid_usage,
            },
            EditBehaviorDto::LayerTap {
                target_layer_index,
                tap_hid_usage,
            } => EditBehavior::LayerTap {
                target_layer_index,
                tap: tap_hid_usage,
            },
            EditBehaviorDto::StickyKey { hid_usage } => EditBehavior::StickyKey(hid_usage),
            EditBehaviorDto::StickyLayer { target_layer_index } => {
                EditBehavior::StickyLayer(target_layer_index)
            }
            EditBehaviorDto::Bluetooth { command, value } => {
                EditBehavior::Bluetooth { command, value }
            }
            EditBehaviorDto::OutputSelection { value } => EditBehavior::OutputSelection(value),
            EditBehaviorDto::MouseKeyPress { value } => EditBehavior::MouseKeyPress(value),
            EditBehaviorDto::MouseMove { value } => EditBehavior::MouseMove(value),
            EditBehaviorDto::MouseScroll { value } => EditBehavior::MouseScroll(value),
            EditBehaviorDto::CapsWord => EditBehavior::CapsWord,
            EditBehaviorDto::KeyRepeat => EditBehavior::KeyRepeat,
            EditBehaviorDto::Reset => EditBehavior::Reset,
            EditBehaviorDto::Bootloader => EditBehavior::Bootloader,
            EditBehaviorDto::StudioUnlock => EditBehavior::StudioUnlock,
            EditBehaviorDto::GraveEscape => EditBehavior::GraveEscape,
        }
    }
}

#[tauri::command]
pub fn studio_key_catalog() -> Vec<KeyCatalogEntry> {
    key_catalog()
}

#[tauri::command]
pub async fn resolve_studio_behavior_labels(
    device_id: String,
    raw_bindings: Vec<StudioRawBinding>,
    state: State<'_, AppState>,
) -> Result<Vec<StudioBindingLabelPatch>, String> {
    let studio_config = state.config.lock().unwrap().studio.clone();
    let edit = Arc::clone(&state.studio_edit);
    tauri::async_runtime::spawn_blocking(move || {
        {
            let mut guard = edit.lock().unwrap();
            if let Some(session) = guard.as_mut() {
                if session.device_id == device_id {
                    return session
                        .resolve_behavior_labels(
                            raw_bindings,
                            Duration::from_millis(studio_config.keymap_read_timeout_ms.max(1)),
                        )
                        .map_err(studio_error_code);
                }
                return Err(studio_error_code(StudioError::PortBusy));
            }
        }
        resolve_behavior_labels_for_device(&device_id, raw_bindings, &studio_config)
            .map_err(studio_error_code)
    })
    .await
    .map_err(|_| "studio_behavior_resolve_failed".to_string())?
}

#[tauri::command]
pub async fn studio_begin_edit(
    device_id: String,
    force_discard: bool,
    label_patches: Vec<StudioBindingLabelPatch>,
    state: State<'_, AppState>,
) -> Result<StudioKeymapSnapshot, String> {
    let studio_config = state.config.lock().unwrap().studio.clone();
    let edit = Arc::clone(&state.studio_edit);
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = edit.lock().unwrap();
        if let Some(session) = guard.as_mut() {
            if session.device_id == device_id {
                session.seed_behavior_labels(&label_patches);
                if force_discard {
                    return session.discard().map_err(studio_error_code);
                }
                return session.snapshot().map_err(studio_error_code);
            }
            if force_discard {
                let _ = session.discard();
                *guard = None;
            } else if session.has_unsaved().map_err(studio_error_code)? {
                return Err(studio_error_code(StudioError::UnsavedChangesExist));
            } else {
                *guard = None;
            }
        }

        let (session, snapshot) = StudioEditSession::open_with_snapshot(&device_id, &studio_config)
            .map_err(studio_error_code)?;
        let mut session = session;
        session.seed_behavior_labels(&label_patches);
        *guard = Some(session);
        Ok(snapshot)
    })
    .await
    .map_err(|_| "studio_begin_edit_failed".to_string())?
}

#[tauri::command]
pub async fn studio_set_key(
    device_id: String,
    layer_id: u32,
    position: i32,
    behavior: EditBehaviorDto,
    state: State<'_, AppState>,
) -> Result<StudioKeymapSnapshot, String> {
    let edit = Arc::clone(&state.studio_edit);
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = edit.lock().unwrap();
        let session = guard
            .as_mut()
            .ok_or_else(|| studio_error_code(StudioError::NoEditSession))?;
        if session.device_id != device_id {
            return Err(studio_error_code(StudioError::SessionDeviceMismatch));
        }
        session
            .set_binding(layer_id, position, behavior.into())
            .map_err(studio_error_code)
    })
    .await
    .map_err(|_| "studio_set_key_failed".to_string())?
}

#[tauri::command]
pub async fn studio_add_layer(
    device_id: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<StudioKeymapSnapshot, String> {
    let edit = Arc::clone(&state.studio_edit);
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = edit.lock().unwrap();
        let session = guard
            .as_mut()
            .ok_or_else(|| studio_error_code(StudioError::NoEditSession))?;
        if session.device_id != device_id {
            return Err(studio_error_code(StudioError::SessionDeviceMismatch));
        }
        session.add_layer(name).map_err(studio_error_code)
    })
    .await
    .map_err(|_| "studio_add_layer_failed".to_string())?
}

#[tauri::command]
pub async fn studio_rename_layer(
    device_id: String,
    layer_id: u32,
    name: String,
    state: State<'_, AppState>,
) -> Result<StudioKeymapSnapshot, String> {
    let edit = Arc::clone(&state.studio_edit);
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = edit.lock().unwrap();
        let session = guard
            .as_mut()
            .ok_or_else(|| studio_error_code(StudioError::NoEditSession))?;
        if session.device_id != device_id {
            return Err(studio_error_code(StudioError::SessionDeviceMismatch));
        }
        session
            .rename_layer(layer_id, name)
            .map_err(studio_error_code)
    })
    .await
    .map_err(|_| "studio_rename_layer_failed".to_string())?
}

#[tauri::command]
pub async fn studio_remove_layer(
    device_id: String,
    layer_index: u32,
    state: State<'_, AppState>,
) -> Result<StudioKeymapSnapshot, String> {
    let edit = Arc::clone(&state.studio_edit);
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = edit.lock().unwrap();
        let session = guard
            .as_mut()
            .ok_or_else(|| studio_error_code(StudioError::NoEditSession))?;
        if session.device_id != device_id {
            return Err(studio_error_code(StudioError::SessionDeviceMismatch));
        }
        session.remove_layer(layer_index).map_err(studio_error_code)
    })
    .await
    .map_err(|_| "studio_remove_layer_failed".to_string())?
}

#[tauri::command]
pub async fn studio_save_changes(
    device_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let edit = Arc::clone(&state.studio_edit);
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = edit.lock().unwrap();
        let session = guard
            .as_mut()
            .ok_or_else(|| studio_error_code(StudioError::NoEditSession))?;
        if session.device_id != device_id {
            return Err(studio_error_code(StudioError::SessionDeviceMismatch));
        }
        session.save().map_err(studio_error_code)
    })
    .await
    .map_err(|_| "studio_save_failed".to_string())?
}

#[tauri::command]
pub async fn studio_discard_changes(
    device_id: String,
    state: State<'_, AppState>,
) -> Result<StudioKeymapSnapshot, String> {
    let edit = Arc::clone(&state.studio_edit);
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = edit.lock().unwrap();
        let session = guard
            .as_mut()
            .ok_or_else(|| studio_error_code(StudioError::NoEditSession))?;
        if session.device_id != device_id {
            return Err(studio_error_code(StudioError::SessionDeviceMismatch));
        }
        session.discard().map_err(studio_error_code)
    })
    .await
    .map_err(|_| "studio_discard_failed".to_string())?
}

#[tauri::command]
pub async fn studio_has_unsaved(
    device_id: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let edit = Arc::clone(&state.studio_edit);
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = edit.lock().unwrap();
        let session = guard
            .as_mut()
            .ok_or_else(|| studio_error_code(StudioError::NoEditSession))?;
        if session.device_id != device_id {
            return Err(studio_error_code(StudioError::SessionDeviceMismatch));
        }
        session.has_unsaved().map_err(studio_error_code)
    })
    .await
    .map_err(|_| "studio_has_unsaved_failed".to_string())?
}

#[tauri::command]
pub async fn studio_end_edit(device_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let edit = Arc::clone(&state.studio_edit);
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = edit.lock().unwrap();
        let Some(session) = guard.as_mut() else {
            return Ok(());
        };
        if session.device_id != device_id {
            tracing::debug!(
                requested_device_id = %device_id,
                active_device_id = %session.device_id,
                "ignored stale studio edit cleanup"
            );
            return Ok(());
        }
        if session.has_unsaved().map_err(studio_error_code)? {
            return Err(studio_error_code(StudioError::UnsavedChangesExist));
        }
        *guard = None;
        Ok(())
    })
    .await
    .map_err(|_| "studio_end_edit_failed".to_string())?
}

#[tauri::command]
pub async fn studio_abort_edit(
    device_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let edit = Arc::clone(&state.studio_edit);
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = edit.lock().unwrap();
        let Some(session) = guard.as_ref() else {
            return Ok(());
        };
        if session.device_id != device_id {
            tracing::debug!(
                requested_device_id = %device_id,
                active_device_id = %session.device_id,
                "ignored stale studio edit abort"
            );
            return Ok(());
        }
        *guard = None;
        Ok(())
    })
    .await
    .map_err(|_| "studio_abort_edit_failed".to_string())?
}

fn studio_error_code(error: StudioError) -> String {
    match error {
        StudioError::DeviceNotFound => "device_not_found",
        StudioError::Locked => "locked",
        StudioError::Timeout => "timeout",
        StudioError::Disconnected => "disconnected",
        StudioError::InvalidLocation => "invalid_location",
        StudioError::InvalidBehavior => "invalid_behavior",
        StudioError::InvalidParameters => "invalid_parameters",
        StudioError::MissingBehaviorRole => "missing_behavior_role",
        StudioError::SaveFailed => "save_failed",
        StudioError::SaveNotSupported => "save_not_supported",
        StudioError::SaveNoSpace => "save_no_space",
        StudioError::SaveResultUnknown => "save_result_unknown",
        StudioError::NoEditSession => "no_edit_session",
        StudioError::EditSessionExists => "edit_session_exists",
        StudioError::UnsavedChangesExist => "unsaved_changes_exist",
        StudioError::SessionDeviceMismatch => "session_device_mismatch",
        StudioError::PortBusy => "port_busy",
        StudioError::EditingUnsupportedForBle => "editing_unsupported_for_ble",
        StudioError::AddLayerFailed => "add_layer_failed",
        StudioError::AddLayerNoSpace => "add_layer_no_space",
        StudioError::RemoveLayerFailed => "remove_layer_failed",
        StudioError::InvalidLayer => "invalid_layer",
        StudioError::RenameLayerFailed => "rename_layer_failed",
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
    let extras = MonitorExtras {
        config: Arc::clone(&state.config),
        ai_usage_runtime: Arc::clone(&state.ai_usage_runtime),
        ai_usage_refreshing: Arc::clone(&state.ai_usage_refreshing),
        key_stats: Arc::clone(&state.key_stats),
    };

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
            extras,
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

pub(crate) fn spawn_ai_refresh_watcher(
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
        emit_status(&app, &snapshot);
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

/// Emit the status snapshot to the UI and refresh the tray tooltip together so
/// the tray always reflects the latest device/battery state.
pub(crate) fn emit_status(app: &AppHandle, status: &MonitorStatus) {
    let _ = app.emit("status-update", status);
    update_tray_tooltip(app, status);
}

fn update_tray_tooltip(app: &AppHandle, status: &MonitorStatus) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(build_tray_tooltip(status)));
    }
}

/// Build the tray tooltip: "Keylink Studio" plus one line per battery-reporting
/// device. Capped to ~128 chars (Windows tray tooltip limit).
fn build_tray_tooltip(status: &MonitorStatus) -> String {
    let mut text = String::from("Keylink Studio");
    for dev in &status.device_battery {
        let name = dev
            .product
            .as_deref()
            .or(dev.serial_number.as_deref())
            .unwrap_or(&dev.device_key);
        let sources: Vec<String> = dev
            .sources
            .iter()
            .filter_map(|src| {
                let level = src.level?;
                let label = if src.source == 0 {
                    "C".to_string()
                } else {
                    format!("P{}", src.source)
                };
                Some(format!("{}:{}%", label, level))
            })
            .collect();
        text.push('\n');
        let summary = if sources.is_empty() {
            "?".to_string()
        } else {
            sources.join(" ")
        };
        text.push_str(&format!("{}: {}", name, summary));
    }
    if text.chars().count() > 127 {
        text = text.chars().take(126).collect::<String>() + "…";
    }
    text
}

/// Copy the runner's device/AI-usage view into the shared status.
fn apply_runner_view(s: &mut MonitorStatus, runner: &MonitorRunner) {
    s.connected_devices = runner.verified_device_count();
    s.connected_device_names = runner.verified_device_names();
    s.host_link_devices = runner.verified_devices();
    s.ai_usage = runner.ai_usage_statuses();
    s.device_battery = runner.battery_statuses();
    s.device_layers = runner.layer_states();
}

fn apply_monitor_config(
    app: &AppHandle,
    next_config: AppConfig,
    runner: &mut MonitorRunner,
    interval: &mut Duration,
    uplink_interval: &mut Duration,
    status: &Arc<std::sync::Mutex<MonitorStatus>>,
    ai_usage_shared: Option<AiUsageShared>,
    extras: &MonitorExtras,
    actions_cfg: &mut ActionsConfig,
) -> Result<(), String> {
    *actions_cfg = next_config.actions.clone();
    if let Ok(mut store) = extras.key_stats.lock() {
        store.set_flush_interval(Duration::from_secs(
            next_config.stats.flush_interval_sec.max(1),
        ));
    }
    *runner = rebuild_runner(&next_config, ai_usage_shared)?;
    runner.set_key_stats_store(Arc::clone(&extras.key_stats));
    *interval = Duration::from_millis(next_config.polling.interval_ms.max(1));
    *uplink_interval = Duration::from_millis(next_config.polling.uplink_interval_ms.max(5));

    let mut s = status.lock().unwrap();
    s.current_layer = None;
    s.current_rule = None;
    s.connected_devices = 0;
    s.connected_device_names = Vec::new();
    s.host_link_devices = Vec::new();
    s.device_battery = Vec::new();
    s.device_layers = Vec::new();
    s.ai_usage = runner.ai_usage_statuses();
    s.last_error = None;
    emit_status(app, &s);

    Ok(())
}

/// Apply a single monitor command. Returns `true` when the loop should stop.
fn process_command(
    command: MonitorCommand,
    app: &AppHandle,
    runner: &mut MonitorRunner,
    interval: &mut Duration,
    uplink_interval: &mut Duration,
    status: &Arc<std::sync::Mutex<MonitorStatus>>,
    log_entries: &Arc<std::sync::Mutex<std::collections::VecDeque<LogEntry>>>,
    log_counter: &Arc<std::sync::Mutex<u64>>,
    extras: &MonitorExtras,
    actions_cfg: &mut ActionsConfig,
) -> bool {
    match command {
        MonitorCommand::Stop => true,
        MonitorCommand::ForegroundChanged => false,
        MonitorCommand::InjectUplink(device, packet) => {
            runner.inject_uplink(device, packet);
            false
        }
        MonitorCommand::UpdateConfig(next_config, ai_usage_shared) => {
            if let Err(e) = apply_monitor_config(
                app,
                next_config,
                runner,
                interval,
                uplink_interval,
                status,
                ai_usage_shared,
                extras,
                actions_cfg,
            ) {
                let msg = format!("HID init error: {}", e);
                {
                    let mut s = status.lock().unwrap();
                    s.last_error = Some(msg.clone());
                    emit_status(app, &s);
                }
                let entry = add_log(log_entries, log_counter, "error", &msg);
                let _ = app.emit("log-added", entry);
            }
            false
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct KeyPressPayload {
    device_uid: String,
    position: u8,
    pressed: bool,
}

/// Handle uplink events drained from the runner. Returns `true` when a
/// HOST_ACTION requested the monitor loop to stop.
fn handle_uplink_events(
    app: &AppHandle,
    runner: &mut MonitorRunner,
    actions_cfg: &ActionsConfig,
    extras: &MonitorExtras,
    status: &Arc<std::sync::Mutex<MonitorStatus>>,
    log_entries: &Arc<std::sync::Mutex<std::collections::VecDeque<LogEntry>>>,
    log_counter: &Arc<std::sync::Mutex<u64>>,
) -> bool {
    let mut should_stop = false;
    for event in runner.take_uplink_events() {
        match &event.packet {
            UplinkPacket::HostAction(action) => {
                // Bindings are per device; a device without an (enabled)
                // config section only has its actions logged.
                let device_key = uplink_device_key(&event.device);
                let binding = actions_cfg
                    .enabled
                    .then(|| {
                        actions_cfg
                            .devices
                            .get(&device_key)
                            .filter(|device_cfg| device_cfg.enabled)
                            .and_then(|device_cfg| {
                                device_cfg
                                    .bindings
                                    .iter()
                                    .find(|b| b.action_id == action.action_id)
                            })
                    })
                    .flatten();
                let (level, message) = match binding {
                    Some(binding) => {
                        match actions::execute(app, binding, action.value, extras, status) {
                            Ok(actions::ActionOutcome::Continue) => (
                                "info",
                                format!(
                                    "host action {} executed ({:?})",
                                    action.action_id, binding.action
                                ),
                            ),
                            Ok(actions::ActionOutcome::StopRequested) => {
                                should_stop = true;
                                (
                                    "info",
                                    format!("host action {}: stop monitoring", action.action_id),
                                )
                            }
                            Err(error) => (
                                "error",
                                format!("host action {} failed: {}", action.action_id, error),
                            ),
                        }
                    }
                    None if actions_cfg.enabled => (
                        "warn",
                        format!(
                            "unbound host action id={} value={} ({})",
                            action.action_id, action.value, device_key
                        ),
                    ),
                    None => (
                        "warn",
                        format!(
                            "host action id={} ignored (actions disabled)",
                            action.action_id
                        ),
                    ),
                };
                let entry = add_log(log_entries, log_counter, level, &message);
                let _ = app.emit("log-added", entry);
            }
            UplinkPacket::KeyStats(_) => {
                let _ = app.emit("key-stats-updated", uplink_device_key(&event.device));
            }
            UplinkPacket::KeyPress(p) => {
                let _ = app.emit(
                    "key-press-event",
                    KeyPressPayload {
                        device_uid: uplink_device_key(&event.device),
                        position: p.position,
                        pressed: p.pressed,
                    },
                );
            }
            // Battery / layer state are part of MonitorStatus and flow
            // through the regular status-update emission.
            UplinkPacket::Battery(_) | UplinkPacket::LayerState(_) => {}
        }
    }
    should_stop
}

#[allow(clippy::too_many_arguments)]
fn run_monitor_loop(
    app: AppHandle,
    config: AppConfig,
    status: Arc<std::sync::Mutex<MonitorStatus>>,
    log_entries: Arc<std::sync::Mutex<std::collections::VecDeque<LogEntry>>>,
    log_counter: Arc<std::sync::Mutex<u64>>,
    rx: mpsc::Receiver<MonitorCommand>,
    ai_usage_shared: Option<AiUsageShared>,
    extras: MonitorExtras,
) {
    let mut actions_cfg = config.actions.clone();
    let mut runner = match rebuild_runner(&config, ai_usage_shared) {
        Ok(runner) => runner,
        Err(e) => {
            let msg = format!("HID init error: {}", e);
            let entry = add_log(&log_entries, &log_counter, "error", &msg);
            {
                let mut s = status.lock().unwrap();
                s.last_error = Some(msg);
                emit_status(&app, &s);
            }
            let _ = app.emit("log-added", entry);
            return;
        }
    };
    runner.set_key_stats_store(Arc::clone(&extras.key_stats));

    {
        let mut s = status.lock().unwrap();
        s.running = true;
        s.last_error = None;
        emit_status(&app, &s);
    }

    let entry = add_log(&log_entries, &log_counter, "info", "Monitoring started");
    let _ = app.emit("log-added", entry);

    let mut interval = Duration::from_millis(config.polling.interval_ms.max(1));
    let mut uplink_interval = Duration::from_millis(config.polling.uplink_interval_ms.max(5));

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
                        &mut uplink_interval,
                        &status,
                        &log_entries,
                        &log_counter,
                        &extras,
                        &mut actions_cfg,
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
                    emit_status(&app, &s);
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
                emit_status(&app, &s);
            }
            Ok(RunEvent::Unchanged) => {
                let devices = runner.verified_device_count();
                let host_link_devices = runner.verified_devices();
                let ai_usage = runner.ai_usage_statuses();
                let device_battery = runner.battery_statuses();
                let device_layers = runner.layer_states();
                let mut s = status.lock().unwrap();
                if s.connected_devices != devices
                    || s.ai_usage != ai_usage
                    || s.host_link_devices != host_link_devices
                    || s.device_battery != device_battery
                    || s.device_layers != device_layers
                {
                    apply_runner_view(&mut s, &runner);
                    emit_status(&app, &s);
                }
            }
            Err(e) => {
                let msg = format!("Error: {}", e);
                {
                    let mut s = status.lock().unwrap();
                    s.last_error = Some(msg.clone());
                    emit_status(&app, &s);
                }
                let entry = add_log(&log_entries, &log_counter, "error", &msg);
                let _ = app.emit("log-added", entry);
            }
        }

        if handle_uplink_events(
            &app,
            &mut runner,
            &actions_cfg,
            &extras,
            &status,
            &log_entries,
            &log_counter,
        ) {
            break;
        }

        // Wait for the next control-loop tick, draining uplink every uplink_interval_ms.
        let mut should_stop = false;
        let deadline = Instant::now() + interval;
        'wait: loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break 'wait;
            }
            let wait = uplink_interval.min(remaining);
            match rx.recv_timeout(wait) {
                Ok(command) => {
                    if process_command(
                        command,
                        &app,
                        &mut runner,
                        &mut interval,
                        &mut uplink_interval,
                        &status,
                        &log_entries,
                        &log_counter,
                        &extras,
                        &mut actions_cfg,
                    ) {
                        should_stop = true;
                    }
                    break 'wait;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    should_stop = true;
                    break 'wait;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            runner.drain_uplink_only();
            if handle_uplink_events(
                &app,
                &mut runner,
                &actions_cfg,
                &extras,
                &status,
                &log_entries,
                &log_counter,
            ) {
                should_stop = true;
                break 'wait;
            }
        }
        if should_stop {
            break;
        }
    }

    if let Ok(mut store) = extras.key_stats.lock() {
        store.flush_all();
    }

    {
        let mut s = status.lock().unwrap();
        s.running = false;
        s.connected_devices = 0;
        s.connected_device_names = Vec::new();
        s.host_link_devices = Vec::new();
        s.current_layer = None;
        s.current_rule = None;
        s.device_battery = Vec::new();
        s.device_layers = Vec::new();
        emit_status(&app, &s);
    }

    let entry = add_log(&log_entries, &log_counter, "info", "Monitoring stopped");
    let _ = app.emit("log-added", entry);
}

#[tauri::command]
pub fn get_key_stats(
    device_uid: String,
    period: String,
    state: State<AppState>,
) -> Result<KeyStatsSummary, String> {
    let period = StatsPeriod::parse(&period).ok_or_else(|| "invalid_period".to_string())?;
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut store = state.key_stats.lock().map_err(|_| "stats_unavailable")?;
    Ok(store.summary(&device_uid, period, &today))
}

#[tauri::command]
pub fn list_key_stats_devices(state: State<AppState>) -> Vec<String> {
    state
        .key_stats
        .lock()
        .map(|store| store.device_keys())
        .unwrap_or_default()
}

/// Debug-only helper to exercise the uplink path without firmware. Accepts a
/// 32-byte packet payload as hex and feeds it through the monitor loop.
#[tauri::command]
pub fn debug_inject_uplink(
    device_uid: String,
    payload_hex: String,
    state: State<AppState>,
) -> Result<(), String> {
    #[cfg(not(debug_assertions))]
    {
        let _ = (device_uid, payload_hex, state);
        Err("debug_only".to_string())
    }
    #[cfg(debug_assertions)]
    {
        let bytes = decode_hex(&payload_hex)?;
        if bytes.len() != rawhid_host_core::PACKET_SIZE {
            return Err(format!(
                "payload must be {} bytes, got {}",
                rawhid_host_core::PACKET_SIZE,
                bytes.len()
            ));
        }
        let packet = UplinkPacket::decode_payload(&bytes).map_err(|e| e.to_string())?;
        let uid = device_uid
            .strip_prefix("uid:")
            .and_then(|hex| u64::from_str_radix(hex, 16).ok());
        let device = DeviceInfo {
            path: format!("debug:{device_uid}"),
            vendor_id: 0,
            product_id: 0,
            usage_page: 0,
            usage: 0,
            connection_type: rawhid_host_core::hid::DeviceConnectionType::Unknown,
            manufacturer: None,
            product: Some("debug".to_string()),
            serial_number: None,
            capabilities: packet.required_capability(),
            device_uid_hash: uid,
        };
        let tx = state.stop_tx.lock().unwrap();
        match tx.as_ref() {
            Some(tx) => tx
                .send(MonitorCommand::InjectUplink(device, packet))
                .map_err(|_| "monitor_loop_gone".to_string()),
            None => Err("not_running".to_string()),
        }
    }
}

#[cfg(debug_assertions)]
fn decode_hex(input: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.len() % 2 != 0 {
        return Err("hex string must have even length".to_string());
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use rawhid_host_core::runner::{DeviceBatterySource, DeviceBatteryStatus};

    fn battery_status(sources: Vec<DeviceBatterySource>) -> MonitorStatus {
        let mut status = MonitorStatus::default();
        status.device_battery = vec![DeviceBatteryStatus {
            device_key: "uid:00000000000000aa".to_string(),
            serial_number: Some("serial-a".to_string()),
            product: Some("Test Keyboard".to_string()),
            sources,
            updated_unix: 0,
        }];
        status
    }

    #[test]
    fn tray_tooltip_hides_unknown_battery_sources() {
        let status = battery_status(vec![
            DeviceBatterySource {
                source: 0,
                level: None,
            },
            DeviceBatterySource {
                source: 1,
                level: Some(78),
            },
        ]);

        assert_eq!(
            build_tray_tooltip(&status),
            "Keylink Studio\nTest Keyboard: P1:78%"
        );
    }

    #[test]
    fn tray_tooltip_shows_unknown_when_no_source_is_available() {
        let status = battery_status(vec![
            DeviceBatterySource {
                source: 0,
                level: None,
            },
            DeviceBatterySource {
                source: 1,
                level: None,
            },
        ]);

        assert_eq!(
            build_tray_tooltip(&status),
            "Keylink Studio\nTest Keyboard: ?"
        );
    }

    #[test]
    fn tray_tooltip_uses_central_and_peripheral_labels() {
        let status = battery_status(vec![
            DeviceBatterySource {
                source: 0,
                level: Some(92),
            },
            DeviceBatterySource {
                source: 1,
                level: Some(78),
            },
            DeviceBatterySource {
                source: 2,
                level: Some(76),
            },
        ]);

        assert_eq!(
            build_tray_tooltip(&status),
            "Keylink Studio\nTest Keyboard: C:92% P1:78% P2:76%"
        );
    }
}
