//! Platform execution ports for the updater.
//!
//! Manifest parsing, signature verification, release selection, and transaction
//! policy stay in this crate.  These traits are the only boundary the policy
//! layer needs from an operating-system installer; implementations own paths,
//! process control, replacement, and recovery journal I/O.

use crate::{ArtifactDescriptor, UpdateError, UpdateStatus, VerifiedRelease};
use std::path::Path;

/// A platform installer stages an already verified artifact without changing
/// the active installation.  The implementation must keep staging private and
/// must fail closed when the artifact is not a supported transactional payload.
pub trait PlatformInstaller {
    /// Stage a payload for a transaction.  This method does not authorize the
    /// destructive replacement step.
    ///
    /// # Errors
    /// Returns a platform or storage error when staging cannot be completed.
    fn stage(
        &self,
        release: &VerifiedRelease,
        payload: &Path,
        status: &UpdateStatus,
    ) -> Result<(), String>;
}

/// The destructive part of updating is deliberately outside the common policy
/// state machine.  Implementations own elevation, process/service admission,
/// backup/replace, startup self-test evidence, and crash recovery.
pub trait PlatformUpdateExecutor {
    /// The concrete handle is intentionally platform-owned (for example, a
    /// child process on macOS or an elevated handoff on Windows).
    type Handle;

    /// Start execution only after the common transaction has reached `Staged`
    /// and the caller has closed business admission.
    ///
    /// # Errors
    /// Returns an error without weakening the fail-closed update policy.
    fn execute(&self, status: &UpdateStatus) -> Result<Self::Handle, String>;

    /// Recover an interrupted transaction.  Recovery must never resume an
    /// untrusted installer and must preserve the last known-good installation.
    ///
    /// # Errors
    /// Returns an error when the journal or recovery evidence is incomplete.
    fn recover(&self) -> Result<(), String>;

    /// Return platform self-test evidence for a transaction that has restarted.
    ///
    /// # Errors
    /// Returns an error when evidence is absent or does not bind to the
    /// transaction.
    fn self_test_status(&self, transaction_id: &str) -> Result<UpdateStatus, String>;

    /// Persist acknowledgement only after the application has validated its
    /// own startup state.
    ///
    /// # Errors
    /// Returns a storage or transaction-binding error.
    fn acknowledge_self_test(&self, status: &UpdateStatus) -> Result<(), String>;
}

/// A capability is usable only when all three independent proofs are present.
/// Platform implementations must derive these values from real admission and
/// self-test observations; callers cannot construct success from a signature
/// alone.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UpdateCapability {
    pub signed_manifest: bool,
    pub admission_closed: bool,
    pub self_test_passed: bool,
}
impl UpdateCapability {
    #[must_use]
    pub const fn enabled(self) -> bool {
        self.signed_manifest && self.admission_closed && self.self_test_passed
    }

    /// Convert an incomplete proof into the existing fail-closed error model.
    ///
    /// # Errors
    /// Returns `UpdateError::Incompatible` until every proof is present.
    pub fn require(self) -> Result<(), UpdateError> {
        if self.enabled() {
            Ok(())
        } else {
            Err(UpdateError::Incompatible)
        }
    }
}

/// A common receipt binding used by platform adapters when they report local
/// success.  It is intentionally data-only; it does not authorize reopening
/// business admission by itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateExecutionReceipt {
    pub status: UpdateStatus,
    pub artifact: ArtifactDescriptor,
    pub capability: UpdateCapability,
}
