use crate::{Device, Result, StoreError};
use deskfuse_core::DeviceId;
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::{Arc, Mutex};

/// Device roster for the collaboration core database.
#[derive(Clone, Debug)]
pub struct DeviceRepository {
    conn: Arc<Mutex<Connection>>,
}

impl DeviceRepository {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn insert(&self, device: &Device) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO devices (id, public_key, name, device_type, created_at, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                device.id.to_string(),
                device.public_key.as_slice(),
                device.name,
                device.device_type,
                device.created_at,
                device.last_seen,
            ],
        )?;
        Ok(())
    }

    pub fn find_by_id(&self, id: DeviceId) -> Result<Option<Device>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, public_key, name, device_type, created_at, last_seen
             FROM devices WHERE id = ?1",
        )?;
        let device = stmt
            .query_row(params![id.to_string()], |row| {
                let id_text: String = row.get(0)?;
                let public_key: Vec<u8> = row.get(1)?;
                Ok(Device {
                    id: id_text.parse().map_err(|error| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(error))
                    })?,
                    public_key: public_key
                        .try_into()
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    name: row.get(2)?,
                    device_type: row.get(3)?,
                    created_at: row.get(4)?,
                    last_seen: row.get(5)?,
                })
            })
            .optional()?;
        Ok(device)
    }

    pub fn list(&self) -> Result<Vec<Device>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, public_key, name, device_type, created_at, last_seen FROM devices",
        )?;
        let rows = stmt.query_map([], |row| {
            let id_text: String = row.get(0)?;
            let public_key: Vec<u8> = row.get(1)?;
            Ok(Device {
                id: id_text
                    .parse()
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?,
                public_key: public_key
                    .try_into()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                name: row.get(2)?,
                device_type: row.get(3)?,
                created_at: row.get(4)?,
                last_seen: row.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn touch(&self, id: DeviceId, last_seen: i64) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE devices SET last_seen = ?1 WHERE id = ?2",
            params![last_seen, id.to_string()],
        )?;
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| StoreError::Poisoned)
    }
}
