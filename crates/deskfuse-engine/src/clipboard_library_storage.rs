//! Small query rows and immutable payload references. Existing stores migrate
//! in bounded transactions; the legacy query remains correct until backfill ends.
use super::*;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::AtomicBool;

pub(super) struct Writer<'a> {
    pub db: std::sync::MutexGuard<'a, Connection>,
    pub foreground: &'a AtomicU64,
}
impl Deref for Writer<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.db
    }
}
impl DerefMut for Writer<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.db
    }
}
impl Drop for Writer<'_> {
    fn drop(&mut self) {
        self.foreground.fetch_sub(1, Ordering::AcqRel);
    }
}

struct BackgroundWriter<'a>(std::sync::MutexGuard<'a, Connection>);
impl Deref for BackgroundWriter<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.0
    }
}
impl DerefMut for BackgroundWriter<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.0
    }
}
impl Drop for BackgroundWriter<'_> {
    fn drop(&mut self) {
        self.0.progress_handler(0, None::<fn() -> bool>);
    }
}

pub(super) fn install_clocks(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS cl_author_clocks(author TEXT PRIMARY KEY,revision INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS cl_member_frontier(key TEXT NOT NULL,op_id TEXT NOT NULL,body TEXT NOT NULL,PRIMARY KEY(key,op_id));
        CREATE TABLE IF NOT EXISTS cl_member_frontier_state(key TEXT PRIMARY KEY);
        CREATE TRIGGER IF NOT EXISTS cl_operations_clock_ai AFTER INSERT ON cl_operations BEGIN
            INSERT INTO cl_author_clocks VALUES(json_extract(new.body,'$.author'),json_extract(new.body,'$.revision')) ON CONFLICT(author) DO UPDATE SET revision=MAX(revision,excluded.revision);
        END;").map_err(err)?;
    let ready: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM cl_meta WHERE key='author_clocks_v1')",
            [],
            |r| r.get(0),
        )
        .map_err(err)?;
    if !ready {
        // One covering-index migration; normal writes touch only the author row.
        db.execute_batch("BEGIN;
            INSERT INTO cl_author_clocks SELECT json_extract(body,'$.author'),MAX(json_extract(body,'$.revision')) FROM cl_operations GROUP BY json_extract(body,'$.author') ON CONFLICT(author) DO UPDATE SET revision=MAX(revision,excluded.revision);
            INSERT INTO cl_meta VALUES('author_clocks_v1','1'); COMMIT;").map_err(err)?;
    }
    Ok(())
}

pub(super) fn install(db: &Connection) -> Result<bool> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS cl_item_metadata(
        id INTEGER PRIMARY KEY,global_id TEXT UNIQUE,digest TEXT,summary TEXT,
        payload INTEGER,search_text TEXT,search_index TEXT,copied INTEGER,
        imported INTEGER,ordinary INTEGER,blob_hash TEXT,origin_device TEXT,
        title TEXT,ocr_text TEXT,ocr_version INTEGER,remote INTEGER,last_used INTEGER);
        CREATE INDEX IF NOT EXISTS cl_metadata_page ON cl_item_metadata(MAX(COALESCE(last_used,0),COALESCE(copied,imported)) DESC,id DESC);
        CREATE INDEX IF NOT EXISTS cl_metadata_image ON cl_item_metadata(digest,ordinary,(payload IS NOT NULL),(COALESCE(json_extract(summary,'$.file_count'),0)=0),id);
        CREATE INDEX IF NOT EXISTS cl_metadata_expiry ON cl_item_metadata(ordinary,COALESCE(copied,imported));
        CREATE INDEX IF NOT EXISTS cl_metadata_last_used ON cl_item_metadata(last_used);
        CREATE INDEX IF NOT EXISTS cl_metadata_blob ON cl_item_metadata(blob_hash);
        CREATE INDEX IF NOT EXISTS cl_metadata_app ON cl_item_metadata(json_extract(summary,'$.source_app_id'),MAX(COALESCE(last_used,0),COALESCE(copied,imported)) DESC,id DESC);
        CREATE INDEX IF NOT EXISTS cl_metadata_device ON cl_item_metadata(json_extract(summary,'$.source_device_id'),MAX(COALESCE(last_used,0),COALESCE(copied,imported)) DESC,id DESC);
        CREATE TABLE IF NOT EXISTS cl_short_terms(term TEXT NOT NULL,item_id INTEGER NOT NULL,PRIMARY KEY(term,item_id));
        CREATE INDEX IF NOT EXISTS cl_short_terms_item ON cl_short_terms(item_id);
        CREATE TABLE IF NOT EXISTS cl_short_pending(item_id INTEGER PRIMARY KEY,cursor INTEGER NOT NULL DEFAULT 0,revision INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS cl_legacy_payloads(id INTEGER PRIMARY KEY);
        CREATE TABLE IF NOT EXISTS cl_blob_birth(hash TEXT PRIMARY KEY,created INTEGER NOT NULL);
        CREATE TRIGGER IF NOT EXISTS cl_blob_birth_ai AFTER INSERT ON cl_blobs BEGIN INSERT OR IGNORE INTO cl_blob_birth VALUES(new.hash,CAST(strftime('%s','now') AS INTEGER)*1000); END;
        CREATE TRIGGER IF NOT EXISTS cl_blob_birth_ad AFTER DELETE ON cl_blobs BEGIN DELETE FROM cl_blob_birth WHERE hash=old.hash; END;
        CREATE TABLE IF NOT EXISTS cl_source_apps(id TEXT PRIMARY KEY,name TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS cl_source_devices(id TEXT PRIMARY KEY,name TEXT NOT NULL);
        CREATE TRIGGER IF NOT EXISTS cl_short_search_au AFTER UPDATE OF search_index ON cl_items BEGIN
            INSERT INTO cl_short_pending(item_id) VALUES(new.id) ON CONFLICT(item_id) DO UPDATE SET cursor=0,revision=revision+1;
        END;
        CREATE TRIGGER IF NOT EXISTS cl_short_search_ai AFTER INSERT ON cl_items BEGIN
            INSERT INTO cl_short_pending(item_id) VALUES(new.id) ON CONFLICT(item_id) DO UPDATE SET cursor=0,revision=revision+1;
        END;
        CREATE TRIGGER IF NOT EXISTS cl_short_search_ad AFTER DELETE ON cl_items BEGIN
            DELETE FROM cl_short_pending WHERE item_id=old.id;
            DELETE FROM cl_short_terms WHERE item_id=old.id;
        END;").map_err(err)?;
    let columns = "id,global_id,digest,summary,payload,search_text,search_index,copied,imported,ordinary,blob_hash,origin_device,title,ocr_text,ocr_version,remote,last_used";
    let values = "new.id,new.global_id,new.digest,new.summary,CASE WHEN new.payload IS NOT NULL OR EXISTS(SELECT 1 FROM cl_blobs WHERE hash=new.blob_hash) THEN 1 END,new.search_text,new.search_index,new.copied,new.imported,new.ordinary,new.blob_hash,new.origin_device,new.title,new.ocr_text,new.ocr_version,new.remote,new.last_used";
    db.execute_batch(&format!(
        "CREATE INDEX IF NOT EXISTS cl_metadata_kind ON cl_item_metadata(({}),MAX(COALESCE(last_used,0),COALESCE(copied,imported)) DESC,id DESC);",
        super::query::KIND.replace("i.", "")
    ))
    .map_err(err)?;
    for (name, action) in [("ai", "INSERT"), ("au", "UPDATE")] {
        db.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS cl_metadata_{name} AFTER {action} ON cl_items BEGIN
            INSERT OR REPLACE INTO cl_item_metadata({columns}) VALUES({values});
            DELETE FROM cl_legacy_payloads WHERE id=new.id AND new.payload IS NULL;
            INSERT OR IGNORE INTO cl_legacy_payloads SELECT new.id WHERE new.payload IS NOT NULL;
            INSERT INTO cl_source_apps SELECT json_extract(new.summary,'$.source_app_id'),COALESCE(json_extract(new.summary,'$.source_app_name'),json_extract(new.summary,'$.source_app_id')) WHERE json_extract(new.summary,'$.source_app_id') IS NOT NULL ON CONFLICT(id) DO UPDATE SET name=excluded.name;
            INSERT INTO cl_source_devices SELECT json_extract(new.summary,'$.source_device_id'),COALESCE(json_extract(new.summary,'$.source'),'') WHERE json_extract(new.summary,'$.source_device_id') IS NOT NULL ON CONFLICT(id) DO UPDATE SET name=excluded.name;
            END;")).map_err(err)?;
    }
    db.execute_batch("CREATE TRIGGER IF NOT EXISTS cl_metadata_ad AFTER DELETE ON cl_items BEGIN DELETE FROM cl_item_metadata WHERE id=old.id; DELETE FROM cl_legacy_payloads WHERE id=old.id; END;
        CREATE TRIGGER IF NOT EXISTS cl_metadata_blob_ai AFTER INSERT ON cl_blobs BEGIN UPDATE cl_item_metadata SET payload=1 WHERE blob_hash=new.hash; END;").map_err(err)?;
    Ok(db
        .query_row(
            "SELECT value FROM cl_meta WHERE key='metadata_ready_v1'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(err)?
        .as_deref()
        == Some("1"))
}

impl Library {
    pub(super) fn metadata_sql(&self, sql: &str) -> String {
        if !self.metadata_ready.load(Ordering::Acquire) {
            return sql.replace("i.payload IS NOT NULL", "(i.payload IS NOT NULL OR EXISTS(SELECT 1 FROM cl_blobs WHERE hash=i.blob_hash))")
                .replace("d.payload IS NOT NULL", "(d.payload IS NOT NULL OR EXISTS(SELECT 1 FROM cl_blobs WHERE hash=d.blob_hash))");
        }
        sql.replace("cl_items i", "cl_item_metadata i")
            .replace("cl_items d", "cl_item_metadata d")
            .replace("cl_items a", "cl_item_metadata a")
            .replace("cl_items base", "cl_item_metadata base")
    }
    pub(super) fn storage_step(&self) -> Result<bool> {
        if !self.metadata_ready.load(Ordering::Acquire) {
            let Some(mut db) = self.background_writer()? else {
                return Ok(false);
            };
            let tx = db.transaction().map_err(err)?;
            let cursor: i64 = tx
                .query_row(
                    "SELECT CAST(value AS INTEGER) FROM cl_meta WHERE key='metadata_cursor_v1'",
                    [],
                    |r| r.get(0),
                )
                .optional()
                .map_err(err)?
                .unwrap_or(0);
            let next: Option<i64> = tx.query_row("SELECT MAX(id) FROM (SELECT id FROM cl_items WHERE id>?1 ORDER BY id LIMIT 256)", [cursor], |r| r.get(0)).map_err(err)?;
            if let Some(next) = next {
                tx.execute("INSERT OR IGNORE INTO cl_item_metadata SELECT id,global_id,digest,summary,CASE WHEN payload IS NOT NULL OR EXISTS(SELECT 1 FROM cl_blobs WHERE hash=cl_items.blob_hash) THEN 1 END,search_text,search_index,copied,imported,ordinary,blob_hash,origin_device,title,ocr_text,ocr_version,remote,last_used FROM cl_items WHERE id>?1 AND id<=?2", params![cursor,next]).map_err(err)?;
                tx.execute("INSERT OR IGNORE INTO cl_short_pending(item_id) SELECT id FROM cl_items WHERE id>?1 AND id<=?2", params![cursor,next]).map_err(err)?;
                tx.execute("INSERT OR IGNORE INTO cl_legacy_payloads SELECT id FROM cl_items WHERE id>?1 AND id<=?2 AND payload IS NOT NULL", params![cursor,next]).map_err(err)?;
                tx.execute("INSERT OR IGNORE INTO cl_source_apps SELECT json_extract(summary,'$.source_app_id'),COALESCE(json_extract(summary,'$.source_app_name'),json_extract(summary,'$.source_app_id')) FROM cl_item_metadata WHERE id>?1 AND id<=?2 AND json_extract(summary,'$.source_app_id') IS NOT NULL", params![cursor,next]).map_err(err)?;
                tx.execute("INSERT OR IGNORE INTO cl_source_devices SELECT json_extract(summary,'$.source_device_id'),COALESCE(json_extract(summary,'$.source'),'') FROM cl_item_metadata WHERE id>?1 AND id<=?2 AND json_extract(summary,'$.source_device_id') IS NOT NULL", params![cursor,next]).map_err(err)?;
                tx.execute("INSERT INTO cl_meta VALUES('metadata_cursor_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [next.to_string()]).map_err(err)?;
                tx.commit().map_err(err)?;
                return Ok(true);
            }
            tx.execute("INSERT INTO cl_meta SELECT 'usage_clock',CAST(COALESCE(MAX(last_used),0) AS TEXT) FROM cl_item_metadata WHERE true ON CONFLICT(key) DO UPDATE SET value=CAST(MAX(CAST(value AS INTEGER),CAST(excluded.value AS INTEGER)) AS TEXT)", []).map_err(err)?;
            tx.execute(
                "INSERT OR REPLACE INTO cl_meta VALUES('metadata_ready_v1','1')",
                [],
            )
            .map_err(err)?;
            tx.commit().map_err(err)?;
            self.metadata_ready.store(true, Ordering::Release);
            self.changed();
        }
        {
            let Some(db) = self.background_writer()? else {
                return Ok(false);
            };
            let ready: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM cl_meta WHERE key='body_lookup_index_v1')",
                    [],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if !ready {
                db.execute_batch("CREATE INDEX IF NOT EXISTS cl_items_blob_hash ON cl_items(blob_hash); INSERT OR REPLACE INTO cl_meta VALUES('body_lookup_index_v1','1');").map_err(err)?;
                return Ok(true);
            }
        }
        if self.reconcile_index_step()? || self.publish_backfill_step()? {
            return Ok(true);
        }
        self.index_short_step()
    }

    fn reconcile_index_step(&self) -> Result<bool> {
        let Some(mut db) = self.background_writer()? else {
            return Ok(false);
        };
        let cursor: Option<String> = db
            .query_row(
                "SELECT value FROM cl_meta WHERE key='index_reconcile_cursor'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        let Some(cursor) = cursor else {
            return Ok(false);
        };
        let rows = db.prepare("SELECT key,value,author FROM cl_records WHERE key GLOB 'index:*' AND key>?1 AND deleted=0 ORDER BY key LIMIT 16").map_err(err)?.query_map([cursor], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(err)?.collect::<std::result::Result<Vec<_>,_>>().map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        let before = tx.total_changes();
        if settings_db(&tx)?.sync_index {
            for (_, raw, author) in &rows {
                materialize_index(&tx, &serde_json::from_str(raw).map_err(err)?, author)?;
            }
        }
        let changed = tx.total_changes() > before;
        if let Some((key, _, _)) = rows.last() {
            tx.execute(
                "UPDATE cl_meta SET value=?1 WHERE key='index_reconcile_cursor'",
                [key],
            )
            .map_err(err)?;
        } else {
            tx.execute("DELETE FROM cl_meta WHERE key='index_reconcile_cursor'", [])
                .map_err(err)?;
        }
        tx.commit().map_err(err)?;
        if changed {
            self.changed();
        }
        Ok(!rows.is_empty())
    }
    fn publish_backfill_step(&self) -> Result<bool> {
        let Some(mut db) = self.background_writer()? else {
            return Ok(false);
        };
        let cursor: Option<u64> = db
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM cl_meta WHERE key='publish_backfill_cursor'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        let Some(cursor) = cursor else {
            return Ok(false);
        };
        let next: Option<u64> = db.query_row("SELECT MAX(id) FROM (SELECT id FROM cl_item_metadata WHERE id>?1 ORDER BY id LIMIT 256)", [cursor], |r| r.get(0)).map_err(err)?;
        let tx = db.transaction().map_err(err)?;
        if let Some(next) = next {
            if settings_db(&tx)?.sync_index {
                let revision: u64 = tx.query_row("INSERT INTO cl_meta VALUES('publication_counter','1') ON CONFLICT(key) DO UPDATE SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT) RETURNING CAST(value AS INTEGER)", [], |r|r.get(0)).map_err(err)?;
                tx.execute("INSERT INTO cl_pending_publications SELECT i.id,?3 FROM cl_item_metadata i LEFT JOIN cl_records r ON r.key='index:' || i.global_id WHERE i.id>?1 AND i.id<=?2 AND i.ordinary=1 AND i.remote=0 AND ((r.key IS NULL AND i.digest LIKE 'image-v1:%') OR (r.key IS NOT NULL AND (COALESCE(json_extract(r.value,'$.digest'),'')<>i.digest OR COALESCE(json_extract(r.value,'$.title'),'')<>COALESCE(i.title,'') OR COALESCE(json_extract(r.value,'$.ocr_text'),'')<>COALESCE(i.ocr_text,'') OR COALESCE(json_extract(r.value,'$.blob_hash'),'')<>COALESCE(i.blob_hash,'')))) ON CONFLICT(id) DO NOTHING", params![cursor,next,revision]).map_err(err)?;
            }
            tx.execute(
                "UPDATE cl_meta SET value=?1 WHERE key='publish_backfill_cursor'",
                [next.to_string()],
            )
            .map_err(err)?;
        } else {
            tx.execute(
                "DELETE FROM cl_meta WHERE key='publish_backfill_cursor'",
                [],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)?;
        Ok(next.is_some())
    }
    fn background_writer(&self) -> Result<Option<BackgroundWriter<'_>>> {
        if self.foreground_writers.load(Ordering::Acquire) != 0 {
            return Ok(None);
        }
        let db = match self.db.try_lock() {
            Ok(db) => db,
            Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
            Err(error) => return Err(err(error)),
        };
        let foreground = self.foreground_writers.clone();
        db.progress_handler(1000, Some(move || foreground.load(Ordering::Acquire) != 0));
        Ok(Some(BackgroundWriter(db)))
    }
    fn index_short_step(&self) -> Result<bool> {
        // Index at most 512 bytes per transaction. Keep the pending marker until
        // complete, so queries remain exact throughout interrupted backfills.
        let batch = {
            let db = self.reader()?;
            let row = db.query_row("SELECT i.id,p.cursor,p.revision FROM cl_short_pending p JOIN cl_item_metadata i ON i.id=p.item_id ORDER BY p.item_id LIMIT 1", [], |r| Ok((r.get::<_,u64>(0)?,r.get::<_,usize>(1)?,r.get::<_,u64>(2)?))).optional().map_err(err)?;
            if let Some((id, cursor, revision)) = row {
                let document = db
                    .blob_open(
                        rusqlite::DatabaseName::Main,
                        "cl_item_metadata",
                        "search_index",
                        id as i64,
                        true,
                    )
                    .map_err(err)?;
                let length = document.len();
                let cursor = cursor.min(length);
                let start = cursor.saturating_sub(4);
                let mut bytes = vec![0; (length - start).min(520)];
                document.read_at_exact(&mut bytes, start).map_err(err)?;
                let mut leading = 0;
                while leading < bytes.len() && bytes[leading] & 0xc0 == 0x80 {
                    leading += 1;
                }
                let mut extent = (cursor + 512).min(length) - start;
                while std::str::from_utf8(&bytes[leading..extent]).is_err() {
                    extent -= 1;
                }
                let text = std::str::from_utf8(&bytes[leading..extent])
                    .map_err(err)?
                    .to_owned();
                Some((id, cursor, revision, start + extent, length, text))
            } else {
                None
            }
        };
        let Some((id, cursor, revision, end, length, text)) = batch else {
            return self.migrate_payload_step();
        };
        let mut terms = HashSet::new();
        let mut previous = None;
        for ch in text.chars().flat_map(char::to_lowercase) {
            terms.insert(ch.to_string());
            if let Some(before) = previous {
                terms.insert(format!("{before}{ch}"));
            }
            previous = Some(ch);
        }
        let Some(mut db) = self.background_writer()? else {
            return Ok(false);
        };
        let tx = db.transaction().map_err(err)?;
        let unchanged: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM cl_short_pending WHERE item_id=?1 AND revision=?2 AND cursor=?3)", params![id,revision,cursor], |r| r.get(0)).map_err(err)?;
        if unchanged {
            if cursor == 0 {
                tx.execute("DELETE FROM cl_short_terms WHERE item_id=?1", [id])
                    .map_err(err)?;
            }
            {
                let mut insert = tx
                    .prepare_cached("INSERT OR IGNORE INTO cl_short_terms VALUES(?1,?2)")
                    .map_err(err)?;
                for term in terms {
                    insert.execute(params![term, id]).map_err(err)?;
                }
            }
            if end == length {
                tx.execute("DELETE FROM cl_short_pending WHERE item_id=?1", [id])
                    .map_err(err)?;
            } else {
                tx.execute(
                    "UPDATE cl_short_pending SET cursor=?2 WHERE item_id=?1",
                    params![id, end],
                )
                .map_err(err)?;
            }
        }
        tx.commit().map_err(err)?;
        Ok(true)
    }
    pub(super) fn migrate_payload_step(&self) -> Result<bool> {
        // Pending IDs prevent scanning all normalized rows on every idle tick.
        // Old rows without a hash are hashed through a bounded read buffer,
        // outside the writer. No original body is duplicated in Rust memory.
        let row = {
            let db = self.reader()?;
            db.query_row("SELECT i.id,i.blob_hash FROM cl_legacy_payloads p JOIN cl_items i ON i.id=p.id ORDER BY p.id LIMIT 1", [], |r| Ok((r.get::<_,u64>(0)?,r.get::<_,Option<String>>(1)?))).optional().map_err(err)?
        };
        let Some((id, old_hash)) = row else {
            return Ok(false);
        };
        let hash = if let Some(hash) = &old_hash {
            hash.clone()
        } else {
            let db = self.reader()?;
            let body = db
                .blob_open(
                    rusqlite::DatabaseName::Main,
                    "cl_items",
                    "payload",
                    id as i64,
                    true,
                )
                .map_err(err)?;
            let mut buffer = vec![0; 65_536];
            let mut digest = Sha256::new();
            let mut offset = 0;
            while offset < body.len() {
                if self.foreground_writers.load(Ordering::Acquire) != 0 {
                    return Ok(false);
                }
                let count = buffer.len().min(body.len() - offset);
                body.read_at_exact(&mut buffer[..count], offset)
                    .map_err(err)?;
                digest.update(&buffer[..count]);
                offset += count;
            }
            hex::encode(digest.finalize())
        };
        let Some(mut db) = self.background_writer()? else {
            return Ok(false);
        };
        let tx = db.transaction().map_err(err)?;
        tx.execute("INSERT OR IGNORE INTO cl_blobs SELECT ?2,payload FROM cl_items WHERE id=?1 AND blob_hash IS ?3 AND payload IS NOT NULL", params![id,hash,old_hash]).map_err(err)?;
        tx.execute("UPDATE cl_items SET payload=NULL,blob_hash=?2 WHERE id=?1 AND blob_hash IS ?3 AND EXISTS(SELECT 1 FROM cl_blobs WHERE hash=?2)", params![id,hash,old_hash]).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(true)
    }
    pub(super) fn prune_step(&self) -> Result<bool> {
        let Some(db) = self.background_writer()? else {
            return Ok(false);
        };
        let cutoff = now_ms().saturating_sub(settings_db(&db)?.retention_days as u64 * 86400000);
        let deleted = db.execute("DELETE FROM cl_items WHERE id IN (SELECT id FROM cl_items WHERE ordinary=1 AND (COALESCE(copied,imported)<?1 OR id<=?2) LIMIT 32)", params![cutoff,cleared_history_id(&db)?]).map_err(err)?;
        if deleted > 0 {
            self.changed();
        }
        Ok(deleted > 0)
    }
    pub(super) fn collect_blobs_step(&self) -> Result<()> {
        let Some(mut db) = self.background_writer()? else {
            return Ok(());
        };
        let tx = db.transaction().map_err(err)?;
        tx.execute("DELETE FROM cl_blobs WHERE hash IN (SELECT b.hash FROM cl_blobs b WHERE NOT EXISTS(SELECT 1 FROM cl_items WHERE blob_hash=b.hash) AND NOT EXISTS(SELECT 1 FROM cl_blob_birth WHERE hash=b.hash AND created>?1) LIMIT 8)", [now_ms().saturating_sub(600_000)]).map_err(err)?;
        tx.commit().map_err(err)
    }
    /// 分批扫描本机文件类历史条目，删除任一源路径已不存在的死引用。
    /// 游标按 id 推进，扫完一轮后回到表头重新开始；远端索引投影（remote=1）
    /// 与手动列表归档（ordinary=0）不在清理范围内。
    pub(super) fn collect_missing_files_step(&self, cursor: u64) -> Result<(u64, bool)> {
        const BATCH: u64 = 16;
        let Some(db) = self.background_writer()? else {
            return Ok((cursor, false));
        };
        let rows: Vec<(u64, Option<Vec<u8>>)> = db
            .prepare(
                "SELECT id, COALESCE(payload,(SELECT bytes FROM cl_blobs WHERE hash=cl_items.blob_hash)) FROM cl_items WHERE ordinary=1 AND remote=0 AND json_extract(summary,'$.kind')='files' AND id>?1 ORDER BY id LIMIT ?2",
            )
            .map_err(err)?
            .query_map(params![cursor, BATCH], |r| {
                Ok((r.get::<_, u64>(0)?, r.get::<_, Option<Vec<u8>>>(1)?))
            })
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        if rows.is_empty() {
            return Ok((0, false));
        }
        let next_cursor = rows.last().map(|(id, _)| *id).unwrap_or(cursor);
        let exhausted = (rows.len() as u64) < BATCH;
        let mut missing = Vec::new();
        for (id, bytes) in &rows {
            let dead = bytes
                .as_deref()
                .and_then(|bytes| decode(bytes).ok())
                .is_some_and(|snapshot| {
                    !snapshot.file_urls.is_empty()
                        && snapshot
                            .file_urls
                            .iter()
                            .any(|path| !std::path::Path::new(path).exists())
                });
            if dead {
                missing.push(*id);
            }
        }
        if !missing.is_empty() {
            db.execute(
                &format!(
                    "DELETE FROM cl_items WHERE id IN ({})",
                    missing.iter().map(|_| "?").collect::<Vec<_>>().join(",")
                ),
                params_from_iter(missing.iter()),
            )
            .map_err(err)?;
            self.changed();
        }
        // 不足一批说明已扫到表尾，下一轮从表头重新开始。
        Ok((if exhausted { 0 } else { next_cursor }, !exhausted))
    }
}

pub(super) fn ready(value: bool) -> AtomicBool {
    AtomicBool::new(value)
}
