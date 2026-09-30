use crate::{ContentEntry, Result, StoreError};
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::{Arc, Mutex};

/// Content-library items. This database is independent of device/trust state.
#[derive(Clone, Debug)]
pub struct ContentRepository {
    conn: Arc<Mutex<Connection>>,
}

impl ContentRepository {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn insert(&self, entry: &ContentEntry) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO cl_contents
             (id, content_type, title, snippet, data_hash, size, created_at, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                entry.id,
                entry.content_type,
                entry.title,
                entry.snippet,
                entry.data_hash,
                entry.size,
                entry.created_at,
                entry.indexed_at,
            ],
        )?;
        conn.execute(
            "INSERT INTO cl_search(content_id, title, snippet) VALUES (?1, ?2, ?3)",
            params![entry.id, entry.title, entry.snippet],
        )?;
        Ok(())
    }

    pub fn find(&self, id: &str) -> Result<Option<ContentEntry>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, content_type, title, snippet, data_hash, size, created_at, indexed_at
             FROM cl_contents WHERE id = ?1",
        )?;
        let entry = stmt
            .query_row(params![id], |row| {
                Ok(ContentEntry {
                    id: row.get(0)?,
                    content_type: row.get(1)?,
                    title: row.get(2)?,
                    snippet: row.get(3)?,
                    data_hash: row.get(4)?,
                    size: row.get(5)?,
                    created_at: row.get(6)?,
                    indexed_at: row.get(7)?,
                })
            })
            .optional()?;
        Ok(entry)
    }

    pub fn attach_file(
        &self,
        content_id: &str,
        file_path: &str,
        file_hash: Option<&str>,
    ) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR REPLACE INTO cl_files (content_id, file_path, file_hash)
             VALUES (?1, ?2, ?3)",
            params![content_id, file_path, file_hash],
        )?;
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| StoreError::Poisoned)
    }
}
