//! Persistent state for uniDesk.
//!
//! [`SQLiteStore`] is intentionally a small synchronous store.  It keeps the
//! SQLite connection private and protects it with a mutex, so the store can be
//! shared by the synchronous application services without exposing a
//! `rusqlite::Connection` or imposing an async runtime on callers.
//!
//! The database schema uses explicit columns for domain values rather than
//! serializing Rust enum discriminants.  This makes the on-disk representation
//! stable across Rust/compiler versions and leaves room for future migrations.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations, missing_docs)]

use std::{
    convert::TryInto,
    env, fmt, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use unidesk_core::{
    ConnectionState, DeviceId, Endpoint, LivenessState, PeerSnapshot, RouteCandidate, RouteHealth,
    RouteId, RouteSource, TrustState,
};
use unidesk_identity::{Fingerprint, TrustRecord, TrustStore};

/// The latest schema version understood by this crate.
pub const CURRENT_SCHEMA_VERSION: i64 = 4;

const SETTINGS_PUBLIC_NETWORK_ENABLED: &str = "public_network_enabled";

/// Apply the common policy to each connection to the local state database.
///
/// WAL lets history readers coexist with device/settings writes. Transactions
/// reserve the writer before reading: upgrading a deferred read transaction can
/// fail immediately (or with BUSY_SNAPSHOT in WAL), bypassing the busy timeout.
/// In-memory stores retain SQLite's memory journal for isolated callers.
pub fn configure_connection(connection: &mut Connection) -> rusqlite::Result<()> {
    connection.busy_timeout(Duration::from_secs(5))?;
    let journal: String = connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    if journal != "wal" && journal != "memory" {
        connection.pragma_update(None, "journal_mode", "WAL")?;
    }
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.set_transaction_behavior(TransactionBehavior::Immediate);
    Ok(())
}

/// A result returned by persistence operations.
pub type Result<T> = std::result::Result<T, StoreError>;

/// Errors returned by the SQLite-backed store.
#[derive(Debug, Error)]
pub enum StoreError {
    /// SQLite rejected a query or transaction.
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A parent directory could not be created or opened.
    #[error("could not prepare store directory: {0}")]
    Io(#[from] io::Error),
    /// A JSON state payload could not be decoded or encoded.
    #[error("state serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    /// The default store path could not be determined.
    #[error("could not determine the user's home directory")]
    HomeDirectoryUnavailable,
    /// A database is newer than this binary can safely understand.
    #[error("unsupported store schema version {found}; latest supported version is {supported}")]
    UnsupportedSchemaVersion {
        /// Version found in the database.
        found: i64,
        /// Highest version this crate supports.
        supported: i64,
    },
    /// A migration was malformed or could not be applied.
    #[error("store migration failed: {0}")]
    Migration(String),
    /// Stored bytes do not represent a valid device ID.
    #[error("invalid stored device ID: expected {expected} bytes, got {actual}")]
    InvalidDeviceId {
        /// Required byte length.
        expected: usize,
        /// Actual byte length.
        actual: usize,
    },
    /// Stored bytes do not represent a valid public key.
    #[error("invalid stored public key: expected {expected} bytes, got {actual}")]
    InvalidPublicKey {
        /// Required byte length.
        expected: usize,
        /// Actual byte length.
        actual: usize,
    },
    /// A persisted enum value is unknown.
    #[error("invalid stored {kind} value {value:?}")]
    InvalidEnum {
        /// Stable field/enum category.
        kind: &'static str,
        /// Value found in the database.
        value: String,
    },
    /// A persisted route ID was not valid.
    #[error("invalid stored route ID {value:?}")]
    InvalidRouteId {
        /// Value found in the database.
        value: String,
    },
    /// Persisted peer data violated a core-domain invariant.
    #[error("invalid peer snapshot: {0}")]
    InvalidPeer(String),
    /// A store mutex was poisoned by a panicking operation.
    #[error("store connection lock is poisoned")]
    Poisoned,
    /// A requested peer was not present.
    #[error("peer {0} was not found")]
    PeerNotFound(DeviceId),
}

/// Persisted peer metadata and its route candidates.
///
/// The shape intentionally mirrors [`PeerSnapshot`] and adds an explicit
/// update timestamp.  Routes are stored in their own table, but are returned
/// together with the peer to callers for a complete round trip.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PeerRecord {
    /// Stable remote device identity.
    pub id: DeviceId,
    /// User-visible peer name.
    pub display_name: String,
    /// Ed25519 public key bytes.
    pub public_key: [u8; 32],
    /// User trust/pairing state.
    pub trust: TrustState,
    /// Last known reachability state.
    pub liveness: LivenessState,
    /// Current transport state.
    pub connection: ConnectionState,
    /// Known route candidates for this peer.
    pub routes: Vec<RouteCandidate>,
    /// The selected route, if one is selected.
    pub selected_route: Option<RouteId>,
    /// Last update time, represented as Unix milliseconds.
    pub updated_at_unix_ms: i64,
}

/// Compatibility alias for code that calls this a stored or paired peer.
pub type StoredPeer = PeerRecord;
/// Compatibility alias for code that calls this a paired peer.
pub type PairedPeer = PeerRecord;
/// Compatibility alias for callers that use a generic store name.
pub type Store = SQLiteStore;

impl PeerRecord {
    /// Build a persisted record from a validated core snapshot.
    pub fn from_snapshot(snapshot: PeerSnapshot, updated_at_unix_ms: i64) -> Result<Self> {
        snapshot
            .validate()
            .map_err(|error| StoreError::InvalidPeer(error.to_string()))?;
        Ok(Self {
            id: snapshot.id,
            display_name: snapshot.display_name,
            public_key: snapshot.public_key,
            trust: snapshot.trust,
            liveness: snapshot.liveness,
            connection: snapshot.connection,
            routes: snapshot.routes,
            selected_route: snapshot.selected_route,
            updated_at_unix_ms,
        })
    }

    /// Construct and validate a peer record from its fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: DeviceId,
        display_name: impl Into<String>,
        public_key: [u8; 32],
        trust: TrustState,
        liveness: LivenessState,
        connection: ConnectionState,
        routes: Vec<RouteCandidate>,
        selected_route: Option<RouteId>,
        updated_at_unix_ms: i64,
    ) -> Result<Self> {
        let snapshot = PeerSnapshot::new(
            id,
            display_name,
            public_key,
            liveness,
            trust,
            connection,
            routes,
            selected_route,
        )
        .map_err(|error| StoreError::InvalidPeer(error.to_string()))?;
        Self::from_snapshot(snapshot, updated_at_unix_ms)
    }

    /// Return this record as a core snapshot, checking invariants first.
    pub fn snapshot(&self) -> Result<PeerSnapshot> {
        PeerSnapshot::new(
            self.id,
            self.display_name.clone(),
            self.public_key,
            self.liveness,
            self.trust,
            self.connection,
            self.routes.clone(),
            self.selected_route,
        )
        .map_err(|error| StoreError::InvalidPeer(error.to_string()))
    }

    /// Return the stable remote device identity.
    pub const fn device_id(&self) -> DeviceId {
        self.id
    }

    /// Return the record's update time.
    pub const fn updated_at_unix_ms(&self) -> i64 {
        self.updated_at_unix_ms
    }

    /// Alias for [`PeerRecord::updated_at_unix_ms`].
    pub const fn updated_at(&self) -> i64 {
        self.updated_at_unix_ms
    }
}

/// Store-level settings persisted as key/value values.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Whether the additive public-network transport is enabled.
    pub public_network_enabled: bool,
}

/// A manually configured endpoint for one peer.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ManualEndpoint {
    /// Device to which this endpoint belongs.
    pub peer_id: DeviceId,
    /// Validated host/port endpoint.
    pub endpoint: Endpoint,
}

impl ManualEndpoint {
    /// Construct a manual endpoint.
    #[must_use]
    pub const fn new(peer_id: DeviceId, endpoint: Endpoint) -> Self {
        Self { peer_id, endpoint }
    }

    /// Alias for callers that use `device_id` terminology.
    #[must_use]
    pub const fn device_id(&self) -> DeviceId {
        self.peer_id
    }
}

/// Synchronous persistence contract for paired peers and their routes.
pub trait PeerStore: Send + Sync {
    /// Insert or replace one peer and all of its routes.
    fn upsert_peer(&self, peer: PeerRecord) -> Result<()>;
    /// Return one peer by stable device ID.
    fn get_peer(&self, id: DeviceId) -> Result<Option<PeerRecord>>;
    /// Return all peers in deterministic device-ID order.
    fn list_peers(&self) -> Result<Vec<PeerRecord>>;
    /// Delete one peer and its routes. Returns whether a row existed.
    fn delete_peer(&self, id: DeviceId) -> Result<bool>;
    /// Insert or replace one route for an existing peer.
    fn upsert_route(&self, peer_id: DeviceId, route: RouteCandidate) -> Result<()>;
    /// Return routes for a peer in deterministic route-ID order.
    fn list_routes(&self, peer_id: DeviceId) -> Result<Vec<RouteCandidate>>;
    /// Delete one route. Returns whether a row existed.
    fn delete_route(&self, route_id: RouteId) -> Result<bool>;

    /// Alias for [`PeerStore::upsert_peer`].
    fn upsert(&self, peer: PeerRecord) -> Result<()> {
        self.upsert_peer(peer)
    }
    /// Alias for [`PeerStore::get_peer`].
    fn get(&self, id: DeviceId) -> Result<Option<PeerRecord>> {
        self.get_peer(id)
    }
    /// Alias for [`PeerStore::list_peers`].
    fn list(&self) -> Result<Vec<PeerRecord>> {
        self.list_peers()
    }
    /// Alias for [`PeerStore::delete_peer`].
    fn remove(&self, id: DeviceId) -> Result<bool> {
        self.delete_peer(id)
    }
}

/// Synchronous persistence contract for feature settings.
pub trait SettingsStore: Send + Sync {
    /// Load per-device physical route priorities.
    fn get_route_preferences(
        &self,
    ) -> Result<std::collections::BTreeMap<DeviceId, unidesk_core::RoutePreference>> {
        Ok(Default::default())
    }
    /// Persist per-device priorities without disabling fallback.
    fn set_route_preferences(
        &self,
        _preferences: std::collections::BTreeMap<DeviceId, unidesk_core::RoutePreference>,
    ) -> Result<()> {
        Ok(())
    }
    /// Load settings, applying safe defaults for missing legacy keys.
    fn get_settings(&self) -> Result<Settings>;
    /// Replace settings.
    fn set_settings(&self, settings: Settings) -> Result<()>;
    /// Read the public-network feature flag.
    fn public_network_enabled(&self) -> Result<bool>;
    /// Update the public-network feature flag.
    fn set_public_network_enabled(&self, enabled: bool) -> Result<()>;

    /// Alias for [`SettingsStore::get_settings`].
    fn load(&self) -> Result<Settings> {
        self.get_settings()
    }
    /// Alias for [`SettingsStore::set_settings`].
    fn save(&self, settings: Settings) -> Result<()> {
        self.set_settings(settings)
    }
}

/// Synchronous persistence contract for user-entered endpoints.
pub trait ManualEndpointStore: Send + Sync {
    /// Insert an endpoint, returning `true`; duplicate inserts return `false`.
    fn add_manual_endpoint(&self, endpoint: ManualEndpoint) -> Result<bool>;
    /// Return all manually configured endpoints for one peer.
    fn list_manual_endpoints(&self, peer_id: DeviceId) -> Result<Vec<ManualEndpoint>>;
    /// Remove one endpoint, returning whether it existed.
    fn delete_manual_endpoint(&self, endpoint: &ManualEndpoint) -> Result<bool>;

    /// Alias for [`ManualEndpointStore::add_manual_endpoint`].
    fn add(&self, endpoint: ManualEndpoint) -> Result<bool> {
        self.add_manual_endpoint(endpoint)
    }
    /// Alias for [`ManualEndpointStore::list_manual_endpoints`].
    fn list(&self, peer_id: DeviceId) -> Result<Vec<ManualEndpoint>> {
        self.list_manual_endpoints(peer_id)
    }
    /// Alias for [`ManualEndpointStore::delete_manual_endpoint`].
    fn remove(&self, endpoint: &ManualEndpoint) -> Result<bool> {
        self.delete_manual_endpoint(endpoint)
    }
}

/// A thread-safe SQLite state store.
///
/// The connection is private and never returned to callers.  This is
/// deliberate: exposing a connection would allow an uncoordinated transaction
/// to violate the store's invariants and would prevent the type from being
/// safely shared as `dyn PeerStore + SettingsStore`.
#[derive(Clone)]
pub struct SQLiteStore {
    connection: Arc<Mutex<Connection>>,
}

impl fmt::Debug for SQLiteStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SQLiteStore")
            .field("schema_version", &self.schema_version().unwrap_or(-1))
            .finish_non_exhaustive()
    }
}

/// Persisted history ID, payload and optional copy timestamp.
pub type ClipboardHistoryRow = (u64, Vec<u8>, Option<u64>);

impl SQLiteStore {
    /// Load version-1 snapshots and their optional copy time, newest first (at most 50).
    pub fn clipboard_history(&self) -> Result<Vec<ClipboardHistoryRow>> {
        let connection = self.lock_connection()?;
        let mut statement = connection.prepare(
            "SELECT id, payload, copied_at_ms FROM clipboard_history_v1 ORDER BY position DESC LIMIT 50",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Atomically upsert one changed snapshot and remove entries no longer retained.
    /// A missing changed entry only applies retention, including clearing history.
    pub fn save_clipboard_history(
        &self,
        changed: Option<(u64, &[u8], Option<u64>)>,
        retained_ids: &[u64],
    ) -> Result<()> {
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction()?;
        if let Some((id, payload, copied_at_ms)) = changed {
            transaction.execute(
                "INSERT INTO clipboard_history_v1 (id, position, payload, copied_at_ms)
                 VALUES (?1, (SELECT COALESCE(MAX(position), 0) + 1 FROM clipboard_history_v1), ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET position=excluded.position,
                     payload=excluded.payload, copied_at_ms=excluded.copied_at_ms",
                params![id, payload, copied_at_ms],
            )?;
        }
        let ids = {
            let mut statement = transaction.prepare("SELECT id FROM clipboard_history_v1")?;
            statement
                .query_map([], |row| row.get::<_, u64>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for id in ids {
            if !retained_ids.contains(&id) {
                transaction.execute("DELETE FROM clipboard_history_v1 WHERE id=?1", [id])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Load persisted clipboard file receiving policy, preserving legacy defaults.
    pub fn file_transfer_settings(&self) -> Result<unidesk_core::FileTransferSettings> {
        let connection = self.lock_connection()?;
        let value: Option<String> = connection
            .query_row(
                "SELECT value FROM settings WHERE key = 'file_transfer_policy_v1'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let settings = value
            .map(|value| serde_json::from_str::<unidesk_core::FileTransferSettings>(&value))
            .transpose()?
            .unwrap_or_default();
        settings.validate().map_err(|message| {
            StoreError::Io(io::Error::new(io::ErrorKind::InvalidData, message))
        })?;
        Ok(settings)
    }

    /// Persist the complete validated file transfer policy in one atomic row update.
    pub fn set_file_transfer_settings(
        &self,
        settings: &unidesk_core::FileTransferSettings,
    ) -> Result<()> {
        settings.validate().map_err(|message| {
            StoreError::Io(io::Error::new(io::ErrorKind::InvalidInput, message))
        })?;
        self.lock_connection()?.execute("INSERT INTO settings (key,value) VALUES ('file_transfer_policy_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![serde_json::to_string(settings)?])?;
        Ok(())
    }

    /// Load the current definitions, with a one-time import path for image-only settings.
    pub fn workflows(&self) -> Result<Vec<unidesk_core::workflow::Workflow>> {
        let connection = self.lock_connection()?;
        let value: Option<(String, String)> = connection.query_row(
            "SELECT key,value FROM settings WHERE key IN ('workflows_v2','image_workflows_v1','workflows_v1') ORDER BY CASE key WHEN 'workflows_v2' THEN 0 WHEN 'image_workflows_v1' THEN 1 ELSE 2 END LIMIT 1",
            [], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        let Some((key, json)) = value else {
            return Ok(Vec::new());
        };
        let mut workflows: Vec<unidesk_core::workflow::Workflow> = serde_json::from_str(&json)?;
        if key != "workflows_v2" {
            for workflow in &mut workflows {
                if workflow.version == 1 {
                    workflow.version = 2;
                    workflow.input_type = unidesk_core::workflow::WorkflowContentType::Image;
                    for step in &mut workflow.steps {
                        if step.kind == unidesk_core::workflow::WorkflowStepKind::SystemShortcut {
                            step.shortcut_output =
                                unidesk_core::workflow::WorkflowContentType::Image;
                        }
                    }
                }
            }
        }
        Ok(workflows)
    }

    /// Legacy service adapter; new synchronized definitions use workflow_catalog_json.
    pub fn set_workflows(&self, workflows: &[unidesk_core::workflow::Workflow]) -> Result<()> {
        for workflow in workflows {
            workflow.validate().map_err(|message| {
                StoreError::Io(io::Error::new(io::ErrorKind::InvalidInput, message))
            })?;
        }
        let connection = self.lock_connection()?;
        let transaction = connection.unchecked_transaction()?;
        transaction.execute("INSERT INTO settings (key,value) VALUES ('workflows_v2',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![serde_json::to_string(workflows)?])?;
        transaction.execute(
            "DELETE FROM settings WHERE key IN ('image_workflows_v1','workflows_v1')",
            [],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Catalog is an atomic document: revisions, tombstones, conflicts and local bindings
    /// are committed together, without replacing any other settings.
    pub fn workflow_catalog_json(&self) -> Result<Option<String>> {
        Ok(self
            .lock_connection()?
            .query_row(
                "SELECT value FROM settings WHERE key = 'workflow_catalog_v2'",
                [],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Commit the catalog and retire imported image-only rows in the same transaction.
    pub fn set_workflow_catalog_json(&self, json: &str) -> Result<()> {
        let _: serde_json::Value = serde_json::from_str(json)?;
        let connection = self.lock_connection()?;
        let transaction = connection.unchecked_transaction()?;
        transaction.execute("INSERT INTO settings (key,value) VALUES ('workflow_catalog_v2',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![json])?;
        transaction.execute("DELETE FROM settings WHERE key IN ('image_workflows_v1','workflows_v1','workflows_v2')", [])?;
        transaction.commit()?;
        Ok(())
    }

    /// Open the default per-user database at
    /// `~/Library/Application Support/uniDesk/state.sqlite3`.
    pub fn new() -> Result<Self> {
        Self::open(Self::default_path()?)
    }

    /// Open a database at an explicit path, creating parent directories.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Self::from_connection(connection)
    }

    /// Open an isolated in-memory database.
    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    /// Alias for [`SQLiteStore::open_in_memory`], useful in tests.
    pub fn in_memory() -> Result<Self> {
        Self::open_in_memory()
    }

    /// Alias for [`SQLiteStore::open`].
    pub fn open_path(path: impl AsRef<Path>) -> Result<Self> {
        Self::open(path)
    }

    /// Alias for [`SQLiteStore::open_in_memory`].
    pub fn new_in_memory() -> Result<Self> {
        Self::open_in_memory()
    }

    /// Return the platform-specific default path without opening it.
    pub fn default_path() -> Result<PathBuf> {
        let home = env::var_os("HOME").ok_or(StoreError::HomeDirectoryUnavailable)?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("uniDesk")
            .join("state.sqlite3"))
    }

    /// Return the current schema version recorded in the migration table.
    pub fn schema_version(&self) -> Result<i64> {
        let connection = self.lock_connection()?;
        Ok(connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?)
    }

    /// Apply migrations again. This is safe and idempotent.
    pub fn migrate(&self) -> Result<()> {
        let mut connection = self.lock_connection()?;
        migrate_connection(&mut connection)
    }

    /// Save a core snapshot with an explicit update timestamp.
    pub fn upsert_snapshot(&self, snapshot: PeerSnapshot, updated_at_unix_ms: i64) -> Result<()> {
        self.upsert_peer(PeerRecord::from_snapshot(snapshot, updated_at_unix_ms)?)
    }

    /// Save a core snapshot using the current Unix time in milliseconds.
    pub fn upsert_snapshot_now(&self, snapshot: PeerSnapshot) -> Result<()> {
        self.upsert_snapshot(snapshot, unix_time_ms())
    }

    /// Return a snapshot without the persistence timestamp.
    pub fn get_snapshot(&self, id: DeviceId) -> Result<Option<PeerSnapshot>> {
        self.get_peer(id)?.map(|peer| peer.snapshot()).transpose()
    }

    /// Replace a peer's route set while preserving its peer metadata.
    pub fn replace_routes(&self, peer_id: DeviceId, routes: &[RouteCandidate]) -> Result<()> {
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction()?;
        let exists: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM peers WHERE device_id = ?1",
                [device_id_blob(peer_id).as_slice()],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(StoreError::PeerNotFound(peer_id));
        }
        transaction.execute(
            "DELETE FROM routes WHERE device_id = ?1",
            [device_id_blob(peer_id).as_slice()],
        )?;
        for route in routes {
            insert_route(&transaction, peer_id, route)?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Add a manual endpoint, returning `false` when it was already present.
    pub fn add_endpoint(&self, endpoint: ManualEndpoint) -> Result<bool> {
        self.add_manual_endpoint(endpoint)
    }

    /// Remove a manual endpoint, returning `false` when it was absent.
    pub fn remove_endpoint(&self, endpoint: &ManualEndpoint) -> Result<bool> {
        self.delete_manual_endpoint(endpoint)
    }

    /// Add a validated host/port for one peer, returning `false` on a duplicate.
    pub fn add_manual_endpoint_for_peer(
        &self,
        peer_id: DeviceId,
        endpoint: Endpoint,
    ) -> Result<bool> {
        self.add_manual_endpoint(ManualEndpoint::new(peer_id, endpoint))
    }

    /// Return the current Unix timestamp in milliseconds.
    #[must_use]
    pub fn now_unix_ms() -> i64 {
        unix_time_ms()
    }

    fn from_connection(mut connection: Connection) -> Result<Self> {
        configure_connection(&mut connection)?;
        migrate_connection(&mut connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    fn lock_connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| StoreError::Poisoned)
    }
}

impl PeerStore for SQLiteStore {
    fn upsert_peer(&self, peer: PeerRecord) -> Result<()> {
        let snapshot = peer.snapshot()?;
        let device_id = device_id_blob(peer.id);
        let public_key = peer.public_key.to_vec();
        let selected_route = peer.selected_route.map(|route| route.to_string());
        let connection = self.lock_connection()?;
        let transaction = connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO peers (
                device_id, display_name, public_key, trust, liveness, connection,
                selected_route, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(device_id) DO UPDATE SET
                display_name = excluded.display_name,
                public_key = excluded.public_key,
                trust = excluded.trust,
                liveness = excluded.liveness,
                connection = excluded.connection,
                selected_route = excluded.selected_route,
                updated_at = excluded.updated_at",
            params![
                device_id.as_slice(),
                peer.display_name,
                public_key,
                trust_to_str(peer.trust),
                liveness_to_str(peer.liveness),
                connection_to_str(peer.connection),
                selected_route,
                peer.updated_at_unix_ms,
            ],
        )?;
        transaction.execute(
            "DELETE FROM routes WHERE device_id = ?1",
            [device_id.as_slice()],
        )?;
        for route in &snapshot.routes {
            insert_route(&transaction, peer.id, route)?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn get_peer(&self, id: DeviceId) -> Result<Option<PeerRecord>> {
        let connection = self.lock_connection()?;
        let peer = connection
            .query_row(
                "SELECT device_id, display_name, public_key, trust, liveness,
                        connection, selected_route, updated_at
                   FROM peers WHERE device_id = ?1",
                [device_id_blob(id).as_slice()],
                row_to_peer,
            )
            .optional()?;
        peer.map(|mut peer| {
            peer.routes = load_routes(&connection, id)?;
            peer.snapshot()?;
            Ok(peer)
        })
        .transpose()
    }

    fn list_peers(&self) -> Result<Vec<PeerRecord>> {
        let connection = self.lock_connection()?;
        let mut statement = connection.prepare(
            "SELECT device_id, display_name, public_key, trust, liveness,
                    connection, selected_route, updated_at
               FROM peers ORDER BY device_id ASC",
        )?;
        let rows = statement.query_map([], row_to_peer)?;
        let mut peers = Vec::new();
        for row in rows {
            let mut peer = row?;
            peer.routes = load_routes(&connection, peer.id)?;
            peer.snapshot()?;
            peers.push(peer);
        }
        Ok(peers)
    }

    fn delete_peer(&self, id: DeviceId) -> Result<bool> {
        let connection = self.lock_connection()?;
        Ok(connection.execute(
            "DELETE FROM peers WHERE device_id = ?1",
            [device_id_blob(id).as_slice()],
        )? > 0)
    }

    fn upsert_route(&self, peer_id: DeviceId, route: RouteCandidate) -> Result<()> {
        let connection = self.lock_connection()?;
        let transaction = connection.unchecked_transaction()?;
        let exists: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM peers WHERE device_id = ?1",
                [device_id_blob(peer_id).as_slice()],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(StoreError::PeerNotFound(peer_id));
        }
        insert_route(&transaction, peer_id, &route)?;
        transaction.commit()?;
        Ok(())
    }

    fn list_routes(&self, peer_id: DeviceId) -> Result<Vec<RouteCandidate>> {
        let connection = self.lock_connection()?;
        load_routes(&connection, peer_id)
    }

    fn delete_route(&self, route_id: RouteId) -> Result<bool> {
        let connection = self.lock_connection()?;
        Ok(connection.execute(
            "DELETE FROM routes WHERE route_id = ?1",
            [route_id.to_string()],
        )? > 0)
    }
}

impl TrustStore for SQLiteStore {
    fn upsert(
        &self,
        record: TrustRecord,
    ) -> std::result::Result<(), unidesk_identity::IdentityError> {
        let connection = self
            .lock_connection()
            .map_err(|error| unidesk_identity::IdentityError::TrustStore(error.to_string()))?;
        connection
            .execute(
                "INSERT INTO trust_records (device_id, fingerprint, revoked)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(device_id) DO UPDATE SET
                    fingerprint = excluded.fingerprint,
                    revoked = excluded.revoked",
                params![
                    record.device_id.into_bytes().as_slice(),
                    record.fingerprint.into_bytes().as_slice(),
                    bool_to_sql(record.revoked),
                ],
            )
            .map_err(|error| unidesk_identity::IdentityError::TrustStore(error.to_string()))?;
        Ok(())
    }

    fn get(
        &self,
        device_id: &unidesk_identity::DeviceId,
    ) -> std::result::Result<Option<TrustRecord>, unidesk_identity::IdentityError> {
        let connection = self
            .lock_connection()
            .map_err(|error| unidesk_identity::IdentityError::TrustStore(error.to_string()))?;
        let row = connection
            .query_row(
                "SELECT device_id, fingerprint, revoked
                   FROM trust_records WHERE device_id = ?1",
                [device_id.into_bytes().as_slice()],
                |row| {
                    let device_id_bytes: Vec<u8> = row.get(0)?;
                    let fingerprint_bytes: Vec<u8> = row.get(1)?;
                    let device_id = device_id_from_blob(&device_id_bytes).map_err(to_sql_error)?;
                    let fingerprint: [u8; 32] =
                        fingerprint_bytes.try_into().map_err(|bytes: Vec<u8>| {
                            to_sql_error(StoreError::InvalidPublicKey {
                                expected: 32,
                                actual: bytes.len(),
                            })
                        })?;
                    let revoked = sql_to_bool(row.get::<_, i64>(2)?).ok_or_else(|| {
                        to_sql_error(StoreError::InvalidEnum {
                            kind: "trust_revoked",
                            value: row.get::<_, i64>(2).unwrap_or_default().to_string(),
                        })
                    })?;
                    Ok(TrustRecord {
                        device_id: unidesk_identity::DeviceId::from_bytes(device_id.into_bytes()),
                        fingerprint: Fingerprint::from_bytes(fingerprint),
                        revoked,
                    })
                },
            )
            .optional()
            .map_err(|error| unidesk_identity::IdentityError::TrustStore(error.to_string()))?;
        Ok(row)
    }

    fn list(&self) -> std::result::Result<Vec<TrustRecord>, unidesk_identity::IdentityError> {
        let connection = self
            .lock_connection()
            .map_err(|error| unidesk_identity::IdentityError::TrustStore(error.to_string()))?;
        let mut statement = connection
            .prepare(
                "SELECT device_id, fingerprint, revoked
                   FROM trust_records ORDER BY device_id ASC",
            )
            .map_err(|error| unidesk_identity::IdentityError::TrustStore(error.to_string()))?;
        let rows = statement
            .query_map([], |row| {
                let device_id_bytes: Vec<u8> = row.get(0)?;
                let fingerprint_bytes: Vec<u8> = row.get(1)?;
                let device_id = device_id_from_blob(&device_id_bytes).map_err(to_sql_error)?;
                let fingerprint: [u8; 32] =
                    fingerprint_bytes.try_into().map_err(|bytes: Vec<u8>| {
                        to_sql_error(StoreError::InvalidPublicKey {
                            expected: 32,
                            actual: bytes.len(),
                        })
                    })?;
                let revoked_value: i64 = row.get(2)?;
                let revoked = sql_to_bool(revoked_value).ok_or_else(|| {
                    to_sql_error(StoreError::InvalidEnum {
                        kind: "trust_revoked",
                        value: revoked_value.to_string(),
                    })
                })?;
                Ok(TrustRecord {
                    device_id: unidesk_identity::DeviceId::from_bytes(device_id.into_bytes()),
                    fingerprint: Fingerprint::from_bytes(fingerprint),
                    revoked,
                })
            })
            .map_err(|error| unidesk_identity::IdentityError::TrustStore(error.to_string()))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| unidesk_identity::IdentityError::TrustStore(error.to_string()))
    }

    fn revoke(
        &self,
        device_id: &unidesk_identity::DeviceId,
    ) -> std::result::Result<(), unidesk_identity::IdentityError> {
        let connection = self
            .lock_connection()
            .map_err(|error| unidesk_identity::IdentityError::TrustStore(error.to_string()))?;
        connection
            .execute(
                "UPDATE trust_records SET revoked = 1 WHERE device_id = ?1",
                [device_id.into_bytes().as_slice()],
            )
            .map_err(|error| unidesk_identity::IdentityError::TrustStore(error.to_string()))?;
        Ok(())
    }
}

impl SettingsStore for SQLiteStore {
    fn get_route_preferences(
        &self,
    ) -> Result<std::collections::BTreeMap<DeviceId, unidesk_core::RoutePreference>> {
        let connection = self.lock_connection()?;
        let value: Option<String> = connection
            .query_row(
                "SELECT value FROM settings WHERE key = 'route_preferences'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).map_err(StoreError::Serialization))
            .transpose()
            .map(Option::unwrap_or_default)
    }
    fn set_route_preferences(
        &self,
        preferences: std::collections::BTreeMap<DeviceId, unidesk_core::RoutePreference>,
    ) -> Result<()> {
        let value = serde_json::to_string(&preferences).map_err(StoreError::Serialization)?;
        self.lock_connection()?.execute("INSERT INTO settings (key,value) VALUES ('route_preferences',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [value])?;
        Ok(())
    }
    fn get_settings(&self) -> Result<Settings> {
        let connection = self.lock_connection()?;
        let value: Option<String> = connection
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [SETTINGS_PUBLIC_NETWORK_ENABLED],
                |row| row.get(0),
            )
            .optional()?;
        Ok(Settings {
            // Missing values are deliberately false for databases created by
            // older builds and for partially initialized legacy databases.
            public_network_enabled: value.as_deref().is_some_and(parse_bool),
        })
    }

    fn set_settings(&self, settings: Settings) -> Result<()> {
        let connection = self.lock_connection()?;
        connection.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![
                SETTINGS_PUBLIC_NETWORK_ENABLED,
                bool_to_str(settings.public_network_enabled)
            ],
        )?;
        Ok(())
    }

    fn public_network_enabled(&self) -> Result<bool> {
        Ok(self.get_settings()?.public_network_enabled)
    }

    fn set_public_network_enabled(&self, enabled: bool) -> Result<()> {
        self.set_settings(Settings {
            public_network_enabled: enabled,
        })
    }
}

impl ManualEndpointStore for SQLiteStore {
    fn add_manual_endpoint(&self, endpoint: ManualEndpoint) -> Result<bool> {
        let connection = self.lock_connection()?;
        Ok(connection.execute(
            "INSERT OR IGNORE INTO manual_endpoints (device_id, host, port)
             VALUES (?1, ?2, ?3)",
            params![
                device_id_blob(endpoint.peer_id).as_slice(),
                endpoint.endpoint.host,
                i64::from(endpoint.endpoint.port),
            ],
        )? > 0)
    }

    fn list_manual_endpoints(&self, peer_id: DeviceId) -> Result<Vec<ManualEndpoint>> {
        let connection = self.lock_connection()?;
        let mut statement = connection.prepare(
            "SELECT host, port FROM manual_endpoints
               WHERE device_id = ?1 ORDER BY host ASC, port ASC",
        )?;
        let rows = statement.query_map([device_id_blob(peer_id).as_slice()], |row| {
            let host: String = row.get(0)?;
            let port: i64 = row.get(1)?;
            Ok((host, port))
        })?;
        let mut endpoints = Vec::new();
        for row in rows {
            let (host, port) = row?;
            let port = u16::try_from(port).map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Integer,
                    Box::new(StoreError::InvalidEnum {
                        kind: "endpoint_port",
                        value: port.to_string(),
                    }),
                )
            })?;
            let endpoint = Endpoint::new(host, port).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(StoreError::InvalidEnum {
                        kind: "endpoint",
                        value: error.to_string(),
                    }),
                )
            })?;
            endpoints.push(ManualEndpoint::new(peer_id, endpoint));
        }
        Ok(endpoints)
    }

    fn delete_manual_endpoint(&self, endpoint: &ManualEndpoint) -> Result<bool> {
        let connection = self.lock_connection()?;
        Ok(connection.execute(
            "DELETE FROM manual_endpoints WHERE device_id = ?1 AND host = ?2 AND port = ?3",
            params![
                device_id_blob(endpoint.peer_id).as_slice(),
                endpoint.endpoint.host,
                i64::from(endpoint.endpoint.port),
            ],
        )? > 0)
    }
}

fn migrate_connection(connection: &mut Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version INTEGER PRIMARY KEY NOT NULL,
             applied_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS peers (
             device_id BLOB PRIMARY KEY NOT NULL,
             display_name TEXT NOT NULL,
             public_key BLOB NOT NULL,
             trust TEXT NOT NULL,
             liveness TEXT NOT NULL,
             connection TEXT NOT NULL,
             selected_route TEXT,
             updated_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS routes (
             route_id TEXT PRIMARY KEY NOT NULL,
             device_id BLOB NOT NULL,
             host TEXT NOT NULL,
             port INTEGER NOT NULL,
             source TEXT NOT NULL,
             health TEXT NOT NULL,
             last_success_unix_ms INTEGER,
             failure_count INTEGER NOT NULL,
             capabilities TEXT NOT NULL,
             FOREIGN KEY(device_id) REFERENCES peers(device_id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS routes_device_id_idx ON routes(device_id);
         CREATE TABLE IF NOT EXISTS settings (
             key TEXT PRIMARY KEY NOT NULL,
             value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS manual_endpoints (
             device_id BLOB NOT NULL,
             host TEXT NOT NULL,
             port INTEGER NOT NULL,
             PRIMARY KEY(device_id, host, port)
         );
         CREATE INDEX IF NOT EXISTS manual_endpoints_device_id_idx
             ON manual_endpoints(device_id);
         CREATE TABLE IF NOT EXISTS trust_records (
             device_id BLOB PRIMARY KEY NOT NULL,
             fingerprint BLOB NOT NULL,
             revoked INTEGER NOT NULL CHECK (revoked IN (0, 1))
         );",
    )?;

    let found: i64 = connection.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;
    if found > CURRENT_SCHEMA_VERSION {
        return Err(StoreError::UnsupportedSchemaVersion {
            found,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }
    if found < CURRENT_SCHEMA_VERSION {
        let transaction = connection.transaction()?;
        // Version 1 is the first schema. The settings key is inserted with a
        // safe false default; INSERT OR IGNORE also preserves legacy values.
        transaction.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
            params![SETTINGS_PUBLIC_NETWORK_ENABLED, bool_to_str(false)],
        )?;
        if found < 2 {
            backfill_trust_records(&transaction)?;
        }
        if found < 3 {
            transaction.execute_batch(
                "CREATE TABLE clipboard_history_v1 (
                    id INTEGER PRIMARY KEY NOT NULL,
                    position INTEGER NOT NULL,
                    payload BLOB NOT NULL
                );",
            )?;
        }
        if found < 4 {
            // Keep the binary snapshot encoding unchanged and preserve untimed
            // entries as NULL rather than inventing a copy date at migration.
            transaction.execute_batch(
                "ALTER TABLE clipboard_history_v1 ADD COLUMN copied_at_ms INTEGER;",
            )?;
        }
        transaction.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
            params![CURRENT_SCHEMA_VERSION, unix_time_ms()],
        )?;
        transaction.commit()?;
    } else {
        // A version-1 database may have been created by an early build that
        // omitted this key. Keep that database safe and compatible.
        connection.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
            params![SETTINGS_PUBLIC_NETWORK_ENABLED, bool_to_str(false)],
        )?;
    }
    Ok(())
}

fn backfill_trust_records(transaction: &Transaction<'_>) -> Result<()> {
    let mut statement = transaction.prepare(
        "SELECT device_id, public_key, trust FROM peers
           WHERE trust IN ('trusted', 'blocked')",
    )?;
    let rows = statement.query_map([], |row| {
        let device_id: Vec<u8> = row.get(0)?;
        let public_key: Vec<u8> = row.get(1)?;
        let trust: String = row.get(2)?;
        Ok((device_id, public_key, trust))
    })?;
    let mut records = Vec::new();
    for row in rows {
        let (stored_id, public_key, trust) = row?;
        let Ok(stored_id) = device_id_from_blob(&stored_id) else {
            continue;
        };
        let Ok(public_key) = <[u8; 32]>::try_from(public_key) else {
            continue;
        };
        // Legacy rows are trusted only when their public key derives exactly
        // the stored device id.  A malformed or mismatched row is ignored,
        // which is safer than granting trust during migration.
        let Ok(derived_id) = DeviceId::from_public_key(public_key) else {
            continue;
        };
        if derived_id != stored_id {
            continue;
        }
        records.push((stored_id, public_key, trust == "blocked"));
    }
    drop(statement);
    for (device_id, _public_key, revoked) in records {
        transaction.execute(
            "INSERT OR IGNORE INTO trust_records (device_id, fingerprint, revoked)
             VALUES (?1, ?2, ?3)",
            params![
                device_id.into_bytes().as_slice(),
                device_id.into_bytes().as_slice(),
                bool_to_sql(revoked),
            ],
        )?;
    }
    Ok(())
}

fn insert_route(
    transaction: &Transaction<'_>,
    peer_id: DeviceId,
    route: &RouteCandidate,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO routes (
             route_id, device_id, host, port, source, health,
             last_success_unix_ms, failure_count, capabilities
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(route_id) DO UPDATE SET
             device_id = excluded.device_id,
             host = excluded.host,
             port = excluded.port,
             source = excluded.source,
             health = excluded.health,
             last_success_unix_ms = excluded.last_success_unix_ms,
             failure_count = excluded.failure_count,
             capabilities = excluded.capabilities",
        params![
            route.id.to_string(),
            device_id_blob(peer_id).as_slice(),
            route.endpoint.host,
            i64::from(route.endpoint.port),
            route_source_to_str(route.source),
            route_health_to_str(route.health),
            route.last_success_unix_ms.map(|value| value as i64),
            i64::from(route.failure_count),
            route.capabilities.bits().to_string(),
        ],
    )?;
    Ok(())
}

fn row_to_peer(row: &Row<'_>) -> rusqlite::Result<PeerRecord> {
    let id_bytes: Vec<u8> = row.get(0)?;
    let id = device_id_from_blob(&id_bytes).map_err(to_sql_error)?;
    let display_name = row.get(1)?;
    let public_key_vec: Vec<u8> = row.get(2)?;
    let public_key: [u8; 32] = public_key_vec.try_into().map_err(|bytes: Vec<u8>| {
        to_sql_error(StoreError::InvalidPublicKey {
            expected: 32,
            actual: bytes.len(),
        })
    })?;
    let trust = trust_from_str(&row.get::<_, String>(3)?).map_err(to_sql_error)?;
    let liveness = liveness_from_str(&row.get::<_, String>(4)?).map_err(to_sql_error)?;
    let connection = connection_from_str(&row.get::<_, String>(5)?).map_err(to_sql_error)?;
    let selected_route = row
        .get::<_, Option<String>>(6)?
        .map(|value| {
            value
                .parse::<RouteId>()
                .map_err(|_| StoreError::InvalidRouteId { value })
        })
        .transpose()
        .map_err(to_sql_error)?;
    let updated_at_unix_ms = row.get(7)?;
    Ok(PeerRecord {
        id,
        display_name,
        public_key,
        trust,
        liveness,
        connection,
        routes: Vec::new(),
        selected_route,
        updated_at_unix_ms,
    })
}

fn load_routes(connection: &Connection, peer_id: DeviceId) -> Result<Vec<RouteCandidate>> {
    let mut statement = connection.prepare(
        "SELECT route_id, host, port, source, health,
                last_success_unix_ms, failure_count, capabilities
           FROM routes WHERE device_id = ?1 ORDER BY route_id ASC",
    )?;
    let rows = statement.query_map([device_id_blob(peer_id).as_slice()], |row| {
        let route_id_text: String = row.get(0)?;
        let route_id = route_id_text.parse::<RouteId>().map_err(|_| {
            to_sql_error(StoreError::InvalidRouteId {
                value: route_id_text,
            })
        })?;
        let host: String = row.get(1)?;
        let port: i64 = row.get(2)?;
        let port = u16::try_from(port).map_err(|_| {
            to_sql_error(StoreError::InvalidEnum {
                kind: "endpoint_port",
                value: port.to_string(),
            })
        })?;
        let endpoint = Endpoint::new(host, port)
            .map_err(|error| to_sql_error(StoreError::InvalidPeer(error.to_string())))?;
        let source = route_source_from_str(&row.get::<_, String>(3)?).map_err(to_sql_error)?;
        let health = route_health_from_str(&row.get::<_, String>(4)?).map_err(to_sql_error)?;
        let last_success_unix_ms = row.get(5)?;
        let failure_count = row.get::<_, i64>(6)?;
        let failure_count = u32::try_from(failure_count).map_err(|_| {
            to_sql_error(StoreError::InvalidEnum {
                kind: "failure_count",
                value: failure_count.to_string(),
            })
        })?;
        let capabilities_text: String = row.get(7)?;
        let capabilities = capabilities_text.parse::<u64>().map_err(|_| {
            to_sql_error(StoreError::InvalidEnum {
                kind: "capabilities",
                value: capabilities_text,
            })
        })?;
        Ok(RouteCandidate::with_id(
            route_id,
            endpoint,
            source,
            health,
            last_success_unix_ms,
            failure_count,
            unidesk_core::Capabilities::from_bits(capabilities),
        ))
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(StoreError::from)
}

fn to_sql_error(error: StoreError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, Box::new(error))
}

fn device_id_blob(id: DeviceId) -> [u8; 32] {
    id.into_bytes()
}

fn device_id_from_blob(bytes: &[u8]) -> Result<DeviceId> {
    let array: [u8; 32] = bytes.try_into().map_err(|_| StoreError::InvalidDeviceId {
        expected: 32,
        actual: bytes.len(),
    })?;
    Ok(DeviceId::from_bytes(array))
}

fn unix_time_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn bool_to_str(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn bool_to_sql(value: bool) -> i64 {
    i64::from(value)
}

fn sql_to_bool(value: i64) -> Option<bool> {
    match value {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(value, "1" | "true" | "TRUE" | "True" | "yes" | "on")
}

fn trust_to_str(value: TrustState) -> &'static str {
    match value {
        TrustState::Unknown => "unknown",
        TrustState::Pairing => "pairing",
        TrustState::Pending => "pending",
        TrustState::Trusted => "trusted",
        TrustState::Blocked => "blocked",
    }
}

fn trust_from_str(value: &str) -> Result<TrustState> {
    match value {
        "unknown" => Ok(TrustState::Unknown),
        "pairing" => Ok(TrustState::Pairing),
        "pending" => Ok(TrustState::Pending),
        "trusted" => Ok(TrustState::Trusted),
        "blocked" => Ok(TrustState::Blocked),
        value => Err(StoreError::InvalidEnum {
            kind: "trust",
            value: value.to_owned(),
        }),
    }
}

fn liveness_to_str(value: LivenessState) -> &'static str {
    match value {
        LivenessState::Unknown => "unknown",
        LivenessState::Online => "online",
        LivenessState::Offline => "offline",
        LivenessState::Stale => "stale",
    }
}

fn liveness_from_str(value: &str) -> Result<LivenessState> {
    match value {
        "unknown" => Ok(LivenessState::Unknown),
        "online" => Ok(LivenessState::Online),
        "offline" => Ok(LivenessState::Offline),
        "stale" => Ok(LivenessState::Stale),
        value => Err(StoreError::InvalidEnum {
            kind: "liveness",
            value: value.to_owned(),
        }),
    }
}

fn connection_to_str(value: ConnectionState) -> &'static str {
    match value {
        ConnectionState::Disconnected => "disconnected",
        ConnectionState::Connecting => "connecting",
        ConnectionState::Connected => "connected",
        ConnectionState::Backoff => "backoff",
        ConnectionState::Failed => "failed",
    }
}

fn connection_from_str(value: &str) -> Result<ConnectionState> {
    match value {
        "disconnected" => Ok(ConnectionState::Disconnected),
        "connecting" => Ok(ConnectionState::Connecting),
        "connected" => Ok(ConnectionState::Connected),
        "backoff" => Ok(ConnectionState::Backoff),
        "failed" => Ok(ConnectionState::Failed),
        value => Err(StoreError::InvalidEnum {
            kind: "connection",
            value: value.to_owned(),
        }),
    }
}

fn route_source_to_str(value: RouteSource) -> &'static str {
    match value {
        RouteSource::Mdns => "mdns",
        RouteSource::Manual => "manual",
        RouteSource::CachedDirect => "cached_direct",
        RouteSource::PublicDirect => "public_direct",
        RouteSource::PublicRelay => "public_relay",
    }
}

fn route_source_from_str(value: &str) -> Result<RouteSource> {
    match value {
        "mdns" => Ok(RouteSource::Mdns),
        "manual" => Ok(RouteSource::Manual),
        "cached_direct" | "cached" => Ok(RouteSource::CachedDirect),
        "public_direct" => Ok(RouteSource::PublicDirect),
        "public_relay" => Ok(RouteSource::PublicRelay),
        value => Err(StoreError::InvalidEnum {
            kind: "route_source",
            value: value.to_owned(),
        }),
    }
}

fn route_health_to_str(value: RouteHealth) -> &'static str {
    match value {
        RouteHealth::Healthy => "healthy",
        RouteHealth::Unknown => "unknown",
        RouteHealth::Backoff => "backoff",
        RouteHealth::Unavailable => "unavailable",
    }
}

fn route_health_from_str(value: &str) -> Result<RouteHealth> {
    match value {
        "healthy" => Ok(RouteHealth::Healthy),
        "unknown" => Ok(RouteHealth::Unknown),
        "backoff" => Ok(RouteHealth::Backoff),
        "unavailable" => Ok(RouteHealth::Unavailable),
        value => Err(StoreError::InvalidEnum {
            kind: "route_health",
            value: value.to_owned(),
        }),
    }
}

/// Stable JSON encoding for a peer record, useful for export or diagnostics.
pub fn serialize_peer(peer: &PeerRecord) -> Result<String> {
    Ok(serde_json::to_string(peer)?)
}

/// Decode a peer record exported by [`serialize_peer`].
pub fn deserialize_peer(value: &str) -> Result<PeerRecord> {
    let peer: PeerRecord = serde_json::from_str(value)?;
    peer.snapshot()?;
    Ok(peer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use unidesk_core::Capabilities;
    use unidesk_identity::{Fingerprint, TrustRecord, TrustStore};

    fn peer() -> PeerRecord {
        let public_key = [7_u8; 32];
        let id = DeviceId::from_public_key(public_key).expect("valid public key");
        let route = RouteCandidate::new(
            Endpoint::new("192.0.2.10", 24_000).expect("valid endpoint"),
            RouteSource::Manual,
            RouteHealth::Healthy,
            Capabilities::NETWORK | Capabilities::INPUT,
        );
        let route_id = route.id;
        PeerRecord::new(
            id,
            "Test Mac",
            public_key,
            TrustState::Trusted,
            LivenessState::Online,
            ConnectionState::Connected,
            vec![route],
            Some(route_id),
            1234,
        )
        .expect("valid peer")
    }

    #[test]
    fn migrations_are_idempotent_and_have_safe_default() {
        let store = SQLiteStore::open_in_memory().expect("open store");
        assert_eq!(
            store.schema_version().expect("version"),
            CURRENT_SCHEMA_VERSION
        );
        store.migrate().expect("second migration");
        assert_eq!(
            store.schema_version().expect("version"),
            CURRENT_SCHEMA_VERSION
        );
        assert!(!store.public_network_enabled().expect("default setting"));
    }

    #[test]
    fn peer_and_routes_round_trip() {
        let store = SQLiteStore::open_in_memory().expect("open store");
        let original = peer();
        store.upsert_peer(original.clone()).expect("write peer");
        let loaded = store
            .get_peer(original.id)
            .expect("read peer")
            .expect("peer exists");
        assert_eq!(loaded, original);
        assert_eq!(
            store.list_routes(original.id).expect("routes"),
            original.routes
        );
    }

    #[test]
    fn manual_endpoints_are_deduplicated_and_can_be_deleted() {
        let store = SQLiteStore::open_in_memory().expect("open store");
        let id = peer().id;
        let endpoint =
            ManualEndpoint::new(id, Endpoint::new("example.test", 443).expect("endpoint"));
        assert!(store.add_manual_endpoint(endpoint.clone()).expect("insert"));
        assert!(
            !store
                .add_manual_endpoint(endpoint.clone())
                .expect("duplicate")
        );
        assert_eq!(
            store.list_manual_endpoints(id).expect("list"),
            vec![endpoint.clone()]
        );
        assert!(store.delete_manual_endpoint(&endpoint).expect("delete"));
        assert!(!store.delete_manual_endpoint(&endpoint).expect("missing"));
    }

    #[test]
    fn legacy_database_without_public_flag_defaults_to_false() {
        let connection = Connection::open_in_memory().expect("legacy db");
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY NOT NULL, applied_at INTEGER NOT NULL);
                 INSERT INTO schema_migrations(version, applied_at) VALUES (1, 1);
                 CREATE TABLE peers(device_id BLOB PRIMARY KEY NOT NULL, display_name TEXT NOT NULL, public_key BLOB NOT NULL, trust TEXT NOT NULL, liveness TEXT NOT NULL, connection TEXT NOT NULL, selected_route TEXT, updated_at INTEGER NOT NULL);
                 CREATE TABLE routes(route_id TEXT PRIMARY KEY NOT NULL, device_id BLOB NOT NULL, host TEXT NOT NULL, port INTEGER NOT NULL, source TEXT NOT NULL, health TEXT NOT NULL, last_success_unix_ms INTEGER, failure_count INTEGER NOT NULL, capabilities TEXT NOT NULL);
                 CREATE TABLE settings(key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
                 CREATE TABLE manual_endpoints(device_id BLOB NOT NULL, host TEXT NOT NULL, port INTEGER NOT NULL, PRIMARY KEY(device_id, host, port));",
            )
            .expect("legacy schema");
        let store = SQLiteStore::from_connection(connection).expect("migrate legacy");
        assert!(!store.public_network_enabled().expect("default"));
    }

    #[test]
    fn update_and_delete_work() {
        let store = SQLiteStore::open_in_memory().expect("open store");
        let mut value = peer();
        store.upsert_peer(value.clone()).expect("insert");
        value.display_name = "Renamed".to_owned();
        value.updated_at_unix_ms = 9999;
        store.upsert_peer(value.clone()).expect("update");
        assert_eq!(
            store.get_peer(value.id).expect("read").expect("peer"),
            value
        );
        assert!(store.delete_peer(value.id).expect("delete"));
        assert!(store.get_peer(value.id).expect("read").is_none());
    }

    #[test]
    fn settings_update_and_store_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SQLiteStore>();
        let store = Arc::new(SQLiteStore::open_in_memory().expect("open store"));
        store.set_public_network_enabled(true).expect("enable");
        assert!(store.public_network_enabled().expect("read"));
        store.set_settings(Settings::default()).expect("reset");
        assert!(!store.public_network_enabled().expect("read"));
    }

    #[test]
    fn trust_store_round_trip_clone_and_revoke() {
        let store = SQLiteStore::open_in_memory().expect("open store");
        let public_key = [7_u8; 32];
        let device_id = unidesk_identity::DeviceId::from_public_key(public_key).expect("id");
        let record = TrustRecord::new(
            device_id,
            Fingerprint::from_public_key(public_key).expect("fingerprint"),
        );
        let clone = store.clone();
        TrustStore::upsert(&clone, record.clone()).expect("trust");
        assert_eq!(
            TrustStore::get(&store, &device_id).expect("get"),
            Some(record.clone())
        );
        assert!(store.is_trusted(&device_id).expect("is trusted"));
        store.revoke(&device_id).expect("revoke");
        assert!(!clone.is_trusted(&device_id).expect("revoked"));
        assert_eq!(
            TrustStore::list(&clone).expect("list"),
            vec![TrustRecord::revoked(
                device_id,
                Fingerprint::from_public_key(public_key).expect("fingerprint"),
            )]
        );
    }

    #[test]
    fn v1_trust_backfill_validates_public_key_binding() {
        let connection = Connection::open_in_memory().expect("legacy db");
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY NOT NULL, applied_at INTEGER NOT NULL);
                 INSERT INTO schema_migrations(version, applied_at) VALUES (1, 1);
                 CREATE TABLE peers(device_id BLOB PRIMARY KEY NOT NULL, display_name TEXT NOT NULL, public_key BLOB NOT NULL, trust TEXT NOT NULL, liveness TEXT NOT NULL, connection TEXT NOT NULL, selected_route TEXT, updated_at INTEGER NOT NULL);
                 CREATE TABLE routes(route_id TEXT PRIMARY KEY NOT NULL, device_id BLOB NOT NULL, host TEXT NOT NULL, port INTEGER NOT NULL, source TEXT NOT NULL, health TEXT NOT NULL, last_success_unix_ms INTEGER, failure_count INTEGER NOT NULL, capabilities TEXT NOT NULL);
                 CREATE TABLE settings(key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
                 CREATE TABLE manual_endpoints(device_id BLOB NOT NULL, host TEXT NOT NULL, port INTEGER NOT NULL, PRIMARY KEY(device_id, host, port));",
            )
            .expect("legacy schema");
        let trusted_key = [9_u8; 32];
        let trusted_id = DeviceId::from_public_key(trusted_key).expect("trusted id");
        connection
            .execute(
                "INSERT INTO peers(device_id, display_name, public_key, trust, liveness, connection, selected_route, updated_at)
                 VALUES (?1, 'trusted', ?2, 'trusted', 'offline', 'disconnected', NULL, 1)",
                params![trusted_id.into_bytes().as_slice(), trusted_key.as_slice()],
            )
            .expect("trusted peer");
        let blocked_key = [10_u8; 32];
        let blocked_id = DeviceId::from_public_key(blocked_key).expect("blocked id");
        connection
            .execute(
                "INSERT INTO peers(device_id, display_name, public_key, trust, liveness, connection, selected_route, updated_at)
                 VALUES (?1, 'blocked', ?2, 'blocked', 'offline', 'disconnected', NULL, 1)",
                params![blocked_id.into_bytes().as_slice(), blocked_key.as_slice()],
            )
            .expect("blocked peer");
        let mismatched_id = DeviceId::from_public_key([11_u8; 32]).expect("mismatch id");
        connection
            .execute(
                "INSERT INTO peers(device_id, display_name, public_key, trust, liveness, connection, selected_route, updated_at)
                 VALUES (?1, 'mismatch', ?2, 'trusted', 'offline', 'disconnected', NULL, 1)",
                params![mismatched_id.into_bytes().as_slice(), [12_u8; 32].as_slice()],
            )
            .expect("mismatched peer");

        let store = SQLiteStore::from_connection(connection).expect("migrate legacy");
        let records = TrustStore::list(&store).expect("trust records");
        assert_eq!(records.len(), 2);
        assert_eq!(
            TrustStore::get(&store, &trusted_id).expect("trusted"),
            Some(TrustRecord::new(
                trusted_id,
                Fingerprint::from_public_key(trusted_key).expect("fingerprint"),
            ))
        );
        assert_eq!(
            TrustStore::get(&store, &blocked_id).expect("blocked"),
            Some(TrustRecord::revoked(
                blocked_id,
                Fingerprint::from_public_key(blocked_key).expect("fingerprint"),
            ))
        );
        assert!(
            TrustStore::get(&store, &mismatched_id)
                .expect("mismatched")
                .is_none()
        );
        store.migrate().expect("idempotent migration");
        assert_eq!(TrustStore::list(&store).expect("trust records").len(), 2);
    }

    #[test]
    fn stable_serialization_round_trip() {
        let original = peer();
        let serialized = serialize_peer(&original).expect("serialize");
        let decoded = deserialize_peer(&serialized).expect("deserialize");
        assert_eq!(decoded, original);
        assert_eq!(serialize_peer(&decoded).expect("serialize"), serialized);
    }

    #[test]
    fn default_path_is_never_used_by_isolated_constructor() {
        let store = SQLiteStore::open_in_memory().expect("open store");
        store.set_public_network_enabled(true).expect("setting");
        assert!(store.public_network_enabled().expect("read"));
        // Keep this test intentionally free of SQLiteStore::new(), which would
        // touch the user's real application database.
    }
}

/// Independent versioned input configuration; legacy public-network writes never replace it.
pub trait InputSharingStore: Send + Sync {
    /// Load enable-cycle epochs and emergency pauses; malformed rows are errors.
    fn get_input_activation_policy(&self) -> Result<Option<unidesk_core::InputActivationPolicy>>;
    /// Atomically replace the independent local activation policy row.
    fn set_input_activation_policy(
        &self,
        policy: unidesk_core::InputActivationPolicy,
    ) -> Result<()>;
    /// Commit a local settings edge and its enable-cycle policy in one transaction.
    fn set_input_sharing_settings_and_policy(
        &self,
        settings: unidesk_core::InputSharingSettings,
        policy: unidesk_core::InputActivationPolicy,
    ) -> Result<()>;
    /// Read the independently persisted named input configurations.
    fn get_input_sharing_scenes(&self) -> Result<Vec<unidesk_core::InputSharingScene>>;
    /// Validate and atomically replace the named input configurations.
    fn set_input_sharing_scenes(&self, scenes: Vec<unidesk_core::InputSharingScene>) -> Result<()>;
    /// Read settings, returning disabled defaults for missing or malformed rows.
    fn get_input_sharing_settings(&self) -> Result<unidesk_core::InputSharingSettings>;
    /// Validate and atomically persist only the input-sharing row.
    fn set_input_sharing_settings(
        &self,
        settings: unidesk_core::InputSharingSettings,
    ) -> Result<()>;
}
impl InputSharingStore for SQLiteStore {
    fn set_input_sharing_settings_and_policy(
        &self,
        settings: unidesk_core::InputSharingSettings,
        policy: unidesk_core::InputActivationPolicy,
    ) -> Result<()> {
        settings
            .validate()
            .map_err(|error| StoreError::Migration(error.to_string()))?;
        policy
            .validate()
            .map_err(|error| StoreError::Migration(error.to_string()))?;
        let settings = serde_json::to_string(&settings)?;
        let policy = serde_json::to_string(&policy)?;
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction()?;
        for (key, value) in [
            ("input_sharing_v1", settings),
            ("input_activation_policy_v1", policy),
        ] {
            transaction.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, value])?;
        }
        transaction.commit()?;
        Ok(())
    }
    fn get_input_activation_policy(&self) -> Result<Option<unidesk_core::InputActivationPolicy>> {
        let value: Option<String> = self
            .lock_connection()?
            .query_row(
                "SELECT value FROM settings WHERE key='input_activation_policy_v1'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(value) = value else {
            return Ok(None);
        };
        let policy: unidesk_core::InputActivationPolicy = serde_json::from_str(&value)?;
        policy
            .validate()
            .map_err(|error| StoreError::Migration(error.to_string()))?;
        Ok(Some(policy))
    }
    fn set_input_activation_policy(
        &self,
        policy: unidesk_core::InputActivationPolicy,
    ) -> Result<()> {
        policy
            .validate()
            .map_err(|error| StoreError::Migration(error.to_string()))?;
        let value = serde_json::to_string(&policy)?;
        self.lock_connection()?.execute("INSERT INTO settings(key,value) VALUES('input_activation_policy_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [value])?;
        Ok(())
    }
    fn get_input_sharing_scenes(&self) -> Result<Vec<unidesk_core::InputSharingScene>> {
        let value: Option<String> = self
            .lock_connection()?
            .query_row(
                "SELECT value FROM settings WHERE key='input_sharing_scenes_v1'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(value
            .and_then(|v| serde_json::from_str::<Vec<unidesk_core::InputSharingScene>>(&v).ok())
            .filter(|v| unidesk_core::InputSharingScene::validate_list(v).is_ok())
            .unwrap_or_default())
    }
    fn set_input_sharing_scenes(&self, scenes: Vec<unidesk_core::InputSharingScene>) -> Result<()> {
        unidesk_core::InputSharingScene::validate_list(&scenes)
            .map_err(|e| StoreError::Migration(e.to_string()))?;
        let value = serde_json::to_string(&scenes)?;
        self.lock_connection()?.execute("INSERT INTO settings(key,value) VALUES('input_sharing_scenes_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[value])?;
        Ok(())
    }
    fn get_input_sharing_settings(&self) -> Result<unidesk_core::InputSharingSettings> {
        let connection = self.lock_connection()?;
        let value: Option<String> = connection
            .query_row(
                "SELECT value FROM settings WHERE key = 'input_sharing_v1'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value
            .and_then(|v| serde_json::from_str::<unidesk_core::InputSharingSettings>(&v).ok())
            .filter(|s| s.validate().is_ok())
            .unwrap_or_default())
    }
    fn set_input_sharing_settings(
        &self,
        settings: unidesk_core::InputSharingSettings,
    ) -> Result<()> {
        settings
            .validate()
            .map_err(|error| StoreError::Migration(error.to_string()))?;
        let value = serde_json::to_string(&settings)?;
        self.lock_connection()?.execute("INSERT INTO settings (key,value) VALUES ('input_sharing_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[value])?;
        Ok(())
    }
}

#[cfg(test)]
mod input_store_tests {
    use super::*;
    #[test]
    fn activation_policy_round_trip_invalid_rows_fail_closed_and_settings_edges_are_atomic() {
        use unidesk_core::{
            InputActivationId, InputActivationPolicy, InputShareRole, InputSharingSettings,
        };
        let store = SQLiteStore::in_memory().unwrap();
        assert_eq!(store.get_input_activation_policy().unwrap(), None);
        let peer = DeviceId::from_bytes([4; 32]);
        let intent = InputActivationId::new();
        let policy = InputActivationPolicy {
            environment_rules: Default::default(),
            controller_intents: std::collections::BTreeMap::from([(peer, intent)]),
            paused_intents: std::collections::BTreeSet::from([(peer, InputActivationId::new())]),
        };
        store.set_input_activation_policy(policy.clone()).unwrap();
        assert_eq!(
            store.get_input_activation_policy().unwrap(),
            Some(policy.clone())
        );
        assert_eq!(
            store.get_input_sharing_settings().unwrap(),
            InputSharingSettings::default()
        );
        let settings = InputSharingSettings {
            enabled: true,
            role: InputShareRole::Controller,
            participants: vec![peer],
            ..Default::default()
        };
        store
            .set_input_sharing_settings_and_policy(settings.clone(), policy.clone())
            .unwrap();
        assert_eq!(store.get_input_sharing_settings().unwrap(), settings);
        let mut duplicate_peer = policy.clone();
        duplicate_peer
            .paused_intents
            .insert((peer, InputActivationId::new()));
        assert!(
            store
                .set_input_activation_policy(duplicate_peer.clone())
                .is_err()
        );
        // A malformed stored policy must fail closed on load as well as write.
        store
            .lock_connection()
            .unwrap()
            .execute(
                "UPDATE settings SET value=?1 WHERE key='input_activation_policy_v1'",
                [serde_json::to_string(&duplicate_peer).unwrap()],
            )
            .unwrap();
        assert!(store.get_input_activation_policy().is_err());
        store.set_input_activation_policy(policy.clone()).unwrap();
        let mut invalid = policy.clone();
        invalid.controller_intents.insert(
            peer,
            serde_json::from_str("\"00000000-0000-0000-0000-000000000000\"").unwrap(),
        );
        assert!(
            store
                .set_input_sharing_settings_and_policy(InputSharingSettings::default(), invalid)
                .is_err()
        );
        assert_eq!(store.get_input_sharing_settings().unwrap(), settings);
        assert_eq!(store.get_input_activation_policy().unwrap(), Some(policy));
        store
            .lock_connection()
            .unwrap()
            .execute(
                "UPDATE settings SET value='invalid' WHERE key='input_activation_policy_v1'",
                [],
            )
            .unwrap();
        assert!(store.get_input_activation_policy().is_err());
        assert_eq!(store.get_input_sharing_settings().unwrap(), settings);
    }
    #[test]
    fn input_settings_are_independent_and_bad_rows_fail_closed() {
        let store = SQLiteStore::in_memory().unwrap();
        let mut settings = unidesk_core::InputSharingSettings {
            enabled: true,
            role: unidesk_core::InputShareRole::Controller,
            ..Default::default()
        };
        store.set_input_sharing_settings(settings.clone()).unwrap();
        store.set_public_network_enabled(true).unwrap();
        assert_eq!(store.get_input_sharing_settings().unwrap(), settings);
        assert!(store.get_settings().unwrap().public_network_enabled);
        settings.disabled_corners = 255;
        assert!(store.set_input_sharing_settings(settings).is_err());
        store
            .lock_connection()
            .unwrap()
            .execute(
                "UPDATE settings SET value='broken' WHERE key='input_sharing_v1'",
                [],
            )
            .unwrap();
        assert!(!store.get_input_sharing_settings().unwrap().enabled);
    }
}

/// Owner-signed device-network state is independent from input/public configuration.
pub trait DeviceNetworkStore: Send + Sync {
    /// Read local trust provenance independently from the owner-signed roster.
    fn get_network_trust_origins(&self) -> Result<Option<unidesk_identity::NetworkTrustOrigins>>;
    /// Persist local trust provenance before changing membership-derived trust.
    fn set_network_trust_origins(
        &self,
        origins: unidesk_identity::NetworkTrustOrigins,
    ) -> Result<()>;

    /// Read and verify the last durable owner roster.
    fn get_device_network(&self) -> Result<Option<unidesk_identity::SignedNetworkRoster>>;
    /// Verify and atomically persist the roster, or clear membership.
    fn set_device_network(
        &self,
        roster: Option<unidesk_identity::SignedNetworkRoster>,
    ) -> Result<()>;
}
impl DeviceNetworkStore for SQLiteStore {
    fn get_network_trust_origins(&self) -> Result<Option<unidesk_identity::NetworkTrustOrigins>> {
        let value: Option<String> = self
            .lock_connection()?
            .query_row(
                "SELECT value FROM settings WHERE key='device_network_trust_v1'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        value
            .map(|v| serde_json::from_str(&v).map_err(StoreError::Serialization))
            .transpose()
    }
    fn set_network_trust_origins(
        &self,
        origins: unidesk_identity::NetworkTrustOrigins,
    ) -> Result<()> {
        let value = serde_json::to_string(&origins)?;
        self.lock_connection()?.execute("INSERT INTO settings(key,value) VALUES('device_network_trust_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[value])?;
        Ok(())
    }

    fn get_device_network(&self) -> Result<Option<unidesk_identity::SignedNetworkRoster>> {
        let value: Option<String> = self
            .lock_connection()?
            .query_row(
                "SELECT value FROM settings WHERE key='device_network_v1'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let Some(value) = value else {
            return Ok(None);
        };
        let roster: unidesk_identity::SignedNetworkRoster = serde_json::from_str(&value)?;
        roster.verify().map_err(StoreError::Migration)?;
        Ok(Some(roster))
    }
    fn set_device_network(
        &self,
        roster: Option<unidesk_identity::SignedNetworkRoster>,
    ) -> Result<()> {
        if let Some(roster) = roster {
            roster.verify().map_err(StoreError::Migration)?;
            let value = serde_json::to_string(&roster)?;
            self.lock_connection()?.execute("INSERT INTO settings(key,value) VALUES('device_network_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[value])?;
        } else {
            self.lock_connection()?
                .execute("DELETE FROM settings WHERE key='device_network_v1'", [])?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod network_config_tests {
    use super::*;
    #[test]
    fn corrupted_network_is_distinct_from_absent_and_scene_rows_are_independent() {
        let store = SQLiteStore::in_memory().unwrap();
        assert!(store.get_device_network().unwrap().is_none());
        store
            .lock_connection()
            .unwrap()
            .execute(
                "INSERT INTO settings(key,value) VALUES('device_network_v1','broken')",
                [],
            )
            .unwrap();
        assert!(store.get_device_network().is_err());
        let identity = unidesk_identity::LocalIdentity::from_bytes(&[111; 32]).unwrap();
        let owner = unidesk_identity::NetworkMember {
            device_id: identity.device_id(),
            public_key: identity.verifying_key_bytes(),
            display_name: "owner".into(),
            capabilities: unidesk_core::Capabilities::DEVICE_NETWORK,
            endpoint: Endpoint::new("10.0.0.1", 1234).unwrap(),
        };
        let mut signed = unidesk_identity::SignedNetworkRoster::signed(
            unidesk_identity::NetworkRoster {
                merge_permits: Vec::new(),
                network_id: unidesk_core::RequestId::new(),
                owner: owner.clone(),
                epoch: 1,
                members: vec![owner],
            },
            &identity,
        )
        .unwrap();
        store.set_device_network(Some(signed.clone())).unwrap();
        assert!(store.get_device_network().unwrap().is_some());
        signed.signature[0] ^= 1;
        store
            .lock_connection()
            .unwrap()
            .execute(
                "UPDATE settings SET value=?1 WHERE key='device_network_v1'",
                [serde_json::to_string(&signed).unwrap()],
            )
            .unwrap();
        assert!(store.get_device_network().is_err());
        let scene =
            unidesk_core::InputSharingScene::new("Desk".into(), Default::default()).unwrap();
        store.set_input_sharing_scenes(vec![scene.clone()]).unwrap();
        store.set_public_network_enabled(true).unwrap();
        assert_eq!(
            store.get_input_sharing_scenes().unwrap(),
            vec![scene.clone()]
        );
        assert!(
            store
                .set_input_sharing_scenes(vec![scene.clone(), scene])
                .is_err()
        );
        store.set_device_network(None).unwrap();
        assert!(store.get_device_network().unwrap().is_none());
        assert_eq!(store.get_input_sharing_scenes().unwrap().len(), 1);
    }
    #[test]
    fn file_transfer_policy_is_persistent_and_does_not_replace_other_settings() {
        let store = SQLiteStore::open_in_memory().unwrap();
        assert_eq!(
            store.file_transfer_settings().unwrap(),
            unidesk_core::FileTransferSettings::default()
        );
        let settings = unidesk_core::FileTransferSettings {
            small_file_prefetch_enabled: true,
            small_file_threshold_bytes: 20 * 1024 * 1024,
            cache_max_bytes: 64 * 1024 * 1024,
        };
        store.set_file_transfer_settings(&settings).unwrap();
        store.set_public_network_enabled(true).unwrap();
        assert_eq!(store.file_transfer_settings().unwrap(), settings);
        let invalid = unidesk_core::FileTransferSettings {
            cache_max_bytes: 1,
            ..settings.clone()
        };
        assert!(store.set_file_transfer_settings(&invalid).is_err());
        assert_eq!(store.file_transfer_settings().unwrap(), settings);
    }
}
