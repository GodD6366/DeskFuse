use crate::{Result, StoreError};
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::{Arc, Mutex};

/// Opaque configuration blobs for the collaboration core.
#[derive(Clone, Debug)]
pub struct ConfigRepository {
    conn: Arc<Mutex<Connection>>,
}

impl ConfigRepository {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn put(&self, key: &str, value: &[u8], updated_at: i64) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR REPLACE INTO config (key, value, updated_at) VALUES (?1, ?2, ?3)",
            params![key, value, updated_at],
        )?;
        Ok(())
    }

    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT value FROM config WHERE key = ?1")?;
        let value = stmt.query_row(params![key], |row| row.get(0)).optional()?;
        Ok(value)
    }

    pub fn save_checkpoint(
        &self,
        transfer_id: &str,
        device_id: &str,
        progress: &[u8],
        updated_at: i64,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR REPLACE INTO transfer_checkpoints
             (transfer_id, device_id, progress, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params![transfer_id, device_id, progress, updated_at],
        )?;
        Ok(())
    }

    pub fn checkpoint(&self, transfer_id: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.lock()?;
        let mut stmt =
            conn.prepare("SELECT progress FROM transfer_checkpoints WHERE transfer_id = ?1")?;
        Ok(stmt
            .query_row(params![transfer_id], |row| row.get(0))
            .optional()?)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| StoreError::Poisoned)
    }
}
