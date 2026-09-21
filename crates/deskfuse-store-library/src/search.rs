use crate::{ContentEntry, Result, StoreError};
use rusqlite::{Connection, params};
use std::sync::{Arc, Mutex};

/// FTS5 search over content-library titles and snippets.
#[derive(Clone, Debug)]
pub struct SearchIndex {
    conn: Arc<Mutex<Connection>>,
}

impl SearchIndex {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn query(&self, needle: &str, limit: usize) -> Result<Vec<ContentEntry>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT c.id, c.content_type, c.title, c.snippet, c.data_hash, c.size, c.created_at, c.indexed_at
             FROM cl_search s
             JOIN cl_contents c ON c.id = s.content_id
             WHERE ifnull(s.title, '') LIKE '%' || ?1 || '%'
                OR ifnull(s.snippet, '') LIKE '%' || ?1 || '%'
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![needle, limit as i64], |row| {
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
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| StoreError::Poisoned)
    }
}
