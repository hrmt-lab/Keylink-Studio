use std::{thread, time::Duration};

use tracing::{debug, info, warn};

use crate::{
    active_app::{ActiveAppError, ActiveAppProvider},
    app_match::{match_action, LayerAction},
    config::AppConfig,
    hid::{HidDeviceManager, HidError, HidTransport},
    time::{Clock, SystemClock, TimeError, TimeSyncState},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEvent {
    SetLayer { layer: u8, rule_name: String },
    Clear,
    Unchanged,
}

#[derive(Debug)]
pub struct Runner<P, T, C = SystemClock> {
    config: AppConfig,
    app_provider: P,
    hid: HidDeviceManager<T>,
    clock: C,
    time_sync: TimeSyncState,
    last_action: Option<LayerAction>,
    last_device_generation: u64,
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
        Self {
            config,
            app_provider,
            hid,
            clock,
            time_sync: TimeSyncState::default(),
            last_action: None,
            last_device_generation: 0,
        }
    }

    pub fn tick(&mut self) -> Result<RunEvent, RunnerError> {
        self.hid.ensure_verified()?;
        let device_generation = self.hid.device_generation();
        self.sync_time_if_due(device_generation)?;
        let app = match self.app_provider.active_app() {
            Ok(app) => app,
            Err(ActiveAppError::NoForegroundWindow) => return Ok(RunEvent::Unchanged),
            Err(error) => return Err(error.into()),
        };
        let (action, matched) = if self.config.layer_switch.enabled {
            match_action(&app, &self.config.layer_switch.rules)
        } else {
            (LayerAction::Clear, None)
        };
        if self.last_action == Some(action) && self.last_device_generation == device_generation {
            return Ok(RunEvent::Unchanged);
        }

        match action {
            LayerAction::Set(layer) => {
                let sent = self.hid.send_set_layer(layer)?;
                let rule_name = matched
                    .map(|matched| matched.rule.name.clone())
                    .unwrap_or_else(|| "<unknown>".to_string());
                info!(
                    "set layer {} by rule {} for {} devices",
                    layer, rule_name, sent
                );
                self.last_action = Some(action);
                self.last_device_generation = self.hid.device_generation();
                Ok(RunEvent::SetLayer { layer, rule_name })
            }
            LayerAction::Clear => {
                let sent = self.hid.send_clear()?;
                debug!("clear layer for {} devices", sent);
                self.last_action = Some(action);
                self.last_device_generation = self.hid.device_generation();
                Ok(RunEvent::Clear)
            }
        }
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

    pub fn run_forever(&mut self) -> ! {
        let interval = Duration::from_millis(self.config.polling.interval_ms);
        loop {
            if let Err(error) = self.tick() {
                warn!("run tick failed: {}", error);
            }
            thread::sleep(interval);
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
    use std::{cell::RefCell, path::PathBuf};

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
            _packet: Packet,
            _timeout_ms: i32,
        ) -> Result<bool, HidError> {
            Ok(true)
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
            manufacturer: None,
            product: None,
            serial_number: None,
        }
    }

    #[test]
    fn unchanged_state_does_not_send_again() {
        let config = AppConfig {
            layer_switch: crate::config::LayerSwitchConfig {
                enabled: true,
                rules: vec![RuleConfig {
                    name: "notepad".to_string(),
                    layer: 1,
                    path: None,
                    exe: Some("notepad.exe".to_string()),
                    title: None,
                }],
            },
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
            layer_switch: crate::config::LayerSwitchConfig {
                enabled: true,
                rules: vec![RuleConfig {
                    name: "notepad".to_string(),
                    layer: 1,
                    path: None,
                    exe: Some("notepad.exe".to_string()),
                    title: None,
                }],
            },
            ..AppConfig::default()
        };
        let transport = MockTransport::default();
        let hid = HidDeviceManager::new(HidConfig::default(), transport);
        let mut runner = Runner::new(config, NoForegroundProvider, hid);

        assert_eq!(runner.tick().unwrap(), RunEvent::Unchanged);
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
