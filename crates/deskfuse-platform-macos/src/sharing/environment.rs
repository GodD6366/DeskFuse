//! Input helper admission. TCC and console-session interpretation stay native.
use deskfuse_platform::{
    DesktopState, InputBlockerKind, InputCapability, InputEnvironmentPort, InputEnvironmentSnapshot,
};

pub struct MacOSInputEnvironment;
impl InputEnvironmentPort for MacOSInputEnvironment {
    fn snapshot(&self) -> InputEnvironmentSnapshot {
        let active = super::session_is_active();
        let desktop = if active {
            DesktopState::UserDesktop
        } else {
            DesktopState::Unavailable
        };
        // The modifying Default tap needs Accessibility, not the listen-only
        // Input Monitoring grant. CGEventTapCreate remains the final authority.
        if !super::may_capture() || !super::may_post() {
            return InputEnvironmentSnapshot::blocked(
                desktop,
                InputBlockerKind::AccessibilityRequired,
                "等待辅助功能权限",
            );
        }
        if !active {
            return InputEnvironmentSnapshot::blocked(
                desktop,
                InputBlockerKind::InactiveDesktop,
                "等待解锁本机桌面",
            );
        }
        InputEnvironmentSnapshot {
            desktop,
            capture: InputCapability::Allowed,
            inject: InputCapability::Allowed,
        }
    }
}
