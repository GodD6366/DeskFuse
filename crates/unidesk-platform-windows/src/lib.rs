//! Windows native adapters. Native operations are exported on Windows only;
//! importing this crate on another platform never manufactures successful I/O.
#[cfg(windows)]
pub mod capture;
#[cfg(windows)]
mod clipboard;
mod clipboard_formats;
#[cfg(windows)]
mod display;
pub mod hid;
#[cfg(windows)]
pub mod input;
#[cfg(windows)]
pub mod session;

#[cfg(windows)]
pub use clipboard::WindowsPasteboard;
#[cfg(windows)]
pub use display::{NativeDisplay, display_topology, main_display_geometry, native_displays};
pub use unidesk_platform::{MainDisplayGeometry, PlatformError, PlatformResult};

#[cfg(windows)]
pub mod filesystem;
