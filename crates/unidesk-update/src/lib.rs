//! Release authorization is independent of pairing. Never install an unverified manifest.
#![forbid(unsafe_code)]
mod artifact;
mod io;
pub use io::{pinned_release_key, write_json_atomic};
mod cache;
mod coordinator;
#[cfg(target_os = "macos")]
pub mod installer;
mod manifest;
mod selection;
mod transaction;

pub use artifact::{VerifiedArtifact, verify_artifact};
pub use cache::{CachedArtifact, ReleaseCache};
pub use coordinator::allocate_release;
pub use manifest::*;
pub use selection::*;
pub use transaction::*;

/// Separate from clipboard/file cache allowance.
pub const UPDATE_CACHE_LIMIT: u64 = 4 * 1024 * 1024 * 1024;
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub const SELF_TEST_TIMEOUT_MS: u64 = 60_000;

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("invalid release metadata: {0}")]
    Invalid(&'static str),
    #[error("release signature is invalid")]
    Signature,
    #[error("artifact size or SHA-256 does not match the signed manifest")]
    ArtifactMismatch,
    #[error("conflicting artifacts for the same release version and build")]
    Conflict,
    #[error("update transition is not valid in the current phase")]
    Transition,
    #[error("update target is not compatible with this installation")]
    Incompatible,
    #[error("update filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid update metadata encoding: {0}")]
    Json(#[from] serde_json::Error),
}
