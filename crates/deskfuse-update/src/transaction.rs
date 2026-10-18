use crate::{
    ArtifactDescriptor, ReleaseVersion, SELF_TEST_TIMEOUT_MS, Target, UpdateError,
    VerifiedArtifact, VerifiedRelease,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    Downloading,
    Verifying,
    WaitingForIdle,
    Staged,
    Installing,
    AwaitingSelfTest,
    AwaitingReconnect,
    Complete,
    RollingBack,
    Quarantined,
    Cancelled,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateStatus {
    pub transaction_id: Uuid,
    pub release: ReleaseVersion,
    pub artifact: ArtifactDescriptor,
    pub phase: UpdatePhase,
    pub received_bytes: u64,
    /// Metadata-only fixed reason code, not raw command output or user content.
    pub failure: Option<FailureReason>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    Verification,
    Storage,
    Permission,
    Installer,
    SelfTest,
    Interrupted,
}

// These activities can overlap; they are independent observations, not states.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ActiveWork {
    pub receiving: bool,
    pub pasting: bool,
    pub exporting: bool,
    pub controlling: bool,
}
impl ActiveWork {
    #[must_use]
    pub fn is_idle(self) -> bool {
        !(self.receiving || self.pasting || self.exporting || self.controlling)
    }
}

/// Pure policy, not an installer. No public setter and no deserialization into a
/// runnable transaction. Journal recovery must reverify the persisted release.
#[derive(Debug)]
pub struct UpdateTransaction {
    status: UpdateStatus,
    self_test_started_ms: Option<u64>,
}
impl UpdateTransaction {
    ///
    /// # Errors
    /// Rejects invalid phases, mismatched evidence, invalid ranges or non-monotonic time.
    pub fn new(release: &VerifiedRelease, target: Target) -> Result<Self, UpdateError> {
        let artifact = release
            .artifact(target)
            .ok_or(UpdateError::Incompatible)?
            .clone();
        Ok(Self {
            status: UpdateStatus {
                transaction_id: Uuid::new_v4(),
                release: release.manifest().release.clone(),
                artifact,
                phase: UpdatePhase::Downloading,
                received_bytes: 0,
                failure: None,
            },
            self_test_started_ms: None,
        })
    }
    /// GitHub LAN transfers have a SHA-256 descriptor, not a LAN-signed manifest.
    ///
    /// # Errors
    /// Rejects an invalid artifact descriptor.
    pub fn for_artifact(
        release: &ReleaseVersion,
        artifact: ArtifactDescriptor,
    ) -> Result<Self, UpdateError> {
        artifact.validate()?;
        Ok(Self {
            status: UpdateStatus {
                transaction_id: Uuid::new_v4(),
                release: release.clone(),
                artifact,
                phase: UpdatePhase::Downloading,
                received_bytes: 0,
                failure: None,
            },
            self_test_started_ms: None,
        })
    }
    #[must_use]
    pub fn status(&self) -> &UpdateStatus {
        &self.status
    }
    fn require(&self, phase: UpdatePhase) -> Result<(), UpdateError> {
        if self.status.phase == phase {
            Ok(())
        } else {
            Err(UpdateError::Transition)
        }
    }
    /// Strictly sequential; restarted source downloads create a new transaction.
    ///
    /// # Errors
    /// Rejects invalid phases, mismatched evidence, invalid ranges or non-monotonic time.
    pub fn received(&mut self, offset: u64, length: u64) -> Result<(), UpdateError> {
        self.require(UpdatePhase::Downloading)?;
        let end = offset
            .checked_add(length)
            .ok_or(UpdateError::Invalid("download overflow"))?;
        if offset != self.status.received_bytes || length == 0 || end > self.status.artifact.size {
            return Err(UpdateError::Invalid("download range"));
        }
        self.status.received_bytes = end;
        if end == self.status.artifact.size {
            self.status.phase = UpdatePhase::Verifying;
        }
        Ok(())
    }
    ///
    /// # Errors
    /// Rejects invalid phases, mismatched evidence, invalid ranges or non-monotonic time.
    pub fn verified(&mut self, artifact: &VerifiedArtifact) -> Result<(), UpdateError> {
        self.require(UpdatePhase::Verifying)?;
        if artifact.release != self.status.release || artifact.descriptor != self.status.artifact {
            return Err(UpdateError::ArtifactMismatch);
        }
        self.status.phase = UpdatePhase::WaitingForIdle;
        Ok(())
    }
    /// True means staging may begin. Installer must also hold its admission lock.
    ///
    /// # Errors
    /// Rejects invalid phases, mismatched evidence, invalid ranges or non-monotonic time.
    pub fn stage(&mut self, work: ActiveWork) -> Result<bool, UpdateError> {
        self.require(UpdatePhase::WaitingForIdle)?;
        if !work.is_idle() {
            return Ok(false);
        }
        self.status.phase = UpdatePhase::Staged;
        Ok(true)
    }
    /// Recheck work after staging; never silently abandon a newly started operation.
    ///
    /// # Errors
    /// Rejects invalid phases, mismatched evidence, invalid ranges or non-monotonic time.
    pub fn begin_install(&mut self, work: ActiveWork) -> Result<bool, UpdateError> {
        self.require(UpdatePhase::Staged)?;
        if !work.is_idle() {
            self.status.phase = UpdatePhase::WaitingForIdle;
            return Ok(false);
        }
        self.status.phase = UpdatePhase::Installing;
        Ok(true)
    }
    /// Supply an external installer's monotonic elapsed milliseconds, not wall time.
    ///
    /// # Errors
    /// Rejects invalid phases, mismatched evidence, invalid ranges or non-monotonic time.
    pub fn process_started(&mut self, now_ms: u64) -> Result<(), UpdateError> {
        self.require(UpdatePhase::Installing)?;
        self.self_test_started_ms = Some(now_ms);
        self.status.phase = UpdatePhase::AwaitingSelfTest;
        Ok(())
    }
    ///
    /// # Errors
    /// Rejects invalid phases, mismatched evidence, invalid ranges or non-monotonic time.
    pub fn self_test_ok(
        &mut self,
        transaction_id: Uuid,
        release: &ReleaseVersion,
        artifact_sha256: &str,
        now_ms: u64,
    ) -> Result<(), UpdateError> {
        self.require(UpdatePhase::AwaitingSelfTest)?;
        if self.timed_out(now_ms)? {
            return Err(UpdateError::Transition);
        }
        if transaction_id != self.status.transaction_id
            || release != &self.status.release
            || artifact_sha256 != self.status.artifact.sha256
        {
            return Err(UpdateError::ArtifactMismatch);
        }
        self.status.phase = UpdatePhase::AwaitingReconnect;
        Ok(())
    }
    ///
    /// # Errors
    /// Rejects invalid phases, mismatched evidence, invalid ranges or non-monotonic time.
    pub fn timed_out(&mut self, now_ms: u64) -> Result<bool, UpdateError> {
        if self.status.phase != UpdatePhase::AwaitingSelfTest {
            return Ok(false);
        }
        let started = self.self_test_started_ms.ok_or(UpdateError::Transition)?;
        let elapsed = now_ms
            .checked_sub(started)
            .ok_or(UpdateError::Invalid("monotonic clock moved backwards"))?;
        if elapsed >= SELF_TEST_TIMEOUT_MS {
            self.fail(FailureReason::SelfTest)?;
            return Ok(true);
        }
        Ok(false)
    }
    ///
    /// # Errors
    /// Rejects invalid phases, mismatched evidence, invalid ranges or non-monotonic time.
    pub fn confirm_reconnect(
        &mut self,
        release: &ReleaseVersion,
        artifact: &ArtifactDescriptor,
    ) -> Result<(), UpdateError> {
        self.require(UpdatePhase::AwaitingReconnect)?;
        if release != &self.status.release || artifact != &self.status.artifact {
            return Err(UpdateError::ArtifactMismatch);
        }
        self.status.phase = UpdatePhase::Complete;
        Ok(())
    }
    ///
    /// # Errors
    /// Rejects invalid phases, mismatched evidence, invalid ranges or non-monotonic time.
    pub fn fail(&mut self, reason: FailureReason) -> Result<(), UpdateError> {
        self.status.phase = match self.status.phase {
            UpdatePhase::Downloading
            | UpdatePhase::Verifying
            | UpdatePhase::WaitingForIdle
            | UpdatePhase::Staged => UpdatePhase::Quarantined,
            UpdatePhase::Installing | UpdatePhase::AwaitingSelfTest => UpdatePhase::RollingBack,
            // Local success is permanent even if the peer never reconnects.
            _ => return Err(UpdateError::Transition),
        };
        self.status.failure = Some(reason);
        Ok(())
    }
    ///
    /// # Errors
    /// Rejects invalid phases, mismatched evidence, invalid ranges or non-monotonic time.
    pub fn rollback_complete(&mut self) -> Result<(), UpdateError> {
        self.require(UpdatePhase::RollingBack)?;
        self.status.phase = UpdatePhase::Quarantined;
        Ok(())
    }
    ///
    /// # Errors
    /// Rejects invalid phases, mismatched evidence, invalid ranges or non-monotonic time.
    pub fn cancel(&mut self) -> Result<(), UpdateError> {
        if !matches!(
            self.status.phase,
            UpdatePhase::Downloading
                | UpdatePhase::Verifying
                | UpdatePhase::WaitingForIdle
                | UpdatePhase::Staged
        ) {
            return Err(UpdateError::Transition);
        }
        self.status.phase = UpdatePhase::Cancelled;
        Ok(())
    }
}
