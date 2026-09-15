use std::fmt;

/// Errors returned by the platform boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformError {
    /// The requested operation does not exist on the current target, or is
    /// intentionally not implemented by this boundary yet.
    Unsupported { operation: &'static str },
    /// Opening a system settings URL failed.
    SystemSettings {
        operation: &'static str,
        message: String,
    },
    /// Input injection is intentionally disabled.  A caller must not mistake
    /// a no-op/test implementation for a real input injector.
    InputInjectionDisabled,
    /// An in-memory test provider could not update its state.
    StatePoisoned { provider: &'static str },
    /// Reading the pasteboard failed or returned unusable content.
    PasteboardReadFailed,
    /// File content is advertised, but no usable local file reference exists.
    PasteboardFileUnavailable,
    /// Writing to the pasteboard failed.
    PasteboardWriteFailed,
    /// Main-display geometry could not be read.
    DisplayUnavailable,
    /// A global input capture tap could not be installed or stopped.
    InputCaptureUnavailable,
    /// macOS TCC Accessibility permission is required.
    InputPermissionRequired,
    /// A native input event could not be created or posted.
    InputInjectionFailed,
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { operation } => {
                write!(f, "platform operation is unsupported: {operation}")
            }
            Self::SystemSettings { operation, message } => {
                write!(
                    f,
                    "failed to open system settings for {operation}: {message}"
                )
            }
            Self::InputInjectionDisabled => {
                f.write_str("input injection is disabled by the platform boundary")
            }
            Self::StatePoisoned { provider } => {
                write!(f, "in-memory provider state is poisoned: {provider}")
            }
            Self::PasteboardReadFailed => f.write_str("could not read the system clipboard"),
            Self::PasteboardFileUnavailable => f.write_str(
                "剪贴板中的文件尚未就绪，请先将文件保存或下载到本机，再从文件管理器复制。",
            ),
            Self::PasteboardWriteFailed => f.write_str("could not write to the system clipboard"),
            Self::DisplayUnavailable => f.write_str("main-display geometry is unavailable"),
            Self::InputCaptureUnavailable => {
                f.write_str("could not install a global input capture tap")
            }
            Self::InputPermissionRequired => {
                f.write_str("macOS input permission is required (辅助功能)")
            }
            Self::InputInjectionFailed => {
                f.write_str("could not create or post a native input event")
            }
        }
    }
}

impl std::error::Error for PlatformError {}

/// Result type used by all platform boundary traits.
pub type PlatformResult<T> = Result<T, PlatformError>;
