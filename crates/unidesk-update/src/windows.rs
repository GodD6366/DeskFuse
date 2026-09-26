//! Windows helper handoff and transaction policy.
//!
//! The privileged helper implements machine admission, SCM/process quiescence,
//! database backup/restore and startup evidence in `installer_windows`. This
//! policy module still does not launch installers or grant capability authority:
//! the published NSIS package has no signed transactional activation contract,
//! so Windows must not advertise LAN_UPDATES and SERVICE_RUNNING is not commit
//! evidence.
use crate::{
    ActiveWork, ArtifactDescriptor, FailureReason, Installation, PackageKind, ReleaseVersion,
    SELF_TEST_TIMEOUT_MS, SignedReleaseManifest, Target, UpdateError, UpdatePhase, UpdateStatus,
    VerifiedRelease, pinned_release_key, select_release, verify_artifact,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

/// The handoff deliberately contains no executable, destination or database
/// paths. The trusted helper must derive all paths from its installed policy.
/// This record is untrusted until `verify` succeeds in the helper process.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsInstallRequest {
    pub transaction_id: Uuid,
    pub signed_release: SignedReleaseManifest,
}

impl WindowsInstallRequest {
    /// Verify using the application's pinned release key, never a key provided
    /// by the desktop, network, or journal. `payload` must already be copied
    /// into ACL-protected machine staging, with reparse points excluded and a
    /// no-write/no-delete handle retained through execution by the native helper.
    ///
    /// # Errors
    /// Rejects invalid signatures, incompatible targets, downgrade, quarantine,
    /// protocol/OS mismatch and altered package bytes.
    pub fn verify(
        &self,
        installed: &Installation<'_>,
        payload: &Path,
    ) -> Result<WindowsTransaction, UpdateError> {
        let release = self.signed_release.verify(&pinned_release_key()?)?;
        self.verify_release(&release, installed, payload)
    }

    fn verify_release(
        &self,
        release: &VerifiedRelease,
        installed: &Installation<'_>,
        payload: &Path,
    ) -> Result<WindowsTransaction, UpdateError> {
        if self.transaction_id.is_nil() || installed.target != Target::WindowsX64 {
            return Err(UpdateError::Incompatible);
        }
        let offers = [release.clone()];
        let (_, descriptor) =
            select_release(installed, &offers)?.ok_or(UpdateError::Incompatible)?;
        if descriptor.kind != PackageKind::WindowsNsisExe {
            return Err(UpdateError::Incompatible);
        }
        self.verified_transaction(release, payload)
    }

    /// Recovery does not apply upgrade selection: the new version may already
    /// be installed, or the failed release may already have been quarantined.
    /// The checkpoint must come from protected helper storage, not client IPC.
    /// This only determines recovery policy; it never performs a restore.
    /// # Errors
    /// Rejects altered signatures, package bytes or checkpoint bindings.
    pub fn recovery(
        &self,
        saved: &WindowsCheckpoint,
        payload: &Path,
    ) -> Result<WindowsRecovery, UpdateError> {
        let release = self.signed_release.verify(&pinned_release_key()?)?;
        self.verified_transaction(&release, payload)?
            .recovery(saved)
    }

    fn verified_transaction(
        &self,
        release: &VerifiedRelease,
        payload: &Path,
    ) -> Result<WindowsTransaction, UpdateError> {
        if self.transaction_id.is_nil() {
            return Err(UpdateError::Invalid("nil Windows transaction"));
        }
        let artifact = verify_artifact(release, Target::WindowsX64, payload)?;
        Ok(WindowsTransaction {
            status: UpdateStatus {
                transaction_id: self.transaction_id,
                release: artifact.release().clone(),
                artifact: artifact.descriptor().clone(),
                phase: UpdatePhase::WaitingForIdle,
                received_bytes: artifact.descriptor().size,
                failure: None,
            },
            started_ms: None,
            backup_complete: false,
        })
    }
}

/// Observations made under the helper's exclusive admission lock. A GUI idle
/// snapshot alone is insufficient: all sessions must stop writing, including
/// service database writers and input helpers, before backup begins.
#[derive(Clone, Copy, Debug, Default)]
pub struct WindowsAdmission {
    pub work: ActiveWork,
    pub business_admission_closed: bool,
    pub service_stopped: bool,
    pub all_desktops_stopped: bool,
    pub all_input_helpers_stopped: bool,
}
impl WindowsAdmission {
    fn permits_backup(self) -> bool {
        self.work.is_idle()
            && self.business_admission_closed
            && self.service_stopped
            && self.all_desktops_stopped
            && self.all_input_helpers_stopped
    }
}

/// Native helper's local startup evidence, delivered over an authenticated,
/// transaction-bound channel before business writes are admitted. Network
/// reachability deliberately is not part of this contract.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsSelfTest {
    pub transaction_id: Uuid,
    pub release: ReleaseVersion,
    pub artifact: ArtifactDescriptor,
    pub service_ready: bool,
    pub desktop_ready: bool,
    pub database_ready: bool,
    pub input_helper_ready: bool,
    pub business_admission_closed: bool,
}

/// Serialized helper checkpoint. Callers must persist this in machine-owned
/// storage before the associated destructive operation. This is not executable
/// authority; recovery must verify the request again and restore backups before
/// starting either old or new business processes.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsCheckpoint {
    pub status: UpdateStatus,
    pub backup_complete: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowsRecovery {
    /// No destructive operation was authorized; keep the current installation.
    DiscardStaging,
    /// Restore application and database/WAL/SHM backups; never touch identity,
    /// trust, user source files, or unrelated service configuration.
    RestoreBackup,
    /// Local success is permanent even if the sender is offline.
    KeepCommitted,
}

/// In-memory policy. Not deserializable into a runnable transaction, and never
/// authorizes execution directly from a journal phase.
#[derive(Debug)]
pub struct WindowsTransaction {
    status: UpdateStatus,
    started_ms: Option<u64>,
    backup_complete: bool,
}
impl WindowsTransaction {
    #[must_use]
    pub fn status(&self) -> &UpdateStatus {
        &self.status
    }

    #[must_use]
    pub fn checkpoint(&self) -> WindowsCheckpoint {
        WindowsCheckpoint {
            status: self.status.clone(),
            backup_complete: self.backup_complete,
        }
    }

    /// The caller takes and flushes program and all migrating database backups
    /// after this transition, while retaining the admission lock.
    /// # Errors
    /// Rejects invalid phases or incomplete quiescence.
    pub fn begin_backup(&mut self, admission: WindowsAdmission) -> Result<(), UpdateError> {
        self.require(UpdatePhase::WaitingForIdle)?;
        if !admission.permits_backup() {
            return Err(UpdateError::Transition);
        }
        self.status.phase = UpdatePhase::Staged;
        Ok(())
    }

    /// Invoke only after backups are complete and flushed. Persist the resulting
    /// checkpoint BEFORE replacement. Partial backups never authorize deletion.
    /// # Errors
    /// Rejects an invalid phase.
    pub fn backup_completed(&mut self) -> Result<(), UpdateError> {
        self.require(UpdatePhase::Staged)?;
        self.backup_complete = true;
        self.status.phase = UpdatePhase::Installing;
        Ok(())
    }

    /// A successful installer exit starts local self-test, not normal operation.
    /// The external helper owns the monotonic clock and the 60 second deadline.
    /// # Errors
    /// Rejects an invalid phase.
    pub fn installed(&mut self, now_ms: u64) -> Result<(), UpdateError> {
        self.require(UpdatePhase::Installing)?;
        self.status.phase = UpdatePhase::AwaitingSelfTest;
        self.started_ms = Some(now_ms);
        Ok(())
    }

    /// # Errors
    /// Rejects mismatched evidence, timeout, incomplete startup, or clock reversal.
    pub fn acknowledge(
        &mut self,
        evidence: &WindowsSelfTest,
        now_ms: u64,
    ) -> Result<(), UpdateError> {
        self.require(UpdatePhase::AwaitingSelfTest)?;
        if self.poll_timeout(now_ms)? {
            return Err(UpdateError::Transition);
        }
        if evidence.transaction_id != self.status.transaction_id
            || evidence.release != self.status.release
            || evidence.artifact != self.status.artifact
        {
            return Err(UpdateError::ArtifactMismatch);
        }
        if !(evidence.service_ready
            && evidence.desktop_ready
            && evidence.database_ready
            && evidence.input_helper_ready
            && evidence.business_admission_closed)
        {
            self.fail(FailureReason::SelfTest)?;
            return Err(UpdateError::Transition);
        }
        // Persist this checkpoint before opening business admission. Afterwards
        // a missing remote receipt must never restore an older database.
        self.status.phase = UpdatePhase::AwaitingReconnect;
        Ok(())
    }

    /// # Errors
    /// Rejects clock reversal or invalid self-test state.
    pub fn poll_timeout(&mut self, now_ms: u64) -> Result<bool, UpdateError> {
        self.require(UpdatePhase::AwaitingSelfTest)?;
        let elapsed = now_ms
            .checked_sub(self.started_ms.ok_or(UpdateError::Transition)?)
            .ok_or(UpdateError::Invalid("monotonic clock moved backwards"))?;
        if elapsed >= SELF_TEST_TIMEOUT_MS {
            self.fail(FailureReason::SelfTest)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// # Errors
    /// Committed and terminal transactions cannot fail or roll back.
    pub fn fail(&mut self, reason: FailureReason) -> Result<(), UpdateError> {
        self.status.phase = match self.status.phase {
            UpdatePhase::WaitingForIdle | UpdatePhase::Staged => terminal_failure(reason),
            UpdatePhase::Installing | UpdatePhase::AwaitingSelfTest => UpdatePhase::RollingBack,
            _ => return Err(UpdateError::Transition),
        };
        self.status.failure = Some(reason);
        Ok(())
    }

    /// Call only after both program AND database restore have succeeded. A failed
    /// restore leaves RollingBack and the backup available for startup recovery.
    /// # Errors
    /// Rejects an invalid phase or missing failure reason.
    pub fn rollback_completed(&mut self) -> Result<(), UpdateError> {
        self.require(UpdatePhase::RollingBack)?;
        self.status.phase = terminal_failure(self.status.failure.ok_or(UpdateError::Transition)?);
        Ok(())
    }

    /// Compare an untrusted persisted checkpoint with freshly verified evidence.
    /// Never resume an interrupted installer; restore the known-good snapshot.
    /// # Errors
    /// Rejects forged transaction/release/artifact fields and impossible phases.
    fn recovery(&self, saved: &WindowsCheckpoint) -> Result<WindowsRecovery, UpdateError> {
        if saved.status.transaction_id != self.status.transaction_id
            || saved.status.release != self.status.release
            || saved.status.artifact != self.status.artifact
            || saved.status.received_bytes != self.status.received_bytes
        {
            return Err(UpdateError::ArtifactMismatch);
        }
        match (saved.status.phase, saved.backup_complete) {
            (UpdatePhase::WaitingForIdle | UpdatePhase::Staged, false) => {
                Ok(WindowsRecovery::DiscardStaging)
            }
            (
                UpdatePhase::Installing | UpdatePhase::AwaitingSelfTest | UpdatePhase::RollingBack,
                true,
            ) => Ok(WindowsRecovery::RestoreBackup),
            (UpdatePhase::AwaitingReconnect | UpdatePhase::Complete, true) => {
                Ok(WindowsRecovery::KeepCommitted)
            }
            (UpdatePhase::Cancelled | UpdatePhase::Quarantined, _) => {
                Ok(WindowsRecovery::DiscardStaging)
            }
            _ => Err(UpdateError::Transition),
        }
    }

    fn require(&self, phase: UpdatePhase) -> Result<(), UpdateError> {
        if self.status.phase == phase {
            Ok(())
        } else {
            Err(UpdateError::Transition)
        }
    }
}

fn terminal_failure(reason: FailureReason) -> UpdatePhase {
    match reason {
        FailureReason::Verification | FailureReason::SelfTest => UpdatePhase::Quarantined,
        // Interrupted install, disk full, permissions or NSIS errors are not
        // proof that the publisher's bytes are faulty. Keep the release eligible.
        _ => UpdatePhase::Cancelled,
    }
}
