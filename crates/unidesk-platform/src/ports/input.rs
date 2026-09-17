//! Native input execution. Collaboration owns leases; the host posts events.
use crate::PlatformResult;
use async_trait::async_trait;
use unidesk_core::SharedInputEvent;

/// Execute a single normalized input event against the local session.
#[async_trait]
pub trait InputExecutionPort: Send + Sync {
    async fn execute(&self, event: SharedInputEvent) -> PlatformResult<()>;

    /// Release every held key and mouse button. Called on lease expiry and faults.
    async fn release_all(&self) -> PlatformResult<()>;
}

/// Records executed events for tests. Never posts to the OS.
#[derive(Debug, Default)]
pub struct MemoryInputExecution {
    events: std::sync::Mutex<Vec<SharedInputEvent>>,
    released: std::sync::atomic::AtomicU64,
}

impl MemoryInputExecution {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn executed(&self) -> Vec<SharedInputEvent> {
        self.events
            .lock()
            .map(|events| events.clone())
            .unwrap_or_default()
    }

    pub fn release_count(&self) -> u64 {
        self.released.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl InputExecutionPort for MemoryInputExecution {
    async fn execute(&self, event: SharedInputEvent) -> PlatformResult<()> {
        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
        Ok(())
    }

    async fn release_all(&self) -> PlatformResult<()> {
        self.released
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut events) = self.events.lock() {
            events.clear();
        }
        Ok(())
    }
}
