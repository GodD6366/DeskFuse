//! Windows native adapters. Native operations are exported on Windows only;
//! importing this crate on another platform never manufactures successful I/O.
#[cfg(windows)]
pub mod capture;
#[cfg(windows)]
mod clipboard;
mod clipboard_formats;
#[cfg(windows)]
mod display;
#[cfg(any(windows, test))]
mod display_edid;
pub mod hid;
#[cfg(windows)]
pub mod input;
#[cfg(windows)]
mod input_environment;
#[cfg(windows)]
mod raw_mouse;
#[cfg(windows)]
pub use input_environment::WindowsInputEnvironment;
#[cfg(windows)]
pub mod session;
#[cfg(windows)]
pub mod sharing;

#[cfg(windows)]
pub use clipboard::WindowsPasteboard;
pub use deskfuse_platform::{MainDisplayGeometry, PlatformError, PlatformResult};
#[cfg(windows)]
pub use display::{NativeDisplay, display_topology, main_display_geometry, native_displays};
#[cfg(windows)]
pub use sharing::{
    WindowsSharingCapture, WindowsSharingInjector, create_permission_service, session_is_active,
    warp_pointer,
};

#[cfg(windows)]
pub mod filesystem;
