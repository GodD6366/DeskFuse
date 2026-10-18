//! Core collaboration persistence. Content-library tables do not belong here.
#![forbid(unsafe_code)]

mod config_repo;
mod device_repo;
mod trust_repo;

pub use config_repo::ConfigRepository;
pub use device_repo::DeviceRepository;
pub use trust_repo::TrustRepository;

use deskfuse_core::DeviceId;
use rusqlite::Connection;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use thiserror::Error;

const SCHEMA: &str = include_str!("schema.sql");

/// Errors from the core state database.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("could not prepare store directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("store connection lock is poisoned")]
    Poisoned,
    #[error("invalid stored device ID")]
    InvalidDeviceId,
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// A paired or discovered device persisted by the collaboration core.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Device {
    pub id: DeviceId,
    pub public_key: [u8; 32],
    pub name: String,
    pub device_type: String,
    pub created_at: i64,
    pub last_seen: Option<i64>,
}

/// A local trust grant. Identity remains the authority; this is durable memory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustGrant {
    pub device_id: DeviceId,
    pub trusted_at: i64,
    pub trusted_by: String,
}

/// Opened core database with domain repositories.
#[derive(Clone, Debug)]
pub struct CoreStore {
    pub devices: DeviceRepository,
    pub trust: TrustRepository,
    pub config: ConfigRepository,
}

impl CoreStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    pub fn memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA)?;
        let conn = Arc::new(Mutex::new(conn));
        Ok(Self {
            devices: DeviceRepository::new(Arc::clone(&conn)),
            trust: TrustRepository::new(Arc::clone(&conn)),
            config: ConfigRepository::new(conn),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_device() -> Device {
        let public_key = [7_u8; 32];
        Device {
            id: DeviceId::from_public_key(public_key).expect("id"),
            public_key,
            name: "mac-a".into(),
            device_type: "macos".into(),
            created_at: 1,
            last_seen: None,
        }
    }

    #[test]
    fn round_trips_devices_and_trust() {
        let store = CoreStore::memory().expect("memory");
        let device = sample_device();
        store.devices.insert(&device).expect("insert");
        store
            .trust
            .insert(&TrustGrant {
                device_id: device.id,
                trusted_at: 2,
                trusted_by: "local".into(),
            })
            .expect("trust");
        assert_eq!(
            store.devices.find_by_id(device.id).expect("find"),
            Some(device.clone())
        );
        assert!(store.trust.find(device.id).expect("trust").is_some());
        store
            .config
            .put("clipboard.enabled", b"1", 3)
            .expect("config");
        assert_eq!(
            store.config.get("clipboard.enabled").expect("get"),
            Some(b"1".to_vec())
        );
    }
}
