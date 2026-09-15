//! Permission vocabulary used by the existing macOS presentation model.
//! Windows uses session capability probes instead of fabricated TCC grants.

/// A macOS privacy permission used by `uniDesk`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionKind {
    Accessibility,
    FullDiskAccess,
    InputMonitoring,
}

impl PermissionKind {
    /// The privacy pane URL understood by `open` on macOS.
    #[must_use]
    pub const fn system_settings_url(self) -> &'static str {
        match self {
            Self::Accessibility => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
            }
            Self::FullDiskAccess => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"
            }
            Self::InputMonitoring => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent"
            }
        }
    }
}

/// What the platform boundary can currently say about a permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionStatus {
    Granted,
    Denied,
    Unknown,
    NeedsUserAction,
}

impl PermissionStatus {
    #[must_use]
    pub const fn is_granted(self) -> bool {
        matches!(self, Self::Granted)
    }

    #[must_use]
    pub const fn needs_user_action(self) -> bool {
        matches!(self, Self::NeedsUserAction)
    }
}

/// A point-in-time snapshot of the permissions relevant to `uniDesk`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionSnapshot {
    pub accessibility: PermissionStatus,
    pub full_disk_access: PermissionStatus,
    pub input_monitoring: PermissionStatus,
}

impl PermissionSnapshot {
    #[must_use]
    pub const fn new(
        accessibility: PermissionStatus,
        full_disk_access: PermissionStatus,
        input_monitoring: PermissionStatus,
    ) -> Self {
        Self {
            accessibility,
            full_disk_access,
            input_monitoring,
        }
    }

    #[must_use]
    pub const fn all_unknown() -> Self {
        Self::new(
            PermissionStatus::Unknown,
            PermissionStatus::Unknown,
            PermissionStatus::Unknown,
        )
    }

    #[must_use]
    pub const fn status(self, kind: PermissionKind) -> PermissionStatus {
        match kind {
            PermissionKind::Accessibility => self.accessibility,
            PermissionKind::FullDiskAccess => self.full_disk_access,
            PermissionKind::InputMonitoring => self.input_monitoring,
        }
    }

    #[must_use]
    pub const fn accessibility(self) -> PermissionStatus {
        self.accessibility
    }

    #[must_use]
    pub const fn full_disk_access(self) -> PermissionStatus {
        self.full_disk_access
    }

    #[must_use]
    pub const fn input_monitoring(self) -> PermissionStatus {
        self.input_monitoring
    }
}

impl Default for PermissionSnapshot {
    fn default() -> Self {
        Self::all_unknown()
    }
}

use crate::PlatformResult;
pub trait PermissionService: Send + Sync {
    fn snapshot(&self) -> PermissionSnapshot;

    fn status(&self, kind: PermissionKind) -> PermissionStatus {
        self.snapshot().status(kind)
    }

    /// # Errors
    /// Returns an error when the platform cannot open the requested settings pane.
    fn open_system_settings(&self, kind: PermissionKind) -> PlatformResult<()>;
}
