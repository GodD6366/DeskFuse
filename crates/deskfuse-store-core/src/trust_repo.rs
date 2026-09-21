use crate::{Result, StoreError, TrustGrant};
use deskfuse_core::DeviceId;
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::{Arc, Mutex};

/// Trust grants stored independently from the content library.
#[derive(Clone, Debug)]
pub struct TrustRepository {
    conn: Arc<Mutex<Connection>>,
}

impl TrustRepository {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn insert(&self, grant: &TrustGrant) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR REPLACE INTO trust (device_id, trusted_at, trusted_by)
             VALUES (?1, ?2, ?3)",
            params![
                grant.device_id.to_string(),
                grant.trusted_at,
                grant.trusted_by
            ],
        )?;
        Ok(())
    }

    pub fn find(&self, device_id: DeviceId) -> Result<Option<TrustGrant>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT device_id, trusted_at, trusted_by FROM trust WHERE device_id = ?1")?;
        let grant = stmt
            .query_row(params![device_id.to_string()], |row| {
                let id_text: String = row.get(0)?;
                Ok(TrustGrant {
                    device_id: id_text.parse().map_err(|error| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(error))
                    })?,
                    trusted_at: row.get(1)?,
                    trusted_by: row.get(2)?,
                })
            })
            .optional()?;
        Ok(grant)
    }

    pub fn remove(&self, device_id: DeviceId) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM trust WHERE device_id = ?1",
            params![device_id.to_string()],
        )?;
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| StoreError::Poisoned)
    }
}
