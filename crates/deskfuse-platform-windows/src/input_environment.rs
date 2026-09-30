//! Ordinary Default-desktop admission, independent of macOS TCC vocabulary.
use deskfuse_platform::{
    DesktopState, InputBlockerKind, InputCapability, InputEnvironmentPort,
    InputEnvironmentSnapshot,
};

pub struct WindowsInputEnvironment;
impl InputEnvironmentPort for WindowsInputEnvironment {
    fn snapshot(&self) -> InputEnvironmentSnapshot {
        let session = crate::session::session_capabilities();
        if session.desktop != DesktopState::UserDesktop || !session.user_content_allowed {
            return InputEnvironmentSnapshot::blocked(
                session.desktop,
                InputBlockerKind::InactiveDesktop,
                format!("Windows input desktop unavailable: {:?} (native error: {:?})",
                    session.blockers, session.native_error),
            );
        }
        // This only permits trying ordinary capture/SendInput. It does not
        // claim UIAccess, elevation, or remote_input_allowed. The service must
        // independently authenticate the helper's actual token, SID, logon,
        // process lifetime and session before granting a network input lease.
        InputEnvironmentSnapshot {
            desktop: session.desktop,
            capture: InputCapability::Allowed,
            inject: InputCapability::Allowed,
        }
    }
}
