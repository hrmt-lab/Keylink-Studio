use std::{
    collections::HashMap,
    thread,
    time::{Duration, Instant},
};

use tracing::{debug, info, warn};

use crate::{
    active_app::{ActiveAppError, ActiveAppProvider},
    ai_usage::{
        AiUsageProviderStatus, AiUsageRefreshError, AiUsageRuntime, AiUsageSendState, AiUsageShared,
    },
    app_match::{match_action, LayerAction},
    config::{AppConfig, UnmatchedAction},
    hid::{DeviceInfo, HidDeviceManager, HidError, HidTransport},
    packet::UplinkPacket,
    stats::SharedKeyStatsStore,
    time::{Clock, SystemClock, TimeError, TimeSnapshot, TimeSyncState},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEvent {
    SetLayer { layer: u8, rule_name: String },
    Clear,
    Unchanged,
}

/// A device-initiated packet drained from a verified device.
#[derive(Debug, Clone, PartialEq)]
pub struct UplinkEvent {
    pub device: DeviceInfo,
    pub packet: UplinkPacket,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DeviceBatterySource {
    /// 0 = central/self, 1..=3 = peripheral 1..=3.
    pub source: u8,
    /// 0..=100; None = unknown / disconnected.
    pub level: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DeviceBatteryStatus {
    pub device_key: String,
    pub serial_number: Option<String>,
    pub product: Option<String>,
    pub sources: Vec<DeviceBatterySource>,
    pub updated_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DeviceLayerState {
    pub device_key: String,
    pub serial_number: Option<String>,
    pub product: Option<String>,
    pub active_layer: u8,
    pub layer_mask: u32,
}

#[derive(Debug)]
pub struct Runner<P, T, C = SystemClock> {
    config: AppConfig,
    app_provider: P,
    hid: HidDeviceManager<T>,
    clock: C,
    time_sync: TimeSyncState,
    ai_usage_shared: Option<AiUsageShared>,
    owned_ai_usage_runtime: Option<AiUsageRuntime>,
    ai_usage_send: AiUsageSendState,
    managed_layers: HashMap<String, ManagedLayerState>,
    last_device_generation: u64,
    pending_uplink_events: Vec<UplinkEvent>,
    battery: HashMap<String, DeviceBatteryStatus>,
    layer_states: HashMap<String, DeviceLayerState>,
    last_host_action_seq: HashMap<String, u8>,
    last_key_stats_seq: HashMap<String, u8>,
    key_stats: Option<SharedKeyStatsStore>,
}

impl<P, T> Runner<P, T, SystemClock>
where
    P: ActiveAppProvider,
    T: HidTransport,
{
    pub fn new(config: AppConfig, app_provider: P, hid: HidDeviceManager<T>) -> Self {
        Self::new_with_clock(config, app_provider, hid, SystemClock)
    }
}

impl<P, T, C> Runner<P, T, C>
where
    P: ActiveAppProvider,
    T: HidTransport,
    C: Clock,
{
    pub fn new_with_clock(
        config: AppConfig,
        app_provider: P,
        hid: HidDeviceManager<T>,
        clock: C,
    ) -> Self {
        let owned_ai_usage_runtime = AiUsageRuntime::start(config.ai_usage.clone());
        let ai_usage_shared = owned_ai_usage_runtime.as_ref().map(AiUsageRuntime::shared);
        Self {
            config,
            app_provider,
            hid,
            clock,
            time_sync: TimeSyncState::default(),
            ai_usage_shared,
            owned_ai_usage_runtime,
            ai_usage_send: AiUsageSendState::default(),
            managed_layers: HashMap::new(),
            last_device_generation: 0,
            pending_uplink_events: Vec::new(),
            battery: HashMap::new(),
            layer_states: HashMap::new(),
            last_host_action_seq: HashMap::new(),
            last_key_stats_seq: HashMap::new(),
            key_stats: None,
        }
    }

    pub fn new_with_ai_usage_shared(
        config: AppConfig,
        app_provider: P,
        hid: HidDeviceManager<T>,
        clock: C,
        ai_usage_shared: Option<AiUsageShared>,
    ) -> Self {
        Self {
            config,
            app_provider,
            hid,
            clock,
            time_sync: TimeSyncState::default(),
            ai_usage_shared,
            owned_ai_usage_runtime: None,
            ai_usage_send: AiUsageSendState::default(),
            managed_layers: HashMap::new(),
            last_device_generation: 0,
            pending_uplink_events: Vec::new(),
            battery: HashMap::new(),
            layer_states: HashMap::new(),
            last_host_action_seq: HashMap::new(),
            last_key_stats_seq: HashMap::new(),
            key_stats: None,
        }
    }

    pub fn tick(&mut self) -> Result<RunEvent, RunnerError> {
        self.hid.ensure_verified()?;
        self.process_uplink();
        let device_generation = self.hid.device_generation();
        self.sync_time_if_due(device_generation)?;
        self.sync_ai_usage_if_due(device_generation)?;
        let app = match self.app_provider.active_app() {
            Ok(app) => app,
            Err(ActiveAppError::NoForegroundWindow) => return Ok(RunEvent::Unchanged),
            Err(error) => return Err(error.into()),
        };
        let event = self.sync_app_layers(&app, device_generation)?;
        self.last_device_generation = self.hid.device_generation();
        Ok(event)
    }

    /// Attach the shared key-stats store fed by KEY_STATS uplink packets.
    pub fn set_key_stats_store(&mut self, store: SharedKeyStatsStore) {
        self.key_stats = Some(store);
    }

    /// Take all uplink events drained since the previous call.
    pub fn take_uplink_events(&mut self) -> Vec<UplinkEvent> {
        std::mem::take(&mut self.pending_uplink_events)
    }

    /// Drain HID packets and ingest them into pending_uplink_events.
    /// Does not run flush_if_due or prune_uplink_state; those run in tick().
    pub fn drain_uplink_only(&mut self) {
        let drained = self.hid.drain_uplink();
        if !drained.is_empty() {
            let snapshot = self.clock.now().ok();
            for (device, packet) in drained {
                self.ingest_uplink(device, packet, &snapshot);
            }
        }
    }

    pub fn battery_statuses(&self) -> Vec<DeviceBatteryStatus> {
        let mut statuses: Vec<DeviceBatteryStatus> = self.battery.values().cloned().collect();
        statuses.sort_by(|a, b| a.device_key.cmp(&b.device_key));
        statuses
    }

    pub fn layer_states(&self) -> Vec<DeviceLayerState> {
        let mut states: Vec<DeviceLayerState> = self.layer_states.values().cloned().collect();
        states.sort_by(|a, b| a.device_key.cmp(&b.device_key));
        states
    }

    fn process_uplink(&mut self) {
        self.drain_uplink_only();
        if let Some(store) = &self.key_stats {
            if let Ok(mut store) = store.lock() {
                store.flush_if_due(Instant::now());
            }
        }
        self.prune_uplink_state();
    }

    /// Feed a single uplink packet through the normal handling path. Public
    /// for debug injection; production packets arrive via `drain_uplink`.
    pub fn inject_uplink(&mut self, device: DeviceInfo, packet: UplinkPacket) {
        let snapshot = self.clock.now().ok();
        self.ingest_uplink(device, packet, &snapshot);
    }

    fn ingest_uplink(
        &mut self,
        device: DeviceInfo,
        packet: UplinkPacket,
        snapshot: &Option<TimeSnapshot>,
    ) {
        {
            {
                let missing_capability = device.capabilities & packet.required_capability() == 0;
                if missing_capability && !matches!(packet, UplinkPacket::Battery(_)) {
                    debug!(
                        "dropping uplink {:?} from {}: capability not advertised",
                        packet.packet_type(),
                        device.path
                    );
                    return;
                }
                if missing_capability {
                    debug!(
                        "accepting battery uplink from {} even though BATTERY capability is not advertised",
                        device.path
                    );
                }
                let device_key = uplink_device_key(&device);
                match &packet {
                    UplinkPacket::Battery(battery) => {
                        self.battery.insert(
                            device_key,
                            DeviceBatteryStatus {
                                device_key: uplink_device_key(&device),
                                serial_number: device.serial_number.clone(),
                                product: device.product.clone(),
                                sources: battery
                                    .entries
                                    .iter()
                                    .map(|entry| DeviceBatterySource {
                                        source: entry.source,
                                        level: entry.level,
                                    })
                                    .collect(),
                                updated_unix: snapshot
                                    .as_ref()
                                    .map(|s| s.unix_time_sec)
                                    .unwrap_or(0),
                            },
                        );
                    }
                    UplinkPacket::LayerState(state) => {
                        // Display-only: this must never feed back into
                        // managed_layers / APP_LAYER sending.
                        self.layer_states.insert(
                            device_key,
                            DeviceLayerState {
                                device_key: uplink_device_key(&device),
                                serial_number: device.serial_number.clone(),
                                product: device.product.clone(),
                                active_layer: state.active_layer,
                                layer_mask: state.layer_mask,
                            },
                        );
                    }
                    UplinkPacket::HostAction(action) => {
                        // Debounce a firmware double-send of the same seq.
                        if self.last_host_action_seq.get(&device_key) == Some(&action.seq) {
                            return;
                        }
                        self.last_host_action_seq.insert(device_key, action.seq);
                    }
                    UplinkPacket::KeyPress(_) => {
                        // Real-time event; no state to maintain on the host side.
                    }
                    UplinkPacket::KeyStats(stats) => {
                        if let Some(previous) = self.last_key_stats_seq.get(&device_key) {
                            let expected = previous.wrapping_add(1);
                            if stats.seq != expected {
                                warn!(
                                    "key stats seq gap for {device_key}: expected {expected}, got {}",
                                    stats.seq
                                );
                            }
                        }
                        self.last_key_stats_seq
                            .insert(device_key.clone(), stats.seq);
                        if self.config.stats.enabled {
                            if let (Some(store), Some(snapshot)) = (&self.key_stats, &snapshot) {
                                if let Ok(mut store) = store.lock() {
                                    store.apply_diff(
                                        &device_key,
                                        &stats.entries,
                                        &local_date(snapshot),
                                    );
                                }
                            }
                        }
                    }
                }
                self.pending_uplink_events
                    .push(UplinkEvent { device, packet });
            }
        }
    }

    /// Drop battery / layer entries for devices that are no longer verified.
    fn prune_uplink_state(&mut self) {
        let keys: Vec<String> = self
            .hid
            .verified_devices()
            .iter()
            .map(uplink_device_key)
            .collect();
        self.battery.retain(|key, _| keys.contains(key));
        self.layer_states.retain(|key, _| keys.contains(key));
    }

    pub fn verified_device_count(&self) -> usize {
        self.hid.verified_devices().len()
    }

    pub fn verified_device_names(&self) -> Vec<String> {
        self.hid
            .verified_devices()
            .iter()
            .map(|d| {
                d.product
                    .clone()
                    .or_else(|| d.manufacturer.clone())
                    .unwrap_or_else(|| format!("{:04X}:{:04X}", d.vendor_id, d.product_id))
            })
            .collect()
    }

    pub fn verified_devices(&self) -> Vec<DeviceInfo> {
        self.hid.verified_devices().to_vec()
    }

    pub fn ai_usage_statuses(&self) -> Vec<AiUsageProviderStatus> {
        self.ai_usage_shared
            .as_ref()
            .map(|shared| shared.statuses(self.config.ai_usage.stale_after_sec))
            .unwrap_or_default()
    }

    pub fn refresh_ai_usage(&self) -> Result<(), AiUsageRefreshError> {
        self.owned_ai_usage_runtime
            .as_ref()
            .ok_or(AiUsageRefreshError::Stopped)
            .and_then(AiUsageRuntime::refresh)
    }

    pub fn run_forever(&mut self) -> ! {
        let interval = Duration::from_millis(self.config.polling.interval_ms.max(1));
        let uplink_interval = Duration::from_millis(self.config.polling.uplink_interval_ms.max(5));
        loop {
            if let Err(error) = self.tick() {
                warn!("run tick failed: {}", error);
            }
            // CLI surface for device-initiated packets: log only.
            for event in self.take_uplink_events() {
                info!("uplink from {}: {:?}", event.device.path, event.packet);
            }
            // Drain uplink in uplink_interval slices while waiting for next tick.
            let deadline = Instant::now() + interval;
            while Instant::now() < deadline {
                let remaining = deadline.saturating_duration_since(Instant::now());
                thread::sleep(uplink_interval.min(remaining));
                self.drain_uplink_only();
                for event in self.take_uplink_events() {
                    info!("uplink from {}: {:?}", event.device.path, event.packet);
                }
            }
        }
    }

    fn sync_app_layers(
        &mut self,
        app: &crate::ActiveApp,
        device_generation: u64,
    ) -> Result<RunEvent, RunnerError> {
        let devices = self.hid.verified_devices().to_vec();
        let mut changed = false;
        let mut first_set: Option<(u8, String)> = None;
        let resend_due_to_generation = self.last_device_generation != device_generation;

        for device in devices {
            if !supports_app_layer(&device) {
                continue;
            }
            let Some(uid) = device.device_uid_hash else {
                continue;
            };
            let device_key = device_uid_key(uid);
            let layer_switch = &self.config.layer_switch;
            let device_config = layer_switch.devices.get(&device_key);
            // Device-specific unmatched_action overrides the global default when set.
            let unmatched_action = device_config
                .and_then(|cfg| cfg.unmatched_action)
                .unwrap_or(layer_switch.unmatched_action);

            let (action, rule_name) = if !layer_switch.enabled {
                (LayerAction::Clear, None)
            } else if let Some(device_config) = device_config {
                if device_config.enabled {
                    let (action, matched) = match_action(app, &device_config.rules);
                    (action, matched.map(|m| m.rule.name.clone()))
                } else {
                    (LayerAction::Clear, None)
                }
            } else {
                // No device-specific config: this device is not layer-managed.
                // Clear only removes a leftover managed layer if the device
                // config was deleted while monitoring was running.
                (LayerAction::Clear, None)
            };

            let state = self.managed_layers.entry(device_key.clone()).or_default();

            match action {
                LayerAction::Set(layer) => {
                    if state.active_layer != Some(layer) || resend_due_to_generation {
                        self.hid.send_set_layer_to_device(&device, layer)?;
                        state.active_layer = Some(layer);
                        let rule_name = rule_name.unwrap_or_else(|| "<unknown>".to_string());
                        info!(
                            "set layer {} by rule {} for {}",
                            layer, rule_name, device_key
                        );
                        if first_set.is_none() {
                            first_set = Some((layer, rule_name));
                        }
                        changed = true;
                    }
                }
                LayerAction::Clear => {
                    if state.active_layer.is_some()
                        && unmatched_action == UnmatchedAction::ClearManaged
                    {
                        self.hid.send_clear_to_device(&device)?;
                        state.active_layer = None;
                        debug!("clear layer for {}", device_key);
                        changed = true;
                    }
                }
            }
        }

        if let Some((layer, rule_name)) = first_set {
            Ok(RunEvent::SetLayer { layer, rule_name })
        } else if changed {
            Ok(RunEvent::Clear)
        } else {
            Ok(RunEvent::Unchanged)
        }
    }
    fn sync_time_if_due(&mut self, device_generation: u64) -> Result<(), RunnerError> {
        if !self.config.time.enabled {
            return Ok(());
        }
        let snapshot = self.clock.now()?;
        if let Some(packet) =
            self.time_sync
                .build_due_packet(&self.config.time, snapshot, device_generation)?
        {
            let sent = self.hid.send_time_sync(packet)?;
            debug!("time sync sent for {} devices", sent);
        }
        Ok(())
    }

    fn sync_ai_usage_if_due(&mut self, device_generation: u64) -> Result<(), RunnerError> {
        if !self.config.ai_usage.enabled {
            return Ok(());
        }
        let Some(shared) = &self.ai_usage_shared else {
            return Ok(());
        };
        for packet in self.ai_usage_send.due_packets(
            shared,
            self.config.ai_usage.stale_after_sec,
            device_generation,
        ) {
            let sent = self.hid.send_ai_usage(packet)?;
            debug!("AI usage sent for {} devices", sent);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ManagedLayerState {
    active_layer: Option<u8>,
}

fn supports_app_layer(device: &DeviceInfo) -> bool {
    device.capabilities & crate::packet::CAPABILITY_APP_LAYER != 0
}

fn device_uid_key(uid: u64) -> String {
    format!("uid:{uid:016x}")
}

/// Stable key for uplink state maps: the device uid when available,
/// otherwise the HID path.
pub fn uplink_device_key(device: &DeviceInfo) -> String {
    match device.device_uid_hash {
        Some(uid) => device_uid_key(uid),
        None => format!("path:{}", device.path),
    }
}

/// Local calendar date ("YYYY-MM-DD") for a clock snapshot.
fn local_date(snapshot: &TimeSnapshot) -> String {
    let local_secs = snapshot.unix_time_sec as i64 + i64::from(snapshot.tz_offset_min) * 60;
    chrono::DateTime::from_timestamp(local_secs, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "1970-01-01".to_string())
}
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("active app error: {0}")]
    ActiveApp(#[from] ActiveAppError),
    #[error("HID error: {0}")]
    Hid(#[from] HidError),
    #[error("time error: {0}")]
    Time(#[from] TimeError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{hid::DeviceInfo, HidConfig, HidTransport, Packet, RuleConfig};
    use std::{cell::RefCell, path::PathBuf, rc::Rc};

    #[derive(Debug)]
    struct MockAppProvider {
        app: crate::ActiveApp,
    }

    impl ActiveAppProvider for MockAppProvider {
        fn active_app(&self) -> Result<crate::ActiveApp, ActiveAppError> {
            Ok(self.app.clone())
        }
    }

    #[derive(Debug)]
    struct NoForegroundProvider;

    impl ActiveAppProvider for NoForegroundProvider {
        fn active_app(&self) -> Result<crate::ActiveApp, ActiveAppError> {
            Err(ActiveAppError::NoForegroundWindow)
        }
    }

    #[derive(Debug, Default)]
    struct MockTransport {
        writes: RefCell<Vec<[u8; crate::REPORT_SIZE]>>,
    }

    impl HidTransport for MockTransport {
        fn candidates(&self, _usage_page: u16, _usage: u16) -> Result<Vec<DeviceInfo>, HidError> {
            Ok(vec![device("keyboard")])
        }

        fn hello(
            &self,
            _device: &DeviceInfo,
            packet: Packet,
            _timeout_ms: i32,
        ) -> Result<Option<crate::packet::DeviceHello>, HidError> {
            Ok(Some(crate::packet::DeviceHello {
                protocol_min: 0,
                protocol_max: 0,
                seq: packet.seq,
                capabilities: crate::packet::CAPABILITY_APP_LAYER,
                device_uid_hash: Some(1),
            }))
        }

        fn write_report(
            &self,
            _device: &DeviceInfo,
            report: &[u8; crate::REPORT_SIZE],
        ) -> Result<(), HidError> {
            self.writes.borrow_mut().push(*report);
            Ok(())
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct FakeClock {
        snapshot: crate::TimeSnapshot,
    }

    impl Clock for FakeClock {
        fn now(&self) -> Result<crate::TimeSnapshot, TimeError> {
            Ok(self.snapshot)
        }
    }

    fn device(path: &str) -> DeviceInfo {
        DeviceInfo {
            path: path.to_string(),
            vendor_id: 1,
            product_id: 2,
            usage_page: 0xFF60,
            usage: 0x61,
            connection_type: crate::hid::DeviceConnectionType::Usb,
            manufacturer: None,
            product: None,
            serial_number: None,
            capabilities: crate::packet::CAPABILITY_APP_LAYER,
            device_uid_hash: Some(1),
        }
    }

    fn single_device_config(rules: Vec<RuleConfig>) -> crate::config::LayerSwitchConfig {
        let mut devices = std::collections::BTreeMap::new();
        devices.insert(
            // Matches the uid (1) used by `device()`.
            "uid:0000000000000001".to_string(),
            crate::config::DeviceLayerSwitchConfig {
                rules,
                ..Default::default()
            },
        );
        crate::config::LayerSwitchConfig {
            enabled: true,
            unmatched_action: crate::config::UnmatchedAction::ClearManaged,
            devices,
        }
    }

    #[test]
    fn unchanged_state_does_not_send_again() {
        let config = AppConfig {
            layer_switch: single_device_config(vec![RuleConfig {
                name: "notepad".to_string(),
                layer: 1,
                path: None,
                exe: Some("notepad.exe".to_string()),
                title: None,
            }]),
            ..AppConfig::default()
        };
        let provider = MockAppProvider {
            app: crate::ActiveApp {
                process_path: Some(PathBuf::from("C:\\Windows\\notepad.exe")),
                exe: Some("notepad.exe".to_string()),
                title: None,
            },
        };
        let transport = MockTransport::default();
        let hid = HidDeviceManager::new(HidConfig::default(), transport);
        let mut runner = Runner::new(config, provider, hid);

        assert!(matches!(runner.tick().unwrap(), RunEvent::SetLayer { .. }));
        assert_eq!(runner.tick().unwrap(), RunEvent::Unchanged);
    }

    #[test]
    fn no_foreground_window_does_not_clear() {
        let config = AppConfig {
            layer_switch: single_device_config(vec![RuleConfig {
                name: "notepad".to_string(),
                layer: 1,
                path: None,
                exe: Some("notepad.exe".to_string()),
                title: None,
            }]),
            ..AppConfig::default()
        };
        let transport = MockTransport::default();
        let hid = HidDeviceManager::new(HidConfig::default(), transport);
        let mut runner = Runner::new(config, NoForegroundProvider, hid);

        assert_eq!(runner.tick().unwrap(), RunEvent::Unchanged);
    }

    #[derive(Debug, Default)]
    struct MultiMockTransport {
        devices: Vec<DeviceInfo>,
        writes: Rc<RefCell<Vec<(String, [u8; crate::REPORT_SIZE])>>>,
        uplink: Rc<RefCell<HashMap<String, std::collections::VecDeque<[u8; crate::PACKET_SIZE]>>>>,
    }

    impl HidTransport for MultiMockTransport {
        fn candidates(&self, _usage_page: u16, _usage: u16) -> Result<Vec<DeviceInfo>, HidError> {
            Ok(self.devices.clone())
        }

        fn hello(
            &self,
            device: &DeviceInfo,
            packet: Packet,
            _timeout_ms: i32,
        ) -> Result<Option<crate::packet::DeviceHello>, HidError> {
            Ok(Some(crate::packet::DeviceHello {
                protocol_min: 0,
                protocol_max: 0,
                seq: packet.seq,
                capabilities: device.capabilities,
                device_uid_hash: device.device_uid_hash,
            }))
        }

        fn write_report(
            &self,
            device: &DeviceInfo,
            report: &[u8; crate::REPORT_SIZE],
        ) -> Result<(), HidError> {
            self.writes
                .borrow_mut()
                .push((device.path.clone(), *report));
            Ok(())
        }

        fn read_packet(
            &self,
            device: &DeviceInfo,
            _timeout_ms: i32,
        ) -> Result<Option<[u8; crate::PACKET_SIZE]>, HidError> {
            Ok(self
                .uplink
                .borrow_mut()
                .get_mut(&device.path)
                .and_then(std::collections::VecDeque::pop_front))
        }
    }

    fn device_with_uid(path: &str, uid: u64, capabilities: u32) -> DeviceInfo {
        DeviceInfo {
            path: path.to_string(),
            vendor_id: 1,
            product_id: 2,
            usage_page: 0xFF60,
            usage: 0x61,
            connection_type: crate::hid::DeviceConnectionType::Usb,
            manufacturer: None,
            product: None,
            serial_number: None,
            capabilities,
            device_uid_hash: Some(uid),
        }
    }

    fn code_app_provider() -> MockAppProvider {
        MockAppProvider {
            app: crate::ActiveApp {
                process_path: Some(PathBuf::from("C:\\Apps\\Code.exe")),
                exe: Some("Code.exe".to_string()),
                title: None,
            },
        }
    }

    fn code_rule(name: &str, layer: u8) -> RuleConfig {
        RuleConfig {
            name: name.to_string(),
            layer,
            path: None,
            exe: Some("Code.exe".to_string()),
            title: None,
        }
    }

    #[test]
    fn device_config_rules_apply_per_uid() {
        let mut devices = std::collections::BTreeMap::new();
        devices.insert(
            "uid:00000000000000aa".to_string(),
            crate::config::DeviceLayerSwitchConfig {
                rules: vec![code_rule("device-a", 3)],
                ..Default::default()
            },
        );
        // Device B has a config entry but no rules: nothing is sent to it.
        devices.insert(
            "uid:00000000000000bb".to_string(),
            crate::config::DeviceLayerSwitchConfig::default(),
        );
        let config = AppConfig {
            layer_switch: crate::config::LayerSwitchConfig {
                enabled: true,
                unmatched_action: crate::config::UnmatchedAction::ClearManaged,
                devices,
            },
            ..AppConfig::default()
        };
        let transport = MultiMockTransport {
            devices: vec![
                device_with_uid("a", 0xaa, crate::packet::CAPABILITY_APP_LAYER),
                device_with_uid("b", 0xbb, crate::packet::CAPABILITY_APP_LAYER),
            ],
            writes: Rc::new(RefCell::new(Vec::new())),
            uplink: Rc::default(),
        };
        let writes = transport.writes.clone();
        let hid = HidDeviceManager::new(HidConfig::default(), transport);
        let mut runner = Runner::new(config, code_app_provider(), hid);

        assert!(matches!(
            runner.tick().unwrap(),
            RunEvent::SetLayer { layer: 3, .. }
        ));
        let writes = writes.borrow();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].0, "a");
        assert_eq!(writes[0].1[6], 3);
    }

    #[test]
    fn missing_device_config_does_not_switch() {
        let config = AppConfig {
            layer_switch: crate::config::LayerSwitchConfig {
                enabled: true,
                unmatched_action: crate::config::UnmatchedAction::ClearManaged,
                devices: std::collections::BTreeMap::new(),
            },
            ..AppConfig::default()
        };
        let transport = MultiMockTransport {
            devices: vec![device_with_uid(
                "a",
                0xaa,
                crate::packet::CAPABILITY_APP_LAYER,
            )],
            writes: Rc::new(RefCell::new(Vec::new())),
            uplink: Rc::default(),
        };
        let writes = transport.writes.clone();
        let hid = HidDeviceManager::new(HidConfig::default(), transport);
        let mut runner = Runner::new(config, code_app_provider(), hid);

        assert_eq!(runner.tick().unwrap(), RunEvent::Unchanged);
        assert!(writes.borrow().is_empty());
    }

    #[test]
    fn app_layer_capability_is_required_for_sending() {
        let mut devices = std::collections::BTreeMap::new();
        devices.insert(
            "uid:00000000000000aa".to_string(),
            crate::config::DeviceLayerSwitchConfig {
                rules: vec![code_rule("device", 2)],
                ..Default::default()
            },
        );
        let config = AppConfig {
            layer_switch: crate::config::LayerSwitchConfig {
                enabled: true,
                unmatched_action: crate::config::UnmatchedAction::ClearManaged,
                devices,
            },
            ..AppConfig::default()
        };
        let transport = MultiMockTransport {
            devices: vec![device_with_uid("a", 0xaa, 0)],
            writes: Rc::new(RefCell::new(Vec::new())),
            uplink: Rc::default(),
        };
        let writes = transport.writes.clone();
        let hid = HidDeviceManager::new(HidConfig::default(), transport);
        let mut runner = Runner::new(config, code_app_provider(), hid);

        assert_eq!(runner.tick().unwrap(), RunEvent::Unchanged);
        assert!(writes.borrow().is_empty());
    }

    #[derive(Debug)]
    struct SwappableProvider {
        app: RefCell<crate::ActiveApp>,
    }

    impl ActiveAppProvider for SwappableProvider {
        fn active_app(&self) -> Result<crate::ActiveApp, ActiveAppError> {
            Ok(self.app.borrow().clone())
        }
    }

    #[test]
    fn device_unmatched_action_keep_overrides_global_clear() {
        let mut devices = std::collections::BTreeMap::new();
        devices.insert(
            "uid:00000000000000aa".to_string(),
            crate::config::DeviceLayerSwitchConfig {
                rules: vec![code_rule("device", 3)],
                unmatched_action: Some(crate::config::UnmatchedAction::Keep),
                ..Default::default()
            },
        );
        let config = AppConfig {
            layer_switch: crate::config::LayerSwitchConfig {
                enabled: true,
                // Global default would clear, but the device override keeps.
                unmatched_action: crate::config::UnmatchedAction::ClearManaged,
                devices,
            },
            ..AppConfig::default()
        };
        let transport = MultiMockTransport {
            devices: vec![device_with_uid(
                "a",
                0xaa,
                crate::packet::CAPABILITY_APP_LAYER,
            )],
            writes: Rc::new(RefCell::new(Vec::new())),
            uplink: Rc::default(),
        };
        let writes = transport.writes.clone();
        let hid = HidDeviceManager::new(HidConfig::default(), transport);
        let provider = SwappableProvider {
            app: RefCell::new(crate::ActiveApp {
                process_path: Some(PathBuf::from("C:\\Apps\\Code.exe")),
                exe: Some("Code.exe".to_string()),
                title: None,
            }),
        };
        let mut runner = Runner::new(config, provider, hid);

        assert!(matches!(
            runner.tick().unwrap(),
            RunEvent::SetLayer { layer: 3, .. }
        ));

        // Switch to an app that matches no rule; Keep must suppress the clear.
        *runner.app_provider.app.borrow_mut() = crate::ActiveApp {
            process_path: Some(PathBuf::from("C:\\Apps\\other.exe")),
            exe: Some("other.exe".to_string()),
            title: None,
        };
        assert_eq!(runner.tick().unwrap(), RunEvent::Unchanged);

        // Only the initial set was written; no clear packet.
        assert_eq!(writes.borrow().len(), 1);
        assert_eq!(writes.borrow()[0].1[5], 1); // AppLayerAction::Set
    }

    fn push_uplink(
        uplink: &Rc<RefCell<HashMap<String, std::collections::VecDeque<[u8; crate::PACKET_SIZE]>>>>,
        path: &str,
        packet: &crate::packet::UplinkPacket,
    ) {
        uplink
            .borrow_mut()
            .entry(path.to_string())
            .or_default()
            .push_back(packet.encode_payload());
    }

    #[test]
    fn uplink_updates_battery_and_layer_views() {
        use crate::packet::{
            BatteryEntry, BatteryStatusPacket, LayerStatePacket, UplinkPacket, CAPABILITY_BATTERY,
            CAPABILITY_LAYER_STATE,
        };

        let transport = MultiMockTransport {
            devices: vec![device_with_uid(
                "a",
                0xaa,
                CAPABILITY_BATTERY | CAPABILITY_LAYER_STATE,
            )],
            writes: Rc::new(RefCell::new(Vec::new())),
            uplink: Rc::default(),
        };
        let uplink = transport.uplink.clone();
        push_uplink(
            &uplink,
            "a",
            &UplinkPacket::Battery(BatteryStatusPacket {
                entries: vec![
                    BatteryEntry {
                        source: 0,
                        level: Some(90),
                    },
                    BatteryEntry {
                        source: 1,
                        level: None,
                    },
                ],
            }),
        );
        push_uplink(
            &uplink,
            "a",
            &UplinkPacket::LayerState(LayerStatePacket {
                active_layer: 4,
                layer_mask: 0b10001,
                seq: 1,
            }),
        );
        let hid = HidDeviceManager::new(HidConfig::default(), transport);
        let mut runner = Runner::new(AppConfig::default(), code_app_provider(), hid);

        runner.tick().unwrap();

        let battery = runner.battery_statuses();
        assert_eq!(battery.len(), 1);
        assert_eq!(battery[0].device_key, "uid:00000000000000aa");
        assert_eq!(
            battery[0].sources,
            vec![
                DeviceBatterySource {
                    source: 0,
                    level: Some(90)
                },
                DeviceBatterySource {
                    source: 1,
                    level: None
                },
            ]
        );

        let layers = runner.layer_states();
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].active_layer, 4);
        assert_eq!(layers[0].layer_mask, 0b10001);

        assert_eq!(runner.take_uplink_events().len(), 2);
        assert!(runner.take_uplink_events().is_empty());
    }

    #[test]
    fn layer_state_uplink_does_not_affect_managed_layers() {
        use crate::packet::{
            LayerStatePacket, UplinkPacket, CAPABILITY_APP_LAYER, CAPABILITY_LAYER_STATE,
        };

        let mut devices = std::collections::BTreeMap::new();
        devices.insert(
            "uid:00000000000000aa".to_string(),
            crate::config::DeviceLayerSwitchConfig {
                rules: vec![code_rule("device", 3)],
                ..Default::default()
            },
        );
        let config = AppConfig {
            layer_switch: crate::config::LayerSwitchConfig {
                enabled: true,
                unmatched_action: crate::config::UnmatchedAction::ClearManaged,
                devices,
            },
            ..AppConfig::default()
        };
        let transport = MultiMockTransport {
            devices: vec![device_with_uid(
                "a",
                0xaa,
                CAPABILITY_APP_LAYER | CAPABILITY_LAYER_STATE,
            )],
            writes: Rc::new(RefCell::new(Vec::new())),
            uplink: Rc::default(),
        };
        let writes = transport.writes.clone();
        let uplink = transport.uplink.clone();
        let hid = HidDeviceManager::new(HidConfig::default(), transport);
        let mut runner = Runner::new(config, code_app_provider(), hid);

        assert!(matches!(
            runner.tick().unwrap(),
            RunEvent::SetLayer { layer: 3, .. }
        ));
        assert_eq!(writes.borrow().len(), 1);

        // The keyboard reports a manual layer change. This must be reflected
        // in the view only: no extra APP_LAYER write, managed state untouched.
        push_uplink(
            &uplink,
            "a",
            &UplinkPacket::LayerState(LayerStatePacket {
                active_layer: 7,
                layer_mask: 0,
                seq: 0,
            }),
        );
        assert_eq!(runner.tick().unwrap(), RunEvent::Unchanged);

        assert_eq!(writes.borrow().len(), 1);
        assert_eq!(runner.layer_states()[0].active_layer, 7);
    }

    #[test]
    fn host_action_duplicate_seq_is_dropped() {
        use crate::packet::{HostActionPacket, UplinkPacket, CAPABILITY_HOST_ACTION};

        let transport = MultiMockTransport {
            devices: vec![device_with_uid("a", 0xaa, CAPABILITY_HOST_ACTION)],
            writes: Rc::new(RefCell::new(Vec::new())),
            uplink: Rc::default(),
        };
        let uplink = transport.uplink.clone();
        let action = |seq: u8| {
            UplinkPacket::HostAction(HostActionPacket {
                action_id: 1,
                value: 0,
                seq,
            })
        };
        push_uplink(&uplink, "a", &action(5));
        push_uplink(&uplink, "a", &action(5)); // duplicate, dropped
        push_uplink(&uplink, "a", &action(6));
        let hid = HidDeviceManager::new(HidConfig::default(), transport);
        let mut runner = Runner::new(AppConfig::default(), code_app_provider(), hid);

        runner.tick().unwrap();

        let events = runner.take_uplink_events();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn battery_uplink_without_capability_is_still_displayed() {
        use crate::packet::{BatteryEntry, BatteryStatusPacket, UplinkPacket};

        let transport = MultiMockTransport {
            devices: vec![device_with_uid(
                "a",
                0xaa,
                crate::packet::CAPABILITY_APP_LAYER,
            )],
            writes: Rc::new(RefCell::new(Vec::new())),
            uplink: Rc::default(),
        };
        let uplink = transport.uplink.clone();
        push_uplink(
            &uplink,
            "a",
            &UplinkPacket::Battery(BatteryStatusPacket {
                entries: vec![BatteryEntry {
                    source: 0,
                    level: Some(50),
                }],
            }),
        );
        let hid = HidDeviceManager::new(HidConfig::default(), transport);
        let mut runner = Runner::new(AppConfig::default(), code_app_provider(), hid);

        runner.tick().unwrap();

        let battery = runner.battery_statuses();
        assert_eq!(battery.len(), 1);
        assert_eq!(battery[0].sources[0].level, Some(50));
        assert_eq!(runner.take_uplink_events().len(), 1);
    }

    #[test]
    fn key_stats_uplink_feeds_store() {
        use crate::packet::{KeyStatsEntry, KeyStatsPacket, UplinkPacket, CAPABILITY_KEY_STATS};
        use crate::stats::{KeyStatsStore, StatsPeriod};
        use std::sync::{Arc, Mutex};

        let dir = std::env::temp_dir().join(format!(
            "keylink-studio-runner-stats-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Arc::new(Mutex::new(KeyStatsStore::new(
            dir.clone(),
            Duration::from_secs(3600),
        )));

        let transport = MultiMockTransport {
            devices: vec![device_with_uid("a", 0xaa, CAPABILITY_KEY_STATS)],
            writes: Rc::new(RefCell::new(Vec::new())),
            uplink: Rc::default(),
        };
        let uplink = transport.uplink.clone();
        push_uplink(
            &uplink,
            "a",
            &UplinkPacket::KeyStats(KeyStatsPacket {
                entries: vec![
                    KeyStatsEntry {
                        position: 2,
                        delta: 5,
                    },
                    KeyStatsEntry {
                        position: 9,
                        delta: 1,
                    },
                ],
                more_follows: false,
                seq: 0,
            }),
        );
        let hid = HidDeviceManager::new(HidConfig::default(), transport);
        let clock = FakeClock {
            snapshot: crate::TimeSnapshot {
                unix_time_sec: 1_765_000_000,
                tz_offset_min: 540,
            },
        };
        let mut runner =
            Runner::new_with_clock(AppConfig::default(), code_app_provider(), hid, clock);
        runner.set_key_stats_store(store.clone());

        runner.tick().unwrap();

        let summary =
            store
                .lock()
                .unwrap()
                .summary("uid:00000000000000aa", StatsPeriod::All, "2026-01-01");
        assert_eq!(summary.total, 6);
        assert_eq!(runner.take_uplink_events().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn time_sync_is_sent_on_initial_tick() {
        let config = AppConfig {
            time: crate::config::TimeConfig {
                enabled: true,
                ..Default::default()
            },
            ..AppConfig::default()
        };
        let provider = MockAppProvider {
            app: crate::ActiveApp::default(),
        };
        let transport = MockTransport::default();
        let hid = HidDeviceManager::new(HidConfig::default(), transport);
        let clock = FakeClock {
            snapshot: crate::TimeSnapshot {
                unix_time_sec: 1_704_122_200,
                tz_offset_min: 540,
            },
        };
        let mut runner = Runner::new_with_clock(config, provider, hid, clock);

        let _ = runner.tick().unwrap();
    }
}
