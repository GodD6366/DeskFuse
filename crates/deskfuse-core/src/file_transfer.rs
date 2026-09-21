//! User policy for receiving clipboard files and retaining application-owned copies.
use serde::{Deserialize, Serialize};

/// Local receiving policy. All sizes are bytes, with MiB used by the UI.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileTransferSettings {
    /// Request small clipboard files before the user pastes them.
    pub small_file_prefetch_enabled: bool,
    /// Largest complete transfer eligible for prefetch, inclusive.
    pub small_file_threshold_bytes: u64,
    /// Maximum sum of committed cache data and active transfer reservations.
    pub cache_max_bytes: u64,
}
impl Default for FileTransferSettings {
    fn default() -> Self {
        Self {
            small_file_prefetch_enabled: false,
            small_file_threshold_bytes: 10 * 1024 * 1024,
            cache_max_bytes: 1024 * 1024 * 1024,
        }
    }
}
impl FileTransferSettings {
    /// Validate a policy before persistence or application.
    pub fn validate(&self) -> Result<(), String> {
        if self.small_file_threshold_bytes == 0 || self.small_file_threshold_bytes > 1024_u64.pow(4)
        {
            return Err("小文件阈值必须大于 0 且不超过 1 TiB".into());
        }
        if self.cache_max_bytes < 1024 * 1024 || self.cache_max_bytes > 16 * 1024_u64.pow(4) {
            return Err("缓存上限必须在 1 MiB 到 16 TiB 之间".into());
        }
        if self.small_file_prefetch_enabled
            && self.small_file_threshold_bytes > self.cache_max_bytes
        {
            return Err("小文件阈值不能超过缓存上限".into());
        }
        Ok(())
    }
    /// Whether a metadata offer can be requested without a user paste.
    pub fn should_prefetch(&self, total_bytes: u64) -> bool {
        self.small_file_prefetch_enabled
            && total_bytes <= self.small_file_threshold_bytes
            && total_bytes <= self.cache_max_bytes
    }
}
/// Current cache accounting, suitable for settings and transfer history.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TransferCacheSnapshot {
    /// Logical bytes currently stored, including private staging.
    pub used_bytes: u64,
    /// Full expected sizes reserved by active transfers (includes written bytes).
    pub active_reserved_bytes: u64,
    /// Configured limit in bytes.
    pub max_bytes: u64,
    /// Finished cache directories that can be cleared.
    pub completed_transfers: usize,
    /// Transfers protected from cleanup.
    pub active_transfers: usize,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_is_lazy_and_prefetch_boundary_is_inclusive() {
        let mut settings = FileTransferSettings::default();
        assert!(!settings.should_prefetch(1));
        settings.small_file_prefetch_enabled = true;
        assert!(settings.should_prefetch(settings.small_file_threshold_bytes));
        assert!(!settings.should_prefetch(settings.small_file_threshold_bytes + 1));
        settings.cache_max_bytes = 1024 * 1024;
        assert!(settings.validate().is_err());
    }
}
