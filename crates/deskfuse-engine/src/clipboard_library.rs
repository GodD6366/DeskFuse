//! SQLite-backed clipboard library. Native payloads remain lazy, opaque and local.
use crate::model::ClipboardHistoryItem;
use bincode::Options;
use deskfuse_platform::{PasteboardItem, PasteboardRepresentation, PasteboardSnapshot};
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

#[path = "clipboard_library_query.rs"]
mod query;
#[path = "clipboard_library_storage.rs"]
mod storage;
#[path = "clipboard_library_work.rs"]
mod work;

#[cfg(test)]
#[path = "clipboard_library_tests.rs"]
mod performance_regressions;

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

/// 复制来源文件是否位于临时/暂存目录。
///
/// 远程控制（网易 UU 远程、Windows App）、接力/通用剪贴板、输入法等应用
/// 复制文件时会把内容暂存到这类目录，源文件很快就会被清理；入库后只留下
/// 无法还原的死引用，因此这类文件复制不入库。
pub(crate) fn temporary_clipboard_file(path: &std::path::Path) -> bool {
    use std::path::Component;
    let parts: Vec<&str> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    // 系统临时目录：/tmp 与 /var/folders（macOS 每用户临时目录在其下；
    // /var 是指向 /private/var 的符号链接，两种写法都覆盖）。
    let is = |i: usize, value: &str| parts.get(i) == Some(&value);
    if is(0, "tmp")
        || (is(0, "private") && is(1, "tmp"))
        || (is(0, "var") && is(1, "folders"))
        || (is(0, "private") && is(1, "var") && is(2, "folders"))
    {
        return true;
    }
    // $TMPDIR 可能被重定向到非标准位置（沙箱、CI），兜底匹配一次。
    let temp_dir = std::env::temp_dir();
    if temp_dir != std::path::Path::new("/tmp") && path.starts_with(&temp_dir) {
        return true;
    }
    // 应用容器内的 tmp（远程桌面类应用复制文件时的暂存目录）。
    if parts
        .windows(5)
        .any(|w| w[0] == "Library" && w[1] == "Containers" && w[3] == "Data" && w[4] == "tmp")
    {
        return true;
    }
    // 通用剪贴板（接力）落盘的共享暂存目录。
    if parts.windows(3).any(|w| {
        w[0] == "Group Containers"
            && w[1] == "group.com.apple.coreservices.useractivityd"
            && w[2] == "shared-pasteboard"
    }) {
        return true;
    }
    // 其他应用的缓存目录（输入法等暂存复制内容）。DeskFuse 自身接收文件的
    // 缓存受传输租约保护，不属于临时文件。
    if let Some(cache) = dirs::cache_dir()
        && let Ok(relative) = path.strip_prefix(&cache)
    {
        let first = relative.components().find_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        });
        return !matches!(first, Some("com.deskfuse.app") | Some("com.unidesk.app"));
    }
    // 其他应用 Application Support 下的隐藏暂存文件（如 .uuremote_ 前缀）。
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
        && parts
            .windows(2)
            .any(|w| w[0] == "Library" && w[1] == "Application Support")
    {
        return true;
    }
    false
}

/// 入库前对文件类快照的处理决定。
enum TemporaryFileAction {
    /// 非临时文件（或混合来源），原样入库。
    Keep,
    /// 全部为临时文件但伴随位图/文本等持久内容：剥离文件路径，保留其余内容。
    Strip(PasteboardSnapshot),
    /// 整个快照只有临时文件路径，跳过入库。
    Skip,
}

fn temporary_file_action(snapshot: &PasteboardSnapshot) -> TemporaryFileAction {
    if snapshot.file_urls.is_empty()
        || !snapshot
            .file_urls
            .iter()
            .all(|p| temporary_clipboard_file(std::path::Path::new(p)))
    {
        return TemporaryFileAction::Keep;
    }
    let mut stripped = snapshot.clone();
    stripped.file_urls.clear();
    if has_durable_content(&stripped) {
        TemporaryFileAction::Strip(stripped)
    } else {
        TemporaryFileAction::Skip
    }
}

/// 剥离文件路径后仍值得入库的内容：文本、位图或任何非元数据表示。
fn has_durable_content(snapshot: &PasteboardSnapshot) -> bool {
    if snapshot.text.is_some() || snapshot.png.is_some() {
        return true;
    }
    const METADATA_TYPES: [&str; 3] = [
        METADATA_TYPE,
        "com.deskfuse.clipboard-origin",
        "com.apple.is-remote-clipboard",
    ];
    snapshot
        .items
        .iter()
        .flat_map(|item| &item.representations)
        .any(|rep| !METADATA_TYPES.contains(&rep.type_name.as_str()))
}
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

const QUERY_SELECT: &str = "SELECT i.id,i.global_id,i.summary,'',i.payload IS NOT NULL,i.ordinary,COALESCE(i.title,''),COALESCE(i.ocr_text,''),COALESCE(i.ocr_version,0),COALESCE(i.remote,0),MAX(COALESCE(i.last_used,0),COALESCE(i.copied,i.imported)) FROM cl_items i";
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
const OPERATION_INDEXES: &str = "
CREATE INDEX IF NOT EXISTS cl_records_member_item ON cl_records(
    json_extract(value,'$.item_id'),json_extract(value,'$.list_id')) WHERE deleted=0 AND key GLOB 'member:*';
CREATE INDEX IF NOT EXISTS cl_records_member_list ON cl_records(json_extract(value,'$.list_id'),json_extract(value,'$.item_id')) WHERE key GLOB 'member:*';
CREATE INDEX IF NOT EXISTS cl_operations_author_revision ON cl_operations(
    json_extract(body,'$.author'),json_extract(body,'$.revision'));
CREATE INDEX IF NOT EXISTS cl_operations_revision ON cl_operations(json_extract(body,'$.revision'));
";
pub struct Library {
    db: Mutex<Connection>,
    readers: Vec<Mutex<Connection>>,
    next_reader: AtomicU64,
    foreground_writers: Arc<AtomicU64>,
    work: Arc<work::Queue>,
    metadata_ready: std::sync::atomic::AtomicBool,
    query_cancellations: Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>,
    listener: Mutex<Option<futures::channel::mpsc::UnboundedSender<crate::tokio_bridge::UiEvent>>>,
    device: Mutex<String>,
    generation: AtomicU64,
    counts_generation: AtomicU64,
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
    work::start(&lib);
    lib.reconcile_index()?;
    let _ = LIBRARY.set(lib);
    Ok(LIBRARY.get().unwrap().clone())
}
impl Library {
    #[cfg(test)]
    fn open() -> Result<Self> {
        Self::open_database(tempfile::tempdir().map_err(err)?)
    }
    #[cfg(not(test))]
    fn open() -> Result<Self> {
        Self::open_database()
    }
    fn open_database(#[cfg(test)] test_directory: tempfile::TempDir) -> Result<Self> {
        // Ensure the canonical store has performed its own versioned migrations first.
        #[cfg(test)]
        let database_path = test_directory.path().join("state.sqlite3");
        #[cfg(not(test))]
        let database_path = deskfuse_store::SQLiteStore::default_path().map_err(err)?;
        let _canonical = deskfuse_store::SQLiteStore::open(&database_path).map_err(err)?;
        let mut db = Connection::open(&database_path).map_err(err)?;
        deskfuse_store::configure_connection(&mut db).map_err(err)?;
        query::configure(&db)?;
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
   CREATE TABLE IF NOT EXISTS cl_pending_publications(id INTEGER PRIMARY KEY,revision INTEGER NOT NULL);
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
        db.execute_batch(OPERATION_INDEXES).map_err(err)?;
        storage::install_clocks(&db)?;
        let metadata_ready = storage::install(&db)?;
        // Two readers keep UI/count queries concurrent while avoiding a third
        // long-lived SQLite page cache and file-descriptor set.
        let readers = (0..2)
            .map(|_| {
                let mut reader = Connection::open(&database_path).map_err(err)?;
                deskfuse_store::configure_connection(&mut reader).map_err(err)?;
                query::configure(&reader)?;
                reader
                    .pragma_update(None, "query_only", true)
                    .map_err(err)?;
                Ok(Mutex::new(reader))
            })
            .collect::<Result<Vec<_>>>()?;
        let this = Self {
            #[cfg(test)]
            _test_directory: test_directory,
            db: Mutex::new(db),
            readers,
            next_reader: AtomicU64::new(0),
            foreground_writers: Arc::new(AtomicU64::new(0)),
            work: Arc::new(work::Queue::default()),
            metadata_ready: storage::ready(metadata_ready),
            query_cancellations: Mutex::new(HashMap::new()),
            listener: Mutex::new(None),
            device: Mutex::new(String::new()),
            generation: AtomicU64::new(1),
            counts_generation: AtomicU64::new(1),
            state_cache: Mutex::new(None),
        };
        this.import_legacy()?;
        this.migrate_image_digests()?;
        this.seed_default_lists()?;
        #[cfg(test)]
        this.reconcile_index()?;
        Ok(this)
    }
    fn migrate_image_digests(&self) -> Result<()> {
        let mut db = self.writer()?;
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
                .query_row("SELECT COALESCE(payload,(SELECT bytes FROM cl_blobs WHERE hash=cl_items.blob_hash)) FROM cl_items WHERE id=?1", [id], |r| {
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
        let mut db = self.writer()?;
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
    fn reader(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        let index = self.next_reader.fetch_add(1, Ordering::Relaxed) as usize % self.readers.len();
        // Counts/media never force a page to queue behind a busy reader.
        for offset in 0..self.readers.len() {
            if let Ok(db) = self.readers[(index + offset) % self.readers.len()].try_lock() {
                return Ok(db);
            }
        }
        self.readers[index].lock().map_err(err)
    }
    fn writer(&self) -> Result<storage::Writer<'_>> {
        self.foreground_writers.fetch_add(1, Ordering::AcqRel);
        match self.db.lock() {
            Ok(db) => Ok(storage::Writer {
                db,
                foreground: &self.foreground_writers,
            }),
            Err(error) => {
                self.foreground_writers.fetch_sub(1, Ordering::AcqRel);
                Err(err(error))
            }
        }
    }
    /// Optional history bookkeeping never delays a successful pasteboard write.
    pub fn defer_used(&self, id: u64) {
        self.work.used(id);
    }
    fn publication_intent_db(&self, db: &Connection, id: u64) -> Result<()> {
        if !self.work.is_running() {
            return Ok(());
        }
        let revision: u64 = db.query_row("INSERT INTO cl_meta VALUES('publication_counter','1') ON CONFLICT(key) DO UPDATE SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT) RETURNING CAST(value AS INTEGER)", [], |r| r.get(0)).map_err(err)?;
        db.execute("INSERT INTO cl_pending_publications VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision", params![id,revision]).map_err(err)?;
        Ok(())
    }
    fn publication_committed(&self, id: u64) -> Result<()> {
        if self.work.is_running() {
            self.work.wake();
            Ok(())
        } else {
            self.publish_index_for(id)
        }
    }
    fn publish_index_later(&self, id: u64) -> Result<()> {
        if self.work.is_running() {
            let mut db = self.writer()?;
            let tx = db.transaction().map_err(err)?;
            self.publication_intent_db(&tx, id)?;
            tx.commit().map_err(err)?;
        }
        self.publication_committed(id)
    }
    fn changed(&self) {
        self.counts_generation.fetch_add(1, Ordering::AcqRel);
        self.changed_order();
    }
    fn changed_order(&self) {
        let revision = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if let Ok(mut cache) = self.state_cache.lock() {
            *cache = None;
        }
        if let Ok(listener) = self.listener.lock()
            && let Some(listener) = listener.as_ref()
        {
            let _ = listener.unbounded_send(crate::tokio_bridge::UiEvent::ClipboardLibraryChanged(
                revision,
            ));
        }
    }
    pub fn subscribe_changes(
        &self,
        listener: futures::channel::mpsc::UnboundedSender<crate::tokio_bridge::UiEvent>,
    ) {
        if let Ok(mut current) = self.listener.lock() {
            *current = Some(listener);
        }
    }
    /// Local reuse changes ordering, never the original source/copy metadata.
    /// Called only after the native pasteboard write has succeeded.
    pub fn mark_used(&self, id: u64) -> Result<()> {
        self.mark_used_at(id, now_ms())
    }
    fn mark_used_at(&self, id: u64, time: u64) -> Result<()> {
        let mut db = self.writer()?;
        let tx = db.transaction().map_err(err)?;
        let time: u64 = tx.query_row("INSERT INTO cl_meta(key,value) VALUES('usage_clock',?1) ON CONFLICT(key) DO UPDATE SET value=CAST(MAX(CAST(value AS INTEGER)+1,CAST(excluded.value AS INTEGER)) AS TEXT) RETURNING CAST(value AS INTEGER)", [time.to_string()], |r| r.get(0)).map_err(err)?;
        let updated = tx
            .execute(
                "UPDATE cl_items SET last_used=?1 WHERE id=?2",
                params![time, id],
            )
            .map_err(err)?;
        tx.commit().map_err(err)?;
        if updated > 0 {
            self.changed_order();
        }
        Ok(())
    }

    pub fn set_local_device(&self, id: String) -> Result<()> {
        *self.device.lock().map_err(err)? = id;
        if self.work.is_running() {
            self.writer()?
                .execute(
                    "INSERT OR REPLACE INTO cl_meta VALUES('publish_backfill_cursor','0')",
                    [],
                )
                .map_err(err)?;
            self.work.wake();
            return Ok(());
        }
        // Publish migrated hashes through normal sync operations, so old remote
        // index rows acquire the same identity without deleting anyone's data.
        let ids = {
            let db = self.writer()?;
            db.prepare("SELECT i.id FROM cl_items i LEFT JOIN cl_records r ON r.key='index:' || i.global_id WHERE i.ordinary=1 AND i.remote=0 AND i.digest LIKE 'image-v1:%' AND (r.key IS NULL OR COALESCE(json_extract(r.value,'$.digest'),'')<>i.digest)")
                .map_err(err)?.query_map([], |r| r.get::<_, u64>(0)).map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>().map_err(err)?
        };
        for id in ids {
            self.publish_index_later(id)?;
        }
        Ok(())
    }
    pub fn settings(&self) -> Result<Settings> {
        let db = self.reader()?;
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
            self.publication_committed(id)?;
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
            self.publication_committed(id)?;
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
        let warning = id.and_then(|id| self.publication_committed(id).err());
        Ok((id, warning))
    }
    fn ingest_record(&self, snapshot: &PasteboardSnapshot, source: &str) -> Result<Option<u64>> {
        let origin = self.origin(snapshot, source);
        if origin.no_history || self.excluded(origin.app_id.as_deref()) {
            return Ok(None);
        }
        let stripped;
        let snapshot: &PasteboardSnapshot = match temporary_file_action(snapshot) {
            TemporaryFileAction::Keep => snapshot,
            TemporaryFileAction::Skip => return Ok(None),
            TemporaryFileAction::Strip(s) => {
                stripped = s;
                &stripped
            }
        };
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
        let stripped;
        let snapshot: &PasteboardSnapshot = match temporary_file_action(snapshot) {
            TemporaryFileAction::Keep => snapshot,
            TemporaryFileAction::Skip => return Ok(None),
            TemporaryFileAction::Strip(s) => {
                stripped = s;
                &stripped
            }
        };
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
        let mut db = self.writer()?;
        let tx = db.transaction().map_err(err)?;
        let mut found: Option<u64> = tx
            .query_row(
                "SELECT id FROM cl_items WHERE digest=?1 AND ordinary=1 ORDER BY (payload IS NOT NULL OR EXISTS(SELECT 1 FROM cl_blobs WHERE hash=cl_items.blob_hash)) DESC,COALESCE(json_extract(summary,'$.file_count'),0)=0 DESC,id DESC LIMIT 1",
                [&fingerprint],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        if let Some(id) = found
            && fingerprint.starts_with("image-v1:")
            && !snapshot.file_urls.is_empty()
            && tx.query_row("SELECT (payload IS NOT NULL OR EXISTS(SELECT 1 FROM cl_blobs WHERE hash=cl_items.blob_hash)) AND COALESCE(json_extract(summary,'$.file_count'),0)=0 FROM cl_items WHERE id=?1", [id], |r| r.get::<_, bool>(0)).map_err(err)?
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
        tx.execute(
            "INSERT OR IGNORE INTO cl_blobs VALUES(?1,?2)",
            params![blob_hash, bytes],
        )
        .map_err(err)?;
        tx.execute("UPDATE cl_items SET payload=NULL,summary=?2,search_text=?3,copied=?4,imported=?5,origin_device=?6,blob_hash=?7,digest=?9,remote=0 WHERE id=?8",params![Option::<Vec<u8>>::None,serde_json::to_string(&s).map_err(err)?,search,origin.copied_at_ms,now_ms(),origin.device_id,blob_hash,id,fingerprint]).map_err(err)?;
        self.publication_intent_db(&tx, id)?;
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
        let row = {
            let db = self.reader()?;
            let sql = self.metadata_sql("SELECT i.global_id,i.digest,i.summary,COALESCE(i.blob_hash,''),i.title,i.ocr_text,COALESCE(i.search_text,'') FROM cl_items i WHERE i.id=?1");
            db.query_row(&sql, [id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })
            .optional()
            .map_err(err)?
        };
        let Some((gid, digest, summary, blob_hash, title, ocr_text, search_text)) = row else {
            return Ok(());
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
        if self.work.is_running() {
            self.writer()?.execute_batch("INSERT OR REPLACE INTO cl_meta VALUES('index_reconcile_cursor',''); INSERT OR REPLACE INTO cl_meta VALUES('publish_backfill_cursor','0');").map_err(err)?;
            self.work.wake();
            return Ok(());
        }
        let mut db = self.writer()?;
        let tx = db.transaction().map_err(err)?;
        reconcile_index_db(&tx)?;
        tx.commit().map_err(err)
    }

    pub fn snapshot(&self, id: u64) -> Result<PasteboardSnapshot> {
        let db = self.reader()?;
        let cutoff = now_ms().saturating_sub(settings_db(&db)?.retention_days as u64 * 86400000);
        let row: Option<(Option<Vec<u8>>, Option<String>, String)> = db
            .query_row(
                "SELECT COALESCE(payload,(SELECT bytes FROM cl_blobs WHERE hash=cl_items.blob_hash)),summary,COALESCE(blob_hash,'') FROM cl_items WHERE id=?1 AND (ordinary=0 OR (COALESCE(copied,imported)>=?2 AND id>?3))",
                params![id, cutoff, cleared_history_id(&db)?],
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
        let db = self.reader()?;
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
        let rows = {
            let db = self.reader()?;
            let cutoff =
                now_ms().saturating_sub(settings_db(&db)?.retention_days as u64 * 86400000);
            let unique = UNIQUE_IMAGE_ROW.replace(
                "d.ordinary=1 AND",
                &format!(
                    "d.ordinary=1 AND d.id>{} AND COALESCE(d.copied,d.imported)>={cutoff} AND",
                    cleared_history_id(&db)?
                ),
            );
            let sql = self.metadata_sql(&format!("SELECT i.id,i.summary FROM cl_items i WHERE i.ordinary=1 AND i.id>?3 AND i.payload IS NOT NULL AND COALESCE(i.copied,i.imported)>=?2 AND {unique} ORDER BY {QUERY_ORDER} LIMIT ?1"));
            db.prepare(&sql)
                .map_err(err)?
                .query_map(params![limit, cutoff, cleared_history_id(&db)?], |r| {
                    Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)?
        };
        let mut recent = Vec::new();
        let mut retained_bytes = 0usize;
        for (id, raw) in rows {
            let snapshot = self.snapshot(id)?;
            let size = crate::tokio_bridge::clipboard_snapshot_bytes(&snapshot);
            if !recent.is_empty()
                && retained_bytes.saturating_add(size)
                    > crate::tokio_bridge::CLIPBOARD_HOT_CACHE_BYTES
            {
                break;
            }
            let summary: Summary = serde_json::from_str(&raw).map_err(err)?;
            recent.push((id, snapshot, summary.source, summary.copied_at_ms));
            retained_bytes = retained_bytes.saturating_add(size);
        }
        Ok(recent)
    }
    pub fn prune(&self) -> Result<()> {
        let mut db = self.writer()?;
        let cutoff = now_ms().saturating_sub(settings_db(&db)?.retention_days as u64 * 86400000);
        let tx = db.transaction().map_err(err)?;
        let n = tx
            .execute(
                "DELETE FROM cl_items WHERE id IN (SELECT id FROM cl_items WHERE ordinary=1 AND (COALESCE(copied,imported)<?1 OR id<=?2) LIMIT 32)",
                params![cutoff,cleared_history_id(&tx)?],
            )
            .map_err(err)?;
        tx.commit().map_err(err)?;
        if n > 0 {
            self.changed()
        }
        Ok(())
    }
    pub fn clear_history(&self) -> Result<()> {
        // A single watermark makes bulk removal immediately visible. Reclaim
        // native bytes in cancellable background batches, preserving archives.
        self.writer()?.execute("INSERT INTO cl_meta SELECT 'cleared_history_id',CAST(COALESCE(MAX(id),0) AS TEXT) FROM cl_items WHERE true ON CONFLICT(key) DO UPDATE SET value=excluded.value", []).map_err(err)?;
        self.changed();
        self.work.cleanup();
        Ok(())
    }
    pub fn query(&self, request: &str) -> Result<String> {
        let q: Value = serde_json::from_str(request).map_err(err)?;
        if q["op"] == "cancel_query" {
            if let Some(id) = q["query_id"].as_str() {
                let mut cancellations = self.query_cancellations.lock().map_err(err)?;
                if cancellations.len() >= 512 {
                    cancellations.retain(|_, flag| !flag.load(Ordering::Acquire));
                }
                cancellations
                    .entry(id.into())
                    .or_insert_with(|| Arc::new(std::sync::atomic::AtomicBool::new(false)))
                    .store(true, Ordering::Release);
            }
            return Ok("{}".into());
        }
        if q["op"] == "revision" {
            return Ok(json!({"revision": self.generation()}).to_string());
        }
        if q["op"] == "state" {
            return self.state();
        }
        self.query_items(q)
    }
    fn query_counts_db(&self, db: &Connection, q: &Value) -> Result<String> {
        let lists = lists_db(db)?;
        let requests = q["requests"].as_array().ok_or("计数请求无效")?;
        if requests.len() != lists.len() + 1 {
            return Err("列表已变化，请刷新".into());
        }
        let mut counts = std::collections::BTreeMap::new();
        for request in requests {
            let key = request["list_id"].as_str().unwrap_or("all").to_string();
            let mut filter = request.clone();
            filter["query"] = json!("");
            filter["source"] = Value::Null;
            filter["kind"] = json!("all");
            filter["count_only"] = json!(true);
            let result: Value =
                serde_json::from_str(&self.query_items_db(db, filter)?).map_err(err)?;
            counts.insert(key, result["total"].clone());
        }
        serde_json::to_string(&counts).map_err(err)
    }
    fn state(&self) -> Result<String> {
        let generation = self.generation();
        if let Some((cached_generation, value)) = self.state_cache.lock().map_err(err)?.as_ref()
            && *cached_generation == generation
        {
            return Ok(value.clone());
        }
        let db = self.reader()?;
        let lists = lists_db(&db)?;
        let settings = settings_db(&db)?;
        let apps = db
            .prepare("SELECT id,name FROM cl_source_apps ORDER BY id")
            .map_err(err)?
            .query_map([], |r| {
                Ok(json!({"bundle_id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?}))
            })
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        let devices = db
            .prepare("SELECT id,name FROM cl_source_devices ORDER BY id")
            .map_err(err)?
            .query_map([], |r| {
                Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?}))
            })
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        let value = serde_json::to_string(&json!({"lists":lists,"settings":settings,"apps":apps,"devices":devices,"revision":generation,"counts_revision":self.counts_generation.load(Ordering::Acquire)})).map_err(err)?;
        *self.state_cache.lock().map_err(err)? = Some((generation, value.clone()));
        Ok(value)
    }
    fn query_items(&self, q: Value) -> Result<String> {
        let id = q["query_id"].as_str().map(str::to_owned);
        let started = std::time::Instant::now();
        let mut cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        if let Some(id) = &id {
            cancel = self
                .query_cancellations
                .lock()
                .map_err(err)?
                .entry(id.clone())
                .or_insert(cancel)
                .clone();
        }
        let db = match self.reader() {
            Ok(db) => db,
            Err(error) => {
                if let Some(id) = &id {
                    self.query_cancellations.lock().map_err(err)?.remove(id);
                }
                return Err(error);
            }
        };
        let progress_cancel = cancel.clone();
        db.progress_handler(1000, Some(move || progress_cancel.load(Ordering::Acquire)));
        let result = if cancel.load(Ordering::Acquire) {
            Err("查询已取消".into())
        } else if q["op"] == "counts" {
            self.query_counts_db(&db, &q)
        } else {
            self.query_items_db(&db, q)
        };
        db.progress_handler(0, None::<fn() -> bool>);
        let canceled = if let Some(id) = &id {
            let mut queries = self.query_cancellations.lock().map_err(err)?;
            let canceled = cancel.load(Ordering::Acquire);
            queries.remove(id);
            canceled
        } else {
            false
        };
        let result = if canceled {
            Err("查询已取消".into())
        } else {
            result
        };
        self.record_duration("query", id, started, result.is_ok());
        result
    }
    fn record_duration(
        &self,
        operation: &str,
        request: Option<String>,
        started: std::time::Instant,
        succeeded: bool,
    ) {
        if deskfuse_core::debug_logging::enabled() {
            static PERFORMANCE: OnceLock<crate::diagnostics::Diagnostics> = OnceLock::new();
            PERFORMANCE
                .get_or_init(|| crate::diagnostics::Diagnostics::new(128))
                .record_with_metadata(
                    crate::diagnostics::DiagnosticLevel::Info,
                    "clipboard_library",
                    operation,
                    request,
                    Some(started.elapsed().as_millis() as u64),
                    Some(format!("succeeded={succeeded}")),
                    None,
                );
        }
    }

    fn query_items_db(&self, db: &Connection, q: Value) -> Result<String> {
        let revision = self.generation();
        let cutoff = now_ms().saturating_sub(settings_db(db)?.retention_days as u64 * 86400000);
        let cleared = cleared_history_id(db)?;
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
        let mut clauses: Vec<String> = Vec::with_capacity(5);
        let mut filter_params: Vec<SqlValue> = Vec::with_capacity(8);
        if manual {
            // A manual list shows the frozen archive when one exists, and its
            // membership key is indexed by the primary key of cl_records.
            clauses.push("NOT (i.ordinary=1 AND EXISTS(SELECT 1 FROM cl_items a WHERE a.global_id='archive:' || i.global_id AND a.ordinary=0))".into());
            clauses.push("EXISTS(SELECT 1 FROM cl_records m WHERE m.key='member:' || ? || ':' || CASE WHEN i.ordinary=1 THEN 'archive:' || i.global_id ELSE i.global_id END AND m.deleted=0)".into());
            clauses.push("NOT EXISTS(SELECT 1 FROM cl_records tombstone WHERE tombstone.key='item:' || CASE WHEN i.ordinary=1 THEN 'archive:' || i.global_id ELSE i.global_id END AND tombstone.deleted=1)".into());
            filter_params.push(SqlValue::Text(list_id.unwrap_or_default().to_string()));
            clauses.push("i.id IN (SELECT a.id FROM cl_items a JOIN cl_records m ON a.global_id=json_extract(m.value,'$.item_id') WHERE m.key GLOB 'member:*' AND m.deleted=0 AND json_extract(m.value,'$.list_id')=? UNION SELECT base.id FROM cl_items base JOIN cl_records m ON base.global_id=substr(json_extract(m.value,'$.item_id'),9) WHERE m.key GLOB 'member:*' AND m.deleted=0 AND json_extract(m.value,'$.list_id')=? AND base.ordinary=1)".into());
            filter_params
                .extend((0..2).map(|_| SqlValue::Text(list_id.unwrap_or_default().to_string())));
        } else {
            clauses.push(UNIQUE_IMAGE_ROW.replace("d.ordinary=1 AND", &format!("d.ordinary=1 AND d.id>{cleared} AND COALESCE(d.copied,d.imported)>={cutoff} AND")));
            // An archive with a live ordinary source is hidden outside manual
            // lists. The old per-row SELECT EXISTS is now one indexed predicate.
            clauses.push(format!("NOT (i.ordinary=0 AND i.global_id LIKE 'archive:%' AND EXISTS(SELECT 1 FROM cl_items base WHERE base.global_id=substr(i.global_id,9) AND base.ordinary=1 AND base.id>{cleared} AND COALESCE(base.copied,base.imported)>={cutoff}))"));
            // Orphaned archive rows are never shown. Restrict the membership
            // probe to active list IDs so deleted-list tombstones stay hidden
            // without materializing every membership in Rust.
            clauses.push("(i.ordinary=1 OR EXISTS(SELECT 1 FROM cl_records m JOIN cl_records l ON l.key='list:' || json_extract(m.value,'$.list_id') AND l.deleted=0 WHERE m.key GLOB 'member:*' AND m.deleted=0 AND json_extract(m.value,'$.item_id')=i.global_id))".into());
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
                clauses.push("instr(unicode_lower(i.search_index),?)>0".into());
                filter_params.push(SqlValue::Text(query.clone()));
            } else {
                if self.metadata_ready.load(Ordering::Acquire) {
                    clauses.push("i.id IN (SELECT item_id FROM cl_short_terms WHERE term=? UNION SELECT item_id FROM cl_short_pending)".into());
                    filter_params.push(SqlValue::Text(query.clone()));
                }
                clauses.push("instr(unicode_lower(i.search_index),?)>0".into());
                filter_params.push(SqlValue::Text(query.clone()));
            }
        }
        clauses.push(format!(
            "(i.ordinary=0 OR (i.id>{cleared} AND COALESCE(i.copied,i.imported)>=?))"
        ));
        filter_params.push(SqlValue::Integer(cutoff.min(i64::MAX as u64) as i64));
        if kind != "all" {
            clauses.push(format!("({} = ?)", query::KIND));
            filter_params.push(SqlValue::Text(kind.to_string()));
        }
        if let Some(list) = selected.filter(|list| list.kind == "smart") {
            let rules = list
                .rules
                .iter()
                .enumerate()
                .map(|(index, rule)| query::rule(index, rule, &q, &mut filter_params))
                .collect::<Vec<_>>();
            if !rules.is_empty() {
                clauses.push(format!(
                    "({})",
                    rules.join(if list.match_mode == "any" {
                        " OR "
                    } else {
                        " AND "
                    })
                ));
            }
        }
        let where_sql = clauses.join(" AND ");
        let include_total = count_only || q["include_total"].as_bool().unwrap_or(true);
        {
            let count_sql = format!("SELECT COUNT(*) FROM cl_items i WHERE {where_sql}");
            let total: usize = if include_total {
                db.query_row(
                    &self.metadata_sql(&count_sql),
                    params_from_iter(filter_params.iter()),
                    |row| row.get::<_, i64>(0),
                )
                .map_err(err)?
                .max(0) as usize
            } else {
                0
            };

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
            let mut stmt = db.prepare(&self.metadata_sql(&page_sql)).map_err(err)?;
            let rows = stmt
                .query_map(params_from_iter(page_params.iter()), QueryRow::from_row)
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)?;
            let has_more = rows.len() > limit;
            let mut page = rows;
            page.truncate(limit);
            let memberships = memberships_for(db, &page, &lists)?;
            let related_ids = self.related_item_ids(db, &page)?;
            let items = page
                .iter()
                .map(|row| {
                    let mut value = self.query_item_value(
                        row,
                        memberships
                            .get(row.member_id())
                            .cloned()
                            .unwrap_or_default(),
                    )?;
                    value["related_ids"] = json!(
                        related_ids
                            .get(row.member_id())
                            .cloned()
                            .unwrap_or_else(|| vec![row.id])
                    );
                    Ok(value)
                })
                .collect::<Result<Vec<_>>>()?;
            let next_cursor = if has_more {
                page.last()
                    .map(|row| json!({"sort_at_ms": row.sort_at_ms.max(0) as u64, "id": row.id}))
            } else {
                None
            };
            serde_json::to_string(&json!({
                "items": items,
                "total": total,
                "total_exact": include_total,
                "next_offset": has_more.then_some(offset.saturating_add(limit)),
                "next_cursor": next_cursor,
                "revision": revision
            }))
            .map_err(err)
        }
    }

    /// Return action identities from small metadata rows, never native bodies.
    /// Deleting an archive must also hide its ordinary history in the hot UI.
    fn related_item_ids(
        &self,
        db: &Connection,
        rows: &[QueryRow],
    ) -> Result<HashMap<String, Vec<u64>>> {
        let mut result = HashMap::<String, Vec<u64>>::new();
        let mut statement =
            db.prepare(&self.metadata_sql(
                "SELECT i.id FROM cl_items i WHERE i.global_id=?1 OR i.global_id=?2",
            ))
            .map_err(err)?;
        for row in rows {
            let key = row.member_id();
            if result.contains_key(key) {
                continue;
            }
            let base = key.strip_prefix("archive:").unwrap_or(key);
            let ids = statement
                .query_map(params![key, base], |r| r.get::<_, u64>(0))
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)?;
            result.insert(key.to_owned(), ids);
        }
        Ok(result)
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
                let was_syncing = self.settings()?.sync_index;
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
                self.writer()?.execute("INSERT INTO cl_meta VALUES('settings',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[serde_json::to_string(&s).map_err(err)?]).map_err(err)?;
                self.changed();
                // Re-enabling index sync re-materializes already-stored records.
                if s.sync_index && !was_syncing {
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
                    let db = self.writer()?;
                    lists_db(&db)?
                };
                let by_id = lists
                    .iter()
                    .map(|l| (l.id.as_str(), l))
                    .collect::<HashMap<_, _>>();
                let mut changes = Vec::new();
                for (i, id) in ids.iter().enumerate() {
                    if let Some(l) = id.as_str().and_then(|id| by_id.get(id).copied()) {
                        if l.position == i as i64 {
                            continue;
                        }
                        let mut l = l.clone();
                        l.position = i as i64;
                        changes.push((
                            format!("list:{}", l.id),
                            serde_json::to_value(l).map_err(err)?,
                            false,
                        ));
                    }
                }
                self.local_records_with_delete(changes, &[])?;
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
                    let db = self.writer()?;
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
                let mut changes = Vec::new();
                for global_id in archived {
                    let members = {
                        let db = self.writer()?;
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
                        changes.push((key, serde_json::from_str(&value).map_err(err)?, true));
                    }
                    changes.push((format!("item:{global_id}"), json!({"id": global_id}), true));
                }
                self.local_records_with_delete(changes, &bases.into_iter().collect::<Vec<_>>())?;
            }
            "set_item_text" => {
                let id = q["id"].as_u64().ok_or("内容 ID 无效")?;
                let text: String = q["text"]
                    .as_str()
                    .ok_or("文本内容无效")?
                    .chars()
                    .take(1_000_000)
                    .collect();
                if text.trim().is_empty() {
                    return Err("文本内容不能为空".into());
                }
                let rtf: Option<Vec<u8>> = q
                    .get("rtf")
                    .filter(|v| !v.is_null())
                    .map(|v| serde_json::from_value::<Vec<u8>>(v.clone()).map_err(err))
                    .transpose()?
                    .filter(|v| !v.is_empty());
                let (gid, ordinary, remote, summary_raw) = {
                    let db = self.writer()?;
                    db.query_row(
                        "SELECT global_id,ordinary,COALESCE(remote,0),summary FROM cl_items WHERE id=?1",
                        [id],
                        |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, bool>(1)?,
                                r.get::<_, bool>(2)?,
                                r.get::<_, String>(3)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(err)?
                    .ok_or("内容不存在")?
                };
                // 远端索引行会被下一次 reconcile 覆盖，不支持原地编辑。
                if remote {
                    return Err("远端内容请先复制到本机再编辑".into());
                }
                let mut s: Summary = serde_json::from_str(&summary_raw).map_err(err)?;
                if s.file_count > 0 || s.has_image {
                    return Err("仅文本内容支持编辑".into());
                }
                // 确认内容本地可用；不可用时 snapshot 会排队按需拉取并返回等待错误。
                self.snapshot(id)?;
                let mut representations = vec![PasteboardRepresentation {
                    type_name: "public.utf8-plain-text".into(),
                    data: text.as_bytes().to_vec(),
                }];
                if let Some(rtf) = rtf {
                    representations.push(PasteboardRepresentation {
                        type_name: "public.rtf".into(),
                        data: rtf,
                    });
                }
                let edited = PasteboardSnapshot {
                    change_count: 0,
                    text: Some(text.clone()),
                    png: None,
                    file_urls: Vec::new(),
                    items: vec![PasteboardItem { representations }],
                };
                let bytes = encode(&edited, "编辑")?;
                let hash = hex::encode(Sha256::digest(&bytes));
                s.preview = text.chars().take(160).collect();
                s.length = text.len() as u64;
                s.mime = "text/plain".into();
                s.kind = "text".into();
                let search = crate::clipboard_echo::normalize_clipboard_text(&text).to_lowercase();
                if ordinary {
                    // 与 ingest 相同的 blob 化形态：payload 置空，内容经 blob_hash 引用。
                    let digest = content_digest(&edited)?;
                    let mut db = self.writer()?;
                    let tx = db.transaction().map_err(err)?;
                    tx.execute(
                        "INSERT OR IGNORE INTO cl_blobs VALUES(?1,?2)",
                        params![hash, bytes],
                    )
                    .map_err(err)?;
                    tx.execute(
                        "UPDATE cl_items SET digest=?1,summary=?2,payload=NULL,search_text=?3,blob_hash=?4 WHERE id=?5",
                        params![digest, serde_json::to_string(&s).map_err(err)?, search, hash, id],
                    )
                    .map_err(err)?;
                    self.publication_intent_db(&tx, id)?;
                    tx.commit().map_err(err)?;
                    drop(db);
                    self.changed();
                    self.publication_committed(id)?;
                } else {
                    // 归档内容经 item 记录同步；materialize 按新 blob 更新本地行。
                    self.write_blob(&hash, &bytes)?;
                    self.local_record(
                        format!("item:{gid}"),
                        json!({"id":gid,"summary":serde_json::to_value(&s).map_err(err)?,"blob_hash":hash}),
                        false,
                    )?;
                }
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
                let mut db = self.writer()?;
                let tx = db.transaction().map_err(err)?;
                let updated = tx
                    .execute(
                        "UPDATE cl_items SET title=?1 WHERE id=?2",
                        params![stored, id],
                    )
                    .map_err(err)?;
                if updated == 0 {
                    return Err("内容不存在".into());
                }
                self.publication_intent_db(&tx, id as u64)?;
                tx.commit().map_err(err)?;
                drop(db);
                self.changed();
                self.publication_committed(id as u64)?;
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
                let mut db = self.writer()?;
                let tx = db.transaction().map_err(err)?;
                let updated = tx
                    .execute(
                        "UPDATE cl_items SET ocr_text=?1,ocr_version=?2 WHERE id=?3",
                        params![if text.is_empty() { None } else { Some(text) }, version, id],
                    )
                    .map_err(err)?;
                if updated == 0 {
                    return Err("内容不存在".into());
                }
                self.publication_intent_db(&tx, id as u64)?;
                tx.commit().map_err(err)?;
                drop(db);
                self.changed();
                self.publication_committed(id as u64)?;
            }
            "want_item_payload" => {
                let id = q["id"].as_u64().ok_or("内容 ID 无效")? as i64;
                let hash: String = {
                    let db = self.writer()?;
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
                let db = self.writer()?;
                db.execute("DELETE FROM cl_items WHERE remote=1", [])
                    .map_err(err)?;
                drop(db);
                self.changed();
            }
            _ => return Err("未知剪贴板操作".into()),
        }
        if q["compact"].as_bool().unwrap_or(false) {
            let mut response = json!({"revision":self.generation(),"counts_revision":self.counts_generation.load(Ordering::Acquire)});
            let op = q["op"].as_str().unwrap_or("");
            if matches!(op, "save_list" | "delete_list" | "reorder") {
                let db = self.reader()?;
                response["lists"] = serde_json::to_value(lists_db(&db)?).map_err(err)?;
            }
            if op == "settings" {
                response["settings"] = serde_json::to_value(self.settings()?).map_err(err)?;
            }
            Ok(response.to_string())
        } else {
            self.state()
        }
    }
    fn membership(&self, list: &str, id: u64, present: bool) -> Result<()> {
        let (gid, summary, ordinary, body_hash) = {
            let db = self.writer()?;
            let manual: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM cl_records WHERE key=?1 AND deleted=0 AND json_extract(value,'$.kind')='manual')",
                [format!("list:{list}")], |r| r.get(0),
            ).map_err(err)?;
            if !manual {
                return Err("只能加入手动列表".into());
            }
            db.query_row(
                "SELECT global_id,summary,ordinary,blob_hash FROM cl_items WHERE id=?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, bool>(2)?,
                        r.get::<_, Option<String>>(3)?,
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
            let db = self.writer()?;
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM cl_items WHERE global_id=?1 AND ordinary=0)",
                [&gid],
                |row| row.get::<_, bool>(0),
            )
            .map_err(err)?
        } else {
            false
        };
        // Reuse the frozen archive across lists. A refreshed ordinary row can
        // have a different app/date; it must not replace already pinned content.
        // A metadata-only remote archive can join another list while its original
        // blob is still syncing, without being overwritten by local history.
        if present && !existing_archive {
            // A short synced text is complete in its summary; freeze that text
            // instead of failing until the richer payload transfer arrives.
            let summary_value: Value = serde_json::from_str(&summary).map_err(err)?;
            if summary_value["file_count"].as_u64().unwrap_or(0) == 0
                && let Some(hash) = body_hash
            {
                let available: bool = self
                    .reader()?
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM cl_blobs WHERE hash=?1)",
                        [&hash],
                        |r| r.get(0),
                    )
                    .map_err(err)?;
                if available {
                    return self.local_records_with_delete(
                        vec![
                            (
                                format!("item:{gid}"),
                                json!({"id":gid,"summary":summary_value,"blob_hash":hash}),
                                false,
                            ),
                            (
                                format!("member:{list}:{gid}"),
                                json!({"list_id":list,"item_id":gid}),
                                false,
                            ),
                        ],
                        &[],
                    );
                }
            }
            let snapshot = self.snapshot(id)?;
            let device = self.device.lock().map_err(err)?.clone();
            let snapshot = crate::library_files::register(&snapshot, &device)?;
            let bytes = encode(&snapshot, "归档")?;
            let hash = hex::encode(Sha256::digest(&bytes));
            self.write_blob(&hash, &bytes)?;
            let summary: Value = serde_json::from_str(&summary).map_err(err)?;
            return self.local_records_with_delete(
                vec![
                    (
                        format!("item:{gid}"),
                        json!({"id":gid,"summary":summary,"blob_hash":hash}),
                        false,
                    ),
                    (
                        format!("member:{list}:{gid}"),
                        json!({"list_id":list,"item_id":gid}),
                        false,
                    ),
                ],
                &[],
            );
        }
        self.local_record(
            format!("member:{list}:{gid}"),
            json!({"list_id":list,"item_id":gid}),
            !present,
        )
    }
    pub fn export_operations(&self, after: u64, limit: usize) -> Result<Vec<Value>> {
        let db = self.reader()?;
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
        self.local_records_with_delete(vec![(key, value, deleted)], &[])
    }
    fn local_records_with_delete(
        &self,
        changes: Vec<(String, Value, bool)>,
        bases: &[String],
    ) -> Result<()> {
        if changes.is_empty() {
            return self.merge_operations_with_delete(Vec::new(), bases);
        }
        let author = self.device.lock().map_err(err)?.clone();
        if author.is_empty() {
            return Err("设备身份尚未就绪".into());
        }
        let mut db = self.writer()?;
        let tx = db.transaction().map_err(err)?;
        let previous: u64 = tx
            .query_row(
                "SELECT COALESCE(MAX(revision),0) FROM cl_author_clocks",
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
        let mut revision = previous.max(local);
        let mut clock = std::collections::BTreeMap::<String, u64>::new();
        {
            let mut statement = tx
                .prepare("SELECT author,revision FROM cl_author_clocks")
                .map_err(err)?;
            for row in statement
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))
                .map_err(err)?
            {
                let (author, revision) = row.map_err(err)?;
                clock.insert(author, revision);
            }
        }
        let mut operations = Vec::with_capacity(changes.len());
        for (key, value, deleted) in changes {
            revision = revision
                .checked_add(1)
                .filter(|v| *v <= i64::MAX as u64)
                .ok_or("列表版本已溢出")?;
            clock.insert(author.clone(), revision);
            operations.push(json!({"op_id":uuid::Uuid::new_v4().to_string(),"key":key,"revision":revision,"author":author,"clock":clock,"deleted":deleted,"value":value}));
        }
        tx.execute("INSERT INTO cl_meta VALUES('counter',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[revision.to_string()]).map_err(err)?;
        tx.commit().map_err(err)?;
        drop(db);
        self.merge_operations_with_delete(operations, bases)
    }

    pub fn merge_operations(&self, ops: Vec<Value>) -> Result<()> {
        self.merge_operations_with_delete(ops, &[])
    }
    fn merge_operations_with_delete(&self, ops: Vec<Value>, bases: &[String]) -> Result<()> {
        let mut db = self.writer()?;
        let tx = db.transaction().map_err(err)?;
        let sync_index = settings_db(&tx)?.sync_index;
        let mut changed = false;
        let mut reconcile_items = HashSet::new();
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
            changed |= !key.starts_with("index:");
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
                let winner = member_winner(&tx, key, &op)?;
                tx.execute("INSERT INTO cl_clocks VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, serde_json::to_string(&winner["clock"]).map_err(err)?]).map_err(err)?;
                tx.execute("INSERT INTO cl_records VALUES(?1,?2,?3,?4,?5) ON CONFLICT(key) DO UPDATE SET revision=excluded.revision,author=excluded.author,deleted=excluded.deleted,value=excluded.value",
                    params![key, winner["revision"].as_u64(), winner["author"].as_str(), winner["deleted"].as_bool(), serde_json::to_string(&winner["value"]).map_err(err)?]).map_err(err)?;
                if let Some(id) = value["item_id"].as_str() {
                    reconcile_items.insert(id.to_string());
                }
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
                if let Some(id) = key.strip_prefix("item:") {
                    reconcile_items.insert(id.to_string());
                }
                if let Some(list) = key.strip_prefix("list:")
                    && previous
                        .as_ref()
                        .is_none_or(|(_, _, old_deleted)| *old_deleted != deleted)
                {
                    let mut members = tx.prepare("SELECT json_extract(value,'$.item_id') FROM cl_records WHERE key GLOB 'member:*' AND json_extract(value,'$.list_id')=?1").map_err(err)?;
                    for item in members
                        .query_map([list], |r| r.get::<_, String>(0))
                        .map_err(err)?
                    {
                        reconcile_items.insert(item.map_err(err)?);
                    }
                }
                tx.execute("INSERT INTO cl_clocks VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,serde_json::to_string(&clock).map_err(err)?]).map_err(err)?;
                tx.execute("INSERT INTO cl_records VALUES(?1,?2,?3,?4,?5) ON CONFLICT(key) DO UPDATE SET revision=excluded.revision,author=excluded.author,deleted=excluded.deleted,value=excluded.value",params![key,revision,author,deleted,serde_json::to_string(value).map_err(err)?]).map_err(err)?;
                if key.starts_with("item:") && !deleted {
                    materialize(&tx, value)?;
                }
                if key.starts_with("index:") && !deleted && sync_index {
                    let before = tx.total_changes();
                    materialize_index(&tx, value, author)?;
                    changed |= tx.total_changes() > before;
                }
            }
        }
        if !reconcile_items.is_empty() {
            reconcile_archives(&tx, &reconcile_items)?;
        }
        for base in bases {
            changed |= tx
                .execute(
                    "DELETE FROM cl_items WHERE global_id=?1 OR global_id=?2",
                    params![base, format!("archive:{base}")],
                )
                .map_err(err)?
                > 0;
        }
        tx.commit().map_err(err)?;
        drop(db);
        if changed {
            self.changed()
        }
        Ok(())
    }
    pub fn read_blob(&self, id: &str) -> Result<Option<Vec<u8>>> {
        let db = self.reader()?;
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
        let text = decode(bytes)?.text.unwrap_or_default().to_lowercase();
        let db = self.writer()?;
        db.execute(
            "INSERT OR IGNORE INTO cl_blobs VALUES(?1,?2)",
            params![id, bytes],
        )
        .map_err(err)?;
        db.execute(
            "UPDATE cl_items SET payload=NULL,search_text=?2 WHERE blob_hash=?1 AND (payload IS NOT NULL OR search_text IS NOT ?2)",
            params![id,text],
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
        let db = self.writer()?;
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
        let db = self.writer()?;
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
fn cleared_history_id(db: &Connection) -> Result<u64> {
    Ok(db
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM cl_meta WHERE key='cleared_history_id'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?
        .unwrap_or(0))
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
        .map(|_| "json_extract(value,'$.item_id')=?")
        .collect::<Vec<_>>()
        .join(" OR ");
    let sql = format!(
        "SELECT value FROM cl_records WHERE key GLOB 'member:*' AND deleted=0 AND ({predicates}) AND NOT EXISTS(SELECT 1 FROM cl_records tombstone WHERE tombstone.key='item:' || json_extract(cl_records.value,'$.item_id') AND tombstone.deleted=1)"
    );
    let params = item_ids
        .iter()
        .map(|item_id| SqlValue::Text(item_id.clone()))
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
fn materialize(db: &Connection, v: &Value) -> Result<()> {
    let gid = v["id"].as_str().ok_or("归档ID无效")?;
    let hash = v["blob_hash"].as_str().ok_or("归档摘要无效")?;
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("归档摘要无效".into());
    }
    let mut s: Summary = serde_json::from_value(v["summary"].clone()).map_err(err)?;
    let existing: Option<(u64, Option<String>, bool)> = db
        .query_row(
            "SELECT id,blob_hash,(payload IS NOT NULL OR EXISTS(SELECT 1 FROM cl_blobs WHERE hash=cl_items.blob_hash)) FROM cl_items WHERE global_id=?1",
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
            "UPDATE cl_items SET summary=?1,copied=?2 WHERE id=?3 AND (summary IS NOT ?1 OR copied IS NOT ?2)",
            params![serde_json::to_string(&s).map_err(err)?, s.copied_at_ms, id],
        )
        .map_err(err)?;
        return Ok(());
    }
    let existing_text: Option<String> = db
        .query_row(
            "SELECT search_text FROM cl_items WHERE blob_hash=?1 LIMIT 1",
            [hash],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    let text = if let Some(text) = existing_text {
        text
    } else {
        let bytes: Option<Vec<u8>> = db
            .query_row("SELECT bytes FROM cl_blobs WHERE hash=?1", [hash], |r| {
                r.get(0)
            })
            .optional()
            .map_err(err)?;
        bytes
            .as_deref()
            .map(decode)
            .transpose()?
            .and_then(|s| s.text)
            .unwrap_or_default()
            .to_lowercase()
    };
    db.execute("INSERT OR IGNORE INTO cl_items(global_id,digest,summary,payload,search_text,copied,imported,ordinary,blob_hash,origin_device) VALUES(?1,?2,?3,?4,?5,?6,?7,0,?8,?9)",params![gid,hash,serde_json::to_string(&s).map_err(err)?,Option::<Vec<u8>>::None,text,s.copied_at_ms,now_ms(),hash,s.source_device_id]).map_err(err)?;
    s.id = db
        .query_row("SELECT id FROM cl_items WHERE global_id=?1", [gid], |r| {
            r.get(0)
        })
        .map_err(err)?;
    db.execute(
        "UPDATE cl_items SET summary=?1,payload=?3,search_text=?4,blob_hash=?5,digest=?5,copied=?6,origin_device=?7 WHERE global_id=?2",
        params![serde_json::to_string(&s).map_err(err)?, gid, Option::<Vec<u8>>::None, text, hash, s.copied_at_ms, s.source_device_id],
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
#[cfg(test)]
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
fn reconcile_archives(db: &Connection, seen: &HashSet<String>) -> Result<()> {
    for id in seen {
        let deleted: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM cl_records WHERE key=?1 AND deleted=1)",
                [format!("item:{id}")],
                |row| row.get(0),
            )
            .map_err(err)?;
        let active: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM cl_records m JOIN cl_records l ON l.key='list:' || json_extract(m.value,'$.list_id') AND l.deleted=0 WHERE m.key GLOB 'member:*' AND m.deleted=0 AND json_extract(m.value,'$.item_id')=?1)", [id], |r| r.get(0)).map_err(err)?;
        if !deleted && active {
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
            db.execute(
                "DELETE FROM cl_items WHERE global_id=?1 AND ordinary=0",
                [id],
            )
            .map_err(err)?;
        }
    }
    Ok(())
}

/// Return the remove-wins register value over the maximal causal operations.
/// The query is indexed by record key and never loads clipboard payloads.
fn member_winner(db: &Connection, key: &str, incoming: &Value) -> Result<Value> {
    type Clock = std::collections::BTreeMap<String, u64>;
    let mut frontier: Vec<(Value, Clock)> = Vec::new();
    let ready: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM cl_member_frontier_state WHERE key=?1)",
            [key],
            |r| r.get(0),
        )
        .map_err(err)?;
    let sql = if ready {
        "SELECT body FROM cl_member_frontier WHERE key=?1"
    } else {
        "SELECT body FROM cl_operations WHERE json_extract(body,'$.key')=?1 ORDER BY seq"
    };
    let mut statement = db.prepare(sql).map_err(err)?;
    let mut rows = statement
        .query_map([key], |row| row.get::<_, String>(0))
        .map_err(err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)?;
    if ready {
        rows.push(serde_json::to_string(incoming).map_err(err)?);
    }
    let dominates = |a: &Clock, b: &Clock| {
        b.iter()
            .all(|(device, revision)| a.get(device).unwrap_or(&0) >= revision)
    };
    for row in rows {
        let value: Value = serde_json::from_str(&row).map_err(err)?;
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
    db.execute("DELETE FROM cl_member_frontier WHERE key=?1", [key])
        .map_err(err)?;
    for (value, _) in &frontier {
        db.execute(
            "INSERT INTO cl_member_frontier VALUES(?1,?2,?3)",
            params![
                key,
                value["op_id"].as_str(),
                serde_json::to_string(value).map_err(err)?
            ],
        )
        .map_err(err)?;
    }
    db.execute(
        "INSERT OR IGNORE INTO cl_member_frontier_state VALUES(?1)",
        [key],
    )
    .map_err(err)?;
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

#[cfg(test)]
mod temp_file_tests {
    use super::*;
    use deskfuse_platform::{PasteboardItem, PasteboardRepresentation};

    #[test]
    fn temporary_clipboard_file_covers_known_staging_locations() {
        // 系统临时目录与应用容器 tmp
        for path in [
            "/tmp/uu.tmp",
            "/private/tmp/uu.tmp",
            "/var/folders/m5/abc/T/unidesk-file-paste-1234/a.bin",
            "/private/var/folders/m5/abc/T/uu.tmp",
            "/Users/godd/Library/Containers/com.microsoft.rdc.macos/Data/tmp/ABC/1.png",
        ] {
            assert!(
                temporary_clipboard_file(std::path::Path::new(path)),
                "{path} 应判定为临时文件"
            );
        }
        // 通用剪贴板（接力）共享暂存目录
        assert!(temporary_clipboard_file(std::path::Path::new(
            "/Users/godd/Library/Group Containers/group.com.apple.coreservices.useractivityd/shared-pasteboard/items/UUID/a.jsonl"
        )));
        // 其他应用 Application Support 下的隐藏暂存文件（网易 UU 远程）
        assert!(temporary_clipboard_file(std::path::Path::new(
            "/Users/godd/Library/Application Support/com.netease.uuremote.server/Clipboard/.uuremote_aeaws1"
        )));
        // 缓存目录：其他应用算临时，DeskFuse 自身接收缓存除外
        let cache = dirs::cache_dir().unwrap();
        assert!(temporary_clipboard_file(
            &cache.join("WeType/dsclp/mac_1.png")
        ));
        assert!(!temporary_clipboard_file(
            &cache.join("com.deskfuse.app/clipboard/t-1/a.jpg")
        ));
        assert!(!temporary_clipboard_file(
            &cache.join("com.unidesk.app/clipboard/t-1/a.jpg")
        ));
        // 正常位置不算临时文件
        for path in [
            "/Users/godd/Downloads/IMG_8452.HEIC",
            "/Users/godd/Documents/.hidden-note",
            "/Users/godd/Library/Application Support/com.netease.uuremote.server/Clipboard/visible.txt",
        ] {
            assert!(
                !temporary_clipboard_file(std::path::Path::new(path)),
                "{path} 不应判定为临时文件"
            );
        }
    }

    fn file_snapshot(path: &str) -> PasteboardSnapshot {
        PasteboardSnapshot {
            file_urls: vec![path.into()],
            ..Default::default()
        }
    }

    #[test]
    fn ingest_skips_snapshots_with_only_temporary_files() {
        let library = Library::open().unwrap();
        let snapshot = file_snapshot("/var/folders/m5/abc/T/uu-remote-tmp");
        assert_eq!(library.ingest(&snapshot, "本机").unwrap(), None);
        let count: i64 = {
            let db = library.db.lock().unwrap();
            db.query_row("SELECT COUNT(*) FROM cl_items", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(count, 0);
    }

    #[test]
    fn ingest_keeps_remaining_content_when_temporary_file_path_is_stripped() {
        let library = Library::open().unwrap();
        let mut snapshot = file_snapshot("/var/folders/m5/abc/T/staged.png");
        snapshot.png = Some(vec![137, 80, 78, 71]);
        snapshot.items = vec![PasteboardItem {
            representations: vec![PasteboardRepresentation {
                type_name: "public.png".into(),
                data: vec![137, 80, 78, 71],
            }],
        }];
        let id = library.ingest(&snapshot, "本机").unwrap().unwrap();
        let raw: String = {
            let db = library.db.lock().unwrap();
            db.query_row("SELECT summary FROM cl_items WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .unwrap()
        };
        let summary: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(summary["kind"], "image");
        assert_eq!(summary["file_count"], 0);
        assert!(library.snapshot(id).unwrap().file_urls.is_empty());
    }

    #[test]
    fn ingest_keeps_real_file_copies() {
        let library = Library::open().unwrap();
        let snapshot = file_snapshot("/Users/godd/Downloads/IMG_8452.HEIC");
        assert!(library.ingest(&snapshot, "本机").unwrap().is_some());
    }

    #[test]
    fn missing_files_step_deletes_dead_file_entries() {
        let library = Library::open().unwrap();
        let directory = tempfile::tempdir().unwrap();
        // 用一个一定不属于临时目录判定的路径形态不现实（测试临时目录本身就在
        // $TMPDIR 下），因此直接写入库行，绕过入库过滤。
        let alive = directory.path().join("alive.txt");
        std::fs::write(&alive, b"alive").unwrap();
        let dead = directory.path().join("dead.txt");
        std::fs::write(&dead, b"dead").unwrap();
        let insert = |path: &std::path::Path| {
            let snapshot = file_snapshot(&path.to_string_lossy());
            let bytes = encode(&snapshot, "test").unwrap();
            let s = json!({"id":0,"copied_at_ms":1,"mime":"application/x-deskfuse-file-clipboard;version=1",
                "length":1,"preview":"fixture","source":"test","has_image":false,
                "file_count":1,"kind":"files"});
            let db = library.db.lock().unwrap();
            db.execute(
                "INSERT INTO cl_items(global_id,digest,summary,imported,blob_hash) VALUES(?1,?1,?2,1,?3)",
                params![uuid::Uuid::new_v4().to_string(), s.to_string(), hex::encode(Sha256::digest(&bytes))],
            )
            .unwrap();
            let id = db.last_insert_rowid() as u64;
            db.execute(
                "INSERT INTO cl_blobs VALUES(?1,?2)",
                params![hex::encode(Sha256::digest(&bytes)), bytes],
            )
            .unwrap();
            id
        };
        let alive_id = insert(&alive);
        insert(&dead);
        std::fs::remove_file(&dead).unwrap();

        let (cursor, more) = library.collect_missing_files_step(0).unwrap();
        assert!(!more);
        assert_eq!(cursor, 0);
        let remaining: Vec<u64> = {
            let db = library.db.lock().unwrap();
            db.prepare("SELECT id FROM cl_items ORDER BY id")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(remaining, vec![alive_id]);
    }
}
