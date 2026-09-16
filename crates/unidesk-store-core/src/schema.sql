CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY NOT NULL,
    applied_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS devices (
    id TEXT PRIMARY KEY,
    public_key BLOB NOT NULL,
    name TEXT NOT NULL,
    device_type TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_seen INTEGER
);

CREATE TABLE IF NOT EXISTS trust (
    device_id TEXT PRIMARY KEY,
    trusted_at INTEGER NOT NULL,
    trusted_by TEXT NOT NULL,
    FOREIGN KEY (device_id) REFERENCES devices(id)
);

CREATE TABLE IF NOT EXISTS config (
    key TEXT PRIMARY KEY,
    value BLOB NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS transfer_checkpoints (
    transfer_id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL,
    progress BLOB NOT NULL,
    updated_at INTEGER NOT NULL
);
