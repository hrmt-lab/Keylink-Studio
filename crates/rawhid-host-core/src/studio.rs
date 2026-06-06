use std::{
    collections::{BTreeMap, BTreeSet},
    sync::mpsc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use thiserror::Error;
use zmk_studio_api::{
    proto::zmk::{self, core::LockState},
    transport::serial::SerialTransportError,
    ClientError, HidUsage, StudioClient,
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StudioRawBinding {
    pub behavior_id: i32,
    pub param1: u32,
    pub param2: u32,
}

#[derive(Debug, Error)]
pub enum StudioError {
    #[error("device not found")]
    DeviceNotFound,
    #[error("studio device is locked")]
    Locked,
    #[error("RPC timeout")]
    Timeout,
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

pub fn probe_usb_serial_devices(config: &StudioConfig) -> Vec<StudioDeviceStatus> {
    let candidates = match serial_candidates() {
        Ok(candidates) => candidates,
        Err(_) => Vec::new(),
    };

    candidates
        .into_iter()
        .map(|candidate| probe_candidate(candidate, config.probe_timeout_ms))
        .collect()
}

pub fn read_keymap_for_device(
    device_id: &str,
    config: &StudioConfig,
) -> Result<StudioKeymapSnapshot, StudioError> {
    let candidates = serial_candidates().map_err(|_| StudioError::DeviceNotFound)?;
    let Some(candidate) = candidates
        .into_iter()
        .find(|candidate| candidate.id() == device_id)
    else {
        return Err(StudioError::DeviceNotFound);
    };

    run_with_timeout(config.keymap_read_timeout_ms, move || {
        read_keymap(candidate)
    })
    .unwrap_or(Err(StudioError::Timeout))
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
    let mut client = match StudioClient::open_serial(&candidate.port_name) {
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

fn read_keymap(candidate: StudioPortCandidate) -> Result<StudioKeymapSnapshot, StudioError> {
    let mut client =
        StudioClient::open_serial(&candidate.port_name).map_err(|_| StudioError::RpcFailed)?;
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

    let keymap = client.get_keymap().map_err(|error| match error {
        ClientError::Meta(_) => StudioError::Locked,
        _ => StudioError::RpcFailed,
    })?;
    let layout_selection = select_layout(client.get_physical_layouts().ok(), &keymap);
    let behavior_names = behavior_names_for_keymap(&mut client, &keymap);

    Ok(StudioKeymapSnapshot {
        device_id: candidate.id(),
        device_name: if info.name.is_empty() {
            candidate.display_name()
        } else {
            info.name
        },
        connection_type: "usb_serial".to_string(),
        lock_state,
        physical_layouts: layout_selection.physical_layouts,
        selected_physical_layout_index: layout_selection.selected_physical_layout_index,
        selected_physical_layout_name: layout_selection.selected_physical_layout_name,
        layout_source: layout_selection.layout_source,
        selected_layout_keys: layout_selection.selected_layout_keys,
        layers: keymap_to_layers(keymap, &behavior_names),
        updated_ms: now_ms(),
    })
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
    client: &mut StudioClient<zmk_studio_api::transport::serial::SerialTransport>,
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
        "sticky key" => Some((
            "&sk".to_string(),
            key_label(param1),
            format!("&sk {}", key_label(param1)),
        )),
        "layer-tap" => Some((
            format!("&lt {}", param1),
            key_label(param2),
            format!("&lt {} {}", param1, key_label(param2)),
        )),
        "mod-tap" => Some((
            format!("&mt {}", key_label(param1)),
            key_label(param2),
            format!("&mt {} {}", key_label(param1), key_label(param2)),
        )),
        "momentary layer" => Some((
            "&mo".to_string(),
            param1.to_string(),
            format!("&mo {}", param1),
        )),
        "toggle layer" => Some((
            "&tog".to_string(),
            param1.to_string(),
            format!("&tog {}", param1),
        )),
        "to layer" => Some((
            "&to".to_string(),
            param1.to_string(),
            format!("&to {}", param1),
        )),
        "sticky layer" => Some((
            "&sl".to_string(),
            param1.to_string(),
            format!("&sl {}", param1),
        )),
        "transparent" => Some(("&trans".to_string(), String::new(), "&trans".to_string())),
        "none" => Some((String::new(), String::new(), "&none".to_string())),
        "caps word" => Some((
            "&caps_word".to_string(),
            String::new(),
            "&caps_word".to_string(),
        )),
        "key repeat" => Some((
            "&key_repeat".to_string(),
            String::new(),
            "&key_repeat".to_string(),
        )),
        "studio unlock" => Some((
            "&studio_unlock".to_string(),
            String::new(),
            "&studio_unlock".to_string(),
        )),
        "bootloader" => Some((
            "&bootloader".to_string(),
            String::new(),
            "&bootloader".to_string(),
        )),
        "reset" => Some(("&reset".to_string(), String::new(), "&reset".to_string())),
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

fn normalize_key_name(name: &str) -> String {
    if let Some(digit) = number_key_digit(name) {
        return digit.to_string();
    }
    name.to_string()
}

fn number_key_digit(name: &str) -> Option<char> {
    const PREFIXES: [&str; 5] = ["NUMBER_", "NUM_", "N", "KP_NUMBER_", "KP_N"];
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
    fn key_label_formats_modified_hid_usage() {
        assert_eq!(key_label(0x0507_004C), "LC(LA(DEL))");
    }
    #[test]
    fn key_label_normalizes_number_keys() {
        assert_eq!(key_label(0x0007_001E), "1");
        assert_eq!(key_label(0x0007_0059), "1");
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
