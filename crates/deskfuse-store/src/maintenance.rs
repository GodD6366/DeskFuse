//! 本地数据库体积维护。
//!
//! 旧版本既没有限制 WAL 体积，也没有开启 `auto_vacuum`：删除历史后空闲页
//! 永久留在文件里，WAL 也停在历史峰值。升级后的第一次启动会执行一次
//! "数据优化"阶段，把数据库转换为增量回收模式并整理一次；之后由运行期的
//! 维护线程按小步回收空闲页，不再需要整库重写。
use crate::{Result, configure_connection};
use rusqlite::Connection;
use std::{path::Path, time::Duration};

/// 每个连接在检查点后把 WAL 截回到不超过该体积。
pub const WAL_SIZE_LIMIT_BYTES: i64 = 64 * 1024 * 1024;
/// 空闲页超过该体积才值得回收，避免为零碎空间反复写盘。
pub const RECLAIM_THRESHOLD_BYTES: u64 = 32 * 1024 * 1024;
/// SQLite `auto_vacuum` 的增量模式取值。
const AUTO_VACUUM_INCREMENTAL: i64 = 2;
/// 优化阶段独占数据库的时间很短，但要容忍其他进程刚好持有写锁。
const OPTIMIZE_BUSY_TIMEOUT: Duration = Duration::from_secs(30);

/// 数据库页面统计。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageStats {
    /// 页大小（字节）。
    pub page_size: u64,
    /// 文件总页数。
    pub page_count: u64,
    /// 空闲页数。
    pub freelist_count: u64,
    /// `auto_vacuum` 模式：0 关闭、1 完整、2 增量。
    pub auto_vacuum: i64,
}

impl StorageStats {
    /// 读取当前连接所在数据库的页面统计。
    pub fn read(connection: &Connection) -> rusqlite::Result<Self> {
        let pragma =
            |name: &str| connection.pragma_query_value(None, name, |row| row.get::<_, i64>(0));
        Ok(Self {
            page_size: u64::try_from(pragma("page_size")?).unwrap_or(0),
            page_count: u64::try_from(pragma("page_count")?).unwrap_or(0),
            freelist_count: u64::try_from(pragma("freelist_count")?).unwrap_or(0),
            auto_vacuum: pragma("auto_vacuum")?,
        })
    }
    /// 空闲页合计字节数。
    pub fn free_bytes(&self) -> u64 {
        self.freelist_count.saturating_mul(self.page_size)
    }
    /// 是否已处于增量回收模式。
    pub fn incremental(&self) -> bool {
        self.auto_vacuum == AUTO_VACUUM_INCREMENTAL
    }
    /// 已有数据的旧库还没转换为增量回收模式。空库会在建表时直接采用增量模式。
    fn needs_conversion(&self) -> bool {
        !self.incremental() && self.page_count > 0
    }
}

/// 一次优化阶段的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptimizationOutcome {
    /// 数据库文件不存在（全新安装）。
    Missing,
    /// 已是增量模式且没有值得回收的空间，只截断了 WAL。
    AlreadyOptimal,
    /// 旧库已转换为增量回收模式并整理。
    Compacted,
    /// 增量模式下回收了积累的空闲页。
    Reclaimed,
    /// 本次无法完成（如磁盘空间不足或数据库被占用），下次启动重试。
    Deferred(String),
}

/// 优化阶段的结果与前后体积。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptimizationReport {
    /// 本次执行的动作。
    pub outcome: OptimizationOutcome,
    /// 优化前数据库与 WAL 文件合计体积。
    pub bytes_before: u64,
    /// 优化后数据库与 WAL 文件合计体积。
    pub bytes_after: u64,
}

/// 数据库与 WAL 文件的合计体积。
fn on_disk_bytes(path: &Path) -> u64 {
    let size = |path: &Path| std::fs::metadata(path).map_or(0, |meta| meta.len());
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    size(path).saturating_add(size(Path::new(&wal)))
}

fn wal_bytes(path: &Path) -> u64 {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    std::fs::metadata(Path::new(&wal)).map_or(0, |meta| meta.len())
}

/// 启动时的廉价判断：是否需要展示"数据优化"阶段。只读页头，不写库。
pub fn needs_optimization(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let connection = Connection::open(path)?;
    let stats = StorageStats::read(&connection)?;
    Ok(stats.needs_conversion()
        || stats.free_bytes() >= RECLAIM_THRESHOLD_BYTES
        || wal_bytes(path) > WAL_SIZE_LIMIT_BYTES as u64)
}

/// 升级后的数据优化阶段。必须在引擎打开其他连接之前调用：
/// 整理期间持有写锁，其他写入者只能等待。
///
/// 整理在一个事务里完成，失败（磁盘写满、被占用）时原库保持不变，
/// 结果记为 [`OptimizationOutcome::Deferred`]，下次启动重试。
pub fn optimize_database(path: &Path) -> Result<OptimizationReport> {
    if !path.exists() {
        return Ok(OptimizationReport {
            outcome: OptimizationOutcome::Missing,
            bytes_before: 0,
            bytes_after: 0,
        });
    }
    let bytes_before = on_disk_bytes(path);
    let mut connection = Connection::open(path)?;
    configure_connection(&mut connection)?;
    connection.busy_timeout(OPTIMIZE_BUSY_TIMEOUT)?;
    let stats = StorageStats::read(&connection)?;
    let outcome = if stats.needs_conversion() {
        // auto_vacuum 从 NONE 切换到 INCREMENTAL 只能通过一次 VACUUM 生效。
        let converted = connection
            .execute_batch("PRAGMA auto_vacuum = INCREMENTAL; VACUUM;")
            .map(|()| OptimizationOutcome::Compacted);
        deferred_on_failure(converted)?
    } else if stats.incremental() && stats.free_bytes() >= RECLAIM_THRESHOLD_BYTES {
        let reclaimed =
            incremental_vacuum(&connection, None).map(|()| OptimizationOutcome::Reclaimed);
        deferred_on_failure(reclaimed)?
    } else {
        OptimizationOutcome::AlreadyOptimal
    };
    truncate_wal(&connection)?;
    drop(connection);
    Ok(OptimizationReport {
        outcome,
        bytes_before,
        bytes_after: on_disk_bytes(path),
    })
}

/// 可恢复的失败转为延后：原库不受影响，下次启动重试。其余错误照常上报。
fn deferred_on_failure(
    result: rusqlite::Result<OptimizationOutcome>,
) -> Result<OptimizationOutcome> {
    use rusqlite::ErrorCode;
    match result {
        Ok(outcome) => Ok(outcome),
        Err(rusqlite::Error::SqliteFailure(error, _))
            if matches!(
                error.code,
                ErrorCode::DiskFull | ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            ) =>
        {
            let reason = if error.code == ErrorCode::DiskFull {
                "磁盘空间不足"
            } else {
                "数据库正被占用"
            };
            Ok(OptimizationOutcome::Deferred(reason.into()))
        }
        Err(error) => Err(error.into()),
    }
}

/// `incremental_vacuum` 每执行一步只归还一页，必须把语句跑完才会回收全部目标页。
fn incremental_vacuum(connection: &Connection, max_pages: Option<u32>) -> rusqlite::Result<()> {
    let sql = max_pages.map_or_else(
        || "PRAGMA incremental_vacuum".to_owned(),
        |pages| format!("PRAGMA incremental_vacuum({pages})"),
    );
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query([])?;
    while rows.next()?.is_some() {}
    Ok(())
}

/// 检查点后把 WAL 截断为零。仍有读者时 SQLite 只做部分检查点，不视为错误。
fn truncate_wal(connection: &Connection) -> Result<()> {
    connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    Ok(())
}

/// 运行期的小步回收：增量模式下空闲页超过阈值时归还最多 `max_pages` 页。
/// 返回是否还有值得继续回收的空间。调用方负责在空闲时、可让路的连接上执行。
pub fn reclaim_free_pages_step(connection: &Connection, max_pages: u32) -> rusqlite::Result<bool> {
    let stats = StorageStats::read(connection)?;
    if !stats.incremental() || stats.free_bytes() < RECLAIM_THRESHOLD_BYTES {
        return Ok(false);
    }
    incremental_vacuum(connection, Some(max_pages))?;
    let after = StorageStats::read(connection)?;
    Ok(after.free_bytes() >= RECLAIM_THRESHOLD_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 模拟旧版本：未开启 auto_vacuum，写入大量数据后删除，留下空闲页。
    fn legacy_database(path: &Path) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "PRAGMA auto_vacuum = NONE;
                 PRAGMA journal_mode = WAL;
                 CREATE TABLE blobs (id INTEGER PRIMARY KEY, payload BLOB NOT NULL);",
            )
            .unwrap();
        let payload = vec![7_u8; 64 * 1024];
        let transaction = connection.unchecked_transaction().unwrap();
        for _ in 0..800 {
            transaction
                .execute("INSERT INTO blobs (payload) VALUES (?1)", [&payload])
                .unwrap();
        }
        transaction.commit().unwrap();
        connection
            .execute("DELETE FROM blobs WHERE id > 20", [])
            .unwrap();
        connection
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "deskfuse-store-maintenance-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("state.sqlite3")
    }

    #[test]
    fn legacy_database_is_converted_and_shrunk_once() {
        let path = temp_path("convert");
        legacy_database(&path);
        assert!(needs_optimization(&path).unwrap());
        let report = optimize_database(&path).unwrap();
        assert_eq!(report.outcome, OptimizationOutcome::Compacted);
        assert!(report.bytes_after * 4 < report.bytes_before, "{report:?}");
        let connection = Connection::open(&path).unwrap();
        let stats = StorageStats::read(&connection).unwrap();
        assert!(stats.incremental());
        assert_eq!(stats.freelist_count, 0);
        let rows: i64 = connection
            .query_row("SELECT COUNT(*) FROM blobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 20);
        drop(connection);
        // 第二次启动不再整理。
        assert!(!needs_optimization(&path).unwrap());
        assert_eq!(
            optimize_database(&path).unwrap().outcome,
            OptimizationOutcome::AlreadyOptimal
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn incremental_database_reclaims_in_bounded_steps() {
        let path = temp_path("reclaim");
        legacy_database(&path);
        optimize_database(&path).unwrap();
        let connection = Connection::open(&path).unwrap();
        let payload = vec![9_u8; 64 * 1024];
        for _ in 0..800 {
            connection
                .execute("INSERT INTO blobs (payload) VALUES (?1)", [&payload])
                .unwrap();
        }
        connection
            .execute("DELETE FROM blobs WHERE id > 20", [])
            .unwrap();
        let before = StorageStats::read(&connection).unwrap().free_bytes();
        assert!(before >= RECLAIM_THRESHOLD_BYTES);
        let mut steps = 0;
        while reclaim_free_pages_step(&connection, 1024).unwrap() {
            steps += 1;
            assert!(steps < 100);
        }
        let after = StorageStats::read(&connection).unwrap().free_bytes();
        assert!(after < RECLAIM_THRESHOLD_BYTES);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn missing_database_needs_nothing() {
        let path = temp_path("missing");
        assert!(!needs_optimization(&path).unwrap());
        assert_eq!(
            optimize_database(&path).unwrap().outcome,
            OptimizationOutcome::Missing
        );
    }

    #[test]
    fn new_database_starts_incremental_with_bounded_wal() {
        let path = temp_path("fresh");
        let store = crate::SQLiteStore::open(&path).unwrap();
        drop(store);
        let connection = Connection::open(&path).unwrap();
        assert!(StorageStats::read(&connection).unwrap().incremental());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
