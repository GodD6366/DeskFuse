use unidesk_update::ReleaseCache;

/// Host-facing download helper. Artifact bytes are never logged.
#[derive(Debug, Default)]
pub struct Downloader;

impl Downloader {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    pub fn cache_type_name(&self) -> &'static str {
        let _ = std::any::type_name::<ReleaseCache>();
        "release-cache"
    }
}
