//! Host Ports: platform-neutral contracts injected by the application host.
//!
//! This module has no OS, AppKit, Win32, or GPUI dependency. Concrete adapters
//! live in the macOS/Windows host, never in the collaboration core.
mod clipboard;
mod credential;
mod file;
mod input;
mod pasteboard_adapter;
mod permission;
mod workflow;

pub use clipboard::*;
pub use credential::*;
pub use file::*;
pub use input::*;
pub use pasteboard_adapter::{PasteboardPort, apply_clipboard};
pub use permission::*;
pub use workflow::*;

use std::sync::Arc;

/// Bundle of host-injected ports used to construct a collaboration core.
#[derive(Clone)]
pub struct HostPorts {
    pub clipboard_capture: Arc<dyn ClipboardCapturePort>,
    pub clipboard_apply: Arc<dyn ClipboardApplyPort>,
    pub file_authorization: Arc<dyn FileAuthorizationPort>,
    pub input_execution: Arc<dyn InputExecutionPort>,
    pub credential_store: Arc<dyn CredentialStorePort>,
    pub permission_observer: Arc<dyn PermissionObserverPort>,
}

impl HostPorts {
    pub fn memory(data_dir: impl Into<std::path::PathBuf>) -> Self {
        let data_dir = data_dir.into();
        let clipboard = Arc::new(MemoryClipboard::new());
        Self {
            clipboard_capture: clipboard.clone(),
            clipboard_apply: clipboard,
            file_authorization: Arc::new(MemoryFileAuthorization::new(data_dir.join("inbox"))),
            input_execution: Arc::new(MemoryInputExecution::new()),
            credential_store: Arc::new(MemoryCredentialStore::new(data_dir.join("identity.key"))),
            permission_observer: Arc::new(MemoryPermissionObserver::all_unknown()),
        }
    }

    pub fn input_only(data_dir: impl Into<std::path::PathBuf>) -> Self {
        Self::memory(data_dir)
    }
}
