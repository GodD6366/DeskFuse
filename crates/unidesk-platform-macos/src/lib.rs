//! Platform boundary for macOS permissions, pasteboard, display geometry, and
//! global input integration.
//!
//! This crate deliberately has no UI framework dependency. It contains only
//! the data types and narrow traits that the application layer needs in order
//! to talk to the platform. Native operations fail explicitly when a macOS
//! TCC grant is missing and never claim that a no-op reached the OS.

mod wake;
pub use wake::MacOSWakeObserver;

mod display;
mod error;
mod finder_paste;
mod finder_script;
mod input;
mod input_native;
mod paste_shortcut;
mod pasteboard;
mod permissions;
pub mod sharing;
mod status_item;

pub use display::{MainDisplayGeometry, display_topology, main_display_geometry};
pub use error::{PlatformError, PlatformResult};
pub use finder_paste::{
    DeliveryResult, FinderDeliveryItem, FinderDestination, capture_finder_destination,
    deliver_finder_files, is_finder_pid,
};
pub use input::{
    ButtonState, GlobalInputCapture, InputEvent, InputInjector, KeyCode, MemoryGlobalInputCapture,
    MemoryInputInjector, MouseButton, NoopGlobalInputCapture, NoopInputInjector,
};
pub use input_native::{
    MacOSGlobalInputCapture, MacOSInputInjector, SYNTHETIC_PASTE_EVENT_MARKER, post_paste_shortcut,
};
pub use paste_shortcut::{
    MacOSPasteShortcutMonitor, WorkflowHotkey, WorkflowHotkeyEvent, WorkflowHotkeys,
    workflow_frontmost_pid, workflow_modifiers_released, workflow_paste_if_current,
};
pub use pasteboard::{
    MacOSPasteboard, MemoryPasteboard, Pasteboard, PasteboardItem, PasteboardRepresentation,
    PasteboardSnapshot, REMOTE_ORIGIN_MARKER_TYPE,
};
pub use permissions::{
    MacOSPermissionService, MemoryPermissionService, PermissionKind, PermissionService,
    PermissionSnapshot, PermissionStatus, PlatformCapabilities, StubPermissionService,
    create_permission_service, request_permission, system_settings_url,
};
pub use status_item::{
    MemoryStatusItemController, NoopStatusItemController, StatusItemController, StatusItemState,
};

pub mod wifi_environment;

#[cfg(target_os = "macos")]
pub mod image_processor;

/// Foreground application observed at clipboard capture; absence stays unknown.
pub fn clipboard_source_application() -> (Option<String>, Option<String>) {
    let app = objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication();
    (
        app.as_ref()
            .and_then(|a| a.bundleIdentifier())
            .map(|s| s.to_string()),
        app.as_ref()
            .and_then(|a| a.localizedName())
            .map(|s| s.to_string()),
    )
}

mod file_paste_capture;
pub use file_paste_capture::{CapturedFilePaste, FilePasteCapture, FilePasteReference};
