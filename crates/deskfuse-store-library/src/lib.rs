//! Content-library persistence. Device/trust tables live in `deskfuse-store-core`.
#![forbid(unsafe_code)]

mod content_repo;
mod search;

pub use content_repo::ContentRepository;
pub use search::SearchIndex;

use rusqlite::Connection;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use thiserror::Error;

const SCHEMA: &str = include_str!("schema.sql");

/// Errors from the content-library database.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("could not prepare store directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("store connection lock is poisoned")]
    Poisoned,
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// One indexed clipboard or file item. Bodies stay in the cache, not this row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentEntry {
    pub id: String,
    pub content_type: String,
    pub title: Option<String>,
    pub snippet: Option<String>,
    pub data_hash: Option<String>,
    pub size: Option<i64>,
    pub created_at: i64,
    pub indexed_at: i64,
}

/// Opened library database.
#[derive(Clone, Debug)]
pub struct LibraryStore {
    pub contents: ContentRepository,
    pub search: SearchIndex,
}

impl LibraryStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::from_connection(Connection::open(path)?)
    }

    pub fn memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA)?;
        let conn = Arc::new(Mutex::new(conn));
        Ok(Self {
            contents: ContentRepository::new(Arc::clone(&conn)),
            search: SearchIndex::new(conn),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_and_searches_title() {
        let store = LibraryStore::memory().expect("memory");
        store
            .contents
            .insert(&ContentEntry {
                id: "c1".into(),
                content_type: "text/plain".into(),
                title: Some("Hello from Mac".into()),
                snippet: Some("Hello from Mac".into()),
                data_hash: None,
                size: Some(14),
                created_at: 1,
                indexed_at: 1,
            })
            .expect("insert");
        let hits = store.search.query("Hello", 8).expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "c1");
    }
}
