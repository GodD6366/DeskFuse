//! SQLite-backed clipboard library. Native payloads remain lazy, opaque and local.
use crate::model::ClipboardHistoryItem;
use bincode::Options;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use unidesk_platform::PasteboardSnapshot;

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
pub struct Library {
    db: Mutex<Connection>,
    device: Mutex<String>,
    generation: AtomicU64,
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
        let _canonical = unidesk_store::SQLiteStore::new().map_err(err)?;
        let mut db = Connection::open(unidesk_store::SQLiteStore::default_path().map_err(err)?)
            .map_err(err)?;
        unidesk_store::configure_connection(&mut db).map_err(err)?;
        db.execute_batch("PRAGMA foreign_keys=ON;
   CREATE TABLE IF NOT EXISTS cl_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS cl_items(id INTEGER PRIMARY KEY AUTOINCREMENT,global_id TEXT UNIQUE NOT NULL,digest TEXT NOT NULL,summary TEXT NOT NULL,payload BLOB,search_text TEXT NOT NULL DEFAULT '',copied INTEGER,imported INTEGER NOT NULL,ordinary INTEGER NOT NULL DEFAULT 1,blob_hash TEXT,origin_device TEXT);
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
        add_column("last_used", "ALTER TABLE cl_items ADD COLUMN last_used INTEGER")?;
        db.execute_batch("CREATE INDEX IF NOT EXISTS cl_items_usage_order ON cl_items(MAX(COALESCE(last_used,0),COALESCE(copied,imported)) DESC,id DESC);").map_err(err)?;
        let this = Self {
            db: Mutex::new(db),
            device: Mutex::new(String::new()),
            generation: AtomicU64::new(1),
        };
        this.import_legacy()?;
        this.seed_default_lists()?;
        this.prune()?;
        this.reconcile_index()?;
        Ok(this)
    }
    fn seed_default_lists(&self) -> Result<()> {
        // Identical bootstrap operations on every device deduplicate on sync.
        // Replay uses the normal merge path, so edits and deletion tombstones win.
        // UUIDs are derived from com.unidesk.app/clipboard/defaults/v1/{kind}/{list|operation}.
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
    }
    /// Local reuse changes ordering, never the original source/copy metadata.
    /// Called only after the native pasteboard write has succeeded.
    pub fn mark_used(&self, id: u64) -> Result<()> {
        let db = self.db.lock().map_err(err)?;
        let updated = db.execute(
            "UPDATE cl_items SET last_used=MAX(?1,COALESCE((SELECT MAX(last_used)+1 FROM cl_items),0)) WHERE id=?2",
            params![now_ms(), id],
        ).map_err(err)?;
        if updated > 0 { self.changed(); }
        Ok(())
    }

    pub fn set_local_device(&self, id: String) -> Result<()> {
        *self.device.lock().map_err(err)? = id;
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
            let (id, name) = unidesk_platform_macos::clipboard_source_application();
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
        let bytes = encode(snapshot, source)?;
        let mut db = self.db.lock().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        let found: Option<u64> = tx
            .query_row(
                "SELECT id FROM cl_items WHERE digest=?1 AND ordinary=1 LIMIT 1",
                [&fingerprint],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
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
        let s = summary(id, snapshot, &origin);
        let blob_hash = hex::encode(Sha256::digest(&bytes));
        tx.execute("UPDATE cl_items SET payload=?1,summary=?2,search_text=?3,copied=?4,imported=?5,origin_device=?6,blob_hash=?7 WHERE id=?8",params![bytes,serde_json::to_string(&s).map_err(err)?,snapshot.text.as_deref().unwrap_or("").to_lowercase(),origin.copied_at_ms,now_ms(),origin.device_id,blob_hash,id]).map_err(err)?;
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
        let db = self.db.lock().map_err(err)?;
        reconcile_index_db(&db)
    }

    pub fn snapshot(&self, id: u64) -> Result<PasteboardSnapshot> {
        self.prune()?;
        let db = self.db.lock().map_err(err)?;
        let bytes: Option<Vec<u8>> = db
            .query_row("SELECT payload FROM cl_items WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(err)?
            .flatten();
        decode(&bytes.ok_or("内容尚未同步或历史已过期")?)
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
            image_png: crate::tokio_bridge::clipboard_preview_png(&snapshot),
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
        let mut statement = db.prepare("SELECT id,payload,summary FROM cl_items WHERE ordinary=1 AND payload IS NOT NULL ORDER BY MAX(COALESCE(last_used,0),COALESCE(copied,imported)) DESC,id DESC LIMIT ?1").map_err(err)?;
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
        self.prune()?;
        let q: Value = serde_json::from_str(request).map_err(err)?;
        if q["op"] == "state" {
            return self.state();
        }
        self.query_items(q)
    }
    fn state(&self) -> Result<String> {
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
        serde_json::to_string(&json!({"lists":lists,"settings":settings,"apps":apps.values().collect::<Vec<_>>(),"devices":devices.values().collect::<Vec<_>>(),"revision":self.generation()})).map_err(err)
    }
    fn query_items(&self, q: Value) -> Result<String> {
        let db = self.db.lock().map_err(err)?;
        let list_id = q["list_id"].as_str();
        let lists = lists_db(&db)?;
        let selected = if let Some(id) = list_id {
            Some(lists.iter().find(|l| l.id == id).ok_or("列表已删除")?)
        } else {
            None
        };
        let query = q["query"].as_str().unwrap_or("").trim().to_lowercase();
        let kind = q["kind"].as_str().unwrap_or("all");
        let source = q["source"].as_str();
        let offset = q["offset"].as_u64().unwrap_or(0) as usize;
        let limit = q["limit"].as_u64().unwrap_or(50).clamp(1, 100) as usize;
        // Iterate only summaries/text from SQLite; payloads and image bytes are never read here.
        let needs_text = !query.is_empty()
            || selected.is_some_and(|list| list.rules.iter().any(|rule| rule.field == "text"));
        let sql = if needs_text {
            "SELECT id,global_id,summary,search_text,payload IS NOT NULL,ordinary,COALESCE(title,''),COALESCE(ocr_text,''),COALESCE(ocr_version,0),COALESCE(remote,0),MAX(COALESCE(last_used,0),COALESCE(copied,imported)) FROM cl_items ORDER BY MAX(COALESCE(last_used,0),COALESCE(copied,imported)) DESC,id DESC"
        } else {
            "SELECT id,global_id,summary,'',payload IS NOT NULL,ordinary,COALESCE(title,''),COALESCE(ocr_text,''),COALESCE(ocr_version,0),COALESCE(remote,0),MAX(COALESCE(last_used,0),COALESCE(copied,imported)) FROM cl_items ORDER BY MAX(COALESCE(last_used,0),COALESCE(copied,imported)) DESC,id DESC"
        };
        let mut stmt = db.prepare(sql).map_err(err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, u64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, bool>(4)?,
                    r.get::<_, bool>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, i64>(8)?,
                    r.get::<_, bool>(9)?,
                    r.get::<_, i64>(10)?,
                ))
            })
            .map_err(err)?;
        let memberships = members_db(&db)?;
        let mut total = 0usize;
        let mut items = vec![];
        for row in rows {
            let (
                id,
                gid,
                raw,
                text,
                available,
                ordinary,
                title,
                ocr_text,
                ocr_version,
                remote,
                sort_at_ms,
            ) = row.map_err(err)?;
            let title_lower = title.to_lowercase();
            let ocr_lower = ocr_text.to_lowercase();
            let mut s: Summary = serde_json::from_str(&raw).map_err(err)?;
            project_image_file(&mut s);
            let member: Vec<_> = memberships
                .iter()
                .filter(|(_, v)| v == &gid || *v == format!("archive:{gid}"))
                .map(|(l, _)| l.clone())
                .collect();
            if !ordinary && member.is_empty() {
                continue;
            }
            // Manual lists display the pinned archive, not both it and its
            // refreshable ordinary-history source row.
            if ordinary
                && selected.is_some_and(|l| l.kind == "manual")
                && db
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM cl_items WHERE global_id=?1 AND ordinary=0)",
                        [format!("archive:{gid}")],
                        |r| r.get::<_, bool>(0),
                    )
                    .map_err(err)?
            {
                continue;
            }
            if !ordinary
                && selected.is_none_or(|l| l.kind != "manual")
                && gid.starts_with("archive:")
                && db
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM cl_items WHERE global_id=?1 AND ordinary=1)",
                        [&gid[8..]],
                        |r| r.get::<_, bool>(0),
                    )
                    .map_err(err)?
            {
                continue;
            }
            if kind != "all" && kind != s.kind {
                continue;
            }
            if source.is_some_and(|v| s.source_device_id.as_deref() != Some(v) && s.source != v) {
                continue;
            }
            if !query.is_empty()
                && !text.contains(&query)
                && !title_lower.contains(&query)
                && !ocr_lower.contains(&query)
                && !s.preview.to_lowercase().contains(&query)
                && !s.source.to_lowercase().contains(&query)
            {
                continue;
            }
            if let Some(l) = selected {
                if l.kind == "manual" {
                    if !member.contains(&l.id) {
                        continue;
                    }
                } else {
                    let matches = |(i, r): (usize, &Rule)| {
                        matches_rule(i, r, &s, &text, &title_lower, &ocr_lower, &q)
                    };
                    if !(if l.match_mode == "any" {
                        l.rules.iter().enumerate().any(matches)
                    } else {
                        l.rules.iter().enumerate().all(matches)
                    }) {
                        continue;
                    }
                }
            }
            if total >= offset && items.len() < limit {
                let mut value = serde_json::to_value(s).map_err(err)?;
                value["id"] = json!(id);
                value["available"] = json!(available);
                value["memberships"] = json!(member);
                value["title"] = json!(title);
                value["ocr_text"] = json!(ocr_text);
                value["ocr_version"] = json!(ocr_version.max(0) as u64);
                value["remote"] = json!(remote);
                // Same key the page is ordered by, so the client can interleave
                // snapshot-only records without reordering the page.
                value["sort_at_ms"] = json!(sort_at_ms.max(0) as u64);
                items.push(value);
            }
            total += 1;
        }
        serde_json::to_string(&json!({"items":items,"total":total,"next_offset":if offset+limit<total{Some(offset+limit)}else{None},"revision":self.generation()})).map_err(err)
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
                let mut archived = Vec::new();
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
                            archived.push(base.clone());
                        }
                        db.execute("DELETE FROM cl_items WHERE id=?1", [id])
                            .map_err(err)?;
                        db.execute(
                            "DELETE FROM cl_items WHERE global_id=?1 OR global_id=?2",
                            params![base, format!("archive:{base}")],
                        )
                        .map_err(err)?;
                    }
                }
                for global_id in archived {
                    self.local_record(format!("item:{global_id}"), Value::Null, true)?;
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
            let bytes = payload.ok_or("内容尚未同步")?;
            let snapshot = decode(&bytes)?;
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
        .prepare("SELECT value FROM cl_records WHERE key LIKE 'list:%' AND deleted=0")
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
        .prepare("SELECT value FROM cl_records WHERE key LIKE 'member:%' AND deleted=0")
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
                "UPDATE cl_items SET summary=?1,title=?2,ocr_text=?3,search_text=?4,blob_hash=?5,digest=?6,copied=?7 WHERE id=?8",
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
        .prepare("SELECT value,author FROM cl_records WHERE key LIKE 'index:%' AND deleted=0")
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
                METADATA_TYPE | "com.unidesk.clipboard-origin" | "com.apple.is-remote-clipboard"
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
fn is_image_snapshot(s: &PasteboardSnapshot) -> bool {
    s.png.is_some()
        || s.items.iter().flat_map(|i| &i.representations).any(|r| {
            ["public.png", "public.jpeg", "public.tiff", "public.heic"]
                .contains(&r.type_name.as_str())
        })
}
/// Identity used to collapse the same logical content into one history row.
///
/// The exact `digest` covers every pasted representation, so a synced copy and
/// a richer local capture of the same text get different digests and used to
/// create two rows. Plain text is therefore keyed by its bytes; files keep the
/// exact digest because equal names can still mean different content.
fn content_digest(snapshot: &PasteboardSnapshot) -> Result<String> {
    if snapshot.file_urls.is_empty()
        && !is_image_snapshot(snapshot)
        && let Some(text) = &snapshot.text
    {
        let mut buf = Vec::with_capacity(text.len() + 5);
        buf.extend_from_slice(b"text\0");
        buf.extend_from_slice(text.as_bytes());
        return Ok(hex::encode(Sha256::digest(&buf)));
    }
    digest(snapshot)
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
            unidesk_transfer::CLIPBOARD_FILE_MIME.to_string(),
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
    } else if s.png.is_some()
        || s.items.iter().flat_map(|i| &i.representations).any(|r| {
            ["public.png", "public.jpeg", "public.tiff", "public.heic"]
                .contains(&r.type_name.as_str())
        })
    {
        (
            "image",
            "image/png".into(),
            "图像".into(),
            s.png.as_ref().map_or(0, Vec::len),
        )
    } else if let Some(text) = &s.text {
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
            .prepare("SELECT value FROM cl_records WHERE key LIKE 'member:%'")
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
        if active.iter().any(|(_, i)| i == &id) {
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
