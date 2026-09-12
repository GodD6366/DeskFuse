use super::{ClipboardHistoryStore, PasteboardSnapshot, SQLiteStore};
use bincode::Options;

// Keep native content separate from lossy display previews. The SQLite table
// version identifies this binary encoding, including the snapshot field order.
#[derive(serde::Serialize, serde::Deserialize)]
struct SnapshotV1 {
    source: String,
    snapshot: PasteboardSnapshot,
}

impl ClipboardHistoryStore {
    pub(super) fn open() -> Result<Self, String> {
        #[cfg(not(test))]
        let database = SQLiteStore::new();
        #[cfg(test)]
        let database = SQLiteStore::open_in_memory();
        let database = database.map_err(|error| format!("剪贴板历史数据库打开失败：{error}"))?;
        let rows = database
            .clipboard_history()
            .map_err(|error| format!("剪贴板历史读取失败：{error}"))?;
        let next_id = rows.iter().map(|(id, _, _)| *id).max().unwrap_or(0);
        let mut history = Self::default();
        for (id, bytes, copied_at_ms) in rows.into_iter().rev() {
            let entry: SnapshotV1 = bincode::DefaultOptions::new()
                .with_limit(bytes.len() as u64)
                .deserialize(&bytes)
                .map_err(|_| "剪贴板历史读取失败：记录格式无效，已保留数据库".to_owned())?;
            if history.insert(entry.snapshot, entry.source)
                && let Some((item, _)) = history.items.front_mut()
            {
                item.id = id;
                // Loading retained history is not a new copy event.
                item.copied_at_ms = copied_at_ms;
            }
        }
        history.next_id = next_id;
        history.database = Some(database);
        Ok(history)
    }

    pub(super) fn persist_latest(&self) -> Result<(), String> {
        let Some(database) = &self.database else {
            return Ok(());
        };
        let Some((item, snapshot)) = self.items.front() else {
            return self.persist_retained();
        };
        let payload = bincode::DefaultOptions::new()
            .serialize(&SnapshotV1 {
                source: item.source.clone(),
                snapshot: snapshot.clone(),
            })
            .map_err(|_| "剪贴板历史保存失败：内容编码失败".to_owned())?;
        let ids: Vec<_> = self.items.iter().map(|(item, _)| item.id).collect();
        database
            .save_clipboard_history(Some((item.id, &payload, item.copied_at_ms)), &ids)
            .map_err(|error| format!("剪贴板历史保存失败：{error}"))
    }

    pub(super) fn persist_retained(&self) -> Result<(), String> {
        let Some(database) = &self.database else {
            return Ok(());
        };
        let ids: Vec<_> = self.items.iter().map(|(item, _)| item.id).collect();
        database
            .save_clipboard_history(None, &ids)
            .map_err(|error| format!("剪贴板历史清理失败：{error}"))
    }
}
