use std::sync::{Arc, Mutex};

use crate::{PlatformError, PlatformResult};

pub use unidesk_platform::{PermissionKind, PermissionSnapshot, PermissionStatus};

/// Permission operations exposed to the application layer.
///
/// `snapshot` is deliberately a read operation.  `open_system_settings` only
/// opens the corresponding privacy pane; it does not attempt to grant a
/// permission or to infer a grant from the existence of the pane.
pub use unidesk_platform::PermissionService;

/// URL helper shared by macOS callers and tests.
pub const fn system_settings_url(kind: PermissionKind) -> &'static str {
    kind.system_settings_url()
}

/// Open the exact privacy pane where the user can grant the permission.
///
/// UniDesk intentionally does not ask Accessibility to display its own native
/// prompt.  A single, app-owned onboarding window explains why both required
/// grants are needed and sends every action through the same System Settings
/// flow.  This also avoids stacking an opaque system alert on top of the
/// onboarding window.
pub fn request_permission(kind: PermissionKind) -> PlatformResult<()> {
    #[cfg(target_os = "macos")]
    {
        open_system_settings(kind)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = kind;
        Err(PlatformError::Unsupported {
            operation: "request_permission",
        })
    }
}

/// The capabilities this platform boundary can expose without a UI toolkit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformCapabilities {
    /// A native status item can exist on macOS.  This is a boundary capability,
    /// not a statement that a status item has already been created.
    pub status_item: bool,
    /// Global input capture is not claimed until the required permission is
    /// granted by the user.
    pub global_input_capture: bool,
    /// Input injection is intentionally not exposed by this crate yet.
    pub input_injection: bool,
    pub permissions: PermissionSnapshot,
}

impl PlatformCapabilities {
    pub const fn new(
        status_item: bool,
        global_input_capture: bool,
        input_injection: bool,
        permissions: PermissionSnapshot,
    ) -> Self {
        Self {
            status_item,
            global_input_capture,
            input_injection,
            permissions,
        }
    }

    pub const fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::new(true, false, false, PermissionSnapshot::all_unknown())
        }
        #[cfg(not(target_os = "macos"))]
        {
            Self::new(false, false, false, PermissionSnapshot::all_unknown())
        }
    }

    pub const fn supports_status_item(self) -> bool {
        self.status_item
    }

    pub const fn supports_global_input_capture(self) -> bool {
        self.global_input_capture
    }

    pub const fn supports_input_injection(self) -> bool {
        self.input_injection
    }
}

impl Default for PlatformCapabilities {
    fn default() -> Self {
        Self::current()
    }
}

/// The real macOS permission provider.
#[cfg(target_os = "macos")]
pub struct MacOSPermissionService;

#[cfg(target_os = "macos")]
impl MacOSPermissionService {
    pub const fn new() -> Self {
        Self
    }

    fn accessibility_status() -> PermissionStatus {
        if macos_accessibility_is_trusted() {
            PermissionStatus::Granted
        } else {
            // AXIsProcessTrusted cannot distinguish a denial from a not-yet-
            // answered prompt, so do not claim Denied here.
            PermissionStatus::NeedsUserAction
        }
    }

    fn full_disk_access_status() -> PermissionStatus {
        if macos_full_disk_access_is_granted() {
            PermissionStatus::Granted
        } else {
            PermissionStatus::NeedsUserAction
        }
    }
}

#[cfg(target_os = "macos")]
impl Default for MacOSPermissionService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "macos")]
impl PermissionService for MacOSPermissionService {
    fn snapshot(&self) -> PermissionSnapshot {
        PermissionSnapshot::new(
            Self::accessibility_status(),
            Self::full_disk_access_status(),
            // Production taps use Accessibility. Do not probe an unused grant.
            PermissionStatus::Unknown,
        )
    }

    fn open_system_settings(&self, kind: PermissionKind) -> PlatformResult<()> {
        open_system_settings(kind)
    }
}

/// A platform stub.  It never pretends that a permission was granted and
/// never attempts to run a system-settings command.  It is used by non-macOS
/// builds and is also available on macOS for deterministic tests.
#[derive(Debug, Default, Clone, Copy)]
pub struct StubPermissionService;

impl StubPermissionService {
    pub const fn new() -> Self {
        Self
    }
}

impl PermissionService for StubPermissionService {
    fn snapshot(&self) -> PermissionSnapshot {
        PermissionSnapshot::all_unknown()
    }

    fn open_system_settings(&self, _kind: PermissionKind) -> PlatformResult<()> {
        Err(PlatformError::Unsupported {
            operation: "open_system_settings",
        })
    }
}

/// Keep a stable provider name on non-macOS while retaining a real type on
/// macOS.  Callers can use this alias when they do not need a concrete test
/// provider.
#[cfg(not(target_os = "macos"))]
pub type MacOSPermissionService = StubPermissionService;

/// A permission provider for deterministic tests and higher-level integration.
/// It records settings-pane requests but never opens an external process.
#[derive(Debug, Clone)]
pub struct MemoryPermissionService {
    snapshot: Arc<Mutex<PermissionSnapshot>>,
    opened_settings: Arc<Mutex<Vec<PermissionKind>>>,
}

impl Default for MemoryPermissionService {
    fn default() -> Self {
        Self::new(PermissionSnapshot::all_unknown())
    }
}

impl MemoryPermissionService {
    pub fn new(snapshot: PermissionSnapshot) -> Self {
        Self {
            snapshot: Arc::new(Mutex::new(snapshot)),
            opened_settings: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn set_snapshot(&self, snapshot: PermissionSnapshot) -> PlatformResult<()> {
        let mut current = self
            .snapshot
            .lock()
            .map_err(|_| PlatformError::StatePoisoned {
                provider: "MemoryPermissionService.snapshot",
            })?;
        *current = snapshot;
        Ok(())
    }

    pub fn opened_settings(&self) -> PlatformResult<Vec<PermissionKind>> {
        let opened = self
            .opened_settings
            .lock()
            .map_err(|_| PlatformError::StatePoisoned {
                provider: "MemoryPermissionService.opened_settings",
            })?;
        Ok(opened.clone())
    }
}

impl PermissionService for MemoryPermissionService {
    fn snapshot(&self) -> PermissionSnapshot {
        // A poisoned test mutex is not expected in normal use.  Returning an
        // all-unknown snapshot is safer than claiming a permission was given.
        self.snapshot
            .lock()
            .map(|snapshot| *snapshot)
            .unwrap_or_else(|_| PermissionSnapshot::all_unknown())
    }

    fn open_system_settings(&self, kind: PermissionKind) -> PlatformResult<()> {
        let mut opened = self
            .opened_settings
            .lock()
            .map_err(|_| PlatformError::StatePoisoned {
                provider: "MemoryPermissionService.opened_settings",
            })?;
        opened.push(kind);
        Ok(())
    }
}

/// Construct the default permission provider for the current target.
pub fn create_permission_service() -> Box<dyn PermissionService> {
    #[cfg(target_os = "macos")]
    {
        Box::new(MacOSPermissionService::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(StubPermissionService::new())
    }
}

#[cfg(target_os = "macos")]
fn open_system_settings(kind: PermissionKind) -> PlatformResult<()> {
    std::process::Command::new("open")
        .arg(kind.system_settings_url())
        .status()
        .map_err(|error| PlatformError::SystemSettings {
            operation: "open_system_settings",
            message: error.to_string(),
        })
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(PlatformError::SystemSettings {
                    operation: "open_system_settings",
                    message: format!("open exited with {status}"),
                })
            }
        })
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn macos_accessibility_is_trusted() -> bool {
    // SAFETY: `AXIsProcessTrusted` is a process-local, side-effect-free
    // function exported by Apple's ApplicationServices framework.  It takes no
    // pointers and has no ownership requirements.  The call is kept in this
    // private wrapper so unsafe code cannot leak into the public API.
    unsafe { AXIsProcessTrusted() != 0 }
}

#[cfg(target_os = "macos")]
fn macos_full_disk_access_is_granted() -> bool {
    use std::{fs, path::PathBuf};

    fn can_read(path: PathBuf) -> bool {
        if path.is_dir() {
            fs::read_dir(path)
                .and_then(|mut entries| entries.next().transpose().map(|_| ()))
                .is_ok()
        } else {
            fs::File::open(path).is_ok()
        }
    }

    let mut probes = vec![
        PathBuf::from("/var/root"),
        PathBuf::from("/private/etc/sudoers"),
        PathBuf::from("/Library/Application Support/com.apple.TCC/TCC.db"),
        PathBuf::from("/private/var/db/SystemPolicy"),
    ];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        probes.extend([
            home.join("Library/Mail"),
            home.join("Library/Safari/Databases"),
            home.join("Library/Messages"),
            home.join("Library/Application Support/com.apple.sharedfilelist"),
        ]);
    }

    probes
        .into_iter()
        .filter(|path| path.exists())
        .any(can_read)
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_permission_provider_records_settings_without_opening_them() {
        let provider = MemoryPermissionService::new(PermissionSnapshot::new(
            PermissionStatus::Granted,
            PermissionStatus::Unknown,
            PermissionStatus::NeedsUserAction,
        ));

        assert_eq!(
            provider.status(PermissionKind::Accessibility),
            PermissionStatus::Granted
        );
        provider
            .open_system_settings(PermissionKind::InputMonitoring)
            .expect("memory provider does not spawn a process");
        assert_eq!(
            provider.opened_settings().expect("memory state is valid"),
            vec![PermissionKind::InputMonitoring]
        );
    }

    #[test]
    fn settings_urls_are_stable_and_kind_specific() {
        assert!(system_settings_url(PermissionKind::Accessibility).contains("Accessibility"));
        assert!(system_settings_url(PermissionKind::FullDiskAccess).contains("AllFiles"));
        assert!(system_settings_url(PermissionKind::InputMonitoring).contains("ListenEvent"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_provider_does_not_claim_permissions() {
        let provider = StubPermissionService::new();
        assert_eq!(provider.snapshot(), PermissionSnapshot::all_unknown());
        assert!(matches!(
            provider.open_system_settings(PermissionKind::Accessibility),
            Err(PlatformError::Unsupported { .. })
        ));
    }
}
