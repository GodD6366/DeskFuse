//! Session roster for connected, trusted devices. Pairing authority stays in identity/runtime.
use crate::api::{ConnectionState, DeviceInfo, DeviceState, SessionInfo};
use std::collections::BTreeMap;
use unidesk_core::DeviceId;

#[derive(Clone, Debug)]
struct TrackedDevice {
    info: DeviceInfo,
    trusted: bool,
    connected: bool,
    generation: u64,
}

/// In-memory device/session map owned by the collaboration core.
#[derive(Debug, Default)]
pub struct SessionManager {
    devices: BTreeMap<DeviceId, TrackedDevice>,
    next_generation: u64,
}

impl SessionManager {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn discover(&mut self, info: DeviceInfo) {
        self.devices.entry(info.id).or_insert(TrackedDevice {
            info,
            trusted: false,
            connected: false,
            generation: 0,
        });
    }

    pub fn lose(&mut self, device_id: DeviceId) -> bool {
        self.devices.remove(&device_id).is_some()
    }

    pub fn accept(&mut self, device_id: DeviceId) -> Option<DeviceInfo> {
        let device = self.devices.get_mut(&device_id)?;
        device.trusted = true;
        Some(device.info.clone())
    }

    pub fn reject(&mut self, device_id: DeviceId) -> bool {
        self.devices.remove(&device_id).is_some()
    }

    pub fn remove_trust(&mut self, device_id: DeviceId) -> bool {
        if let Some(device) = self.devices.get_mut(&device_id) {
            device.trusted = false;
            device.connected = false;
            true
        } else {
            false
        }
    }

    pub fn connect(&mut self, device_id: DeviceId) -> Option<SessionInfo> {
        let device = self.devices.get_mut(&device_id)?;
        if !device.trusted {
            return None;
        }
        self.next_generation += 1;
        device.connected = true;
        device.generation = self.next_generation;
        Some(SessionInfo {
            device_id,
            generation: device.generation,
        })
    }

    pub fn disconnect(&mut self, device_id: DeviceId) -> bool {
        if let Some(device) = self.devices.get_mut(&device_id) {
            let was_connected = device.connected;
            device.connected = false;
            was_connected
        } else {
            false
        }
    }

    pub fn is_connected(&self, device_id: DeviceId) -> bool {
        self.devices
            .get(&device_id)
            .is_some_and(|device| device.connected)
    }

    pub fn device_states(&self) -> Vec<DeviceState> {
        self.devices
            .values()
            .map(|device| DeviceState {
                id: device.info.id,
                name: device.info.name.clone(),
                trusted: device.trusted,
                connected: device.connected,
            })
            .collect()
    }

    pub fn connections(&self) -> Vec<ConnectionState> {
        self.devices
            .values()
            .filter(|device| device.connected)
            .map(|device| ConnectionState {
                device_id: device.info.id,
                generation: device.generation,
            })
            .collect()
    }
}
