//! SQLite-backed clipboard library. Native payloads remain lazy, opaque and local.
use crate::model::ClipboardHistoryItem;
use bincode::Options;
use deskfuse_platform::PasteboardSnapshot;
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub const METADATA_TYPE: &str = crate::clipboard_bundle::SOURCE_METADATA_TYPE;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Origin {
    pub device_id: String,
    pub device_name: String,
    pub app_id: Option<String>,
    pub app_name: Option<String>,
    pub copied_at_ms: Option<u64>,
    #[serde(default)]
    pub no_history: bool,
}
#[derive(Serialize, Deserialize)]
struct SnapshotV1 {
    source: String,
    snapshot: PasteboardSnapshot,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rule {
    pub field: String,
    pub op: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub value2: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct List {
    pub id: String,
    pub name: String,
    pub kind: String,
    #[serde(default = "all")]
    pub match_mode: String,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub position: i64,
}
fn all() -> String {
    "all".into()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct App {
    pub bundle_id: String,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Shortcut {
    pub enabled: bool,
    pub key_code: u16,
    pub modifiers: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub retention_days: u32,
    pub excluded_apps: Vec<App>,
    pub shortcut: Shortcut,
    /// Publish and accept the search index (titles, recognized text, metadata)
    /// across paired devices. Payloads stay on demand either way.
    #[serde(default = "yes")]
    pub sync_index: bool,
}
fn yes() -> bool {
    true
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            retention_days: 30,
            excluded_apps: vec![],
            shortcut: Shortcut {
                enabled: true,
                key_code: 9,
                modifiers: 0x180000,
            },
            sync_index: true,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Summary {
    id: u64,
    copied_at_ms: Option<u64>,
    mime: String,
    length: u64,
    preview: String,
    source: String,
    has_image: bool,
    file_count: u32,
    source_device_id: Option<String>,
    source_app_id: Option<String>,
    source_app_name: Option<String>,
    kind: String,
}

/// A synced text row carries its preview in the index record. `length` is the
/// byte count of the original text, so a preview that matches it is the whole
/// text: the row can be used before the on-demand payload transfer finishes.
fn reconstructed_text_snapshot(s: &Summary) -> Option<PasteboardSnapshot> {
    (!s.preview.is_empty()
        && s.kind == "text"
        && !s.has_image
        && s.file_count == 0
        && s.preview.len() as u64 == s.length)
        .then(|| PasteboardSnapshot {
            text: Some(s.preview.clone()),
            ..Default::default()
        })
}

#[derive(Clone, Debug)]
struct QueryRow {
    id: u64,
    member_id: String,
    summary: String,
    search_text: String,
    available: bool,
    title: String,
    ocr_text: String,
    ocr_version: i64,
    remote: bool,
    sort_at_ms: i64,
}

impl QueryRow {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let ordinary: bool = row.get(5)?;
        let global_id: String = row.get(1)?;
        Ok(Self {
            id: row.get(0)?,
            // Memberships point to the frozen archive, including from ordinary history.
            member_id: if ordinary {
                format!("archive:{global_id}")
            } else {
                global_id
            },
            summary: row.get(2)?,
            search_text: row.get(3)?,
            available: row.get(4)?,
            title: row.get(6)?,
            ocr_text: row.get(7)?,
            ocr_version: row.get(8)?,
            remote: row.get(9)?,
            sort_at_ms: row.get(10)?,
        })
    }

    fn member_id(&self) -> &str {
        &self.member_id
    }
}

const QUERY_SELECT: &str = "SELECT i.id,i.global_id,i.summary,i.search_text,i.payload IS NOT NULL,i.ordinary,COALESCE(i.title,''),COALESCE(i.ocr_text,''),COALESCE(i.ocr_version,0),COALESCE(i.remote,0),MAX(COALESCE(i.last_used,0),COALESCE(i.copied,i.imported)) FROM cl_items i";
// Keep original records and manual-list membership intact. Ordinary history
// presents one exact image, preferring a usable native image over a file alias
// or a remote index that has not downloaded its payload.
const UNIQUE_IMAGE_ROW: &str = "(i.ordinary=0 OR i.digest NOT LIKE 'image-v1:%' OR NOT EXISTS(SELECT 1 FROM cl_items d WHERE d.ordinary=1 AND d.digest=i.digest AND (d.payload IS NOT NULL,COALESCE(json_extract(d.summary,'$.file_count'),0)=0,d.id) > (i.payload IS NOT NULL,COALESCE(json_extract(i.summary,'$.file_count'),0)=0,i.id)))";
const QUERY_ORDER: &str =
    "MAX(COALESCE(i.last_used,0),COALESCE(i.copied,i.imported)) DESC,i.id DESC";
// Metadata after the payload column can live on SQLite overflow pages. Merely
// omitting the BLOB from SELECT does not prevent walking those pages. Keep both
// the image winner comparison and the complete list projection in covering
// indexes, so counts, pages and smart rules never need the payload table pages.
const QUERY_INDEXES: &str = "
CREATE INDEX IF NOT EXISTS cl_items_image_identity ON cl_items(
    digest,ordinary,(payload IS NOT NULL),
    (COALESCE(json_extract(summary,'$.file_count'),0)=0),id);
CREATE INDEX IF NOT EXISTS cl_items_metadata_page ON cl_items(
    MAX(COALESCE(last_used,0),COALESCE(copied,imported)) DESC,id DESC,
    global_id,summary,search_text,(payload IS NOT NULL),ordinary,
    title,ocr_text,ocr_version,remote,digest,search_index);
";
pub struct Library {
    db: Mutex<Connection>,
    device: Mutex<String>,
    generation: AtomicU64,
    last_prune_ms: AtomicU64,
    state_cache: Mutex<Option<(u64, String)>>,
    // Drop the connection before removing its test database directory.
    #[cfg(test)]
    _test_directory: tempfile::TempDir,
}
static LIBRARY: OnceLock<Arc<Library>> = OnceLock::new();
static LIBRARY_INIT: Mutex<()> = Mutex::new(());
pub fn global() -> Result<Arc<Library>> {
    if let Some(v) = LIBRARY.get() {
        return Ok(v.clone());
    }
    // FFI queries and the runtime can arrive together at startup. OnceLock::set
    // alone only selects the winner after both initializers have already run
    // migrations/import/pruning against the same file. Retry failures, but never
    // execute two initializations concurrently.
    let _initializing = LIBRARY_INIT.lock().map_err(err)?;
    if let Some(v) = LIBRARY.get() {
        return Ok(v.clone());
    }
    let lib = Arc::new(Library::open()?);
    let _ = LIBRARY.set(lib);
    Ok(LIBRARY.get().unwrap().clone())
}
impl Library {
    fn open() -> Result<Self> {
        // Ensure the canonical store has performed its own versioned migrations first.
        #[cfg(test)]
        let test_directory = tempfile::tempdir().map_err(err)?;
        #[cfg(test)]
        let database_path = test_directory.path().join("state.sqlite3");
        #[cfg(not(test))]
        let database_path = deskfuse_store::SQLiteStore::default_path().map_err(err)?;
        let _canonical = deskfuse_store::SQLiteStore::open(&database_path).map_err(err)?;
        let mut db = Connection::open(&database_path).map_err(err)?;
        deskfuse_store::configure_connection(&mut db).map_err(err)?;
        db.execute_batch("PRAGMA foreign_keys=ON;
   CREATE TABLE IF NOT EXISTS cl_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS cl_items(id INTEGER PRIMARY KEY AUTOINCREMENT,global_id TEXT UNIQUE NOT NULL,digest TEXT NOT NULL,summary TEXT NOT NULL,payload BLOB,search_text TEXT NOT NULL DEFAULT '',search_index TEXT NOT NULL DEFAULT '',copied INTEGER,imported INTEGER NOT NULL,ordinary INTEGER NOT NULL DEFAULT 1,blob_hash TEXT,origin_device TEXT);
   CREATE INDEX IF NOT EXISTS cl_items_date ON cl_items(copied DESC,id DESC);
   CREATE INDEX IF NOT EXISTS cl_items_page_order ON cl_items(COALESCE(copied,imported) DESC,id DESC);
   CREATE INDEX IF NOT EXISTS cl_items_digest ON cl_items(digest);
   CREATE TABLE IF NOT EXISTS cl_records(key TEXT PRIMARY KEY,revision INTEGER NOT NULL,author TEXT NOT NULL,deleted INTEGER NOT NULL,value TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS cl_operations(seq INTEGER PRIMARY KEY AUTOINCREMENT,op_id TEXT UNIQUE NOT NULL,body TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS cl_blobs(hash TEXT PRIMARY KEY,bytes BLOB NOT NULL);
   CREATE TABLE IF NOT EXISTS cl_clocks(key TEXT PRIMARY KEY,value TEXT NOT NULL);
   CREATE INDEX IF NOT EXISTS cl_operations_key ON cl_operations(json_extract(body,'$.key'));
  ").map_err(err)?;
        // Additive migrations for per-record user data on existing stores.
        let add_column = |name: &str, ddl: &str| -> Result<()> {
            let exists: i64 = db
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('cl_items') WHERE name=?1",
                    [name],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if exists == 0 {
                db.execute(ddl, []).map_err(err)?;
            }
            Ok(())
        };
        add_column("title", "ALTER TABLE cl_items ADD COLUMN title TEXT")?;
        add_column("ocr_text", "ALTER TABLE cl_items ADD COLUMN ocr_text TEXT")?;
        add_column(
            "ocr_version",
            "ALTER TABLE cl_items ADD COLUMN ocr_version INTEGER NOT NULL DEFAULT 0",
        )?;
        add_column(
            "remote",
            "ALTER TABLE cl_items ADD COLUMN remote INTEGER NOT NULL DEFAULT 0",
        )?;
        add_column(
            "last_used",
            "ALTER TABLE cl_items ADD COLUMN last_used INTEGER",
        )?;
        add_column(
            "search_index",
            "ALTER TABLE cl_items ADD COLUMN search_index TEXT NOT NULL DEFAULT ''",
        )?;
        db.execute_batch("CREATE INDEX IF NOT EXISTS cl_items_usage_order ON cl_items(MAX(COALESCE(last_used,0),COALESCE(copied,imported)) DESC,id DESC);
CREATE VIRTUAL TABLE IF NOT EXISTS cl_items_fts USING fts5(search_index, content='cl_items', content_rowid='id', tokenize='trigram', detail=none);
CREATE TRIGGER IF NOT EXISTS cl_items_search_ai AFTER INSERT ON cl_items BEGIN
    UPDATE cl_items SET search_index=trim(coalesce(new.search_text,'') || char(10) || coalesce(new.title,'') || char(10) || coalesce(new.ocr_text,'') || char(10) || coalesce(json_extract(new.summary,'$.preview'),'') || char(10) || coalesce(json_extract(new.summary,'$.source'),'') ) WHERE id=new.id;
    INSERT INTO cl_items_fts(rowid,search_index) SELECT new.id,search_index FROM cl_items WHERE id=new.id;
END;
CREATE TRIGGER IF NOT EXISTS cl_items_search_au AFTER UPDATE OF summary,search_text,title,ocr_text ON cl_items BEGIN
    UPDATE cl_items SET search_index=trim(coalesce(new.search_text,'') || char(10) || coalesce(new.title,'') || char(10) || coalesce(new.ocr_text,'') || char(10) || coalesce(json_extract(new.summary,'$.preview'),'') || char(10) || coalesce(json_extract(new.summary,'$.source'),'') ) WHERE id=new.id;
    INSERT INTO cl_items_fts(cl_items_fts,rowid,search_index) VALUES('delete',old.id,old.search_index);
    INSERT INTO cl_items_fts(rowid,search_index) SELECT new.id,search_index FROM cl_items WHERE id=new.id;
END;
CREATE TRIGGER IF NOT EXISTS cl_items_search_ad AFTER DELETE ON cl_items BEGIN
    INSERT INTO cl_items_fts(cl_items_fts,rowid,search_index) VALUES('delete',old.id,old.search_index);
END;").map_err(err)?;
        let search_index_version: Option<String> = db
            .query_row(
                "SELECT value FROM cl_meta WHERE key='search_index_v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(err)?;
        // External-content FTS COUNT(*) reads cl_items, not the index. It
        // cannot detect index corruption and needlessly scans history at launch.
        if search_index_version.as_deref() != Some("1") {
            let tx = db.transaction().map_err(err)?;
            // Build the denormalized search document once. The FTS table stores
            // only its on-disk index; row payloads and OCR are never retained in
            // the Rust process while querying.
            tx.execute(
                "UPDATE cl_items SET search_index=trim(coalesce(search_text,'') || char(10) || coalesce(title,'') || char(10) || coalesce(ocr_text,'') || char(10) || coalesce(json_extract(summary,'$.preview'),'') || char(10) || coalesce(json_extract(summary,'$.source'),'') )",
                [],
            )
            .map_err(err)?;
            tx.execute(
                "INSERT INTO cl_items_fts(cl_items_fts) VALUES('rebuild')",
                [],
            )
            .map_err(err)?;
            tx.execute(
                "INSERT INTO cl_meta(key,value) VALUES('search_index_v1','1') ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [],
            )
            .map_err(err)?;
            tx.commit().map_err(err)?;
        }
        db.execute_batch(QUERY_INDEXES).map_err(err)?;
        let this = Self {
            #[cfg(test)]
            _test_directory: test_directory,
            db: Mutex::new(db),
            device: Mutex::new(String::new()),
            generation: AtomicU64::new(1),
            last_prune_ms: AtomicU64::new(0),
            state_cache: Mutex::new(None),
        };
        this.import_legacy()?;
        this.migrate_image_digests()?;
        this.seed_default_lists()?;
        this.prune()?;
        this.last_prune_ms.store(now_ms(), Ordering::Release);
        this.reconcile_index()?;
        Ok(this)
    }
    fn migrate_image_digests(&self) -> Result<()> {
        let mut db = self.db.lock().map_err(err)?;
        if db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM cl_meta WHERE key='image_digest_v1')",
                [],
                |r| r.get::<_, bool>(0),
            )
            .map_err(err)?
        {
            return Ok(());
        }
        let tx = db.transaction().map_err(err)?;
        let ids = tx.prepare("SELECT id FROM cl_items WHERE ordinary=1 AND payload IS NOT NULL AND json_extract(summary,'$.kind')='image'")
            .map_err(err)?.query_map([], |r| r.get::<_, u64>(0)).map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>().map_err(err)?;
        // Stream payloads one at a time; never load the image history into RAM.
        for id in ids {
            let bytes: Vec<u8> = tx
                .query_row("SELECT payload FROM cl_items WHERE id=?1", [id], |r| {
                    r.get(0)
                })
                .map_err(err)?;
            if let Ok(snapshot) = decode(&bytes)
                && let Some(digest) = crate::clipboard_image::snapshot_digest(&snapshot)
            {
                tx.execute(
                    "UPDATE cl_items SET digest=?1 WHERE id=?2",
                    params![digest, id],
                )
                .map_err(err)?;
            }
        }
        tx.execute(
            "INSERT INTO cl_meta(key,value) VALUES('image_digest_v1','1')",
            [],
        )
        .map_err(err)?;
        tx.commit().map_err(err)
    }
    fn seed_default_lists(&self) -> Result<()> {
        // Identical bootstrap operations on every device deduplicate on sync.
        // Replay uses the normal merge path, so edits and deletion tombstones win.
        // UUIDs are derived from com.deskfuse.app/clipboard/defaults/v1/{kind}/{list|operation}.
        const AUTHOR: &str = "builtin:clipboard-type-groups:v1";
        let defaults = [
            (
                "67f10b9a-9f3c-508d-8090-4192e3ea297f",
                "377a95ef-606b-59e7-b8fc-156746c822b4",
                "图片",
                "image",
            ),
            (
                "9bd6453f-f270-5d1e-b806-b7cab97ae197",
                "f969b1cd-1ac8-5e24-bc5c-156feb559a80",
                "文件",
                "files",
            ),
        ];
        let operations = defaults
            .into_iter()
            .enumerate()
            .map(|(index, (id, op_id, name, kind))| {
                let revision = index as u64 + 1;
                json!({
                    "op_id": op_id, "key": format!("list:{id}"),
                    "revision": revision, "author": AUTHOR,
                    "clock": { (AUTHOR): revision }, "deleted": false,
                    "value": {
                        "id": id, "name": name, "kind": "smart", "match_mode": "all",
                        "position": index as i64 - 2,
                        "rules": [{ "field": "kind", "op": "is", "value": kind }]
                    }
                })
            })
            .collect();
        self.merge_operations(operations)
    }
    fn import_legacy(&self) -> Result<()> {
        let mut db = self.db.lock().map_err(err)?;
        let done: Option<String> = db
            .query_row(
                "SELECT value FROM cl_meta WHERE key='imported_v1'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        if done.as_deref() == Some("1") {
            // A committed migration is the proof that the old payload table is
            // no longer authoritative. Remove leftovers from earlier builds too.
            db.execute("DELETE FROM clipboard_history_v1", [])
                .map_err(err)?;
            return Ok(());
        }
        let rows = db
            .prepare("SELECT id,payload,copied_at_ms FROM clipboard_history_v1 ORDER BY position")
            .map_err(err)?
            .query_map([], |r| {
                Ok((
                    r.get::<_, u64>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, Option<u64>>(2)?,
                ))
            })
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        for (id, bytes, copied) in rows {
            let old: SnapshotV1 = bincode::DefaultOptions::new()
                .with_limit(bytes.len() as u64)
                .deserialize(&bytes)
                .map_err(|_| "旧剪贴板历史格式无效，已保留数据库".to_string())?;
            let origin = Origin {
                device_name: old.source.clone(),
                copied_at_ms: copied,
                ..Default::default()
            };
            let summary = summary(id, &old.snapshot, &origin);
            tx.execute("INSERT OR IGNORE INTO cl_items(id,global_id,digest,summary,payload,search_text,copied,imported) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![id,uuid::Uuid::new_v4().to_string(),digest(&old.snapshot)?,serde_json::to_string(&summary).map_err(err)?,bytes,old.snapshot.text.unwrap_or_default().to_lowercase(),copied,now_ms()]).map_err(err)?;
        }
        // Clear old bytes in the same transaction as their replacement rows;
        // an import failure rolls back both sides and never loses history.
        tx.execute("DELETE FROM clipboard_history_v1", [])
            .map_err(err)?;
        tx.execute("INSERT INTO cl_meta VALUES('imported_v1','1')", [])
            .map_err(err)?;
        tx.commit().map_err(err)
    }
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
    fn changed(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut cache) = self.state_cache.lock() {
            *cache = None;
        }
    }
    /// Local reuse changes ordering, never the original source/copy metadata.
    /// Called only after the native pasteboard write has succeeded.
    pub fn mark_used(&self, id: u64) -> Result<()> {
        let db = self.db.lock().map_err(err)?;
        let updated = db.execute(
            "UPDATE cl_items SET last_used=MAX(?1,COALESCE((SELECT MAX(last_used)+1 FROM cl_items),0)) WHERE id=?2",
            params![now_ms(), id],
        ).map_err(err)?;
        if updated > 0 {
            self.changed();
        }
        Ok(())
    }

    pub fn set_local_device(&self, id: String) -> Result<()> {
        *self.device.lock().map_err(err)? = id;
        // Publish migrated hashes through normal sync operations, so old remote
        // index rows acquire the same identity without deleting anyone's data.
        let ids = {
            let db = self.db.lock().map_err(err)?;
            db.prepare("SELECT i.id FROM cl_items i LEFT JOIN cl_records r ON r.key='index:' || i.global_id WHERE i.ordinary=1 AND i.remote=0 AND i.digest LIKE 'image-v1:%' AND (r.key IS NULL OR COALESCE(json_extract(r.value,'$.digest'),'')<>i.digest)")
                .map_err(err)?.query_map([], |r| r.get::<_, u64>(0)).map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>().map_err(err)?
        };
        for id in ids {
            self.publish_index_for(id)?;
        }
        Ok(())
    }
    pub fn settings(&self) -> Result<Settings> {
        let db = self.db.lock().map_err(err)?;
        settings_db(&db)
    }
    pub fn origin(&self, snapshot: &PasteboardSnapshot, source: &str) -> Origin {
        if snapshot
            .items
            .iter()
            .flat_map(|i| &i.representations)
            .any(|r| r.type_name == METADATA_TYPE)
        {
            let m = crate::clipboard_bundle::source_metadata(snapshot);
            return Origin {
                device_id: m.device_id.unwrap_or_default(),
                device_name: source.into(),
                app_id: m.app_bundle_id,
                app_name: None,
                copied_at_ms: m.created_at_ms,
                no_history: m.no_history,
            };
        }
        let local = source == "本机" || source == "工作流";
        let mut origin = Origin {
            device_id: if local {
                self.device.lock().map(|v| v.clone()).unwrap_or_default()
            } else {
                String::new()
            },
            device_name: source.into(),
            copied_at_ms: local.then(now_ms),
            ..Default::default()
        };
        #[cfg(target_os = "macos")]
        if local {
            let (id, name) = deskfuse_platform_macos::clipboard_source_application();
            origin.app_id = id;
            origin.app_name = name;
        }
        origin.no_history = self.excluded(origin.app_id.as_deref());
        origin
    }
    pub fn excluded(&self, app: Option<&str>) -> bool {
        app.is_some_and(|app| {
            self.settings()
                .is_ok_and(|s| s.excluded_apps.iter().any(|a| a.bundle_id == app))
        })
    }
    pub fn ingest(&self, snapshot: &PasteboardSnapshot, source: &str) -> Result<Option<u64>> {
        let id = self.ingest_record(snapshot, source)?;
        if let Some(id) = id {
            self.publish_index_for(id)?;
        }
        Ok(id)
    }
    /// Blocking capture paths compute the image digest once in their single
    /// decode pass and reuse it here instead of decoding the snapshot again.
    pub(crate) fn ingest_with_image_digest(
        &self,
        snapshot: &PasteboardSnapshot,
        source: &str,
        image_digest: Option<String>,
    ) -> Result<Option<u64>> {
        let id = self.ingest_record_with_image_digest(snapshot, source, image_digest)?;
        if let Some(id) = id {
            self.publish_index_for(id)?;
        }
        Ok(id)
    }
    /// HTTP uploads must distinguish a committed local record from an optional
    /// cross-device index publication failure before releasing source-file ownership.
    pub(crate) fn ingest_preserving_commit(
        &self,
        snapshot: &PasteboardSnapshot,
        source: &str,
    ) -> Result<(Option<u64>, Option<String>)> {
        let id = self.ingest_record(snapshot, source)?;
        let warning = id.and_then(|id| self.publish_index_for(id).err());
        Ok((id, warning))
    }
    fn ingest_record(&self, snapshot: &PasteboardSnapshot, source: &str) -> Result<Option<u64>> {
        let origin = self.origin(snapshot, source);
        if origin.no_history || self.excluded(origin.app_id.as_deref()) {
            return Ok(None);
        }
        let fingerprint = content_digest(snapshot)?;
        self.ingest_record_resolved(snapshot, source, origin, fingerprint)
    }
    fn ingest_record_with_image_digest(
        &self,
        snapshot: &PasteboardSnapshot,
        source: &str,
        image_digest: Option<String>,
    ) -> Result<Option<u64>> {
        let origin = self.origin(snapshot, source);
        if origin.no_history || self.excluded(origin.app_id.as_deref()) {
            return Ok(None);
        }
        let fingerprint = content_digest_with(snapshot, image_digest)?;
        self.ingest_record_resolved(snapshot, source, origin, fingerprint)
    }
    fn ingest_record_resolved(
        &self,
        snapshot: &PasteboardSnapshot,
        source: &str,
        origin: Origin,
        fingerprint: String,
    ) -> Result<Option<u64>> {
        let bytes = encode(snapshot, source)?;
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        let mut found: Option<u64> = tx
            .query_row(
                "SELECT id FROM cl_items WHERE digest=?1 AND ordinary=1 ORDER BY payload IS NOT NULL DESC,COALESCE(json_extract(summary,'$.file_count'),0)=0 DESC,id DESC LIMIT 1",
                [&fingerprint],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        if let Some(id) = found
            && fingerprint.starts_with("image-v1:")
            && !snapshot.file_urls.is_empty()
            && tx.query_row("SELECT payload IS NOT NULL AND COALESCE(json_extract(summary,'$.file_count'),0)=0 FROM cl_items WHERE id=?1", [id], |r| r.get::<_, bool>(0)).map_err(err)?
        {
            // Handoff's file alias must not replace a durable, directly pasteable
            // image with a cache path that may later disappear.
            tx.execute("UPDATE cl_items SET last_used=?1 WHERE id=?2", params![now_ms(), id]).map_err(err)?;
            tx.commit().map_err(err)?;
            drop(db);
            self.changed();
            return Ok(Some(id));
        }
        // 接力回写可能带上 TIFF/PNG 预览，digest 方案变了；用已有纯文本行兜底合并。
        let mut found_by_text = false;
        if found.is_none()
            && snapshot.file_urls.is_empty()
            && let Some(text) = crate::clipboard_echo::snapshot_plain_texts(snapshot)
                .into_iter()
                .next()
        {
            found = tx
                .query_row(
                    "SELECT id FROM cl_items WHERE ordinary=1 AND search_text=?1 AND json_extract(summary,'$.kind')='text' LIMIT 1",
                    [crate::clipboard_echo::normalize_clipboard_text(&text).to_lowercase()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(err)?;
            found_by_text = found.is_some();
        }
        let id = if let Some(id) = found {
            id
        } else {
            tx.execute(
                "INSERT INTO cl_items(global_id,digest,summary,imported) VALUES(?1,?2,'{}',?3)",
                params![uuid::Uuid::new_v4().to_string(), fingerprint, now_ms()],
            )
            .map_err(err)?;
            tx.last_insert_rowid() as u64
        };
        let mut s = summary(id, snapshot, &origin);
        if found_by_text
            && s.kind != "text"
            && let Some(text) = crate::clipboard_echo::snapshot_plain_texts(snapshot)
                .into_iter()
                .next()
        {
            s.kind = "text".into();
            s.has_image = false;
            s.mime = "text/plain".into();
            s.preview = text.chars().take(160).collect();
            s.length = text.len() as u64;
        }
        let blob_hash = hex::encode(Sha256::digest(&bytes));
        let search = crate::clipboard_echo::snapshot_plain_texts(snapshot)
            .into_iter()
            .next()
            .map(|text| crate::clipboard_echo::normalize_clipboard_text(&text).to_lowercase())
            .unwrap_or_default();
        tx.execute("UPDATE cl_items SET payload=?1,summary=?2,search_text=?3,copied=?4,imported=?5,origin_device=?6,blob_hash=?7,digest=?9,remote=0 WHERE id=?8",params![bytes,serde_json::to_string(&s).map_err(err)?,search,origin.copied_at_ms,now_ms(),origin.device_id,blob_hash,id,fingerprint]).map_err(err)?;
        tx.commit().map_err(err)?;
        drop(db);
        self.changed();
        Ok(Some(id))
    }

    /// Emit this record's `index:` sync record with its current summary, title,
    /// and recognized text. Idempotent: the local row already exists, so the
    /// materialize step is a no-op on the publisher.
    fn publish_index_for(&self, id: u64) -> Result<()> {
        if self.device.lock().map_err(err)?.is_empty() {
            return Ok(());
        }
        if !self.settings()?.sync_index {
            return Ok(());
        }
        let (gid, digest, summary, blob_hash, title, ocr_text, search_text): (
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
        ) = {
            let db = self.db.lock().map_err(err)?;
            db.query_row(
                "SELECT global_id,digest,summary,COALESCE(blob_hash,''),title,ocr_text,COALESCE(search_text,'') FROM cl_items WHERE id=?1",
                [id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .optional()
            .map_err(err)?
            .ok_or("内容不存在")?
        };
        if digest.is_empty() {
            return Ok(());
        }
        self.local_record(
            format!("index:{gid}"),
            json!({
                "id": gid,
                "digest": digest,
                "summary": serde_json::from_str::<Value>(&summary).map_err(err)?,
                "title": title,
                "ocr_text": ocr_text,
                "search_text": search_text,
                "blob_hash": blob_hash,
            }),
            false,
        )
    }

    /// Materialize remote index rows for every stored `index:` record. Used at
    /// startup and when index sync is re-enabled, so previously received
    /// records become searchable again.
    fn reconcile_index(&self) -> Result<()> {
        if !self.settings()?.sync_index {
            return Ok(());
        }
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        reconcile_index_db(&tx)?;
        tx.commit().map_err(err)
    }

    pub fn snapshot(&self, id: u64) -> Result<PasteboardSnapshot> {
        self.prune()?;
        let db = self.db.lock().map_err(err)?;
        let row: Option<(Option<Vec<u8>>, Option<String>, String)> = db
            .query_row(
                "SELECT payload,summary,COALESCE(blob_hash,'') FROM cl_items WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(err)?;
        drop(db);
        let (bytes, summary, blob_hash) = row.ok_or("内容不存在")?;
        if let Some(bytes) = bytes {
            // Decode outside the connection lock: a large native image must not
            // block metadata queries while bincode expands its representations.
            return decode(&bytes);
        }
        // A synced row keeps only its metadata until the payload is fetched, and
        // an attempted use is exactly the on-demand trigger. Queue the fetch so
        // the caller never confirms a failure that would have resolved itself.
        if blob_hash.len() == 64 && blob_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            let _ = self.want_blob(&blob_hash);
        }
        let summary = summary.and_then(|raw| serde_json::from_str::<Summary>(&raw).ok());
        if let Some(snapshot) = summary.as_ref().and_then(reconstructed_text_snapshot) {
            return Ok(snapshot);
        }
        if blob_hash.is_empty() {
            Err("历史内容已过期或被清理".into())
        } else {
            Err("内容正在从来源设备获取，请稍后重试".into())
        }
    }
    /// Return only a display-sized derivative across FFI, not the 1600px
    /// detail preview or the original restorable clipboard representations.
    pub fn thumbnail(&self, id: u64, max_pixel_size: u32) -> Result<Vec<u8>> {
        let mut snapshot = self.snapshot(id)?;
        let device = self.device.lock().map_err(err)?.clone();
        if let Some(paths) = crate::library_files::local_paths_for_preview(&snapshot, &device)? {
            snapshot.file_urls = paths;
        }
        Ok(crate::tokio_bridge::clipboard_preview_png_at_size(
            &snapshot,
            max_pixel_size.clamp(64, 480),
            None,
        )
        .map(|bytes| Arc::try_unwrap(bytes).unwrap_or_else(|bytes| (*bytes).clone()))
        .unwrap_or_default())
    }

    pub fn content(&self, id: u64) -> Result<ClipboardHistoryItem> {
        let mut snapshot = self.snapshot(id)?;
        let reference = crate::library_files::reference(&snapshot)?;
        if reference.is_some() {
            let device = self.device.lock().map_err(err)?.clone();
            if let Some(local) = crate::library_files::resolve_local(&snapshot, &device)? {
                snapshot = local;
            }
        }
        let db = self.db.lock().map_err(err)?;
        let raw: String = db
            .query_row("SELECT summary FROM cl_items WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .map_err(err)?;
        drop(db);
        let mut s: Summary = serde_json::from_str(&raw).map_err(err)?;
        project_image_file(&mut s);
        Ok(ClipboardHistoryItem {
            id,
            source_app_id: s.source_app_id,
            copied_at_ms: s.copied_at_ms,
            mime: s.mime,
            length: s.length as usize,
            preview: s.preview,
            content: snapshot
                .text
                .clone()
                .unwrap_or_else(|| reference.map(|r| r.names.join("、")).unwrap_or_default()),
            content_truncated: false,
            image_png: crate::tokio_bridge::clipboard_preview_png(&snapshot, None),
            file_paths: snapshot.file_urls,
            source: s.source,
        })
    }
    #[allow(clippy::type_complexity)] // Tuple mirrors the SQL row shape.
    pub fn recent(
        &self,
        limit: usize,
    ) -> Result<Vec<(u64, PasteboardSnapshot, String, Option<u64>)>> {
        self.prune()?;
        let db = self.db.lock().map_err(err)?;
        let mut statement = db.prepare(&format!("SELECT i.id,i.payload,i.summary FROM cl_items i WHERE i.ordinary=1 AND i.payload IS NOT NULL AND {UNIQUE_IMAGE_ROW} ORDER BY MAX(COALESCE(i.last_used,0),COALESCE(i.copied,i.imported)) DESC,i.id DESC LIMIT ?1")).map_err(err)?;
        let mut rows = statement.query([limit]).map_err(err)?;
        let mut recent = Vec::new();
        let mut retained_bytes = 0usize;
        while let Some(row) = rows.next().map_err(err)? {
            let bytes: Vec<u8> = row.get(1).map_err(err)?;
            let snapshot = decode(&bytes)?;
            let size = crate::tokio_bridge::clipboard_snapshot_bytes(&snapshot);
            if !recent.is_empty()
                && retained_bytes.saturating_add(size)
                    > crate::tokio_bridge::CLIPBOARD_HOT_CACHE_BYTES
            {
                break;
            }
            let raw: String = row.get(2).map_err(err)?;
            let summary: Summary = serde_json::from_str(&raw).map_err(err)?;
            recent.push((
                row.get(0).map_err(err)?,
                snapshot,
                summary.source,
                summary.copied_at_ms,
            ));
            retained_bytes = retained_bytes.saturating_add(size);
        }
        Ok(recent)
    }
    pub fn prune(&self) -> Result<()> {
        let mut db = self.db.lock().map_err(err)?;
        let cutoff = now_ms().saturating_sub(settings_db(&db)?.retention_days as u64 * 86400000);
        let tx = db.transaction().map_err(err)?;
        let n = tx
            .execute(
                "DELETE FROM cl_items WHERE ordinary=1 AND COALESCE(copied,imported)<?1",
                [cutoff],
            )
            .map_err(err)?;
        tx.commit().map_err(err)?;
        if n > 0 {
            self.changed()
        }
        self.last_prune_ms.store(now_ms(), Ordering::Release);
        Ok(())
    }
    pub fn clear_history(&self) -> Result<()> {
        self.db
            .lock()
            .map_err(err)?
            .execute("DELETE FROM cl_items WHERE ordinary=1", [])
            .map_err(err)?;
        self.changed();
        Ok(())
    }
    pub fn query(&self, request: &str) -> Result<String> {
        self.maybe_prune()?;
        let q: Value = serde_json::from_str(request).map_err(err)?;
        if q["op"] == "state" {
            return self.state();
        }
        if q["op"] == "counts" {
            let db = self.db.lock().map_err(err)?;
            let lists = lists_db(&db)?;
            let requests = q["requests"].as_array().ok_or("计数请求无效")?;
            if requests.len() != lists.len() + 1 {
                return Err("列表已变化，请刷新".into());
            }
            let mut counts = std::collections::BTreeMap::new();
            for request in requests {
                let key = request["list_id"].as_str().unwrap_or("all").to_string();
                // Navigation totals deliberately ignore search, device and pagination.
                let mut filter = request.clone();
                filter["query"] = json!("");
                filter["source"] = Value::Null;
                filter["kind"] = json!("all");
                filter["count_only"] = json!(true);
                let result: Value =
                    serde_json::from_str(&self.query_items_db(&db, filter)?).map_err(err)?;
                counts.insert(key, result["total"].clone());
            }
            return serde_json::to_string(&counts).map_err(err);
        }
        self.query_items(q)
    }
    /// Retention cleanup is maintenance work. Running it before every keystroke
    /// turns a search into a full write transaction, so keep the query path
    /// bounded and let the explicit maintenance call do the immediate cleanup.
    fn maybe_prune(&self) -> Result<()> {
        let now = now_ms();
        let last = self.last_prune_ms.load(Ordering::Acquire);
        if now.saturating_sub(last) < 60_000 {
            return Ok(());
        }
        if self
            .last_prune_ms
            .compare_exchange(last, now, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            && let Err(error) = self.prune()
        {
            self.last_prune_ms.store(0, Ordering::Release);
            return Err(error);
        }
        Ok(())
    }
    fn state(&self) -> Result<String> {
        let generation = self.generation();
        if let Some((cached_generation, value)) = self.state_cache.lock().map_err(err)?.as_ref()
            && *cached_generation == generation
        {
            return Ok(value.clone());
        }
        let db = self.db.lock().map_err(err)?;
        let lists = lists_db(&db)?;
        let settings = settings_db(&db)?;
        let mut apps = std::collections::BTreeMap::new();
        let mut devices = std::collections::BTreeMap::new();
        let mut stmt=db.prepare("SELECT DISTINCT json_extract(summary,'$.source_app_id'),json_extract(summary,'$.source_app_name'),json_extract(summary,'$.source_device_id'),json_extract(summary,'$.source') FROM cl_items").map_err(err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(err)?;
        for row in rows {
            let (a, n, d, s) = row.map_err(err)?;
            if let Some(a) = a {
                apps.insert(
                    a.clone(),
                    json!({"bundle_id":a,"name":n.unwrap_or_else(||a.clone())}),
                );
            }
            if let Some(d) = d {
                devices.insert(d.clone(), json!({"id":d,"name":s}));
            }
        }
        let value = serde_json::to_string(&json!({"lists":lists,"settings":settings,"apps":apps.values().collect::<Vec<_>>(),"devices":devices.values().collect::<Vec<_>>(),"revision":generation})).map_err(err)?;
        *self.state_cache.lock().map_err(err)? = Some((generation, value.clone()));
        Ok(value)
    }
    fn query_items(&self, q: Value) -> Result<String> {
        let db = self.db.lock().map_err(err)?;
        self.query_items_db(&db, q)
    }

    fn query_items_db(&self, db: &Connection, q: Value) -> Result<String> {
        let count_only = q["count_only"].as_bool().unwrap_or(false);
        let list_id = q["list_id"].as_str();
        let lists = lists_db(db)?;
        let selected = if let Some(id) = list_id {
            Some(lists.iter().find(|l| l.id == id).ok_or("列表已删除")?)
        } else {
            None
        };
        let query = q["query"].as_str().unwrap_or("").trim().to_lowercase();
        let kind = q["kind"].as_str().unwrap_or("all");
        let source = q["source"].as_str();
        // Twenty rows are enough to fill the visible history and keep OCR/title
        // strings bounded in the client. Callers may request a smaller page.
        let offset = q["offset"].as_u64().unwrap_or(0).min(100_000_000) as usize;
        let limit = q["limit"].as_u64().unwrap_or(20).clamp(1, 100) as usize;
        let cursor_sort_at_ms = q["cursor"]["sort_at_ms"]
            .as_u64()
            .or_else(|| q["cursor_sort_at_ms"].as_u64());
        let cursor_id = q["cursor"]["id"]
            .as_u64()
            .or_else(|| q["cursor_id"].as_u64());
        let cursor = cursor_sort_at_ms.zip(cursor_id);
        let manual = selected.is_some_and(|list| list.kind == "manual");
        // These two cases can be answered entirely by SQLite. Smart-list rules
        // still need Summary decoding, but the iterator remains streaming and
        // never builds a 100k-row Rust collection.
        let fast_path = (selected.is_none() || manual) && kind == "all";

        let mut clauses: Vec<String> = Vec::with_capacity(5);
        let mut filter_params: Vec<SqlValue> = Vec::with_capacity(8);
        if manual {
            // A manual list shows the frozen archive when one exists, and its
            // membership key is indexed by the primary key of cl_records.
            clauses.push("NOT (i.ordinary=1 AND EXISTS(SELECT 1 FROM cl_items a WHERE a.global_id='archive:' || i.global_id AND a.ordinary=0))".into());
            clauses.push("EXISTS(SELECT 1 FROM cl_records m WHERE m.key='member:' || ? || ':' || CASE WHEN i.ordinary=1 THEN 'archive:' || i.global_id ELSE i.global_id END AND m.deleted=0)".into());
            clauses.push("NOT EXISTS(SELECT 1 FROM cl_records tombstone WHERE tombstone.key='item:' || CASE WHEN i.ordinary=1 THEN 'archive:' || i.global_id ELSE i.global_id END AND tombstone.deleted=1)".into());
            filter_params.push(SqlValue::Text(list_id.unwrap_or_default().to_string()));
        } else {
            clauses.push(UNIQUE_IMAGE_ROW.into());
            // An archive with a live ordinary source is hidden outside manual
            // lists. The old per-row SELECT EXISTS is now one indexed predicate.
            clauses.push("NOT (i.ordinary=0 AND i.global_id LIKE 'archive:%' AND EXISTS(SELECT 1 FROM cl_items base WHERE base.global_id=substr(i.global_id,9) AND base.ordinary=1))".into());
            // Orphaned archive rows are never shown. Restrict the membership
            // probe to active list IDs so deleted-list tombstones stay hidden
            // without materializing every membership in Rust.
            if lists.is_empty() {
                clauses.push("i.ordinary=1".into());
            } else {
                let active_memberships = lists
                    .iter()
                    .map(|_| {
                        "m.key='member:' || ? || ':' || CASE WHEN i.ordinary=1 THEN 'archive:' || i.global_id ELSE i.global_id END"
                    })
                    .collect::<Vec<_>>()
                    .join(" OR ");
                clauses.push(format!(
                    "(i.ordinary=1 OR EXISTS(SELECT 1 FROM cl_records m WHERE m.deleted=0 AND ({active_memberships})))"
                ));
                filter_params.extend(lists.iter().map(|list| SqlValue::Text(list.id.clone())));
            }
        }
        if let Some(source) = source {
            clauses.push("(json_extract(i.summary,'$.source_device_id')=? OR json_extract(i.summary,'$.source')=?)".into());
            filter_params.push(SqlValue::Text(source.to_string()));
            filter_params.push(SqlValue::Text(source.to_string()));
        }
        if !query.is_empty() {
            if let Some(fts_query) = fts_query(&query) {
                clauses.push(
                    "i.id IN (SELECT rowid FROM cl_items_fts WHERE cl_items_fts MATCH ?)".into(),
                );
                filter_params.push(SqlValue::Text(fts_query));
                // Trigram candidates can contain the same trigrams in a
                // different order. Keep the original substring predicate as
                // the exact, low-memory verification step.
                clauses.push("i.search_index LIKE '%' || ? || '%' COLLATE NOCASE".into());
                filter_params.push(SqlValue::Text(query.clone()));
            } else {
                clauses.push("i.search_index LIKE '%' || ? || '%' COLLATE NOCASE".into());
                filter_params.push(SqlValue::Text(query.clone()));
            }
        }
        let where_sql = clauses.join(" AND ");

        if fast_path {
            let count_sql = format!("SELECT COUNT(*) FROM cl_items i WHERE {where_sql}");
            let total: usize = db
                .query_row(&count_sql, params_from_iter(filter_params.iter()), |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(err)?
                .max(0) as usize;

            if count_only {
                return Ok(json!({"total": total}).to_string());
            }
            let mut page_clauses = clauses;
            let mut page_params = filter_params;
            if let Some((sort_at_ms, id)) = cursor {
                page_clauses.push(
                    "(MAX(COALESCE(i.last_used,0),COALESCE(i.copied,i.imported))<? OR (MAX(COALESCE(i.last_used,0),COALESCE(i.copied,i.imported))=? AND i.id<?))".into(),
                );
                page_params.push(SqlValue::Integer(sort_at_ms.min(i64::MAX as u64) as i64));
                page_params.push(SqlValue::Integer(sort_at_ms.min(i64::MAX as u64) as i64));
                page_params.push(SqlValue::Integer(id.min(i64::MAX as u64) as i64));
            }
            let page_sql = format!(
                "{QUERY_SELECT} WHERE {} ORDER BY {QUERY_ORDER} LIMIT ? OFFSET ?",
                page_clauses.join(" AND ")
            );
            page_params.push(SqlValue::Integer((limit + 1) as i64));
            page_params.push(SqlValue::Integer(
                if cursor.is_some() { 0 } else { offset } as i64
            ));
            let mut stmt = db.prepare(&page_sql).map_err(err)?;
            let rows = stmt
                .query_map(params_from_iter(page_params.iter()), QueryRow::from_row)
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)?;
            let has_more = rows.len() > limit;
            let mut page = rows;
            page.truncate(limit);
            let memberships = memberships_for(db, &page, &lists)?;
            let items = page
                .iter()
                .map(|row| {
                    self.query_item_value(
                        row,
                        memberships
                            .get(row.member_id())
                            .cloned()
                            .unwrap_or_default(),
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            let next_cursor = if has_more {
                page.last()
                    .map(|row| json!({"sort_at_ms": row.sort_at_ms.max(0) as u64, "id": row.id}))
            } else {
                None
            };
            return serde_json::to_string(&json!({
                "items": items,
                "total": total,
                "next_offset": has_more.then_some(offset.saturating_add(limit)),
                "next_cursor": next_cursor,
                "revision": self.generation()
            }))
            .map_err(err);
        }

        // Smart lists have arbitrary rule combinations. Keep their existing
        // semantics, but stream the SQL rows and apply cursor/offset after the
        // rule match so `total` remains the full result count.
        let sql = format!("{QUERY_SELECT} WHERE {where_sql} ORDER BY {QUERY_ORDER}");
        let mut stmt = db.prepare(&sql).map_err(err)?;
        let rows = stmt
            .query_map(params_from_iter(filter_params.iter()), QueryRow::from_row)
            .map_err(err)?;
        let mut total = 0usize;
        let mut after_cursor = cursor.is_none();
        let mut visible_count = 0usize;
        let page_offset = if cursor.is_some() { 0 } else { offset };
        let mut page: Vec<(QueryRow, Summary)> = Vec::with_capacity(limit);
        let mut has_more = false;
        for row in rows {
            let row = row.map_err(err)?;
            let mut summary: Summary = serde_json::from_str(&row.summary).map_err(err)?;
            project_image_file(&mut summary);
            if kind != "all" && kind != summary.kind {
                continue;
            }
            let title_lower = row.title.to_lowercase();
            let ocr_lower = row.ocr_text.to_lowercase();
            if let Some(list) = selected {
                let matches = |(index, rule): (usize, &Rule)| {
                    matches_rule(
                        index,
                        rule,
                        &summary,
                        &row.search_text,
                        &title_lower,
                        &ocr_lower,
                        &q,
                    )
                };
                let matches = if list.match_mode == "any" {
                    list.rules.iter().enumerate().any(matches)
                } else {
                    list.rules.iter().enumerate().all(matches)
                };
                if !matches {
                    continue;
                }
            }
            total = total.saturating_add(1);
            if count_only {
                continue;
            }
            if let Some((cursor_sort, cursor_row_id)) = cursor
                && !after_cursor
            {
                let row_sort = row.sort_at_ms.max(0) as u64;
                if row_sort < cursor_sort || (row_sort == cursor_sort && row.id < cursor_row_id) {
                    after_cursor = true;
                } else {
                    continue;
                }
            }
            if visible_count < page_offset {
                visible_count += 1;
                continue;
            }
            if page.len() < limit {
                page.push((row, summary));
                visible_count += 1;
            } else {
                has_more = true;
            }
        }
        if count_only {
            return Ok(json!({"total": total}).to_string());
        }
        let page_rows: Vec<QueryRow> = page.iter().map(|(row, _)| row.clone()).collect();
        let memberships = memberships_for(db, &page_rows, &lists)?;
        let items = page
            .into_iter()
            .map(|(row, mut summary)| {
                let member = memberships
                    .get(row.member_id())
                    .cloned()
                    .unwrap_or_default();
                query_item_value_with_summary(&row, &mut summary, member)
            })
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = if has_more {
            page_rows
                .last()
                .map(|row| json!({"sort_at_ms": row.sort_at_ms.max(0) as u64, "id": row.id}))
        } else {
            None
        };
        serde_json::to_string(&json!({
            "items": items,
            "total": total,
            "next_offset": has_more.then_some(offset.saturating_add(limit)),
            "next_cursor": next_cursor,
            "revision": self.generation()
        }))
        .map_err(err)
    }

    fn query_item_value(&self, row: &QueryRow, memberships: Vec<String>) -> Result<Value> {
        let mut summary: Summary = serde_json::from_str(&row.summary).map_err(err)?;
        project_image_file(&mut summary);
        query_item_value_with_summary(row, &mut summary, memberships)
    }
    pub fn mutate(&self, request: &str) -> Result<String> {
        let q: Value = serde_json::from_str(request).map_err(err)?;
        match q["op"].as_str().unwrap_or("") {
            "settings" => {
                let mut current = serde_json::to_value(self.settings()?).map_err(err)?;
                for (k, v) in q["settings"].as_object().ok_or("设置无效")? {
                    current[k] = v.clone()
                }
                let mut s: Settings = serde_json::from_value(current).map_err(err)?;
                if s.retention_days == 0 {
                    return Err("保留天数必须大于零".into());
                }
                s.excluded_apps
                    .sort_by(|a, b| a.bundle_id.cmp(&b.bundle_id));
                s.excluded_apps.dedup_by(|a, b| a.bundle_id == b.bundle_id);
                self.db.lock().map_err(err)?.execute("INSERT INTO cl_meta VALUES('settings',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[serde_json::to_string(&s).map_err(err)?]).map_err(err)?;
                self.changed();
                // Re-enabling index sync re-materializes already-stored records.
                if s.sync_index {
                    self.reconcile_index()?;
                }
            }
            "save_list" => {
                let mut value = q["list"].clone();
                if value["id"].as_str().is_none_or(str::is_empty) {
                    value["id"] = json!(uuid::Uuid::new_v4().to_string())
                }
                let l: List = serde_json::from_value(value).map_err(err)?;
                validate_list(&l)?;
                self.local_record(
                    format!("list:{}", l.id),
                    serde_json::to_value(l).map_err(err)?,
                    false,
                )?;
            }
            "delete_list" => {
                let id = q["id"].as_str().ok_or("列表 ID 无效")?;
                self.local_record(format!("list:{id}"), Value::Null, true)?;
            }
            "reorder" => {
                let ids = q["ids"].as_array().ok_or("列表顺序无效")?;
                let lists = {
                    let db = self.db.lock().map_err(err)?;
                    lists_db(&db)?
                };
                for (i, id) in ids.iter().enumerate() {
                    if let Some(l) = lists.iter().find(|l| Some(l.id.as_str()) == id.as_str()) {
                        let mut l = l.clone();
                        l.position = i as i64;
                        self.local_record(
                            format!("list:{}", l.id),
                            serde_json::to_value(l).map_err(err)?,
                            false,
                        )?;
                    }
                }
            }
            "membership" => {
                let list = q["list_id"].as_str().ok_or("列表 ID 无效")?;
                let id = q["history_id"].as_u64().ok_or("内容 ID 无效")?;
                let present = q["present"].as_bool().ok_or("成员操作无效")?;
                self.membership(list, id, present)?;
            }
            "delete_items" => {
                let ids: Vec<i64> = q["ids"]
                    .as_array()
                    .ok_or("内容 ID 无效")?
                    .iter()
                    .filter_map(|v| v.as_u64())
                    .map(|v| v as i64)
                    .collect();
                if ids.is_empty() {
                    return Err("内容 ID 无效".into());
                }
                // Frozen manual-list archives share the row's global id and must
                // be removed with it; their sync record gets a tombstone so a
                // merge cannot resurrect the archive.
                let mut archived = std::collections::BTreeSet::new();
                let mut bases = std::collections::BTreeSet::new();
                {
                    let db = self.db.lock().map_err(err)?;
                    for id in ids {
                        let row: Option<(String, bool)> = db
                            .query_row(
                                "SELECT global_id,ordinary FROM cl_items WHERE id=?1",
                                [id],
                                |r| Ok((r.get(0)?, r.get(1)?)),
                            )
                            .optional()
                            .map_err(err)?;
                        let Some((global_id, ordinary)) = row else {
                            continue;
                        };
                        let base = global_id
                            .strip_prefix("archive:")
                            .unwrap_or(&global_id)
                            .to_string();
                        let has_archive = !ordinary
                            || db
                                .query_row(
                                    "SELECT EXISTS(SELECT 1 FROM cl_items WHERE global_id=?1)",
                                    [format!("archive:{base}")],
                                    |r| r.get::<_, bool>(0),
                                )
                                .map_err(err)?;
                        if has_archive {
                            archived.insert(format!("archive:{base}"));
                        }
                        bases.insert(base);
                    }
                }
                // Persist valid archive tombstones before removing local rows. A
                // rejected operation must not leave deleted data behind a cached state.
                for global_id in archived {
                    let members = {
                        let db = self.db.lock().map_err(err)?;
                        let mut stmt = db.prepare(
                            "SELECT key,value FROM cl_records WHERE key GLOB 'member:*' AND deleted=0 AND json_extract(value,'$.item_id')=?1"
                        ).map_err(err)?;
                        stmt.query_map([&global_id], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                        })
                        .map_err(err)?
                        .collect::<std::result::Result<Vec<_>, _>>()
                        .map_err(err)?
                    };
                    for (key, value) in members {
                        self.local_record(key, serde_json::from_str(&value).map_err(err)?, true)?;
                    }
                    self.local_record(format!("item:{global_id}"), json!({"id": global_id}), true)?;
                }
                {
                    let mut db = self.db.lock().map_err(err)?;
                    let tx = db.transaction().map_err(err)?;
                    for base in bases {
                        tx.execute(
                            "DELETE FROM cl_items WHERE global_id=?1 OR global_id=?2",
                            params![base, format!("archive:{base}")],
                        )
                        .map_err(err)?;
                    }
                    tx.commit().map_err(err)?;
                }
                self.changed();
            }
            "set_item_title" => {
                let id = q["id"].as_u64().ok_or("内容 ID 无效")? as i64;
                let title: String = q["title"]
                    .as_str()
                    .unwrap_or("")
                    .trim()
                    .chars()
                    .take(200)
                    .collect();
                let stored = if title.is_empty() { None } else { Some(title) };
                let db = self.db.lock().map_err(err)?;
                let updated = db
                    .execute(
                        "UPDATE cl_items SET title=?1 WHERE id=?2",
                        params![stored, id],
                    )
                    .map_err(err)?;
                drop(db);
                if updated == 0 {
                    return Err("内容不存在".into());
                }
                self.changed();
                self.publish_index_for(id as u64)?;
            }
            "set_item_ocr" => {
                let id = q["id"].as_u64().ok_or("内容 ID 无效")? as i64;
                let text: String = q["text"]
                    .as_str()
                    .unwrap_or("")
                    .trim()
                    .chars()
                    .take(20_000)
                    .collect();
                let version = q["version"].as_u64().unwrap_or(0) as i64;
                let db = self.db.lock().map_err(err)?;
                let updated = db
                    .execute(
                        "UPDATE cl_items SET ocr_text=?1,ocr_version=?2 WHERE id=?3",
                        params![if text.is_empty() { None } else { Some(text) }, version, id],
                    )
                    .map_err(err)?;
                drop(db);
                if updated == 0 {
                    return Err("内容不存在".into());
                }
                self.changed();
                self.publish_index_for(id as u64)?;
            }
            "want_item_payload" => {
                let id = q["id"].as_u64().ok_or("内容 ID 无效")? as i64;
                let hash: String = {
                    let db = self.db.lock().map_err(err)?;
                    db.query_row(
                        "SELECT COALESCE(blob_hash,'') FROM cl_items WHERE id=?1",
                        [id],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(err)?
                    .unwrap_or_default()
                };
                if !hash.is_empty() {
                    self.want_blob(&hash)?;
                }
            }
            "clear_remote_index" => {
                let db = self.db.lock().map_err(err)?;
                db.execute("DELETE FROM cl_items WHERE remote=1", [])
                    .map_err(err)?;
                drop(db);
                self.changed();
            }
            _ => return Err("未知剪贴板操作".into()),
        }
        self.state()
    }
    fn membership(&self, list: &str, id: u64, present: bool) -> Result<()> {
        let (gid, summary, payload, ordinary) = {
            let db = self.db.lock().map_err(err)?;
            if !lists_db(&db)?
                .iter()
                .any(|l| l.id == list && l.kind == "manual")
            {
                return Err("只能加入手动列表".into());
            }
            db.query_row(
                "SELECT global_id,summary,payload,ordinary FROM cl_items WHERE id=?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<Vec<u8>>>(2)?,
                        r.get::<_, bool>(3)?,
                    ))
                },
            )
            .map_err(err)?
        };
        let gid = if ordinary {
            format!("archive:{gid}")
        } else {
            gid
        };
        let existing_archive = if present {
            let db = self.db.lock().map_err(err)?;
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM cl_items WHERE global_id=?1 AND ordinary=0)",
                [&gid],
                |row| row.get::<_, bool>(0),
            )
            .map_err(err)?
        } else {
            false
        };
        let mut archive_blob = None;
        // Reuse the frozen archive across lists. A refreshed ordinary row can
        // have a different app/date; it must not replace already pinned content.
        // A metadata-only remote archive can join another list while its original
        // blob is still syncing, without being overwritten by local history.
        if present && !existing_archive {
            // A short synced text is complete in its summary; freeze that text
            // instead of failing until the richer payload transfer arrives.
            let snapshot = match payload {
                Some(bytes) => decode(&bytes)?,
                None => {
                    let summary: Summary = serde_json::from_str(&summary).map_err(err)?;
                    reconstructed_text_snapshot(&summary).ok_or("内容尚未同步")?
                }
            };
            let device = self.device.lock().map_err(err)?.clone();
            let snapshot = crate::library_files::register(&snapshot, &device)?;
            let bytes = encode(&snapshot, "归档")?;
            let hash = hex::encode(Sha256::digest(&bytes));
            self.write_blob(&hash, &bytes)?;
            let summary: Value = serde_json::from_str(&summary).map_err(err)?;
            self.local_record(
                format!("item:{gid}"),
                json!({"id":gid,"summary":summary,"blob_hash":hash}),
                false,
            )?;
            archive_blob = Some((hash, bytes));
        }
        self.local_record(
            format!("member:{list}:{gid}"),
            json!({"list_id":list,"item_id":gid}),
            !present,
        )?;
        // A former remove may be reconciled between the item and causal re-add.
        // Hydrate again after the membership exists so re-adding cannot lose data.
        if let Some((hash, bytes)) = archive_blob {
            self.write_blob(&hash, &bytes)?;
        }
        Ok(())
    }
    pub fn export_operations(&self, after: u64, limit: usize) -> Result<Vec<Value>> {
        let db = self.db.lock().map_err(err)?;
        let mut stmt = db
            .prepare("SELECT seq,body FROM cl_operations WHERE seq>?1 ORDER BY seq LIMIT ?2")
            .map_err(err)?;
        let rows = stmt
            .query_map(params![after, limit.min(100)], |r| {
                Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(err)?;
        rows.map(|r| {
            let (seq, raw) = r.map_err(err)?;
            let mut v: Value = serde_json::from_str(&raw).map_err(err)?;
            v["seq"] = json!(seq);
            Ok(v)
        })
        .collect()
    }
    fn local_record(&self, key: String, value: Value, deleted: bool) -> Result<()> {
        let author = self.device.lock().map_err(err)?.clone();
        if author.is_empty() {
            return Err("设备身份尚未就绪".into());
        }
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        let previous: u64 = tx
            .query_row(
                "SELECT COALESCE(MAX(json_extract(body,'$.revision')),0) FROM cl_operations",
                [],
                |r| r.get(0),
            )
            .map_err(err)?;
        let local: u64 = tx
            .query_row("SELECT value FROM cl_meta WHERE key='counter'", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .map_err(err)?
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let revision = previous
            .max(local)
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or("列表版本已溢出")?;
        tx.execute("INSERT INTO cl_meta VALUES('counter',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[revision.to_string()]).map_err(err)?;
        let mut clock = std::collections::BTreeMap::<String, u64>::new();
        {
            let mut stmt=tx.prepare("SELECT json_extract(body,'$.author'),MAX(json_extract(body,'$.revision')) FROM cl_operations GROUP BY json_extract(body,'$.author')").map_err(err)?;
            for row in stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))
                .map_err(err)?
            {
                let (a, r) = row.map_err(err)?;
                clock.insert(a, r);
            }
        }
        clock.insert(author.clone(), revision);
        tx.commit().map_err(err)?;
        drop(db);
        self.merge_operations(vec![json!({"op_id":uuid::Uuid::new_v4().to_string(),"key":key,"revision":revision,"author":author,"clock":clock,"deleted":deleted,"value":value})])
    }

    pub fn merge_operations(&self, ops: Vec<Value>) -> Result<()> {
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        let sync_index = settings_db(&tx)?.sync_index;
        let mut changed = false;
        let mut needs_reconcile = false;
        for mut op in ops {
            op.as_object_mut().ok_or("同步操作无效")?.remove("seq");
            let op_id = op["op_id"].as_str().ok_or("同步操作缺少ID")?;
            let key = op["key"].as_str().ok_or("同步操作缺少key")?;
            let revision = op["revision"]
                .as_u64()
                .filter(|v| *v <= i64::MAX as u64)
                .ok_or("同步版本无效")?;
            let author = op["author"].as_str().ok_or("同步作者无效")?;
            let deleted = op["deleted"].as_bool().ok_or("同步删除标记无效")?;
            let value = &op["value"];
            if key.starts_with("list:") && !deleted {
                let list = serde_json::from_value::<List>(value.clone()).map_err(err)?;
                validate_list(&list)?;
                if key != format!("list:{}", list.id) {
                    return Err("列表记录身份不一致".into());
                }
            }
            if !["list:", "member:", "item:", "index:"]
                .iter()
                .any(|p| key.starts_with(p))
            {
                return Err("同步记录类型无效".into());
            }
            if key.starts_with("item:")
                && value["id"]
                    .as_str()
                    .is_none_or(|id| key != format!("item:{id}"))
            {
                return Err("归档记录身份不一致".into());
            }
            if key.starts_with("index:")
                && value["id"]
                    .as_str()
                    .is_none_or(|id| key != format!("index:{id}"))
            {
                return Err("索引记录身份不一致".into());
            }
            let inserted = tx
                .execute(
                    "INSERT OR IGNORE INTO cl_operations(op_id,body) VALUES(?1,?2)",
                    params![op_id, serde_json::to_string(&op).map_err(err)?],
                )
                .map_err(err)?;
            if inserted == 0 {
                continue;
            }
            changed = true;
            let previous: Option<(u64, String, bool)> = tx
                .query_row(
                    "SELECT revision,author,deleted FROM cl_records WHERE key=?1",
                    [key],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(err)?;
            let clock: std::collections::BTreeMap<String, u64> =
                serde_json::from_value(op["clock"].clone())
                    .map_err(|_| "同步操作缺少因果版本".to_string())?;
            if clock.get(author) != Some(&revision) {
                return Err("同步因果版本无效".into());
            }
            if key.starts_with("member:") {
                let list_id = value["list_id"].as_str().ok_or("成员列表 ID 无效")?;
                let item_id = value["item_id"].as_str().ok_or("成员内容 ID 无效")?;
                if key != format!("member:{list_id}:{item_id}") {
                    return Err("成员记录身份不一致".into());
                }
                // A single winner is insufficient: a later third operation can
                // be concurrent with a previously losing removal. Recompute the
                // causal frontier from durable operations, then remove wins only
                // among concurrent maxima; a causally newer add may restore it.
                let winner = member_winner(&tx, key)?;
                tx.execute("INSERT INTO cl_clocks VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, serde_json::to_string(&winner["clock"]).map_err(err)?]).map_err(err)?;
                tx.execute("INSERT INTO cl_records VALUES(?1,?2,?3,?4,?5) ON CONFLICT(key) DO UPDATE SET revision=excluded.revision,author=excluded.author,deleted=excluded.deleted,value=excluded.value",
                    params![key, winner["revision"].as_u64(), winner["author"].as_str(), winner["deleted"].as_bool(), serde_json::to_string(&winner["value"]).map_err(err)?]).map_err(err)?;
                needs_reconcile = true;
                continue;
            }
            let old_clock: std::collections::BTreeMap<String, u64> = tx
                .query_row("SELECT value FROM cl_clocks WHERE key=?1", [key], |r| {
                    r.get::<_, String>(0)
                })
                .optional()
                .map_err(err)?
                .map(|s| serde_json::from_str(&s).map_err(err))
                .transpose()?
                .unwrap_or_default();
            let dominates =
                |a: &std::collections::BTreeMap<String, u64>,
                 b: &std::collections::BTreeMap<String, u64>| {
                    b.iter().all(|(k, v)| a.get(k).unwrap_or(&0) >= v)
                };
            let concurrent = !dominates(&clock, &old_clock) && !dominates(&old_clock, &clock);
            let accept = previous.as_ref().is_none_or(|(r, a, d)| {
                if key.starts_with("list:") && *d {
                    return false;
                }
                if key.starts_with("list:") && deleted {
                    return true;
                }
                if key.starts_with("member:") && concurrent && deleted != *d {
                    return deleted;
                }
                (revision, author) > (*r, a.as_str())
            });

            if accept {
                needs_reconcile |= key.starts_with("item:") || key.starts_with("list:");
                tx.execute("INSERT INTO cl_clocks VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,serde_json::to_string(&clock).map_err(err)?]).map_err(err)?;
                tx.execute("INSERT INTO cl_records VALUES(?1,?2,?3,?4,?5) ON CONFLICT(key) DO UPDATE SET revision=excluded.revision,author=excluded.author,deleted=excluded.deleted,value=excluded.value",params![key,revision,author,deleted,serde_json::to_string(value).map_err(err)?]).map_err(err)?;
                if key.starts_with("item:") && !deleted {
                    materialize(&tx, value)?;
                }
                if key.starts_with("index:") && !deleted && sync_index {
                    materialize_index(&tx, value, author)?;
                }
            }
        }
        if needs_reconcile {
            reconcile_archives(&tx)?;
        }
        tx.commit().map_err(err)?;
        drop(db);
        if changed {
            self.changed()
        }
        Ok(())
    }
    pub fn read_blob(&self, id: &str) -> Result<Option<Vec<u8>>> {
        let db = self.db.lock().map_err(err)?;
        let archived: Option<Vec<u8>> = db
            .query_row("SELECT bytes FROM cl_blobs WHERE hash=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(err)?;
        if archived.is_some() {
            return Ok(archived);
        }
        // Ordinary items keep their encoded snapshot in cl_items rather than a
        // second copy in cl_blobs; serve those for on-demand peer fetches.
        db.query_row(
            "SELECT payload FROM cl_items WHERE blob_hash=?1 AND payload IS NOT NULL LIMIT 1",
            [id],
            |r| r.get::<_, Option<Vec<u8>>>(0),
        )
        .optional()
        .map_err(err)
        .map(Option::flatten)
    }
    pub fn write_blob(&self, id: &str, bytes: &[u8]) -> Result<()> {
        if hex::encode(Sha256::digest(bytes)) != id {
            return Err("归档内容校验失败".into());
        }
        let _ = decode(bytes)?;
        let db = self.db.lock().map_err(err)?;
        db.execute(
            "INSERT OR IGNORE INTO cl_blobs VALUES(?1,?2)",
            params![id, bytes],
        )
        .map_err(err)?;
        db.execute(
            "UPDATE cl_items SET payload=?1,search_text=?3 WHERE blob_hash=?2",
            params![
                bytes,
                id,
                decode(bytes)?.text.unwrap_or_default().to_lowercase()
            ],
        )
        .map_err(err)?;
        drop(db);
        self.changed();
        Ok(())
    }

    /// Queue a content hash for on-demand fetch from a paired device. Ordinary
    /// history index rows are never fetched eagerly.
    pub fn want_blob(&self, hash: &str) -> Result<()> {
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("内容摘要无效".into());
        }
        let db = self.db.lock().map_err(err)?;
        let raw: Option<String> = db
            .query_row(
                "SELECT value FROM cl_meta WHERE key='wanted_blobs'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        let mut wanted: Vec<String> = raw
            .as_deref()
            .and_then(|value| serde_json::from_str(value).ok())
            .unwrap_or_default();
        if !wanted.iter().any(|h| h == hash) {
            wanted.push(hash.to_string());
        }
        if wanted.len() > 256 {
            let excess = wanted.len() - 256;
            wanted.drain(0..excess);
        }
        db.execute(
            "INSERT INTO cl_meta VALUES('wanted_blobs',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [serde_json::to_string(&wanted).map_err(err)?],
        )
        .map_err(err)?;
        Ok(())
    }

    /// Take and clear the pending fetch list. Called by the library sync worker
    /// on its tick; clearing avoids re-requesting the same blob every tick.
    pub fn take_wanted_blobs(&self) -> Result<Vec<String>> {
        let db = self.db.lock().map_err(err)?;
        let raw: Option<String> = db
            .query_row(
                "SELECT value FROM cl_meta WHERE key='wanted_blobs'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        let Some(raw) = raw else {
            return Ok(Vec::new());
        };
        db.execute("DELETE FROM cl_meta WHERE key='wanted_blobs'", [])
            .map_err(err)?;
        Ok(serde_json::from_str(&raw).unwrap_or_default())
    }
}
fn fts_query(query: &str) -> Option<String> {
    let chars: Vec<char> = query.chars().collect();
    if chars.len() < 3 {
        return None;
    }
    let mut seen = HashSet::new();
    let mut terms = Vec::with_capacity(chars.len() - 2);
    for window in chars.windows(3) {
        let term: String = window.iter().collect();
        if seen.insert(term.clone()) {
            // detail=none FTS5 indexes individual trigram tokens but does not
            // support phrase queries. Quoted single-token terms keep punctuation
            // and whitespace literal while the SQL LIKE below verifies order.
            terms.push(format!("\"{}\"", term.replace('\"', "\"\"")));
        }
    }
    (!terms.is_empty()).then(|| terms.join(" AND "))
}

fn query_item_value_with_summary(
    row: &QueryRow,
    summary: &mut Summary,
    memberships: Vec<String>,
) -> Result<Value> {
    let available = row.available || reconstructed_text_snapshot(summary).is_some();
    let mut value = serde_json::to_value(summary).map_err(err)?;
    value["id"] = json!(row.id);
    value["available"] = json!(available);
    value["memberships"] = json!(memberships);
    value["title"] = json!(row.title);
    value["ocr_text"] = json!(row.ocr_text);
    value["ocr_version"] = json!(row.ocr_version.max(0) as u64);
    value["remote"] = json!(row.remote);
    // Same key the page is ordered by, so the client can interleave snapshot
    // rows without reordering the returned page.
    value["sort_at_ms"] = json!(row.sort_at_ms.max(0) as u64);
    Ok(value)
}

fn memberships_for(
    db: &Connection,
    rows: &[QueryRow],
    lists: &[List],
) -> Result<HashMap<String, Vec<String>>> {
    if rows.is_empty() {
        return Ok(HashMap::new());
    }
    let active: HashSet<&str> = lists.iter().map(|list| list.id.as_str()).collect();
    let mut result: HashMap<String, Vec<String>> = HashMap::new();
    let item_ids: Vec<String> = rows
        .iter()
        .map(QueryRow::member_id)
        .map(str::to_owned)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    // One SQLite scan for the page is cheaper than one full membership scan per
    // row. The returned values are still bounded by the page's item IDs.
    let predicates = item_ids
        .iter()
        .map(|_| "key LIKE ?")
        .collect::<Vec<_>>()
        .join(" OR ");
    let sql = format!(
        "SELECT value FROM cl_records WHERE key GLOB 'member:*' AND deleted=0 AND ({predicates}) AND NOT EXISTS(SELECT 1 FROM cl_records tombstone WHERE tombstone.key='item:' || json_extract(cl_records.value,'$.item_id') AND tombstone.deleted=1)"
    );
    let params = item_ids
        .iter()
        .map(|item_id| SqlValue::Text(format!("member:%:{item_id}")))
        .collect::<Vec<_>>();
    let mut stmt = db.prepare(&sql).map_err(err)?;
    for raw in stmt
        .query_map(params_from_iter(params.iter()), |value| {
            value.get::<_, String>(0)
        })
        .map_err(err)?
    {
        let value: Value = serde_json::from_str(&raw.map_err(err)?).map_err(err)?;
        let Some(list_id) = value["list_id"].as_str() else {
            continue;
        };
        let Some(member_id) = value["item_id"].as_str() else {
            continue;
        };
        if active.contains(list_id) {
            result
                .entry(member_id.to_string())
                .or_default()
                .push(list_id.to_string());
        }
    }
    Ok(result)
}

fn settings_db(db: &Connection) -> Result<Settings> {
    let raw: Option<String> = db
        .query_row("SELECT value FROM cl_meta WHERE key='settings'", [], |r| {
            r.get(0)
        })
        .optional()
        .map_err(err)?;
    raw.map(|r| serde_json::from_str(&r).map_err(err))
        .unwrap_or_else(|| Ok(Settings::default()))
}
fn lists_db(db: &Connection) -> Result<Vec<List>> {
    let mut stmt = db
        .prepare("SELECT value FROM cl_records WHERE key GLOB 'list:*' AND deleted=0")
        .map_err(err)?;
    let mut lists = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(err)?
        .map(|r| serde_json::from_str::<List>(&r.map_err(err)?).map_err(err))
        .collect::<Result<Vec<_>>>()?;
    lists.sort_by(|a, b| (a.position, &a.id).cmp(&(b.position, &b.id)));
    Ok(lists)
}
fn members_db(db: &Connection) -> Result<Vec<(String, String)>> {
    let lists = lists_db(db)?;
    let mut stmt = db
        .prepare("SELECT value FROM cl_records WHERE key GLOB 'member:*' AND deleted=0")
        .map_err(err)?;
    let mut out = vec![];
    for raw in stmt.query_map([], |r| r.get::<_, String>(0)).map_err(err)? {
        let v: Value = serde_json::from_str(&raw.map_err(err)?).map_err(err)?;
        if let (Some(l), Some(i)) = (v["list_id"].as_str(), v["item_id"].as_str())
            && lists.iter().any(|s| s.id == l)
        {
            out.push((l.into(), i.into()));
        }
    }
    Ok(out)
}
fn materialize(db: &Connection, v: &Value) -> Result<()> {
    let gid = v["id"].as_str().ok_or("归档ID无效")?;
    let hash = v["blob_hash"].as_str().ok_or("归档摘要无效")?;
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("归档摘要无效".into());
    }
    let mut s: Summary = serde_json::from_value(v["summary"].clone()).map_err(err)?;
    let existing: Option<(u64, Option<String>, bool)> = db
        .query_row(
            "SELECT id,blob_hash,payload IS NOT NULL FROM cl_items WHERE global_id=?1",
            [gid],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(err)?;
    if let Some((id, Some(existing_hash), true)) = existing
        && existing_hash == hash
    {
        s.id = id;
        db.execute(
            "UPDATE cl_items SET summary=?1,copied=?2 WHERE id=?3",
            params![serde_json::to_string(&s).map_err(err)?, s.copied_at_ms, id],
        )
        .map_err(err)?;
        return Ok(());
    }
    let bytes: Option<Vec<u8>> = db
        .query_row("SELECT bytes FROM cl_blobs WHERE hash=?1", [hash], |r| {
            r.get(0)
        })
        .optional()
        .map_err(err)?;
    let text = bytes
        .as_deref()
        .map(decode)
        .transpose()?
        .and_then(|s| s.text)
        .unwrap_or_default()
        .to_lowercase();
    db.execute("INSERT OR IGNORE INTO cl_items(global_id,digest,summary,payload,search_text,copied,imported,ordinary,blob_hash,origin_device) VALUES(?1,?2,?3,?4,?5,?6,?7,0,?8,?9)",params![gid,hash,serde_json::to_string(&s).map_err(err)?,bytes,text,s.copied_at_ms,now_ms(),hash,s.source_device_id]).map_err(err)?;
    s.id = db
        .query_row("SELECT id FROM cl_items WHERE global_id=?1", [gid], |r| {
            r.get(0)
        })
        .map_err(err)?;
    db.execute(
        "UPDATE cl_items SET summary=?1,payload=?3,search_text=?4,blob_hash=?5,digest=?5,copied=?6,origin_device=?7 WHERE global_id=?2",
        params![serde_json::to_string(&s).map_err(err)?, gid, bytes, text, hash, s.copied_at_ms, s.source_device_id],
    )
    .map_err(err)?;
    Ok(())
}
/// Project a synced `index:` record into a searchable row. Merges by global id
/// and by content digest so the same content never appears twice, and never
/// overwrites a local row that already holds the payload.
fn materialize_index(db: &Connection, v: &Value, author: &str) -> Result<()> {
    let gid = v["id"].as_str().ok_or("索引ID无效")?;
    let digest = v["digest"].as_str().unwrap_or("");
    let mut s: Summary = serde_json::from_value(v["summary"].clone()).map_err(err)?;
    let blob_hash = v["blob_hash"].as_str().unwrap_or("");
    let title = v["title"].as_str();
    let ocr_text = v["ocr_text"].as_str();
    let search_text = v["search_text"].as_str().unwrap_or("");

    if let Some((id, ordinary, remote)) = db
        .query_row(
            "SELECT id,ordinary,remote FROM cl_items WHERE global_id=?1",
            [gid],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            },
        )
        .optional()
        .map_err(err)?
    {
        if !remote {
            // A local record already covers this content; never overwrite it.
            return Ok(());
        }
        if ordinary {
            s.id = id as u64;
            db.execute(
                "UPDATE cl_items SET summary=?1,title=?2,ocr_text=?3,search_text=?4,blob_hash=?5,digest=?6,copied=?7 WHERE id=?8 AND (summary IS NOT ?1 OR title IS NOT ?2 OR ocr_text IS NOT ?3 OR search_text IS NOT ?4 OR blob_hash IS NOT ?5 OR digest IS NOT ?6 OR copied IS NOT ?7)",
                params![
                    serde_json::to_string(&s).map_err(err)?,
                    title,
                    ocr_text,
                    search_text,
                    blob_hash,
                    digest,
                    s.copied_at_ms,
                    id
                ],
            )
            .map_err(err)?;
        }
        return Ok(());
    }
    if !digest.is_empty()
        && db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM cl_items WHERE digest=?1 AND ordinary=1)",
                [digest],
                |r| r.get::<_, bool>(0),
            )
            .map_err(err)?
    {
        return Ok(());
    }
    // The sender's own "本机" is meaningless here: attribute the remote index
    // row to the sending device.
    s.source = format!("远端 {author}");
    s.source_device_id = Some(author.to_string());
    db.execute(
        "INSERT OR IGNORE INTO cl_items(global_id,digest,summary,payload,search_text,copied,imported,ordinary,remote,blob_hash,origin_device,title,ocr_text) VALUES(?1,?2,?3,NULL,?4,?5,?6,1,1,?7,?8,?9,?10)",
        params![
            gid,
            digest,
            serde_json::to_string(&s).map_err(err)?,
            search_text,
            s.copied_at_ms,
            now_ms(),
            blob_hash,
            author,
            title,
            ocr_text
        ],
    )
    .map_err(err)?;
    Ok(())
}

fn reconcile_index_db(db: &Connection) -> Result<()> {
    let mut stmt = db
        .prepare("SELECT value,author FROM cl_records WHERE key GLOB 'index:*' AND deleted=0")
        .map_err(err)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(err)?;
    for row in rows {
        let (raw, author) = row.map_err(err)?;
        let value: Value = serde_json::from_str(&raw).map_err(err)?;
        materialize_index(db, &value, &author)?;
    }
    Ok(())
}

fn validate_list(l: &List) -> Result<()> {
    if l.id.is_empty()
        || l.name.trim().is_empty()
        || !["smart", "manual"].contains(&l.kind.as_str())
        || !["all", "any"].contains(&l.match_mode.as_str())
    {
        return Err("列表名称或类型无效".into());
    }
    for r in &l.rules {
        if !["app", "date", "kind", "text", "device"].contains(&r.field.as_str()) {
            return Err("智能条件字段无效".into());
        }
        if ![
            "is",
            "is_not",
            "contains",
            "not_contains",
            "today",
            "yesterday",
            "last_days",
            "between",
            "range",
        ]
        .contains(&r.op.as_str())
        {
            return Err("智能条件运算无效".into());
        }
        if r.op == "last_days" && r.value.parse::<u32>().ok().filter(|v| *v > 0).is_none() {
            return Err("最近天数必须大于零".into());
        }
    }
    Ok(())
}
fn matches_rule(
    index: usize,
    r: &Rule,
    s: &Summary,
    text: &str,
    title: &str,
    ocr: &str,
    q: &Value,
) -> bool {
    match r.field.as_str() {
        "app" | "device" => {
            let actual = if r.field == "app" {
                s.source_app_id.as_deref()
            } else {
                s.source_device_id.as_deref()
            };
            actual.is_some_and(|v| {
                if r.op == "is_not" {
                    v != r.value
                } else {
                    v == r.value
                }
            })
        }
        "kind" => {
            if r.op == "is_not" {
                s.kind != r.value
            } else {
                s.kind == r.value
            }
        }
        "text" => {
            let needle = r.value.to_lowercase();
            let found = text.contains(&needle) || title.contains(&needle) || ocr.contains(&needle);
            if r.op == "not_contains" {
                !found
            } else {
                found
            }
        }
        "date" => {
            let Some(t) = s.copied_at_ms else {
                return false;
            };
            if let Some(window) = q["rule_windows"].get(index.to_string()) {
                return window["start"]
                    .as_u64()
                    .zip(window["end"].as_u64())
                    .is_some_and(|(start, end)| t >= start && t < end);
            }
            let start = q["day_start_ms"].as_u64().unwrap_or(0);
            let end = q["day_end_ms"].as_u64().unwrap_or(0);
            match r.op.as_str() {
                "today" => t >= start && t < end,
                "yesterday" => t >= q["yesterday_start_ms"].as_u64().unwrap_or(start) && t < start,
                "last_days" => {
                    let n = r.value.parse::<u64>().unwrap_or(1);
                    t >= start.saturating_sub(n.saturating_sub(1) * 86400000) && t < end
                }
                "between" => {
                    t >= r.value.parse().unwrap_or(u64::MAX) && t < r.value2.parse().unwrap_or(0)
                }
                _ => false,
            }
        }
        _ => false,
    }
}
fn encode(snapshot: &PasteboardSnapshot, source: &str) -> Result<Vec<u8>> {
    bincode::DefaultOptions::new()
        .serialize(&SnapshotV1 {
            source: source.into(),
            snapshot: snapshot.clone(),
        })
        .map_err(err)
}
fn decode(bytes: &[u8]) -> Result<PasteboardSnapshot> {
    bincode::DefaultOptions::new()
        .with_limit(bytes.len() as u64)
        .deserialize::<SnapshotV1>(bytes)
        .map(|v| v.snapshot)
        .map_err(|_| "剪贴板内容编码无效".into())
}
fn digest(snapshot: &PasteboardSnapshot) -> Result<String> {
    let mut s = snapshot.clone();
    s.change_count = 0;
    for item in &mut s.items {
        item.representations.retain(|r| {
            !matches!(
                r.type_name.as_str(),
                METADATA_TYPE | "com.deskfuse.clipboard-origin" | "com.apple.is-remote-clipboard"
            )
        });
    }
    s.items.retain(|i| !i.representations.is_empty());
    Ok(hex::encode(Sha256::digest(
        bincode::DefaultOptions::new().serialize(&s).map_err(err)?,
    )))
}
/// File URL image records keep their original file restore semantics, but use
/// the image presentation rather than a filename-only card. Never applies to
/// plain text, even when that text happens to end in an image extension.
pub(crate) fn image_file_mime(path: &str) -> Option<&'static str> {
    match std::path::Path::new(path)
        .extension()?
        .to_str()?
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "heic" | "heif" => Some("image/heic"),
        "tif" | "tiff" => Some("image/tiff"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}
fn project_image_file(summary: &mut Summary) {
    if summary.kind == "files"
        && summary.file_count == 1
        && let Some(mime) = image_file_mime(&summary.preview)
    {
        summary.kind = "image".into();
        summary.mime = mime.into();
        summary.has_image = true;
    }
}
/// Identity used to collapse the same logical content into one history row.
///
/// The exact `digest` covers every pasted representation, so a synced copy and
/// a richer local capture of the same text get different digests and used to
/// create two rows. Plain text is therefore keyed by its bytes even when a
/// TIFF preview is attached (Handoff often adds one). Images use exact decoded
/// pixels, including a single image file alias. Other files keep the full digest.
fn text_content_digest(snapshot: &PasteboardSnapshot) -> Option<String> {
    if snapshot.file_urls.is_empty()
        && let Some(text) = crate::clipboard_echo::snapshot_plain_texts(snapshot)
            .into_iter()
            .next()
        && (!is_primary_image(snapshot) || text_outranks_companion_image(&text))
    {
        let normalized = crate::clipboard_echo::normalize_clipboard_text(&text);
        let mut buf = Vec::with_capacity(normalized.len() + 5);
        buf.extend_from_slice(b"text\0");
        buf.extend_from_slice(normalized.as_bytes());
        return Some(hex::encode(Sha256::digest(&buf)));
    }
    None
}
fn content_digest(snapshot: &PasteboardSnapshot) -> Result<String> {
    if let Some(digest) = text_content_digest(snapshot) {
        return Ok(digest);
    }
    if let Some(image_digest) = crate::clipboard_image::snapshot_digest(snapshot) {
        return Ok(image_digest);
    }
    digest(snapshot)
}
/// Like [`content_digest`], but reuses an image digest from the caller's
/// single decode pass instead of decoding the snapshot again.
fn content_digest_with(
    snapshot: &PasteboardSnapshot,
    image_digest: Option<String>,
) -> Result<String> {
    if let Some(digest) = text_content_digest(snapshot) {
        return Ok(digest);
    }
    if let Some(image_digest) = image_digest {
        return Ok(image_digest);
    }
    digest(snapshot)
}
fn is_primary_image(s: &PasteboardSnapshot) -> bool {
    s.png.is_some()
        || s.items.iter().flat_map(|i| &i.representations).any(|r| {
            matches!(
                r.type_name.as_str(),
                "public.png" | "public.jpeg" | "public.heic" | "public.heif"
            )
        })
}
/// 接力给长文本配的 TIFF/PNG 预览不能另开一条历史。短图注仍跟图像走。
fn text_outranks_companion_image(text: &str) -> bool {
    text.contains('\n') || text.chars().count() > 80
}
fn summary(id: u64, s: &PasteboardSnapshot, o: &Origin) -> Summary {
    let lan_photo = s
        .items
        .iter()
        .flat_map(|item| &item.representations)
        .find(|rep| rep.type_name == crate::clipboard_bundle::LAN_PHOTO_TYPE)
        .and_then(|rep| serde_json::from_slice::<Value>(&rep.data).ok());
    let (kind, mime, preview, length) = if s.file_urls.len() == 1
        && let Some(metadata) = lan_photo.as_ref()
    {
        (
            "image",
            metadata["mime"]
                .as_str()
                .unwrap_or("image/jpeg")
                .to_string(),
            std::path::Path::new(&s.file_urls[0])
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            metadata["size"].as_u64().unwrap_or(0) as usize,
        )
    } else if !s.file_urls.is_empty() {
        (
            "files",
            deskfuse_transfer::CLIPBOARD_FILE_MIME.to_string(),
            s.file_urls
                .iter()
                .take(3)
                .map(|p| {
                    std::path::Path::new(p)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join("、"),
            s.file_urls.len(),
        )
    } else if (s.png.is_some()
        || s.items.iter().flat_map(|i| &i.representations).any(|r| {
            [
                "public.png",
                "public.jpeg",
                "public.tiff",
                "public.heic",
                "public.heif",
            ]
            .contains(&r.type_name.as_str())
        }))
        && crate::clipboard_echo::snapshot_plain_texts(s)
            .first()
            .is_none_or(|text| !text_outranks_companion_image(text))
    {
        (
            "image",
            "image/png".into(),
            "图像".into(),
            s.png.as_ref().map_or(0, Vec::len),
        )
    } else if let Some(text) = crate::clipboard_echo::snapshot_plain_texts(s)
        .into_iter()
        .next()
    {
        (
            "text",
            "text/plain".into(),
            text.chars().take(160).collect(),
            text.len(),
        )
    } else {
        (
            "other",
            "application/octet-stream".into(),
            "剪贴板内容".into(),
            0,
        )
    };
    let mut result = Summary {
        id,
        copied_at_ms: o.copied_at_ms,
        mime,
        length: length as u64,
        preview,
        source: o.device_name.clone(),
        has_image: kind == "image",
        file_count: s.file_urls.len() as u32,
        source_device_id: (!o.device_id.is_empty()).then(|| o.device_id.clone()),
        source_app_id: o.app_id.clone(),
        source_app_name: o.app_name.clone(),
        kind: kind.into(),
    };
    project_image_file(&mut result);
    result
}

// Rebuild active member projections after out-of-order delivery; reclaim only archives
// with observed membership operations, never items waiting for their first membership.
fn reconcile_archives(db: &Connection) -> Result<()> {
    let active = members_db(db)?;
    let mut seen = std::collections::BTreeSet::new();
    {
        let mut stmt = db
            .prepare("SELECT value FROM cl_records WHERE key GLOB 'member:*'")
            .map_err(err)?;
        for raw in stmt.query_map([], |r| r.get::<_, String>(0)).map_err(err)? {
            let v: Value = serde_json::from_str(&raw.map_err(err)?).map_err(err)?;
            if let Some(id) = v["item_id"].as_str() {
                seen.insert(id.to_string());
            }
        }
    }
    let mut retired_hashes = std::collections::BTreeSet::new();
    for id in seen {
        let deleted: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM cl_records WHERE key=?1 AND deleted=1)",
                [format!("item:{id}")],
                |row| row.get(0),
            )
            .map_err(err)?;
        if !deleted && active.iter().any(|(_, i)| i == &id) {
            let raw: Option<String> = db
                .query_row(
                    "SELECT value FROM cl_records WHERE key=?1 AND deleted=0",
                    [format!("item:{id}")],
                    |r| r.get(0),
                )
                .optional()
                .map_err(err)?;
            if let Some(raw) = raw {
                materialize(db, &serde_json::from_str(&raw).map_err(err)?)?;
            }
        } else {
            let hash: Option<String> = db
                .query_row(
                    "SELECT blob_hash FROM cl_items WHERE global_id=?1 AND ordinary=0",
                    [&id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(err)?
                .flatten();
            if let Some(hash) = hash {
                retired_hashes.insert(hash);
            }
            db.execute(
                "DELETE FROM cl_items WHERE global_id=?1 AND ordinary=0",
                [id],
            )
            .map_err(err)?;
        }
    }
    for hash in retired_hashes {
        db.execute("DELETE FROM cl_blobs WHERE hash=?1 AND NOT EXISTS(SELECT 1 FROM cl_items WHERE blob_hash=?1)", [hash]).map_err(err)?;
    }
    Ok(())
}

/// Return the remove-wins register value over the maximal causal operations.
/// The query is indexed by record key and never loads clipboard payloads.
fn member_winner(db: &Connection, key: &str) -> Result<Value> {
    type Clock = std::collections::BTreeMap<String, u64>;
    let mut frontier: Vec<(Value, Clock)> = Vec::new();
    let mut statement = db
        .prepare("SELECT body FROM cl_operations WHERE json_extract(body,'$.key')=?1 ORDER BY seq")
        .map_err(err)?;
    let rows = statement
        .query_map([key], |row| row.get::<_, String>(0))
        .map_err(err)?;
    let dominates = |a: &Clock, b: &Clock| {
        b.iter()
            .all(|(device, revision)| a.get(device).unwrap_or(&0) >= revision)
    };
    for row in rows {
        let value: Value = serde_json::from_str(&row.map_err(err)?).map_err(err)?;
        let clock: Clock = serde_json::from_value(value["clock"].clone()).map_err(err)?;
        if frontier
            .iter()
            .any(|(_, old)| dominates(old, &clock) && old != &clock)
        {
            continue;
        }
        frontier.retain(|(_, old)| !dominates(&clock, old) || old == &clock);
        frontier.push((value, clock));
    }
    frontier
        .into_iter()
        .map(|(value, _)| value)
        .max_by(|a, b| {
            (
                a["deleted"].as_bool(),
                a["revision"].as_u64(),
                a["author"].as_str(),
                a["op_id"].as_str(),
            )
                .cmp(&(
                    b["deleted"].as_bool(),
                    b["revision"].as_u64(),
                    b["author"].as_str(),
                    b["op_id"].as_str(),
                ))
        })
        .ok_or_else(|| "成员操作缺失".into())
}

#[cfg(test)]
mod membership_tests {
    use super::*;

    fn page(library: &Library, list: Option<&str>) -> Value {
        serde_json::from_str(
            &library
                .query(&json!({"op": "items", "list_id": list}).to_string())
                .unwrap(),
        )
        .unwrap()
    }

    fn counts(library: &Library) -> Value {
        let state: Value = serde_json::from_str(&library.state().unwrap()).unwrap();
        let mut requests = vec![
            json!({"list_id": null, "query": "does-not-match", "source": "missing", "offset": 999}),
        ];
        requests.extend(state["lists"].as_array().unwrap().iter().map(|list| {
            json!({"list_id": list["id"], "query": "does-not-match", "source": "missing", "offset": 999})
        }));
        serde_json::from_str(
            &library
                .query(&json!({"op": "counts", "requests": requests}).to_string())
                .unwrap(),
        )
        .unwrap()
    }

    fn manual_count(library: &Library, list: &str) -> u64 {
        counts(library)[list].as_u64().unwrap()
    }

    #[test]
    fn navigation_counts_match_full_pages_not_names_filters_or_recent_cache() {
        let library = Library::open().unwrap();
        library.set_local_device("count-test".into()).unwrap();
        for (id, kind) in [
            ("renamed-images", "image"),
            ("renamed-files", "files"),
            ("renamed-text", "text"),
        ] {
            library
                .mutate(
                    &json!({"op": "save_list", "list": {
                        "id": id, "name": "custom name", "kind": "smart",
                        "rules": [{"field": "kind", "op": "is", "value": kind}]
                    }})
                    .to_string(),
                )
                .unwrap();
        }
        {
            let db = library.db.lock().unwrap();
            for n in 0..90 {
                let kind = ["image", "files", "text"][n % 3];
                let summary = json!({"id": n+1, "mime": "text/plain", "length": 1,
                    "preview": "fixture", "source": "local", "has_image": kind == "image",
                    "file_count": if kind == "files" { 1 } else { 0 }, "kind": kind});
                db.execute("INSERT INTO cl_items(global_id,digest,summary,imported,ordinary) VALUES(?1,?1,?2,?3,1)",
                    params![format!("fixture:{n}"), summary.to_string(), now_ms()]).unwrap();
            }
        }
        let expected = counts(&library);
        assert_eq!(expected["all"], 90);
        for id in ["renamed-images", "renamed-files", "renamed-text"] {
            assert_eq!(expected[id], 30);
            assert_eq!(page(&library, Some(id))["total"], expected[id]);
            assert_eq!(counts(&library), expected);
        }
        assert_eq!(page(&library, None)["total"], expected["all"]);
        assert_eq!(page(&library, None)["items"].as_array().unwrap().len(), 20);
        library.mutate(&json!({"op": "save_list", "list": {
            "id": "renamed-images", "name": "files and images", "kind": "smart", "match_mode": "any",
            "rules": [{"field": "kind", "op": "is", "value": "image"}, {"field": "kind", "op": "is", "value": "files"}]
        }}).to_string()).unwrap();
        assert_eq!(counts(&library)["renamed-images"], 60);
        library
            .mutate(&json!({"op": "delete_items", "ids": [1]}).to_string())
            .unwrap();
        assert_eq!(counts(&library)["all"], 89);
        assert_eq!(counts(&library)["renamed-images"], 59);
        assert_eq!(
            counts(&library)["renamed-images"],
            page(&library, Some("renamed-images"))["total"]
        );
    }

    #[test]
    fn deleting_archived_items_updates_cached_counts_and_remote_members() {
        let library = Library::open().unwrap();
        library.set_local_device("delete-test".into()).unwrap();
        for id in ["first", "second"] {
            library
                .mutate(
                    &json!({"op": "save_list", "list": {
                        "id": id, "name": id, "kind": "manual"
                    }})
                    .to_string(),
                )
                .unwrap();
        }
        let mut originals = Vec::new();
        for text in ["archived one", "archived two"] {
            let id = library
                .ingest(
                    &PasteboardSnapshot {
                        text: Some(text.into()),
                        ..Default::default()
                    },
                    "local",
                )
                .unwrap()
                .unwrap();
            for list in ["first", "second"] {
                library.membership(list, id, true).unwrap();
            }
            originals.push(id);
        }
        let archived_page = page(&library, Some("first"));
        let archive_id = archived_page["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["preview"].as_str().unwrap().contains("archived two"))
            .unwrap()["id"]
            .as_u64()
            .unwrap();
        let before = library.generation();
        assert_eq!(manual_count(&library, "first"), 2);
        let old_ops = library.export_operations(0, 100).unwrap();
        let peer = Library::open().unwrap();
        peer.set_local_device("delete-peer".into()).unwrap();
        peer.merge_operations(old_ops.clone()).unwrap();
        assert_eq!(manual_count(&peer, "first"), 2);

        // Mix ordinary and archive IDs as a real batch deletion can do.
        let response: Value = serde_json::from_str(
            &library
                .mutate(
                    &json!({
                        "op": "delete_items", "ids": [originals[0], archive_id]
                    })
                    .to_string(),
                )
                .unwrap(),
        )
        .unwrap();
        assert!(response["revision"].as_u64().unwrap() > before);
        assert_eq!(manual_count(&library, "first"), 0);
        assert_eq!(manual_count(&library, "second"), 0);
        assert_eq!(counts(&library)["all"], 0);
        peer.merge_operations(library.export_operations(0, 100).unwrap())
            .unwrap();
        peer.merge_operations(old_ops).unwrap();
        assert_eq!(manual_count(&peer, "first"), 0);
        assert_eq!(manual_count(&peer, "second"), 0);

        library
            .mutate(&json!({"op": "delete_list", "id": "first"}).to_string())
            .unwrap();
        assert!(counts(&library).get("first").is_none());
        assert_eq!(manual_count(&library, "second"), 0);
    }

    #[test]
    fn manual_membership_uses_archive_identity_in_pages_and_badges() {
        let library = Library::open().unwrap();
        library
            .set_local_device("membership-test-device".into())
            .unwrap();
        for id in ["first", "second"] {
            library
                .mutate(
                    &json!({
                        "op": "save_list", "list": {"id": id, "name": id, "kind": "manual"}
                    })
                    .to_string(),
                )
                .unwrap();
        }
        let id = library
            .ingest(
                &PasteboardSnapshot {
                    text: Some("manual membership regression".into()),
                    ..Default::default()
                },
                "local",
            )
            .unwrap()
            .unwrap();
        assert_eq!(manual_count(&library, "first"), 0);
        library.membership("first", id, true).unwrap();
        assert_eq!(manual_count(&library, "first"), 1);
        let all = page(&library, None);
        assert_eq!(all["total"], 1);
        assert_eq!(all["items"][0]["id"], id);
        assert_eq!(all["items"][0]["memberships"], json!(["first"]));
        let manual = page(&library, Some("first"));
        assert_eq!(manual["total"], 1);
        let archive = manual["items"][0]["id"].as_u64().unwrap();
        assert_ne!(archive, id);
        assert_eq!(manual["items"][0]["memberships"], json!(["first"]));

        // An archive can join a second list and survive ordinary-history cleanup.
        library.membership("second", archive, true).unwrap();
        library.clear_history().unwrap();
        assert_eq!(manual_count(&library, "first"), 1);
        assert_eq!(manual_count(&library, "second"), 1);
        assert_eq!(page(&library, None)["total"], 1);
        assert_eq!(page(&library, Some("second"))["items"][0]["id"], archive);
        library.membership("first", archive, false).unwrap();
        assert_eq!(manual_count(&library, "first"), 0);
        assert_eq!(manual_count(&library, "second"), 1);
        assert_eq!(page(&library, Some("first"))["total"], 0);
        assert_eq!(
            page(&library, None)["items"][0]["memberships"],
            json!(["second"])
        );
        library.membership("second", archive, false).unwrap();
        assert_eq!(manual_count(&library, "second"), 0);
        assert_eq!(page(&library, None)["total"], 0);
    }
}

#[cfg(test)]
mod remote_text_tests {
    use super::*;

    /// Materialize a synced `index:` record the way the library sync worker does.
    fn sync_text(library: &Library, gid: &str, hash: &str, text: &str) -> u64 {
        let summary = json!({
            "id": 0,
            "copied_at_ms": now_ms(),
            "mime": "text/plain",
            "length": text.len() as u64,
            "preview": text.chars().take(160).collect::<String>(),
            "source": "本机",
            "has_image": false,
            "file_count": 0,
            "source_device_id": null,
            "source_app_id": null,
            "source_app_name": null,
            "kind": "text",
        });
        library
            .merge_operations(vec![json!({
                "op_id": format!("op-{gid}"),
                "key": format!("index:{gid}"),
                "revision": 1,
                "author": "peer-device",
                "clock": {"peer-device": 1},
                "deleted": false,
                "value": {
                    "id": gid,
                    "digest": format!("text:{gid}"),
                    "summary": summary,
                    "title": null,
                    "ocr_text": null,
                    "search_text": text.to_lowercase(),
                    "blob_hash": hash,
                },
            })])
            .unwrap();
        let db = library.db.lock().unwrap();
        db.query_row("SELECT id FROM cl_items WHERE global_id=?1", [gid], |r| {
            r.get(0)
        })
        .unwrap()
    }

    #[test]
    fn startup_index_reconciliation_skips_unchanged_rows_and_repairs_changes() {
        let library = Library::open().unwrap();
        let id = sync_text(
            &library,
            "startup-index",
            "blob-startup",
            "searchable startup text",
        );
        let mut db = library.db.lock().unwrap();
        // Normalize existing rows once, then a normal reopen must not rewrite
        // their search index (or cause one disk commit per historical item).
        reconcile_index_db(&db).unwrap();
        let before = db.total_changes();
        reconcile_index_db(&db).unwrap();
        assert_eq!(db.total_changes(), before);
        db.execute("UPDATE cl_items SET title='stale' WHERE id=?1", [id])
            .unwrap();
        let tx = db.transaction().unwrap();
        reconcile_index_db(&tx).unwrap();
        tx.commit().unwrap();
        let title: Option<String> = db
            .query_row("SELECT title FROM cl_items WHERE id=?1", [id], |r| r.get(0))
            .unwrap();
        assert_eq!(title, None);
        let hits: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM cl_items_fts WHERE cl_items_fts MATCH 'sea'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1);
        let before = db.total_changes();
        reconcile_index_db(&db).unwrap();
        assert_eq!(db.total_changes(), before);
    }

    fn page_row(library: &Library, id: u64) -> Value {
        let page: Value = serde_json::from_str(
            &library
                .query(&json!({"op": "items", "limit": 50}).to_string())
                .unwrap(),
        )
        .unwrap();
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"].as_u64() == Some(id))
            .unwrap()
            .clone()
    }

    #[test]
    fn complete_short_text_is_usable_before_its_payload_arrives() {
        let library = Library::open().unwrap();
        library.set_local_device("receive-test".into()).unwrap();
        let text = "ssh -i ~/.ssh/my_docker_key godd@64.32.22.19";
        let hash = "a".repeat(64);
        let id = sync_text(&library, "remote-short", &hash, text);
        let row = page_row(&library, id);
        assert_eq!(row["remote"], json!(true));
        assert_eq!(row["available"], json!(true));
        assert_eq!(library.snapshot(id).unwrap().text.as_deref(), Some(text));
        // The full-fidelity blob is still fetched in the background.
        assert_eq!(library.take_wanted_blobs().unwrap(), vec![hash]);
    }

    #[test]
    fn truncated_text_stays_unavailable_until_the_payload_arrives() {
        let library = Library::open().unwrap();
        library.set_local_device("receive-test".into()).unwrap();
        let text = "长文本".repeat(100);
        let hash = "b".repeat(64);
        let id = sync_text(&library, "remote-long", &hash, &text);
        assert_eq!(page_row(&library, id)["available"], json!(false));
        assert_eq!(
            library.snapshot(id).unwrap_err(),
            "内容正在从来源设备获取，请稍后重试"
        );
        assert_eq!(library.take_wanted_blobs().unwrap(), vec![hash]);
    }
}

#[cfg(test)]
mod query_performance_tests {
    use super::*;

    #[test]
    fn image_dedup_and_pages_use_metadata_indexes_with_large_payloads() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            r#"CREATE TABLE cl_items(
                id INTEGER PRIMARY KEY,global_id TEXT UNIQUE,digest TEXT,summary TEXT,
                payload BLOB,search_text TEXT,search_index TEXT,copied INTEGER,
                imported INTEGER,ordinary INTEGER,title TEXT,ocr_text TEXT,
                ocr_version INTEGER,remote INTEGER,last_used INTEGER);
             CREATE INDEX cl_items_digest ON cl_items(digest);
             CREATE INDEX cl_items_usage_order ON cl_items(
                MAX(COALESCE(last_used,0),COALESCE(copied,imported)) DESC,id DESC);
             INSERT INTO cl_items VALUES
                (1,'remote-old','image-v1:a','{"file_count":0}',NULL,'','',10,10,1,NULL,NULL,0,1,NULL),
                (2,'native','image-v1:a','{"file_count":0}',zeroblob(2097152),'','',20,20,1,NULL,NULL,0,0,NULL),
                (3,'file-alias','image-v1:a','{"file_count":1}',zeroblob(2097152),'','',30,30,1,NULL,NULL,0,0,NULL),
                (4,'remote-new','image-v1:a','{"file_count":0}',NULL,'','',40,40,1,NULL,NULL,0,1,NULL),
                (5,'text','text:a','{"file_count":0}',X'0102','text','text',50,50,1,NULL,NULL,0,0,NULL),
                (6,'archive:native','image-v1:a','{"file_count":0}',zeroblob(2097152),'','',60,60,0,NULL,NULL,0,0,NULL);"#,
        ).unwrap();
        let sql = format!("{QUERY_SELECT} WHERE {UNIQUE_IMAGE_ROW} ORDER BY {QUERY_ORDER}");
        let query_ids = |sql: &str| {
            db.prepare(sql)
                .unwrap()
                .query_map([], |row| row.get::<_, u64>(0))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
        };
        let original = query_ids(&sql);
        assert_eq!(original, [6, 5, 2]);
        db.execute_batch(QUERY_INDEXES).unwrap();
        db.execute_batch(QUERY_INDEXES).unwrap(); // Existing stores reopen safely.
        assert_eq!(query_ids(&sql), original);
        assert_eq!(query_ids(&format!("{sql} LIMIT 2")), [6, 5]);
        let cursor_sql = format!(
            "{QUERY_SELECT} WHERE {UNIQUE_IMAGE_ROW} AND
             (MAX(COALESCE(i.last_used,0),COALESCE(i.copied,i.imported)),i.id)<(50,5)
             ORDER BY {QUERY_ORDER} LIMIT 2"
        );
        assert_eq!(query_ids(&cursor_sql), [2]);
        let count_sql = format!("SELECT COUNT(*) FROM cl_items i WHERE {UNIQUE_IMAGE_ROW}");
        assert_eq!(
            db.query_row(&count_sql, [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        for statement in [&sql, &format!("{sql} LIMIT 21"), &count_sql] {
            let plan = db
                .prepare(&format!("EXPLAIN QUERY PLAN {statement}"))
                .unwrap()
                .query_map([], |r| r.get::<_, String>(3))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
                .join("\n");
            assert!(plan.contains("INDEX cl_items_image_identity"), "{plan}");
            if statement != &count_sql {
                assert!(plan.contains("INDEX cl_items_metadata_page"), "{plan}");
            }
            // SQLite 3.46 does not label expression indexes as COVERING in the
            // plan text. Verify the actual VM never reads a cl_items column.
            let root: i64 = db
                .query_row(
                    "SELECT rootpage FROM sqlite_master WHERE name='cl_items'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let ops = db
                .prepare(&format!("EXPLAIN {statement}"))
                .unwrap()
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                })
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap();
            let table_cursors: HashSet<i64> = ops
                .iter()
                .filter(|(op, _, p2)| op == "OpenRead" && *p2 == root)
                .map(|(_, p1, _)| *p1)
                .collect();
            let table_reads: Vec<_> = ops
                .iter()
                .filter(|(op, p1, _)| op == "Column" && table_cursors.contains(p1))
                .collect();
            assert!(
                table_reads.is_empty(),
                "payload table reads: {table_reads:?}; {plan}"
            );
        }
        assert_eq!(
            db.query_row("SELECT length(payload) FROM cl_items WHERE id=2", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2_097_152
        );
    }
}

#[cfg(test)]
mod digest_tests {
    use super::*;
    use deskfuse_platform::{PasteboardItem, PasteboardRepresentation};

    fn text(value: &str) -> PasteboardSnapshot {
        PasteboardSnapshot {
            text: Some(value.into()),
            ..Default::default()
        }
    }

    #[test]
    fn tiff_preview_shares_plain_text_digest() {
        let mut with_preview = text("帮我安装 kimi code");
        with_preview.items = vec![PasteboardItem {
            representations: vec![PasteboardRepresentation {
                type_name: "public.tiff".into(),
                data: vec![0, 1, 2],
            }],
        }];
        assert_eq!(
            content_digest(&text("帮我安装 kimi code")).unwrap(),
            content_digest(&with_preview).unwrap()
        );
        assert_eq!(
            content_digest(&text("a\r\nb")).unwrap(),
            content_digest(&text("a\nb")).unwrap()
        );
    }

    #[test]
    fn real_png_keeps_its_own_digest() {
        let mut image = text("caption");
        image.png = Some(vec![137, 80, 78, 71]);
        assert_ne!(
            content_digest(&text("caption")).unwrap(),
            content_digest(&image).unwrap()
        );
    }
}
