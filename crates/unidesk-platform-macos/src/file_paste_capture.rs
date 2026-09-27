//! macOS event-tap data is represented by the platform-neutral file-reference port.
//!
//! The event tap itself remains in this crate; only its queued reference types
//! are shared with the engine.
pub use unidesk_platform::ports::{CapturedFilePaste, FilePasteCapture, FilePasteReference};
