CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY NOT NULL,
    applied_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS cl_contents (
    id TEXT PRIMARY KEY,
    content_type TEXT NOT NULL,
    title TEXT,
    snippet TEXT,
    data_hash TEXT,
    size INTEGER,
    created_at INTEGER NOT NULL,
    indexed_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS cl_search (
    content_id TEXT PRIMARY KEY,
    title TEXT,
    snippet TEXT,
    FOREIGN KEY (content_id) REFERENCES cl_contents(id)
);

CREATE TABLE IF NOT EXISTS cl_files (
    content_id TEXT NOT NULL,
    file_path TEXT NOT NULL,
    file_hash TEXT,
    PRIMARY KEY (content_id, file_path),
    FOREIGN KEY (content_id) REFERENCES cl_contents(id)
);

CREATE TABLE IF NOT EXISTS cl_sync_state (
    device_id TEXT PRIMARY KEY,
    last_sync_cursor INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS cl_sync_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_type TEXT NOT NULL,
    content_id TEXT NOT NULL,
    device_id TEXT,
    timestamp INTEGER NOT NULL
);
