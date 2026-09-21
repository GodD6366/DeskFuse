use super::ClipboardHistoryStore;

impl ClipboardHistoryStore {
    pub(super) fn open() -> Result<Self, String> {
        let library = crate::clipboard_library::global()?;
        let mut history = Self::default();
        for (id, snapshot, source, copied_at_ms) in library.recent(50)?.into_iter().rev() {
            if history.insert(snapshot, source) {
                history.adopt_front_id(id);
                if let Some((item, _)) = history.items.front_mut() {
                    item.copied_at_ms = copied_at_ms;
                }
            }
            history.next_id = history.next_id.max(id);
        }
        Ok(history)
    }
    pub(super) fn persist_retained(&self) -> Result<(), String> {
        // This deque is only the bounded presentation cache, never the durable retention set.
        crate::clipboard_library::global()?.prune()
    }
}
