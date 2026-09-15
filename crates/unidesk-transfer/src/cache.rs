//! Quota and retention for the application's private file clipboard cache.
//!
//! Only canonical UUID child directories are owned. No user source or paste
//! destination is ever passed to this manager, and symlinks are never followed.
use crate::TransferId;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::SystemTime,
};
use unidesk_core::TransferCacheSnapshot;

/// Synchronous leases owned by UI/native file promise lifetimes.
pub type ExternalCacheLeases = Arc<Mutex<HashMap<TransferId, Vec<String>>>>;

#[derive(Debug)]
pub struct TransferCache {
    root: PathBuf,
    max_bytes: u64,
    active: HashMap<TransferId, u64>,
    pins: HashMap<TransferId, usize>,
    external: ExternalCacheLeases,
    partial_cleanup: CacheCleanup,
}
#[derive(Debug, Default)]
pub struct CacheCleanup {
    pub removed_bytes: u64,
    pub removed_paths: Vec<PathBuf>,
    pub failed_transfers: usize,
}
impl TransferCache {
    pub fn new(root: PathBuf, max_bytes: u64) -> Result<Self, String> {
        // Existing ancestors may not be replaced with links into user folders.
        let mut parent = Some(root.as_path());
        while let Some(path) = parent {
            match fs::symlink_metadata(path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err("缓存目录不能是符号链接".into());
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
            parent = path.parent();
        }
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        Ok(Self {
            root,
            max_bytes,
            active: HashMap::new(),
            pins: HashMap::new(),
            external: Arc::default(),
            partial_cleanup: CacheCleanup::default(),
        })
    }
    pub fn set_external_leases(&mut self, leases: ExternalCacheLeases) {
        self.external = leases;
    }
    pub fn take_partial_cleanup(&mut self) -> CacheCleanup {
        std::mem::take(&mut self.partial_cleanup)
    }
    fn externally_pinned(&self, id: TransferId) -> bool {
        self.external
            .lock()
            .map(|leases| {
                leases
                    .values()
                    .flatten()
                    .any(|path| Path::new(path).strip_prefix(self.destination(id)).is_ok())
            })
            .unwrap_or(true)
    }
    fn protected(&self, id: TransferId) -> bool {
        self.active.contains_key(&id) || self.pins.contains_key(&id) || self.externally_pinned(id)
    }
    fn remove_unpinned(&self, id: TransferId, path: &Path) -> Result<bool, String> {
        let leases = self.external.lock().map_err(|_| "缓存导出租期锁不可用")?;
        if leases
            .values()
            .flatten()
            .any(|source| Path::new(source).strip_prefix(self.destination(id)).is_ok())
        {
            return Ok(false);
        }
        remove_owned_directory(&self.root, path)?;
        drop(leases);
        Ok(true)
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn destination(&self, id: TransferId) -> PathBuf {
        self.root.join(id.to_string())
    }
    fn entries(&self) -> Result<Vec<(TransferId, PathBuf, u64, SystemTime)>, String> {
        let mut result = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Ok(id) = name.parse::<TransferId>() else {
                continue;
            };
            if id.to_string() != name {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                continue;
            }
            result.push((
                id,
                entry.path(),
                tree_bytes(&entry.path())?,
                metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ));
        }
        result.sort_by_key(|entry| entry.3);
        Ok(result)
    }
    /// Pin cached files currently being exported to another device. Inputs are
    /// inspected lexically only; paths outside the private root are ignored.
    pub fn pin_sources(&mut self, paths: &[String]) -> Vec<TransferId> {
        let ids = paths
            .iter()
            .filter_map(|path| {
                Path::new(path)
                    .strip_prefix(&self.root)
                    .ok()?
                    .components()
                    .next()?
                    .as_os_str()
                    .to_str()?
                    .parse::<TransferId>()
                    .ok()
            })
            .collect::<std::collections::BTreeSet<_>>();
        for id in &ids {
            *self.pins.entry(*id).or_default() += 1;
        }
        ids.into_iter().collect()
    }
    pub fn unpin_sources(&mut self, ids: &[TransferId]) {
        for id in ids {
            if let Some(count) = self.pins.get_mut(id) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.pins.remove(id);
                }
            }
        }
    }
    fn protected_bytes(&self, entries: &[(TransferId, PathBuf, u64, SystemTime)]) -> u64 {
        self.active.values().sum::<u64>().saturating_add(
            entries
                .iter()
                .filter(|entry| {
                    (self.pins.contains_key(&entry.0) || self.externally_pinned(entry.0))
                        && !self.active.contains_key(&entry.0)
                })
                .map(|entry| entry.2)
                .sum::<u64>(),
        )
    }
    pub fn snapshot(&self) -> Result<TransferCacheSnapshot, String> {
        let entries = self.entries()?;
        Ok(TransferCacheSnapshot {
            used_bytes: entries.iter().map(|e| e.2).sum(),
            active_reserved_bytes: self.protected_bytes(&entries),
            max_bytes: self.max_bytes,
            completed_transfers: entries.iter().filter(|e| !self.protected(e.0)).count(),
            active_transfers: self
                .active
                .keys()
                .chain(self.pins.keys())
                .copied()
                .chain(
                    entries
                        .iter()
                        .filter(|entry| self.externally_pinned(entry.0))
                        .map(|entry| entry.0),
                )
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
        })
    }
    pub fn set_limit(&mut self, limit: u64) -> Result<CacheCleanup, String> {
        if self.protected_bytes(&self.entries()?) > limit {
            return Err("新上限小于正在传输的文件总量，请先取消任务或等待完成".into());
        }
        self.max_bytes = limit;
        self.make_room(0)
    }
    pub fn reserve(&mut self, id: TransferId, size: u64) -> Result<CacheCleanup, String> {
        if self.active.contains_key(&id) {
            return Ok(CacheCleanup::default());
        }
        let destination = self.destination(id);
        if destination.exists() && tree_bytes(&destination)? > 0 {
            return Err("传输标识已被已接收文件使用，请重新复制生成新请求".into());
        }
        let cleanup = self.make_room(size)?;
        self.active.insert(id, size);
        Ok(cleanup)
    }
    fn make_room(&mut self, required: u64) -> Result<CacheCleanup, String> {
        let entries = self.entries()?;
        let protected = self.protected_bytes(&entries);
        let reserved = self.active.values().sum::<u64>();
        if required.saturating_add(protected) > self.max_bytes {
            return Err(format!(
                "缓存空间不足：任务需要 {required} 字节，上限 {} 字节，活动任务已预留 {reserved} 字节",
                self.max_bytes
            ));
        }
        let mut accounted = reserved.saturating_add(
            entries
                .iter()
                .filter(|e| !self.active.contains_key(&e.0))
                .map(|e| e.2)
                .sum::<u64>(),
        );
        let mut result = CacheCleanup::default();
        for (id, path, size, _) in entries {
            if accounted.saturating_add(required) <= self.max_bytes {
                break;
            }
            if self.protected(id) {
                continue;
            }
            match self.remove_unpinned(id, &path) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => {
                    result.failed_transfers += 1;
                    self.partial_cleanup = result;
                    return Err(error);
                }
            }
            accounted = accounted.saturating_sub(size);
            result.removed_bytes = result.removed_bytes.saturating_add(size);
            result.removed_paths.push(path);
        }
        if accounted.saturating_add(required) > self.max_bytes {
            self.partial_cleanup = result;
            return Err("剩余空间被正在使用的文件占用，请完成粘贴或导出后重试".into());
        }
        Ok(result)
    }
    pub fn finish(&mut self, id: TransferId) {
        self.active.remove(&id);
    }
    pub fn clear_completed(&mut self) -> Result<CacheCleanup, String> {
        let mut result = CacheCleanup::default();
        for (id, path, size, _) in self.entries()? {
            if self.protected(id) {
                continue;
            }
            match self.remove_unpinned(id, &path) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(_) => {
                    result.failed_transfers += 1;
                    continue;
                }
            }
            result.removed_bytes = result.removed_bytes.saturating_add(size);
            result.removed_paths.push(path);
        }
        Ok(result)
    }
}
fn tree_bytes(path: &Path) -> Result<u64, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() {
        return Ok(0);
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut size = 0_u64;
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        size = size.saturating_add(tree_bytes(&entry.map_err(|e| e.to_string())?.path())?);
    }
    Ok(size)
}
fn remove_owned_directory(root: &Path, path: &Path) -> Result<(), String> {
    if path.parent() != Some(root)
        || fs::symlink_metadata(path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
    {
        return Err("拒绝清除缓存目录以外的路径".into());
    }
    fs::remove_dir_all(path).map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quota_reserves_active_bytes_and_cleanup_never_follows_user_links() {
        let temp = tempfile::tempdir().unwrap();
        let mut cache =
            TransferCache::new(temp.path().canonicalize().unwrap().join("cache"), 12).unwrap();
        let completed = TransferId::new();
        fs::create_dir(cache.destination(completed)).unwrap();
        fs::write(cache.destination(completed).join("old"), [0; 8]).unwrap();
        let active = TransferId::new();
        let removed = cache.reserve(active, 8).unwrap();
        assert_eq!(removed.removed_bytes, 8);
        fs::create_dir(cache.destination(active)).unwrap();
        fs::write(cache.destination(active).join("partial"), [0; 4]).unwrap();
        assert!(cache.reserve(TransferId::new(), 5).is_err());
        assert!(cache.set_limit(7).is_err());
        let source = temp.path().join("user-file");
        fs::write(&source, b"keep").unwrap();
        let complete2 = TransferId::new();
        fs::create_dir(cache.destination(complete2)).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&source, cache.destination(complete2).join("link")).unwrap();
        cache.clear_completed().unwrap();
        assert!(cache.destination(active).join("partial").exists());
        assert_eq!(fs::read(source).unwrap(), b"keep");
        assert_eq!(cache.snapshot().unwrap().active_reserved_bytes, 8);
    }
    #[test]
    fn outgoing_cache_sources_are_protected_from_quota_eviction_and_clear() {
        let temp = tempfile::tempdir().unwrap();
        let mut cache =
            TransferCache::new(temp.path().canonicalize().unwrap().join("cache"), 12).unwrap();
        let id = TransferId::new();
        fs::create_dir(cache.destination(id)).unwrap();
        let source = cache.destination(id).join("source");
        fs::write(&source, [0; 8]).unwrap();
        let pins = cache.pin_sources(&[source.to_string_lossy().into_owned()]);
        assert!(cache.reserve(TransferId::new(), 5).is_err());
        assert!(cache.set_limit(7).is_err());
        assert_eq!(cache.clear_completed().unwrap().removed_bytes, 0);
        assert!(source.exists());
        cache.unpin_sources(&pins);
        assert_eq!(cache.clear_completed().unwrap().removed_bytes, 8);
        assert!(!source.exists());
    }
    #[cfg(unix)]
    #[test]
    fn eviction_failure_reports_already_removed_paths_and_bytes() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let mut cache =
            TransferCache::new(temp.path().canonicalize().unwrap().join("cache"), 32).unwrap();
        let removed = cache.destination(TransferId::new());
        let retained = cache.destination(TransferId::new());
        for (index, path) in [&removed, &retained].into_iter().enumerate() {
            fs::create_dir(path).unwrap();
            fs::write(path.join("file"), [0; 8]).unwrap();
            fs::File::open(path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(
                    SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(index as u64 + 1),
                ))
                .unwrap();
        }
        // Listing and byte accounting still work, but unlinking the file fails.
        fs::set_permissions(&retained, fs::Permissions::from_mode(0o500)).unwrap();
        let result = cache.set_limit(4);
        fs::set_permissions(&retained, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        let cleanup = cache.take_partial_cleanup();
        assert_eq!(cleanup.removed_paths, vec![removed.clone()]);
        assert_eq!(cleanup.removed_bytes, 8);
        assert_eq!(cleanup.failed_transfers, 1);
        assert!(!removed.exists());
        assert!(retained.join("file").exists());
        assert_eq!(cache.take_partial_cleanup().removed_bytes, 0);
    }
    #[test]
    fn native_export_leases_are_synchronous_and_block_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let mut cache =
            TransferCache::new(temp.path().canonicalize().unwrap().join("cache"), 12).unwrap();
        let id = TransferId::new();
        fs::create_dir(cache.destination(id)).unwrap();
        let path = cache.destination(id).join("file");
        fs::write(&path, [0; 8]).unwrap();
        let leases: ExternalCacheLeases = Arc::default();
        cache.set_external_leases(leases.clone());
        let lease = TransferId::new();
        leases
            .lock()
            .unwrap()
            .insert(lease, vec![path.to_string_lossy().into_owned()]);
        assert_eq!(cache.clear_completed().unwrap().removed_bytes, 0);
        assert_eq!(cache.snapshot().unwrap().active_reserved_bytes, 8);
        assert!(cache.reserve(TransferId::new(), 5).is_err());
        leases.lock().unwrap().remove(&lease);
        assert_eq!(cache.clear_completed().unwrap().removed_bytes, 8);
    }
}
