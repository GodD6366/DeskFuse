import CryptoKit
import Darwin
import Foundation
import SQLite3

/// Runs before the engine opens the destination. The app owns its instance lock.
enum HistoryImport {
    struct Failure: LocalizedError {
        let message: String
        var errorDescription: String? { message }
    }

    private final class Database {
        let handle: OpaquePointer
        init(_ path: URL, readOnly: Bool = false) throws {
            var value: OpaquePointer?
            let flags = readOnly ? SQLITE_OPEN_READONLY : SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE
            guard sqlite3_open_v2(path.path, &value, flags, nil) == SQLITE_OK, let value else {
                if let value { sqlite3_close(value) }
                throw Failure(message: "无法打开历史数据库")
            }
            handle = value
            sqlite3_busy_timeout(handle, 5_000)
        }
        deinit { sqlite3_close(handle) }
        func run(_ sql: String) throws {
            guard sqlite3_exec(handle, sql, nil, nil, nil) == SQLITE_OK else {
                throw Failure(message: "数据库操作失败：\(String(cString: sqlite3_errmsg(handle)))")
            }
        }
        func rows(_ sql: String) throws -> [[String]] {
            var statement: OpaquePointer?
            guard sqlite3_prepare_v2(handle, sql, -1, &statement, nil) == SQLITE_OK else {
                throw Failure(message: "无法读取历史数据库结构")
            }
            defer { sqlite3_finalize(statement) }
            var result: [[String]] = []
            var status = sqlite3_step(statement)
            while status == SQLITE_ROW {
                result.append((0..<sqlite3_column_count(statement)).map { index in
                    sqlite3_column_text(statement, index).map { String(cString: $0) } ?? ""
                })
                status = sqlite3_step(statement)
            }
            guard status == SQLITE_DONE else { throw Failure(message: "读取历史数据库失败") }
            return result
        }
        func backup(to path: URL) throws {
            let target = try Database(path)
            guard let backup = sqlite3_backup_init(target.handle, "main", handle, "main") else {
                throw Failure(message: "无法创建导入前备份")
            }
            let status = sqlite3_backup_step(backup, -1)
            let finish = sqlite3_backup_finish(backup)
            guard status == SQLITE_DONE, finish == SQLITE_OK,
                  try target.rows("PRAGMA quick_check") == [["ok"]] else {
                throw Failure(message: "备份或数据库完整性检查失败，未导入")
            }
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: path.path)
        }
    }

    private static func quote(_ value: String) -> String { "'" + value.replacingOccurrences(of: "'", with: "''") + "'" }
    private static func sourceID(_ source: URL) -> String {
        SHA256.hash(data: Data(source.resolvingSymlinksInPath().path.utf8)).map { String(format: "%02x", $0) }.joined()
    }
    static func alreadyImported(source: URL, destination: URL) throws -> Bool {
        let path = destination.appendingPathComponent("state.sqlite3")
        guard FileManager.default.fileExists(atPath: path.path) else { return false }
        let db = try Database(path, readOnly: true)
        guard !(try db.rows("SELECT name FROM sqlite_master WHERE name='clipboard_imports'")).isEmpty else { return false }
        return !(try db.rows("SELECT 1 FROM clipboard_imports WHERE source_id=\(quote(sourceID(source)))")).isEmpty
    }

    static func filesImported(sourceCache: URL, destination: URL) throws -> Bool {
        let path = destination.appendingPathComponent("state.sqlite3")
        guard FileManager.default.fileExists(atPath: path.path) else { return false }
        let db = try Database(path, readOnly: true)
        guard !(try db.rows("SELECT name FROM sqlite_master WHERE name='file_imports'")).isEmpty else { return false }
        return !(try db.rows("SELECT 1 FROM file_imports WHERE source_id=\(quote(sourceID(sourceCache)))")).isEmpty
    }

    private static func importFiles(sourceCache: URL, destination: URL) throws {
        let process = Process()
        process.executableURL = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/deskfuse-history-files")
        process.arguments = [sourceCache.resolvingSymlinksInPath().path,
                             destination.appendingPathComponent("imported-files/clipboard").path,
                             destination.appendingPathComponent("state.sqlite3").path]
        let output = Pipe()
        process.standardOutput = output
        process.standardError = output
        try process.run()
        let result = output.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            throw Failure(message: "历史文件迁移未完成，可重试。\n" + (String(data: result, encoding: .utf8) ?? ""))
        }
    }

    static func run(source: URL, destination: URL, sourceCache: URL) throws -> Int {
        let source = source.resolvingSymlinksInPath()
        let destination = destination.resolvingSymlinksInPath()
        guard source != destination else { throw Failure(message: "来源和目标目录不能相同") }
        let fd = open(source.appendingPathComponent("instance.lock").path, O_CREAT | O_RDWR | O_CLOEXEC, S_IRUSR | S_IWUSR)
        guard fd >= 0 else { throw Failure(message: "无法锁定旧数据目录") }
        defer { close(fd) }
        guard flock(fd, LOCK_EX | LOCK_NB) == 0 else { throw Failure(message: "请先退出旧应用，再重试导入") }
        if try alreadyImported(source: source, destination: destination) {
            try importFiles(sourceCache: sourceCache, destination: destination)
            return 0
        }
        let manager = FileManager.default
        let backupRoot = destination.appendingPathComponent("history-import-backups", isDirectory: true)
        let backup = backupRoot.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try manager.createDirectory(at: backup, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try manager.setAttributes([.posixPermissions: 0o700], ofItemAtPath: backupRoot.path)
        let old = try Database(source.appendingPathComponent("state.sqlite3"), readOnly: true)
        let sourceBackup = backup.appendingPathComponent("source.sqlite3")
        try old.backup(to: sourceBackup)
        let path = destination.appendingPathComponent("state.sqlite3")
        let existed = manager.fileExists(atPath: path.path)
        let db = try Database(path)
        if existed { try db.backup(to: backup.appendingPathComponent("destination.sqlite3")) }
        try manager.setAttributes([.posixPermissions: 0o600], ofItemAtPath: path.path)
        try db.run("ATTACH DATABASE \(quote(sourceBackup.path)) AS prior; BEGIN IMMEDIATE;")
        do {
            try db.run("""
                CREATE TABLE IF NOT EXISTS cl_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS cl_items(id INTEGER PRIMARY KEY AUTOINCREMENT,global_id TEXT UNIQUE NOT NULL,digest TEXT NOT NULL,summary TEXT NOT NULL,payload BLOB,search_text TEXT NOT NULL DEFAULT '',search_index TEXT NOT NULL DEFAULT '',copied INTEGER,imported INTEGER NOT NULL,ordinary INTEGER NOT NULL DEFAULT 1,blob_hash TEXT,origin_device TEXT,title TEXT,ocr_text TEXT,ocr_version INTEGER NOT NULL DEFAULT 0,remote INTEGER NOT NULL DEFAULT 0,last_used INTEGER);
                CREATE TABLE IF NOT EXISTS cl_records(key TEXT PRIMARY KEY,revision INTEGER NOT NULL,author TEXT NOT NULL,deleted INTEGER NOT NULL,value TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS cl_operations(seq INTEGER PRIMARY KEY AUTOINCREMENT,op_id TEXT UNIQUE NOT NULL,body TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS cl_blobs(hash TEXT PRIMARY KEY,bytes BLOB NOT NULL);
                CREATE TEMP TABLE existing_records AS SELECT key FROM cl_records;
                """)
            let before = Int(try db.rows("SELECT count(*) FROM cl_items")[0][0]) ?? 0
            for (table, excluded) in [("cl_items", "id"), ("cl_operations", "seq"), ("cl_records", ""), ("cl_blobs", "")] {
                let target = try db.rows("PRAGMA main.table_info(\(table))").map { $0[1] }
                let previous = try db.rows("PRAGMA prior.table_info(\(table))").map { $0[1] }
                guard Set(target) == Set(previous) else { throw Failure(message: "历史数据库版本不受支持，未导入") }
                let fields = target.filter { $0 != excluded }.map { "\"\($0)\"" }.joined(separator: ",")
                let condition = table == "cl_operations" ? " WHERE json_extract(body,'$.key') NOT IN (SELECT key FROM existing_records)" : ""
                try db.run("INSERT OR IGNORE INTO \(table)(\(fields)) SELECT \(fields) FROM prior.\(table)\(condition)")
            }
            guard try db.rows("SELECT name FROM prior.sqlite_master WHERE name='cl_file_resources'").isEmpty else {
                throw Failure(message: "旧库含文件归档资源，需要单独迁移；本次导入已回滚")
            }
            try db.run("INSERT INTO cl_meta(key,value) VALUES('counter',CAST((SELECT COALESCE(MAX(revision),0) FROM cl_records) AS TEXT)) ON CONFLICT(key) DO UPDATE SET value=CAST(MAX(CAST(value AS INTEGER),CAST(excluded.value AS INTEGER)) AS TEXT)")
            if !(try db.rows("SELECT name FROM sqlite_master WHERE name='cl_items_fts'")).isEmpty {
                try db.run("INSERT INTO cl_items_fts(cl_items_fts) VALUES('integrity-check')")
            }
            let after = Int(try db.rows("SELECT count(*) FROM cl_items")[0][0]) ?? 0
            try db.run("CREATE TABLE IF NOT EXISTS clipboard_imports(source_id TEXT PRIMARY KEY,imported_at TEXT NOT NULL); INSERT INTO clipboard_imports VALUES(\(quote(sourceID(source))),datetime('now')); COMMIT;")
            try importFiles(sourceCache: sourceCache, destination: destination)
            return after - before
        } catch {
            try? db.run("ROLLBACK")
            throw error
        }
    }
}
