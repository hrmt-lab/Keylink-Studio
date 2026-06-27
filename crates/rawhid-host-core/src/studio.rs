use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator;
use thiserror::Error;
use zmk_studio_api::{
    proto::zmk::{self, core::LockState},
    transport::serial::SerialTransport,
    transport::serial::SerialTransportError,
    transport::PlatformBleTransport,
    Behavior, ClientError, HidUsage, Keycode, StudioClient,
};

use crate::config::StudioConfig;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StudioRpcStatus {
    Ok,
    Failed,
    Timeout,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StudioLockState {
    Locked,
    Unlocked,
    Unknown,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeymapViewerStatus {
    Available,
    Locked,
    Unsupported,
    Failed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StudioErrorCode {
    None,
    NoSerialPorts,
    OpenFailed,
    RpcTimeout,
    RpcFailed,
    ProtocolMismatch,
    Locked,
    DeviceNotFound,
    KeymapReadFailed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StudioDeviceStatus {
    pub id: String,
    pub connection_type: String,
    pub port_name: String,
    pub display_name: String,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub serial_number: Option<String>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub transport_detected: bool,
    pub rpc_status: StudioRpcStatus,
    pub lock_state: StudioLockState,
    pub keymap_viewer_status: KeymapViewerStatus,
    pub error_code: StudioErrorCode,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StudioKeymapSnapshot {
    pub device_id: String,
    pub device_name: String,
    pub connection_type: String,
    pub lock_state: StudioLockState,
    pub physical_layouts: Vec<StudioPhysicalLayout>,
    pub selected_physical_layout_index: Option<usize>,
    pub selected_physical_layout_name: Option<String>,
    pub layout_source: StudioLayoutSource,
    pub selected_layout_keys: Vec<StudioPhysicalKey>,
    pub layers: Vec<StudioLayer>,
    pub updated_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StudioLayoutSource {
    StudioPhysicalLayout,
    GridFallback,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StudioPhysicalLayout {
    pub index: usize,
    pub name: String,
    pub keys: Vec<StudioPhysicalKey>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StudioPhysicalKey {
    pub position: usize,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub r: i32,
    pub rx: i32,
    pub ry: i32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StudioLayer {
    pub index: usize,
    pub id: u32,
    pub name: String,
    pub bindings: Vec<StudioBinding>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StudioBinding {
    pub position: usize,
    pub binding_label: String,
    pub primary_label: String,
    pub secondary_label: String,
    pub full_label: String,
    pub behavior: String,
    pub params: Vec<u32>,
    pub raw: StudioRawBinding,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StudioRawBinding {
    pub behavior_id: i32,
    pub param1: u32,
    pub param2: u32,
}

pub const KEYMAP_BACKUP_SCHEMA: &str = "keylink-studio.keymap-backup";
pub const KEYMAP_BACKUP_SCHEMA_VERSION: u32 = 1;
pub const KEYMAP_BACKUP_MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeymapBackup {
    pub schema: String,
    pub schema_version: u32,
    pub app_version: String,
    pub exported_at_ms: u64,
    pub device: BackupDevice,
    pub layout: BackupLayout,
    pub behavior_catalog: BTreeMap<i32, String>,
    pub layers: Vec<BackupLayer>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupDevice {
    pub name: String,
    pub connection_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupLayout {
    pub selected_physical_layout_name: Option<String>,
    pub positions: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupLayer {
    pub index: usize,
    pub id: u32,
    pub name: String,
    pub bindings: Vec<BackupBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupBinding {
    pub position: usize,
    pub behavior_id: i32,
    pub param1: u32,
    pub param2: u32,
    pub behavior: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RestorePlan {
    pub report: RestoreReport,
    pub writes: Vec<RawBindingWrite>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RestoreReport {
    pub can_apply: bool,
    pub behavior_verification: BehaviorVerification,
    pub source_device_name: String,
    pub exported_at_ms: u64,
    pub will_write: usize,
    pub unchanged_skipped: usize,
    pub blocked: usize,
    pub changed_keys: Vec<RestoreChangedKey>,
    pub warnings: Vec<RestoreIssue>,
    pub errors: Vec<RestoreIssue>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RestoreChangedKey {
    pub layer_index: usize,
    pub position: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BehaviorVerification {
    Done,
    Skipped,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RestoreIssue {
    pub code: String,
    pub layer_index: Option<usize>,
    pub position: Option<usize>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RawBindingWrite {
    pub layer_index: usize,
    pub layer_id: u32,
    pub position: i32,
    pub behavior_id: i32,
    pub param1: u32,
    pub param2: u32,
}

#[derive(Debug, Error)]
pub enum KeymapFileError {
    #[error("keymap_invalid_file")]
    InvalidFile,
    #[error("keymap_unsupported_version")]
    UnsupportedVersion,
    #[error("keymap_file_too_large")]
    FileTooLarge,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StudioBindingLabelPatch {
    pub behavior_id: i32,
    pub param1: u32,
    pub param2: u32,
    pub behavior: String,
    pub binding_label: String,
    pub primary_label: String,
    pub secondary_label: String,
    pub full_label: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct KeyCatalogEntry {
    pub display: String,
    pub canonical: String,
    pub hid_usage: u32,
    pub category: String,
    pub aliases: Vec<String>,
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditBehavior {
    KeyPress(u32),
    Transparent,
    None,
    MomentaryLayer(u32),
    ToggleLayer(u32),
    ToLayer(u32),
    ModTap { hold: u32, tap: u32 },
    LayerTap { target_layer_index: u32, tap: u32 },
    StickyKey(u32),
    StickyLayer(u32),
    Bluetooth { command: u32, value: u32 },
    OutputSelection(u32),
    MouseKeyPress(u32),
    MouseMove(u32),
    MouseScroll(u32),
    CapsWord,
    KeyRepeat,
    Reset,
    Bootloader,
    StudioUnlock,
    GraveEscape,
}

#[derive(Debug, Error)]
pub enum StudioError {
    #[error("device not found")]
    DeviceNotFound,
    #[error("studio device is locked")]
    Locked,
    #[error("RPC timeout")]
    Timeout,
    #[error("device disconnected")]
    Disconnected,
    #[error("invalid key location")]
    InvalidLocation,
    #[error("invalid behavior")]
    InvalidBehavior,
    #[error("invalid behavior parameters")]
    InvalidParameters,
    #[error("missing behavior role")]
    MissingBehaviorRole,
    #[error("save failed")]
    SaveFailed,
    #[error("save is not supported")]
    SaveNotSupported,
    #[error("no space left for save")]
    SaveNoSpace,
    #[error("save result is unknown")]
    SaveResultUnknown,
    #[error("no edit session")]
    NoEditSession,
    #[error("edit session already exists")]
    EditSessionExists,
    #[error("unsaved changes exist")]
    UnsavedChangesExist,
    #[error("edit session device mismatch")]
    SessionDeviceMismatch,
    #[error("studio port is busy")]
    PortBusy,
    #[error("BLE Studio editing is not supported yet")]
    EditingUnsupportedForBle,
    #[error("add layer failed")]
    AddLayerFailed,
    #[error("no space left for layer")]
    AddLayerNoSpace,
    #[error("remove layer failed")]
    RemoveLayerFailed,
    #[error("invalid layer")]
    InvalidLayer,
    #[error("rename layer failed")]
    RenameLayerFailed,
    #[error("RPC failed")]
    RpcFailed,
}

#[derive(Debug, Clone)]
struct StudioPortCandidate {
    port_name: String,
    vid: Option<u16>,
    pid: Option<u16>,
    serial_number: Option<String>,
    manufacturer: Option<String>,
    product: Option<String>,
}

impl StudioPortCandidate {
    fn id(&self) -> String {
        format!(
            "serial:{}",
            stable_device_id(
                &self.port_name,
                self.vid,
                self.pid,
                self.serial_number.as_deref(),
            )
        )
    }

    fn legacy_id(&self) -> String {
        stable_device_id(
            &self.port_name,
            self.vid,
            self.pid,
            self.serial_number.as_deref(),
        )
    }

    fn display_name(&self) -> String {
        self.product
            .clone()
            .or_else(|| self.manufacturer.clone())
            .unwrap_or_else(|| self.port_name.clone())
    }
}

#[derive(Debug, Clone)]
struct StudioBleCandidate {
    device_id_json: String,
    local_name: Option<String>,
}

impl StudioBleCandidate {
    fn id(&self) -> String {
        format!("ble:{}", hex_encode(self.device_id_json.as_bytes()))
    }

    fn endpoint_label(&self) -> String {
        self.local_name
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "BLE Studio".to_string())
    }
}

#[derive(Debug, Clone)]
enum StudioDeviceCandidate {
    Serial(StudioPortCandidate),
    Ble(StudioBleCandidate),
}

impl StudioDeviceCandidate {
    fn id(&self) -> String {
        match self {
            Self::Serial(candidate) => candidate.id(),
            Self::Ble(candidate) => candidate.id(),
        }
    }

    fn display_name(&self) -> String {
        match self {
            Self::Serial(candidate) => candidate.display_name(),
            Self::Ble(candidate) => candidate.endpoint_label(),
        }
    }

    fn connection_type(&self) -> &'static str {
        match self {
            Self::Serial(_) => "usb_serial",
            Self::Ble(_) => "ble_studio",
        }
    }
}

enum StudioDeviceRef {
    Serial(String),
    Ble(String),
}

impl StudioDeviceRef {
    fn parse(device_id: &str) -> Result<Self, StudioError> {
        if let Some(encoded) = device_id.strip_prefix("ble:") {
            let bytes = hex_decode(encoded).ok_or(StudioError::DeviceNotFound)?;
            let json = String::from_utf8(bytes).map_err(|_| StudioError::DeviceNotFound)?;
            return Ok(Self::Ble(json));
        }
        if let Some(id) = device_id.strip_prefix("serial:") {
            return Ok(Self::Serial(id.to_string()));
        }
        Ok(Self::Serial(device_id.to_string()))
    }
}

enum StudioTransport {
    Serial(SerialTransport),
    Ble(PlatformBleTransport),
}

impl Read for StudioTransport {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Serial(inner) => inner.read(buf),
            Self::Ble(inner) => inner.read(buf),
        }
    }
}

impl Write for StudioTransport {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Serial(inner) => inner.write(buf),
            Self::Ble(inner) => inner.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Serial(inner) => inner.flush(),
            Self::Ble(inner) => inner.flush(),
        }
    }
}

pub fn probe_studio_devices(config: &StudioConfig) -> Vec<StudioDeviceStatus> {
    let candidates = match serial_candidates() {
        Ok(candidates) => candidates,
        Err(_) => Vec::new(),
    };

    let mut devices: Vec<_> = candidates
        .into_iter()
        .map(|candidate| probe_candidate(candidate, config.probe_timeout_ms))
        .collect();

    match ble_candidates() {
        Ok(candidates) => {
            devices.extend(
                candidates
                    .into_iter()
                    .map(|candidate| probe_ble_candidate(candidate, config.probe_timeout_ms)),
            );
        }
        Err(error) => {
            tracing::warn!(error = %error, "failed to list ZMK Studio BLE devices");
        }
    }

    devices
}

pub fn read_keymap_for_device(
    device_id: &str,
    config: &StudioConfig,
) -> Result<StudioKeymapSnapshot, StudioError> {
    let device_ref = StudioDeviceRef::parse(device_id)?;

    run_with_timeout(config.keymap_read_timeout_ms, move || {
        read_keymap(device_ref)
    })
    .unwrap_or(Err(StudioError::Timeout))
}

pub fn resolve_behavior_labels_for_device(
    device_id: &str,
    raw_bindings: Vec<StudioRawBinding>,
    config: &StudioConfig,
) -> Result<Vec<StudioBindingLabelPatch>, StudioError> {
    let device_ref = StudioDeviceRef::parse(device_id)?;
    let candidate = resolve_device_candidate(device_ref)?;
    let mut client = open_studio_client(&candidate).map_err(|_| StudioError::RpcFailed)?;
    let behavior_names = behavior_names_for_raw_bindings(
        &mut client,
        &raw_bindings,
        Duration::from_millis(config.keymap_read_timeout_ms.max(1)),
    );
    Ok(label_patches_for_raw_bindings(
        raw_bindings,
        &behavior_names,
    ))
}

pub fn keymap_backup_from_snapshot(
    snapshot: &StudioKeymapSnapshot,
    behavior_catalog: BTreeMap<i32, String>,
    app_version: &str,
) -> KeymapBackup {
    let positions = snapshot
        .selected_layout_keys
        .iter()
        .map(|key| key.position)
        .collect();
    let mut catalog = behavior_catalog;
    for layer in &snapshot.layers {
        for binding in &layer.bindings {
            if !binding.behavior.starts_with("behavior ") {
                catalog
                    .entry(binding.raw.behavior_id)
                    .or_insert_with(|| binding.behavior.clone());
            }
        }
    }

    KeymapBackup {
        schema: KEYMAP_BACKUP_SCHEMA.to_string(),
        schema_version: KEYMAP_BACKUP_SCHEMA_VERSION,
        app_version: app_version.to_string(),
        exported_at_ms: now_ms(),
        device: BackupDevice {
            name: snapshot.device_name.clone(),
            connection_type: snapshot.connection_type.clone(),
        },
        layout: BackupLayout {
            selected_physical_layout_name: snapshot.selected_physical_layout_name.clone(),
            positions,
        },
        behavior_catalog: catalog,
        layers: snapshot
            .layers
            .iter()
            .map(|layer| BackupLayer {
                index: layer.index,
                id: layer.id,
                name: layer.name.clone(),
                bindings: layer
                    .bindings
                    .iter()
                    .map(|binding| BackupBinding {
                        position: binding.position,
                        behavior_id: binding.raw.behavior_id,
                        param1: binding.raw.param1,
                        param2: binding.raw.param2,
                        behavior: binding.behavior.clone(),
                        label: binding.full_label.clone(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub fn serialize_keymap_backup(backup: &KeymapBackup) -> Result<String, KeymapFileError> {
    serde_json::to_string_pretty(backup).map_err(|_| KeymapFileError::InvalidFile)
}

pub fn parse_keymap_backup(text: &str) -> Result<KeymapBackup, KeymapFileError> {
    if text.len() > KEYMAP_BACKUP_MAX_BYTES {
        return Err(KeymapFileError::FileTooLarge);
    }
    let backup: KeymapBackup =
        serde_json::from_str(text).map_err(|_| KeymapFileError::InvalidFile)?;
    if backup.schema != KEYMAP_BACKUP_SCHEMA {
        return Err(KeymapFileError::InvalidFile);
    }
    if backup.schema_version != KEYMAP_BACKUP_SCHEMA_VERSION {
        return Err(KeymapFileError::UnsupportedVersion);
    }
    Ok(backup)
}

pub fn plan_keymap_restore(
    current: &StudioKeymapSnapshot,
    target_behavior_names: Option<&BTreeMap<i32, String>>,
    backup: &KeymapBackup,
) -> RestorePlan {
    let mut warnings = Vec::new();
    let errors = Vec::new();
    let mut writes = Vec::new();
    let mut unchanged_skipped = 0usize;
    let mut blocked = 0usize;

    let placeholder_only = backup.behavior_catalog.is_empty()
        || backup
            .layers
            .iter()
            .flat_map(|layer| layer.bindings.iter())
            .all(|binding| is_placeholder_behavior_name(&binding.behavior));
    let behavior_verification = if target_behavior_names.is_some() && !placeholder_only {
        BehaviorVerification::Done
    } else {
        BehaviorVerification::Skipped
    };

    for (layer_index, backup_layer) in backup.layers.iter().enumerate() {
        let Some(current_layer) = current.layers.get(layer_index) else {
            continue;
        };
        let current_by_position: BTreeMap<usize, &StudioBinding> = current_layer
            .bindings
            .iter()
            .map(|binding| (binding.position, binding))
            .collect();
        for backup_binding in &backup_layer.bindings {
            let Some(current_binding) = current_by_position.get(&backup_binding.position) else {
                continue;
            };
            if same_raw(&current_binding.raw, backup_binding) {
                unchanged_skipped += 1;
                continue;
            }
            if behavior_verification == BehaviorVerification::Done {
                let target_names = target_behavior_names.expect("checked above");
                let backup_name = backup
                    .behavior_catalog
                    .get(&backup_binding.behavior_id)
                    .or_else(|| {
                        if is_placeholder_behavior_name(&backup_binding.behavior) {
                            None
                        } else {
                            Some(&backup_binding.behavior)
                        }
                    });
                match (backup_name, target_names.get(&backup_binding.behavior_id)) {
                    (_, None) => {
                        blocked += 1;
                        warnings.push(issue(
                            "behavior_missing",
                            Some(layer_index),
                            Some(backup_binding.position),
                            "target behavior id was not found",
                        ));
                        continue;
                    }
                    (None, Some(_)) => {
                        blocked += 1;
                        warnings.push(issue(
                            "behavior_unverified",
                            Some(layer_index),
                            Some(backup_binding.position),
                            "backup behavior name is unresolved",
                        ));
                        continue;
                    }
                    (Some(source), Some(target))
                        if normalize_behavior_name(source) != normalize_behavior_name(target) =>
                    {
                        blocked += 1;
                        warnings.push(issue(
                            "behavior_conflict",
                            Some(layer_index),
                            Some(backup_binding.position),
                            "behavior name differs for the same id",
                        ));
                        continue;
                    }
                    _ => {}
                }
            }
            writes.push(RawBindingWrite {
                layer_index,
                layer_id: current_layer.id,
                position: backup_binding.position as i32,
                behavior_id: backup_binding.behavior_id,
                param1: backup_binding.param1,
                param2: backup_binding.param2,
            });
        }
    }

    let can_apply = errors.is_empty();
    let changed_keys = writes
        .iter()
        .map(|write| RestoreChangedKey {
            layer_index: write.layer_index,
            position: write.position as usize,
        })
        .collect();
    RestorePlan {
        report: RestoreReport {
            can_apply,
            behavior_verification,
            source_device_name: backup.device.name.clone(),
            exported_at_ms: backup.exported_at_ms,
            will_write: writes.len(),
            unchanged_skipped,
            blocked,
            changed_keys,
            warnings,
            errors,
        },
        writes,
    }
}

fn same_raw(current: &StudioRawBinding, backup: &BackupBinding) -> bool {
    current.behavior_id == backup.behavior_id
        && current.param1 == backup.param1
        && current.param2 == backup.param2
}

fn is_placeholder_behavior_name(name: &str) -> bool {
    let Some(rest) = name.trim().strip_prefix("behavior ") else {
        return false;
    };
    !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit() || ch == '-')
}

fn normalize_behavior_name(name: &str) -> String {
    name.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| match ch {
            '_' | '-' => ' ',
            ch if ch.is_ascii_whitespace() => ' ',
            ch => ch,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn issue(
    code: &str,
    layer_index: Option<usize>,
    position: Option<usize>,
    message: &str,
) -> RestoreIssue {
    RestoreIssue {
        code: code.to_string(),
        layer_index,
        position,
        message: message.to_string(),
    }
}

pub struct StudioEditSession {
    client: StudioClient<StudioTransport>,
    pub device_id: String,
    fallback_name: String,
    connection_type: String,
    layout_selection: LayoutSelection,
    behavior_names: BTreeMap<i32, String>,
    snapshot_mode: StudioSnapshotMode,
}

impl StudioEditSession {
    pub fn open(device_id: &str, config: &StudioConfig) -> Result<Self, StudioError> {
        Self::open_with_snapshot(device_id, config).map(|(session, _)| session)
    }

    pub fn open_with_snapshot(
        device_id: &str,
        _config: &StudioConfig,
    ) -> Result<(Self, StudioKeymapSnapshot), StudioError> {
        let device_ref = StudioDeviceRef::parse(device_id)?;
        let candidate = resolve_device_candidate(device_ref)?;
        let snapshot_mode = snapshot_mode_for_candidate(&candidate);
        let mut client = open_studio_client(&candidate).map_err(|_| StudioError::Disconnected)?;
        let lock_state = client
            .get_lock_state()
            .map_err(map_client_to_studio_error)?;
        if lock_state_to_status(lock_state) != StudioLockState::Unlocked {
            return Err(StudioError::Locked);
        }

        let (keymap, layout_selection, behavior_names) =
            fetch_snapshot_data(&mut client, snapshot_mode)?;
        let device_id = candidate.id();
        let fallback_name = candidate.display_name();
        let connection_type = candidate.connection_type().to_string();
        let snapshot = snapshot_from_parts(
            device_id.clone(),
            fallback_name.clone(),
            connection_type.clone(),
            StudioLockState::Unlocked,
            keymap,
            layout_selection.clone(),
            behavior_names.clone(),
        );

        Ok((
            Self {
                client,
                device_id,
                fallback_name,
                connection_type,
                layout_selection,
                behavior_names,
                snapshot_mode,
            },
            snapshot,
        ))
    }

    pub fn has_unsaved(&mut self) -> Result<bool, StudioError> {
        self.client
            .check_unsaved_changes()
            .map_err(map_client_to_studio_error)
    }

    pub fn snapshot(&mut self) -> Result<StudioKeymapSnapshot, StudioError> {
        let keymap = self
            .client
            .get_keymap()
            .map_err(map_client_to_studio_error)?;
        if self.snapshot_mode == StudioSnapshotMode::Full {
            self.behavior_names
                .extend(behavior_names_for_keymap(&mut self.client, &keymap));
        }
        Ok(snapshot_from_parts(
            self.device_id.clone(),
            self.fallback_name.clone(),
            self.connection_type.clone(),
            StudioLockState::Unlocked,
            keymap,
            self.layout_selection.clone(),
            self.behavior_names.clone(),
        ))
    }

    pub fn seed_behavior_labels(&mut self, patches: &[StudioBindingLabelPatch]) {
        for patch in patches {
            if patch.behavior.starts_with("behavior ") {
                continue;
            }
            self.behavior_names
                .insert(patch.behavior_id, patch.behavior.clone());
        }
    }

    pub fn resolve_behavior_labels(
        &mut self,
        raw_bindings: Vec<StudioRawBinding>,
        timeout: Duration,
    ) -> Result<Vec<StudioBindingLabelPatch>, StudioError> {
        let mut behavior_names = self.behavior_names.clone();
        let resolved_names =
            behavior_names_for_raw_bindings(&mut self.client, &raw_bindings, timeout);
        behavior_names.extend(resolved_names);
        self.behavior_names = behavior_names.clone();
        Ok(label_patches_for_raw_bindings(
            raw_bindings,
            &behavior_names,
        ))
    }

    pub fn resolve_behavior_names(
        &mut self,
        behavior_ids: &BTreeSet<i32>,
        timeout: Duration,
    ) -> Result<Option<BTreeMap<i32, String>>, StudioError> {
        let deadline = Instant::now() + timeout;
        let mut names = BTreeMap::new();
        for id in behavior_ids {
            let Ok(id_u32) = u32::try_from(*id) else {
                continue;
            };
            if Instant::now() >= deadline {
                break;
            }
            match self.client.get_behavior_details(id_u32) {
                Ok(details) => {
                    names.insert(*id, details.display_name);
                }
                Err(error) => {
                    tracing::debug!(
                        behavior_id = id_u32,
                        error = %error,
                        "failed to verify Studio behavior details"
                    );
                }
            }
        }
        if names.is_empty() {
            Ok(None)
        } else {
            self.behavior_names.extend(names.clone());
            Ok(Some(names))
        }
    }

    pub fn set_binding(
        &mut self,
        layer_id: u32,
        position: i32,
        behavior: EditBehavior,
    ) -> Result<StudioKeymapSnapshot, StudioError> {
        self.client
            .set_key_at(layer_id, position, behavior_to_zmk(behavior))
            .map_err(map_client_to_studio_error)?;
        self.snapshot()
    }

    pub fn apply_raw_writes(
        &mut self,
        writes: &[RawBindingWrite],
    ) -> Result<StudioKeymapSnapshot, StudioError> {
        for write in writes {
            self.client
                .set_key_at(
                    write.layer_id,
                    write.position,
                    Behavior::Unknown {
                        behavior_id: write.behavior_id,
                        param1: write.param1,
                        param2: write.param2,
                    },
                )
                .map_err(map_client_to_studio_error)?;
        }
        self.snapshot()
    }

    pub fn save(&mut self) -> Result<(), StudioError> {
        match self.client.save_changes() {
            Ok(()) => Ok(()),
            Err(save_error) => match self.client.check_unsaved_changes() {
                Ok(false) => Ok(()),
                Ok(true) => Err(map_client_to_studio_error(save_error)),
                Err(_) => Err(StudioError::SaveResultUnknown),
            },
        }
    }

    pub fn discard(&mut self) -> Result<StudioKeymapSnapshot, StudioError> {
        self.client
            .discard_changes()
            .map_err(map_client_to_studio_error)?;
        self.snapshot()
    }

    pub fn add_layer(&mut self, name: String) -> Result<StudioKeymapSnapshot, StudioError> {
        let details = self
            .client
            .add_layer()
            .map_err(map_client_to_studio_error)?;
        let layer = details.layer.ok_or(StudioError::RpcFailed)?;
        self.client
            .set_layer_props(layer.id, name)
            .map_err(map_client_to_studio_error)?;
        self.snapshot()
    }

    pub fn rename_layer(
        &mut self,
        layer_id: u32,
        name: String,
    ) -> Result<StudioKeymapSnapshot, StudioError> {
        self.client
            .set_layer_props(layer_id, name)
            .map_err(map_client_to_studio_error)?;
        self.snapshot()
    }

    pub fn remove_layer(&mut self, layer_index: u32) -> Result<StudioKeymapSnapshot, StudioError> {
        self.client
            .remove_layer(layer_index)
            .map_err(map_client_to_studio_error)?;
        self.snapshot()
    }
}

pub fn key_catalog() -> Vec<KeyCatalogEntry> {
    let mut entries: Vec<_> = Keycode::iter()
        .map(|keycode| {
            let canonical = keycode.to_name().to_string();
            let display = display_key_name(&canonical);
            let hid_usage = keycode.to_hid_usage();
            let category = key_category(&canonical, hid_usage).to_string();
            let names = key_names(&canonical);
            let aliases = key_aliases(&canonical, &display, &names);
            KeyCatalogEntry {
                display,
                canonical,
                hid_usage,
                category,
                aliases,
                names,
            }
        })
        .collect();

    entries.sort_by(|a, b| {
        let a_rank = category_rank(&a.category);
        let b_rank = category_rank(&b.category);
        a_rank
            .cmp(&b_rank)
            .then_with(|| key_sort_value(a).cmp(&key_sort_value(b)))
            .then_with(|| a.display.cmp(&b.display))
    });
    entries
}

fn probe_candidate(candidate: StudioPortCandidate, timeout_ms: u64) -> StudioDeviceStatus {
    let fallback = base_status(&candidate);
    match run_with_timeout(timeout_ms, move || probe_candidate_rpc(candidate)) {
        Some(status) => status,
        None => StudioDeviceStatus {
            rpc_status: StudioRpcStatus::Timeout,
            lock_state: StudioLockState::Unknown,
            keymap_viewer_status: KeymapViewerStatus::Failed,
            error_code: StudioErrorCode::RpcTimeout,
            ..fallback
        },
    }
}

fn probe_candidate_rpc(candidate: StudioPortCandidate) -> StudioDeviceStatus {
    let mut status = base_status(&candidate);
    let mut client = match open_serial_client(&candidate.port_name) {
        Ok(client) => client,
        Err(error) => {
            status.rpc_status = StudioRpcStatus::Failed;
            status.error_code = map_serial_error(&error);
            return status;
        }
    };

    let info = match client.get_device_info() {
        Ok(info) => info,
        Err(error) => {
            status.rpc_status = StudioRpcStatus::Failed;
            status.error_code = map_client_error(&error);
            return status;
        }
    };

    status.rpc_status = StudioRpcStatus::Ok;
    status.error_code = StudioErrorCode::None;
    if !info.name.is_empty() {
        status.display_name = info.name.clone();
        status.product = Some(info.name);
    }
    if !info.serial_number.is_empty() {
        status.serial_number = Some(serial_bytes_to_string(&info.serial_number));
    }

    match client.get_lock_state() {
        Ok(lock_state) => {
            status.lock_state = lock_state_to_status(lock_state);
            status.keymap_viewer_status = match status.lock_state {
                StudioLockState::Unlocked => KeymapViewerStatus::Available,
                StudioLockState::Locked => KeymapViewerStatus::Locked,
                StudioLockState::Unknown => KeymapViewerStatus::Failed,
            };
            if status.lock_state == StudioLockState::Locked {
                status.error_code = StudioErrorCode::Locked;
            }
        }
        Err(error) => {
            status.rpc_status = StudioRpcStatus::Failed;
            status.lock_state = StudioLockState::Unknown;
            status.keymap_viewer_status = KeymapViewerStatus::Failed;
            status.error_code = map_client_error(&error);
        }
    }

    status
}

fn probe_ble_candidate(candidate: StudioBleCandidate, timeout_ms: u64) -> StudioDeviceStatus {
    let fallback = ble_base_status(&candidate);
    match run_with_timeout(timeout_ms, move || probe_ble_candidate_rpc(candidate)) {
        Some(status) => status,
        None => StudioDeviceStatus {
            rpc_status: StudioRpcStatus::Timeout,
            lock_state: StudioLockState::Unknown,
            keymap_viewer_status: KeymapViewerStatus::Failed,
            error_code: StudioErrorCode::RpcTimeout,
            ..fallback
        },
    }
}

fn probe_ble_candidate_rpc(candidate: StudioBleCandidate) -> StudioDeviceStatus {
    let mut status = ble_base_status(&candidate);
    let mut client = match open_ble_client(&candidate.device_id_json) {
        Ok(client) => client,
        Err(error) => {
            status.rpc_status = StudioRpcStatus::Failed;
            status.error_code = map_ble_error(&error);
            return status;
        }
    };

    let info = match client.get_device_info() {
        Ok(info) => info,
        Err(error) => {
            status.rpc_status = StudioRpcStatus::Failed;
            status.error_code = map_client_error(&error);
            return status;
        }
    };

    status.rpc_status = StudioRpcStatus::Ok;
    status.error_code = StudioErrorCode::None;
    if !info.name.is_empty() {
        status.display_name = info.name.clone();
        status.product = Some(info.name);
    }
    if !info.serial_number.is_empty() {
        status.serial_number = Some(serial_bytes_to_string(&info.serial_number));
    }

    match client.get_lock_state() {
        Ok(lock_state) => {
            status.lock_state = lock_state_to_status(lock_state);
            status.keymap_viewer_status = match status.lock_state {
                StudioLockState::Unlocked => KeymapViewerStatus::Available,
                StudioLockState::Locked => KeymapViewerStatus::Locked,
                StudioLockState::Unknown => KeymapViewerStatus::Failed,
            };
            if status.lock_state == StudioLockState::Locked {
                status.error_code = StudioErrorCode::Locked;
            }
        }
        Err(error) => {
            status.rpc_status = StudioRpcStatus::Failed;
            status.lock_state = StudioLockState::Unknown;
            status.keymap_viewer_status = KeymapViewerStatus::Failed;
            status.error_code = map_client_error(&error);
        }
    }

    status
}

fn read_keymap(device_ref: StudioDeviceRef) -> Result<StudioKeymapSnapshot, StudioError> {
    let candidate = resolve_device_candidate(device_ref)?;
    let snapshot_mode = snapshot_mode_for_candidate(&candidate);
    let mut client = open_studio_client(&candidate).map_err(|_| StudioError::RpcFailed)?;
    let info = client
        .get_device_info()
        .map_err(|_| StudioError::RpcFailed)?;
    let lock_state = client
        .get_lock_state()
        .map_err(|_| StudioError::RpcFailed)?;
    let lock_state = lock_state_to_status(lock_state);
    if lock_state == StudioLockState::Locked {
        return Err(StudioError::Locked);
    }

    let (keymap, layout_selection, behavior_names) =
        fetch_snapshot_data(&mut client, snapshot_mode)?;
    Ok(snapshot_from_parts(
        candidate.id(),
        if info.name.is_empty() {
            candidate.display_name()
        } else {
            info.name
        },
        candidate.connection_type().to_string(),
        lock_state,
        keymap,
        layout_selection,
        behavior_names,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StudioSnapshotMode {
    Full,
    LayoutOnly,
}

fn snapshot_mode_for_candidate(candidate: &StudioDeviceCandidate) -> StudioSnapshotMode {
    match candidate {
        StudioDeviceCandidate::Serial(_) => StudioSnapshotMode::Full,
        StudioDeviceCandidate::Ble(_) => StudioSnapshotMode::LayoutOnly,
    }
}

fn fetch_snapshot_data(
    client: &mut StudioClient<StudioTransport>,
    mode: StudioSnapshotMode,
) -> Result<(zmk::keymap::Keymap, LayoutSelection, BTreeMap<i32, String>), StudioError> {
    let keymap = client.get_keymap().map_err(map_client_to_studio_error)?;
    let layout_selection = select_layout(client.get_physical_layouts().ok(), &keymap);
    let behavior_names = match mode {
        StudioSnapshotMode::Full => behavior_names_for_keymap(client, &keymap),
        StudioSnapshotMode::LayoutOnly => BTreeMap::new(),
    };
    Ok((keymap, layout_selection, behavior_names))
}

fn snapshot_from_parts(
    device_id: String,
    device_name: String,
    connection_type: String,
    lock_state: StudioLockState,
    keymap: zmk::keymap::Keymap,
    layout_selection: LayoutSelection,
    behavior_names: BTreeMap<i32, String>,
) -> StudioKeymapSnapshot {
    StudioKeymapSnapshot {
        device_id,
        device_name,
        connection_type,
        lock_state,
        physical_layouts: layout_selection.physical_layouts,
        selected_physical_layout_index: layout_selection.selected_physical_layout_index,
        selected_physical_layout_name: layout_selection.selected_physical_layout_name,
        layout_source: layout_selection.layout_source,
        selected_layout_keys: layout_selection.selected_layout_keys,
        layers: keymap_to_layers(keymap, &behavior_names),
        updated_ms: now_ms(),
    }
}

fn base_status(candidate: &StudioPortCandidate) -> StudioDeviceStatus {
    StudioDeviceStatus {
        id: candidate.id(),
        connection_type: "usb_serial".to_string(),
        port_name: candidate.port_name.clone(),
        display_name: candidate.display_name(),
        vid: candidate.vid,
        pid: candidate.pid,
        serial_number: candidate.serial_number.clone(),
        manufacturer: candidate.manufacturer.clone(),
        product: candidate.product.clone(),
        transport_detected: true,
        rpc_status: StudioRpcStatus::Unavailable,
        lock_state: StudioLockState::Unknown,
        keymap_viewer_status: KeymapViewerStatus::Unsupported,
        error_code: StudioErrorCode::None,
    }
}

fn ble_base_status(candidate: &StudioBleCandidate) -> StudioDeviceStatus {
    StudioDeviceStatus {
        id: candidate.id(),
        connection_type: "ble_studio".to_string(),
        port_name: candidate.endpoint_label(),
        display_name: candidate.endpoint_label(),
        vid: None,
        pid: None,
        serial_number: None,
        manufacturer: None,
        product: candidate.local_name.clone(),
        transport_detected: true,
        rpc_status: StudioRpcStatus::Unavailable,
        lock_state: StudioLockState::Unknown,
        keymap_viewer_status: KeymapViewerStatus::Unsupported,
        error_code: StudioErrorCode::None,
    }
}

fn serial_candidates() -> Result<Vec<StudioPortCandidate>, serialport::Error> {
    let mut candidates: Vec<_> = serialport::available_ports()?
        .into_iter()
        .filter_map(|port| match port.port_type {
            serialport::SerialPortType::UsbPort(info) => Some(StudioPortCandidate {
                port_name: port.port_name,
                vid: Some(info.vid),
                pid: Some(info.pid),
                serial_number: info.serial_number,
                manufacturer: info.manufacturer,
                product: info.product,
            }),
            _ => None,
        })
        .collect();
    candidates.sort_by(|a, b| a.port_name.cmp(&b.port_name));
    Ok(candidates)
}

fn ble_candidates() -> Result<Vec<StudioBleCandidate>, zmk_studio_api::transport::PlatformBleError>
{
    let mut candidates: Vec<_> = StudioClient::<PlatformBleTransport>::list_ble_devices()?
        .into_iter()
        .map(|device| StudioBleCandidate {
            device_id_json: device.device_id,
            local_name: device.local_name,
        })
        .collect();
    candidates.sort_by(|a, b| a.endpoint_label().cmp(&b.endpoint_label()));
    Ok(candidates)
}

fn resolve_device_candidate(
    device_ref: StudioDeviceRef,
) -> Result<StudioDeviceCandidate, StudioError> {
    match device_ref {
        StudioDeviceRef::Serial(id) => {
            let candidates = serial_candidates().map_err(|_| StudioError::DeviceNotFound)?;
            candidates
                .into_iter()
                .find(|candidate| candidate.id() == id || candidate.legacy_id() == id)
                .map(StudioDeviceCandidate::Serial)
                .ok_or(StudioError::DeviceNotFound)
        }
        StudioDeviceRef::Ble(device_id_json) => {
            Ok(StudioDeviceCandidate::Ble(StudioBleCandidate {
                device_id_json,
                local_name: None,
            }))
        }
    }
}

fn open_studio_client(
    candidate: &StudioDeviceCandidate,
) -> Result<StudioClient<StudioTransport>, StudioOpenError> {
    match candidate {
        StudioDeviceCandidate::Serial(candidate) => {
            open_serial_client(&candidate.port_name).map_err(|_| StudioOpenError::Serial)
        }
        StudioDeviceCandidate::Ble(candidate) => {
            open_ble_client(&candidate.device_id_json).map_err(|_| StudioOpenError::Ble)
        }
    }
}

fn open_serial_client(path: &str) -> Result<StudioClient<StudioTransport>, SerialTransportError> {
    let transport = SerialTransport::open(path)?;
    Ok(StudioClient::new(StudioTransport::Serial(transport)))
}

fn open_ble_client(
    device_id_json: &str,
) -> Result<StudioClient<StudioTransport>, zmk_studio_api::transport::PlatformBleError> {
    let transport = PlatformBleTransport::connect_device(device_id_json)?;
    Ok(StudioClient::new(StudioTransport::Ble(transport)))
}

enum StudioOpenError {
    Serial,
    Ble,
}

#[derive(Debug, Clone)]
struct LayoutSelection {
    physical_layouts: Vec<StudioPhysicalLayout>,
    selected_physical_layout_index: Option<usize>,
    selected_physical_layout_name: Option<String>,
    layout_source: StudioLayoutSource,
    selected_layout_keys: Vec<StudioPhysicalKey>,
}

fn select_layout(
    physical_layouts: Option<zmk::keymap::PhysicalLayouts>,
    keymap: &zmk::keymap::Keymap,
) -> LayoutSelection {
    if let Some(layouts) = physical_layouts {
        let active_index = layouts.active_layout_index as usize;
        let converted = physical_layouts_to_view(layouts.layouts);
        if let Some(selected) = converted.get(active_index) {
            if !selected.keys.is_empty() {
                let selected_name = selected.name.clone();
                let selected_keys = selected.keys.clone();
                return LayoutSelection {
                    physical_layouts: converted,
                    selected_physical_layout_index: Some(active_index),
                    selected_physical_layout_name: Some(selected_name),
                    layout_source: StudioLayoutSource::StudioPhysicalLayout,
                    selected_layout_keys: selected_keys,
                };
            }
        }

        return LayoutSelection {
            physical_layouts: converted,
            selected_physical_layout_index: None,
            selected_physical_layout_name: None,
            layout_source: StudioLayoutSource::GridFallback,
            selected_layout_keys: grid_fallback_keys(max_binding_count(keymap)),
        };
    }

    LayoutSelection {
        physical_layouts: Vec::new(),
        selected_physical_layout_index: None,
        selected_physical_layout_name: None,
        layout_source: StudioLayoutSource::GridFallback,
        selected_layout_keys: grid_fallback_keys(max_binding_count(keymap)),
    }
}

fn physical_layouts_to_view(
    layouts: Vec<zmk::keymap::PhysicalLayout>,
) -> Vec<StudioPhysicalLayout> {
    layouts
        .into_iter()
        .enumerate()
        .map(|(index, layout)| StudioPhysicalLayout {
            index,
            name: if layout.name.is_empty() {
                format!("Layout {}", index)
            } else {
                layout.name
            },
            keys: layout
                .keys
                .into_iter()
                .enumerate()
                .map(|(position, key)| StudioPhysicalKey {
                    position,
                    x: key.x,
                    y: key.y,
                    width: key.width,
                    height: key.height,
                    r: key.r,
                    rx: key.rx,
                    ry: key.ry,
                })
                .collect(),
        })
        .collect()
}

fn max_binding_count(keymap: &zmk::keymap::Keymap) -> usize {
    keymap
        .layers
        .iter()
        .map(|layer| layer.bindings.len())
        .max()
        .unwrap_or(0)
}

fn grid_fallback_keys(position_count: usize) -> Vec<StudioPhysicalKey> {
    if position_count == 0 {
        return Vec::new();
    }

    let columns = (((position_count as f64) * 1.6).sqrt().ceil() as usize).clamp(1, 12);
    (0..position_count)
        .map(|position| StudioPhysicalKey {
            position,
            x: ((position % columns) * 110) as i32,
            y: ((position / columns) * 110) as i32,
            width: 100,
            height: 100,
            r: 0,
            rx: 0,
            ry: 0,
        })
        .collect()
}

fn behavior_names_for_keymap(
    client: &mut StudioClient<StudioTransport>,
    keymap: &zmk::keymap::Keymap,
) -> BTreeMap<i32, String> {
    let ids: BTreeSet<u32> = keymap
        .layers
        .iter()
        .flat_map(|layer| layer.bindings.iter())
        .filter_map(|binding| u32::try_from(binding.behavior_id).ok())
        .collect();

    ids.into_iter()
        .filter_map(|id| {
            client
                .get_behavior_details(id)
                .ok()
                .map(|details| (id as i32, details.display_name))
        })
        .collect()
}

fn behavior_names_for_raw_bindings(
    client: &mut StudioClient<StudioTransport>,
    raw_bindings: &[StudioRawBinding],
    timeout: Duration,
) -> BTreeMap<i32, String> {
    let deadline = Instant::now() + timeout;
    let ids: BTreeSet<u32> = raw_bindings
        .iter()
        .filter_map(|binding| u32::try_from(binding.behavior_id).ok())
        .collect();
    let mut names = BTreeMap::new();

    for id in ids {
        if Instant::now() >= deadline {
            break;
        }
        match client.get_behavior_details(id) {
            Ok(details) => {
                names.insert(id as i32, details.display_name);
            }
            Err(error) => {
                tracing::debug!(
                    behavior_id = id,
                    error = %error,
                    "failed to resolve Studio behavior details"
                );
            }
        }
    }

    names
}

fn label_patches_for_raw_bindings(
    raw_bindings: Vec<StudioRawBinding>,
    behavior_names: &BTreeMap<i32, String>,
) -> Vec<StudioBindingLabelPatch> {
    let mut seen = BTreeSet::new();
    raw_bindings
        .into_iter()
        .filter_map(|raw| {
            if !seen.insert((raw.behavior_id, raw.param1, raw.param2)) {
                return None;
            }
            let behavior = behavior_names.get(&raw.behavior_id)?.clone();
            let labels = binding_labels(&behavior, raw.param1, raw.param2);
            Some(StudioBindingLabelPatch {
                behavior_id: raw.behavior_id,
                param1: raw.param1,
                param2: raw.param2,
                behavior,
                binding_label: labels.full_label.clone(),
                primary_label: labels.primary_label,
                secondary_label: labels.secondary_label,
                full_label: labels.full_label,
            })
        })
        .collect()
}

fn keymap_to_layers(
    keymap: zmk::keymap::Keymap,
    behavior_names: &BTreeMap<i32, String>,
) -> Vec<StudioLayer> {
    keymap
        .layers
        .into_iter()
        .enumerate()
        .map(|(index, layer)| StudioLayer {
            index,
            id: layer.id,
            name: if layer.name.is_empty() {
                format!("Layer {}", index)
            } else {
                layer.name
            },
            bindings: layer
                .bindings
                .into_iter()
                .enumerate()
                .map(|(position, binding)| binding_to_view(position, binding, behavior_names))
                .collect(),
        })
        .collect()
}

struct BindingLabels {
    primary_label: String,
    secondary_label: String,
    full_label: String,
}

fn binding_labels(behavior: &str, param1: u32, param2: u32) -> BindingLabels {
    let behavior_key = behavior.trim().to_ascii_lowercase();
    let labels = match behavior_key.as_str() {
        "key press" => Some((
            key_label(param1),
            String::new(),
            format!("&kp {}", key_label(param1)),
        )),
        "key toggle" => Some((
            "&kt".to_string(),
            key_label(param1),
            format!("&kt {}", key_label(param1)),
        )),
        "sticky key" | "sk" => Some((
            format!("sk {}", behavior_key_display_label(param1, false)),
            String::new(),
            format!("&sk {}", behavior_key_display_label(param1, true)),
        )),
        "layer-tap" => Some((
            format!("lt {} {}", param1, display_key_label(param2)),
            String::new(),
            format!("&lt {} {}", param1, zmk_key_label(param2)),
        )),
        "mod-tap" => Some((
            format!(
                "mt {} {}",
                modifier_combo_label(param1, false),
                display_key_label(param2)
            ),
            String::new(),
            format!(
                "&mt {} {}",
                modifier_combo_label(param1, true),
                zmk_key_label(param2)
            ),
        )),
        "momentary layer" => Some((
            format!("mo {}", param1),
            String::new(),
            format!("&mo {}", param1),
        )),
        "toggle layer" => Some((
            format!("tog {}", param1),
            String::new(),
            format!("&tog {}", param1),
        )),
        "to layer" => Some((
            format!("to {}", param1),
            String::new(),
            format!("&to {}", param1),
        )),
        "sticky layer" | "sl" => Some((
            format!("sl {}", param1),
            String::new(),
            format!("&sl {}", param1),
        )),
        "bluetooth" | "bt" => Some(bluetooth_labels(param1, param2)),
        "output selection" | "output" | "out" => Some(output_labels(param1)),
        "mouse key press" | "mouse_key_press" | "mkp" => Some(mouse_button_labels(param1)),
        "mouse move" | "mouse_move" | "mmv" => Some(mouse_move_labels(param1)),
        "mouse scroll" | "mouse_scroll" | "msc" => Some(mouse_scroll_labels(param1)),
        "transparent" => Some(("&trans".to_string(), String::new(), "&trans".to_string())),
        "none" => Some((String::new(), String::new(), "&none".to_string())),
        "caps word" | "caps_word" => Some((
            "caps word".to_string(),
            String::new(),
            "&caps_word".to_string(),
        )),
        "key repeat" | "key_repeat" => Some((
            "key repeat".to_string(),
            String::new(),
            "&key_repeat".to_string(),
        )),
        "studio unlock" | "studio_unlock" => Some((
            "studio unlock".to_string(),
            String::new(),
            "&studio_unlock".to_string(),
        )),
        "bootloader" => Some((
            "bootloader".to_string(),
            String::new(),
            "&bootloader".to_string(),
        )),
        "reset" => Some(("reset".to_string(), String::new(), "&reset".to_string())),
        "grave/escape" | "grave escape" | "grave_escape" | "gresc" => Some((
            "grave escape".to_string(),
            String::new(),
            "&gresc".to_string(),
        )),
        _ => None,
    };

    if let Some((primary_label, secondary_label, full_label)) = labels {
        return BindingLabels {
            primary_label,
            secondary_label,
            full_label,
        };
    }

    let full_label = format!("{}({}, {})", behavior, param1, param2);
    BindingLabels {
        primary_label: behavior.to_string(),
        secondary_label: String::new(),
        full_label,
    }
}

fn mouse_button_labels(value: u32) -> (String, String, String) {
    match value {
        0x01 => (
            "Left Click".to_string(),
            String::new(),
            "&mkp LCLK".to_string(),
        ),
        0x02 => (
            "Right Click".to_string(),
            String::new(),
            "&mkp RCLK".to_string(),
        ),
        0x04 => (
            "Middle Click".to_string(),
            String::new(),
            "&mkp MCLK".to_string(),
        ),
        0x08 => (
            "Button 4".to_string(),
            String::new(),
            "&mkp MB4".to_string(),
        ),
        0x10 => (
            "Button 5".to_string(),
            String::new(),
            "&mkp MB5".to_string(),
        ),
        _ => (
            format!("Mouse Button {}", value),
            String::new(),
            format!("&mkp {}", value),
        ),
    }
}

fn mouse_move_labels(value: u32) -> (String, String, String) {
    match value {
        0x0000_FDA8 => (
            "Move Up".to_string(),
            String::new(),
            "&mmv MOVE_UP".to_string(),
        ),
        0x0000_0258 => (
            "Move Down".to_string(),
            String::new(),
            "&mmv MOVE_DOWN".to_string(),
        ),
        0xFDA8_0000 => (
            "Move Left".to_string(),
            String::new(),
            "&mmv MOVE_LEFT".to_string(),
        ),
        0x0258_0000 => (
            "Move Right".to_string(),
            String::new(),
            "&mmv MOVE_RIGHT".to_string(),
        ),
        _ => (
            format!("Move {}", value),
            String::new(),
            format!("&mmv {}", value),
        ),
    }
}

fn mouse_scroll_labels(value: u32) -> (String, String, String) {
    match value {
        0x0000_000A => (
            "Scroll Up".to_string(),
            String::new(),
            "&msc SCRL_UP".to_string(),
        ),
        0x0000_FFF6 => (
            "Scroll Down".to_string(),
            String::new(),
            "&msc SCRL_DOWN".to_string(),
        ),
        0xFFF6_0000 => (
            "Scroll Left".to_string(),
            String::new(),
            "&msc SCRL_LEFT".to_string(),
        ),
        0x000A_0000 => (
            "Scroll Right".to_string(),
            String::new(),
            "&msc SCRL_RIGHT".to_string(),
        ),
        _ => (
            format!("Scroll {}", value),
            String::new(),
            format!("&msc {}", value),
        ),
    }
}

fn bluetooth_labels(command: u32, value: u32) -> (String, String, String) {
    match command {
        0 => (
            "bt CLR".to_string(),
            String::new(),
            "&bt BT_CLR".to_string(),
        ),
        1 => (
            "bt NEXT".to_string(),
            String::new(),
            "&bt BT_NXT".to_string(),
        ),
        2 => (
            "bt PREV".to_string(),
            String::new(),
            "&bt BT_PRV".to_string(),
        ),
        3 => (
            format!("bt {}", value),
            String::new(),
            format!("&bt BT_SEL {}", value),
        ),
        4 => (
            "bt CLR ALL".to_string(),
            String::new(),
            "&bt BT_CLR_ALL".to_string(),
        ),
        5 => (
            format!("bt DISC {}", value),
            String::new(),
            format!("&bt BT_DISC {}", value),
        ),
        _ => (
            format!("bt {} {}", command, value),
            String::new(),
            format!("&bt {} {}", command, value),
        ),
    }
}

fn output_labels(value: u32) -> (String, String, String) {
    match value {
        0 => (
            "out TOG".to_string(),
            String::new(),
            "&out OUT_TOG".to_string(),
        ),
        1 => (
            "out USB".to_string(),
            String::new(),
            "&out OUT_USB".to_string(),
        ),
        2 => (
            "out BLE".to_string(),
            String::new(),
            "&out OUT_BLE".to_string(),
        ),
        3 => (
            "out NONE".to_string(),
            String::new(),
            "&out OUT_NONE".to_string(),
        ),
        _ => (
            format!("out {}", value),
            String::new(),
            format!("&out {}", value),
        ),
    }
}

fn key_label(encoded: u32) -> String {
    let usage = HidUsage::from_encoded(encoded);
    let base = usage
        .known_base_keycode()
        .map(|keycode| normalize_key_name(keycode.to_name()))
        .unwrap_or_else(|| usage.base().to_string());
    usage
        .modifier_labels()
        .into_iter()
        .rev()
        .fold(base, |label, modifier| {
            format!("{}({})", modifier_label(modifier), label)
        })
}

fn display_key_label(encoded: u32) -> String {
    let usage = HidUsage::from_encoded(encoded);
    let base = usage
        .known_base_keycode()
        .map(|keycode| display_key_name(keycode.to_name()))
        .unwrap_or_else(|| usage.base().to_string());
    usage
        .modifier_labels()
        .into_iter()
        .rev()
        .fold(base, |label, modifier| {
            format!("{}({})", modifier_label(modifier), label)
        })
}

fn zmk_key_label(encoded: u32) -> String {
    let usage = HidUsage::from_encoded(encoded);
    let base = usage
        .known_base_keycode()
        .map(|keycode| zmk_key_name(keycode.to_name()))
        .unwrap_or_else(|| usage.base().to_string());
    usage
        .modifier_labels()
        .into_iter()
        .rev()
        .fold(base, |label, modifier| {
            format!("{}({})", modifier_label(modifier), label)
        })
}

fn zmk_key_name(name: &str) -> String {
    match normalize_key_name(name).as_str() {
        "SPC" => "SPACE".to_string(),
        normalized => normalized.to_string(),
    }
}

fn behavior_key_display_label(encoded: u32, zmk_names: bool) -> String {
    let usage = HidUsage::from_encoded(encoded);
    let base_is_modifier = usage
        .known_base_keycode()
        .and_then(|keycode| modifier_name(&normalize_key_name(keycode.to_name()), zmk_names))
        .is_some();
    // A modifier keycode itself (e.g. &sk LSHIFT) is shown as a modifier combo
    // ("LShift"). A normal key carrying implicit modifier bits (e.g. &sk LC(A))
    // must use the nested LC(A) form instead, which zmk_/display_key_label
    // already render.
    if base_is_modifier {
        return modifier_combo_label(encoded, zmk_names);
    }
    if zmk_names {
        zmk_key_label(encoded)
    } else {
        display_key_label(encoded)
    }
}

fn modifier_combo_label(encoded: u32, zmk_names: bool) -> String {
    let usage = HidUsage::from_encoded(encoded);
    let mut labels = Vec::new();
    if let Some(keycode) = usage.known_base_keycode() {
        let name = normalize_key_name(keycode.to_name());
        if let Some(label) = modifier_name(&name, zmk_names) {
            labels.push(label);
        } else {
            labels.push(if zmk_names {
                name
            } else {
                display_key_name(&name)
            });
        }
    }
    for modifier in usage.modifier_labels() {
        if let Some(label) = modifier_name(modifier, zmk_names) {
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
    }
    if labels.is_empty() {
        return key_label(encoded);
    }
    labels.join("+")
}

fn modifier_name(name: &str, zmk_names: bool) -> Option<String> {
    let zmk = match name {
        "LEFT_CONTROL" | "LCTL" | "LCTRL" => "LCTRL",
        "LEFT_SHIFT" | "LSFT" | "LSHFT" | "LSHIFT" => "LSHIFT",
        "LEFT_ALT" | "LALT" => "LALT",
        "LEFT_GUI" | "LGUI" => "LGUI",
        "RIGHT_CONTROL" | "RCTL" | "RCTRL" => "RCTRL",
        "RIGHT_SHIFT" | "RSFT" | "RSHFT" | "RSHIFT" => "RSHIFT",
        "RIGHT_ALT" | "RALT" => "RALT",
        "RIGHT_GUI" | "RGUI" => "RGUI",
        _ => return None,
    };
    if zmk_names {
        return Some(zmk.to_string());
    }
    Some(
        match zmk {
            "LCTRL" => "LCtrl",
            "LSHIFT" => "LShift",
            "LALT" => "LAlt",
            "LGUI" => "LGUI",
            "RCTRL" => "RCtrl",
            "RSHIFT" => "RShift",
            "RALT" => "RAlt",
            "RGUI" => "RGUI",
            _ => zmk,
        }
        .to_string(),
    )
}

fn normalize_key_name(name: &str) -> String {
    if let Some(digit) = keypad_digit(name) {
        return format!("Num {}", digit);
    }
    if let Some(digit) = number_key_digit(name) {
        return us_number_display(digit);
    }
    match name {
        "LEFT_COMMAND" | "LCMD" | "LEFT_META" | "LMETA" | "LEFT_WIN" | "LWIN" => {
            "LEFT_GUI".to_string()
        }
        "RIGHT_COMMAND" | "RCMD" | "RIGHT_META" | "RMETA" | "RIGHT_WIN" | "RWIN" => {
            "RIGHT_GUI".to_string()
        }
        _ => name.to_string(),
    }
}

fn number_key_digit(name: &str) -> Option<char> {
    const PREFIXES: [&str; 3] = ["NUMBER_", "NUM_", "N"];
    for prefix in PREFIXES {
        if let Some(rest) = name.strip_prefix(prefix) {
            if rest.len() == 1 {
                let digit = rest.as_bytes()[0] as char;
                if digit.is_ascii_digit() {
                    return Some(digit);
                }
            }
        }
    }
    None
}
fn modifier_label(label: &str) -> &str {
    match label {
        "LCTL" => "LC",
        "LSFT" => "LS",
        "LALT" => "LA",
        "LGUI" => "LG",
        "RCTL" => "RC",
        "RSFT" => "RS",
        "RALT" => "RA",
        "RGUI" => "RG",
        _ => label,
    }
}

fn behavior_to_zmk(behavior: EditBehavior) -> Behavior {
    match behavior {
        EditBehavior::KeyPress(encoded) => Behavior::KeyPress(HidUsage::from_encoded(encoded)),
        EditBehavior::Transparent => Behavior::Transparent,
        EditBehavior::None => Behavior::None,
        EditBehavior::MomentaryLayer(layer) => Behavior::MomentaryLayer { layer_id: layer },
        EditBehavior::ToggleLayer(layer) => Behavior::ToggleLayer { layer_id: layer },
        EditBehavior::ToLayer(layer) => Behavior::ToLayer { layer_id: layer },
        EditBehavior::ModTap { hold, tap } => Behavior::ModTap {
            hold: HidUsage::from_encoded(hold),
            tap: HidUsage::from_encoded(tap),
        },
        EditBehavior::LayerTap {
            target_layer_index,
            tap,
        } => Behavior::LayerTap {
            layer_id: target_layer_index,
            tap: HidUsage::from_encoded(tap),
        },
        EditBehavior::StickyKey(encoded) => Behavior::StickyKey(HidUsage::from_encoded(encoded)),
        EditBehavior::StickyLayer(layer) => Behavior::StickyLayer { layer_id: layer },
        EditBehavior::Bluetooth { command, value } => Behavior::Bluetooth { command, value },
        EditBehavior::OutputSelection(value) => Behavior::OutputSelection { value },
        EditBehavior::MouseKeyPress(value) => Behavior::MouseKeyPress { value },
        EditBehavior::MouseMove(value) => Behavior::MouseMove { value },
        EditBehavior::MouseScroll(value) => Behavior::MouseScroll { value },
        EditBehavior::CapsWord => Behavior::CapsWord,
        EditBehavior::KeyRepeat => Behavior::KeyRepeat,
        EditBehavior::Reset => Behavior::Reset,
        EditBehavior::Bootloader => Behavior::Bootloader,
        EditBehavior::StudioUnlock => Behavior::StudioUnlock,
        EditBehavior::GraveEscape => Behavior::GraveEscape,
    }
}

fn map_client_to_studio_error(error: ClientError) -> StudioError {
    match error {
        ClientError::Io(err) if err.kind() == std::io::ErrorKind::TimedOut => StudioError::Timeout,
        ClientError::Io(_) => StudioError::Disconnected,
        ClientError::Meta(_) => StudioError::Locked,
        ClientError::SetLayerBindingFailed(code) => match code {
            zmk::keymap::SetLayerBindingResponse::SetLayerBindingRespInvalidLocation => {
                StudioError::InvalidLocation
            }
            zmk::keymap::SetLayerBindingResponse::SetLayerBindingRespInvalidBehavior => {
                StudioError::InvalidBehavior
            }
            zmk::keymap::SetLayerBindingResponse::SetLayerBindingRespInvalidParameters => {
                StudioError::InvalidParameters
            }
            _ => StudioError::RpcFailed,
        },
        ClientError::InvalidLayerOrPosition { .. } => StudioError::InvalidLocation,
        ClientError::MissingBehaviorRole(_) => StudioError::MissingBehaviorRole,
        ClientError::SaveChangesFailed(code) => match code {
            zmk::keymap::SaveChangesErrorCode::SaveChangesErrGeneric => StudioError::SaveFailed,
            zmk::keymap::SaveChangesErrorCode::SaveChangesErrNotSupported => {
                StudioError::SaveNotSupported
            }
            zmk::keymap::SaveChangesErrorCode::SaveChangesErrNoSpace => StudioError::SaveNoSpace,
            _ => StudioError::SaveFailed,
        },
        ClientError::AddLayerFailed(code) => match code {
            zmk::keymap::AddLayerErrorCode::AddLayerErrNoSpace => StudioError::AddLayerNoSpace,
            _ => StudioError::AddLayerFailed,
        },
        ClientError::RemoveLayerFailed(code) => match code {
            zmk::keymap::RemoveLayerErrorCode::RemoveLayerErrInvalidIndex => {
                StudioError::InvalidLayer
            }
            _ => StudioError::RemoveLayerFailed,
        },
        ClientError::SetLayerPropsFailed(code) => match code {
            zmk::keymap::SetLayerPropsResponse::SetLayerPropsRespErrInvalidId => {
                StudioError::InvalidLayer
            }
            _ => StudioError::RenameLayerFailed,
        },
        _ => StudioError::RpcFailed,
    }
}

fn display_key_name(canonical: &str) -> String {
    if let Some(digit) = keypad_digit(canonical) {
        return format!("Num {}", digit);
    }
    let normalized = normalize_key_name(canonical);
    match normalized.as_str() {
        "ESC" | "ESCAPE" => "Esc".to_string(),
        "BKSP" | "BSPC" | "BACKSPACE" => "Backspace".to_string(),
        "RET" | "RETURN" => "Enter".to_string(),
        "SPACE" => "Space".to_string(),
        "SPC" => "Space".to_string(),
        "DELETE" => "Delete".to_string(),
        "LEFT_CONTROL" | "LCTL" | "LCTRL" => "Left Control".to_string(),
        "RIGHT_CONTROL" | "RCTL" | "RCTRL" => "Right Control".to_string(),
        "LEFT_SHIFT" | "LSFT" | "LSHFT" | "LSHIFT" => "Left Shift".to_string(),
        "RIGHT_SHIFT" | "RSFT" | "RSHFT" | "RSHIFT" => "Right Shift".to_string(),
        "LEFT_ALT" | "LALT" => "Left Alt".to_string(),
        "RIGHT_ALT" | "RALT" => "Right Alt".to_string(),
        "LEFT_COMMAND" | "LCMD" | "LEFT_GUI" | "LGUI" | "LEFT_META" | "LMETA" | "LEFT_WIN"
        | "LWIN" => "Left GUI".to_string(),
        "RIGHT_COMMAND" | "RCMD" | "RIGHT_GUI" | "RGUI" | "RIGHT_META" | "RMETA" | "RIGHT_WIN"
        | "RWIN" => "Right GUI".to_string(),
        "LEFT_ARROW" | "LARW" | "LEFT" => "Left Arrow".to_string(),
        "RIGHT_ARROW" | "RARW" | "RIGHT" => "Right Arrow".to_string(),
        "UP_ARROW" | "UARW" | "UP" => "Up Arrow".to_string(),
        "DOWN_ARROW" | "DARW" | "DOWN" => "Down Arrow".to_string(),
        "PAGE_UP" => "Page Up".to_string(),
        "PAGE_DOWN" => "Page Down".to_string(),
        "PRINTSCREEN" => "Print Screen".to_string(),
        "PAUSE_BREAK" => "Pause / Break".to_string(),
        "CAPSLOCK" => "Caps Lock".to_string(),
        "SCROLLLOCK" => "Scroll Lock".to_string(),
        "KP_NUMLOCK" => "Numlock and Clear".to_string(),
        "KP_ENTER" => "Enter".to_string(),
        "KP_DOT" => "Decimal Separator".to_string(),
        "KP_EQUAL" => "Equal".to_string(),
        "KP_PLUS" => "Plus".to_string(),
        "KP_SUBTRACT" => "Minus".to_string(),
        "KP_ASTERISK" => "Asterisk / Star".to_string(),
        "KP_DIVIDE" => "Forward Slash".to_string(),
        "SINGLE_QUOTE" | "APOS" | "APOSTROPHE" | "QUOT" | "SQT" => "' \"".to_string(),
        "DOUBLE_QUOTES" | "DQT" => "\"".to_string(),
        "MINUS" => "- _".to_string(),
        "EQUAL" | "EQL" => "= +".to_string(),
        "GRAVE" | "GRAV" => "` ~".to_string(),
        "COMMA" | "CMMA" => ", <".to_string(),
        "PERIOD" | "DOT" => ". >".to_string(),
        "SLASH" | "FSLH" => "/ ?".to_string(),
        "BACKSLASH" | "BSLH" => "\\ |".to_string(),
        "SEMICOLON" | "SCLN" | "SEMI" => "; :".to_string(),
        "LEFT_BRACKET" | "LBKT" => "[ {".to_string(),
        "RIGHT_BRACKET" | "RBKT" => "] }".to_string(),
        "LEFT_BRACE" | "LBRC" => "{".to_string(),
        "RIGHT_BRACE" | "RBRC" => "}".to_string(),
        "LEFT_PARENTHESIS" | "LPAR" => "(".to_string(),
        "RIGHT_PARENTHESIS" | "RPAR" => ")".to_string(),
        "EXCLAMATION" | "EXCL" => "!".to_string(),
        "AT_SIGN" | "AT" => "@".to_string(),
        "HASH" | "POUND" => "#".to_string(),
        "DOLLAR" | "DLLR" => "$".to_string(),
        "PERCENT" | "PRCNT" => "%".to_string(),
        "CARET" => "^".to_string(),
        "AMPERSAND" | "AMPS" => "&".to_string(),
        "ASTERISK" | "ASTRK" | "STAR" => "*".to_string(),
        "PLUS" => "+".to_string(),
        "UNDERSCORE" | "UNDER" => "_".to_string(),
        "QUESTION" | "QMARK" => "?".to_string(),
        "PIPE" => "|".to_string(),
        "COLON" => ":".to_string(),
        "LESS_THAN" | "LT" => "<".to_string(),
        "GREATER_THAN" | "GT" => ">".to_string(),
        "TILDE" => "~".to_string(),
        other => other.replace('_', " "),
    }
}

fn key_category(canonical: &str, hid_usage: u32) -> &'static str {
    let usage_page = (hid_usage >> 16) & 0xff;
    let usage_id = hid_usage & 0xffff;
    let keyboard_modifier_bits = hid_usage >> 24;

    if usage_page == 0x01 {
        return "power_lock";
    }
    if is_power_lock_keycode(canonical) {
        return "power_lock";
    }

    if usage_page == 0x07 {
        if is_keypad_usage(usage_id) {
            return "keypad";
        }
        if canonical.len() == 1 && canonical.as_bytes()[0].is_ascii_uppercase() {
            return "letters";
        }
        if (0x04..=0x1d).contains(&usage_id) {
            return "letters";
        }
        if (0x1e..=0x27).contains(&usage_id) && keyboard_modifier_bits == 0 {
            return "numbers";
        }
        if is_keyboard_symbol_usage(usage_id)
            || ((0x1e..=0x27).contains(&usage_id) && keyboard_modifier_bits != 0)
        {
            return "symbols";
        }
        if matches!(usage_id, 0x28..=0x2c | 0x49 | 0x4c | 0x9e) {
            return "control";
        }
        if matches!(usage_id, 0x4a | 0x4b | 0x4d..=0x52 | 0x65) {
            return "navigation";
        }
        if matches!(usage_id, 0x39 | 0x47 | 0x82..=0x84) {
            return "locks";
        }
        if matches!(usage_id, 0x3a..=0x45 | 0x68..=0x73) {
            return "function";
        }
        if (0x87..=0x8f).contains(&usage_id) {
            return "international";
        }
        if (0x90..=0x98).contains(&usage_id) {
            return "language";
        }
        if (0xe0..=0xe7).contains(&usage_id) {
            return "modifiers";
        }
        if is_keyboard_editing_usage(usage_id) {
            return "editing";
        }
        if is_keyboard_media_usage(usage_id) {
            return "media";
        }
        if is_keyboard_application_usage(usage_id) {
            return "applications";
        }
        return "miscellaneous";
    }

    if is_editing_keycode(canonical) {
        return "editing";
    }
    if canonical.starts_with("C_KEYBOARD_INPUT_ASSIST") {
        return "input_assist";
    }
    if canonical.starts_with("C_AL_") || is_application_keycode(canonical) {
        return "applications";
    }
    if is_media_keycode(canonical) {
        return "media";
    }
    "miscellaneous"
}

fn category_rank(category: &str) -> u8 {
    match category {
        "letters" => 0,
        "numbers" => 1,
        "modifiers" => 2,
        "control" => 3,
        "symbols" => 4,
        "navigation" => 5,
        "locks" => 6,
        "function" => 7,
        "international" => 8,
        "language" => 9,
        "keypad" => 10,
        "editing" => 11,
        "media" => 12,
        "applications" => 13,
        "input_assist" => 14,
        "power_lock" => 15,
        "miscellaneous" => 16,
        _ => 17,
    }
}

fn key_sort_value(entry: &KeyCatalogEntry) -> u32 {
    if entry.category == "letters" {
        return entry.display.as_bytes().first().copied().unwrap_or(b'Z') as u32;
    }
    if entry.category == "keypad" {
        if let Some(digit) = keypad_digit(&entry.canonical) {
            return digit.to_digit(10).unwrap_or(99);
        }
    }
    if let Some(digit) = number_key_digit(&entry.canonical) {
        return digit.to_digit(10).unwrap_or(99);
    }
    entry.hid_usage
}

fn is_editing_keycode(canonical: &str) -> bool {
    matches!(
        canonical,
        "CUT"
            | "COPY"
            | "PSTE"
            | "UNDO"
            | "K_REDO"
            | "K_CUT"
            | "K_COPY"
            | "K_PASTE"
            | "K_UNDO"
            | "K_AGAIN"
            | "C_AC_CUT"
            | "C_AC_COPY"
            | "C_AC_PASTE"
            | "C_AC_UNDO"
            | "C_AC_REDO"
    )
}

fn is_keypad_usage(usage_id: u32) -> bool {
    matches!(
        usage_id,
        0x53..=0x63 | 0x67 | 0x85 | 0x86 | 0xb6 | 0xb7 | 0xd8
    )
}

fn is_keyboard_symbol_usage(usage_id: u32) -> bool {
    matches!(usage_id, 0x2d..=0x38 | 0x64)
}

fn is_keyboard_editing_usage(usage_id: u32) -> bool {
    matches!(usage_id, 0x79..=0x7d)
}

fn is_keyboard_media_usage(usage_id: u32) -> bool {
    matches!(usage_id, 0x7f..=0x81 | 0xe8..=0xef | 0xf3)
}

fn is_keyboard_application_usage(usage_id: u32) -> bool {
    matches!(usage_id, 0x74..=0x78 | 0x7e | 0xf0..=0xf2 | 0xf4..=0xf6 | 0xfa | 0xfb)
}

fn is_application_keycode(canonical: &str) -> bool {
    canonical.starts_with("C_AC_")
        || matches!(
            canonical,
            "K_MENU"
                | "K_SELECT"
                | "K_EXECUTE"
                | "K_REFRESH"
                | "K_STOP"
                | "K_FORWARD"
                | "K_BACK"
                | "K_FIND"
                | "K_FIND2"
                | "K_SCROLL_UP"
                | "K_SCROLL_DOWN"
                | "K_CALCULATOR"
                | "K_HELP"
                | "K_WWW"
        )
}

fn is_media_keycode(canonical: &str) -> bool {
    matches!(
        canonical,
        "K_MUTE"
            | "K_MUTE2"
            | "K_VOLUME_UP"
            | "K_VOLUME_UP2"
            | "K_VOLUME_DOWN"
            | "K_VOLUME_DOWN2"
            | "K_PLAY_PAUSE"
            | "K_STOP2"
            | "K_STOP3"
            | "K_PREVIOUS"
            | "K_NEXT"
            | "K_EJECT"
    ) || (canonical.starts_with("C_")
        && !canonical.starts_with("C_AC_")
        && !canonical.starts_with("C_AL_")
        && !canonical.starts_with("C_KEYBOARD_INPUT_ASSIST")
        && !is_power_lock_keycode(canonical))
}

fn is_power_lock_keycode(canonical: &str) -> bool {
    canonical.starts_with("SYSTEM_")
        || matches!(
            canonical,
            "K_POWER"
                | "K_SLEEP"
                | "K_SCREENSAVER"
                | "C_POWER"
                | "C_RESET"
                | "C_SLEEP"
                | "C_SLEEP_MODE"
                | "C_AL_SCREENSAVER"
                | "C_AL_LOGOFF"
        )
}

fn key_names(canonical: &str) -> Vec<String> {
    let extras: &[&str] = match canonical {
        "RETURN" => &["ENTER", "RET"],
        "ESCAPE" => &["ESC"],
        "BACKSPACE" => &["BKSP", "BSPC"],
        "SPACE" => &["SPC"],
        "EQUAL" | "EQL" => &["EQUAL", "EQL"],
        "LEFT_BRACKET" => &["LBKT"],
        "RIGHT_BRACKET" => &["RBKT"],
        "BACKSLASH" => &["BSLH"],
        "NON_US_HASH" => &["NUHS"],
        "SEMICOLON" => &["SCLN", "SEMI"],
        "SINGLE_QUOTE" => &["APOS", "APOSTROPHE", "QUOT", "SQT"],
        "GRAVE" => &["GRAV"],
        "COMMA" => &["CMMA"],
        "PERIOD" => &["DOT"],
        "SLASH" => &["FSLH"],
        "PRINTSCREEN" => &["PRSC", "PSCRN"],
        "SCROLLLOCK" => &["SCLK", "SLCK"],
        "INSERT" => &["INS"],
        "DELETE" => &["DEL"],
        "PAGE_UP" => &["PG_UP", "PGUP"],
        "PAGE_DOWN" => &["PG_DN", "PGDN"],
        "RIGHT_ARROW" => &["RARW", "RIGHT"],
        "LEFT_ARROW" => &["LARW", "LEFT"],
        "DOWN_ARROW" => &["DARW", "DOWN"],
        "UP_ARROW" => &["UARW", "UP"],
        "K_CONTEXT_MENU" => &["GUI", "K_APP", "K_APPLICATION", "K_CMENU"],
        "KP_NUMLOCK" => &["KP_NLCK", "KP_NUM"],
        "KP_DIVIDE" => &["KDIV", "KP_SLASH"],
        "KP_ASTERISK" => &["KMLT", "KP_MULTIPLY"],
        "KP_SUBTRACT" => &["KMIN", "KP_MINUS"],
        "KP_LEFT_PARENTHESIS" => &["KP_LPAR"],
        "KP_RIGHT_PARENTHESIS" => &["KP_RPAR"],
        "LEFT_CONTROL" => &["LCTL", "LCTRL"],
        "LEFT_SHIFT" => &["LSFT", "LSHFT", "LSHIFT"],
        "LEFT_ALT" => &["LALT"],
        "LEFT_COMMAND" | "LCMD" | "LEFT_GUI" | "LGUI" | "LEFT_META" | "LMETA" | "LEFT_WIN"
        | "LWIN" => &[
            "LCMD",
            "LEFT_GUI",
            "LGUI",
            "LEFT_META",
            "LMETA",
            "LEFT_WIN",
            "LWIN",
        ],
        "RIGHT_CONTROL" => &["RCTL", "RCTRL"],
        "RIGHT_SHIFT" => &["RSFT", "RSHFT", "RSHIFT"],
        "RIGHT_ALT" => &["RALT"],
        "RIGHT_COMMAND" | "RCMD" | "RIGHT_GUI" | "RGUI" | "RIGHT_META" | "RMETA" | "RIGHT_WIN"
        | "RWIN" => &[
            "RCMD",
            "RGUI",
            "RIGHT_GUI",
            "RIGHT_META",
            "RMETA",
            "RIGHT_WIN",
            "RWIN",
        ],
        _ => &[],
    };

    let mut names = Vec::with_capacity(extras.len() + 1);
    names.push(canonical.to_string());
    names.extend(extras.iter().map(|name| name.to_string()));
    names.dedup();
    names
}

fn key_aliases(canonical: &str, display: &str, names: &[String]) -> Vec<String> {
    let mut aliases = vec![display.to_ascii_lowercase(), canonical.to_ascii_lowercase()];
    aliases.extend(names.iter().map(|name| name.to_ascii_lowercase()));
    let extras: &[&str] = match canonical {
        "ESC" | "ESCAPE" => &["esc"],
        "BKSP" | "BSPC" | "BACKSPACE" => &["bs", "bspc", "bksp"],
        "RET" | "RETURN" => &["enter", "ret"],
        "SPC" | "SPACE" => &["spc"],
        "DEL" | "DELETE" => &["del"],
        "LEFT_CONTROL" => &["lctl", "lctrl", "lc"],
        "RIGHT_CONTROL" => &["rctl", "rctrl", "rc"],
        "LEFT_SHIFT" => &["lsft", "lshift", "ls"],
        "RIGHT_SHIFT" => &["rsft", "rshift", "rs"],
        "LEFT_ALT" => &["lalt", "la"],
        "RIGHT_ALT" => &["ralt", "ra"],
        "LEFT_COMMAND" => &["lgui", "lcmd", "lwin", "lg", "left gui"],
        "RIGHT_COMMAND" => &["rgui", "rcmd", "rwin", "rg", "right gui"],
        "MINUS" => &["-"],
        "EQUAL" => &["="],
        "SINGLE_QUOTE" => &["'", "quote", "apostrophe"],
        "DOUBLE_QUOTES" => &["\"", "double quote"],
        "SLASH" => &["/"],
        "BACKSLASH" => &["\\"],
        "COMMA" => &[","],
        "PERIOD" => &["."],
        "SEMICOLON" => &[";"],
        _ => &[],
    };
    aliases.extend(extras.iter().map(|value| value.to_string()));
    if let Some(digit) = keypad_digit(canonical) {
        aliases.extend([
            format!("kp{}", digit),
            format!("kp {}", digit),
            format!("keypad {}", digit),
            format!("numpad {}", digit),
        ]);
    }
    if let Some(digit) = number_key_digit(canonical) {
        aliases.push(digit.to_string());
        if let Some(shifted) = us_shifted_number_symbol(digit) {
            aliases.push(shifted.to_string());
        }
    }
    aliases.sort();
    aliases.dedup();
    aliases
}

fn us_number_display(digit: char) -> String {
    us_shifted_number_symbol(digit)
        .map(|shifted| format!("{} {}", digit, shifted))
        .unwrap_or_else(|| digit.to_string())
}

fn us_shifted_number_symbol(digit: char) -> Option<char> {
    match digit {
        '1' => Some('!'),
        '2' => Some('@'),
        '3' => Some('#'),
        '4' => Some('$'),
        '5' => Some('%'),
        '6' => Some('^'),
        '7' => Some('&'),
        '8' => Some('*'),
        '9' => Some('('),
        '0' => Some(')'),
        _ => None,
    }
}

fn keypad_digit(name: &str) -> Option<char> {
    const PREFIXES: [&str; 2] = ["KP_NUMBER_", "KP_N"];
    for prefix in PREFIXES {
        if let Some(rest) = name.strip_prefix(prefix) {
            if rest.len() == 1 {
                let digit = rest.as_bytes()[0] as char;
                if digit.is_ascii_digit() {
                    return Some(digit);
                }
            }
        }
    }
    None
}
fn binding_to_view(
    position: usize,
    binding: zmk::keymap::BehaviorBinding,
    behavior_names: &BTreeMap<i32, String>,
) -> StudioBinding {
    let behavior = behavior_names
        .get(&binding.behavior_id)
        .cloned()
        .unwrap_or_else(|| format!("behavior {}", binding.behavior_id));
    let params = vec![binding.param1, binding.param2];
    let labels = binding_labels(&behavior, binding.param1, binding.param2);
    StudioBinding {
        position,
        binding_label: labels.full_label.clone(),
        primary_label: labels.primary_label,
        secondary_label: labels.secondary_label,
        full_label: labels.full_label,
        behavior,
        params,
        raw: StudioRawBinding {
            behavior_id: binding.behavior_id,
            param1: binding.param1,
            param2: binding.param2,
        },
    }
}

fn run_with_timeout<T, F>(timeout_ms: u64, f: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = f();
        let _ = tx.send(result);
    });
    rx.recv_timeout(Duration::from_millis(timeout_ms.max(1)))
        .ok()
}

fn lock_state_to_status(lock_state: LockState) -> StudioLockState {
    if lock_state.as_str_name().ends_with("UNLOCKED") {
        StudioLockState::Unlocked
    } else if lock_state.as_str_name().ends_with("LOCKED") {
        StudioLockState::Locked
    } else {
        StudioLockState::Unknown
    }
}

fn map_serial_error(_error: &SerialTransportError) -> StudioErrorCode {
    StudioErrorCode::OpenFailed
}

fn map_ble_error(error: &zmk_studio_api::transport::PlatformBleError) -> StudioErrorCode {
    tracing::debug!(error = %error, "ZMK Studio BLE transport error");
    StudioErrorCode::OpenFailed
}

fn map_client_error(error: &ClientError) -> StudioErrorCode {
    match error {
        ClientError::Io(err) if err.kind() == std::io::ErrorKind::TimedOut => {
            StudioErrorCode::RpcTimeout
        }
        ClientError::Protocol(_)
        | ClientError::MissingResponseType
        | ClientError::MissingSubsystem => StudioErrorCode::ProtocolMismatch,
        ClientError::Meta(_) => StudioErrorCode::Locked,
        _ => StudioErrorCode::RpcFailed,
    }
}

fn stable_device_id(
    port_name: &str,
    vid: Option<u16>,
    pid: Option<u16>,
    serial_number: Option<&str>,
) -> String {
    format!(
        "usb-serial:{}:{}:{}:{}",
        encode_component(port_name),
        vid.map(|value| format!("{:04x}", value))
            .unwrap_or_else(|| "none".to_string()),
        pid.map(|value| format!("{:04x}", value))
            .unwrap_or_else(|| "none".to_string()),
        encode_component(serial_number.unwrap_or("none")),
    )
}

fn encode_component(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(input: &str) -> Option<Vec<u8>> {
    if input.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(input.len() / 2);
    for chunk in input.as_bytes().chunks_exact(2) {
        let high = hex_value(chunk[0])?;
        let low = hex_value(chunk[1])?;
        out.push((high << 4) | low);
    }
    Some(out)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn serial_bytes_to_string(bytes: &[u8]) -> String {
    std::str::from_utf8(bytes)
        .map(|value| value.to_string())
        .unwrap_or_else(|_| bytes.iter().map(|byte| format!("{:02X}", byte)).collect())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_id_distinguishes_same_named_devices_by_port() {
        let a = stable_device_id("COM3", Some(0x1209), Some(0x0001), Some("same"));
        let b = stable_device_id("COM4", Some(0x1209), Some(0x0001), Some("same"));
        assert_ne!(a, b);
    }

    #[test]
    fn studio_device_ref_parses_legacy_serial_ids() {
        match StudioDeviceRef::parse("usb-serial:COM3:1209:0001:same").unwrap() {
            StudioDeviceRef::Serial(id) => assert_eq!(id, "usb-serial:COM3:1209:0001:same"),
            StudioDeviceRef::Ble(_) => panic!("expected serial device ref"),
        }
    }

    #[test]
    fn studio_device_ref_round_trips_ble_json() {
        let json = r#"{"Windows":"device-id"}"#;
        let encoded = format!("ble:{}", hex_encode(json.as_bytes()));
        match StudioDeviceRef::parse(&encoded).unwrap() {
            StudioDeviceRef::Ble(parsed) => assert_eq!(parsed, json),
            StudioDeviceRef::Serial(_) => panic!("expected BLE device ref"),
        }
    }

    #[test]
    fn snapshot_mode_keeps_usb_full_and_ble_layout_only() {
        let serial = StudioDeviceCandidate::Serial(StudioPortCandidate {
            port_name: "COM57".to_string(),
            vid: Some(0x1234),
            pid: Some(0x5678),
            serial_number: Some("serial".to_string()),
            manufacturer: None,
            product: Some("Keyboard".to_string()),
        });
        let ble = StudioDeviceCandidate::Ble(StudioBleCandidate {
            device_id_json: r#"{"Windows":"device-id"}"#.to_string(),
            local_name: Some("Keyboard".to_string()),
        });

        assert_eq!(
            snapshot_mode_for_candidate(&serial),
            StudioSnapshotMode::Full
        );
        assert_eq!(
            snapshot_mode_for_candidate(&ble),
            StudioSnapshotMode::LayoutOnly
        );
        assert_eq!(serial.connection_type(), "usb_serial");
        assert_eq!(ble.connection_type(), "ble_studio");
    }

    #[test]
    fn binding_view_preserves_raw_values() {
        let mut names = BTreeMap::new();
        names.insert(7, "key press".to_string());
        let view = binding_to_view(
            3,
            zmk::keymap::BehaviorBinding {
                behavior_id: 7,
                param1: 4,
                param2: 0,
            },
            &names,
        );
        assert_eq!(view.position, 3);
        assert_eq!(view.behavior, "key press");
        assert_eq!(view.params, vec![4, 0]);
        assert_eq!(view.raw.behavior_id, 7);
        assert_eq!(view.binding_label, "&kp A");
        assert_eq!(view.primary_label, "A");
        assert_eq!(view.secondary_label, "");
        assert_eq!(view.full_label, "&kp A");
    }

    #[test]
    fn unknown_behavior_binding_stays_raw_without_behavior_name() {
        let view = binding_to_view(
            3,
            zmk::keymap::BehaviorBinding {
                behavior_id: 7,
                param1: 0x0007_0004,
                param2: 0,
            },
            &BTreeMap::new(),
        );

        assert_eq!(view.behavior, "behavior 7");
        assert_eq!(view.binding_label, "behavior 7(458756, 0)");
        assert_eq!(view.primary_label, "behavior 7");
        assert_eq!(view.full_label, "behavior 7(458756, 0)");
        assert_eq!(view.raw.behavior_id, 7);
    }

    fn test_snapshot() -> StudioKeymapSnapshot {
        StudioKeymapSnapshot {
            device_id: "serial:test".to_string(),
            device_name: "Test Keyboard".to_string(),
            connection_type: "usb_serial".to_string(),
            lock_state: StudioLockState::Unlocked,
            physical_layouts: Vec::new(),
            selected_physical_layout_index: None,
            selected_physical_layout_name: Some("Default".to_string()),
            layout_source: StudioLayoutSource::GridFallback,
            selected_layout_keys: vec![
                StudioPhysicalKey {
                    position: 0,
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 100,
                    r: 0,
                    rx: 0,
                    ry: 0,
                },
                StudioPhysicalKey {
                    position: 1,
                    x: 100,
                    y: 0,
                    width: 100,
                    height: 100,
                    r: 0,
                    rx: 0,
                    ry: 0,
                },
            ],
            layers: vec![StudioLayer {
                index: 0,
                id: 10,
                name: "Base".to_string(),
                bindings: vec![
                    StudioBinding {
                        position: 0,
                        binding_label: "&kp A".to_string(),
                        primary_label: "A".to_string(),
                        secondary_label: String::new(),
                        full_label: "&kp A".to_string(),
                        behavior: "key press".to_string(),
                        params: vec![0x0007_0004, 0],
                        raw: StudioRawBinding {
                            behavior_id: 1,
                            param1: 0x0007_0004,
                            param2: 0,
                        },
                    },
                    StudioBinding {
                        position: 1,
                        binding_label: "&mt LSHIFT ESC".to_string(),
                        primary_label: "mt LShift Esc".to_string(),
                        secondary_label: String::new(),
                        full_label: "&mt LSHIFT ESC".to_string(),
                        behavior: "mod-tap".to_string(),
                        params: vec![0x0007_00e1, 0x0007_0029],
                        raw: StudioRawBinding {
                            behavior_id: 2,
                            param1: 0x0007_00e1,
                            param2: 0x0007_0029,
                        },
                    },
                ],
            }],
            updated_ms: 1,
        }
    }

    #[test]
    fn keymap_backup_round_trips_raw_bindings() {
        let snapshot = test_snapshot();
        let backup = keymap_backup_from_snapshot(&snapshot, BTreeMap::new(), "0.0.0-test");
        let text = serialize_keymap_backup(&backup).unwrap();
        let parsed = parse_keymap_backup(&text).unwrap();

        assert_eq!(parsed.schema, KEYMAP_BACKUP_SCHEMA);
        assert_eq!(parsed.layers[0].bindings[1].behavior_id, 2);
        assert_eq!(parsed.layers[0].bindings[1].param1, 0x0007_00e1);
        assert_eq!(parsed.layers[0].bindings[1].param2, 0x0007_0029);
    }

    #[test]
    fn keymap_restore_plans_one_changed_raw_write_and_skips_unchanged() {
        let current = test_snapshot();
        let mut backup = keymap_backup_from_snapshot(&current, BTreeMap::new(), "0.0.0-test");
        backup.layers[0].bindings[0].param1 = 0x0007_0005;
        let target_names =
            BTreeMap::from([(1, "key press".to_string()), (2, "mod_tap".to_string())]);
        let plan = plan_keymap_restore(&current, Some(&target_names), &backup);

        assert!(plan.report.can_apply);
        assert_eq!(
            plan.report.behavior_verification,
            BehaviorVerification::Done
        );
        assert_eq!(plan.report.will_write, 1);
        assert_eq!(plan.report.unchanged_skipped, 1);
        assert_eq!(plan.report.changed_keys.len(), 1);
        assert_eq!(plan.report.changed_keys[0].layer_index, 0);
        assert_eq!(plan.report.changed_keys[0].position, 0);
        assert_eq!(plan.writes[0].layer_id, 10);
        assert_eq!(plan.writes[0].behavior_id, 1);
        assert_eq!(plan.writes[0].param1, 0x0007_0005);
    }

    #[test]
    fn keymap_restore_placeholder_catalog_skips_behavior_verification_but_writes_raw() {
        let current = test_snapshot();
        let mut backup = keymap_backup_from_snapshot(&current, BTreeMap::new(), "0.0.0-test");
        backup.behavior_catalog.clear();
        backup.layers[0].bindings[0].behavior = "behavior 1".to_string();
        backup.layers[0].bindings[1].behavior = "behavior 2".to_string();
        backup.layers[0].bindings[0].param1 = 0x0007_0005;

        let plan = plan_keymap_restore(&current, None, &backup);

        assert_eq!(
            plan.report.behavior_verification,
            BehaviorVerification::Skipped
        );
        assert_eq!(plan.report.blocked, 0);
        assert_eq!(plan.report.will_write, 1);
    }

    #[test]
    fn keymap_restore_uses_common_positions_for_structure_mismatch() {
        let current = test_snapshot();
        let mut backup = keymap_backup_from_snapshot(&current, BTreeMap::new(), "0.0.0-test");
        backup.layers.push(backup.layers[0].clone());
        backup.layers[1].index = 1;
        backup.layers[0].bindings.pop();
        backup.layers[0].bindings[0].param1 = 0x0007_0005;
        backup.layers[0].bindings.push(BackupBinding {
            position: 99,
            behavior_id: 1,
            param1: 0x0007_0006,
            param2: 0,
            behavior: "key press".to_string(),
            label: "&kp B".to_string(),
        });

        let plan = plan_keymap_restore(&current, None, &backup);

        assert!(plan.report.can_apply);
        assert!(plan.report.errors.is_empty());
        assert!(plan.report.warnings.is_empty());
        assert_eq!(plan.report.will_write, 1);
        assert_eq!(plan.report.changed_keys[0].position, 0);
        assert_eq!(plan.writes[0].position, 0);
    }

    #[test]
    fn keymap_restore_ignores_target_metadata_mismatch_warnings() {
        let mut current = test_snapshot();
        current.layers.push(current.layers[0].clone());
        current.layers[1].index = 1;
        current.layers[1].id = 11;
        current.layers[1].name = "Fn".to_string();
        let mut backup = keymap_backup_from_snapshot(&current, BTreeMap::new(), "0.0.0-test");
        backup.device.name = "Other".to_string();
        backup.device.connection_type = "ble_studio".to_string();
        backup.layout.selected_physical_layout_name = Some("Other layout".to_string());
        backup.layers.swap(0, 1);
        backup.layers[0].id = 999;

        let plan = plan_keymap_restore(&current, None, &backup);

        assert!(plan.report.can_apply);
        assert!(plan.report.warnings.is_empty());
    }

    #[test]
    fn keymap_file_errors_cover_version_schema_json_and_size() {
        assert!(matches!(
            parse_keymap_backup("{"),
            Err(KeymapFileError::InvalidFile)
        ));
        assert!(matches!(
            parse_keymap_backup(&" ".repeat(KEYMAP_BACKUP_MAX_BYTES + 1)),
            Err(KeymapFileError::FileTooLarge)
        ));
        let mut backup =
            keymap_backup_from_snapshot(&test_snapshot(), BTreeMap::new(), "0.0.0-test");
        backup.schema_version = 999;
        let text = serialize_keymap_backup(&backup).unwrap();
        assert!(matches!(
            parse_keymap_backup(&text),
            Err(KeymapFileError::UnsupportedVersion)
        ));
        backup.schema_version = KEYMAP_BACKUP_SCHEMA_VERSION;
        backup.schema = "other".to_string();
        let text = serialize_keymap_backup(&backup).unwrap();
        assert!(matches!(
            parse_keymap_backup(&text),
            Err(KeymapFileError::InvalidFile)
        ));
    }

    #[test]
    fn behavior_name_normalization_treats_common_separators_as_equal() {
        assert_eq!(
            normalize_behavior_name("mod-tap"),
            normalize_behavior_name("MOD_TAP")
        );
        assert_eq!(
            normalize_behavior_name("mod tap"),
            normalize_behavior_name("mod-tap")
        );
    }

    #[test]
    fn verified_behavior_conflicts_are_blocked() {
        let current = test_snapshot();
        let mut backup = keymap_backup_from_snapshot(&current, BTreeMap::new(), "0.0.0-test");
        backup.layers[0].bindings[0].param1 = 0x0007_0005;
        let target_names =
            BTreeMap::from([(1, "sticky key".to_string()), (2, "mod-tap".to_string())]);

        let plan = plan_keymap_restore(&current, Some(&target_names), &backup);

        assert_eq!(plan.report.will_write, 0);
        assert_eq!(plan.report.blocked, 1);
        assert!(plan.report.changed_keys.is_empty());
        assert!(plan
            .report
            .warnings
            .iter()
            .any(|issue| issue.code == "behavior_conflict"));
    }

    #[test]
    fn key_label_formats_modified_hid_usage() {
        assert_eq!(key_label(0x0507_004C), "LC(LA(DEL))");
    }
    #[test]
    fn key_label_normalizes_number_keys() {
        assert_eq!(key_label(0x0007_001E), "1 !");
        assert_eq!(key_label(0x0007_0059), "Num 1");
    }

    #[test]
    fn key_label_normalizes_meta_to_gui() {
        assert_eq!(normalize_key_name("LEFT_META"), "LEFT_GUI");
        assert_eq!(normalize_key_name("RIGHT_META"), "RIGHT_GUI");
        assert_eq!(display_key_name("LEFT_META"), "Left GUI");
        assert_eq!(display_key_name("RIGHT_META"), "Right GUI");
    }

    #[test]
    fn key_catalog_uses_picker_category_order() {
        let catalog = key_catalog();
        assert!(!catalog.is_empty());
        assert!(catalog.iter().any(|entry| entry.display == "Esc"
            && entry.category == "control"
            && entry.aliases.contains(&"esc".to_string())));
        assert!(catalog
            .iter()
            .any(|entry| entry.display == "A" && entry.category == "letters"));
        assert!(catalog
            .iter()
            .any(|entry| entry.display == "1 !" && entry.category == "numbers"));
        assert!(catalog
            .iter()
            .any(|entry| entry.display == "Num 1" && entry.category == "keypad"));
        assert!(catalog
            .iter()
            .any(|entry| entry.display == "CUT" && entry.category == "editing"));
        assert!(catalog.iter().any(|entry| entry.display == "= +"
            && entry.category == "symbols"
            && entry.names.contains(&"EQL".to_string())));
        assert!(catalog.iter().any(|entry| entry.display == "Left GUI"
            && entry.category == "modifiers"
            && entry.names.contains(&"LGUI".to_string())
            && entry.names.contains(&"LEFT_META".to_string())));
        assert!(catalog
            .iter()
            .any(|entry| entry.canonical.starts_with("C_AL_") && entry.category == "applications"));
        assert_eq!(catalog[0].category, "letters");
        assert!(category_rank("letters") < category_rank("numbers"));
        assert!(category_rank("numbers") < category_rank("modifiers"));
        assert!(category_rank("modifiers") < category_rank("control"));
        assert!(category_rank("control") < category_rank("symbols"));
        assert!(category_rank("symbols") < category_rank("navigation"));
        assert!(category_rank("navigation") < category_rank("locks"));
        assert!(category_rank("locks") < category_rank("function"));
        assert!(category_rank("function") < category_rank("international"));
        assert!(category_rank("international") < category_rank("language"));
        assert!(category_rank("language") < category_rank("keypad"));
        assert!(category_rank("miscellaneous") > category_rank("power_lock"));
    }

    #[test]
    fn key_category_follows_zmk_keycode_reference_membership() {
        assert_eq!(key_category("ENTER", 0x0007_0028), "control");
        assert_eq!(key_category("EQL", 0x0007_002e), "symbols");
        assert_eq!(key_category("EXCL", 0x0207_001e), "symbols");
        assert_eq!(key_category("GUI", 0x0007_0065), "navigation");
        assert_eq!(key_category("PSCRN", 0x0007_0046), "miscellaneous");
        assert_eq!(key_category("KP_CLEAR", 0x0007_00d8), "keypad");
        assert_eq!(key_category("K_STOP3", 0x0007_00f3), "media");
        assert_eq!(key_category("K_REFRESH", 0x0007_00fa), "applications");
        assert_eq!(
            key_category("C_KEYBOARD_INPUT_ASSIST_NEXT", 0x000c_02c8),
            "input_assist"
        );
        assert_eq!(key_category("C_POWER", 0x000c_0030), "power_lock");
    }

    #[test]
    fn edit_behavior_maps_to_typed_zmk_behavior() {
        assert_eq!(
            behavior_to_zmk(EditBehavior::KeyPress(0x0007_0004)),
            Behavior::KeyPress(HidUsage::from_encoded(0x0007_0004))
        );
        assert_eq!(
            behavior_to_zmk(EditBehavior::Transparent),
            Behavior::Transparent
        );
        assert_eq!(behavior_to_zmk(EditBehavior::None), Behavior::None);
        assert_eq!(
            behavior_to_zmk(EditBehavior::MomentaryLayer(1)),
            Behavior::MomentaryLayer { layer_id: 1 }
        );
        assert_eq!(
            behavior_to_zmk(EditBehavior::ToggleLayer(2)),
            Behavior::ToggleLayer { layer_id: 2 }
        );
        assert_eq!(
            behavior_to_zmk(EditBehavior::ToLayer(0)),
            Behavior::ToLayer { layer_id: 0 }
        );
        assert_eq!(
            behavior_to_zmk(EditBehavior::ModTap {
                hold: 0x0207_00E0,
                tap: 0x0007_0029,
            }),
            Behavior::ModTap {
                hold: HidUsage::from_encoded(0x0207_00E0),
                tap: HidUsage::from_encoded(0x0007_0029),
            }
        );
        assert_eq!(
            behavior_to_zmk(EditBehavior::LayerTap {
                target_layer_index: 1,
                tap: 0x0007_002C,
            }),
            Behavior::LayerTap {
                layer_id: 1,
                tap: HidUsage::from_encoded(0x0007_002C),
            }
        );
        assert_eq!(
            behavior_to_zmk(EditBehavior::StickyKey(0x0007_00E1)),
            Behavior::StickyKey(HidUsage::from_encoded(0x0007_00E1))
        );
        assert_eq!(
            behavior_to_zmk(EditBehavior::StickyLayer(3)),
            Behavior::StickyLayer { layer_id: 3 }
        );
        assert_eq!(
            behavior_to_zmk(EditBehavior::Bluetooth {
                command: 3,
                value: 1,
            }),
            Behavior::Bluetooth {
                command: 3,
                value: 1,
            }
        );
        assert_eq!(
            behavior_to_zmk(EditBehavior::OutputSelection(2)),
            Behavior::OutputSelection { value: 2 }
        );
        assert_eq!(
            behavior_to_zmk(EditBehavior::MouseKeyPress(0x01)),
            Behavior::MouseKeyPress { value: 0x01 }
        );
        assert_eq!(
            behavior_to_zmk(EditBehavior::MouseMove(0x0000_FDA8)),
            Behavior::MouseMove { value: 0x0000_FDA8 }
        );
        assert_eq!(
            behavior_to_zmk(EditBehavior::MouseScroll(0x0000_FFF6)),
            Behavior::MouseScroll { value: 0x0000_FFF6 }
        );
        assert_eq!(behavior_to_zmk(EditBehavior::CapsWord), Behavior::CapsWord);
        assert_eq!(
            behavior_to_zmk(EditBehavior::KeyRepeat),
            Behavior::KeyRepeat
        );
        assert_eq!(behavior_to_zmk(EditBehavior::Reset), Behavior::Reset);
        assert_eq!(
            behavior_to_zmk(EditBehavior::Bootloader),
            Behavior::Bootloader
        );
        assert_eq!(
            behavior_to_zmk(EditBehavior::StudioUnlock),
            Behavior::StudioUnlock
        );
        assert_eq!(
            behavior_to_zmk(EditBehavior::GraveEscape),
            Behavior::GraveEscape
        );
    }

    #[test]
    fn layer_behaviors_use_zmk_keymap_style_labels() {
        let mut names = BTreeMap::new();
        names.insert(10, "momentary layer".to_string());
        names.insert(11, "toggle layer".to_string());
        names.insert(12, "to layer".to_string());

        let mo = binding_to_view(
            1,
            zmk::keymap::BehaviorBinding {
                behavior_id: 10,
                param1: 1,
                param2: 0,
            },
            &names,
        );
        assert_eq!(mo.primary_label, "mo 1");
        assert_eq!(mo.secondary_label, "");
        assert_eq!(mo.full_label, "&mo 1");

        let tog = binding_to_view(
            2,
            zmk::keymap::BehaviorBinding {
                behavior_id: 11,
                param1: 2,
                param2: 0,
            },
            &names,
        );
        assert_eq!(tog.primary_label, "tog 2");
        assert_eq!(tog.secondary_label, "");
        assert_eq!(tog.full_label, "&tog 2");

        let to = binding_to_view(
            3,
            zmk::keymap::BehaviorBinding {
                behavior_id: 12,
                param1: 0,
                param2: 0,
            },
            &names,
        );
        assert_eq!(to.primary_label, "to 0");
        assert_eq!(to.secondary_label, "");
        assert_eq!(to.full_label, "&to 0");
    }

    #[test]
    fn tap_hold_behaviors_use_compact_labels() {
        let mut names = BTreeMap::new();
        names.insert(13, "mod-tap".to_string());
        names.insert(14, "layer-tap".to_string());

        let mt = binding_to_view(
            1,
            zmk::keymap::BehaviorBinding {
                behavior_id: 13,
                param1: 0x0207_00E0,
                param2: 0x0007_0029,
            },
            &names,
        );
        assert_eq!(mt.primary_label, "mt LCtrl+LShift Esc");
        assert_eq!(mt.secondary_label, "");
        assert_eq!(mt.full_label, "&mt LCTRL+LSHIFT ESC");

        let lt = binding_to_view(
            2,
            zmk::keymap::BehaviorBinding {
                behavior_id: 14,
                param1: 1,
                param2: 0x0007_002C,
            },
            &names,
        );
        assert_eq!(lt.primary_label, "lt 1 Space");
        assert_eq!(lt.secondary_label, "");
        assert_eq!(lt.full_label, "&lt 1 SPACE");

        // Mod-tap whose tap side carries implicit modifier bits: hold = LSHIFT,
        // tap = LC(A). The tap modifier must survive into both labels.
        let mt_mod = binding_to_view(
            3,
            zmk::keymap::BehaviorBinding {
                behavior_id: 13,
                param1: 0x0007_00E1,
                param2: 0x0107_0004,
            },
            &names,
        );
        assert_eq!(mt_mod.primary_label, "mt LShift LC(A)");
        assert_eq!(mt_mod.full_label, "&mt LSHIFT LC(A)");

        // Layer-tap with a modified tap key: layer 1, tap = LC(A).
        let lt_mod = binding_to_view(
            4,
            zmk::keymap::BehaviorBinding {
                behavior_id: 14,
                param1: 1,
                param2: 0x0107_0004,
            },
            &names,
        );
        assert_eq!(lt_mod.primary_label, "lt 1 LC(A)");
        assert_eq!(lt_mod.full_label, "&lt 1 LC(A)");
    }

    #[test]
    fn advanced_behaviors_use_compact_labels() {
        let mut names = BTreeMap::new();
        names.insert(15, "sticky key".to_string());
        names.insert(16, "sticky layer".to_string());
        names.insert(17, "bluetooth".to_string());
        names.insert(18, "output selection".to_string());

        let sk = binding_to_view(
            1,
            zmk::keymap::BehaviorBinding {
                behavior_id: 15,
                param1: 0x0007_00E1,
                param2: 0,
            },
            &names,
        );
        assert_eq!(sk.primary_label, "sk LShift");
        assert_eq!(sk.secondary_label, "");
        assert_eq!(sk.full_label, "&sk LSHIFT");

        // Sticky key holding a modified normal key (LC(A)) must render nested,
        // not the modifier-combo form "A+LCTRL".
        let sk_mod = binding_to_view(
            5,
            zmk::keymap::BehaviorBinding {
                behavior_id: 15,
                param1: 0x0107_0004,
                param2: 0,
            },
            &names,
        );
        assert_eq!(sk_mod.primary_label, "sk LC(A)");
        assert_eq!(sk_mod.full_label, "&sk LC(A)");

        let sl = binding_to_view(
            2,
            zmk::keymap::BehaviorBinding {
                behavior_id: 16,
                param1: 1,
                param2: 0,
            },
            &names,
        );
        assert_eq!(sl.primary_label, "sl 1");
        assert_eq!(sl.secondary_label, "");
        assert_eq!(sl.full_label, "&sl 1");

        let bt = binding_to_view(
            3,
            zmk::keymap::BehaviorBinding {
                behavior_id: 17,
                param1: 3,
                param2: 1,
            },
            &names,
        );
        assert_eq!(bt.primary_label, "bt 1");
        assert_eq!(bt.secondary_label, "");
        assert_eq!(bt.full_label, "&bt BT_SEL 1");

        let out = binding_to_view(
            4,
            zmk::keymap::BehaviorBinding {
                behavior_id: 18,
                param1: 2,
                param2: 0,
            },
            &names,
        );
        assert_eq!(out.primary_label, "out BLE");
        assert_eq!(out.secondary_label, "");
        assert_eq!(out.full_label, "&out OUT_BLE");

        let out_alias = binding_to_view(
            5,
            zmk::keymap::BehaviorBinding {
                behavior_id: 19,
                param1: 1,
                param2: 0,
            },
            &BTreeMap::from([(19, "out".to_string())]),
        );
        assert_eq!(out_alias.primary_label, "out USB");
        assert_eq!(out_alias.full_label, "&out OUT_USB");
    }

    #[test]
    fn mouse_and_system_behaviors_use_readable_labels() {
        let names = BTreeMap::from([
            (20, "mouse key press".to_string()),
            (21, "mouse_move".to_string()),
            (22, "mouse_scroll".to_string()),
            (23, "caps word".to_string()),
            (24, "key repeat".to_string()),
            (25, "reset".to_string()),
            (26, "bootloader".to_string()),
            (27, "studio unlock".to_string()),
            (28, "grave/escape".to_string()),
        ]);

        let mkp = binding_to_view(
            1,
            zmk::keymap::BehaviorBinding {
                behavior_id: 20,
                param1: 0x01,
                param2: 0,
            },
            &names,
        );
        assert_eq!(mkp.primary_label, "Left Click");
        assert_eq!(mkp.full_label, "&mkp LCLK");

        let mmv = binding_to_view(
            2,
            zmk::keymap::BehaviorBinding {
                behavior_id: 21,
                param1: 0x0000_FDA8,
                param2: 0,
            },
            &names,
        );
        assert_eq!(mmv.primary_label, "Move Up");
        assert_eq!(mmv.full_label, "&mmv MOVE_UP");

        let msc = binding_to_view(
            3,
            zmk::keymap::BehaviorBinding {
                behavior_id: 22,
                param1: 0x0000_FFF6,
                param2: 0,
            },
            &names,
        );
        assert_eq!(msc.primary_label, "Scroll Down");
        assert_eq!(msc.full_label, "&msc SCRL_DOWN");

        let caps = binding_to_view(
            4,
            zmk::keymap::BehaviorBinding {
                behavior_id: 23,
                param1: 0,
                param2: 0,
            },
            &names,
        );
        assert_eq!(caps.primary_label, "caps word");
        assert_eq!(caps.full_label, "&caps_word");

        let repeat = binding_to_view(
            5,
            zmk::keymap::BehaviorBinding {
                behavior_id: 24,
                param1: 0,
                param2: 0,
            },
            &names,
        );
        assert_eq!(repeat.primary_label, "key repeat");
        assert_eq!(repeat.full_label, "&key_repeat");

        let reset = binding_to_view(
            6,
            zmk::keymap::BehaviorBinding {
                behavior_id: 25,
                param1: 0,
                param2: 0,
            },
            &names,
        );
        assert_eq!(reset.primary_label, "reset");
        assert_eq!(reset.full_label, "&reset");

        let bootloader = binding_to_view(
            7,
            zmk::keymap::BehaviorBinding {
                behavior_id: 26,
                param1: 0,
                param2: 0,
            },
            &names,
        );
        assert_eq!(bootloader.primary_label, "bootloader");
        assert_eq!(bootloader.full_label, "&bootloader");

        let studio_unlock = binding_to_view(
            8,
            zmk::keymap::BehaviorBinding {
                behavior_id: 27,
                param1: 0,
                param2: 0,
            },
            &names,
        );
        assert_eq!(studio_unlock.primary_label, "studio unlock");
        assert_eq!(studio_unlock.full_label, "&studio_unlock");

        let grave_escape = binding_to_view(
            9,
            zmk::keymap::BehaviorBinding {
                behavior_id: 28,
                param1: 0,
                param2: 0,
            },
            &names,
        );
        assert_eq!(grave_escape.primary_label, "grave escape");
        assert_eq!(grave_escape.full_label, "&gresc");
    }

    #[test]
    fn client_error_mapping_preserves_edit_specific_codes() {
        assert!(matches!(
            map_client_to_studio_error(ClientError::SetLayerBindingFailed(
                zmk::keymap::SetLayerBindingResponse::SetLayerBindingRespInvalidLocation
            )),
            StudioError::InvalidLocation
        ));
        assert!(matches!(
            map_client_to_studio_error(ClientError::MissingBehaviorRole("Key Press")),
            StudioError::MissingBehaviorRole
        ));
        assert!(matches!(
            map_client_to_studio_error(ClientError::SaveChangesFailed(
                zmk::keymap::SaveChangesErrorCode::SaveChangesErrNoSpace
            )),
            StudioError::SaveNoSpace
        ));
    }

    #[test]
    fn none_binding_has_blank_visible_label() {
        let mut names = BTreeMap::new();
        names.insert(9, "none".to_string());
        let view = binding_to_view(
            4,
            zmk::keymap::BehaviorBinding {
                behavior_id: 9,
                param1: 0,
                param2: 0,
            },
            &names,
        );
        assert_eq!(view.primary_label, "");
        assert_eq!(view.secondary_label, "");
        assert_eq!(view.full_label, "&none");
    }
    #[test]
    fn grid_fallback_uses_clamped_sqrt_columns() {
        let keys = grid_fallback_keys(30);
        assert_eq!(keys.len(), 30);
        assert_eq!(keys[0].x, 0);
        assert_eq!(keys[0].y, 0);
        assert_eq!(keys[6].x, 660);
        assert_eq!(keys[6].y, 0);
        assert_eq!(keys[7].x, 0);
        assert_eq!(keys[7].y, 110);
    }

    #[test]
    fn selects_active_physical_layout() {
        let keymap = zmk::keymap::Keymap {
            layers: vec![zmk::keymap::Layer {
                id: 1,
                name: "Layer 0".to_string(),
                bindings: vec![],
            }],
            available_layers: 1,
            max_layer_name_length: 12,
        };
        let layouts = zmk::keymap::PhysicalLayouts {
            active_layout_index: 1,
            layouts: vec![
                zmk::keymap::PhysicalLayout {
                    name: "A".to_string(),
                    keys: vec![zmk::keymap::KeyPhysicalAttrs {
                        width: 100,
                        height: 100,
                        x: 0,
                        y: 0,
                        r: 0,
                        rx: 0,
                        ry: 0,
                    }],
                },
                zmk::keymap::PhysicalLayout {
                    name: "B".to_string(),
                    keys: vec![zmk::keymap::KeyPhysicalAttrs {
                        width: 150,
                        height: 100,
                        x: 10,
                        y: 20,
                        r: 15,
                        rx: 10,
                        ry: 20,
                    }],
                },
            ],
        };

        let selection = select_layout(Some(layouts), &keymap);
        assert_eq!(
            selection.layout_source,
            StudioLayoutSource::StudioPhysicalLayout
        );
        assert_eq!(selection.selected_physical_layout_index, Some(1));
        assert_eq!(
            selection.selected_physical_layout_name.as_deref(),
            Some("B")
        );
        assert_eq!(selection.selected_layout_keys[0].position, 0);
        assert_eq!(selection.selected_layout_keys[0].width, 150);
        assert_eq!(selection.selected_layout_keys[0].r, 15);
    }

    #[test]
    fn invalid_or_empty_physical_layout_falls_back_to_grid() {
        let keymap = zmk::keymap::Keymap {
            layers: vec![zmk::keymap::Layer {
                id: 1,
                name: "Layer 0".to_string(),
                bindings: vec![
                    zmk::keymap::BehaviorBinding {
                        behavior_id: 1,
                        param1: 1,
                        param2: 0,
                    },
                    zmk::keymap::BehaviorBinding {
                        behavior_id: 1,
                        param1: 2,
                        param2: 0,
                    },
                ],
            }],
            available_layers: 1,
            max_layer_name_length: 12,
        };
        let layouts = zmk::keymap::PhysicalLayouts {
            active_layout_index: 9,
            layouts: vec![zmk::keymap::PhysicalLayout {
                name: "Empty".to_string(),
                keys: vec![],
            }],
        };

        let selection = select_layout(Some(layouts), &keymap);
        assert_eq!(selection.layout_source, StudioLayoutSource::GridFallback);
        assert_eq!(selection.selected_physical_layout_index, None);
        assert_eq!(selection.selected_physical_layout_name, None);
        assert_eq!(selection.selected_layout_keys.len(), 2);
    }
}
