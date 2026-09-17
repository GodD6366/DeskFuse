//! Permission observation. Hosts report native grants; collaboration never fabricates them.
use crate::{PermissionService, PermissionSnapshot, PlatformResult};
use async_trait::async_trait;

/// Observe native permission state. A granted snapshot is not an availability result.
#[async_trait]
pub trait PermissionObserverPort: Send + Sync {
    async fn snapshot(&self) -> PlatformResult<PermissionSnapshot>;
}

/// Fixed snapshot used by tests.
#[derive(Debug, Clone)]
pub struct MemoryPermissionObserver {
    snapshot: PermissionSnapshot,
}

impl MemoryPermissionObserver {
    #[must_use]
    pub const fn new(snapshot: PermissionSnapshot) -> Self {
        Self { snapshot }
    }

    #[must_use]
    pub const fn all_unknown() -> Self {
        Self {
            snapshot: PermissionSnapshot::all_unknown(),
        }
    }
}

#[async_trait]
impl PermissionObserverPort for MemoryPermissionObserver {
    async fn snapshot(&self) -> PlatformResult<PermissionSnapshot> {
        Ok(self.snapshot)
    }
}

/// Observe permissions through the existing synchronous [`PermissionService`].
pub struct PermissionServicePort {
    inner: Box<dyn PermissionService>,
}

impl PermissionServicePort {
    #[must_use]
    pub fn new(inner: Box<dyn PermissionService>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl PermissionObserverPort for PermissionServicePort {
    async fn snapshot(&self) -> PlatformResult<PermissionSnapshot> {
        Ok(self.inner.snapshot())
    }
}
