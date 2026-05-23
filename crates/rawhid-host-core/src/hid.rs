use std::{cell::RefCell, collections::HashMap, ffi::CString, fmt};

use hidapi::HidApi;
use thiserror::Error;
use tracing::{debug, warn};

use crate::{
    config::HidConfig,
    packet::{Packet, PacketType, TimeSyncPacket, PACKET_SIZE, REPORT_SIZE},
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeviceInfo {
    pub path: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProbeResult {
    pub device: DeviceInfo,
    pub hello_ok: bool,
    pub error: Option<String>,
}

pub trait HidTransport {
    fn candidates(&self, usage_page: u16, usage: u16) -> Result<Vec<DeviceInfo>, HidError>;
    fn hello(&self, device: &DeviceInfo, packet: Packet, timeout_ms: i32)
        -> Result<bool, HidError>;
    fn write_report(&self, device: &DeviceInfo, report: &[u8; REPORT_SIZE])
        -> Result<(), HidError>;
    fn forget_device(&self, _device: &DeviceInfo) {}
}

pub struct RealHidTransport {
    api: RefCell<HidApi>,
    handles: RefCell<HashMap<String, hidapi::HidDevice>>,
}

impl RealHidTransport {
    pub fn new() -> Result<Self, HidError> {
        Ok(Self {
            api: RefCell::new(HidApi::new().map_err(HidError::Hid)?),
            handles: RefCell::new(HashMap::new()),
        })
    }

    fn open_device(&self, path: &str) -> Result<hidapi::HidDevice, HidError> {
        let path = CString::new(path).map_err(|_| HidError::InvalidDevicePath)?;
        self.api.borrow().open_path(&path).map_err(HidError::Hid)
    }
}

impl fmt::Debug for RealHidTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RealHidTransport")
            .field("open_handles", &self.handles.borrow().len())
            .finish_non_exhaustive()
    }
}

impl HidTransport for RealHidTransport {
    fn candidates(&self, usage_page: u16, usage: u16) -> Result<Vec<DeviceInfo>, HidError> {
        let mut api = self.api.borrow_mut();
        api.refresh_devices().map_err(HidError::Hid)?;
        Ok(api
            .device_list()
            .filter(|device| device.usage_page() == usage_page && device.usage() == usage)
            .map(|device| DeviceInfo {
                path: device.path().to_string_lossy().to_string(),
                vendor_id: device.vendor_id(),
                product_id: device.product_id(),
                usage_page: device.usage_page(),
                usage: device.usage(),
                manufacturer: device.manufacturer_string().map(ToString::to_string),
                product: device.product_string().map(ToString::to_string),
                serial_number: device.serial_number().map(ToString::to_string),
            })
            .collect())
    }

    fn hello(
        &self,
        device: &DeviceInfo,
        packet: Packet,
        timeout_ms: i32,
    ) -> Result<bool, HidError> {
        let hid = self.open_device(&device.path)?;
        hid.write(&packet.encode_report()).map_err(HidError::Hid)?;

        let mut buffer = [0u8; PACKET_SIZE];
        let read = hid
            .read_timeout(&mut buffer, timeout_ms)
            .map_err(HidError::Hid)?;
        if read == 0 {
            return Ok(false);
        }
        if read != PACKET_SIZE {
            return Ok(false);
        }
        let response = Packet::decode_payload(&buffer).map_err(HidError::Packet)?;
        let hello_ok =
            response.packet_type == PacketType::HelloResponse && response.seq == packet.seq;
        if hello_ok {
            self.handles.borrow_mut().insert(device.path.clone(), hid);
        }
        Ok(hello_ok)
    }

    fn write_report(
        &self,
        device: &DeviceInfo,
        report: &[u8; REPORT_SIZE],
    ) -> Result<(), HidError> {
        if let Some(hid) = self.handles.borrow().get(&device.path) {
            hid.write(report).map_err(HidError::Hid)?;
            return Ok(());
        }

        let hid = self.open_device(&device.path)?;
        hid.write(report).map_err(HidError::Hid)?;
        self.handles.borrow_mut().insert(device.path.clone(), hid);
        Ok(())
    }

    fn forget_device(&self, device: &DeviceInfo) {
        self.handles.borrow_mut().remove(&device.path);
    }
}

#[derive(Debug)]
pub struct HidDeviceManager<T = RealHidTransport> {
    transport: T,
    config: HidConfig,
    verified: Vec<DeviceInfo>,
    seq: u8,
    generation: u64,
}

impl HidDeviceManager<RealHidTransport> {
    pub fn real(config: HidConfig) -> Result<Self, HidError> {
        Ok(Self::new(config, RealHidTransport::new()?))
    }
}

impl<T: HidTransport> HidDeviceManager<T> {
    pub fn new(config: HidConfig, transport: T) -> Self {
        Self {
            transport,
            config,
            verified: Vec::new(),
            seq: 0,
            generation: 0,
        }
    }

    pub fn verified_devices(&self) -> &[DeviceInfo] {
        &self.verified
    }

    pub fn device_generation(&self) -> u64 {
        self.generation
    }

    pub fn probe(&mut self) -> Result<Vec<ProbeResult>, HidError> {
        let candidates = self
            .transport
            .candidates(self.config.usage_page, self.config.usage)?;
        let mut results = Vec::with_capacity(candidates.len());
        let mut verified = Vec::new();

        for device in candidates {
            let seq = self.next_seq();
            let hello = Packet::hello(seq);
            match self
                .transport
                .hello(&device, hello, self.config.hello_timeout_ms)
            {
                Ok(true) => {
                    debug!("Raw HID HELLO succeeded for {}", device.path);
                    verified.push(device.clone());
                    results.push(ProbeResult {
                        device,
                        hello_ok: true,
                        error: None,
                    });
                }
                Ok(false) => {
                    results.push(ProbeResult {
                        device,
                        hello_ok: false,
                        error: None,
                    });
                }
                Err(error) => {
                    let message = error.to_string();
                    results.push(ProbeResult {
                        device,
                        hello_ok: false,
                        error: Some(message),
                    });
                }
            }
        }

        for old_device in &self.verified {
            if !verified.iter().any(|device| device.path == old_device.path) {
                self.transport.forget_device(old_device);
            }
        }

        if self.verified != verified {
            self.generation = self.generation.wrapping_add(1);
        }
        self.verified = verified;
        Ok(results)
    }

    pub fn ensure_verified(&mut self) -> Result<(), HidError> {
        if self.verified.is_empty() {
            let _ = self.probe()?;
        }
        Ok(())
    }

    pub fn send_set_layer(&mut self, layer: u8) -> Result<usize, HidError> {
        let packet = Packet::set_layer(layer, self.next_seq()).map_err(HidError::Packet)?;
        self.send_report_to_verified(packet.encode_report())
    }

    pub fn send_clear(&mut self) -> Result<usize, HidError> {
        let packet = Packet::clear(self.next_seq());
        self.send_report_to_verified(packet.encode_report())
    }

    pub fn send_time_sync(&mut self, packet: TimeSyncPacket) -> Result<usize, HidError> {
        self.send_report_to_verified(packet.encode_report())
    }

    pub fn send_report_to_verified(
        &mut self,
        report: [u8; REPORT_SIZE],
    ) -> Result<usize, HidError> {
        self.ensure_verified()?;
        let mut sent = 0usize;
        let previous_len = self.verified.len();
        let mut retained = Vec::with_capacity(self.verified.len());

        for device in self.verified.drain(..) {
            match self.transport.write_report(&device, &report) {
                Ok(()) => {
                    sent += 1;
                    retained.push(device);
                }
                Err(error) => {
                    warn!("Raw HID write failed for {}: {}", device.path, error);
                    self.transport.forget_device(&device);
                }
            }
        }

        if previous_len != retained.len() {
            self.generation = self.generation.wrapping_add(1);
        }
        self.verified = retained;
        Ok(sent)
    }

    fn next_seq(&mut self) -> u8 {
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        seq
    }
}

#[derive(Debug, Error)]
pub enum HidError {
    #[error("HID error: {0}")]
    Hid(#[from] hidapi::HidError),
    #[error("packet error: {0}")]
    Packet(#[from] crate::packet::PacketError),
    #[error("invalid HID device path")]
    InvalidDevicePath,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashSet};

    #[derive(Debug, Default)]
    struct MockTransport {
        candidates: Vec<DeviceInfo>,
        hello_paths: RefCell<HashSet<String>>,
        failing_writes: RefCell<HashSet<String>>,
        writes: RefCell<Vec<(String, [u8; REPORT_SIZE])>>,
    }

    impl HidTransport for MockTransport {
        fn candidates(&self, _usage_page: u16, _usage: u16) -> Result<Vec<DeviceInfo>, HidError> {
            Ok(self.candidates.clone())
        }

        fn hello(
            &self,
            device: &DeviceInfo,
            _packet: Packet,
            _timeout_ms: i32,
        ) -> Result<bool, HidError> {
            Ok(self.hello_paths.borrow().contains(&device.path))
        }

        fn write_report(
            &self,
            device: &DeviceInfo,
            report: &[u8; REPORT_SIZE],
        ) -> Result<(), HidError> {
            if self.failing_writes.borrow().contains(&device.path) {
                return Err(HidError::InvalidDevicePath);
            }
            self.writes
                .borrow_mut()
                .push((device.path.clone(), *report));
            Ok(())
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
    fn probe_keeps_only_hello_devices() {
        let transport = MockTransport {
            candidates: vec![device("a"), device("b")],
            ..MockTransport::default()
        };
        transport.hello_paths.borrow_mut().insert("b".to_string());
        let mut manager = HidDeviceManager::new(HidConfig::default(), transport);

        let results = manager.probe().unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(manager.verified_devices(), &[device("b")]);
    }

    #[test]
    fn sends_to_multiple_verified_devices() {
        let transport = MockTransport {
            candidates: vec![device("a"), device("b")],
            ..MockTransport::default()
        };
        transport.hello_paths.borrow_mut().insert("a".to_string());
        transport.hello_paths.borrow_mut().insert("b".to_string());
        let mut manager = HidDeviceManager::new(HidConfig::default(), transport);
        manager.probe().unwrap();

        assert_eq!(manager.send_set_layer(4).unwrap(), 2);
    }

    #[test]
    fn successful_write_does_not_change_device_generation() {
        let transport = MockTransport {
            candidates: vec![device("a")],
            ..MockTransport::default()
        };
        transport.hello_paths.borrow_mut().insert("a".to_string());
        let mut manager = HidDeviceManager::new(HidConfig::default(), transport);
        manager.probe().unwrap();
        let generation = manager.device_generation();

        assert_eq!(manager.send_set_layer(4).unwrap(), 1);

        assert_eq!(manager.device_generation(), generation);
    }

    #[test]
    fn write_failure_removes_device() {
        let transport = MockTransport {
            candidates: vec![device("a"), device("b")],
            ..MockTransport::default()
        };
        transport.hello_paths.borrow_mut().insert("a".to_string());
        transport.hello_paths.borrow_mut().insert("b".to_string());
        transport
            .failing_writes
            .borrow_mut()
            .insert("a".to_string());
        let mut manager = HidDeviceManager::new(HidConfig::default(), transport);
        manager.probe().unwrap();

        assert_eq!(manager.send_clear().unwrap(), 1);

        assert_eq!(manager.verified_devices(), &[device("b")]);
        assert_eq!(manager.device_generation(), 2);
    }
}
