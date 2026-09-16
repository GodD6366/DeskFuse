use unidesk_collaboration::ContentHandle;
use unidesk_store_library::{ContentEntry, ContentRepository, SearchIndex};

/// Indexes applied clipboard content into the library store.
pub struct Indexer {
    repo: ContentRepository,
}

impl Indexer {
    #[must_use]
    pub fn new(repo: ContentRepository) -> Self {
        Self { repo }
    }

    pub fn index(&self, handle: &ContentHandle, now: i64) -> unidesk_store_library::Result<()> {
        let title = title_for(handle);
        self.repo.insert(&ContentEntry {
            id: handle.id.to_string(),
            content_type: handle.mime.clone(),
            title: Some(title.clone()),
            snippet: Some(title),
            data_hash: Some(hex32(handle.digest)),
            size: Some(handle.size as i64),
            created_at: now,
            indexed_at: now,
        })
    }
}

pub fn title_for(handle: &ContentHandle) -> String {
    if handle.mime.starts_with("image/") {
        "图片".into()
    } else if handle.mime.starts_with("text/") {
        "文本片段".into()
    } else {
        handle.mime.clone()
    }
}

fn hex32(bytes: [u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Re-export so hosts can run search without reaching into the store crate.
pub type LibrarySearch = SearchIndex;
