//! Machine-service boundary. IPC is metadata-only until desktop session routing
//! is implemented; no file, clipboard, input, shell, or install commands exist.
#[cfg(windows)]
pub mod native;
pub mod policy;
pub mod protocol;
